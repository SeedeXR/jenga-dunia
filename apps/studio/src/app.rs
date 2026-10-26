//! Studio's window (ADR-0017, Phase 2): the engine server, the viewport, the Project, Terrain,
//! Layers and Editor panels, and the log. Every operation runs on a worker thread through the
//! same `Server::invoke` the Editor's calls use (client `gui-studio`), so Studio's own writes
//! are announced to the Editor and the Editor's writes refresh the viewport.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

use eframe::egui::{self, RichText};
use jd_core::{Request, Response, Status};
use jd_ipc::Server;
use serde_json::{Value, json};

use crate::editor::{self, Editor};
use crate::theme::color;
use crate::viewport::Viewport;

/// The viewport mesh keeps at most this many samples along a side (≈ 0.5 M triangles).
const PREVIEW_MAX_PX: u32 = 512;
/// Log lines kept.
const LOG_LINES: usize = 500;

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
    Layers,
    Stats(String),
    Preview([f32; 2]),
}

enum Msg {
    Reply {
        tag: Tag,
        response: Box<Response>,
        ms: u64,
    },
    Event {
        topic: String,
        data: Value,
    },
    EditorExited(std::process::ExitStatus),
}

/// One log line.
#[derive(Debug, Clone)]
pub struct LogLine {
    /// Seconds since Studio started.
    pub at_s: f32,
    /// The text.
    pub text: String,
    /// Shown in the error colour.
    pub error: bool,
}

/// The Studio application.
pub struct StudioApp {
    server: Arc<Server>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: egui::Context,
    started: Instant,
    project: Option<Value>,
    layers: Vec<String>,
    layer: Option<String>,
    /// The 3-D view.
    pub viewport: Viewport,
    log: VecDeque<LogLine>,
    in_flight: usize,
    refreshing: bool,
    refresh_again: bool,
    path_input: String,
    name_input: String,
    editor: Option<Editor>,
    editor_error: Option<String>,
}

impl StudioApp {
    /// Build the app around a bound engine server; installs the theme, starts the Editor when
    /// asked, and loads whatever project the engine has open.
    pub fn new(cc: &eframe::CreationContext<'_>, server: Arc<Server>, options: Options) -> Self {
        crate::theme::install(&cc.egui_ctx);
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
            started: Instant::now(),
            project: None,
            layers: Vec::new(),
            layer: None,
            viewport: Viewport::default(),
            log: VecDeque::new(),
            in_flight: 0,
            refreshing: false,
            refresh_again: false,
            path_input: String::new(),
            name_input: "World".into(),
            editor: None,
            editor_error: None,
        };
        app.log(
            format!("Engine listening (session {}).", app.server.session()),
            false,
        );
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

    /// Engine calls still running.
    pub fn busy(&self) -> bool {
        self.in_flight > 0 || self.refreshing
    }

    /// The log, oldest first.
    pub fn log_lines(&self) -> impl Iterator<Item = &LogLine> {
        self.log.iter()
    }

    fn log(&mut self, text: String, error: bool) {
        if self.log.len() == LOG_LINES {
            self.log.pop_front();
        }
        self.log.push_back(LogLine {
            at_s: self.started.elapsed().as_secs_f32(),
            text,
            error,
        });
    }

    fn start_editor(&mut self, path: PathBuf) {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let editor = Editor::start(path, self.server.socket(), self.server.token(), move |s| {
            let _ = tx.send(Msg::EditorExited(s));
            ctx.request_repaint();
        });
        if let editor::Status::Failed(msg) = &editor.status {
            self.log(msg.clone(), true);
        } else {
            self.log("Editor started.".into(), false);
        }
        self.editor = Some(editor);
    }

    /// Run an operation on a worker thread; its reply comes back as a message.
    fn invoke(&mut self, tag: Tag, op: &str, params: Value) {
        let mut request = Request::new(op, params);
        request.client.get_or_insert_default().kind = "gui-studio".into();
        let (server, tx, ctx) = (self.server.clone(), self.tx.clone(), self.ctx.clone());
        let spawned = std::thread::Builder::new()
            .name("studio-op".into())
            .spawn(move || {
                let started = Instant::now();
                let response = Box::new(server.invoke(request));
                let ms = started.elapsed().as_millis() as u64;
                let _ = tx.send(Msg::Reply { tag, response, ms });
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.in_flight += 1,
            Err(e) => self.log(format!("{op}: could not start a worker thread: {e}"), true),
        }
    }

    /// Re-read the project, its layers and the selected layer's preview (coalesced: a refresh
    /// requested while one runs runs once more afterwards).
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
                Msg::Reply { tag, response, ms } => {
                    self.in_flight = self.in_flight.saturating_sub(1);
                    self.on_reply(tag, *response, ms);
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
                        let crashed = !status.success();
                        self.log(line, crashed);
                    }
                }
            }
        }
    }

    fn on_reply(&mut self, tag: Tag, r: Response, ms: u64) {
        let failed = r.status == Status::Error;
        if let Some(e) = &r.error
            && !matches!(tag, Tag::Info)
        {
            let next = if e.next.is_empty() {
                String::new()
            } else {
                format!(" {}", e.next)
            };
            self.log(
                format!("{}: {} {}.{next}", r.op, e.code.as_str(), e.message),
                true,
            );
        }
        match tag {
            Tag::User => {
                if !failed {
                    self.log(format!("{}: ok ({ms} ms)", r.op), false);
                }
                self.refresh();
            }
            Tag::Info => {
                if failed {
                    // No project open: nothing to show.
                    self.project = None;
                    self.layers.clear();
                    self.viewport.clear();
                    return self.refresh_done();
                }
                let info = r.result.unwrap_or_default();
                let sun = &info["look"]["sun"];
                if let (Some(az), Some(el)) =
                    (sun["azimuth_deg"].as_f64(), sun["elevation_deg"].as_f64())
                {
                    self.viewport.set_sun(az as f32, el as f32);
                }
                if self.path_input.is_empty() {
                    self.path_input = info["path"].as_str().unwrap_or_default().to_owned();
                }
                self.project = Some(info);
                self.invoke(Tag::Layers, "terrain.layers", json!({}));
            }
            Tag::Layers => {
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
                        self.refresh_done();
                    }
                }
            }
            Tag::Stats(layer) => {
                let Some(size) = r.result.as_ref().map(|v| &v["size_px"]) else {
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
                    Tag::Preview(extent),
                    "terrain.preview",
                    json!({ "layer": layer, "lod": lod }),
                );
            }
            Tag::Preview(extent) => {
                if let Some(v) = &r.result {
                    match decode_preview(v) {
                        Ok(pv) => self.viewport.set_terrain(&pv, extent),
                        Err(e) => self.log(format!("terrain.preview: {e}"), true),
                    }
                }
                self.refresh_done();
            }
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Jenga Dunia Studio")
                    .strong()
                    .color(color::FG0),
            );
            if let Some(p) = &self.project {
                ui.separator();
                ui.label(p["name"].as_str().unwrap_or_default());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.busy() {
                    ui.spinner();
                    ui.label(RichText::new("Working").color(color::FG2));
                }
            });
        });
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Project");
            match self.project.clone() {
                Some(p) => {
                    let w = &p["world"];
                    egui::Grid::new("project").num_columns(2).show(ui, |ui| {
                        for (k, v) in [
                            ("Name", p["name"].as_str().unwrap_or_default().to_owned()),
                            ("Path", p["path"].as_str().unwrap_or_default().to_owned()),
                            (
                                "Extent",
                                format!(
                                    "{:.0} x {:.0} m",
                                    w["extent_m"][0].as_f64().unwrap_or(0.0),
                                    w["extent_m"][1].as_f64().unwrap_or(0.0)
                                ),
                            ),
                            (
                                "Resolution",
                                format!("{} x {} px", w["resolution_px"][0], w["resolution_px"][1]),
                            ),
                            ("Seed", w["seed"].to_string()),
                        ] {
                            ui.label(RichText::new(k).color(color::FG2));
                            ui.label(v);
                            ui.end_row();
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            self.invoke(Tag::User, "project.save", json!({}));
                        }
                        if ui.button("Close").clicked() {
                            self.invoke(Tag::User, "project.close", json!({}));
                        }
                    });
                }
                None => {
                    ui.label(RichText::new("No project open.").color(color::FG2));
                }
            }
            ui.add_space(6.0);
            let label = ui.label("Project path");
            ui.text_edit_singleline(&mut self.path_input)
                .labelled_by(label.id);
            let label = ui.label("Name for a new project");
            ui.text_edit_singleline(&mut self.name_input)
                .labelled_by(label.id);
            ui.horizontal(|ui| {
                let path = self.path_input.trim().to_owned();
                if ui
                    .add_enabled(!path.is_empty(), egui::Button::new("Open"))
                    .clicked()
                {
                    self.invoke(Tag::User, "project.open", json!({ "path": path }));
                }
                if ui
                    .add_enabled(!path.is_empty(), egui::Button::new("New"))
                    .clicked()
                {
                    let name = self.name_input.trim().to_owned();
                    self.invoke(
                        Tag::User,
                        "project.create",
                        json!({ "path": path, "name": name }),
                    );
                }
            });

            ui.separator();
            ui.heading("Terrain");
            ui.add_enabled_ui(self.project.is_some(), |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Generate terrain").clicked() {
                        self.invoke(Tag::User, "terrain.generate", json!({}));
                    }
                    if ui.button("Evaluate graph").clicked() {
                        let lod = self
                            .project
                            .as_ref()
                            .and_then(|p| p["settings"]["preview_lod"].as_u64())
                            .unwrap_or(0);
                        self.invoke(Tag::User, "graph.evaluate", json!({ "lod": lod }));
                    }
                });
                let mut selected = self.layer.clone();
                egui::ComboBox::from_label("Layer")
                    .selected_text(selected.clone().unwrap_or_else(|| "none".into()))
                    .show_ui(ui, |ui| {
                        for l in &self.layers {
                            ui.selectable_value(&mut selected, Some(l.clone()), l);
                        }
                    });
                if selected != self.layer {
                    self.layer = selected;
                    self.refresh();
                }
            });

            ui.separator();
            ui.heading("Editor");
            let status = match self.editor.as_ref().map(|e| &e.status) {
                Some(editor::Status::Running(pid)) => format!("Running (process {pid})"),
                Some(editor::Status::Closed) => "Closed".into(),
                Some(editor::Status::GaveUp(m) | editor::Status::Failed(m)) => m.clone(),
                None => self
                    .editor_error
                    .clone()
                    .unwrap_or_else(|| "Not started".into()),
            };
            ui.label(RichText::new(status).color(color::FG1));
            let running = matches!(
                self.editor.as_ref().map(|e| &e.status),
                Some(editor::Status::Running(_))
            );
            if ui
                .add_enabled(!running, egui::Button::new("Open Editor"))
                .clicked()
            {
                match &mut self.editor {
                    Some(e) => e.reopen(),
                    None => match editor::locate(None) {
                        Ok(path) => self.start_editor(path),
                        Err(msg) => {
                            self.log(msg.clone(), true);
                            self.editor_error = Some(msg);
                        }
                    },
                }
            }
        });
    }

    fn log_panel(&mut self, ui: &mut egui::Ui) {
        let row = ui.text_style_height(&egui::TextStyle::Body);
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .auto_shrink(false)
            .show_rows(ui, row, self.log.len(), |ui, range| {
                for line in self.log.range(range) {
                    let c = if line.error { color::ERROR } else { color::FG1 };
                    ui.label(
                        RichText::new(format!("{:>7.1} s  {}", line.at_s, line.text)).color(c),
                    );
                }
            });
    }
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

fn decode_preview(v: &Value) -> Result<jd_terrain::preview::Preview, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(v["bytes_base64"].as_str().unwrap_or_default())
        .map_err(|e| format!("bad base64: {e}"))?;
    jd_terrain::preview::Preview::from_bytes(&bytes)
        .ok_or_else(|| "the preview bytes are not in the wire format".to_owned())
}

impl eframe::App for StudioApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.drain();
        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("log")
            .resizable(true)
            .default_size(150.0)
            .show(ui, |ui| self.log_panel(ui));
        egui::Panel::left("side")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| self.side_panel(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.viewport.show(ui, frame.wgpu_render_state()));
    }

    fn on_exit(&mut self) {
        self.server.shutdown("Studio closed");
        if let Some(editor) = &mut self.editor {
            editor.stop(std::time::Duration::from_secs(3));
        }
    }
}
