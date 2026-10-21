// Hydraulic erosion, the virtual-pipe shallow-water model of jd_terrain::sim::hydraulic_erode_with
// (Mei–Decaudin–Hu 2007), pass for pass: rain → outflow flux per pipe (scaled so a cell never sends
// more than it holds) → water/velocity/erosion/deposition/transport/evaporation. Closed boundary.
// State per cell: (terrain, water, sediment, 0); flux per cell: (left, right, top, bottom).

struct Params {
    w: u32,
    h: u32,
    has_soft: u32,
    has_rain: u32,
    dt: f32,
    rain_rate: f32,
    evaporation: f32,
    capacity_k: f32,
    dissolve_k: f32,
    deposit_k: f32,
    min_tilt: f32,
    max_conc: f32,
    lx: f32,
    ly: f32,
    area: f32,
    g: f32,
    has_drains: u32,
    has_soil: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> state_in: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> state_out: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> flux_in: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> flux_out: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read_write> water_pre: array<f32>;
@group(0) @binding(6) var<storage, read> soft: array<f32>;
@group(0) @binding(7) var<storage, read> rain: array<f32>;
@group(0) @binding(8) var<storage, read> drains: array<f32>;

const PIPES: array<vec2<i32>, 4> = array<vec2<i32>, 4>(vec2(-1, 0), vec2(1, 0), vec2(0, -1), vec2(0, 1));

fn inside(x: i32, y: i32) -> bool {
    return x >= 0 && y >= 0 && x < i32(p.w) && y < i32(p.h);
}

fn pipe_len(k: u32) -> f32 {
    return select(p.ly, p.lx, k < 2u);
}

@compute @workgroup_size(16, 16)
fn rain_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.w || id.y >= p.h { return; }
    let i = id.y * p.w + id.x;
    var scale = 1.0;
    if p.has_rain != 0u { scale = rain[i]; }
    water_pre[i] = state_in[i].y + p.rain_rate * p.dt * scale;
}

@compute @workgroup_size(16, 16)
fn flux_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.w || id.y >= p.h { return; }
    let i = id.y * p.w + id.x;
    let surface = state_in[i].x + water_pre[i];
    var f = vec4<f32>(0.0);
    var total = 0.0;
    for (var k = 0u; k < 4u; k++) {
        let n = vec2<i32>(id.xy) + PIPES[k];
        if !inside(n.x, n.y) { continue; }
        let j = u32(n.y) * p.w + u32(n.x);
        let dh = surface - (state_in[j].x + water_pre[j]);
        let v = max(flux_in[i][k] + p.dt * p.area * p.g * dh / pipe_len(k), 0.0);
        f[k] = v;
        total += v;
    }
    let volume = water_pre[i] * p.area;
    if total * p.dt > volume && total > 0.0 {
        f *= volume / (total * p.dt);
    }
    flux_out[i] = f;
}

@compute @workgroup_size(16, 16)
fn update_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.w || id.y >= p.h { return; }
    let xi = i32(id.x);
    let yi = i32(id.y);
    let i = id.y * p.w + id.x;
    let fo = flux_out[i];
    let out_total = fo.x + fo.y + fo.z + fo.w;
    var in_total = 0.0;
    var sed_in = 0.0;
    var in_lr = 0.0; // inflow from left minus inflow from right (x momentum)
    var in_tb = 0.0;
    for (var k = 0u; k < 4u; k++) {
        let n = vec2<i32>(xi, yi) + PIPES[k];
        if !inside(n.x, n.y) { continue; }
        let j = u32(n.y) * p.w + u32(n.x);
        let f_in = flux_out[j][k ^ 1u]; // the neighbour's pipe pointing back at us
        in_total += f_in;
        // Donor-cell transport: the neighbour's sediment travels at its concentration.
        if water_pre[j] > 0.0 {
            sed_in += state_in[j].z / water_pre[j] * f_in * p.dt / p.area;
        }
        if k == 0u { in_lr += f_in; } else if k == 1u { in_lr -= f_in; } else if k == 2u { in_tb += f_in; } else { in_tb -= f_in; }
    }
    let wp = water_pre[i];
    let d_new = max(wp + (in_total - out_total) * p.dt / p.area, 0.0);
    // Velocity from net horizontal transport through the cell (Mei eq. 8–9).
    let d_bar = 0.5 * (wp + d_new);
    var vel = 0.0;
    if d_bar > 1e-6 {
        let vx = 0.5 * (in_lr + fo.y - fo.x) / (p.ly * d_bar);
        let vy = 0.5 * (in_tb + fo.w - fo.z) / (p.lx * d_bar);
        vel = sqrt(vx * vx + vy * vy);
    }
    // Local tilt from the terrain gradient.
    let x0 = max(xi - 1, 0);
    let x1 = min(xi + 1, i32(p.w) - 1);
    let y0 = max(yi - 1, 0);
    let y1 = min(yi + 1, i32(p.h) - 1);
    let gx = (state_in[u32(yi) * p.w + u32(x1)].x - state_in[u32(yi) * p.w + u32(x0)].x) / (f32(max(x1 - x0, 1)) * p.lx);
    let gy = (state_in[u32(y1) * p.w + u32(xi)].x - state_in[u32(y0) * p.w + u32(xi)].x) / (f32(max(y1 - y0, 1)) * p.ly);
    let grad = sqrt(gx * gx + gy * gy);
    let sin_tilt = max(grad / sqrt(1.0 + grad * grad), p.min_tilt);
    let capacity = p.capacity_k * sin_tilt * vel * d_bar;
    let st = state_in[i];
    var sed_out = 0.0;
    if wp > 0.0 { sed_out = st.z / wp * out_total * p.dt / p.area; }
    var s = max(st.z - sed_out, 0.0) + sed_in;
    var t = st.x;
    var l = st.w; // loose material on the rock
    if capacity > s {
        // Loose material first, at the full rate; the rock underneath at its own.
        var k_soft = 1.0;
        if p.has_soft != 0u { k_soft = soft[i]; }
        let demand = p.dissolve_k * (capacity - s) * p.dt;
        var from_soil = 0.0;
        if p.has_soil != 0u { from_soil = min(demand, l); }
        let delta = from_soil + (demand - from_soil) * k_soft;
        l -= from_soil;
        t -= delta;
        s += delta;
    } else {
        let delta = min(p.deposit_k * (s - capacity) * p.dt, s);
        t += delta;
        l += delta;
        s -= delta;
    }
    let d_final = d_new * (1.0 - p.evaporation * p.dt);
    // Concentration cap against the water that remains: the excess settles now.
    let cap = p.max_conc * d_final;
    if s > cap {
        t += s - cap;
        l += s - cap;
        s = cap;
    }
    // Drains: the water and what it carries leave the field here.
    if p.has_drains != 0u && drains[i] > 0.5 {
        state_out[i] = vec4(t, 0.0, 0.0, l);
        return;
    }
    state_out[i] = vec4(t, d_final, s, l);
}
