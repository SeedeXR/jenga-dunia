//! Studio's window (ADR-0017): the engine server, the viewport and the desktop UI around it —
//! the same header, toolbar, Project card and log as `apps/desktop/src/App.tsx` (owner rule: no
//! visible change from the process split). Every operation runs on a worker thread through the
//! same `Server::invoke` the Editor's calls use (client `gui-studio`), so Studio's own writes are
//! announced to the Editor and the Editor's writes refresh the viewport.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

use eframe::egui::{self, Align, Layout, Margin, RichText, ScrollArea, Stroke};
use jd_core::{Request, Response, Status};
use jd_ipc::Server;
use serde_json::{Value, json};

use crate::editor::{self, Editor};
use crate::look;
use crate::theme::{self, color};
use crate::viewport::Viewport;

/// The viewport mesh keeps at most this many samples along a side: the desktop's default
/// `previewMaxPx` (1024² ≈ 2 M triangles; its 2048² option arrives with Settings, P4).
const PREVIEW_MAX_PX: u32 = 1024;
/// Log lines kept (the desktop keeps 200).
const LOG_LINES: usize = 200;

/// World size presets, as the desktop's `PRESETS` (Unity / Unreal want 2ⁿ+1 samples).
pub const PRESETS: [(&str, u32, f64); 7] = [
    ("Preview 512² · 2 km", 512, 2048.0),
    ("Square 2048² · 8 km", 2048, 8192.0),
    ("Square 4096² · 16 km", 4096, 16384.0),
    ("Unity 1025² · 2 km", 1025, 2048.0),
    ("Unity / Unreal 2049² · 8 km", 2049, 8192.0),
    ("Unreal 4033² · 16 km", 4033, 16128.0),
    ("Unity 4097² · 16 km", 4097, 16384.0),
];

/// How Studio starts.
#[derive(Debug, Default, Clone)]
pub struct Options {
    /// Launch and supervise this Editor binary (`None`: no Editor window).
    pub editor: Option<PathBuf>,
    /// Open this project at start.
    pub project: Option<PathBuf>,
}

/// Why an engine call was made, so its reply lands in the right place.
#[derive(Debug, Clone)]
enum Tag {
    /// A button: log it, then refresh.
    User,
    Info,
    Graph,
    Layers,
    Stats(String),
    Preview([f32; 2], u32, Instant),
}

type Step = Box<dyn FnOnce(&[Response]) -> Option<Request> + Send>;

enum Msg {
    Reply {
        tag: Tag,
        response: Box<Response>,
    },
    /// A chain of operations run on one worker (import, export): their responses in order.
    Chain(Vec<Response>),
    Event {
        topic: String,
        data: Value,
    },
    EditorExited(std::process::ExitStatus),
    Log(String, Level),
    /// Baked maps for a layer (`terrain.textures`, decoded on the worker) and the bake's time.
    Maps(String, Result<crate::viewport::Maps, String>, u64),
    /// The physical-sky preview (`sky.preview`, decoded) for this sun.
    Sky(SunLook, Result<crate::viewport::Image, String>),
}

/// The look's sun as the sky preview needs it: azimuth, elevation, turbidity, clouds, fog.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SunLook([f64; 5]);

impl SunLook {
    /// From `project.info`'s `look.sun` (the desktop's defaults when unset).
    fn of(look_sun: &Value) -> Self {
        let f = |k: &str, d: f64| look_sun[k].as_f64().unwrap_or(d);
        Self([
            f("azimuth_deg", 140.0),
            f("elevation_deg", 35.0),
            f("turbidity", 3.0),
            f("clouds", 0.0),
            f("fog", 0.0),
        ])
    }
}

/// A log line's colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// `text-jd-fg-1`.
    Info,
    /// `text-jd-warn`.
    Warn,
    /// `text-jd-error`.
    Error,
}

/// One log line, as the desktop prints it: local time, then `op: status` or the error.
#[derive(Debug, Clone)]
pub struct LogLine {
    /// `7:14:28 PM`.
    pub time: String,
    /// The text.
    pub text: String,
    /// Its colour.
    pub level: Level,
}

/// The Studio application.
pub struct StudioApp {
    server: Arc<Server>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: egui::Context,
    project: Option<Value>,
    graph: Value,
    layers: Vec<String>,
    layer: Option<String>,
    /// The 3-D view.
    pub viewport: Viewport,
    log: VecDeque<LogLine>,
    busy: usize,
    reads: usize,
    refreshing: bool,
    refresh_again: bool,
    preview: Option<(u32, f64, u64)>,
    path_input: String,
    name_input: String,
    extent_input: String,
    resolution_input: String,
    import_input: String,
    fullscreen: bool,
    editor: Option<Editor>,
    editor_error: Option<String>,
    sky_for: Option<SunLook>,
}

impl StudioApp {
    /// Build the app around a bound engine server; installs the theme, starts the Editor when
    /// asked, and loads whatever project the engine has open.
    pub fn new(cc: &eframe::CreationContext<'_>, server: Arc<Server>, options: Options) -> Self {
        theme::install(&cc.egui_ctx);
        let (tx, rx) = mpsc::channel();
        let (events, ctx) = (tx.clone(), cc.egui_ctx.clone());
        server.on_event(move |topic, data| {
            let _ = events.send(Msg::Event {
                topic: topic.to_owned(),
                data: data.clone(),
            });
            ctx.request_repaint();
        });
        let mut app = Self {
            server,
            tx,
            rx,
            ctx: cc.egui_ctx.clone(),
            project: None,
            graph: Value::Null,
            layers: Vec::new(),
            layer: None,
            viewport: Viewport::default(),
            log: VecDeque::new(),
            busy: 0,
            reads: 0,
            refreshing: false,
            refresh_again: false,
            preview: None,
            path_input: String::new(),
            name_input: "World".into(),
            extent_input: "4096".into(),
            resolution_input: "1024".into(),
            import_input: String::new(),
            fullscreen: false,
            editor: None,
            editor_error: None,
            sky_for: None,
        };
        // The desktop bakes the sky on mount, project or not.
        app.fetch_sky(SunLook::of(&Value::Null));
        if let Some(path) = &options.editor {
            app.start_editor(path.clone());
        }
        match options.project {
            Some(p) => {
                app.path_input = p.display().to_string();
                app.invoke(Tag::User, "project.open", json!({ "path": p }));
            }
            None => app.refresh(),
        }
        app
    }

    /// The selected layer.
    pub fn layer(&self) -> Option<&str> {
        self.layer.as_deref()
    }

    /// The project's layers, as last read.
    pub fn layers(&self) -> &[String] {
        &self.layers
    }

    /// Engine calls still running (buttons and refreshes).
    pub fn busy(&self) -> bool {
        self.busy > 0 || self.reads > 0 || self.refreshing
    }

    /// The log, oldest first.
    pub fn log_lines(&self) -> impl Iterator<Item = &LogLine> {
        self.log.iter()
    }

    fn log(&mut self, text: String, level: Level) {
        if self.log.len() == LOG_LINES {
            self.log.pop_front();
        }
        // `toLocaleTimeString()` in the desktop (en-US): 7:14:28 PM.
        let time = jiff::Zoned::now().strftime("%-I:%M:%S %p").to_string();
        self.log.push_back(LogLine { time, text, level });
    }

    /// The desktop's `describe`: `op: status (warnings)` or `op: CODE message — next`.
    fn log_response(&mut self, r: &Response) {
        if let Some(e) = &r.error {
            let next = if e.next.is_empty() {
                String::new()
            } else {
                format!(" — {}", e.next)
            };
            self.log(
                format!("{}: {} {}{next}", r.op, e.code.as_str(), e.message),
                Level::Error,
            );
            return;
        }
        let warns: Vec<String> = r
            .warnings
            .iter()
            .map(|w| format!("{}: {}", w.code, w.message))
            .collect();
        let status = serde_json::to_value(r.status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        if warns.is_empty() {
            self.log(format!("{}: {status}", r.op), Level::Info);
        } else {
            self.log(
                format!("{}: {status} ({})", r.op, warns.join("; ")),
                Level::Warn,
            );
        }
    }

    fn start_editor(&mut self, path: PathBuf) {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let editor = Editor::start(path, self.server.socket(), self.server.token(), move |s| {
            let _ = tx.send(Msg::EditorExited(s));
            ctx.request_repaint();
        });
        if let editor::Status::Failed(msg) = &editor.status {
            self.log(msg.clone(), Level::Error);
        }
        self.editor = Some(editor);
    }

    fn request(op: &str, params: Value) -> Request {
        let mut request = Request::new(op, params);
        request.client.get_or_insert_default().kind = "gui-studio".into();
        request
    }

    /// Run an operation on a worker thread; its reply comes back as a message.
    fn invoke(&mut self, tag: Tag, op: &str, params: Value) {
        let request = Self::request(op, params);
        let (server, tx, ctx) = (self.server.clone(), self.tx.clone(), self.ctx.clone());
        let user = matches!(tag, Tag::User);
        let spawned = std::thread::Builder::new()
            .name("studio-op".into())
            .spawn(move || {
                let response = Box::new(server.invoke(request));
                let _ = tx.send(Msg::Reply { tag, response });
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) if user => self.busy += 1,
            Ok(_) => self.reads += 1,
            Err(e) => self.log(
                format!("{op}: could not start a worker thread: {e}"),
                Level::Error,
            ),
        }
    }

    /// Run `steps` on one worker, each seeing the previous responses; stops at the first error.
    fn chain(&mut self, steps: Vec<Step>) {
        let (server, tx, ctx) = (self.server.clone(), self.tx.clone(), self.ctx.clone());
        let spawned = std::thread::Builder::new()
            .name("studio-chain".into())
            .spawn(move || {
                let mut done: Vec<Response> = Vec::new();
                for step in steps {
                    let Some(request) = step(&done) else { break };
                    let r = server.invoke(request);
                    let failed = r.status == Status::Error;
                    done.push(r);
                    if failed {
                        break;
                    }
                }
                let _ = tx.send(Msg::Chain(done));
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.busy += 1,
            Err(e) => self.log(
                format!("could not start a worker thread: {e}"),
                Level::Error,
            ),
        }
    }

    /// Re-read the project, its graph and layers, and the selected layer's preview (coalesced: a
    /// refresh requested while one runs runs once more afterwards).
    fn refresh(&mut self) {
        if self.refreshing {
            self.refresh_again = true;
            return;
        }
        self.refreshing = true;
        self.invoke(Tag::Info, "project.info", json!({}));
    }

    fn refresh_done(&mut self) {
        self.refreshing = false;
        if std::mem::take(&mut self.refresh_again) {
            self.refresh();
        }
    }

    fn drain(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Reply { tag, response } => {
                    if matches!(tag, Tag::User) {
                        self.busy = self.busy.saturating_sub(1);
                    } else {
                        self.reads = self.reads.saturating_sub(1);
                    }
                    self.on_reply(tag, *response);
                }
                Msg::Chain(responses) => {
                    self.busy = self.busy.saturating_sub(1);
                    for r in &responses {
                        self.log_response(r);
                    }
                    self.refresh();
                }
                Msg::Event { topic, data } => {
                    if topic == "project_changed" {
                        tracing::debug!(op = %data["op"], "project changed");
                        self.refresh();
                    }
                }
                Msg::EditorExited(status) => {
                    if let Some(editor) = &mut self.editor {
                        let line = editor.exited(status);
                        let level = if status.success() {
                            Level::Info
                        } else {
                            Level::Error
                        };
                        self.log(line, level);
                    }
                }
                Msg::Log(text, level) => self.log(text, level),
                Msg::Maps(layer, maps, ms) => {
                    self.reads = self.reads.saturating_sub(1);
                    // Moved to another layer while baking: this result is stale.
                    if self.layer.as_deref() != Some(layer.as_str()) {
                        continue;
                    }
                    match maps {
                        Ok(m) => {
                            let [w, h] = m.albedo.size;
                            let n = self
                                .project
                                .as_ref()
                                .and_then(|p| p["look"]["materials"].as_array())
                                .map_or(0, Vec::len);
                            let with = if n == 0 {
                                String::new()
                            } else {
                                format!(" with {n} material layer{}", if n == 1 { "" } else { "s" })
                            };
                            self.log(
                                format!("terrain.textures: ok {w}×{h}{with} in {ms} ms"),
                                Level::Info,
                            );
                            self.viewport.set_maps(Some(m));
                        }
                        Err(e) => {
                            self.log(format!("terrain.textures: {e}"), Level::Error);
                            self.viewport.set_maps(None);
                        }
                    }
                }
                Msg::Sky(sun, sky) => {
                    self.reads = self.reads.saturating_sub(1);
                    match sky {
                        Ok(img) => {
                            let [w, h] = img.size;
                            let [az, el, turbidity, clouds, _] = sun.0;
                            self.log(
                                format!("sky.preview: ok {w}×{h} sun {el}°/{az}° turbidity {turbidity} clouds {} %", (clouds * 100.0).round()),
                                Level::Info,
                            );
                            self.viewport.set_sky(Some(img));
                        }
                        Err(e) => self.log(format!("sky.preview: {e}"), Level::Error),
                    }
                }
            }
        }
    }

    fn on_reply(&mut self, tag: Tag, r: Response) {
        let failed = r.status == Status::Error;
        match tag {
            Tag::User => {
                self.log_response(&r);
                self.refresh();
            }
            Tag::Info => {
                if failed {
                    // No project open: nothing to show.
                    self.project = None;
                    self.graph = Value::Null;
                    self.layers.clear();
                    self.viewport.clear();
                    self.preview = None;
                    return self.refresh_done();
                }
                let info = r.result.unwrap_or_default();
                let sun = SunLook::of(&info["look"]["sun"]);
                self.viewport.set_sun(sun.0[0] as f32, sun.0[1] as f32);
                self.viewport.set_fog(sun.0[4] as f32);
                if self.sky_for != Some(sun) {
                    self.fetch_sky(sun);
                }
                if self.path_input.is_empty() {
                    self.path_input = info["path"].as_str().unwrap_or_default().to_owned();
                }
                self.project = Some(info);
                self.invoke(Tag::Graph, "graph.list", json!({}));
                self.invoke(Tag::Layers, "terrain.layers", json!({}));
            }
            Tag::Graph => {
                if !failed {
                    self.graph = r.result.unwrap_or_default();
                }
            }
            Tag::Layers => {
                if failed {
                    self.log_response(&r);
                }
                self.layers = r
                    .result
                    .as_ref()
                    .and_then(|v| v["layers"].as_array())
                    .map_or_else(Vec::new, |a| {
                        a.iter()
                            .filter_map(|n| n.as_str().map(str::to_owned))
                            .collect()
                    });
                let keep = self
                    .layer
                    .as_ref()
                    .filter(|l| self.layers.contains(l))
                    .cloned();
                self.layer = keep.or_else(|| default_layer(&self.layers));
                match self.layer.clone() {
                    Some(layer) => self.invoke(
                        Tag::Stats(layer.clone()),
                        "terrain.stats",
                        json!({ "layer": layer }),
                    ),
                    None => {
                        self.viewport.clear();
                        self.preview = None;
                        self.refresh_done();
                    }
                }
            }
            Tag::Stats(layer) => {
                let Some(size) = r.result.as_ref().map(|v| &v["size_px"]) else {
                    self.log_response(&r);
                    return self.refresh_done();
                };
                let longest = size[0].as_u64().max(size[1].as_u64()).unwrap_or(1) as u32;
                let lod = longest
                    .div_ceil(PREVIEW_MAX_PX)
                    .max(1)
                    .next_power_of_two()
                    .trailing_zeros();
                let extent = self
                    .project
                    .as_ref()
                    .map(|p| &p["world"]["extent_m"])
                    .map_or([1.0, 1.0], |e| {
                        [
                            e[0].as_f64().unwrap_or(1.0) as f32,
                            e[1].as_f64().unwrap_or(1.0) as f32,
                        ]
                    });
                self.invoke(
                    Tag::Preview(extent, lod, Instant::now()),
                    "terrain.preview",
                    json!({ "layer": layer, "lod": lod }),
                );
            }
            Tag::Preview(extent, lod, started) => {
                match r.result.as_ref().map(decode_preview) {
                    Some(Ok((pv, bytes))) => {
                        self.viewport.set_terrain(&pv, extent);
                        self.fetch_maps();
                        self.preview = Some((
                            lod,
                            bytes as f64 / 1_048_576.0,
                            started.elapsed().as_millis() as u64,
                        ));
                    }
                    Some(Err(e)) => self.log(format!("layer_preview: {e}"), Level::Error),
                    None => self.log_response(&r),
                }
                self.refresh_done();
            }
        }
    }

    /// The desktop's `refreshSky`: the physical sky for the look's sun, 512 wide, three times the
    /// bake's exposure (the dome is seen tonemapped).
    fn fetch_sky(&mut self, sun: SunLook) {
        self.sky_for = Some(sun);
        let [az, el, turbidity, clouds, fog] = sun.0;
        let request = Self::request(
            "sky.preview",
            json!({ "sun_azimuth_deg": az, "sun_elevation_deg": el.max(1.0), "turbidity": turbidity, "cloud_cover": clouds, "fog": fog, "exposure": 0.15, "width": 512 }),
        );
        let (server, tx, ctx) = (self.server.clone(), self.tx.clone(), self.ctx.clone());
        self.reads += 1;
        std::thread::spawn(move || {
            let r = server.invoke(request);
            let sky = match (&r.result, &r.error) {
                (Some(v), _) => crate::viewport::Image::from_png_base64(
                    v["png_base64"].as_str().unwrap_or_default(),
                ),
                (None, Some(e)) => Err(format!("{} {}", e.code.as_str(), e.message)),
                (None, None) => Err("no result".into()),
            };
            let _ = tx.send(Msg::Sky(sun, sky));
            ctx.request_repaint();
        });
    }

    /// The desktop's `refreshMaps`: albedo / normal / roughness (/ emissive) baked from the full
    /// field for height-like layers; a live preview (`#lod<N>`) keeps the previous maps.
    fn fetch_maps(&mut self) {
        let Some(layer) = self.layer.clone() else {
            return;
        };
        if !layer
            .split('#')
            .next()
            .is_some_and(|b| b.starts_with("height"))
        {
            return self.viewport.set_maps(None);
        }
        if layer.contains("#lod") {
            return;
        }
        let materials = self.project.as_ref().map_or(json!([]), |p| {
            p["look"]["materials"]
                .as_array()
                .map_or(json!([]), |m| Value::Array(m.clone()))
        });
        let request = Self::request(
            "terrain.textures",
            json!({ "materials": materials, "layer": layer, "max_px": PREVIEW_MAX_PX }),
        );
        let (server, tx, ctx) = (self.server.clone(), self.tx.clone(), self.ctx.clone());
        self.reads += 1;
        std::thread::spawn(move || {
            let started = Instant::now();
            let r = server.invoke(request);
            let maps = match (&r.result, &r.error) {
                (Some(v), _) => (|| {
                    let png = |k: &str| {
                        crate::viewport::Image::from_png_base64(v[k].as_str().unwrap_or_default())
                    };
                    Ok(crate::viewport::Maps {
                        albedo: png("albedo_png_base64")?,
                        normal: png("normal_png_base64")?,
                        roughness: png("roughness_png_base64")?,
                        emissive: match v["emissive_png_base64"].as_str() {
                            Some(e) => Some(crate::viewport::Image::from_png_base64(e)?),
                            None => None,
                        },
                    })
                })(),
                (None, Some(e)) => Err(format!("{} {}", e.code.as_str(), e.message)),
                (None, None) => Err("no result".into()),
            };
            let ms = started.elapsed().as_millis() as u64;
            let _ = tx.send(Msg::Maps(layer, maps, ms));
            ctx.request_repaint();
        });
    }

    fn world_inputs(&self) -> Option<(f64, u32)> {
        Some((
            self.extent_input.trim().parse().ok()?,
            self.resolution_input.trim().parse().ok()?,
        ))
    }

    fn new_project(&mut self, path: String) {
        let Some((extent, resolution)) = self.world_inputs() else {
            return self.log(
                "project.create: extent and resolution must be numbers".into(),
                Level::Error,
            );
        };
        let name = self.name_input.trim().to_owned();
        self.invoke(
            Tag::User,
            "project.create",
            json!({ "path": path, "name": name, "world": { "extent_m": [extent, extent], "resolution_px": [resolution, resolution], "tile_size": 512, "halo": 4, "origin": [0, 0], "seed": 42 } }),
        );
    }

    /// The desktop's `importAsset`: copy into assets/, then a `jd.input.heightmap@1` node right of
    /// the graph.
    fn import_asset(&mut self, path: String) {
        let right = self.graph["nodes"].as_array().map_or(0.0, |nodes| {
            nodes
                .iter()
                .filter_map(|n| n["ui"]["position"][0].as_f64())
                .fold(-320.0, f64::max)
        });
        self.chain(vec![
            Box::new(move |_| {
                Some(Self::request(
                    "project.import_asset",
                    json!({ "path": path }),
                ))
            }),
            Box::new(move |done| {
                let name = done.last()?.result.as_ref()?["name"].clone();
                Some(Self::request(
                    "graph.add_node",
                    json!({ "kind": "jd.input.heightmap@1", "params": { "path": name }, "position": [right + 320.0, 0] }),
                ))
            }),
        ]);
    }

    /// The desktop's `exportHeightmap`: a live preview only published `height#lod<N>`, so the full
    /// resolution is evaluated first.
    fn export_heightmap(&mut self, path: String) {
        let format = format_for(&path);
        let live = self.layer.as_deref().is_some_and(|l| l.contains("#lod"));
        let mut steps: Vec<Step> = Vec::new();
        if live {
            steps.push(Box::new(|_| {
                Some(Self::request("graph.evaluate", json!({ "lod": 0 })))
            }));
        }
        steps.push(Box::new(move |_| {
            Some(Self::request(
                "terrain.export_heightmap",
                json!({ "layer": "height", "path": path, "format": format }),
            ))
        }));
        self.chain(steps);
    }

    /// Toolbar Still: the viewport's last frame as PNG in exports/stills/.
    fn save_still(&mut self, frame: &eframe::Frame) {
        let Some(rs) = frame.wgpu_render_state() else {
            return;
        };
        let Some((w, h, rgba)) = self.viewport.capture(rs) else {
            return self.log("still: the viewport gave no image".into(), Level::Error);
        };
        let mut png = Vec::new();
        let encoded = image::RgbaImage::from_raw(w, h, rgba)
            .ok_or_else(|| "bad frame size".to_owned())
            .and_then(|img| {
                img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                    .map_err(|e| e.to_string())
            });
        if let Err(e) = encoded {
            return self.log(format!("still: {e}"), Level::Error);
        }
        use base64::Engine as _;
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() % 100_000);
        let request = Self::request(
            "project.write_export",
            json!({ "path": format!("stills/view_{n}.png"), "bytes_base64": base64::engine::general_purpose::STANDARD.encode(&png) }),
        );
        let (server, tx, ctx, kb) = (
            self.server.clone(),
            self.tx.clone(),
            self.ctx.clone(),
            png.len() / 1024,
        );
        std::thread::spawn(move || {
            let r = server.invoke(request);
            let line = match r.result.as_ref().and_then(|v| v["path"].as_str()) {
                Some(p) => (format!("still: {p} ({kb} KB)"), Level::Info),
                None => ("still: write failed".to_owned(), Level::Error),
            };
            let _ = tx.send(Msg::Log(line.0, line.1));
            ctx.request_repaint();
        });
    }

    fn header(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let rect = ui.max_rect();
        ui.painter().hline(
            rect.x_range(),
            rect.bottom(),
            Stroke::new(1.0, look::HAIRLINE),
        );
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.add(
                egui::Image::new(egui::include_image!("../../desktop/public/jd-mark.svg"))
                    .fit_to_exact_size(egui::vec2(20.0, 20.0)),
            )
            .widget_info(|| {
                egui::WidgetInfo::labeled(egui::accesskit::Role::Image, true, "Jenga Dunia")
            });
            ui.label(
                RichText::new("Jenga Dunia")
                    .font(theme::semibold(13.0))
                    .color(color::FG0)
                    .extra_letter_spacing(0.3),
            );
            ui.label(
                RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                    .size(12.0)
                    .color(color::FG2),
            );
            let readout = match &self.project {
                Some(p) => format!(
                    "{} — {}² · {} m",
                    p["name"].as_str().unwrap_or_default(),
                    p["world"]["resolution_px"][0],
                    number(&p["world"]["extent_m"][0])
                ),
                None => "no project".into(),
            };
            ui.label(RichText::new(readout).size(12.0).color(color::FG2));
            self.toolbar(ui, frame);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.busy > 0 {
                    ui.label(RichText::new("working…").size(12.0).color(color::ACCENT));
                }
                ui.label(
                    RichText::new(self.viewport_info())
                        .size(12.0)
                        .color(color::FG2),
                );
                if self.layers.len() > 1 {
                    let mut selected = self.layer.clone();
                    egui::ComboBox::from_id_salt("layer-pick")
                        .selected_text(
                            RichText::new(selected.clone().unwrap_or_default()).size(12.0),
                        )
                        .show_ui(ui, |ui| {
                            for l in &self.layers {
                                ui.selectable_value(&mut selected, Some(l.clone()), l);
                            }
                        })
                        .response
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::accesskit::Role::ComboBox,
                                true,
                                "layer",
                            )
                        });
                    ui.label(RichText::new("layer").size(12.0).color(color::FG2));
                    if selected != self.layer {
                        self.layer = selected;
                        self.refresh();
                    }
                }
            });
        });
    }

    fn viewport_info(&self) -> String {
        let tris = self.viewport.info().map_or(0, |i| i.triangles);
        let mut s = format!("viewport: wgpu · {} tris", thousands(tris));
        if let Some(p) = &self.project {
            let sun = &p["look"]["sun"];
            let az = sun["azimuth_deg"].as_f64().unwrap_or(140.0);
            let el = sun["elevation_deg"].as_f64().unwrap_or(35.0);
            s += &format!(" · sun {el}°/{az}°");
        }
        if let Some([w, h]) = self.viewport.maps_px() {
            s += &format!(" · pbr {w}×{h}");
        }
        if let Some([w, h]) = self.viewport.sky_px() {
            s += &format!(" · sky {w}×{h}");
        }
        if self.viewport.frame && self.viewport.info().is_some() {
            s += " · box";
        }
        if let Some((lod, mb, ms)) = self.preview {
            s += &format!(" · preview lod {lod}, {mb:.2} MB in {ms} ms");
        }
        s
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let (x, y) = (ui.cursor().left() - 4.0, ui.max_rect().center().y);
        ui.painter()
            .vline(x, y - 9.0..=y + 9.0, Stroke::new(1.0, color::LINE));
        ui.spacing_mut().item_spacing.x = 2.0;
        let (has_project, idle) = (self.project.is_some(), self.busy == 0);
        let undo = self.graph["undo_depth"].as_u64().unwrap_or(0) > 0;
        let redo = self.graph["redo_depth"].as_u64().unwrap_or(0) > 0;
        if look::icon_button(ui, "file-plus", "New project… (⌘N)", idle).clicked()
            && let Some(p) = rfd::FileDialog::new()
                .set_title("New Jenga Dunia project")
                .set_file_name(format!("{}.jdproj", self.name_input.trim()))
                .save_file()
        {
            let mut p = p.display().to_string();
            if !p.ends_with(".jdproj") {
                p.push_str(".jdproj"); // a project is a directory
            }
            self.path_input = p.clone();
            self.new_project(p);
        }
        if look::icon_button(ui, "folder-open", "Open project… (⌘O)", idle).clicked()
            && let Some(p) = rfd::FileDialog::new()
                .set_title("Open Jenga Dunia project")
                .pick_folder()
        {
            self.path_input = p.display().to_string();
            self.invoke(Tag::User, "project.open", json!({ "path": p }));
        }
        if look::icon_button(ui, "save", "Save (⌘S)", has_project && idle).clicked() {
            self.invoke(Tag::User, "project.save", json!({}));
        }
        if look::icon_button(
            ui,
            "file-input",
            "Import a DEM, heightmap or FBX into assets/ (⌘I)",
            has_project && idle,
        )
        .clicked()
            && let Some(p) = pick_asset()
        {
            self.import_input = p.clone();
            self.import_asset(p);
        }
        if look::icon_button(
            ui,
            "download",
            "Export the heightmap… (⌘E)",
            self.layer.is_some() && idle,
        )
        .clicked()
            && let Some(p) = rfd::FileDialog::new()
                .set_title("Export heightmap")
                .set_file_name("height.exr")
                .add_filter("OpenEXR (32-bit float)", &["exr"])
                .add_filter("PNG (16-bit)", &["png"])
                .add_filter("RAW 16-bit", &["r16"])
                .add_filter("RAW 32-bit float", &["r32"])
                .add_filter("TIFF (32-bit float)", &["tif", "tiff"])
                .save_file()
        {
            self.export_heightmap(p.display().to_string());
        }
        look::toolbar_separator(ui);
        if look::icon_button(
            ui,
            "undo-2",
            "Undo graph edit (⌥⌘Z)",
            has_project && undo && idle,
        )
        .clicked()
        {
            self.invoke(Tag::User, "graph.undo", json!({}));
        }
        if look::icon_button(
            ui,
            "redo-2",
            "Redo graph edit (⇧⌥⌘Z)",
            has_project && redo && idle,
        )
        .clicked()
        {
            self.invoke(Tag::User, "graph.redo", json!({}));
        }
        look::toolbar_separator(ui);
        // ponytail: the desktop's Generate first copies the Noise panel into the graph's noise
        // node; Studio's Noise panel arrives with the panel port (P4), until then both evaluate.
        if look::icon_button(ui, "wand-2", "Generate terrain (⌘G)", has_project && idle).clicked()
        {
            self.invoke(Tag::User, "graph.evaluate", json!({ "lod": 0 }));
        }
        if look::icon_button(ui, "play", "Evaluate the graph", has_project && idle).clicked() {
            self.invoke(Tag::User, "graph.evaluate", json!({ "lod": 0 }));
        }
        look::toolbar_separator(ui);
        if look::icon_button(
            ui,
            "camera",
            "Save the viewport as a still (exports/stills/)",
            has_project && self.viewport.info().is_some() && idle,
        )
        .clicked()
        {
            self.save_still(frame);
        }
        if look::icon_button(ui, "maximize-2", "Fullscreen viewport (Esc leaves)", true).clicked() {
            self.set_fullscreen(ui.ctx(), !self.fullscreen);
        }
        look::toolbar_separator(ui);
        // Studio's own addition: the Editor (node graph) window's state.
        let (tip, running) = match self.editor.as_ref().map(|e| &e.status) {
            Some(editor::Status::Running(pid)) => {
                (format!("Node editor running (process {pid})"), true)
            }
            Some(editor::Status::Closed) => ("Open the node editor".into(), false),
            Some(editor::Status::GaveUp(m) | editor::Status::Failed(m)) => {
                (format!("{m} — click to open it again"), false)
            }
            None => (
                self.editor_error.clone().map_or_else(
                    || "Open the node editor".into(),
                    |e| format!("{e} — click to retry"),
                ),
                false,
            ),
        };
        if look::icon_button(ui, "workflow", &tip, !running).clicked() {
            match &mut self.editor {
                Some(e) => e.reopen(),
                None => match editor::locate(None) {
                    Ok(path) => self.start_editor(path),
                    Err(msg) => {
                        self.log(msg.clone(), Level::Error);
                        self.editor_error = Some(msg);
                    }
                },
            }
        }
    }

    fn set_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        self.fullscreen = on;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
    }

    fn project_card(&mut self, ui: &mut egui::Ui) {
        look::section(ui, "Project");
        let idle = self.busy == 0;
        look::field(ui, "Path", |ui| {
            if look::button(ui, Some("folder-search"), "", false, idle)
                .on_hover_text("Browse… (opens an existing project)")
                .clicked()
                && let Some(p) = rfd::FileDialog::new()
                    .set_title("Open Jenga Dunia project")
                    .pick_folder()
            {
                self.path_input = p.display().to_string();
                self.invoke(Tag::User, "project.open", json!({ "path": p }));
            }
            look::input(
                ui,
                &mut self.path_input,
                112.0,
                "~/Worlds/Hills.jdproj",
                "Path",
            );
        });
        look::field(ui, "Name", |ui| {
            look::input(ui, &mut self.name_input, 144.0, "", "Name");
        });
        look::field(ui, "Extent (m)", |ui| {
            look::input(ui, &mut self.extent_input, 112.0, "", "Extent (m)");
        });
        look::field(ui, "Resolution (px)", |ui| {
            look::input(ui, &mut self.resolution_input, 112.0, "", "Resolution (px)");
        });
        look::field(ui, "Preset", |ui| {
            let mut chosen: Option<usize> = None;
            egui::ComboBox::from_id_salt("world-preset")
                .width(144.0)
                .selected_text(RichText::new("choose…").size(12.0))
                .show_ui(ui, |ui| {
                    for (i, (label, ..)) in PRESETS.iter().enumerate() {
                        if ui.selectable_label(false, *label).clicked() {
                            chosen = Some(i);
                        }
                    }
                })
                .response
                .on_hover_text(
                    "Engine-friendly sizes: Unity and Unreal heightmaps are 2ⁿ+1 samples",
                )
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::accesskit::Role::ComboBox, true, "Preset")
                });
            if let Some(i) = chosen {
                self.resolution_input = PRESETS[i].1.to_string();
                self.extent_input = PRESETS[i].2.to_string();
            }
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let path = self.path_input.trim().to_owned();
            if look::button(
                ui,
                Some("folder-plus"),
                "New",
                true,
                !path.is_empty() && idle,
            )
            .on_hover_text("Create at the path above (⌘N opens a dialog)")
            .clicked()
            {
                self.new_project(path.clone());
            }
            if look::button(
                ui,
                Some("folder-open"),
                "Open",
                false,
                !path.is_empty() && idle,
            )
            .clicked()
            {
                self.invoke(Tag::User, "project.open", json!({ "path": path }));
            }
            if look::button(
                ui,
                Some("save"),
                "Save",
                false,
                self.project.is_some() && idle,
            )
            .clicked()
            {
                self.invoke(Tag::User, "project.save", json!({}));
            }
        });
        let Some(p) = self.project.clone() else {
            return;
        };
        look::field(ui, "Import", |ui| {
            if look::button(ui, Some("folder-search"), "", false, idle)
                .on_hover_text("Browse…")
                .clicked()
                && let Some(path) = pick_asset()
            {
                self.import_input = path.clone();
                self.import_asset(path);
            }
            let path = self.import_input.trim().to_owned();
            if look::button(ui, Some("upload"), "", false, !path.is_empty() && idle)
                .on_hover_text("Import this file (⌘I opens a dialog)")
                .clicked()
            {
                self.import_asset(path);
            }
            look::input(
                ui,
                &mut self.import_input,
                112.0,
                "dem.tif, sculpt.fbx…",
                "Import",
            );
        });
        ui.add_space(12.0);
        egui::Grid::new("project-meta")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                let dt = |ui: &mut egui::Ui, t: &str| {
                    ui.label(RichText::new(t).size(12.0).color(color::FG2));
                };
                dt(ui, "id");
                ui.label(
                    RichText::new(p["project_id"].as_str().unwrap_or_default())
                        .monospace()
                        .size(12.0)
                        .color(color::FG1),
                );
                ui.end_row();
                dt(ui, "dirty");
                dt(ui, &p["dirty"].to_string());
                ui.end_row();
                dt(ui, "journal");
                dt(ui, &p["pending_journal"].to_string());
                ui.end_row();
            });
    }

    fn log_card(&mut self, ui: &mut egui::Ui) {
        look::card()
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        if self.log.is_empty() {
                            ui.label(
                                RichText::new("Operations and warnings appear here.")
                                    .monospace()
                                    .size(12.0)
                                    .color(color::FG2),
                            );
                        }
                        for line in &self.log {
                            let c = match line.level {
                                Level::Info => color::FG1,
                                Level::Warn => color::WARN,
                                Level::Error => color::ERROR,
                            };
                            ui.label(
                                RichText::new(format!("{} {}", line.time, line.text))
                                    .monospace()
                                    .size(12.0)
                                    .color(c),
                            );
                        }
                    });
            });
    }
}

fn pick_asset() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("Import heightmap or mesh")
        .add_filter(
            "Heightmaps and meshes",
            &["png", "exr", "tif", "tiff", "r16", "r32", "fbx"],
        )
        .pick_file()
        .map(|p| p.display().to_string())
}

/// The desktop's `formatFor`: the heightmap format from the file extension.
fn format_for(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "png16",
        Some("r16") => "raw_r16",
        Some("r32") => "raw_r32",
        Some("tif" | "tiff") => "tiff32",
        _ => "exr",
    }
}

/// A JSON number as JavaScript prints it (4096, not 4096.0).
fn number(v: &Value) -> String {
    match v.as_f64() {
        Some(f) if f.fract() == 0.0 => format!("{f:.0}"),
        Some(f) => f.to_string(),
        None => v.to_string(),
    }
}

/// `toLocaleString()` for a count: 522,242.
fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `height` when published, else the first `height#lod…`, else the first layer.
fn default_layer(layers: &[String]) -> Option<String> {
    layers
        .iter()
        .find(|l| *l == "height")
        .or_else(|| layers.iter().find(|l| l.starts_with("height")))
        .or_else(|| layers.first())
        .cloned()
}

fn decode_preview(v: &Value) -> Result<(jd_terrain::preview::Preview, usize), String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(v["bytes_base64"].as_str().unwrap_or_default())
        .map_err(|e| format!("bad base64: {e}"))?;
    let n = bytes.len();
    jd_terrain::preview::Preview::from_bytes(&bytes)
        .map(|p| (p, n))
        .ok_or_else(|| "the preview bytes are not in the wire format".to_owned())
}

impl eframe::App for StudioApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.drain();
        if self.fullscreen && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.set_fullscreen(ui.ctx(), false);
        }
        let bg = egui::Frame::new().fill(color::BG0);
        if !self.fullscreen {
            // The desktop's grid: a 40 px header, the panels, a 160 px log.
            egui::Panel::top("header")
                .exact_size(40.0)
                .frame(
                    egui::Frame::new()
                        .fill(color::BG1)
                        .inner_margin(Margin::symmetric(12, 0)),
                )
                .show(ui, |ui| self.header(ui, frame));
            egui::Panel::bottom("log")
                .exact_size(160.0)
                .frame(bg.inner_margin(Margin {
                    left: 8,
                    right: 8,
                    top: 0,
                    bottom: 8,
                }))
                .show(ui, |ui| self.log_card(ui));
            egui::Panel::left("panel-left")
                .resizable(true)
                .default_size(272.0)
                .size_range(212.0..=532.0)
                .frame(bg.inner_margin(Margin {
                    left: 8,
                    right: 4,
                    top: 8,
                    bottom: 8,
                }))
                .show(ui, |ui| {
                    look::card().show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        ScrollArea::vertical()
                            .auto_shrink(false)
                            .show(ui, |ui| self.project_card(ui));
                    });
                });
        }
        let margin = if self.fullscreen {
            Margin::ZERO
        } else {
            Margin {
                left: 4,
                right: 8,
                top: 8,
                bottom: 8,
            }
        };
        egui::CentralPanel::default()
            .frame(bg.inner_margin(margin))
            .show(ui, |ui| self.viewport.show(ui, frame.wgpu_render_state()));
    }

    fn on_exit(&mut self) {
        self.server.shutdown("Studio closed");
        if let Some(editor) = &mut self.editor {
            editor.stop(std::time::Duration::from_secs(3));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readouts_print_like_the_desktop() {
        assert_eq!(thousands(522_242), "522,242");
        assert_eq!(thousands(12), "12");
        assert_eq!(number(&json!(4096.0)), "4096");
        assert_eq!(number(&json!(0.5)), "0.5");
        assert_eq!(format_for("a/b.PNG"), "png16");
        assert_eq!(format_for("x.tiff"), "tiff32");
        assert_eq!(format_for("x"), "exr");
    }
}
