//! Distance through a mask: the one 8-neighbour Dijkstra the sediment supply and the surface
//! current share (the wave refraction march solves a different equation and keeps its own).

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::atomic::AtomicBool;

use jd_types::JdResult;

use crate::sim::check_cancel;

/// Metres from the nearest seed to every cell, travelling only through `passable` cells
/// (seeds themselves need not be passable: a shore seed relaxes into the water beside it),
/// stopped beyond `stop_m` (those cells stay `INFINITY`). `cancel` is polled every 65 536 pops.
pub(crate) fn water_distance(
    passable: &[bool],
    seeds: impl Iterator<Item = usize>,
    w: usize,
    h: usize,
    spacing_m: [f32; 2],
    stop_m: f32,
    cancel: Option<&AtomicBool>,
) -> JdResult<Vec<f32>> {
    let mut dist = vec![f32::INFINITY; w * h];
    let mut heap: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new(); // (metres as bits, cell)
    for i in seeds {
        dist[i] = 0.0;
        heap.push(Reverse((0, i as u32)));
    }
    let mut pops = 0usize;
    while let Some(Reverse((bits, i))) = heap.pop() {
        let (i, d) = (i as usize, f32::from_bits(bits));
        if d > dist[i] || d > stop_m {
            continue;
        }
        pops += 1;
        if pops.is_multiple_of(65_536) {
            check_cancel(cancel)?;
        }
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        for oy in -1i64..=1 {
            for ox in -1i64..=1 {
                let (nx, ny) = (x + ox, y + oy);
                if (ox, oy) == (0, 0) || nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                if !passable[j] {
                    continue;
                }
                let nd = d
                    + ((ox as f32 * spacing_m[0]).powi(2) + (oy as f32 * spacing_m[1]).powi(2))
                        .sqrt();
                if nd < dist[j] {
                    dist[j] = nd;
                    heap.push(Reverse((nd.to_bits(), j as u32)));
                }
            }
        }
    }
    Ok(dist)
}
