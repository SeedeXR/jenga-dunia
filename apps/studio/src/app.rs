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
use crate::viewport::Camera;
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
    /// `project.info` after Studio's own look write: the project's look, nothing else.
    InfoOnly,
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
    /// The `lakes` depth layer's preview, for the terrain preview of this layer.
    Lakes(String, Result<Option<jd_terrain::preview::Preview>, String>),
    /// The wind sea for a wind (`ocean.waves`).
    Waves([Option<f64>; 3], Box<Response>),
    /// An HDRI's preview (`sky.load_hdri`).
    Hdri(String, Result<(crate::viewport::Image, [f64; 3]), String>),
    /// The horizon skirt's preview (`terrain.horizon`).
    Horizon(Result<jd_terrain::preview::Preview, String>),
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

/// An HDRI on the dome (`sky.load_hdri`): the sun it implies (its size is the dome's).
#[derive(Debug, Clone)]
struct Hdri {
    sun: [f64; 2],
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
    show_horizon: bool,
    /// The Sun panel's values (the desktop's `sun` state; written to the look 400 ms after an
    /// edit settles, the sky re-baked 150 ms after).
    sun: SunLook,
    rain: f64,
    pbr: bool,
    look_due: Option<Instant>,
    sky_due: Option<Instant>,
    /// The physical sky's last image, and an HDRI that replaces it on the dome.
    sky_img: Option<crate::viewport::Image>,
    hdri_path: String,
    hdri: Option<Hdri>,
    /// The wind the last `ocean.waves` was fetched for, and its result.
    waves_for: Option<[Option<f64>; 3]>,
    waves: Option<Value>,
    /// Flythrough keys (the desktop's `flyKeys`, session only), the frames to record, and a
    /// Play or Record under way.
    fly_keys: Vec<Camera>,
    fly_frames: String,
    fly: Option<Fly>,
}

/// A flythrough run along the keys.
enum Fly {
    /// 4 s along the path in the viewport, nothing written.
    Play(Instant),
    /// Frame `k` of `frames`; PNGs go to the writer thread, the camera back to `start` at the end.
    Record {
        k: usize,
        frames: usize,
        start: Camera,
        tx: Sender<(usize, Vec<u8>)>,
    },
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
            show_horizon: false,
            sun: SunLook::of(&Value::Null),
            rain: 0.0,
            pbr: true,
            look_due: None,
            sky_due: None,
            sky_img: None,
            hdri_path: "sky.exr".into(),
            hdri: None,
            waves_for: None,
            waves: None,
            fly_keys: Vec::new(),
            fly_frames: "48".into(),
            fly: None,
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

    /// The header's viewport readout as it reads now.
    pub fn viewport_readout(&self) -> String {
        self.viewport_info()
    }

    /// Set the HDRI field (tests).
    pub fn set_hdri_path(&mut self, path: &str) {
        path.clone_into(&mut self.hdri_path);
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
                        // A look write changes no terrain: re-read the project, not the preview.
                        if data["op"] == "project.set_look" {
                            self.invoke(Tag::InfoOnly, "project.info", json!({}));
                        } else {
                            self.refresh();
                        }
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
                Msg::Lakes(layer, lakes) => {
                    self.reads = self.reads.saturating_sub(1);
                    if self.layer.as_deref() != Some(layer.as_str()) {
                        continue;
                    }
                    match lakes {
                        Ok(pv) => self.viewport.set_lakes(pv),
                        Err(e) => self.log(format!("layer_preview (lakes): {e}"), Level::Error),
                    }
                }
                Msg::Waves(wind, r) => {
                    self.reads = self.reads.saturating_sub(1);
                    self.log_response(&r);
                    if self.waves_for == Some(wind) {
                        self.waves = r.result.clone();
                        self.update_sea();
                    }
                }
                Msg::Horizon(preview) => {
                    self.reads = self.reads.saturating_sub(1);
                    match preview {
                        Ok(pv) if self.show_horizon => self.viewport.set_horizon(Some((&pv, 3.0))),
                        Ok(_) => {}
                        Err(e) => self.log(format!("terrain.horizon: {e}"), Level::Error),
                    }
                }
                Msg::Hdri(path, r) => {
                    self.reads = self.reads.saturating_sub(1);
                    match r {
                        Ok((img, [az, el, lum])) => {
                            let [w, h] = img.size;
                            self.log(
                                format!(
                                    "sky.load_hdri: ok {w}×{h} sun {}°/{}° luminance {lum:.2}",
                                    el.round(),
                                    az.round()
                                ),
                                Level::Info,
                            );
                            self.hdri = Some(Hdri { sun: [az, el] });
                            self.viewport.set_sky(Some(img));
                        }
                        Err(_) => self.log(
                            format!("sky.load_hdri: E_INPUT_INVALID could not read {path}"),
                            Level::Error,
                        ),
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
                            // An HDRI keeps the dome; the physical sky waits for its ×.
                            if self.hdri.is_none() {
                                self.viewport.set_sky(Some(img.clone()));
                            }
                            self.sky_img = Some(img);
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
                if self.look_due.is_none() {
                    self.sun = SunLook::of(&info["look"]["sun"]);
                }
                self.apply_sun();
                if self.sky_for != Some(self.sun) && self.sky_due.is_none() {
                    self.fetch_sky(self.sun);
                }
                if self.path_input.is_empty() {
                    self.path_input = info["path"].as_str().unwrap_or_default().to_owned();
                }
                self.project = Some(info);
                self.invoke(Tag::Graph, "graph.list", json!({}));
                self.invoke(Tag::Layers, "terrain.layers", json!({}));
            }
            Tag::InfoOnly => {
                if !failed {
                    self.project = r.result;
                }
            }
            Tag::Graph => {
                if !failed {
                    self.graph = r.result.unwrap_or_default();
                    self.update_sea();
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
                        self.fetch_horizon();
                        self.fetch_lakes(lod);
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

    /// The graph's sea-level node, as the desktop finds it.
    fn sea_node(&self) -> Option<Value> {
        self.graph["nodes"]
            .as_array()?
            .iter()
            .find(|n| {
                n["kind"]
                    .as_str()
                    .is_some_and(|k| k.starts_with("jd.hydrology.sea_level@"))
            })
            .cloned()
    }

    /// The desktop's sea: the sea-level node's level and water look while a height layer is shown,
    /// and its wind sea (`ocean.waves`, re-fetched when the node's wind — or the world's — changes).
    fn update_sea(&mut self) {
        let node = self.sea_node();
        let heightish = self
            .layer
            .as_deref()
            .is_none_or(|l| l.starts_with("height"));
        // The wind sea follows the node's own wind, else the world's shared wind.
        let wind = node.as_ref().map(|n| {
            let w = self
                .project
                .as_ref()
                .map(|p| p["world"]["wind"].clone())
                .unwrap_or_default();
            let p = &n["params"];
            [
                p["wind_speed_mps"]
                    .as_f64()
                    .or(w["speed_mps"].as_f64())
                    .or(Some(8.0)),
                p["fetch_km"].as_f64(),
                p["wind_azimuth_deg"]
                    .as_f64()
                    .or(w["azimuth_deg"].as_f64())
                    .or(Some(270.0)),
            ]
        });
        if wind != self.waves_for {
            self.waves_for = wind;
            self.waves = None;
            if let Some(w) = wind {
                let request = Self::request(
                    "ocean.waves",
                    json!({ "wind_speed_mps": w[0], "fetch_km": w[1], "wind_azimuth_deg": w[2] }),
                );
                let (server, tx, ctx) = (self.server.clone(), self.tx.clone(), self.ctx.clone());
                self.reads += 1;
                std::thread::spawn(move || {
                    let r = Box::new(server.invoke(request));
                    let _ = tx.send(Msg::Waves(w, r));
                    ctx.request_repaint();
                });
            }
        }
        let sea = node.filter(|_| heightish).map(|n| {
            let p = &n["params"];
            let f = |v: &Value, d: f64| v.as_f64().unwrap_or(d) as f32;
            let tint = p["tint"]
                .as_array()
                .filter(|t| t.len() == 3)
                .map_or([0.1, 0.35, 0.4], |t| {
                    [f(&t[0], 0.1), f(&t[1], 0.35), f(&t[2], 0.4)]
                });
            let w = self.waves.as_ref();
            crate::viewport::Sea {
                level_m: f(&p["level_m"], 0.0),
                visibility_m: f(&p["visibility_m"], 15.0),
                tint,
                breaking_depth_m: w.map_or(0.0, |w| f(&w["breaking_depth_m"], 0.0)),
                waves: w
                    .and_then(|w| w["components"].as_array())
                    .map_or_else(Vec::new, |cs| {
                        cs.iter()
                            .map(|c| crate::viewport::Wave {
                                amplitude_m: f(&c["amplitude_m"], 0.0),
                                wavelength_m: f(&c["wavelength_m"], 1.0),
                                direction_deg: f(&c["direction_deg"], 0.0),
                                speed_mps: f(&c["speed_mps"], 0.0),
                                phase: f(&c["phase"], 0.0),
                            })
                            .collect()
                    }),
                waves_m: w.map(|w| {
                    [
                        f(&w["significant_height_m"], 0.0),
                        f(&w["wavelength_m"], 0.0),
                    ]
                }),
            }
        });
        self.viewport.set_sea(sea);
    }

    /// The desktop's standing water: a published `lakes` layer (`lakes#lod<N>` beside a live
    /// preview) is the fill node's depth map, fetched at the shown preview's lod.
    fn fetch_lakes(&mut self, lod: u32) {
        let Some(layer) = self.layer.clone() else {
            return;
        };
        let live = layer.contains('#');
        let base = |n: &String| n.split('#').next() == Some("lakes");
        let lakes = self
            .layers
            .iter()
            .find(|n| base(n) && n.contains('#') == live)
            .or_else(|| self.layers.iter().find(|n| base(n)))
            .cloned();
        let Some(lakes) = lakes else {
            return self.viewport.set_lakes(None);
        };
        let request = Self::request("terrain.preview", json!({ "layer": lakes, "lod": lod }));
        let (server, tx, ctx) = (self.server.clone(), self.tx.clone(), self.ctx.clone());
        self.reads += 1;
        std::thread::spawn(move || {
            let r = server.invoke(request);
            let pv = match (&r.result, &r.error) {
                (Some(v), _) => decode_preview(v).map(|(p, _)| Some(p)),
                (None, Some(e)) => Err(format!("{} {}", e.code.as_str(), e.message)),
                (None, None) => Ok(None),
            };
            let _ = tx.send(Msg::Lakes(layer, pv));
            ctx.request_repaint();
        });
    }

    /// The desktop's `loadHdri`: the EXR (in assets/ or absolute) as the dome's image.
    fn load_hdri(&mut self) {
        let path = self.hdri_path.trim().to_owned();
        let request = Self::request("sky.load_hdri", json!({ "path": path, "width": 512 }));
        let (server, tx, ctx) = (self.server.clone(), self.tx.clone(), self.ctx.clone());
        self.reads += 1;
        std::thread::spawn(move || {
            let r = server.invoke(request);
            let out = match &r.result {
                Some(v) => crate::viewport::Image::from_png_base64(
                    v["png_base64"].as_str().unwrap_or_default(),
                )
                .map(|img| {
                    let f = |k: &str| v[k].as_f64().unwrap_or(0.0);
                    (
                        img,
                        [
                            f("sun_azimuth_deg"),
                            f("sun_elevation_deg"),
                            f("mean_luminance"),
                        ],
                    )
                }),
                None => Err(r.error.map(|e| e.message).unwrap_or_default()),
            };
            let _ = tx.send(Msg::Hdri(path, out));
            ctx.request_repaint();
        });
    }

    /// The desktop's `refreshHorizon`: the noise continued over 3 × the world at 256².
    fn fetch_horizon(&mut self) {
        if !self.show_horizon || self.project.is_none() {
            return;
        }
        let request = Self::request("terrain.horizon", json!({ "scale": 3.0, "px": 256 }));
        let (server, tx, ctx) = (self.server.clone(), self.tx.clone(), self.ctx.clone());
        self.reads += 1;
        std::thread::spawn(move || {
            let r = server.invoke(request);
            let preview = match (&r.result, &r.error) {
                (Some(v), _) => decode_preview(v).map(|(p, _)| p),
                (None, Some(e)) => Err(format!("{} {}", e.code.as_str(), e.message)),
                (None, None) => Err("no result".into()),
            };
            let _ = tx.send(Msg::Horizon(preview));
            ctx.request_repaint();
        });
    }

    /// The desktop's `refreshMaps`: albedo / normal / roughness (/ emissive) baked from the full
    /// field for height-like layers; a live preview (`#lod<N>`) keeps the previous maps.
    fn fetch_maps(&mut self) {
        let Some(layer) = self.layer.clone() else {
            return;
        };
        if !self.pbr {
            return self.viewport.set_maps(None);
        }
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

    /// The Sun panel's values on the viewport.
    fn apply_sun(&mut self) {
        let [az, el, _, _, fog] = self.sun.0;
        self.viewport.set_sun(az as f32, el as f32);
        self.viewport.set_fog(fog as f32);
        self.viewport.set_rain(self.rain as f32);
    }

    /// The desktop's `setSun`: the viewport follows at once; the look is written 400 ms and the
    /// sky re-baked 150 ms after the edit settles.
    fn sun_edited(&mut self) {
        self.apply_sun();
        let now = Instant::now();
        if self.project.is_some() {
            self.look_due = Some(now + std::time::Duration::from_millis(400));
        }
        self.sky_due = Some(now + std::time::Duration::from_millis(150));
    }

    /// The desktop's `writeLook`: materials, scatter and sun in one `project.set_look`.
    fn write_look(&mut self) {
        let Some(p) = &self.project else { return };
        let look = &p["look"];
        let scatter = if look["scatter"].is_null() {
            json!({ "mask": null, "spacing_m": 40, "seed": 1, "grow": false })
        } else {
            look["scatter"].clone()
        };
        let [az, el, turbidity, clouds, fog] = self.sun.0;
        let params = json!({
            "materials": look["materials"].as_array().cloned().unwrap_or_default(),
            "scatter": scatter,
            "sun": { "azimuth_deg": az, "elevation_deg": el, "turbidity": turbidity, "clouds": clouds, "fog": fog },
        });
        self.invoke(Tag::User, "project.set_look", params);
    }

    /// Fire the debounced look write and sky bake when due.
    fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if self.look_due.is_some_and(|t| now >= t) {
            self.look_due = None;
            self.write_look();
        }
        if self.sky_due.is_some_and(|t| now >= t) {
            self.sky_due = None;
            self.fetch_sky(self.sun);
        }
        if let Some(next) = [self.look_due, self.sky_due].into_iter().flatten().min() {
            ctx.request_repaint_after(next.saturating_duration_since(now));
        }
    }

    /// The right column's Sun section (the desktop's, in its order).
    fn sun_card(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        use crate::widgets::Range;
        look::section(ui, "Sun");
        look::field(ui, "Textured (PBR)", |ui| {
            let mut on = self.pbr;
            look::checkbox(ui, &mut on, "Textured (PBR)").on_hover_text("Shade the terrain with baked albedo, normal and roughness maps (full-field relief at any preview LOD); off = height-ramp vertex colours");
            if on != self.pbr {
                self.pbr = on;
                self.fetch_maps();
            }
        });
        let mut edited = false;
        let [mut az, mut el, mut turbidity, mut clouds, mut fog] = self.sun.0;
        let mut rain = self.rain;
        let mut slider = |ui: &mut egui::Ui,
                          label: String,
                          v: &mut f64,
                          range: std::ops::RangeInclusive<f64>,
                          step: f64,
                          reset: f64,
                          tip: &str| {
            look::field(ui, &label, |ui| {
                let r = ui
                    .add(Range::new(v, range, &label).step(step).reset(reset))
                    .on_hover_text(tip);
                edited |= r.changed();
            });
        };
        slider(
            ui,
            format!("Azimuth {}°", number(&json!(az))),
            &mut az,
            0.0..=359.0,
            1.0,
            140.0,
            "Direction the light comes from, clockwise from north",
        );
        slider(
            ui,
            format!("Elevation {}°", number(&json!(el))),
            &mut el,
            3.0..=90.0,
            1.0,
            35.0,
            "Height of the sun above the horizon; low sun is warm and long-shadowed",
        );
        slider(
            ui,
            format!("Turbidity {}", number(&json!(turbidity))),
            &mut turbidity,
            2.0..=10.0,
            0.5,
            3.0,
            "Atmospheric haze for the sky dome: 2 clear mountain air, 3 clear, 6 hazy, 10 thick haze (Preetham daylight model)",
        );
        let mut hdri_action = None;
        let has_hdri = self.hdri.is_some();
        let can_load =
            self.project.is_some() && self.busy == 0 && !self.hdri_path.trim().is_empty();
        look::field(ui, "HDRI", |ui| {
            if look::button(ui, None, "×", false, has_hdri)
                .on_hover_text("Back to the physical sky")
                .clicked()
            {
                hdri_action = Some(0);
            }
            if look::button(ui, None, "Use sun", false, has_hdri)
                .on_hover_text("Point the sun where the HDRI is brightest")
                .clicked()
            {
                hdri_action = Some(1);
            }
            if look::button(ui, None, "Load", false, can_load)
                .on_hover_text("Load the HDRI onto the dome (sky.load_hdri)")
                .clicked()
            {
                hdri_action = Some(2);
            }
            look::input(ui, &mut self.hdri_path, 96.0, "", "HDRI");
        });
        // Post: viewport-only, never saved (the desktop keeps it in the session too).
        let post = self.viewport.post();
        let [mut exposure, mut bloom, mut vignette] =
            [post.exposure, post.bloom, post.vignette].map(f64::from);
        let mut post_edited = false;
        for (label, v, range, reset, tip) in [
            (
                "Exposure",
                &mut exposure,
                0.25..=4.0,
                1.0,
                "Tone-mapping exposure (ACES while any effect is on); 1 = the plain render",
            ),
            (
                "Bloom",
                &mut bloom,
                0.0..=1.0,
                0.0,
                "Bloom strength on the bright parts (sun glints, lava glow); 0 = off",
            ),
            (
                "Vignette",
                &mut vignette,
                0.0..=1.0,
                0.0,
                "Darken the frame's corners; 0 = off",
            ),
        ] {
            let label = format!("{label} {}", number(&json!((*v * 100.0).round() / 100.0)));
            look::field(ui, &label, |ui| {
                let mut r = Range::new(v, range, &label).step(0.05).reset(reset);
                if reset == 1.0 {
                    r = r.fill_from(1.0);
                }
                post_edited |= ui.add(r).on_hover_text(tip).changed();
            });
        }
        if post_edited {
            let r2 = |v: f64| ((v * 100.0).round() / 100.0) as f32;
            self.viewport.set_post(crate::post::Post {
                exposure: r2(exposure),
                bloom: r2(bloom),
                vignette: r2(vignette),
            });
        }
        self.camera_row(ui, frame);
        slider(
            ui,
            format!("Clouds {} %", (clouds * 100.0).round()),
            &mut clouds,
            0.0..=1.0,
            0.05,
            0.0,
            "Cloud cover on the sky dome and in the HDRI bake: a seeded cumulus-like noise layer, brighter on the sun side",
        );
        slider(
            ui,
            format!("Fog {} %", (fog * 100.0).round()),
            &mut fog,
            0.0..=1.0,
            0.05,
            0.0,
            "Viewport fog: 0 keeps the faint aerial perspective, 1 closes the view within the world's width (a look setting — bakes and exports are untouched)",
        );
        slider(
            ui,
            format!("Rain {} %", (rain * 100.0).round()),
            &mut rain,
            0.0..=1.0,
            0.05,
            0.0,
            "Rain falling in the viewport (look only); the rain itself is the jd.climate.rain node's field — wet surfaces come from a material on that layer",
        );
        match hdri_action {
            Some(0) => {
                self.hdri = None;
                self.viewport.set_sky(self.sky_img.clone());
            }
            Some(1) => {
                if let Some(h) = &self.hdri {
                    az = h.sun[0].round();
                    el = h.sun[1].round().clamp(3.0, 90.0);
                    edited = true;
                }
            }
            Some(2) => self.load_hdri(),
            _ => {}
        }
        if edited {
            // Snap to the slider's 0.05 steps the way the desktop's inputs report them.
            let r2 = |v: f64| (v * 100.0).round() / 100.0;
            self.sun = SunLook([
                az.round(),
                el.round(),
                (turbidity * 2.0).round() / 2.0,
                r2(clouds),
                r2(fog),
            ]);
            self.rain = r2(rain);
            self.sun_edited();
        }
    }

    /// The Sun section's Camera row: flythrough keys, frames, Record / Play / Still, and the key
    /// list (heading · elevation · distance, Go, remove).
    fn camera_row(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        enum Act {
            Add,
            Clear,
            Record,
            Play,
            Still,
            Go(usize),
            Remove(usize),
        }
        let mut act = None;
        let view = self.viewport.info().is_some();
        let n = self.fly_keys.len();
        let path = view && n >= 2;
        let (idle, busy) = (self.fly.is_none(), self.busy > 0);
        let can_still = view && self.project.is_some();
        let frames = &mut self.fly_frames;
        look::field(ui, "Camera", |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    if look::button(ui, None, "+ key", false, view)
                        .on_hover_text("Add the current view as a flythrough key")
                        .clicked()
                    {
                        act = Some(Act::Add);
                    }
                    ui.label(RichText::new(format!("{n} keys")).size(12.0).color(color::FG2));
                    let clear = look::button(ui, None, "×", false, n > 0)
                        .on_hover_text("Drop the flythrough keys");
                    clear.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::accesskit::Role::Button,
                            n > 0,
                            "Drop the flythrough keys",
                        )
                    });
                    if clear.clicked() {
                        act = Some(Act::Clear);
                    }
                    look::input(ui, frames, 56.0, "", "Frames")
                        .on_hover_text("Frames to render along the keys");
                });
                ui.horizontal(|ui| {
                    if look::button(ui, None, "Record", false, path && idle && !busy)
                        .on_hover_text("Render the frames along the keys into exports/flythrough/ (then: ffmpeg -framerate 24 -i frame_%04d.png flythrough.mp4)")
                        .clicked()
                    {
                        act = Some(Act::Record);
                    }
                    if look::button(ui, None, "Play", false, path && idle)
                        .on_hover_text("Play the path in the viewport (4 s, no frames written)")
                        .clicked()
                    {
                        act = Some(Act::Play);
                    }
                    if look::button(ui, None, "Still", false, can_still)
                        .on_hover_text("Save the current frame as exports/stills/view_<n>.png")
                        .clicked()
                    {
                        act = Some(Act::Still);
                    }
                });
            });
        });
        for (i, k) in self.fly_keys.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.add_sized(
                    [16.0, 16.0],
                    egui::Label::new(
                        RichText::new(format!("{}", i + 1))
                            .size(11.0)
                            .color(color::FG2),
                    ),
                );
                ui.label(RichText::new(fly_key_text(k)).size(11.0).color(color::FG1));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = format!("Remove key {}", i + 1);
                    let remove = ui
                        .add(egui::Button::new(
                            RichText::new("×").size(11.0).color(color::ERROR),
                        ))
                        .on_hover_text("Remove this key");
                    remove.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, &label)
                    });
                    if remove.clicked() {
                        act = Some(Act::Remove(i));
                    }
                    if ui
                        .add(egui::Button::new(
                            RichText::new("Go").size(11.0).color(color::FG0),
                        ))
                        .on_hover_text("Move the camera to this key")
                        .clicked()
                    {
                        act = Some(Act::Go(i));
                    }
                });
            });
        }
        match act {
            Some(Act::Add) => self.fly_keys.push(self.viewport.camera),
            Some(Act::Clear) => self.fly_keys.clear(),
            Some(Act::Record) => self.start_record(),
            Some(Act::Play) => self.fly = Some(Fly::Play(Instant::now())),
            Some(Act::Still) => self.save_still(frame),
            Some(Act::Go(i)) => self.viewport.set_camera(self.fly_keys[i]),
            Some(Act::Remove(i)) => {
                self.fly_keys.remove(i);
            }
            None => {}
        }
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
        let png = match self.png(frame) {
            Ok(png) => png,
            Err(e) => return self.log(format!("still: {e}"), Level::Error),
        };
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

    /// The viewport's last rendered frame as PNG bytes.
    fn png(&self, frame: &eframe::Frame) -> Result<Vec<u8>, String> {
        let rs = frame.wgpu_render_state().ok_or("no renderer")?;
        let (w, h, rgba) = self
            .viewport
            .capture(rs)
            .ok_or("the viewport gave no image")?;
        let img = image::RgbaImage::from_raw(w, h, rgba).ok_or("bad frame size")?;
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        Ok(png)
    }

    /// Record: the frames along the keys into exports/flythrough/frame_NNNN.png, written in order
    /// by one worker that logs the desktop's summary line when the last frame is in.
    fn start_record(&mut self) {
        let frames = self.fly_frames.trim().parse().unwrap_or(2).max(2);
        let (tx, rx) = mpsc::channel::<(usize, Vec<u8>)>();
        let (server, out, ctx) = (self.server.clone(), self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            use base64::Engine as _;
            let mut written = 0;
            for (k, png) in rx {
                let r = server.invoke(Self::request(
                    "project.write_export",
                    json!({ "path": format!("flythrough/frame_{k:04}.png"), "bytes_base64": base64::engine::general_purpose::STANDARD.encode(&png) }),
                ));
                written += usize::from(r.result.is_some_and(|v| v["path"].is_string()));
            }
            let level = if written == frames {
                Level::Info
            } else {
                Level::Warn
            };
            let _ = out.send(Msg::Log(
                format!("flythrough: {written}/{frames} frames → exports/flythrough/ (ffmpeg -framerate 24 -i frame_%04d.png …)"),
                level,
            ));
            ctx.request_repaint();
        });
        self.fly = Some(Fly::Record {
            k: 0,
            frames,
            start: self.viewport.camera,
            tx,
        });
    }

    /// Before the viewport renders: the camera for this Play instant or Record frame.
    fn fly_tick(&mut self, ctx: &egui::Context) {
        let t = match &self.fly {
            None => return,
            Some(Fly::Play(started)) => started.elapsed().as_secs_f32() / 4.0,
            Some(Fly::Record { k, frames, .. }) => *k as f32 / (*frames - 1) as f32,
        };
        if let Some(c) = Camera::along(&self.fly_keys, t) {
            self.viewport.set_camera(c);
        }
        if matches!(self.fly, Some(Fly::Play(_))) && t >= 1.0 {
            self.fly = None;
        }
        ctx.request_repaint();
    }

    /// After the viewport rendered a Record frame: hand it to the writer and step on; after the
    /// last one the camera goes back and dropping the sender lets the writer finish.
    fn record_frame(&mut self, frame: &eframe::Frame) {
        if !matches!(self.fly, Some(Fly::Record { .. })) {
            return;
        }
        let Some(Fly::Record {
            k,
            frames,
            start,
            tx,
        }) = self.fly.take()
        else {
            return;
        };
        match self.png(frame) {
            Ok(png) => {
                let _ = tx.send((k, png));
            }
            Err(e) => self.log(format!("flythrough: frame {k}: {e}"), Level::Warn),
        }
        if k + 1 < frames {
            self.fly = Some(Fly::Record {
                k: k + 1,
                frames,
                start,
                tx,
            });
        } else {
            self.viewport.set_camera(start);
        }
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
                // The readout takes what the toolbar and the layer picker leave, truncated with an
                // ellipsis rather than running under the toolbar.
                let picker = if self.layers.len() > 1 { 200.0 } else { 0.0 };
                let room = (ui.available_width() - picker).max(0.0);
                let info = self.viewport_info();
                ui.allocate_ui(egui::vec2(room, ui.available_height()), |ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(info).size(12.0).color(color::FG2))
                                .truncate(),
                        );
                    });
                });
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

    /// The desktop's viewport readout, in its order: tris · sea · waves · fog · sun · pbr · sky ·
    /// box · shore · horizon · preview.
    fn viewport_info(&self) -> String {
        let tris = self.viewport.info().map_or(0, |i| i.triangles);
        let mut s = format!("viewport: wgpu · {} tris", thousands(tris));
        if let Some((level, waves)) = self.viewport.sea_info() {
            s += &format!(" · sea {} m", number(&json!(level)));
            if let Some([h, len]) = waves {
                s += &format!(" · waves {h:.1} m / {} m", len.round());
            }
        }
        let [az, el, _, _, fog] = self.sun.0;
        if fog > 0.0 {
            s += &format!(" · fog {} %", (fog * 100.0).round());
        }
        s += &format!(" · sun {}°/{}°", number(&json!(el)), number(&json!(az)));
        if self.viewport.lake_cells() > 0 {
            s += &format!(
                " · lakes {} cells",
                thousands(self.viewport.lake_cells() as u64)
            );
        }
        if let Some([w, h]) = self.viewport.maps_px() {
            s += &format!(" · pbr {w}×{h}");
        }
        if let Some([w, h]) = self.viewport.sky_px() {
            s += &format!(" · sky {w}×{h}");
        }
        if self.hdri.is_some() {
            s += " · hdri";
        }
        if self.viewport.frame && self.viewport.info().is_some() {
            s += " · box";
        }
        if let Some(rain) = self.viewport.rain() {
            s += &format!(" · rain {} %", (rain * 100.0).round());
        }
        if self.viewport.shore_segments() > 0 {
            s += &format!(
                " · shore {}",
                thousands(self.viewport.shore_segments() as u64)
            );
        }
        if let Some(post) = self.viewport.post().info() {
            s += &format!(" · post {post}");
        }
        if let Some(px) = self.viewport.horizon_px() {
            s += &format!(" · horizon {px}²");
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
            self.open_editor();
        }
    }

    fn chrome_action(
        &mut self,
        action: crate::chrome::Action,
        ctx: &egui::Context,
        frame: &eframe::Frame,
    ) {
        use crate::chrome::Action;
        match action {
            Action::Fit => self.viewport.fit(),
            Action::Still => self.save_still(frame),
            Action::Fullscreen => self.set_fullscreen(ctx, !self.fullscreen),
            Action::Frame => self.viewport.set_frame(!self.viewport.frame),
            Action::Graph => self.open_editor(),
            Action::Horizon => {
                self.show_horizon = !self.show_horizon;
                if self.show_horizon {
                    self.fetch_horizon();
                } else {
                    self.viewport.set_horizon(None);
                }
            }
            Action::Insert(k) => self.add_layer(crate::chrome::STRIP[k].3),
        }
    }

    /// The desktop's `previewLod`: live evaluations stay ≤ 512² (its adaptive bias arrives with
    /// live evaluation in Studio).
    fn preview_lod(&self) -> u32 {
        let res = self.project.as_ref().map_or(512.0, |p| {
            let r = &p["world"]["resolution_px"];
            r[0].as_f64()
                .unwrap_or(512.0)
                .max(r[1].as_f64().unwrap_or(512.0))
        });
        (res / 512.0).log2().ceil().max(0.0) as u32
    }

    /// A layer-strip button: the shared workflow op (as the desktop), then a live evaluation.
    fn add_layer(&mut self, add: crate::chrome::Add) {
        use crate::chrome::Add;
        let max = self
            .viewport
            .info()
            .map_or(500.0, |i| f64::from(i.range_m[1]));
        let request = match add {
            Add::Splice(kind) => {
                let mut params = json!({ "layer": "height", "kind": kind });
                if kind == "jd.shape.primitive@1" {
                    let e = self.project.as_ref().map_or([1000.0, 1000.0], |p| {
                        let x = &p["world"]["extent_m"];
                        [
                            x[0].as_f64().unwrap_or(1000.0),
                            x[1].as_f64().unwrap_or(1000.0),
                        ]
                    });
                    params["params"] = json!({ "shape": "cone", "center_m": [e[0] / 2.0, e[1] / 2.0], "size_m": [e[0] / 3.0, e[1] / 3.0], "height_range_m": [0.0, (max / 3.0).max(50.0)], "op": "max" });
                }
                Self::request("graph.splice", params)
            }
            Add::Simulation(which) => Self::request(
                "graph.add_simulation",
                json!({ "which": which, "max_height_m": max }),
            ),
            Add::Cave(kind) => Self::request("graph.add_cave", json!({ "kind": kind })),
        };
        let lod = self.preview_lod();
        self.chain(vec![
            Box::new(move |_| Some(request)),
            Box::new(move |_| Some(Self::request("graph.evaluate", json!({ "lod": lod })))),
        ]);
    }

    fn open_editor(&mut self) {
        match &mut self.editor {
            Some(e) if !matches!(e.status, editor::Status::Running(_)) => e.reopen(),
            Some(_) => {}
            None => match editor::locate(None) {
                Ok(path) => self.start_editor(path),
                Err(msg) => {
                    self.log(msg.clone(), Level::Error);
                    self.editor_error = Some(msg);
                }
            },
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
/// A flythrough key as the desktop lists it: `326° · 33° up · 2.7 km`.
fn fly_key_text(k: &Camera) -> String {
    let heading = (360.0 - k.theta.to_degrees()).rem_euclid(360.0).round();
    let up = (90.0 - k.phi.to_degrees()).round();
    let distance = if k.radius >= 1000.0 {
        format!("{:.1} km", k.radius / 1000.0)
    } else {
        format!("{} m", k.radius.round())
    };
    format!("{heading}° · {up}° up · {distance}")
}

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
        self.tick(ui.ctx());
        self.fly_tick(ui.ctx());
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
            egui::Panel::right("panel-right")
                .resizable(true)
                .default_size(292.0)
                .size_range(232.0..=572.0)
                .frame(bg.inner_margin(Margin {
                    left: 4,
                    right: 8,
                    top: 8,
                    bottom: 8,
                }))
                .show(ui, |ui| {
                    look::card().show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        ScrollArea::vertical()
                            .id_salt("panel-right")
                            .auto_shrink(false)
                            .show(ui, |ui| self.sun_card(ui, frame));
                    });
                });
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
            .show(ui, |ui| {
                self.viewport.show(ui, frame.wgpu_render_state());
                self.record_frame(frame);
                let cam = self.viewport.camera;
                let st = crate::chrome::State {
                    // The desktop's heading: the camera sits at θ from south towards east.
                    heading: (360.0 - cam.theta.to_degrees()).rem_euclid(360.0),
                    distance_m: cam.radius,
                    range_m: self.viewport.info().map(|i| i.range_m),
                    sea_m: self.viewport.sea_info().map(|(level, _)| level),
                    frame: self.viewport.frame,
                    horizon: self.show_horizon,
                    enabled: self.project.is_some() && self.busy == 0,
                    can_still: self.project.is_some() && self.viewport.info().is_some(),
                };
                if let Some(action) = crate::chrome::show(ui, self.viewport.rect(), &st) {
                    self.chrome_action(action, ui.ctx(), frame);
                }
            });
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
