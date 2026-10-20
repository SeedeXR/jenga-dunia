//! Hydraulic erosion (virtual pipes) on the device — `hydraulic.wgsl` is the pass-for-pass port
//! of `jd_terrain::sim::hydraulic_erode_with`; the host mirrors its validation and `soft` /
//! rain-mask preparation so both engines reject and scale inputs the same way.

use jd_terrain::{Heightfield, HydraulicParams, HydraulicResult};
use jd_types::{ErrorCode, JdError, JdResult};

use crate::{Gpu, binding};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
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
}

impl Gpu {
    /// [`jd_terrain::hydraulic_erode_with`] on the GPU: same model, same validation, same
    /// optional `hardness` (0..1) and `rain` (0..1) fields.
    pub fn hydraulic_erode(
        &self,
        h: &Heightfield,
        spacing_m: [f32; 2],
        p: &HydraulicParams,
        hardness: Option<&Heightfield>,
        rain: Option<&Heightfield>,
    ) -> JdResult<HydraulicResult> {
        for (name, f) in [("hardness", hardness), ("rain", rain)] {
            if let Some(k) = f
                && (k.width(), k.height()) != (h.width(), h.height())
            {
                return Err(JdError::invalid(name, "field size differs from `in`"));
            }
        }
        if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
            return Err(JdError::invalid(
                "spacing_m",
                "sample spacing must be positive",
            ));
        }
        if !(p.dt > 0.0 && p.dt <= 1.0) {
            return Err(JdError::invalid("dt", "must be within (0, 1]"));
        }
        if !(p.max_concentration > 0.0 && p.max_concentration.is_finite()) {
            return Err(JdError::invalid("max_concentration", "must be positive"));
        }
        for (name, v) in [
            ("rain_rate", p.rain_rate),
            ("evaporation", p.evaporation),
            ("capacity_k", p.capacity_k),
            ("dissolve_k", p.dissolve_k),
            ("deposit_k", p.deposit_k),
            ("min_tilt", p.min_tilt),
        ] {
            if !(v >= 0.0 && v.is_finite()) {
                return Err(JdError::invalid(name, "must be finite and ≥ 0"));
            }
        }
        if p.evaporation * p.dt >= 1.0 || p.dissolve_k * p.dt > 1.0 || p.deposit_k * p.dt > 1.0 {
            return Err(JdError::invalid(
                "dt",
                "evaporation·dt must be < 1 and dissolve_k·dt, deposit_k·dt ≤ 1",
            ));
        }
        h.validate_finite()?;
        let (w, hh) = (h.width(), h.height());
        let n = (w * hh) as usize;
        let params = Params {
            w,
            h: hh,
            has_soft: hardness.is_some() as u32,
            has_rain: rain.is_some() as u32,
            dt: p.dt,
            rain_rate: p.rain_rate,
            evaporation: p.evaporation,
            capacity_k: p.capacity_k,
            dissolve_k: p.dissolve_k,
            deposit_k: p.deposit_k,
            min_tilt: p.min_tilt,
            max_conc: p.max_concentration,
            lx: spacing_m[0],
            ly: spacing_m[1],
            area: spacing_m[0] * spacing_m[1],
            g: 9.81,
        };
        let dev = &self.device;
        let shader = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("jd hydraulic"),
            source: wgpu::ShaderSource::Wgsl(include_str!("hydraulic.wgsl").into()),
        });
        let storage = |label: &str, size: u64| {
            dev.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let vec4_bytes = (n * 16) as u64;
        let f32_bytes = (n * 4) as u64;
        let state = [
            storage("state a", vec4_bytes),
            storage("state b", vec4_bytes),
        ];
        let flux = [storage("flux a", vec4_bytes), storage("flux b", vec4_bytes)];
        let water_pre = storage("water_pre", f32_bytes);
        let soft = storage("soft", if hardness.is_some() { f32_bytes } else { 4 });
        let rain_buf = storage("rain", if rain.is_some() { f32_bytes } else { 4 });
        let uniform = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let readback = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: vec4_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue
            .write_buffer(&uniform, 0, bytemuck::bytes_of(&params));
        let init: Vec<[f32; 4]> = h.to_vec().iter().map(|t| [*t, 0.0, 0.0, 0.0]).collect();
        self.queue
            .write_buffer(&state[0], 0, bytemuck::cast_slice(&init));
        self.queue.write_buffer(&flux[0], 0, &vec![0u8; n * 16]);
        if let Some(k) = hardness {
            let soft_v: Vec<f32> = k
                .to_vec()
                .iter()
                .map(|v| 1.0 - 0.9 * v.clamp(0.0, 1.0))
                .collect();
            self.queue
                .write_buffer(&soft, 0, bytemuck::cast_slice(&soft_v));
        }
        if let Some(r) = rain {
            let rain_v: Vec<f32> = r.to_vec().iter().map(|v| v.clamp(0.0, 1.0)).collect();
            self.queue
                .write_buffer(&rain_buf, 0, bytemuck::cast_slice(&rain_v));
        }
        let ro = wgpu::BufferBindingType::Storage { read_only: true };
        let rw = wgpu::BufferBindingType::Storage { read_only: false };
        let layout = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("jd hydraulic"),
            entries: &[
                binding(0, wgpu::BufferBindingType::Uniform),
                binding(1, ro),
                binding(2, rw),
                binding(3, ro),
                binding(4, rw),
                binding(5, rw),
                binding(6, ro),
                binding(7, ro),
            ],
        });
        let pipeline_layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str| {
            dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let passes = [
            pipeline("rain_pass"),
            pipeline("flux_pass"),
            pipeline("update_pass"),
        ];
        let groups: Vec<wgpu::BindGroup> = [(0usize, 1usize), (1, 0)]
            .map(|(src, dst)| {
                dev.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniform.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: state[src].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: state[dst].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: flux[src].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: flux[dst].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: water_pre.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 6,
                            resource: soft.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 7,
                            resource: rain_buf.as_entire_binding(),
                        },
                    ],
                })
            })
            .into();
        let (gx, gy) = (w.div_ceil(16), hh.div_ceil(16));
        let mut encoder = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("jd hydraulic"),
        });
        for it in 0..p.iterations as usize {
            let group = &groups[it % 2];
            for pipe in &passes {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
                pass.set_pipeline(pipe);
                pass.set_bind_group(0, group, &[]);
                pass.dispatch_workgroups(gx, gy, 1);
            }
        }
        encoder.copy_buffer_to_buffer(
            &state[(p.iterations as usize) % 2],
            0,
            &readback,
            0,
            vec4_bytes,
        );
        self.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        dev.poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| JdError::new(ErrorCode::E_INTERNAL, format!("gpu poll: {e:?}")))?;
        rx.recv()
            .map_err(|_| JdError::new(ErrorCode::E_INTERNAL, "gpu readback dropped"))?
            .map_err(|e| JdError::new(ErrorCode::E_INTERNAL, format!("gpu map: {e:?}")))?;
        let view = slice
            .get_mapped_range()
            .map_err(|e| JdError::new(ErrorCode::E_INTERNAL, format!("gpu map range: {e:?}")))?;
        let data: Vec<[f32; 4]> = bytemuck::cast_slice(&view).to_vec();
        drop(view);
        readback.unmap();
        let build = |c: usize| -> JdResult<Heightfield> {
            let mut f = Heightfield::new(w, hh, h.tile_size(), h.halo())?;
            f.fill_with(|x, y| data[(y * w + x) as usize][c]);
            Ok(f)
        };
        Ok(HydraulicResult {
            height: build(0)?,
            water: build(1)?,
            sediment: build(2)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hydraulic_matches_the_cpu_kernel_with_hardness_and_a_rain_mask() {
        let Some(gpu) = Gpu::shared() else {
            eprintln!("no GPU adapter: skipping");
            return;
        };
        // The CPU tests' valley: a tilted trough, 10 m cells; hard rock in the east, rain on the west.
        let mut valley = Heightfield::new(48, 32, 64, 0).expect("v");
        valley.fill_with(|x, y| 200.0 + y as f32 * 2.0 + (x as f32 - 24.0).abs() * 1.5);
        let mut hard = Heightfield::new(48, 32, 64, 0).expect("k");
        hard.fill_with(|x, _| if x >= 30 { 0.9 } else { 0.0 });
        let mut rain = Heightfield::new(48, 32, 64, 0).expect("r");
        rain.fill_with(|x, _| if x < 24 { 1.0 } else { 0.3 });
        let p = HydraulicParams {
            iterations: 80,
            rain_rate: 0.2,
            ..Default::default()
        };
        // Divergence grows with the steps (float order through a nonlinear feedback): print the curve.
        for steps in [1u32, 10, 40] {
            let q = HydraulicParams {
                iterations: steps,
                ..p
            };
            let c = jd_terrain::hydraulic_erode_with(
                &valley,
                [10.0, 10.0],
                &q,
                Some(&hard),
                Some(&rain),
                None,
            )
            .expect("cpu");
            let g = gpu
                .hydraulic_erode(&valley, [10.0, 10.0], &q, Some(&hard), Some(&rain))
                .expect("gpu");
            let d = c
                .height
                .to_vec()
                .iter()
                .zip(&g.height.to_vec())
                .map(|(x, y)| (x - y).abs())
                .fold(0.0f32, f32::max);
            eprintln!("  {steps} steps: height diff {d:e} m");
        }
        let cpu = jd_terrain::hydraulic_erode_with(
            &valley,
            [10.0, 10.0],
            &p,
            Some(&hard),
            Some(&rain),
            None,
        )
        .expect("cpu");
        let g = gpu
            .hydraulic_erode(&valley, [10.0, 10.0], &p, Some(&hard), Some(&rain))
            .expect("gpu");
        let worst = |a: &Heightfield, b: &Heightfield| {
            let (a, b) = (a.to_vec(), b.to_vec());
            a.iter()
                .zip(&b)
                .map(|(x, y)| (x - y).abs())
                .fold(0.0f32, f32::max)
        };
        let (dh, dw, ds) = (
            worst(&cpu.height, &g.height),
            worst(&cpu.water, &g.water),
            worst(&cpu.sediment, &g.sediment),
        );
        eprintln!(
            "parity: height {dh:e} m, water {dw:e} m, sediment {ds:e} m ({})",
            gpu.adapter
        );
        // docs/spec/performance-targets.md: erosion GPU vs CPU ≤ 1e-3 relative; held 100× tighter here.
        let scale = valley.to_vec().iter().cloned().fold(0.0f32, f32::max);
        assert!(
            dh <= 1e-5 * scale && dw <= 1e-3 && ds <= 1e-3,
            "GPU/CPU hydraulic parity: {dh} / {dw} / {ds} (scale {scale})"
        );
        let moved = valley
            .to_vec()
            .iter()
            .zip(&g.height.to_vec())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(moved > 0.01, "the GPU run eroded something: {moved} m");
        assert!(
            gpu.hydraulic_erode(
                &valley,
                [10.0, 10.0],
                &HydraulicParams { dt: 0.0, ..p },
                None,
                None
            )
            .is_err()
        );
    }
}
