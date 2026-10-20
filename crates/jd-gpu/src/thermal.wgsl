// Thermal erosion (Olsen 2004 talus rule), the two passes of jd_terrain::sim::thermal_erode:
// `shed` per cell (amount it sheds, amount / total excess), then `gather` (lose what you shed,
// gain each neighbour's share directed at you). Same neighbour order and distances as the CPU.

struct Params {
    w: u32,
    h: u32,
    tan_t: f32,
    rate: f32,
    dist: array<vec4<f32>, 2>, // 8 neighbour distances (vec4 pairs for 16-byte alignment)
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> height: array<f32>;
@group(0) @binding(2) var<storage, read_write> shed: array<vec2<f32>>;
@group(0) @binding(3) var<storage, read_write> out: array<f32>;

const OFF: array<vec2<i32>, 8> = array<vec2<i32>, 8>(
    vec2(-1, -1), vec2(0, -1), vec2(1, -1), vec2(-1, 0), vec2(1, 0), vec2(-1, 1), vec2(0, 1), vec2(1, 1)
);

fn dist(k: u32) -> f32 {
    return p.dist[k / 4u][k % 4u];
}

fn inside(x: i32, y: i32) -> bool {
    return x >= 0 && y >= 0 && x < i32(p.w) && y < i32(p.h);
}

@compute @workgroup_size(16, 16)
fn shed_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.w || id.y >= p.h { return; }
    let i = id.y * p.w + id.x;
    let hc = height[i];
    var total = 0.0;
    var dmax = 0.0;
    for (var k = 0u; k < 8u; k++) {
        let n = vec2<i32>(id.xy) + OFF[k];
        if !inside(n.x, n.y) { continue; }
        let d = hc - height[u32(n.y) * p.w + u32(n.x)] - p.tan_t * dist(k);
        if d > 0.0 { total += d; dmax = max(dmax, d); }
    }
    if total > 0.0 {
        let amount = p.rate * dmax * 0.5;
        shed[i] = vec2(amount, amount / total);
    } else {
        shed[i] = vec2(0.0, 0.0);
    }
}

@compute @workgroup_size(16, 16)
fn gather_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.w || id.y >= p.h { return; }
    let i = id.y * p.w + id.x;
    let hc = height[i];
    var gained = 0.0;
    for (var k = 0u; k < 8u; k++) {
        let n = vec2<i32>(id.xy) + OFF[k];
        if !inside(n.x, n.y) { continue; }
        let j = u32(n.y) * p.w + u32(n.x);
        let factor = shed[j].y;
        if factor > 0.0 {
            let excess = height[j] - hc - p.tan_t * dist(k);
            if excess > 0.0 { gained += factor * excess; }
        }
    }
    out[i] = hc - shed[i].x + gained;
}
