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

use eframe::egui::{self, Align, Layout, Margin, RichText, ScrollArea, Stroke, vec2};
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
/// The viewport preview's longest side by default (Settings ▸ Preview detail: 1024² or 2048²).
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
    /// `terrain.scatter` for the viewport (points inline).
    Scatter,
    /// `terrain.measure` for the Noise section's readout.
    Measure,
    /// `media.metadata` for the Reference image section.
    Reference,
    /// A Diagnostics button's op (`diag.env`, `project.cache_gc`, `diag.bench`).
    Diag,
    /// `jobs.list` while Studio waits on the engine (the Tasks section).
    Jobs,
    /// `graph.kinds`, once (the Layers section's `+`).
    Kinds,
    /// `graph.sculpt_target` for the brush: its strokes node, then sculpting.
    SculptTarget,
    /// An export or bake: busy while it runs and logged, as a button's op, but nothing on screen
    /// changed, so no refresh (the desktop's `run`).
    Export,
    /// Studio's own `project.set_look`: logged, then the project re-read (no terrain changed;
    /// the desktop's `writeLook` only logs).
    Look,
}

type Step = Box<dyn FnOnce(&[Response]) -> Option<Request> + Send>;

type CaveMeshes = Vec<(
    String,
    Result<(crate::viewport::CaveMesh, CaveSummary), String>,
)>;

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
    /// `caves.mesh` for every cave node (fetch generation, then per node its id and the mesh with
    /// its summary, or the error).
    Caves(u64, CaveMeshes),
    /// The colour layer's preview (`name`, then the preview or the error).
    Color(String, Result<jd_terrain::preview::Preview, String>),
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
    /// The preview LOD bias the Live target drives (the desktop's `previewLodBias`).
    lod_bias: u32,
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
    /// The Scatter section (the look's `scatter`), the points placed from it, and the model field.
    scatter: ScatterLook,
    scatter_points: Option<Vec<[f32; 5]>>,
    scatter_model: String,
    /// The toolbar's colour layer (the desktop's `colorLayer`): drawn on the terrain instead of
    /// the height ramp; and one chosen before it is published (a simulation layer just added),
    /// adopted by the evaluation that publishes it.
    color_layer: Option<String>,
    pending_color: Option<String>,
    /// The Noise section: its settings (session-only, as the desktop's), a Strength or octave
    /// bar being dragged (applied on release), the shown layer's `terrain.stats`, and the last
    /// `terrain.measure`.
    noise: NoisePanel,
    noise_drag: NoiseDrag,
    /// The Export section's fields (session-only, as the desktop's).
    export: ExportPanel,
    /// App settings (remembered between runs, as the desktop's browser store): the viewport
    /// preview's longest side and the live preview's target per evaluation (ms, as typed).
    preview_max_px: u32,
    preview_target_ms: String,
    /// The Tasks section's jobs (`jobs.list`) and when they were last polled; the Diagnostics
    /// section's environment, cache GC and bench results.
    jobs: Vec<Value>,
    jobs_polled: Option<Instant>,
    diag: [Option<Value>; 3],
    /// The Layers section: the node kinds (`graph.kinds`), its add / search / collapsed state, and
    /// the node selected by a row (sent to the Editor as `selection`).
    kinds: Vec<Value>,
    stack: StackPanel,
    selected_node: Option<String>,
    /// Real world's fields, and the Reference image section's path and `media.metadata`.
    geo: GeoPanel,
    reference_path: String,
    reference: Option<Value>,
    layer_stats: Option<Value>,
    measure: Option<Value>,
    /// The material stack (the look's `materials`, the desktop's `materials`): mask, albedo,
    /// emissive, roughness, optional detail tile — as JSON, the shape the engine reads.
    materials: Vec<Value>,
    /// The Caves section: the kind to add, Show caves, X-ray, each cave node's `caves.mesh`
    /// summary, and the generation of the newest mesh fetch (older ones are dropped).
    cave_kind: usize,
    /// The cave whose entrances a click on the terrain places (the viewport's tool is armed),
    /// and whether the cave being added is a tunnel (placing starts at once).
    tunnel_pending: bool,
    /// The viewport's ground tool (sculpt, road, cave entrances), the brush, the strokes node it
    /// paints into, and the grid tool's state.
    drawing: Option<Drawing>,
    brush: Brush,
    sculpt_node: Option<String>,
    grid: GridPanel,
    show_caves: bool,
    xray: f64,
    caves: Vec<CaveRow>,
    cave_gen: u64,
}

/// The Noise panel's fields as typed (the desktop's `defaultNoise`: open_simplex2 fbm, seed 0,
/// 1024 m, 6 octaves, lacunarity 2, gain 0.5, 0..500 m).
#[derive(Debug, Clone, PartialEq)]
struct NoisePanel {
    kind: String,
    fractal: String,
    seed: String,
    feature_size_m: String,
    octaves: String,
    lacunarity: String,
    gain: String,
    height_min: String,
    height_max: String,
}

impl Default for NoisePanel {
    fn default() -> Self {
        Self {
            kind: "open_simplex2".into(),
            fractal: "fbm".into(),
            seed: "0".into(),
            feature_size_m: "1024".into(),
            octaves: "6".into(),
            lacunarity: "2".into(),
            gain: "0.5".into(),
            height_min: "0".into(),
            height_max: "500".into(),
        }
    }
}

/// A noise preset: label, fractal, feature size, octaves, gain, height range.
type NoisePreset = (&'static str, &'static str, f64, u32, f64, [f64; 2]);

/// The desktop's `NOISE_PRESETS`.
const NOISE_PRESETS: [NoisePreset; 4] = [
    ("Mountains 0..500 m", "fbm", 1024.0, 6, 0.5, [0.0, 500.0]),
    (
        "Ridged alpine 0..1200 m",
        "ridged",
        1400.0,
        7,
        0.5,
        [0.0, 1200.0],
    ),
    (
        "Rolling hills 0..150 m",
        "fbm",
        800.0,
        5,
        0.45,
        [0.0, 150.0],
    ),
    ("Desert plain 0..40 m", "fbm", 1200.0, 3, 0.5, [0.0, 40.0]),
];

impl NoisePanel {
    fn apply(
        &mut self,
        (_, fractal, feature, octaves, gain, range): (&str, &str, f64, u32, f64, [f64; 2]),
    ) {
        self.fractal = fractal.into();
        self.feature_size_m = number(&json!(feature));
        self.octaves = octaves.to_string();
        self.gain = number(&json!(gain));
        self.height_min = number(&json!(range[0]));
        self.height_max = number(&json!(range[1]));
    }

    /// The noise as the engine takes it (a field that does not parse is 0, as the desktop's
    /// `Number`).
    fn json(&self) -> Value {
        let f = |s: &str| s.trim().parse::<f64>().unwrap_or(0.0);
        json!({
            "kind": self.kind,
            "fractal": self.fractal,
            "seed": self.seed.trim().parse::<i64>().unwrap_or(0),
            "feature_size_m": f(&self.feature_size_m),
            "octaves": self.octaves.trim().parse::<u32>().unwrap_or(0),
            "lacunarity": f(&self.lacunarity),
            "gain": f(&self.gain),
            "height_range_m": [f(&self.height_min), f(&self.height_max)],
        })
    }
}

/// The Export section's fields: heightmap file, holes layer, engine and tiles, mesh file, HDRI
/// file, timeline frames (the desktop's defaults: height.exr, none, Unity × 1, terrain.glb,
/// sky.exr, 24).
#[derive(Debug, Clone)]
struct ExportPanel {
    path: String,
    holes: Option<String>,
    unreal: bool,
    tiles: String,
    mesh: String,
    sky: String,
    frames: String,
}

impl Default for ExportPanel {
    fn default() -> Self {
        Self {
            path: "height.exr".into(),
            holes: None,
            unreal: false,
            tiles: "1".into(),
            mesh: "terrain.glb".into(),
            sky: "sky.exr".into(),
            frames: "24".into(),
        }
    }
}

/// The Layers section's own state: the kind `+` adds, the search text, collapsed groups.
#[derive(Debug, Clone)]
struct StackPanel {
    kind: String,
    query: String,
    closed: std::collections::HashSet<String>,
}

impl Default for StackPanel {
    fn default() -> Self {
        Self {
            kind: "jd.erosion.thermal@1".into(),
            query: String::new(),
            closed: std::collections::HashSet::new(),
        }
    }
}

/// The desktop's `categoryOf`: the layer stack's badge for a node kind.
fn category_of(kind: &str) -> &'static str {
    match kind.split('.').nth(1).unwrap_or_default() {
        "noise" | "input" | "shape" => "shape",
        "erosion" | "climate" | "volcanic" | "hydrology" | "ocean" => "simulation",
        "analysis" | "mask" | "biome" => "mask",
        "output" => "output",
        _ => "filter",
    }
}

/// The desktop's `stackOf`: the nodes feeding the height output through their `in` ports,
/// nearest first (the chain), and every other node but the outputs.
fn stack_of(nodes: &[Value], edges: &[Value]) -> (Vec<Value>, Vec<Value>) {
    let output = nodes.iter().find(|n| {
        n["kind"]
            .as_str()
            .is_some_and(|k| k.starts_with("jd.output.height@"))
            && n["params"]["layer"].as_str().unwrap_or("height") == "height"
    });
    let mut chain = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut cur = output.cloned();
    while let Some(c) = cur {
        let id = c["id"].as_str().unwrap_or_default().to_owned();
        seen.insert(id.clone());
        let feed = edges
            .iter()
            .find(|e| e["to"]["node"] == id.as_str() && e["to"]["port"] == "in");
        let prev = feed.and_then(|f| nodes.iter().find(|n| n["id"] == f["from"]["node"]));
        cur = match prev {
            Some(p) if !seen.contains(p["id"].as_str().unwrap_or_default()) => {
                chain.push(p.clone());
                Some(p.clone())
            }
            _ => None,
        };
    }
    let others = nodes
        .iter()
        .filter(|n| {
            !seen.contains(n["id"].as_str().unwrap_or_default())
                && !n["kind"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("jd.output.")
        })
        .cloned()
        .collect();
    (chain, others)
}

/// `jd.erosion.thermal@1` → `erosion.thermal`, as the desktop lists kinds.
fn short_kind(kind: &str) -> String {
    let rest = kind.split_once('.').map_or(kind, |(_, r)| r);
    rest.split('@').next().unwrap_or(rest).to_owned()
}

/// A row's name: the node's label, else its short kind.
fn label_of(n: &Value) -> String {
    n["ui"]["label"].as_str().map_or_else(
        || short_kind(n["kind"].as_str().unwrap_or_default()),
        str::to_owned,
    )
}

/// The desktop's blend readout: a blend node's op · opacity · the layer wired into `b`, a placed
/// shape's op × opacity.
fn readout_of(n: &Value, nodes: &[Value], edges: &[Value]) -> Option<String> {
    let kind = n["kind"].as_str().unwrap_or_default();
    let pct = |v: &Value| format!("{} %", (v.as_f64().unwrap_or(1.0) * 100.0).round());
    let id = n["id"].as_str().unwrap_or_default();
    for op in ["min", "max", "lerp"] {
        if kind.starts_with(&format!("jd.math.{op}@")) {
            let src = edges
                .iter()
                .find(|e| e["to"]["node"] == id && e["to"]["port"] == "b")
                .and_then(|e| nodes.iter().find(|x| x["id"] == e["from"]["node"]));
            let tail = src.map_or_else(
                || " · b unwired".to_owned(),
                |s| format!(" · {}", label_of(s)),
            );
            return Some(format!("{op} · {}{tail}", pct(&n["params"]["opacity"])));
        }
    }
    if kind.starts_with("jd.shape.") {
        let op = n["params"]["op"]
            .as_str()
            .unwrap_or(if kind.contains("polygon") {
                "lerp"
            } else {
                "add"
            });
        return Some(format!("{op} · {}", pct(&n["params"]["opacity"])));
    }
    None
}

/// The stack's small icon buttons (`btn px-1 py-0` with a 12 px icon).
fn small_icon_button(ui: &mut egui::Ui, icon: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::image(
            egui::Image::new(look::icon(icon))
                .fit_to_exact_size(vec2(12.0, 12.0))
                .tint(color::FG1),
        )
        .min_size(vec2(18.0, 16.0)),
    )
}

/// The category badge: Tailwind's 900 shade at 60 % behind its 200 shade, as the desktop's.
fn badge(ui: &mut egui::Ui, cat: &str) {
    let (bg, fg) = match cat {
        "shape" => (0x064e3b, 0xa7f3d0),
        "simulation" => (0x78350f, 0xfde68a),
        "mask" => (0x701a75, 0xf5d0fe),
        "output" => (0x27272a, 0xd4d4d8),
        _ => (0x0c4a6e, 0xbae6fd),
    };
    let rgb = |c: u32| egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
    let alpha = if cat == "output" { 1.0 } else { 0.6 };
    egui::Frame::new()
        .fill(rgb(bg).gamma_multiply(alpha))
        .corner_radius(3.0)
        .inner_margin(Margin::symmetric(4, 0))
        .show(ui, |ui| {
            ui.label(RichText::new(cat).size(10.0).color(rgb(fg)));
        });
}

/// What a stroke on the terrain does (the desktop's `drawing` / `sculpting`).
#[derive(Debug, Clone, PartialEq)]
enum Drawing {
    /// The brush: strokes into the strokes node, transforms into the transforms node.
    Sculpt,
    /// The next drag is a road (a spline node), then back to orbiting.
    Road,
    /// Each click is an entrance of this cave node.
    Cave(String),
}

/// A road's bed width (the desktop's `roadWidthM`).
const ROAD_WIDTH_M: f32 = 12.0;

/// The brush (the desktop's `brush` defaults: raise, 150 m, 40 m, amount 0.5, soft, plain).
#[derive(Debug, Clone, PartialEq)]
struct Brush {
    mode: String,
    radius_m: f64,
    strength_m: f64,
    amount: f64,
    hardness: f64,
    target_m: Option<f64>,
    angle_deg: f64,
    factor: f64,
    tilt_deg: f64,
    alpha: String,
}

impl Default for Brush {
    fn default() -> Self {
        Self {
            mode: "raise".into(),
            radius_m: 150.0,
            strength_m: 40.0,
            amount: 0.5,
            hardness: 0.0,
            target_m: None,
            angle_deg: 30.0,
            factor: 1.5,
            tilt_deg: 15.0,
            alpha: "plain".into(),
        }
    }
}

impl Brush {
    /// The transform this mode applies, when it is one (rotate, scale, tilt, push, align).
    fn transform(&self) -> Option<&'static str> {
        ["rotate", "scale", "tilt", "push", "align"]
            .into_iter()
            .find(|m| *m == self.mode)
    }
}

/// The grid tool (the desktop's `grid`: 32 cells a side, smoothness 0.25, step 10 m, …).
#[derive(Debug, Clone, PartialEq)]
struct GridPanel {
    on: bool,
    cells: u32,
    selected: Vec<[u32; 2]>,
    smoothness: f64,
    height_m: f64,
    step_m: f64,
    angle_deg: f64,
    factor: f64,
    tilt_deg: f64,
    tilt_azimuth_deg: f64,
}

impl Default for GridPanel {
    fn default() -> Self {
        Self {
            on: false,
            cells: 32,
            selected: Vec::new(),
            smoothness: 0.25,
            height_m: 0.0,
            step_m: 10.0,
            angle_deg: 90.0,
            factor: 1.5,
            tilt_deg: 10.0,
            tilt_azimuth_deg: 0.0,
        }
    }
}

/// The DEM providers (`geo.fetch_dem`'s `provider`) and their labels.
const GEO_PROVIDERS: [(&str, &str); 3] = [
    ("terrarium", "Terrarium"),
    ("copernicus", "Copernicus"),
    ("opentopography", "OpenTopography"),
];

/// Real world's fields as typed (the desktop's defaults: Nairobi, 2 km, Terrarium).
#[derive(Debug, Clone)]
struct GeoPanel {
    lon: String,
    lat: String,
    radius: String,
    provider: (&'static str, &'static str),
}

impl Default for GeoPanel {
    fn default() -> Self {
        Self {
            lon: "36.82".into(),
            lat: "-1.29".into(),
            radius: "2000".into(),
            provider: GEO_PROVIDERS[0],
        }
    }
}

/// A Strength or octave bar value while it is dragged (the node keeps the applied one).
#[derive(Debug, Default)]
struct NoiseDrag {
    strength: Option<f64>,
    octaves: std::collections::HashMap<usize, f64>,
}

/// The desktop's `JSON.stringify(a) !== JSON.stringify(b)` for parameters: numbers by value
/// (1024 and 1024.0 are the same), arrays element by element.
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same_value(a, b))
        }
        _ => a == b,
    }
}

/// The desktop's cave kinds (`CAVE_KINDS`): node kind and label.
const CAVE_KINDS: [(&str, &str); 9] = [
    ("karst", "Karst (limestone)"),
    ("mountain", "Hill / mountain"),
    ("lava_tube", "Lava tube"),
    ("sea_cave", "Sea cave"),
    ("mine", "Mine"),
    ("tunnel", "Road tunnel"),
    ("catacombs", "Catacombs"),
    ("maze", "Maze (gypsum)"),
    ("flank_margin", "Coastal chambers"),
];

/// One cave node as the panel lists it: `caves.mesh`'s summary, or why it failed.
#[derive(Debug, Clone)]
struct CaveRow {
    id: String,
    summary: Result<CaveSummary, String>,
}

#[derive(Debug, Clone)]
struct CaveSummary {
    length_m: f64,
    entrances: usize,
    speleothems: u64,
    water_level_m: Option<f64>,
    notes: Vec<String>,
}

/// The look's scatter settings as the panel edits them (number fields as typed).
#[derive(Debug, Clone, PartialEq)]
struct ScatterLook {
    mask: Option<String>,
    spacing_m: String,
    seed: String,
    grow: bool,
}

impl ScatterLook {
    /// From the project's look (`None`: the desktop's defaults, spacing 40, seed 1).
    fn of(v: &Value) -> Self {
        Self {
            mask: v["mask"].as_str().map(str::to_owned),
            spacing_m: number(v.get("spacing_m").unwrap_or(&json!(40))),
            seed: number(v.get("seed").unwrap_or(&json!(1))),
            grow: v["grow"].as_bool().unwrap_or(false),
        }
    }

    /// The look's JSON (a number field that does not parse is 0, as the desktop's `Number`).
    fn json(&self) -> Value {
        json!({
            "mask": self.mask,
            "spacing_m": self.spacing_m.trim().parse::<f64>().unwrap_or(0.0),
            "seed": self.seed.trim().parse::<i64>().unwrap_or(0),
            "grow": self.grow,
        })
    }
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
            lod_bias: 0,
            sky_img: None,
            hdri_path: "sky.exr".into(),
            hdri: None,
            waves_for: None,
            waves: None,
            fly_keys: Vec::new(),
            fly_frames: "48".into(),
            fly: None,
            scatter: ScatterLook::of(&Value::Null),
            scatter_points: None,
            scatter_model: String::new(),
            color_layer: None,
            pending_color: None,
            materials: Vec::new(),
            noise: NoisePanel::default(),
            noise_drag: NoiseDrag::default(),
            export: ExportPanel::default(),
            geo: GeoPanel::default(),
            kinds: Vec::new(),
            stack: StackPanel::default(),
            selected_node: None,
            preview_max_px: PREVIEW_MAX_PX,
            preview_target_ms: "0".into(),
            jobs: Vec::new(),
            jobs_polled: None,
            diag: [None, None, None],
            reference_path: String::new(),
            reference: None,
            layer_stats: None,
            measure: None,
            cave_kind: 0,
            tunnel_pending: false,
            drawing: None,
            brush: Brush::default(),
            sculpt_node: None,
            grid: GridPanel::default(),
            show_caves: true,
            xray: 0.0,
            caves: Vec::new(),
            cave_gen: 0,
        };
        // App settings remembered between runs (the desktop keeps them in its browser store).
        if let Some(st) = cc.storage {
            if let Some(lang) = eframe::get_value::<String>(st, "jd.lang") {
                crate::i18n::set(&cc.egui_ctx, &lang);
            }
            if let Some(px) = eframe::get_value::<u32>(st, "jd.previewMaxPx") {
                app.preview_max_px = if px == 2048 { 2048 } else { PREVIEW_MAX_PX };
            }
            if let Some(ms) = eframe::get_value::<String>(st, "jd.previewTargetMs") {
                app.preview_target_ms = ms;
            }
        }
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
        let user = matches!(tag, Tag::User | Tag::Export);
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
                    if matches!(tag, Tag::User | Tag::Export) {
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
                        if r.op == "graph.evaluate"
                            && let Some(report) = &r.result
                        {
                            self.adapt_lod(report);
                        }
                        // A tunnel just added: its portals are placed by clicking the terrain.
                        if r.op == "graph.add_cave"
                            && std::mem::take(&mut self.tunnel_pending)
                            && let Some(id) = r.result.as_ref().and_then(|v| v["id"].as_str())
                        {
                            self.start_placing(id.to_owned());
                        }
                        // A simulation layer: colour by its first mask once it is published (the
                        // desktop's pendingColor).
                        if r.op == "graph.add_simulation"
                            && let Some(first) =
                                r.result.as_ref().and_then(|v| v["published"][0].as_str())
                        {
                            self.pending_color = Some(first.to_owned());
                        }
                    }
                    self.refresh();
                }
                Msg::Event { topic, data } => {
                    // The Editor's selection (a node clicked in the graph) highlights here.
                    if topic == "selection" && data["source"] != "gui-studio" {
                        self.selected_node = data["nodes"][0].as_str().map(str::to_owned);
                    }
                    if topic == "project_changed" {
                        tracing::debug!(op = %data["op"], client = %data["client"], "project changed");
                        // Studio's own writes refresh from their replies (a refresh here would
                        // run mid-chain, e.g. bake maps for a layer not evaluated yet).
                        if data["client"] == "gui-studio" {
                            continue;
                        }
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
                    // Moved to another layer, or PBR switched off, while baking: stale.
                    if self.layer.as_deref() != Some(layer.as_str()) || !self.pbr {
                        continue;
                    }
                    match maps {
                        Ok(m) => {
                            let [w, h] = m.albedo.size;
                            let n = self.materials.len();
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
                Msg::Caves(cave_gen, caves) => {
                    self.reads = self.reads.saturating_sub(1);
                    // A newer fetch is running, or Show caves went off: stale.
                    if cave_gen != self.cave_gen || !self.show_caves {
                        continue;
                    }
                    let meshes: Vec<_> = caves
                        .iter()
                        .filter_map(|(_, r)| r.as_ref().ok().map(|(m, _)| m.clone()))
                        .collect();
                    self.viewport.set_caves(&meshes);
                    self.caves = caves
                        .into_iter()
                        .map(|(id, r)| CaveRow {
                            id,
                            summary: r.map(|(_, s)| s),
                        })
                        .collect();
                }
                Msg::Color(name, pv) => {
                    self.reads = self.reads.saturating_sub(1);
                    // Another colour chosen (or none) while fetching: stale.
                    if self.color_layer.as_deref() != Some(name.as_str()) {
                        continue;
                    }
                    match pv {
                        Ok(pv) => self.viewport.set_overlay(Some((pv, name == "heat"))),
                        Err(e) => self.log(format!("layer_preview (colour): {e}"), Level::Error),
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
                    self.materials = info["look"]["materials"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    let scatter = ScatterLook::of(&info["look"]["scatter"]);
                    if scatter != self.scatter {
                        self.scatter = scatter;
                        self.drop_scatter_points();
                    }
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
                if self.kinds.is_empty() {
                    self.invoke(Tag::Kinds, "graph.kinds", json!({}));
                }
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
                self.layer_stats = r.result.clone();
                self.measure = None;
                let Some(size) = r.result.as_ref().map(|v| &v["size_px"]) else {
                    self.log_response(&r);
                    return self.refresh_done();
                };
                let longest = size[0].as_u64().max(size[1].as_u64()).unwrap_or(1) as u32;
                let lod = longest
                    .div_ceil(self.preview_max_px)
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
            Tag::Look => {
                self.log_response(&r);
                self.invoke(Tag::InfoOnly, "project.info", json!({}));
            }
            Tag::Export => self.log_response(&r),
            Tag::Diag => {
                self.log_response(&r);
                let k = match r.op.as_str() {
                    "diag.env" => 0,
                    "project.cache_gc" => 1,
                    _ => 2,
                };
                if r.result.is_some() {
                    self.diag[k] = r.result;
                }
            }
            Tag::SculptTarget => {
                self.log_response(&r);
                if let Some(id) = r.result.as_ref().and_then(|v| v["id"].as_str()) {
                    self.sculpt_node = Some(id.to_owned());
                    self.selected_node = Some(id.to_owned());
                    self.drawing = Some(Drawing::Sculpt);
                }
                self.refresh();
            }
            Tag::Kinds => {
                if let Some(k) = r.result.as_ref().and_then(Value::as_array) {
                    self.kinds = k.clone();
                }
            }
            Tag::Jobs => {
                self.jobs = r
                    .result
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
            }
            Tag::Reference => {
                self.log_response(&r);
                self.reference = r.result;
            }
            Tag::Measure => {
                self.log_response(&r);
                self.measure = r.result;
            }
            Tag::Scatter => {
                self.log_response(&r);
                if let Some(points) = r.result.as_ref().and_then(|v| v["points"].as_array()) {
                    let f = |p: &Value, k: &str, or: f64| p[k].as_f64().unwrap_or(or) as f32;
                    self.scatter_points = Some(
                        points
                            .iter()
                            .map(|p| {
                                [
                                    f(p, "u", 0.0),
                                    f(p, "v", 0.0),
                                    f(p, "z", 0.0),
                                    f(p, "r", 0.0),
                                    f(p, "age", 1.0),
                                ]
                            })
                            .collect(),
                    );
                    self.show_scatter();
                }
            }
            Tag::Preview(extent, lod, started) => {
                match r.result.as_ref().map(decode_preview) {
                    Some(Ok((pv, bytes))) => {
                        self.viewport.set_terrain(&pv, extent);
                        self.refresh_scatter();
                        self.fetch_maps();
                        self.fetch_horizon();
                        self.fetch_lakes(lod);
                        self.fetch_color(lod);
                        self.fetch_caves(lod);
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

    /// The right column's Actions and Simulation layers sections (the desktop's, in its order):
    /// one-click nodes in front of the height output (or beside it, snow).
    fn actions_card(&mut self, ui: &mut egui::Ui) {
        use crate::chrome::Add;
        const ACTIONS: [(&str, &str, &str); 5] = [
            ("flip_x", "Flip X", "Mirror the terrain east ↔ west"),
            ("flip_y", "Flip Y", "Mirror the terrain north ↔ south"),
            (
                "rotate_left",
                "Rot ⟲",
                "Turn the terrain 90° counter-clockwise (square worlds)",
            ),
            (
                "rotate_right",
                "Rot ⟳",
                "Turn the terrain 90° clockwise (square worlds)",
            ),
            (
                "flatten",
                "Flatten",
                "Flatten the terrain to its mean height (set height_m in the inspector)",
            ),
        ];
        const SIMULATIONS: [(&str, &str); 4] = [
            (
                "snow",
                "A season of snow on the terrain (jd.climate.snowpack): the cover becomes the `snow` layer, a snow material and the viewport colour",
            ),
            (
                "debris",
                "Rockfall debris (jd.erosion.debris) in front of the height output: boulder aprons, the `debris` layer and a rock material",
            ),
            (
                "sand",
                "Wind-blown sand (jd.erosion.aeolian) in front of the height output: dunes along the world wind",
            ),
            (
                "lava",
                "A lava flow from the summit (jd.volcanic.lava) in front of the height output: the `heat` glow and `lava` thickness layers, a basalt material",
            ),
        ];
        let enabled = self.project.is_some() && self.busy == 0;
        let mut add = None;
        look::section(ui, "Actions");
        ui.horizontal_wrapped(|ui| {
            for (k, label, tip) in ACTIONS {
                if look::button(ui, None, label, false, enabled)
                    .on_hover_text(format!("{tip} — a jd.filter.action node in front of the output; undo or bypass it in the layer stack"))
                    .clicked()
                {
                    add = Some(Add::Action(k));
                }
            }
        });
        ui.add_space(12.0);
        look::section(ui, "Simulation layers");
        ui.horizontal_wrapped(|ui| {
            for (k, tip) in SIMULATIONS {
                if look::button(ui, None, &format!("+ {k}"), false, enabled)
                    .on_hover_text(tip)
                    .clicked()
                {
                    add = Some(Add::Simulation(k));
                }
            }
        });
        ui.add(
            egui::Label::new(
                RichText::new("One click adds the node, publishes its masks as layers and a material; tune it in the graph inspector.")
                    .size(12.0)
                    .color(color::FG2),
            )
            .wrap(),
        );
        if let Some(a) = add {
            self.add_layer(a);
        }
    }

    /// The right column's Materials section (the desktop's): one row per layer of the stack —
    /// mask, base colour, emitted light, roughness, detail tile and its metres, remove — and
    /// `+ layer`. An edit writes the look after the debounce and re-bakes the maps.
    fn materials_card(&mut self, ui: &mut egui::Ui) {
        look::section(ui, "Materials");
        // The desktop's `hexOf` / `rgbOf`: 0..1 channels as bytes, no transfer curve.
        let bytes = |v: &Value| -> [u8; 3] {
            std::array::from_fn(|i| {
                (v[i].as_f64().unwrap_or(0.0).clamp(0.0, 1.0) * 255.0).round() as u8
            })
        };
        let unit = |b: [u8; 3]| json!(b.map(|c| f64::from(c) / 255.0));
        let layers = self.layers.clone();
        let before = self.materials.clone();
        let mut remove = None;
        for (i, m) in self.materials.iter_mut().enumerate() {
            ui.horizontal_wrapped(|ui| {
                let mut mask = m["mask"].as_str().map(str::to_owned);
                egui::ComboBox::from_id_salt(("material-mask", i))
                    .width(96.0)
                    .selected_text(RichText::new(mask.as_deref().unwrap_or("everywhere")).size(12.0))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut mask, None, "everywhere");
                        for l in &layers {
                            ui.selectable_value(&mut mask, Some(l.clone()), l);
                        }
                    })
                    .response
                    .on_hover_text("0..1 mask layer that weights this material (everywhere when empty)")
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(egui::accesskit::Role::ComboBox, true, format!("Material {} mask", i + 1))
                    });
                m["mask"] = json!(mask);
                let mut albedo = bytes(&m["albedo"]);
                if egui::color_picker::color_edit_button_srgb(ui, &mut albedo)
                    .on_hover_text("Base colour")
                    .changed()
                {
                    m["albedo"] = unit(albedo);
                }
                let mut glow = if m["emissive"].is_null() { [0; 3] } else { bytes(&m["emissive"]) };
                if egui::color_picker::color_edit_button_srgb(ui, &mut glow)
                    .on_hover_text("Emitted light (black = none): glows in the viewport and the GLB, listed for the engines")
                    .changed()
                {
                    m["emissive"] = if glow == [0; 3] { Value::Null } else { unit(glow) };
                }
                let mut rough = number(&json!(m["roughness"].as_f64().unwrap_or(0.9)));
                if look::input(ui, &mut rough, 56.0, "", &format!("Material {} roughness", i + 1))
                    .on_hover_text("Roughness 0..1")
                    .changed()
                    && let Ok(r) = rough.trim().parse::<f64>()
                {
                    m["roughness"] = json!(r);
                }
                let mut detail = !m["detail"].is_null();
                if look::checkbox(ui, &mut detail, &format!("Material {} detail", i + 1))
                    .on_hover_text("Tileable detail: a seamless grain tile repeated every tile_m metres, blended by this layer's mask in the viewport, baked into exported maps by world position, written as tiles for Unity and Unreal")
                    .changed()
                {
                    m["detail"] = if detail {
                        json!({ "tile_m": 8, "bump": 0.5, "contrast": 0.3, "seed": i, "px": 512 })
                    } else {
                        Value::Null
                    };
                }
                if detail {
                    // Committed on Enter or leaving the field, as the desktop's onBlur.
                    let id = ui.scope_id().with(("material-tile", i));
                    let mut tile = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| number(&m["detail"]["tile_m"]));
                    let r = look::input(ui, &mut tile, 56.0, "", &format!("Material {} tile (m)", i + 1))
                        .on_hover_text("Metres per repeat of the detail tile");
                    if r.lost_focus() {
                        if let Ok(v) = tile.trim().parse::<f64>()
                            && v > 0.0
                        {
                            m["detail"]["tile_m"] = json!(v);
                        }
                        ui.data_mut(|d| d.remove::<String>(id));
                    } else if r.has_focus() {
                        ui.data_mut(|d| d.insert_temp(id, tile));
                    }
                }
                let x = look::button(ui, None, "×", false, true).on_hover_text("Remove this layer");
                x.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, format!("Remove material {}", i + 1))
                });
                if x.clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.materials.remove(i);
        }
        if look::button(ui, None, "+ layer", false, self.project.is_some())
            .on_hover_text("Add a material layer: the stack blends albedo and roughness over the base, each layer by its mask, in order (terrain.textures / export materials)")
            .clicked()
        {
            self.materials.push(json!({ "mask": null, "albedo": [0.5, 0.5, 0.5], "roughness": 0.9 }));
        }
        if self.materials != before {
            // The desktop's setMaterials: the look after the debounce, the maps now.
            if self.project.is_some() {
                self.look_due = Some(Instant::now() + std::time::Duration::from_millis(400));
            }
            self.fetch_maps();
        }
    }

    /// The graph's noise node, if any (the Noise section's Strength and octave bars edit it).
    fn noise_node(&self) -> Option<Value> {
        self.graph["nodes"]
            .as_array()?
            .iter()
            .find(|n| {
                n["kind"]
                    .as_str()
                    .is_some_and(|k| k.starts_with("jd.noise."))
            })
            .cloned()
    }

    /// The desktop's `generate`: the Noise panel into the graph's noise node (only the parameters
    /// that differ, each journaled), then a full evaluation; without a noise node (a graph from an
    /// imported heightmap) the standalone `terrain.generate` fills `height`.
    fn generate(&mut self) {
        let noise = self.noise.json();
        let Some(node) = self.noise_node() else {
            return self.invoke(
                Tag::User,
                "terrain.generate",
                json!({ "noise": noise, "layer": "height" }),
            );
        };
        let id = node["id"].as_str().unwrap_or_default().to_owned();
        let mut steps: Vec<Step> = Vec::new();
        for (key, value) in noise.as_object().into_iter().flatten() {
            if !same_value(&node["params"][key], value) {
                let request = Self::request(
                    "graph.set_param",
                    json!({ "id": id, "key": key, "value": value }),
                );
                steps.push(Box::new(move |_| Some(request)));
            }
        }
        steps.push(Box::new(|_| {
            Some(Self::request("graph.evaluate", json!({ "lod": 0 })))
        }));
        self.chain(steps);
    }

    /// `graph.set_param` on the noise node.
    fn set_noise_param(&mut self, key: &str, value: Value) {
        if let Some(id) = self
            .noise_node()
            .and_then(|n| n["id"].as_str().map(str::to_owned))
        {
            self.invoke(
                Tag::User,
                "graph.set_param",
                json!({ "id": id, "key": key, "value": value }),
            );
        }
    }

    /// The right column's Terrain · Noise section (the desktop's, in its order).
    fn noise_card(&mut self, ui: &mut egui::Ui) {
        use crate::widgets::{Range, VBar};
        look::section(ui, "Terrain · Noise");
        let mut preset = None;
        look::field(ui, "Preset", |ui| {
            egui::ComboBox::from_id_salt("noise-preset")
                .width(144.0)
                .selected_text(RichText::new("choose…").size(12.0))
                .show_ui(ui, |ui| {
                    for (i, p) in NOISE_PRESETS.iter().enumerate() {
                        if ui.selectable_label(false, p.0).clicked() {
                            preset = Some(i);
                        }
                    }
                })
                .response
                .on_hover_text("Fills the noise fields below; a desert plain is the start for dunes (jd.erosion.aeolian@1) and sinkholes (jd.erosion.karst@1)")
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::accesskit::Role::ComboBox, true, "Noise preset")
                });
        });
        if let Some(i) = preset {
            self.noise.apply(NOISE_PRESETS[i]);
        }
        let n = &mut self.noise;
        for (label, value, options) in [
            (
                "Lattice",
                &mut n.kind,
                &[
                    "open_simplex2",
                    "open_simplex2s",
                    "perlin",
                    "value",
                    "value_cubic",
                    "cellular",
                ][..],
            ),
            (
                "Fractal",
                &mut n.fractal,
                &["none", "fbm", "ridged", "ping_pong"][..],
            ),
        ] {
            look::field(ui, label, |ui| {
                egui::ComboBox::from_id_salt(("noise", label))
                    .width(112.0)
                    .selected_text(RichText::new(value.as_str()).size(12.0))
                    .show_ui(ui, |ui| {
                        for o in options {
                            ui.selectable_value(value, (*o).to_owned(), *o);
                        }
                    })
                    .response
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(egui::accesskit::Role::ComboBox, true, label)
                    });
            });
        }
        for (label, value) in [
            ("Seed", &mut n.seed),
            ("Feature size (m)", &mut n.feature_size_m),
            ("Octaves", &mut n.octaves),
            ("Lacunarity", &mut n.lacunarity),
            ("Gain", &mut n.gain),
        ] {
            look::field(ui, label, |ui| {
                look::input(ui, value, 112.0, "", label);
            });
        }
        if let Some(node) = self.noise_node() {
            let strength = node["params"]["strength"].as_f64().unwrap_or(1.0);
            let label = format!("Strength {strength:.2}");
            let mut v = self.noise_drag.strength.unwrap_or(strength);
            let mut apply = None;
            look::field(ui, &label, |ui| {
                let r = ui
                    .add(Range::new(&mut v, 0.0..=2.0, "Strength").step(0.05).reset(1.0).fill_from(1.0))
                    .on_hover_text("General strength: scales the relief around the middle of the height range (World Creator's General Strength)");
                if r.changed() {
                    self.noise_drag.strength = Some(v);
                }
                // Applied on release (the desktop's onPointerUp / onKeyUp), not on every step.
                if r.drag_stopped()
                    || (r.changed() && !r.dragged() && !r.is_pointer_button_down_on())
                {
                    apply = Some(v);
                }
            });
            if let Some(v) = apply {
                self.noise_drag.strength = None;
                self.set_noise_param("strength", json!(v));
            }
            // The octave bars: strength per octave, coarsest first (fbm only); `reset` clears them.
            let count = self
                .noise
                .octaves
                .trim()
                .parse::<usize>()
                .unwrap_or(0)
                .min(16);
            let weights: Vec<f64> = node["params"]["octave_weights"]
                .as_array()
                .map(|a| a.iter().map(|w| w.as_f64().unwrap_or(1.0)).collect())
                .unwrap_or_default();
            let fbm = self.noise.fractal == "fbm";
            let mut reset = false;
            let mut set: Option<Vec<f64>> = None;
            ui.horizontal(|ui| {
                ui.label(RichText::new("Octaves").size(12.0).color(color::FG1));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    reset = ui
                        .add(egui::Button::new(RichText::new("reset").size(10.0).color(color::FG2)).frame(false))
                        .clicked();
                });
            })
            .response
            .on_hover_text("Strength per octave, coarsest first (fbm): drag a bar and release to apply; double-click to reset all to 1");
            if count > 0 {
                let gap = 4.0;
                let w =
                    ((ui.available_width() - gap * (count - 1) as f32) / count as f32).max(16.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for i in 0..count {
                        ui.allocate_ui_with_layout(
                            vec2(w, 80.0),
                            Layout::top_down(Align::Center),
                            |ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                let mut v = self
                                    .noise_drag
                                    .octaves
                                    .get(&i)
                                    .copied()
                                    .unwrap_or_else(|| weights.get(i).copied().unwrap_or(1.0));
                                let r = ui.add_enabled(
                                    fbm,
                                    VBar::new(
                                        &mut v,
                                        0.0..=2.0,
                                        66.0,
                                        &format!("octave {} strength", i + 1),
                                    ),
                                );
                                if r.changed() {
                                    self.noise_drag.octaves.insert(i, v);
                                }
                                if r.drag_stopped()
                                    || (r.changed()
                                        && !r.dragged()
                                        && !r.is_pointer_button_down_on())
                                {
                                    let next = (0..count)
                                        .map(|k| {
                                            if k == i {
                                                v
                                            } else {
                                                weights.get(k).copied().unwrap_or(1.0)
                                            }
                                        })
                                        .collect();
                                    set = Some(next);
                                }
                                ui.label(
                                    RichText::new(format!("{}", i + 1))
                                        .size(9.0)
                                        .color(color::FG2),
                                );
                            },
                        );
                    }
                });
            }
            if reset {
                self.noise_drag.octaves.clear();
                self.set_noise_param("octave_weights", Value::Null);
            } else if let Some(next) = set {
                self.noise_drag.octaves.clear();
                self.set_noise_param("octave_weights", json!(next));
            }
        }
        let n = &mut self.noise;
        look::field(ui, "Height min / max (m)", |ui| {
            look::input(ui, &mut n.height_max, 56.0, "", "Height max (m)");
            look::input(ui, &mut n.height_min, 56.0, "", "Height min (m)");
        });
        ui.add_space(8.0);
        let can = self.project.is_some() && self.busy == 0;
        let generate = look::wide_primary(ui, "mountain", "Generate", can).clicked();
        if generate {
            self.generate();
        }
        // The shown layer's statistics, then Measure landform and its readout.
        if let Some(st) = self.layer_stats.clone() {
            ui.add_space(8.0);
            let grid = |ui: &mut egui::Ui, id: &str, rows: Vec<(&str, String)>| {
                egui::Grid::new(id)
                    .num_columns(2)
                    .spacing(vec2(8.0, 4.0))
                    .show(ui, |ui| {
                        for (k, v) in rows {
                            ui.label(RichText::new(k).size(12.0).color(color::FG2));
                            ui.label(RichText::new(v).size(12.0).color(color::FG2));
                            ui.end_row();
                        }
                    });
            };
            let f = |k: &str| st["stats"][k].as_f64().unwrap_or(0.0);
            let hash: String = st["content_hash"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(16)
                .collect();
            grid(
                ui,
                "layer-stats",
                vec![
                    ("min / max", format!("{:.1} / {:.1} m", f("min"), f("max"))),
                    ("mean", format!("{:.1} m", f("mean"))),
                    (
                        "time",
                        format!("{} ms", st["duration_ms"].as_u64().unwrap_or(0)),
                    ),
                    ("hash", format!("{hash}…")),
                ],
            );
            ui.add_space(8.0);
            if look::button(ui, None, "Measure landform", false, self.busy == 0)
                .on_hover_text("terrain.measure: relief, hypsometric integral, slope statistics and geomorphon shares — compare with a real DEM of the same landform (Reading a landscape)")
                .clicked()
            {
                let layer = self.layer.as_deref().unwrap_or("height").split('#').next().unwrap_or("height").to_owned();
                self.invoke(Tag::Measure, "terrain.measure", json!({ "layer": layer }));
            }
            if let Some(m) = self.measure.clone() {
                ui.add_space(8.0);
                let hi = m["hypsometric_integral"].as_f64().unwrap_or(0.0);
                let age = if hi < 0.45 {
                    "(dissected, old)"
                } else if hi > 0.55 {
                    "(plateau-like, young)"
                } else {
                    ""
                };
                let s = |k: &str| m["slope"][k].as_f64().unwrap_or(0.0);
                let mut forms: Vec<(String, f64)> = m["geomorphons"]
                    .as_object()
                    .map(|o| {
                        o.iter()
                            .map(|(k, v)| (k.clone(), v.as_f64().unwrap_or(0.0)))
                            .collect()
                    })
                    .unwrap_or_default();
                forms.sort_by(|a, b| b.1.total_cmp(&a.1));
                let forms = forms
                    .iter()
                    .take(5)
                    .map(|(k, v)| format!("{k} {:.0} %", v * 100.0))
                    .collect::<Vec<_>>()
                    .join(", ");
                grid(
                    ui,
                    "measure-readout",
                    vec![
                        (
                            "relief",
                            format!("{:.0} m", m["relief_m"].as_f64().unwrap_or(0.0)),
                        ),
                        (
                            "hypsometric",
                            format!("{hi:.2} {age}").trim_end().to_owned(),
                        ),
                        (
                            "slope",
                            format!(
                                "mean {:.1}°, median {:.1}°, p90 {:.1}°",
                                s("mean_deg"),
                                s("p50_deg"),
                                s("p90_deg")
                            ),
                        ),
                        ("landforms", forms),
                    ],
                );
            }
        }
    }

    /// Choose where to save the heightmap, then export it there (the desktop's `pickExportPath`:
    /// the panel's browse button and the toolbar's ⌘E).
    fn pick_export_path(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .set_title("Export heightmap")
            .set_file_name(&self.export.path)
            .add_filter("OpenEXR (32-bit float)", &["exr"])
            .add_filter("PNG (16-bit)", &["png"])
            .add_filter("RAW 16-bit", &["r16"])
            .add_filter("RAW 32-bit float", &["r32"])
            .add_filter("TIFF (32-bit float)", &["tif", "tiff"])
            .save_file()
        {
            self.export.path = p.display().to_string();
            self.export_heightmap(self.export.path.clone());
        }
    }

    /// The right column's Export section (the desktop's, in its order): heightmap, holes, engine
    /// profile, mesh, HDRI, water maps, timeline frames.
    fn export_card(&mut self, ui: &mut egui::Ui) {
        look::section(ui, "Export");
        let idle = self.busy == 0;
        let has_layer = self.layer.is_some();
        let has_project = self.project.is_some();
        let sea = self.graph["nodes"].as_array().and_then(|n| {
            n.iter()
                .find(|n| {
                    n["kind"]
                        .as_str()
                        .is_some_and(|k| k.starts_with("jd.hydrology.sea_level@"))
                })
                .cloned()
        });
        let holes_options: Vec<String> = self
            .layers
            .iter()
            .filter(|l| !l.contains('#'))
            .cloned()
            .collect();
        let e = &mut self.export;
        let mut act: Option<&str> = None;
        look::field(ui, "File", |ui| {
            if look::icon_button(
                ui,
                "folder-search",
                "Choose where to save… (⌘E)",
                has_layer && idle,
            )
            .clicked()
            {
                act = Some("browse");
            }
            look::input(ui, &mut e.path, 112.0, "", "Export file")
                .on_hover_text("Relative paths go to the project's exports/ directory");
        });
        ui.add_space(4.0);
        if look::wide_button(ui, "upload", "Export heightmap", false, has_layer && idle).clicked() {
            act = Some("heightmap");
        }
        look::field(ui, "Holes", |ui| {
            egui::ComboBox::from_id_salt("export-holes")
                .width(112.0)
                .selected_text(RichText::new(e.holes.as_deref().unwrap_or("none")).size(12.0))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut e.holes, None, "none");
                    for l in &holes_options {
                        ui.selectable_value(&mut e.holes, Some(l.clone()), l);
                    }
                })
                .response
                .on_hover_text("A 0..1 layer (a painted mask, a class mask) whose cells ≥ 0.5 are holes: left out of the mesh exports, written as holes.png for the engines (Unity SetHoles, Unreal Visibility)")
                .widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::ComboBox, true, "Holes"));
        });
        look::field(ui, "Engine", |ui| {
            if look::button(ui, Some("upload"), "Export", false, has_layer && idle)
                .on_hover_text("Write exports/<engine>/: heightmap, one weightmap per material mask, RGBA splat maps, README with the import numbers, materials.json")
                .clicked()
            {
                act = Some("profile");
            }
            look::input(ui, &mut e.tiles, 48.0, "", "Tiles per side").on_hover_text("Tiles per side: 1 writes one folder; 2–16 cut the world into tiles × tiles engine folders (tile_<x>_<y>/) sharing one height range, with tiles.json — the resolution must divide");
            egui::ComboBox::from_id_salt("export-engine")
                .width(80.0)
                .selected_text(RichText::new(if e.unreal { "Unreal" } else { "Unity" }).size(12.0))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut e.unreal, false, "Unity");
                    ui.selectable_value(&mut e.unreal, true, "Unreal");
                })
                .response
                .on_hover_text("Unity Terrain (RAW 16-bit + weightmaps) or Unreal Landscape (PNG16 + weightmaps), with the material stack as materials.json")
                .widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::ComboBox, true, "Engine"));
        });
        look::field(ui, "Mesh", |ui| {
            if look::button(ui, Some("upload"), "Export", false, has_layer && idle).clicked() {
                act = Some("mesh");
            }
            look::input(ui, &mut e.mesh, 96.0, "", "Mesh file").on_hover_text("exports/<file>: .glb (materials baked into its textures), .fbx, .obj, .stl, .ply, .usda, .usdz");
        });
        look::field(ui, "HDRI", |ui| {
            if look::button(ui, Some("upload"), "Bake", false, has_project && idle).clicked() {
                act = Some("sky");
            }
            look::input(ui, &mut e.sky, 96.0, "", "HDRI file").on_hover_text("exports/<file>.exr: the viewport's sun, turbidity and clouds as an equirectangular HDRI (a .png preview beside it)");
        });
        ui.add_space(4.0);
        if look::wide_button(ui, "upload", "Export water maps", false, sea.is_some() && idle)
            .on_hover_text("Write exports/water/: depth, colour, foam and a tileable wave normal map for the sea-level node's clarity and wind")
            .clicked()
        {
            act = Some("water");
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            look::input(ui, &mut e.frames, 64.0, "", "Timeline frames").on_hover_text("Frames across the timeline (t = 0 … 1)");
            if look::wide_button(ui, "upload", "Export frames", false, has_project && idle)
                .on_hover_text("Write exports/frames/: one 16-bit heightmap per frame from t = 0 to 1, keyframed parameters animating (timeline.export)")
                .clicked()
            {
                act = Some("frames");
            }
        });
        // The tiles field keeps the desktop's clamp: 1..16, 1 when it does not parse.
        let tiles = self
            .export
            .tiles
            .trim()
            .parse::<u32>()
            .unwrap_or(1)
            .clamp(1, 16);
        let holes = self.export.holes.clone();
        let materials = Value::Array(self.materials.clone());
        match act {
            Some("browse") => self.pick_export_path(),
            Some("heightmap") => self.export_heightmap(self.export.path.clone()),
            Some("profile") => {
                let profile = if self.export.unreal {
                    "unreal"
                } else {
                    "unity"
                };
                // Each mask once, in stack order (the desktop's `new Set`).
                let mut seen = std::collections::HashSet::new();
                let masks: Vec<&str> = self
                    .materials
                    .iter()
                    .filter_map(|m| m["mask"].as_str())
                    .filter(|m| seen.insert(*m))
                    .collect();
                let mut params = json!({ "profile": profile, "dir": profile, "layer": "height", "masks": masks, "materials": materials });
                if tiles > 1 {
                    params["tiles"] = json!(tiles);
                }
                if let Some(h) = &holes {
                    params["holes"] = json!(h);
                }
                // The project's caves go along: their meshes, their mouths merged into holes.png.
                if !self.cave_nodes().is_empty() {
                    params["caves"] = json!(true);
                }
                self.invoke(Tag::Export, "terrain.export_profile", params);
            }
            Some("mesh") => {
                let mut params =
                    json!({ "layer": "height", "path": self.export.mesh, "materials": materials });
                // Without a holes layer chosen, the caves' mouths open the mesh (cave_holes).
                let open = holes.or_else(|| {
                    self.layers
                        .iter()
                        .any(|l| l == "cave_holes")
                        .then(|| "cave_holes".to_owned())
                });
                if let Some(h) = open {
                    params["holes"] = json!(h);
                }
                self.invoke(Tag::Export, "terrain.export_mesh", params);
            }
            Some("sky") => {
                let [az, el, turbidity, clouds, fog] = self.sun.0;
                self.invoke(
                    Tag::Export,
                    "sky.bake",
                    json!({ "sun_azimuth_deg": az, "sun_elevation_deg": el, "turbidity": turbidity, "cloud_cover": clouds, "fog": fog, "path": self.export.sky }),
                );
            }
            Some("water") => {
                if let Some(sea) = sea {
                    let p = &sea["params"];
                    let wind = self
                        .project
                        .as_ref()
                        .map(|p| p["world"]["wind"].clone())
                        .unwrap_or_default();
                    let mut params = json!({
                        "level_m": p["level_m"].as_f64().unwrap_or(0.0),
                        "visibility_m": p["visibility_m"].as_f64().unwrap_or(15.0),
                        "wind_speed_mps": p["wind_speed_mps"].as_f64().or(wind["speed_mps"].as_f64()).unwrap_or(8.0),
                        "fetch_km": p["fetch_km"].as_f64(),
                        "wind_azimuth_deg": p["wind_azimuth_deg"].as_f64().or(wind["azimuth_deg"].as_f64()).unwrap_or(270.0),
                    });
                    if p["tint"].as_array().is_some_and(|t| t.len() == 3) {
                        params["tint"] = p["tint"].clone();
                    }
                    self.invoke(Tag::Export, "ocean.bake", params);
                }
            }
            Some("frames") => {
                let frames = self.export.frames.trim().parse::<u32>().unwrap_or(0);
                self.invoke(
                    Tag::Export,
                    "timeline.export",
                    json!({ "frames": frames, "dir": "frames", "format": "png16" }),
                );
            }
            _ => {}
        }
    }

    /// The desktop's `fetchDem`: the DEM around `center` (lon, lat) from `provider`, the world
    /// placed in its CRS, a heightmap node for it wired into the height output in place of what fed
    /// it, then a full evaluation — one chain, stopping at the first failure.
    fn fetch_dem(&mut self, center: [f64; 2], radius_m: f64, provider: &str) {
        let nodes = self.graph["nodes"].as_array().cloned().unwrap_or_default();
        let right = nodes
            .iter()
            .filter_map(|n| n["ui"]["position"][0].as_f64())
            .fold(-320.0_f64, f64::max);
        let out = nodes
            .iter()
            .find(|n| {
                n["kind"]
                    .as_str()
                    .is_some_and(|k| k.starts_with("jd.output.height"))
            })
            .and_then(|n| n["id"].as_str().map(str::to_owned));
        let feeds: Vec<Value> = match &out {
            Some(id) => self.graph["edges"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|e| e["to"]["node"] == id.as_str() && e["to"]["port"] == "in")
                .cloned()
                .collect(),
            None => Vec::new(),
        };
        let fetch = Self::request(
            "geo.fetch_dem",
            json!({ "center": center, "radius_m": radius_m, "provider": provider, "res_m": 30 }),
        );
        let mut steps: Vec<Step> = vec![
            Box::new(move |_| Some(fetch)),
            Box::new(|done| {
                let dem = done[0].result.as_ref()?;
                Some(Self::request(
                    "project.set_settings",
                    json!({ "crs": dem["crs"], "origin": dem["origin"] }),
                ))
            }),
            Box::new(move |done| {
                let dem = done[0].result.as_ref()?;
                Some(Self::request(
                    "graph.add_node",
                    json!({ "kind": "jd.input.heightmap@1", "params": { "path": dem["asset"], "georeference": true }, "position": [right + 320.0, 0.0] }),
                ))
            }),
        ];
        if let Some(out) = out {
            // The starter noise feeds the height output; the DEM takes its place.
            for e in feeds {
                steps.push(Box::new(move |_| {
                    Some(Self::request(
                        "graph.disconnect",
                        json!({ "from": e["from"]["node"], "from_port": e["from"]["port"], "to": e["to"]["node"], "to_port": e["to"]["port"] }),
                    ))
                }));
            }
            steps.push(Box::new(move |done| {
                let id = done[2].result.as_ref()?["id"].clone();
                Some(Self::request(
                    "graph.connect",
                    json!({ "from": id, "from_port": "out", "to": out, "to_port": "in" }),
                ))
            }));
        }
        steps.push(Box::new(|_| {
            Some(Self::request("graph.evaluate", json!({ "lod": 0 })))
        }));
        self.chain(steps);
    }

    /// Real world (the desktop's row under Import): longitude, latitude, radius, provider, fetch.
    fn real_world_row(&mut self, ui: &mut egui::Ui) {
        let idle = self.busy == 0;
        let mut fetch = false;
        let g = &mut self.geo;
        look::field(ui, "Real world", |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    look::input(ui, &mut g.lon, 64.0, "", "Longitude");
                    look::input(ui, &mut g.lat, 64.0, "", "Latitude");
                });
                ui.horizontal(|ui| {
                    look::input(ui, &mut g.radius, 64.0, "", "Radius (m)");
                    egui::ComboBox::from_id_salt("geo-provider")
                        .width(96.0)
                        .selected_text(RichText::new(g.provider.1).size(12.0))
                        .show_ui(ui, |ui| {
                            for p in GEO_PROVIDERS {
                                ui.selectable_value(&mut g.provider, p, p.1);
                            }
                        })
                        .response
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(egui::accesskit::Role::ComboBox, true, "Provider")
                        });
                });
                let globe = look::button(ui, Some("globe"), "", false, idle)
                    .on_hover_text("Fetch the DEM and place the world on it");
                globe.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::accesskit::Role::Button, idle, "Fetch the DEM")
                });
                fetch = globe.clicked();
            })
            .response
            .on_hover_text("Fetch a DEM around a point (ADR-0014: Terrarium tiles without a key, Copernicus GLO-30, or OpenTopography with JD_OPENTOPOGRAPHY_KEY), place the world in its CRS and feed it to the height output; the provider's notice goes into every export");
        });
        if fetch {
            let f = |s: &str| s.trim().parse::<f64>().unwrap_or(0.0);
            let (lon, lat, radius) = (f(&self.geo.lon), f(&self.geo.lat), f(&self.geo.radius));
            let provider = self.geo.provider.0;
            self.fetch_dem([lon, lat], radius, provider);
        }
    }

    /// The left column's Reference image section: a photo, 360° sphere or video read by
    /// `media.metadata` — where, when, heading, field of view and the sun — with Use sun and
    /// Terrain here (5 km of Terrarium around its position).
    fn reference_card(&mut self, ui: &mut egui::Ui) {
        look::section(ui, "Reference image");
        let idle = self.busy == 0;
        let mut read: Option<String> = None;
        let path = &mut self.reference_path;
        look::field(ui, "File", |ui| {
            if look::button(ui, None, "Read", false, !path.trim().is_empty() && idle).clicked() {
                read = Some(path.trim().to_owned());
            }
            if look::icon_button(
                ui,
                "folder-search",
                "Choose a reference image or video…",
                idle,
            )
            .clicked()
                && let Some(p) = rfd::FileDialog::new()
                    .set_title("Reference image or video")
                    .add_filter(
                        "Images and videos",
                        &[
                            "jpg", "jpeg", "png", "tif", "tiff", "heic", "webp", "mp4", "mov",
                        ],
                    )
                    .pick_file()
            {
                *path = p.display().to_string();
                read = Some(path.clone());
            }
            look::input(ui, path, 96.0, "photo, 360°, video", "Reference file").on_hover_text("A photo (JPEG, PNG, TIFF, HEIF, WebP), a 360° photo sphere or a video (MP4, MOV): media.metadata reads where, when and how it was taken, and the sun");
        });
        if let Some(p) = read {
            self.invoke(Tag::Reference, "media.metadata", json!({ "path": p }));
        }
        let Some(r) = self.reference.clone() else {
            return;
        };
        let f = |v: &Value| v.as_f64().unwrap_or(0.0);
        let kind = match (r["kind"].as_str(), r["px"].as_array()) {
            (Some(k), Some(px)) => format!("{k} {}×{}", px[0], px[1]),
            (Some(k), None) => k.to_owned(),
            _ => String::new(),
        };
        let when = match r["captured_at_utc"].as_str() {
            Some(t) => match r["captured_at_source"].as_str() {
                Some(src) => format!("{t} ({src})"),
                None => t.to_owned(),
            },
            None => "unknown".into(),
        };
        let place = if r["gps"].is_object() {
            format!("{:.4}, {:.4}", f(&r["gps"]["lat"]), f(&r["gps"]["lon"]))
        } else {
            "no GPS".into()
        };
        let heading = if r["heading_deg"].is_number() {
            format!(
                "{:.0}° {}",
                f(&r["heading_deg"]),
                r["heading_ref"].as_str().unwrap_or_default()
            )
        } else {
            "unknown".into()
        };
        let fov = if r["fov_x_deg"].is_number() {
            format!("{:.1}°", f(&r["fov_x_deg"]))
        } else {
            r["projection"]["type"]
                .as_str()
                .unwrap_or("unknown")
                .to_owned()
        };
        let sun = &r["sun"];
        let up = sun.is_object() && !sun["below_horizon"].as_bool().unwrap_or(false);
        let sun_text = if !sun.is_object() {
            "needs place and time".to_owned()
        } else if !up {
            "below the horizon".to_owned()
        } else {
            format!(
                "{:.0}° from north, {:.0}° up",
                f(&sun["azimuth_deg"]),
                f(&sun["elevation_deg"])
            )
        };
        egui::Grid::new("ref-info")
            .num_columns(2)
            .spacing(vec2(8.0, 2.0))
            .show(ui, |ui| {
                for (k, v) in [
                    ("kind", kind),
                    ("when", when),
                    ("where", place),
                    ("heading", heading.trim_end().to_owned()),
                    ("field of view", fov),
                    ("sun", sun_text),
                ] {
                    ui.label(RichText::new(k).size(12.0).color(color::FG2));
                    ui.label(RichText::new(v).size(12.0).color(color::FG2));
                    ui.end_row();
                }
            });
        ui.add_space(4.0);
        let (mut use_sun, mut here) = (false, false);
        ui.horizontal(|ui| {
            use_sun = look::button(ui, None, "Use sun", false, up)
                .on_hover_text("Set the viewport sun to the reference's")
                .clicked();
            here = look::button(ui, None, "Terrain here", false, r["gps"].is_object() && idle)
                .on_hover_text("Fetch the real terrain 5 km around the reference's position (Terrarium) and place the world on it")
                .clicked();
        });
        for n in r["notes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            ui.label(RichText::new(format!("• {n}")).size(12.0).color(color::FG2));
        }
        if use_sun {
            // The desktop's useReferenceSun: whole degrees, elevation held to 3..90.
            self.sun.0[0] = f(&sun["azimuth_deg"]).round().rem_euclid(360.0);
            self.sun.0[1] = f(&sun["elevation_deg"]).round().clamp(3.0, 90.0);
            self.sun_edited();
        }
        if here {
            self.fetch_dem(
                [f(&r["gps"]["lon"]), f(&r["gps"]["lat"])],
                5000.0,
                "terrarium",
            );
        }
    }

    /// The left column's Settings section (the desktop's): language, the project's settings
    /// committed on leaving a field (`project.set_settings`), preview detail and the live target
    /// (app settings, remembered), the world's origin, CRS and shared wind.
    fn settings_card(&mut self, ui: &mut egui::Ui) {
        let Some(p) = self.project.clone() else {
            return;
        };
        look::section(ui, "Settings");
        let ctx = ui.ctx().clone();
        let mut lang = crate::i18n::lang(&ctx);
        let label = crate::i18n::tr(&ctx, "Language");
        look::field(ui, &label, |ui| {
            egui::ComboBox::from_id_salt("setting-lang")
                .width(112.0)
                .selected_text(
                    RichText::new(
                        crate::i18n::LANGS
                            .iter()
                            .find(|l| l.0 == lang)
                            .map_or("English", |l| l.1),
                    )
                    .size(12.0),
                )
                .show_ui(ui, |ui| {
                    for (code, name) in crate::i18n::LANGS {
                        ui.selectable_value(&mut lang, code.to_owned(), name);
                    }
                })
                .response
                .on_hover_text("Interface language (Kiswahili and French are first drafts)")
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::accesskit::Role::ComboBox, true, "Language")
                });
        });
        if lang != crate::i18n::lang(&ctx) {
            crate::i18n::set(&ctx, &lang);
        }
        let s = &p["settings"];
        let w = &p["world"];
        let mut patch: Option<Value> = None;
        let num = |v: &Value| number(v);
        let rows: [(&str, String, &str); 4] = [
            (
                "Autosave (s)",
                num(&s["autosave_interval_s"]),
                "Seconds after the last edit before the project autosaves; 0 disables",
            ),
            (
                "Cache limit (MB)",
                number(&json!(
                    (s["cache_limit_bytes"].as_f64().unwrap_or(0.0) / 1_048_576.0).round()
                )),
                "Layer cache cap on disk; oldest layers are evicted beyond it",
            ),
            (
                "Preview LOD",
                num(&s["preview_lod"]),
                "Preview level of detail hint (resolution ÷ 2^lod)",
            ),
            (
                "Memory budget (GB)",
                number(&json!(
                    s["memory_budget_bytes"].as_f64().unwrap_or(4_294_967_296.0) / 1_073_741_824.0
                )),
                "Evaluations whose memory estimate exceeds this are refused (E_RESOURCE_MEMORY)",
            ),
        ];
        for (label, current, tip) in rows {
            look::field(ui, label, |ui| {
                if let Some(v) = look::commit_input(ui, &current, 112.0, "", label, tip)
                    .map(|t| t.parse::<f64>().unwrap_or(0.0))
                {
                    patch = Some(match label {
                        "Autosave (s)" => json!({ "autosave_interval_s": v as u64 }),
                        "Cache limit (MB)" => {
                            json!({ "cache_limit_bytes": (v.round() as u64) * 1_048_576 })
                        }
                        "Preview LOD" => json!({ "preview_lod": v as u64 }),
                        _ => json!({ "memory_budget_bytes": (v * 1_073_741_824.0).round() as u64 }),
                    });
                }
            });
        }
        let mut px = self.preview_max_px;
        look::field(ui, "Preview detail", |ui| {
            egui::ComboBox::from_id_salt("setting-preview-px")
                .width(112.0)
                .selected_text(RichText::new(if px == 2048 { "2048² (high)" } else { "1024²" }).size(12.0))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut px, 1024, "1024²");
                    ui.selectable_value(&mut px, 2048, "2048² (high)");
                })
                .response
                .on_hover_text("Viewport preview detail: the longest side of the mesh the viewport shows. 1024² ≈ 1 M samples (2 MB a transfer, 2 M triangles); 2048² is 4× that — 8 MB and 8 M triangles, for worlds above 1024² when the fine ridges matter more than the frame rate")
                .widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::ComboBox, true, "Preview detail"));
        });
        if px != self.preview_max_px {
            self.preview_max_px = px;
            self.refresh();
        }
        look::field(ui, "Live target (ms)", |ui| {
            let target = look::input(ui, &mut self.preview_target_ms, 112.0, "", "Live target (ms)").on_hover_text("Live preview target per evaluation, ms (0 = off): the preview LOD steps coarser while evaluations run over it and finer again when they are well under; this app setting is not part of the project");
            // A new target starts from the default LOD again (the desktop's setPreviewTarget).
            if target.changed() {
                self.lod_bias = 0;
            }
        });
        let origin = [
            w["origin"][0].as_f64().unwrap_or(0.0),
            w["origin"][1].as_f64().unwrap_or(0.0),
        ];
        let crs = w["crs"].as_str().unwrap_or_default().to_owned();
        let wind = [
            w["wind"]["azimuth_deg"].as_f64().unwrap_or(270.0),
            w["wind"]["speed_mps"].as_f64().unwrap_or(8.0),
        ];
        let f = |t: &str| t.parse::<f64>().ok().filter(|v| v.is_finite());
        look::field(ui, "Origin x (m)", |ui| {
            if let Some(v) = look::commit_input(ui, &number(&json!(origin[0])), 112.0, "", "Origin x (m)", "World origin, metres: where pixel (0, 0) sits in the CRS. Georeferenced exports carry it; generators sample world space, so moving it moves the window over the noise field (adjacent worlds stay seamless)").and_then(|t| f(&t)) {
                patch = Some(json!({ "origin": [v, origin[1]] }));
            }
        });
        look::field(ui, "Origin y (m)", |ui| {
            if let Some(v) = look::commit_input(
                ui,
                &number(&json!(origin[1])),
                112.0,
                "",
                "Origin y (m)",
                "World origin y (north), metres",
            )
            .and_then(|t| f(&t))
            {
                patch = Some(json!({ "origin": [origin[0], v] }));
            }
        });
        look::field(ui, "CRS", |ui| {
            if let Some(v) = look::commit_input(
                ui,
                &crs,
                112.0,
                "EPSG:32637",
                "CRS",
                "Coordinate reference system of the origin, e.g. EPSG:32637 (UTM 37N); empty = none. GeoTIFF exports carry it",
            ) {
                patch = Some(json!({ "crs": v }));
            }
        });
        look::field(ui, "Wind from (°)", |ui| {
            if let Some(v) = look::commit_input(ui, &number(&json!(wind[0])), 112.0, "", "Wind from (°)", "Shared wind: dunes, yardangs, snow scour and the wind sea follow it unless a node sets its own wind").and_then(|t| f(&t)) {
                patch = Some(json!({ "wind": { "azimuth_deg": v, "speed_mps": wind[1] } }));
            }
        });
        look::field(ui, "Wind (m/s)", |ui| {
            if let Some(v) = look::commit_input(
                ui,
                &number(&json!(wind[1])),
                112.0,
                "",
                "Wind (m/s)",
                "Shared wind speed 10 m above the ground, m/s: 3 a breeze, 8 moderate, 20 a gale",
            )
            .and_then(|t| f(&t))
            {
                patch = Some(json!({ "wind": { "azimuth_deg": wind[0], "speed_mps": v } }));
            }
        });
        if let Some(patch) = patch {
            self.invoke(Tag::User, "project.set_settings", patch);
        }
    }

    /// The left column's Tasks section: the engine's running jobs (`jobs.list`, polled while
    /// Studio waits on it), each with Cancel.
    fn tasks_card(&mut self, ui: &mut egui::Ui) {
        look::section(ui, "Tasks");
        let jobs = self.jobs.clone();
        if jobs.is_empty() {
            ui.label(
                RichText::new("no running jobs")
                    .size(12.0)
                    .color(color::FG2),
            );
        }
        for j in jobs {
            ui.horizontal(|ui| {
                let total = j["total"].as_u64().unwrap_or(0);
                let mut t = format!("#{} {}", j["id"], j["op"].as_str().unwrap_or_default());
                if j["state"] == "queued" {
                    t += " · queued";
                } else if total > 0 {
                    t += &format!(
                        " · {}/{total}",
                        (j["done"].as_u64().unwrap_or(0) + 1).min(total)
                    );
                }
                if let Some(p) = j["priority"].as_str().filter(|p| *p != "normal") {
                    t += &format!(" · {p}");
                }
                t += &format!(
                    " · {:.1} s",
                    j["elapsed_ms"].as_f64().unwrap_or(0.0) / 1000.0
                );
                ui.label(RichText::new(t).size(12.0).color(color::FG1));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let cancelling = j["cancelling"].as_bool().unwrap_or(false);
                    if look::button(
                        ui,
                        None,
                        if cancelling { "…" } else { "Cancel" },
                        false,
                        !cancelling,
                    )
                    .clicked()
                    {
                        self.invoke(Tag::Export, "jobs.cancel", json!({ "id": j["id"] }));
                    }
                });
            });
        }
    }

    /// The left column's Diagnostics section: Environment, Cache GC, Bench 256², and their lines.
    fn diagnostics_card(&mut self, ui: &mut egui::Ui) {
        look::section(ui, "Diagnostics");
        let idle = self.busy == 0;
        let mut run = None;
        ui.horizontal_wrapped(|ui| {
            if look::button(ui, None, "Environment", false, idle).clicked() {
                run = Some(("diag.env", json!({})));
            }
            if look::button(ui, None, "Cache GC", false, self.project.is_some() && idle).clicked() {
                run = Some(("project.cache_gc", json!({})));
            }
            if look::button(ui, None, "Bench 256²", false, idle)
                .on_hover_text("S1 scene at 256² with the real kernels; use `jd diag bench s1` for the full benchmark")
                .clicked()
            {
                run = Some(("diag.bench", json!({ "scene": "s1", "size_px": 256 })));
            }
        });
        if let Some((op, params)) = run {
            self.invoke(Tag::Diag, op, params);
        }
        let mut rows: Vec<(&str, String)> = Vec::new();
        let mb = |v: &Value| v.as_f64().unwrap_or(0.0) / 1_048_576.0;
        if let Some(e) = &self.diag[0] {
            rows.push((
                "host",
                format!(
                    "{} · {} · {} cpus · {:.0} GB",
                    e["os"].as_str().unwrap_or_default(),
                    e["arch"].as_str().unwrap_or_default(),
                    e["cpus"],
                    e["memory_total_bytes"].as_f64().unwrap_or(0.0) / 1_073_741_824.0
                ),
            ));
        }
        if let Some(g) = &self.diag[1] {
            let mut t = format!(
                "{} blobs · {:.1} MB · evicted {}",
                g["blobs"],
                mb(&g["bytes"]),
                g["evicted_blobs"]
            );
            if g["limit_bytes"].is_number() {
                t += &format!(" · limit {:.0} MB", mb(&g["limit_bytes"]));
            }
            rows.push(("cache", t));
        }
        if let Some(b) = &self.diag[2] {
            let mut t = format!(
                "S1 {}² {}: total {:.0} ms",
                b["size_px"],
                b["profile"].as_str().unwrap_or_default(),
                b["total_ms"].as_f64().unwrap_or(0.0)
            );
            if b["peak_rss_bytes"].is_number() {
                t += &format!(" · {:.0} MB", mb(&b["peak_rss_bytes"]));
            }
            let stages: Vec<String> = b["stages"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|s| {
                    let name = s["name"].as_str().unwrap_or_default();
                    format!(
                        "{} {:.0}",
                        name.rsplit('.').next().unwrap_or(name),
                        s["ms"].as_f64().unwrap_or(0.0)
                    )
                })
                .collect();
            t += &format!(" · {}", stages.join(", "));
            rows.push(("bench", t));
        }
        if !rows.is_empty() {
            egui::Grid::new("diag-results")
                .num_columns(2)
                .spacing(vec2(8.0, 2.0))
                .show(ui, |ui| {
                    for (k, v) in rows {
                        ui.label(RichText::new(k).size(12.0).color(color::FG2));
                        ui.add(
                            egui::Label::new(RichText::new(v).size(12.0).color(color::FG1)).wrap(),
                        );
                        ui.end_row();
                    }
                });
        }
    }

    /// The left column's Layers section, the desktop's `LayerStack`: the chain feeding the height
    /// output (top = last applied) with bypass, up / down and remove, the side branches grouped by
    /// category, a search box, and `+` to splice a kind in front of the output.
    fn layers_card(&mut self, ui: &mut egui::Ui) {
        look::section(ui, "Layers");
        let nodes = self.graph["nodes"].as_array().cloned().unwrap_or_default();
        let edges = self.graph["edges"].as_array().cloned().unwrap_or_default();
        let (chain, others) = stack_of(&nodes, &edges);
        let idle = self.busy == 0;
        let has_project = self.project.is_some();
        // `+`: kinds with an `in` input and an `out` output.
        let kinds: Vec<String> = self
            .kinds
            .iter()
            .filter(|k| {
                let has = |side: &str, name: &str| {
                    k[side]
                        .as_array()
                        .is_some_and(|p| p.iter().any(|p| p["name"] == name))
                };
                has("inputs", "in") && has("outputs", "out")
            })
            .filter_map(|k| k["kind"].as_str().map(str::to_owned))
            .collect();
        let mut act: Option<(&str, Value)> = None;
        // Right to left: `+` first, then the kind picker takes exactly what is left (a width asked
        // beyond it would widen the panel a little more every frame).
        let row = vec2(ui.available_width(), 24.0);
        ui.allocate_ui_with_layout(row, Layout::right_to_left(Align::Center), |ui| {
            let add = look::icon_button(
                ui,
                "plus",
                "Splice the chosen node in front of the height output",
                has_project && idle,
            );
            if add.clicked() {
                act = Some((
                    "graph.splice",
                    json!({ "layer": "height", "kind": self.stack.kind }),
                ));
            }
            let k = &mut self.stack.kind;
            egui::ComboBox::from_id_salt("layer-add-kind")
                .width(ui.available_width())
                .selected_text(RichText::new(short_kind(k)).size(12.0))
                .show_ui(ui, |ui| {
                    for kind in &kinds {
                        ui.selectable_value(k, kind.clone(), short_kind(kind));
                    }
                })
                .response
                .on_hover_text(
                    "Node kind to add on top of the stack (in front of the height output)",
                )
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::accesskit::Role::ComboBox, true, "Layer kind")
                });
        });
        // The search box (`Search layers`), with its magnifier.
        egui::Frame::new()
            .fill(color::BG2)
            .stroke(Stroke::new(1.0, egui::Color32::from_white_alpha(13)))
            .corner_radius(6.0)
            .inner_margin(Margin::symmetric(6, 2))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Image::new(look::icon("search"))
                            .fit_to_exact_size(vec2(12.0, 12.0))
                            .tint(color::FG2),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.stack.query)
                            .frame(egui::Frame::NONE)
                            .margin(Margin::ZERO)
                            .desired_width(ui.available_width())
                            .font(egui::FontId::proportional(12.0))
                            .hint_text("Search layers"),
                    )
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::accesskit::Role::TextInput,
                            true,
                            "Search layers",
                        )
                    });
                });
            });
        let q = self.stack.query.trim().to_lowercase();
        let matches = |n: &Value| {
            q.is_empty()
                || n["kind"]
                    .as_str()
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains(&q)
                || n["ui"]["label"]
                    .as_str()
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains(&q)
        };
        let selected = self.selected_node.clone();
        let mut select: Option<String> = None;
        let mut row = |ui: &mut egui::Ui,
                       n: &Value,
                       i: usize,
                       in_chain: bool,
                       len: usize,
                       act: &mut Option<(&str, Value)>| {
            let id = n["id"].as_str().unwrap_or_default().to_owned();
            let kind = n["kind"].as_str().unwrap_or_default();
            let bypass = n["bypass"].as_bool().unwrap_or(false);
            let cat = category_of(kind);
            let fill = if selected.as_deref() == Some(id.as_str()) {
                color::BG2
            } else {
                egui::Color32::TRANSPARENT
            };
            egui::Frame::new()
                .fill(fill)
                .corner_radius(4.0)
                .inner_margin(Margin::symmetric(4, 1))
                .show(ui, |ui| {
                    if bypass {
                        ui.set_opacity(0.5);
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let eye =
                            small_icon_button(ui, if bypass { "eye-off" } else { "eye" }, true)
                                .on_hover_text(if bypass {
                                    "Bypassed: click to apply this layer again"
                                } else {
                                    "Bypass: pass the input through unchanged (graph.set_bypass)"
                                });
                        let name = format!(
                            "{} {}",
                            if bypass { "Apply" } else { "Bypass" },
                            label_of(n)
                        );
                        eye.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::accesskit::Role::Button,
                                true,
                                !bypass,
                                &name,
                            )
                        });
                        if eye.clicked() {
                            *act =
                                Some(("graph.set_bypass", json!({ "id": id, "bypass": !bypass })));
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            if in_chain {
                                let fixed =
                                    kind.starts_with("jd.noise.") || kind.starts_with("jd.input.");
                                let rm = small_icon_button(ui, "trash-2", idle && !fixed)
                                    .on_hover_text("Remove this layer and close the gap");
                                rm.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::accesskit::Role::Button,
                                        idle && !fixed,
                                        format!("Remove {}", label_of(n)),
                                    )
                                });
                                if rm.clicked() {
                                    *act = Some(("graph.unsplice", json!({ "id": id })));
                                }
                                let down = small_icon_button(ui, "arrow-down", idle && i + 1 < len)
                                    .on_hover_text("Apply earlier (move down the stack)");
                                down.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::accesskit::Role::Button,
                                        idle && i + 1 < len,
                                        format!("Move {} down", label_of(n)),
                                    )
                                });
                                if down.clicked() {
                                    *act = Some((
                                        "graph.swap_in_chain",
                                        json!({ "id": id, "dir": "down" }),
                                    ));
                                }
                                let up = small_icon_button(ui, "arrow-up", idle && i > 0)
                                    .on_hover_text("Apply later (move up the stack)");
                                up.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::accesskit::Role::Button,
                                        idle && i > 0,
                                        format!("Move {} up", label_of(n)),
                                    )
                                });
                                if up.clicked() {
                                    *act = Some((
                                        "graph.swap_in_chain",
                                        json!({ "id": id, "dir": "up" }),
                                    ));
                                }
                            }
                            badge(ui, cat);
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                let mut text = egui::text::LayoutJob::default();
                                text.append(
                                    &label_of(n),
                                    0.0,
                                    egui::TextFormat {
                                        font_id: egui::FontId::proportional(12.0),
                                        color: color::FG0,
                                        ..Default::default()
                                    },
                                );
                                if let Some(r) = readout_of(n, &nodes, &edges) {
                                    text.append(
                                        &r,
                                        4.0,
                                        egui::TextFormat {
                                            font_id: egui::FontId::proportional(10.0),
                                            color: color::FG2,
                                            ..Default::default()
                                        },
                                    );
                                }
                                let r = ui
                                    .add(
                                        egui::Label::new(text)
                                            .truncate()
                                            .sense(egui::Sense::click()),
                                    )
                                    .on_hover_text(kind);
                                if r.clicked() {
                                    select = Some(id.clone());
                                }
                            });
                        });
                    });
                });
        };
        let head = |ui: &mut egui::Ui,
                    closed: &mut std::collections::HashSet<String>,
                    key: &str,
                    label: &str,
                    count: usize| {
            let shut = closed.contains(key);
            let r = ui
                .horizontal(|ui| {
                    ui.add(
                        egui::Image::new(look::icon(if shut {
                            "chevron-right"
                        } else {
                            "chevron-down"
                        }))
                        .fit_to_exact_size(vec2(12.0, 12.0))
                        .tint(color::FG1),
                    );
                    ui.label(RichText::new(label).size(11.0).strong().color(color::FG1));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        egui::Frame::new()
                            .fill(color::BG3)
                            .corner_radius(3.0)
                            .inner_margin(Margin::symmetric(4, 0))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(count.to_string())
                                        .size(10.0)
                                        .color(color::FG2),
                                );
                            });
                    });
                })
                .response
                .interact(egui::Sense::click());
            r.widget_info(|| {
                egui::WidgetInfo::selected(egui::accesskit::Role::Button, true, !shut, label)
            });
            if r.clicked() {
                if shut {
                    closed.remove(key);
                } else {
                    closed.insert(key.to_owned());
                }
            }
        };
        let mut closed = std::mem::take(&mut self.stack.closed);
        head(ui, &mut closed, "stack", "Height stack", chain.len());
        if !closed.contains("stack") {
            for (i, n) in chain.iter().enumerate() {
                if matches(n) {
                    row(ui, n, i, true, chain.len(), &mut act);
                }
            }
        }
        for (cat, label) in [
            ("shape", "Shape layers"),
            ("filter", "Filters"),
            ("simulation", "Simulation layers"),
            ("mask", "Mask layers"),
        ] {
            let rows: Vec<&Value> = others
                .iter()
                .filter(|n| category_of(n["kind"].as_str().unwrap_or_default()) == cat)
                .collect();
            if rows.is_empty() {
                continue;
            }
            head(ui, &mut closed, cat, label, rows.len());
            if !closed.contains(cat) {
                ui.indent(cat, |ui| {
                    for (i, n) in rows.iter().enumerate() {
                        if matches(n) {
                            row(ui, n, i, false, rows.len(), &mut act);
                        }
                    }
                });
            }
        }
        self.stack.closed = closed;
        if let Some(id) = select {
            // The Editor's inspector follows over the protocol's `selection` event.
            self.server.emit(
                "selection",
                json!({ "nodes": [id], "source": "gui-studio" }),
            );
            self.selected_node = Some(id);
        }
        if let Some((op, params)) = act {
            let removed = (op == "graph.unsplice")
                .then(|| params["id"].as_str().map(str::to_owned))
                .flatten();
            let lod = self.preview_lod();
            let request = Self::request(op, params);
            self.chain(vec![
                Box::new(move |_| Some(request)),
                Box::new(move |_| Some(Self::request("graph.evaluate", json!({ "lod": lod })))),
            ]);
            if removed.is_some() && removed == self.selected_node {
                self.selected_node = None;
            }
        }
    }

    /// The node selected here or in the Editor (`selection` events cross both ways).
    pub fn selected_node(&self) -> Option<&str> {
        self.selected_node.as_deref()
    }

    /// The grid tool's selected cells: for tests.
    pub fn grid_selection(&self) -> &[[u32; 2]] {
        &self.grid.selected
    }

    /// Cave `i`'s entrances as the graph holds them (metres east, north): for tests.
    pub fn cave_entrances(&self, i: usize) -> Vec<[f64; 2]> {
        self.cave_nodes()
            .get(i)
            .and_then(|n| n["params"]["entrances"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .map(|e| [e[0].as_f64().unwrap_or(0.0), e[1].as_f64().unwrap_or(0.0)])
            .collect()
    }

    /// The desktop's `startCaveEntrance`: clicks on the terrain add entrances to cave `id`.
    fn start_placing(&mut self, id: String) {
        self.drawing = Some(Drawing::Cave(id));
    }

    /// Back to orbiting: no ground tool.
    fn stop_placing(&mut self) {
        self.drawing = None;
    }

    /// The desktop's `startSculpting`: the strokes node in front of the height output (found or
    /// spliced by the core), then the brush paints into it; the grid tool goes off.
    fn start_sculpting(&mut self, layer: &str, clamp: Option<[f64; 2]>) {
        let mut params = json!({ "layer": layer, "kind": "strokes" });
        if let Some(c) = clamp {
            params["clamp"] = json!(c);
        }
        self.grid.on = false;
        self.invoke(Tag::SculptTarget, "graph.sculpt_target", params);
    }

    /// The ground tool's ring (metres) for the current drawing mode: the brush radius, half the
    /// road's width, or max(4 m, 0.6 % of the world's width) for a cave entrance.
    fn tool_radius(&self) -> Option<f32> {
        let width = self.viewport.info().map_or(1000.0, |i| i.extent_m[0]);
        match self.drawing.as_ref()? {
            Drawing::Sculpt => Some(self.brush.radius_m as f32),
            Drawing::Road => Some(ROAD_WIDTH_M / 2.0),
            Drawing::Cave(_) => Some((width * 0.006).max(4.0)),
        }
    }

    /// A stroke drawn with the armed tool (the desktop's `addStroke`): a cave entrance (its last
    /// point), a road (one drag, a spline node in front of the height output, then back to
    /// orbiting), a transform (rotate / scale / tilt / push / align into the transforms node), or
    /// a brush stroke into the strokes node — each followed by an evaluation.
    fn on_stroke(&mut self, stroke: &[[f32; 2]]) {
        let r1 = |v: f32| (f64::from(v) * 10.0).round() / 10.0;
        let Some(drawing) = self.drawing.clone() else {
            return;
        };
        let Some(&last) = stroke.last() else { return };
        let mut steps: Vec<Step> = Vec::new();
        match drawing {
            Drawing::Cave(id) => {
                let r = Self::request(
                    "graph.append_param",
                    json!({ "id": id, "key": "entrances", "value": [r1(last[0]), r1(last[1])] }),
                );
                steps.push(Box::new(move |_| Some(r)));
            }
            Drawing::Road => {
                self.drawing = None;
                if stroke.len() < 2 {
                    return;
                }
                let points: Vec<[f64; 2]> = stroke.iter().map(|p| [r1(p[0]), r1(p[1])]).collect();
                let r = Self::request(
                    "graph.splice",
                    json!({ "layer": "height", "kind": "jd.terrain.spline@1", "params": { "points": points, "width_m": ROAD_WIDTH_M } }),
                );
                steps.push(Box::new(move |_| Some(r)));
            }
            Drawing::Sculpt => {
                let b = self.brush.clone();
                if let Some(kind) = b.transform() {
                    // The drag's start is the centre; its end gives the push vector and the
                    // direction a tilt rises toward.
                    let p0 = stroke[0];
                    let d = [last[0] - p0[0], last[1] - p0[1]];
                    let azimuth = (f64::from(d[0].atan2(d[1])).to_degrees() + 360.0)
                        .rem_euclid(360.0)
                        .round();
                    let op = json!({
                        "kind": kind,
                        "center_m": [r1(p0[0]), r1(p0[1])],
                        "radius_m": b.radius_m,
                        "shape": "round",
                        "falloff": ((1.0 - b.hardness) * 100.0).round() / 100.0,
                        "angle_deg": if kind == "tilt" { b.tilt_deg } else { b.angle_deg },
                        "azimuth_deg": azimuth,
                        "factor": b.factor,
                        "vector_m": [r1(d[0]), r1(d[1])],
                        "strength": b.amount,
                    });
                    let target = Self::request(
                        "graph.sculpt_target",
                        json!({ "layer": "height", "kind": "transforms" }),
                    );
                    steps.push(Box::new(move |_| Some(target)));
                    steps.push(Box::new(move |done| {
                        let id = done[0].result.as_ref()?["id"].clone();
                        Some(Self::request(
                            "graph.append_param",
                            json!({ "id": id, "key": "ops", "value": op }),
                        ))
                    }));
                } else {
                    let Some(id) = self.sculpt_node.clone() else {
                        return;
                    };
                    let lower = b.mode == "lower";
                    let mut value = json!({
                        "points": stroke.iter().map(|p| [r1(p[0]), r1(p[1])]).collect::<Vec<_>>(),
                        "radius_m": b.radius_m,
                        "strength_m": if lower { -b.strength_m.abs() } else { b.strength_m },
                        "mode": if lower { "raise" } else { b.mode.as_str() },
                        "amount": b.amount,
                        "hardness": b.hardness,
                    });
                    if b.mode == "flatten"
                        && let Some(t) = b.target_m
                    {
                        value["target_m"] = json!(t);
                    }
                    if b.alpha != "plain" {
                        value["alpha"] = json!(b.alpha);
                    }
                    // One journal entry per stroke (graph.append_param), not the whole array.
                    let r = Self::request(
                        "graph.append_param",
                        json!({ "id": id, "key": "strokes", "value": value }),
                    );
                    steps.push(Box::new(move |_| Some(r)));
                }
            }
        }
        let lod = self.preview_lod();
        steps.push(Box::new(move |_| {
            Some(Self::request("graph.evaluate", json!({ "lod": lod })))
        }));
        self.chain(steps);
    }

    /// The right column's Brush section, shown while a ground tool is armed (the desktop's
    /// `sculpting`): mode, its transform fields, the alpha, radius, strength or amount, the
    /// flatten height, hardness, the strokes node's layer settings, and the shortcuts.
    fn brush_card(&mut self, ui: &mut egui::Ui) {
        use crate::widgets::Range;
        look::section(ui, "Brush");
        let b = &mut self.brush;
        look::field(ui, "Mode", |ui| {
            egui::ComboBox::from_id_salt("brush-mode")
                .width(144.0)
                .selected_text(
                    RichText::new(if b.mode == "push" {
                        "push / pull"
                    } else {
                        b.mode.as_str()
                    })
                    .size(12.0),
                )
                .show_ui(ui, |ui| {
                    for m in [
                        "raise", "lower", "smooth", "flatten", "rotate", "scale", "tilt", "push",
                        "align",
                    ] {
                        ui.selectable_value(
                            &mut b.mode,
                            m.to_owned(),
                            if m == "push" { "push / pull" } else { m },
                        );
                    }
                })
                .response
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::accesskit::Role::ComboBox, true, "Brush mode")
                });
        });
        let number_field = |ui: &mut egui::Ui, label: &str, v: &mut f64, tip: &str| {
            look::field(ui, label, |ui| {
                let mut text = number(&json!(*v));
                let r = look::input(ui, &mut text, 112.0, "", label);
                let r = if tip.is_empty() {
                    r
                } else {
                    r.on_hover_text(tip)
                };
                if r.changed() {
                    *v = text.trim().parse().unwrap_or(0.0);
                }
            });
        };
        match b.mode.as_str() {
            "rotate" => number_field(
                ui,
                "Angle (°)",
                &mut b.angle_deg,
                "Click (or drag) on the terrain: the footprint turns this much counter-clockwise about where you pressed",
            ),
            "scale" => number_field(
                ui,
                "Factor",
                &mut b.factor,
                "Relief about the footprint's mean: 0 levels it, 2 doubles it",
            ),
            "tilt" => number_field(
                ui,
                "Tilt (°)",
                &mut b.tilt_deg,
                "Drag on the terrain: the footprint tilts this much, rising in the drag's direction",
            ),
            _ => {}
        }
        if b.transform().is_some() {
            let hint = match b.mode.as_str() {
                "push" => "Drag: the terrain under the brush slides with the drag (grab).",
                "align" => "Click: the footprint is levelled onto its own best-fit plane.",
                _ => {
                    "Press where the footprint is centred; each drag is one transform in a jd.sculpt.transforms node."
                }
            };
            ui.add(egui::Label::new(RichText::new(hint).size(11.0).color(color::FG2)).wrap());
        }
        // The alphas: seven square buttons, the chosen one outlined in the accent.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let side = ((ui.available_width() - 6.0 * 4.0) / 7.0).floor();
            for (k, icon, title) in [
                ("plain", "circle", "Plain: the smooth round falloff"),
                (
                    "noise",
                    "sparkles",
                    "Noise: fractal value noise, 0.25..1 of the stroke",
                ),
                (
                    "crater",
                    "circle-dot",
                    "Crater: a raised rim round a sunken bowl",
                ),
                (
                    "ridges",
                    "waves-vertical",
                    "Ridges: parallel bands across the stroke",
                ),
                ("rocks", "gem", "Rocks: rounded rock bumps"),
                ("dots", "grip", "Dots: scattered round dots"),
                (
                    "checker",
                    "grid-2x-2",
                    "Checker: full and quarter strength squares",
                ),
            ] {
                let on = b.alpha == k;
                let (rect, r) = ui.allocate_exact_size(vec2(side, side), egui::Sense::click());
                let fill = if on { color::BG3 } else { color::BG2 };
                let stroke = if on {
                    Stroke::new(1.0, color::ACCENT)
                } else {
                    Stroke::new(1.0, egui::Color32::from_white_alpha(13))
                };
                ui.painter()
                    .rect(rect, 6.0, fill, stroke, egui::StrokeKind::Inside);
                let tint = if on || r.hovered() {
                    color::FG0
                } else {
                    color::FG2
                };
                egui::Image::new(look::icon(icon)).tint(tint).paint_at(
                    ui,
                    egui::Rect::from_center_size(rect.center(), vec2(14.0, 14.0)),
                );
                let r = r.on_hover_text(title);
                r.widget_info(|| {
                    egui::WidgetInfo::selected(egui::accesskit::Role::RadioButton, true, on, title)
                });
                if r.clicked() {
                    b.alpha = k.to_owned();
                }
            }
        });
        ui.add_space(4.0);
        number_field(ui, "Radius (m)", &mut b.radius_m, "");
        if b.mode == "raise" || b.mode == "lower" {
            number_field(ui, "Strength (m)", &mut b.strength_m, "");
        } else {
            number_field(ui, "Amount (0–1)", &mut b.amount, "");
        }
        if b.mode == "flatten" {
            look::field(ui, "Height (m)", |ui| {
                let mut text = b.target_m.map(|t| number(&json!(t))).unwrap_or_default();
                let r = look::input(ui, &mut text, 112.0, "under 1st point", "Height (m)")
                    .on_hover_text("Flatten to this height; empty = the height under the first point of the stroke");
                if r.changed() {
                    b.target_m = text.trim().parse().ok();
                }
            });
        }
        let label = format!("Hardness {:.2}", b.hardness);
        look::field(ui, &label, |ui| {
            ui.add(
                Range::new(&mut b.hardness, 0.0..=1.0, &label)
                    .step(0.05)
                    .reset(0.0),
            )
            .on_hover_text("0 = soft falloff over the whole radius, 1 = hard edge");
        });
        let what = match b.mode.as_str() {
            "raise" => "Drag on the terrain to raise it.",
            "lower" => "Drag on the terrain to lower it.",
            "smooth" => "Drag to blend heights towards their local mean.",
            _ => "Drag to pull heights towards the target height.",
        };
        ui.add(
            egui::Label::new(
                RichText::new(format!("{what} Each stroke is one undo step."))
                    .size(12.0)
                    .color(color::FG2),
            )
            .wrap(),
        );
        // The strokes node's layer settings (committed on leaving a field), and Fit to terrain.
        let node = self.sculpt_node.as_ref().and_then(|id| {
            self.graph["nodes"]
                .as_array()?
                .iter()
                .find(|n| n["id"] == id.as_str())
                .cloned()
        });
        let mut set: Option<(&str, Value)> = None;
        if let Some(n) = &node {
            egui::Frame::new()
                .fill(color::BG2.gamma_multiply(0.6))
                .corner_radius(8.0)
                .inner_margin(Margin::same(8))
                .show(ui, |ui| {
                    ui.label(RichText::new("Layer settings").font(crate::theme::semibold(12.5)).color(color::FG0));
                    for (key, label, default, tip) in [
                        ("height_scale", "Height scale", 1.0, "Multiplies what the strokes changed (0 hides the layer)"),
                        ("height_offset_m", "Height offset (m)", 0.0, "Added wherever the layer applies"),
                        ("blending", "Blending", 1.0, "Opacity of the whole layer, 0..1"),
                        ("level_step_m", "Level step (m)", 0.0, "Quantise the change into steps of this height (0 = off)"),
                    ] {
                        let current = number(&json!(n["params"][key].as_f64().unwrap_or(default)));
                        look::field(ui, label, |ui| {
                            if let Some(v) = look::commit_input(ui, &current, 96.0, "", label, tip)
                                .and_then(|t| t.parse::<f64>().ok())
                            {
                                set = Some((key, json!(v)));
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        let placed = !n["params"]["area"].is_null();
                        ui.label(RichText::new(if placed { "area: placed" } else { "area: whole terrain" }).size(11.0).color(color::FG2));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if look::button(ui, None, "Fit to terrain", true, placed)
                                .on_hover_text("Apply the layer over the whole terrain (clears the area; place one in the inspector)")
                                .clicked()
                            {
                                set = Some(("area", Value::Null));
                            }
                        });
                    });
                });
        }
        if let (Some((key, value)), Some(n)) = (set, node) {
            let lod = self.preview_lod();
            let r = Self::request(
                "graph.set_param",
                json!({ "id": n["id"], "key": key, "value": value }),
            );
            self.chain(vec![
                Box::new(move |_| Some(r)),
                Box::new(move |_| Some(Self::request("graph.evaluate", json!({ "lod": lod })))),
            ]);
        }
        ui.add_space(6.0);
        egui::Grid::new("brush-shortcuts")
            .num_columns(2)
            .spacing(vec2(8.0, 0.0))
            .show(ui, |ui| {
                for (k, v) in [
                    ("Left drag", "stroke"),
                    ("Right drag", "orbit"),
                    ("Ctrl + wheel", "radius"),
                    ("Shift + wheel", "strength"),
                ] {
                    ui.label(RichText::new(k).size(11.0).color(color::FG2));
                    ui.label(RichText::new(v).size(11.0).color(color::FG2));
                    ui.end_row();
                }
            });
        ui.add_space(12.0);
    }

    /// The right column's Grid section, while the grid tool is on (the desktop's): cells a side,
    /// the selection, smoothness, Set height / Clear, Raise / Lower by a step, the selection's
    /// transforms and the shortcuts.
    fn grid_card(&mut self, ui: &mut egui::Ui) {
        use crate::widgets::Range;
        look::section(ui, "Grid");
        let idle = self.busy == 0;
        let g = &mut self.grid;
        let has = !g.selected.is_empty();
        let mut act: Option<&str> = None;
        look::field(ui, "Cells per side", |ui| {
            let before = g.cells;
            egui::ComboBox::from_id_salt("grid-cells")
                .width(80.0)
                .selected_text(RichText::new(g.cells.to_string()).size(12.0))
                .show_ui(ui, |ui| {
                    for n in [8, 16, 32, 64] {
                        ui.selectable_value(&mut g.cells, n, n.to_string());
                    }
                })
                .response
                .widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::accesskit::Role::ComboBox,
                        true,
                        "Cells per side",
                    )
                });
            if g.cells != before {
                g.selected.clear();
            }
        });
        let n = g.selected.len();
        look::field(ui, "Selected", |ui| {
            ui.label(
                RichText::new(format!("{n} cell{}", if n == 1 { "" } else { "s" }))
                    .size(12.0)
                    .color(color::FG1),
            );
        });
        let label = format!("Smoothness {:.2}", g.smoothness);
        look::field(ui, &label, |ui| {
            ui.add(
                Range::new(&mut g.smoothness, 0.0..=1.0, &label)
                    .step(0.05)
                    .reset(0.25),
            )
            .on_hover_text(
                "How far the selection's edge blends into its surroundings (0 = hard cell edges)",
            );
        });
        let num = |ui: &mut egui::Ui, label: &str, v: &mut f64, width: f32| {
            let mut text = number(&json!(*v));
            if look::input(ui, &mut text, width, "", label).changed() {
                *v = text.trim().parse().unwrap_or(0.0);
            }
        };
        look::field(ui, "Height (m)", |ui| {
            num(ui, "Height (m)", &mut g.height_m, 112.0)
        });
        ui.horizontal(|ui| {
            if look::button(ui, None, "Set height", false, has && idle)
                .on_hover_text("Level the selected cells to the height above (one undoable stroke)")
                .clicked()
            {
                act = Some("set");
            }
            if look::button(ui, None, "Clear", false, has).clicked() {
                act = Some("clear");
            }
        });
        look::field(ui, "Step (m)", |ui| {
            num(ui, "Step (m)", &mut g.step_m, 112.0)
        });
        ui.horizontal(|ui| {
            if look::button(ui, None, "Raise", false, has && idle).clicked() {
                act = Some("raise");
            }
            if look::button(ui, None, "Lower", false, has && idle).clicked() {
                act = Some("lower");
            }
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new("Transform selection")
                .size(11.0)
                .strong()
                .color(color::FG1),
        );
        egui::Grid::new("grid-transform-fields")
            .num_columns(4)
            .spacing(vec2(4.0, 4.0))
            .show(ui, |ui| {
                for row in [["Angle", "Factor"], ["Tilt", "Toward"]] {
                    for label in row {
                        ui.label(RichText::new(label).size(11.0).color(color::FG2));
                        let v = match label {
                            "Angle" => &mut g.angle_deg,
                            "Factor" => &mut g.factor,
                            "Tilt" => &mut g.tilt_deg,
                            _ => &mut g.tilt_azimuth_deg,
                        };
                        let r = {
                            let mut text = number(&json!(*v));
                            let r = look::input(ui, &mut text, 64.0, "", label);
                            if r.changed() {
                                *v = text.trim().parse().unwrap_or(0.0);
                            }
                            r
                        };
                        if label == "Toward" {
                            r.on_hover_text(
                                "Direction the tilt rises toward, degrees clockwise from north",
                            );
                        }
                    }
                    ui.end_row();
                }
            });
        ui.horizontal_wrapped(|ui| {
            for (k, label, title) in [
                ("duplicate", "Duplicate", "Copy the selected block one selection-width east"),
                ("rotate", "Rotate", "Turn the selected block by Angle (counter-clockwise)"),
                ("scale", "Scale", "Scale the block's relief about its mean by Factor"),
                ("tilt", "Tilt", "Tilt the block by Tilt degrees, rising toward Toward"),
                ("align", "Align", "Level the block onto its own best-fit plane"),
            ] {
                if look::button(ui, None, label, false, has && idle)
                    .on_hover_text(format!("{title} — appended to a jd.sculpt.transforms node; Raise / Lower above are World Creator's Push / Pull"))
                    .clicked()
                {
                    act = Some(k);
                }
            }
        });
        egui::Grid::new("grid-shortcuts")
            .num_columns(2)
            .spacing(vec2(8.0, 0.0))
            .show(ui, |ui| {
                for (k, v) in [
                    ("Click", "toggle a cell"),
                    ("Left drag", "add cells"),
                    ("Alt + drag", "remove cells"),
                    ("Right drag", "orbit"),
                ] {
                    ui.label(RichText::new(k).size(11.0).color(color::FG2));
                    ui.label(RichText::new(v).size(11.0).color(color::FG2));
                    ui.end_row();
                }
            });
        ui.add_space(12.0);
        match act {
            Some("clear") => self.grid.selected.clear(),
            Some(k @ ("set" | "raise" | "lower")) => self.grid_apply(k),
            Some(k) => self.grid_transform(k),
            None => {}
        }
    }

    /// The desktop's gridApply: one square stamp per selected cell in the strokes node — one
    /// stroke, one undo step — levelling to Height or raising / lowering by Step.
    fn grid_apply(&mut self, kind: &str) {
        let Some(extent) = self.viewport.info().map(|i| i.extent_m) else {
            return;
        };
        let g = &self.grid;
        let (cw, ch) = (
            f64::from(extent[0]) / f64::from(g.cells),
            f64::from(extent[1]) / f64::from(g.cells),
        );
        let r1 = |v: f64| (v * 10.0).round() / 10.0;
        let points: Vec<[f64; 2]> = g
            .selected
            .iter()
            .map(|c| {
                [
                    r1((f64::from(c[0]) + 0.5) * cw),
                    r1((f64::from(c[1]) + 0.5) * ch),
                ]
            })
            .collect();
        let mut value = json!({
            "points": points,
            "radius_m": r1(cw.max(ch) / 2.0),
            "shape": "square",
            "hardness": ((1.0 - g.smoothness) * 100.0).round() / 100.0,
            "amount": 1,
        });
        if kind == "set" {
            value["mode"] = json!("flatten");
            value["target_m"] = json!(g.height_m);
        } else {
            value["mode"] = json!("raise");
            value["strength_m"] = json!(if kind == "raise" { g.step_m } else { -g.step_m });
        }
        self.sculpt_append("strokes", "strokes", value);
    }

    /// The desktop's gridTransform: the selection's bounding square as one block, its op appended
    /// to the transforms node.
    fn grid_transform(&mut self, kind: &str) {
        let Some(extent) = self.viewport.info().map(|i| i.extent_m) else {
            return;
        };
        let g = &self.grid;
        if g.selected.is_empty() {
            return;
        }
        let (cw, ch) = (
            f64::from(extent[0]) / f64::from(g.cells),
            f64::from(extent[1]) / f64::from(g.cells),
        );
        let r1 = |v: f64| (v * 10.0).round() / 10.0;
        let xs = g.selected.iter().map(|c| f64::from(c[0]));
        let ys = g.selected.iter().map(|c| f64::from(c[1]));
        let (x0, x1) = (
            xs.clone().fold(f64::MAX, f64::min),
            xs.fold(f64::MIN, f64::max) + 1.0,
        );
        let (y0, y1) = (
            ys.clone().fold(f64::MAX, f64::min),
            ys.fold(f64::MIN, f64::max) + 1.0,
        );
        let value = json!({
            "kind": kind,
            "center_m": [r1((x0 + x1) / 2.0 * cw), r1((y0 + y1) / 2.0 * ch)],
            "radius_m": r1(((x1 - x0) * cw).max((y1 - y0) * ch) / 2.0),
            "shape": "square",
            "falloff": (g.smoothness * 100.0).round() / 100.0,
            "angle_deg": if kind == "tilt" { g.tilt_deg } else { g.angle_deg },
            "azimuth_deg": g.tilt_azimuth_deg,
            "factor": g.factor,
            // Duplicate: the copy lands one selection-width east.
            "vector_m": if kind == "duplicate" { [r1((x1 - x0) * cw), 0.0] } else { [0.0, 0.0] },
            "strength": 1,
        });
        self.sculpt_append("transforms", "ops", value);
    }

    /// Append `value` to the `kind` sculpt node's `key` (found or spliced by the core), then
    /// evaluate.
    fn sculpt_append(&mut self, kind: &str, key: &'static str, value: Value) {
        let target = Self::request(
            "graph.sculpt_target",
            json!({ "layer": "height", "kind": kind }),
        );
        let lod = self.preview_lod();
        self.chain(vec![
            Box::new(move |_| Some(target)),
            Box::new(move |done| {
                let id = done[0].result.as_ref()?["id"].clone();
                Some(Self::request(
                    "graph.append_param",
                    json!({ "id": id, "key": key, "value": value }),
                ))
            }),
            Box::new(move |_| Some(Self::request("graph.evaluate", json!({ "lod": lod })))),
        ]);
    }

    /// The cave nodes of the graph, in order.
    fn cave_nodes(&self) -> Vec<Value> {
        self.graph["nodes"]
            .as_array()
            .map(|n| {
                n.iter()
                    .filter(|n| {
                        n["kind"]
                            .as_str()
                            .is_some_and(|k| k.starts_with("jd.cave.system@"))
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The desktop's `refreshCaves`: `caves.mesh` for each cave node at the terrain's LOD, on one
    /// worker; none while Show caves is off.
    // ponytail: every cave is re-meshed when the terrain shows; the desktop keeps a cave whose
    // node hash is unchanged — add that when Studio has the evaluation report's hashes.
    fn fetch_caves(&mut self, lod: u32) {
        self.cave_gen += 1;
        let ids: Vec<String> = self
            .cave_nodes()
            .iter()
            .filter_map(|n| n["id"].as_str().map(str::to_owned))
            .collect();
        // The cave being placed is gone (removed, undone): stop placing.
        if matches!(&self.drawing, Some(Drawing::Cave(p)) if !ids.contains(p)) {
            self.stop_placing();
        }
        if !self.show_caves || ids.is_empty() {
            self.caves.clear();
            return self.viewport.set_caves(&[]);
        }
        let (server, tx, ctx, cave_gen) = (
            self.server.clone(),
            self.tx.clone(),
            self.ctx.clone(),
            self.cave_gen,
        );
        self.reads += 1;
        std::thread::spawn(move || {
            let out = ids
                .into_iter()
                .map(|id| {
                    let r =
                        server.invoke(Self::request("caves.mesh", json!({ "id": id, "lod": lod })));
                    let mesh = match (&r.result, &r.error) {
                        (Some(v), _) => cave_mesh(v),
                        (None, Some(e)) => Err(e.message.clone()),
                        (None, None) => Err("caves.mesh failed".into()),
                    };
                    (id, mesh)
                })
                .collect();
            let _ = tx.send(Msg::Caves(cave_gen, out));
            ctx.request_repaint();
        });
    }

    /// One cave's line as the desktop writes it: `1.24 km · 2 entrances · 31 speleothems · water
    /// 120 m`, the error, or `…` while it is meshed.
    fn cave_line(&self, id: &str) -> (String, Vec<String>) {
        match self.caves.iter().find(|c| c.id == id).map(|r| &r.summary) {
            None => ("…".to_owned(), Vec::new()),
            Some(Err(e)) => (e.clone(), Vec::new()),
            Some(Ok(v)) => {
                let mut t = format!(
                    "{:.2} km · {} entrance{}",
                    v.length_m / 1000.0,
                    v.entrances,
                    if v.entrances == 1 { "" } else { "s" }
                );
                if v.speleothems > 0 {
                    t += &format!(" · {} speleothems", v.speleothems);
                }
                if let Some(w) = v.water_level_m {
                    t += &format!(" · water {w:.0} m");
                }
                (t, v.notes.clone())
            }
        }
    }

    /// The right column's Caves section (the desktop's, in its order).
    fn caves_card(&mut self, ui: &mut egui::Ui) {
        look::section(ui, "Caves");
        let can_add = self.project.is_some() && self.busy == 0;
        let mut add = false;
        ui.horizontal(|ui| {
            let mut k = self.cave_kind;
            egui::ComboBox::from_id_salt("cave-kind")
                .width(144.0)
                .selected_text(RichText::new(CAVE_KINDS[k].1).size(12.0))
                .show_ui(ui, |ui| {
                    for (i, (_, label)) in CAVE_KINDS.iter().enumerate() {
                        ui.selectable_value(&mut k, i, *label);
                    }
                })
                .response
                .on_hover_text("Kind of cave: what drives where it goes and its shape (docs/research/caves.md)")
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::accesskit::Role::ComboBox, true, "Cave kind")
                });
            self.cave_kind = k;
            add = look::button(ui, None, "Add cave", false, can_add)
                .on_hover_text("Add a cave system (a jd.cave.system node in front of the height output; its entrance holes and plan are published as cave_holes / cave_plan)")
                .clicked();
        });
        if add {
            self.add_layer(crate::chrome::Add::Cave(CAVE_KINDS[self.cave_kind].0));
        }
        let mut show = self.show_caves;
        look::field(ui, "Show caves", |ui| {
            look::checkbox(ui, &mut show, "Show caves").on_hover_text(
                "Draw the caves under the terrain (walls, speleothems, entrance markers)",
            );
        });
        if show != self.show_caves {
            self.show_caves = show;
            let lod = self.preview.map_or(0, |(lod, ..)| lod);
            self.fetch_caves(lod);
        }
        let footprint = self.color_layer.as_deref() == Some("cave_plan");
        let has_plan = self.layers.iter().any(|l| l == "cave_plan");
        let mut toggle = false;
        look::field(ui, "Map", |ui| {
            let text = if footprint {
                "Footprint on"
            } else {
                "Footprint"
            };
            let fg = if footprint { color::ACCENT } else { color::FG0 };
            toggle = ui
                .add_enabled(has_plan, egui::Button::new(RichText::new(text).size(12.0).color(fg)))
                .on_hover_text("Colour the terrain with the caves' footprint (the cave_plan layer): where something lies underground")
                .clicked();
        });
        if toggle {
            self.set_color_layer((!footprint).then(|| "cave_plan".to_owned()));
        }
        let label = format!("X-ray {} %", (self.xray * 100.0).round());
        let mut xray = self.xray;
        look::field(ui, &label, |ui| {
            ui.add(
                crate::widgets::Range::new(&mut xray, 0.0..=0.9, &label)
                    .step(0.05)
                    .reset(0.0),
            )
            .on_hover_text("See through the terrain to the caves under it");
        });
        if xray != self.xray {
            self.xray = (xray * 100.0).round() / 100.0;
            self.viewport.set_xray(self.xray as f32);
        }
        // One box per cave node: its line, the exports, the entrances, the generator's notes.
        let busy = self.busy > 0;
        let mut toggle_place: Option<String> = None;
        let mut action: Option<(&str, Value)> = None;
        for (ci, node) in self.cave_nodes().iter().enumerate() {
            let id = node["id"].as_str().unwrap_or_default().to_owned();
            let kind = node["params"]["kind"].as_str().unwrap_or("karst");
            let label = CAVE_KINDS
                .iter()
                .find(|(k, _)| *k == kind)
                .map_or("Cave", |(_, l)| l);
            let (line, notes) = self.cave_line(&id);
            let entrances: Vec<[f64; 2]> = node["params"]["entrances"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|e| [e[0].as_f64().unwrap_or(0.0), e[1].as_f64().unwrap_or(0.0)])
                        .collect()
                })
                .unwrap_or_default();
            egui::Frame::new()
                .stroke(Stroke::new(1.0, egui::Color32::from_white_alpha(13)))
                .corner_radius(6.0)
                .inner_margin(Margin::same(6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            for ext in ["fbx", "glb"] {
                                let tip = if ext == "glb" {
                                    format!("Write exports/cave_{}.glb (walls + speleothems, the terrain export's frame)", ci + 1)
                                } else {
                                    format!("Write exports/cave_{}.fbx (for Unity)", ci + 1)
                                };
                                if look::button(ui, None, &format!(".{ext}"), false, !busy)
                                    .on_hover_text(tip)
                                    .clicked()
                                {
                                    action = Some((
                                        "caves.export",
                                        json!({ "id": id, "path": format!("cave_{}.{ext}", ci + 1) }),
                                    ));
                                }
                            }
                            let placing = self.drawing == Some(Drawing::Cave(id.clone()));
                            let fg = if placing { color::ACCENT } else { color::FG0 };
                            let place = ui
                                .add_enabled(
                                    !busy,
                                    egui::Button::new(
                                        RichText::new(if placing { "Placing…" } else { "Place" })
                                            .size(12.0)
                                            .color(fg),
                                    ),
                                )
                                .on_hover_text(if placing {
                                    "Click the terrain to add entrances; click here to stop"
                                } else {
                                    "Place entrances by clicking the terrain (sinkholes, mouths, portals — a tunnel takes two)"
                                });
                            let name = format!("Place entrances of cave {}", ci + 1);
                            place.widget_info(|| {
                                egui::WidgetInfo::selected(egui::accesskit::Role::Button, !busy, placing, &name)
                            });
                            if place.clicked() {
                                toggle_place = Some(id.clone());
                            }
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!("{label} · {line}")).size(11.0).color(color::FG0),
                                    )
                                    .wrap(),
                                );
                            });
                        });
                    });
                    for (i, e) in entrances.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("{}. {} m E, {} m N", i + 1, e[0].round(), e[1].round()))
                                    .size(11.0)
                                    .color(color::FG1),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let remove = ui
                                    .add_enabled(
                                        !busy,
                                        egui::Button::new(RichText::new("×").size(11.0).color(color::ERROR)),
                                    )
                                    .on_hover_text("Remove this entrance");
                                let name = format!("Remove entrance {} of cave {}", i + 1, ci + 1);
                                remove.widget_info(|| {
                                    egui::WidgetInfo::labeled(egui::accesskit::Role::Button, !busy, &name)
                                });
                                if remove.clicked() {
                                    let rest: Vec<[f64; 2]> = entrances
                                        .iter()
                                        .enumerate()
                                        .filter(|(k, _)| *k != i)
                                        .map(|(_, e)| *e)
                                        .collect();
                                    action = Some((
                                        "graph.set_param",
                                        json!({ "id": id, "key": "entrances", "value": rest }),
                                    ));
                                }
                            });
                        });
                    }
                    for n in &notes {
                        ui.label(RichText::new(n).size(11.0).color(color::WARN));
                    }
                });
            ui.add_space(8.0);
        }
        if let Some(id) = toggle_place {
            if self.drawing == Some(Drawing::Cave(id.clone())) {
                self.stop_placing();
            } else {
                self.start_placing(id);
            }
        }
        if let Some((op, params)) = action {
            // An export writes a file; an entrance edit changes the graph (refresh).
            let tag = if op == "caves.export" {
                Tag::Export
            } else {
                Tag::User
            };
            self.invoke(tag, op, params);
        }
    }

    /// The desktop's `setColorLayer`: the overlay goes until the new layer's preview arrives.
    pub fn set_color_layer(&mut self, name: Option<String>) {
        self.color_layer = name;
        self.viewport.set_overlay(None);
        let lod = self.preview.map_or(0, |(lod, ..)| lod);
        self.fetch_color(lod);
    }

    /// The desktop's `refreshColor`: the colour layer's preview at the terrain's LOD, dropped when
    /// the layer is no longer published.
    fn fetch_color(&mut self, lod: u32) {
        if let Some(pending) = self.pending_color.take_if(|p| self.layers.contains(p)) {
            self.color_layer = Some(pending);
        }
        let Some(name) = self.color_layer.clone() else {
            return self.viewport.set_overlay(None);
        };
        if !self.layers.contains(&name) {
            self.color_layer = None;
            return self.viewport.set_overlay(None);
        }
        let request = Self::request("terrain.preview", json!({ "layer": name, "lod": lod }));
        let (server, tx, ctx) = (self.server.clone(), self.tx.clone(), self.ctx.clone());
        self.reads += 1;
        std::thread::spawn(move || {
            let r = server.invoke(request);
            let pv = match (&r.result, &r.error) {
                (Some(v), _) => decode_preview(v).map(|(p, _)| p),
                (None, Some(e)) => Err(format!("{} {}", e.code.as_str(), e.message)),
                (None, None) => Err("no result".into()),
            };
            let _ = tx.send(Msg::Color(name, pv));
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
        let materials = Value::Array(self.materials.clone());
        let request = Self::request(
            "terrain.textures",
            json!({ "materials": materials, "layer": layer, "max_px": self.preview_max_px }),
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
        if self.project.is_none() {
            return;
        }
        let scatter = self.scatter.json();
        let [az, el, turbidity, clouds, fog] = self.sun.0;
        let params = json!({
            "materials": self.materials,
            "scatter": scatter,
            "sun": { "azimuth_deg": az, "elevation_deg": el, "turbidity": turbidity, "clouds": clouds, "fog": fog },
        });
        self.invoke(Tag::Look, "project.set_look", params);
    }

    /// Fire the debounced look write and sky bake when due.
    fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        // Tasks: poll the engine's jobs every 250 ms while a button's op runs (the desktop's
        // pollers around `run`); clear them when nothing does.
        if self.busy > 0 {
            if self
                .jobs_polled
                .is_none_or(|t| now.duration_since(t).as_millis() >= 250)
            {
                self.jobs_polled = Some(now);
                self.invoke(Tag::Jobs, "jobs.list", json!({}));
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        } else if self.jobs_polled.take().is_some() {
            self.jobs.clear();
        }
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

    /// The desktop's `scatterReq`: heights from `height` when published, else the first layer.
    fn scatter_request(&self) -> Value {
        let height = if self.layers.iter().any(|l| l == "height") {
            "height"
        } else {
            self.layers.first().map_or("height", String::as_str)
        };
        let look = self.scatter.json();
        json!({ "mask": self.scatter.mask.clone().unwrap_or_default(), "height": height, "spacing_m": look["spacing_m"], "seed": look["seed"], "follow_time": self.scatter.grow })
    }

    /// Place: the points for the viewport.
    fn place_scatter(&mut self) {
        if self.scatter.mask.is_some() {
            let params = self.scatter_request();
            self.invoke(Tag::Scatter, "terrain.scatter", params);
        }
    }

    /// The desktop's `refreshScatter`: re-placed while the mask is still published, dropped when
    /// it is not.
    fn refresh_scatter(&mut self) {
        let Some(mask) = &self.scatter.mask else {
            return;
        };
        if self.layers.contains(mask) {
            self.place_scatter();
        } else {
            self.scatter.mask = None;
            self.drop_scatter_points();
        }
    }

    fn drop_scatter_points(&mut self) {
        self.scatter_points = None;
        self.viewport.set_instances(None);
    }

    /// The placed points in the viewport: the 1 m proxy cone (rocks for rock-named masks), sized
    /// max(spacing · 0.35, extent · 0.012) as the desktop.
    fn show_scatter(&mut self) {
        let extent = self
            .project
            .as_ref()
            .and_then(|p| p["world"]["extent_m"][0].as_f64())
            .unwrap_or(0.0);
        let spacing = self.scatter.json()["spacing_m"].as_f64().unwrap_or(0.0);
        let mask = self
            .scatter
            .mask
            .as_deref()
            .unwrap_or_default()
            .to_lowercase();
        let instances = self
            .scatter_points
            .as_ref()
            .map(|points| crate::viewport::Instances {
                points: points.clone(),
                size_m: (spacing * 0.35).max(extent * 0.012) as f32,
                rock: ["debris", "rock", "boulder", "scree", "talus"]
                    .iter()
                    .any(|w| mask.contains(w)),
            });
        self.viewport.set_instances(instances.as_ref());
    }

    /// The right column's Scatter section (the desktop's, in its order).
    fn scatter_card(&mut self, ui: &mut egui::Ui) {
        look::section(ui, "Scatter");
        let before = self.scatter.clone();
        let layers = self.layers.clone();
        let sc = &mut self.scatter;
        look::field(ui, "Mask", |ui| {
            egui::ComboBox::from_id_salt("scatter-mask")
                .width(112.0)
                .selected_text(RichText::new(sc.mask.as_deref().unwrap_or("none")).size(12.0))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut sc.mask, None, "none");
                    for l in &layers {
                        ui.selectable_value(&mut sc.mask, Some(l.clone()), l);
                    }
                })
                .response
                .on_hover_text(
                    "0..1 layer used as the instance density (a class mask, rivers, wetness…)",
                )
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::accesskit::Role::ComboBox, true, "Mask")
                });
        });
        look::field(ui, "Spacing (m)", |ui| {
            look::input(ui, &mut sc.spacing_m, 112.0, "", "Spacing (m)");
        });
        look::field(ui, "Seed", |ui| {
            look::input(ui, &mut sc.seed, 112.0, "", "Seed");
        });
        look::field(ui, "Grow with timeline", |ui| {
            look::checkbox(ui, &mut sc.grow, "Grow with timeline").on_hover_text("The population follows the timeline: nobody at t = 0, saplings as it advances, full-grown by t = 1; off = always full-grown");
        });
        if self.scatter != before {
            // The desktop's setScatter: the points go, the look is written after the debounce.
            self.drop_scatter_points();
            if self.project.is_some() {
                self.look_due = Some(Instant::now() + std::time::Duration::from_millis(400));
            }
        }
        let ready = self.scatter.mask.is_some() && self.busy == 0;
        let placed = self.scatter_points.as_ref().map(Vec::len);
        let (mut place, mut export, mut paint) = (false, false, false);
        ui.horizontal_wrapped(|ui| {
            place = look::button(ui, None, "Place", false, ready)
                .on_hover_text("Place instances from the mask (one jittered candidate per spacing cell, kept with probability = mask value)")
                .clicked();
            paint = look::button(ui, None, "Paint density", false, ready).on_hover_text("Paint the density by hand: a clamped stroke node goes in front of the mask's output; drag on the terrain to add density (strength 0.5), negative strength to thin it").clicked();
            look::input(ui, &mut self.scatter_model, 96.0, "pine.fbx", "Model").on_hover_text("Your own model to instance: a binary FBX in the project's assets/ (metres, +Y up; import it with ⌘I). Empty = the 1 m proxy, or a generated rock for rock-named masks");
            export = look::button(ui, None, "Export .glb", false, placed.is_some_and(|n| n > 0) && self.busy == 0)
                .on_hover_text("Write exports/<mask>.glb: the model (or the 1 m proxy) instanced with EXT_mesh_gpu_instancing")
                .clicked();
            if let Some(n) = placed {
                ui.label(RichText::new(format!("{} instances", thousands(n as u64))).size(12.0).color(color::FG2));
            }
        });
        if place {
            self.place_scatter();
        }
        // The desktop's paintDensity: the mask's strokes node, clamped to 0..1, a 0.5 m raise.
        if paint && let Some(mask) = self.scatter.mask.clone() {
            self.brush.mode = "raise".into();
            self.brush.strength_m = 0.5;
            self.start_sculpting(&mask, Some([0.0, 1.0]));
        }
        if export && let Some(mask) = self.scatter.mask.clone() {
            let mut params = self.scatter_request();
            params["path"] = json!(format!("{mask}.glb"));
            let model = self.scatter_model.trim();
            if !model.is_empty() {
                params["model"] = json!(model);
            }
            self.invoke(Tag::Export, "terrain.scatter", params);
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
                let picker = if self.layers.len() > 1 { 400.0 } else { 0.0 };
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
                    // Right to left: the colour picker, then the layer picker.
                    let mut colour = self.color_layer.clone();
                    egui::ComboBox::from_id_salt("color-pick")
                        .selected_text(
                            RichText::new(colour.as_deref().unwrap_or("height ramp")).size(12.0),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut colour, None, "height ramp");
                            for l in &self.layers {
                                ui.selectable_value(&mut colour, Some(l.clone()), l);
                            }
                        })
                        .response
                        .on_hover_text("Colour the terrain by another layer (masks and class maps) instead of the height ramp")
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::accesskit::Role::ComboBox,
                                true,
                                "colour",
                            )
                        });
                    ui.label(RichText::new("colour").size(12.0).color(color::FG2));
                    if colour != self.color_layer {
                        self.set_color_layer(colour);
                    }
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
        if let Some(name) = self
            .color_layer
            .as_ref()
            .filter(|_| self.viewport.overlay_on())
        {
            s += &format!(" · colour {name}");
        }
        if self.viewport.instance_count() > 0 {
            s += &format!(
                " · {} instances",
                thousands(self.viewport.instance_count() as u64)
            );
        }
        if self.viewport.cave_triangles() > 0 {
            s += &format!(
                " · caves {} tris",
                thousands(self.viewport.cave_triangles())
            );
        }
        if self.xray > 0.0 {
            s += &format!(" · x-ray {} %", (self.xray * 100.0).round());
        }
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
        {
            self.pick_export_path();
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
        if look::icon_button(ui, "wand-2", "Generate terrain (⌘G)", has_project && idle).clicked()
        {
            self.generate();
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
            Action::Sculpt => {
                if self.drawing.is_some() {
                    self.drawing = None;
                } else {
                    self.start_sculpting("height", None);
                }
            }
            Action::Grid => {
                // The desktop's toggleGrid: on puts the ground tools away.
                self.grid.on = !self.grid.on;
                if self.grid.on {
                    self.drawing = None;
                }
            }
            Action::Road => {
                self.drawing = if self.drawing == Some(Drawing::Road) {
                    None
                } else {
                    Some(Drawing::Road)
                };
            }
        }
    }

    /// An evaluation at the current preview LOD moves the bias (`adapt_lod_bias`); others, e.g.
    /// a full evaluation of a larger world, say nothing about the preview's cost.
    fn adapt_lod(&mut self, report: &Value) {
        if report["lod"].as_u64() != Some(u64::from(self.preview_lod())) {
            return;
        }
        let target = self.preview_target_ms.trim().parse().unwrap_or(0.0);
        let ms = report["duration_ms"].as_f64().unwrap_or(0.0);
        self.lod_bias = adapt_lod_bias(self.lod_bias, ms, target);
    }

    /// The desktop's `previewLod`: live evaluations stay ≤ 512², plus the Live target's bias.
    fn preview_lod(&self) -> u32 {
        let res = self.project.as_ref().map_or(512.0, |p| {
            let r = &p["world"]["resolution_px"];
            r[0].as_f64()
                .unwrap_or(512.0)
                .max(r[1].as_f64().unwrap_or(512.0))
        });
        (res / 512.0).log2().ceil().max(0.0) as u32 + self.lod_bias
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
            Add::Cave(kind) => {
                // A tunnel needs its two portals: placing starts as soon as it is added.
                self.tunnel_pending = kind == "tunnel";
                Self::request("graph.add_cave", json!({ "kind": kind }))
            }
            Add::Action(action) => Self::request(
                "graph.splice",
                json!({ "layer": "height", "kind": "jd.filter.action@1", "params": { "action": action } }),
            ),
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
        self.real_world_row(ui);
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
        ui.add_space(12.0);
        self.reference_card(ui);
        ui.add_space(12.0);
        self.layers_card(ui);
        ui.add_space(12.0);
        self.settings_card(ui);
        ui.add_space(12.0);
        self.tasks_card(ui);
        ui.add_space(12.0);
        self.diagnostics_card(ui);
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
/// `caves.mesh`'s result: the arrays decoded for the viewport, and the panel's summary.
fn cave_mesh(v: &Value) -> Result<(crate::viewport::CaveMesh, CaveSummary), String> {
    use base64::Engine as _;
    let bytes = |m: &Value, k: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(m[k].as_str().unwrap_or_default())
            .map_err(|e| format!("{k}: {e}"))
    };
    let words = |b: Vec<u8>| -> Vec<[u8; 4]> { b.as_chunks::<4>().0.to_vec() };
    let arrays = |m: &Value| -> Result<crate::viewport::MeshArrays, String> {
        Ok(crate::viewport::MeshArrays {
            positions: words(bytes(m, "positions_base64")?)
                .into_iter()
                .map(f32::from_le_bytes)
                .collect(),
            normals: words(bytes(m, "normals_base64")?)
                .into_iter()
                .map(f32::from_le_bytes)
                .collect(),
            indices: words(bytes(m, "indices_base64")?)
                .into_iter()
                .map(u32::from_le_bytes)
                .collect(),
        })
    };
    let water = v["water_level_m"].as_f64();
    let entrances: Vec<([f32; 3], bool)> = v["entrances"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|e| {
                    let at = |i: usize| e["at"][i].as_f64().unwrap_or(0.0) as f32;
                    (
                        [at(0), at(1), at(2)],
                        e["underwater"].as_bool().unwrap_or(false),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let summary = CaveSummary {
        length_m: v["length_m"].as_f64().unwrap_or(0.0),
        entrances: entrances.len(),
        speleothems: v["speleothem_count"].as_u64().unwrap_or(0),
        water_level_m: water,
        notes: v["notes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|n| n.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
    };
    Ok((
        crate::viewport::CaveMesh {
            walls: arrays(&v["walls"])?,
            speleothems: arrays(&v["speleothems"])?,
            water_level_m: water.map(|w| w as f32),
            entrances,
        },
        summary,
    ))
}

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
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "jd.lang", &crate::i18n::lang(&self.ctx));
        eframe::set_value(storage, "jd.previewMaxPx", &self.preview_max_px);
        eframe::set_value(storage, "jd.previewTargetMs", &self.preview_target_ms);
    }

    /// Only the app settings above persist: panel sizes and scroll positions start fresh, as the
    /// desktop's do.
    fn persist_egui_memory(&self) -> bool {
        false
    }

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
                    look::scroll_card(ui, "panel-right", |ui| {
                        if self.drawing.is_some() {
                            self.brush_card(ui);
                        }
                        if self.grid.on {
                            self.grid_card(ui);
                        }
                        self.sun_card(ui, frame);
                        ui.add_space(12.0);
                        self.scatter_card(ui);
                        ui.add_space(12.0);
                        self.caves_card(ui);
                        ui.add_space(12.0);
                        self.actions_card(ui);
                        ui.add_space(12.0);
                        self.materials_card(ui);
                        ui.add_space(12.0);
                        self.noise_card(ui);
                        ui.add_space(16.0);
                        self.export_card(ui);
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
                    look::scroll_card(ui, "panel-left", |ui| self.project_card(ui));
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
        // The ground tool's ring, after the side columns: a Sculpt / Place / Road click there arms
        // (or disarms) it for this very frame's viewport; the grid overlay likewise.
        self.viewport.tool = self.tool_radius();
        self.viewport
            .set_grid(self.grid.on.then(|| crate::viewport::GridView {
                cells: self.grid.cells,
                selected: self.grid.selected.clone(),
            }));
        egui::CentralPanel::default()
            .frame(bg.inner_margin(margin))
            .show(ui, |ui| {
                self.viewport.show(ui, frame.wgpu_render_state());
                self.record_frame(frame);
                if let Some(stroke) = self.viewport.take_stroke() {
                    self.on_stroke(&stroke);
                }
                // The desktop's gridSelect: add, remove, or toggle the one clicked cell.
                if let Some((cells, op)) = self.viewport.take_grid_select() {
                    for c in cells {
                        let has = self.grid.selected.contains(&c);
                        if op == "add" || (op == "toggle" && !has) {
                            if !has {
                                self.grid.selected.push(c);
                            }
                        } else {
                            self.grid.selected.retain(|x| *x != c);
                        }
                    }
                }
                // The desktop's `adjustBrush`: the radius in whole metres (≥ 1), the strength to
                // 0.1 m.
                if let Some((radius, factor)) = self.viewport.take_adjust() {
                    let f = f64::from(factor);
                    if radius {
                        self.brush.radius_m = (self.brush.radius_m * f).round().max(1.0);
                    } else {
                        self.brush.strength_m = (self.brush.strength_m * f * 10.0).round() / 10.0;
                    }
                }
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
                    // The desktop's `sculpting` is on for any ground tool (road, entrances too).
                    sculpting: self.drawing.is_some(),
                    road: self.drawing == Some(Drawing::Road),
                    grid: self.grid.on,
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

/// The desktop's `adaptLodBias` (lib/previewLod.ts): over the target → one LOD coarser, under a
/// quarter of it → one finer, never below the default nor above 4; a target of 0 = off.
fn adapt_lod_bias(bias: u32, ms: f64, target: f64) -> u32 {
    if target.is_nan() || target <= 0.0 {
        0
    } else if ms > target {
        (bias + 1).min(4)
    } else if ms * 4.0 < target {
        bias.saturating_sub(1)
    } else {
        bias
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_live_target_moves_the_preview_lod_like_the_desktop() {
        assert_eq!(adapt_lod_bias(0, 300.0, 200.0), 1);
        assert_eq!(adapt_lod_bias(4, 300.0, 200.0), 4);
        assert_eq!(adapt_lod_bias(2, 40.0, 200.0), 1);
        assert_eq!(adapt_lod_bias(0, 40.0, 200.0), 0);
        assert_eq!(adapt_lod_bias(2, 100.0, 200.0), 2);
        assert_eq!(adapt_lod_bias(3, 900.0, 0.0), 0);
    }

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
