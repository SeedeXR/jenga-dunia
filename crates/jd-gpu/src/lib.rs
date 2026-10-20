//! `jd-gpu`: headless wgpu compute for the terrain kernels (ADR-0003: all computation stays in
//! Rust on a headless device; the viewport only receives tiles). Kernels: thermal erosion and
//! hydraulic (virtual pipes) erosion, line-for-line ports of `jd_terrain::sim` to WGSL,
//! parity-tested against the CPU.
//!
//! ponytail: one process-wide device (`Gpu::shared`), buffers re-created per call; the upgrade
//! path is a per-kernel buffer pool and cached pipelines.

mod hydraulic;

use jd_terrain::Heightfield;
use jd_types::{ErrorCode, JdError, JdResult};

/// A headless compute device. `None` when no adapter is available (CI runners without a GPU).
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Adapter name and backend, e.g. `Apple M4 (Metal)`.
    pub adapter: String,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    w: u32,
    h: u32,
    tan_t: f32,
    rate: f32,
    has_hardness: u32,
    _pad: [u32; 3],
    dist: [f32; 8],
}

impl Gpu {
    /// The process-wide device, opened on first use; `None` when the machine has no adapter.
    pub fn shared() -> Option<&'static Gpu> {
        static GPU: std::sync::OnceLock<Option<Gpu>> = std::sync::OnceLock::new();
        GPU.get_or_init(Gpu::new).as_ref()
    }

    /// Open the first available adapter headlessly.
    pub fn new() -> Option<Gpu> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok()?;
        let info = adapter.get_info();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        Some(Gpu {
            device,
            queue,
            adapter: format!("{} ({:?})", info.name, info.backend),
        })
    }

    /// Thermal erosion on the GPU: the same talus rule, neighbour order, distances and hardness
    /// rule as [`jd_terrain::thermal_erode_with`], `iterations` ping-pong steps on the device.
    pub fn thermal_erode(
        &self,
        h: &Heightfield,
        spacing_m: [f32; 2],
        talus_deg: f32,
        iterations: u32,
        rate: f32,
        hardness: Option<&Heightfield>,
    ) -> JdResult<Heightfield> {
        if let Some(k) = hardness
            && (k.width(), k.height()) != (h.width(), h.height())
        {
            return Err(JdError::invalid("hardness", "field size differs from `in`"));
        }
        if !(0.0..90.0).contains(&talus_deg) {
            return Err(JdError::invalid("talus_deg", "must be within 0..90"));
        }
        if !(0.0 < rate && rate <= jd_terrain::THERMAL_MAX_RATE) {
            return Err(JdError::invalid("rate", "must be within (0, 0.25]"));
        }
        if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
            return Err(JdError::invalid(
                "spacing_m",
                "sample spacing must be positive",
            ));
        }
        h.validate_finite()?;
        let (w, hh) = (h.width(), h.height());
        let n = (w * hh) as usize;
        let offsets: [(f32, f32); 8] = [
            (-1.0, -1.0),
            (0.0, -1.0),
            (1.0, -1.0),
            (-1.0, 0.0),
            (1.0, 0.0),
            (-1.0, 1.0),
            (0.0, 1.0),
            (1.0, 1.0),
        ];
        let params = Params {
            w,
            h: hh,
            tan_t: talus_deg.to_radians().tan(),
            rate,
            has_hardness: hardness.is_some() as u32,
            _pad: [0; 3],
            dist: offsets.map(|(dx, dy)| (dx * spacing_m[0]).hypot(dy * spacing_m[1])),
        };
        let dev = &self.device;
        let shader = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("jd thermal"),
            source: wgpu::ShaderSource::Wgsl(include_str!("thermal.wgsl").into()),
        });
        let f32_bytes = (n * 4) as u64;
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
        let bufs = [
            storage("height a", f32_bytes),
            storage("height b", f32_bytes),
        ];
        let shed = storage("shed", f32_bytes * 2);
        let hard = storage("hardness", if hardness.is_some() { f32_bytes } else { 4 });
        if let Some(k) = hardness {
            self.queue
                .write_buffer(&hard, 0, bytemuck::cast_slice(&k.to_vec()));
        }
        let uniform = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let readback = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: f32_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue
            .write_buffer(&uniform, 0, bytemuck::bytes_of(&params));
        self.queue
            .write_buffer(&bufs[0], 0, bytemuck::cast_slice(&h.to_vec()));
        let layout = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("jd thermal"),
            entries: &[
                binding(0, wgpu::BufferBindingType::Uniform),
                binding(1, wgpu::BufferBindingType::Storage { read_only: true }),
                binding(2, wgpu::BufferBindingType::Storage { read_only: false }),
                binding(3, wgpu::BufferBindingType::Storage { read_only: false }),
                binding(4, wgpu::BufferBindingType::Storage { read_only: true }),
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
        let (shed_pipe, gather_pipe) = (pipeline("shed_pass"), pipeline("gather_pass"));
        // Two bind groups: read a → write b, and read b → write a.
        let groups: Vec<wgpu::BindGroup> = [(0, 1), (1, 0)]
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
                            resource: bufs[src].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: shed.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: bufs[dst].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,

                            resource: hard.as_entire_binding(),
                        },
                    ],
                })
            })
            .into();
        let (gx, gy) = (w.div_ceil(16), hh.div_ceil(16));
        let mut encoder = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("jd thermal"),
        });
        for it in 0..iterations as usize {
            let group = &groups[it % 2];
            for pipe in [&shed_pipe, &gather_pipe] {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
                pass.set_pipeline(pipe);
                pass.set_bind_group(0, group, &[]);
                pass.dispatch_workgroups(gx, gy, 1);
            }
        }
        let result = &bufs[(iterations as usize) % 2];
        encoder.copy_buffer_to_buffer(result, 0, &readback, 0, f32_bytes);
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
        let data: Vec<f32> = bytemuck::cast_slice(&view).to_vec();
        drop(view);
        readback.unmap();
        let mut out = Heightfield::new(w, hh, h.tile_size(), h.halo())?;
        out.fill_with(|x, y| data[(y * w + x) as usize]);
        Ok(out)
    }
}

pub(crate) fn binding(index: u32, ty: wgpu::BufferBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: index,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thermal_matches_the_cpu_kernel_and_conserves_mass() {
        let Some(gpu) = Gpu::new() else {
            eprintln!("no GPU adapter: skipping");
            return;
        };
        eprintln!("adapter: {}", gpu.adapter);
        // A rough 96×80 field: a spike, a ridge and noise-like bumps, 10 m cells.
        let mut h = Heightfield::new(96, 80, 32, 4).expect("hf");
        h.fill_with(|x, y| {
            let (fx, fy) = (x as f32, y as f32);
            let spike = if (x, y) == (40, 30) { 400.0 } else { 0.0 };
            let ridge = (200.0 - (fx - 60.0).abs() * 30.0).max(0.0);
            spike
                + ridge
                + 15.0 * ((fx * 0.7).sin() + (fy * 0.5).cos())
                + (fx * 1.3 + fy * 2.1).sin() * 8.0
        });
        let cpu = jd_terrain::thermal_erode(&h, [10.0, 10.0], 35.0, 40, 0.2, None).expect("cpu");
        let gpu_out = gpu
            .thermal_erode(&h, [10.0, 10.0], 35.0, 40, 0.2, None)
            .expect("gpu");
        let (a, b) = (cpu.to_vec(), gpu_out.to_vec());
        let scale = a.iter().cloned().fold(0.0f32, f32::max).max(1.0);
        let worst = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max)
            / scale;
        eprintln!(
            "parity: max relative difference {worst:e} over {} cells",
            a.len()
        );
        assert!(
            worst <= 1e-4,
            "GPU/CPU parity: max relative difference {worst}"
        );
        let mass = |v: &[f32]| v.iter().map(|x| *x as f64).sum::<f64>();
        let before = mass(&h.to_vec());
        assert!(
            (mass(&b) - before).abs() <= 1e-3 * before.abs().max(1.0),
            "mass conserved: {before} → {}",
            mass(&b)
        );
        assert!(
            gpu_out.get(40, 30) < 300.0,
            "the spike slumped: {}",
            gpu_out.get(40, 30)
        );
        assert!(
            gpu.thermal_erode(&h, [10.0, 10.0], 95.0, 1, 0.2, None)
                .is_err()
        );
        // Hardness: the east half is hard rock; the CPU and GPU agree there too.
        let mut k = Heightfield::new(96, 80, 32, 4).expect("k");
        k.fill_with(|x, _| if x >= 48 { 0.8 } else { 0.0 });
        let cpu_k = jd_terrain::thermal_erode_with(&h, [10.0, 10.0], 35.0, 40, 0.2, Some(&k), None)
            .expect("cpu k");
        let gpu_k = gpu
            .thermal_erode(&h, [10.0, 10.0], 35.0, 40, 0.2, Some(&k))
            .expect("gpu k");
        let (a, b) = (cpu_k.to_vec(), gpu_k.to_vec());
        let worst_k = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max)
            / scale;
        eprintln!("parity with hardness: {worst_k:e}");
        assert!(worst_k <= 1e-4, "GPU/CPU parity with hardness: {worst_k}");
        assert!(
            gpu_k.get(70, 10) != gpu_out.get(70, 10) || a == b,
            "hardness changes the hard half"
        );
        assert!(
            gpu.thermal_erode(
                &h,
                [10.0, 10.0],
                35.0,
                1,
                0.2,
                Some(&Heightfield::new(8, 8, 8, 0).expect("s"))
            )
            .is_err()
        );
    }

    /// `cargo test -p jd-gpu --release -- --ignored --nocapture bench`: GPU vs CPU wall time,
    /// 2048² × 50 thermal steps — the number in docs/spec/performance-targets.md.
    #[test]
    #[ignore]
    fn bench_2048_thermal_gpu_vs_cpu() {
        let Some(gpu) = Gpu::new() else {
            return;
        };
        let mut h = Heightfield::new(2048, 2048, 512, 4).expect("hf");
        h.fill_with(|x, y| {
            let (fx, fy) = (x as f32 * 0.01, y as f32 * 0.01);
            400.0 * (fx.sin() * fy.cos()) + 60.0 * ((fx * 7.3).sin() + (fy * 5.1).cos())
        });
        let t = std::time::Instant::now();
        let cpu = jd_terrain::thermal_erode(&h, [5.0, 5.0], 35.0, 50, 0.2, None).expect("cpu");
        let cpu_ms = t.elapsed().as_millis();
        let t = std::time::Instant::now();
        let g = gpu
            .thermal_erode(&h, [5.0, 5.0], 35.0, 50, 0.2, None)
            .expect("gpu");
        let gpu_ms = t.elapsed().as_millis();
        let (a, b) = (cpu.to_vec(), g.to_vec());
        let worst = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max);
        eprintln!(
            "bench 2048² × 50 thermal: cpu {cpu_ms} ms, gpu {gpu_ms} ms ({}), max abs diff {worst:e} m",
            gpu.adapter
        );
    }
}
