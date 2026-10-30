//! Renders one small scene through the style-shader samples in four looks and writes PNGs.
//!
//! Usage: `render_looks <out dir>`. Looks: `pbr` (GGX + ACES), `toon` (bands + rim + post outlines +
//! PBR Neutral), `flat` (faceted low-poly + outlines + PBR Neutral), `pixel` (toon at 320x180, 1 px
//! outlines, clamp curve, 16-colour palette with 4x4 Bayer dither, sharp upscale to 1280x720).
//! There are no shadows: the samples have no shadow map. Research proof, not engine code.

use glam::{Mat4, Quat, Vec3};
use std::borrow::Cow;
use std::process::ExitCode;

const W: u32 = 1280;
const H: u32 = 720;
const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const NORMALS: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const LDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const OUT_SRGB: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const NEAR: f32 = 0.1;
const FAR: f32 = 200.0;

fn src(name: &str) -> String {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
    std::fs::read_to_string(format!("{dir}/{name}.wgsl")).unwrap_or_default()
}

fn bytes_f32(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn bytes_u32(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Gpu {
    fn tex(&self, w: u32, h: u32, format: wgpu::TextureFormat, extra: wgpu::TextureUsages) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | extra,
            view_formats: &[],
        })
    }

    fn buffer(&self, data: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (data.len() as u64).next_multiple_of(16),
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buf, 0, data);
        buf
    }

    fn module(&self, name: &str) -> wgpu::ShaderModule {
        self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(name),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(src(name))),
        })
    }

    fn fullscreen(&self, module: &wgpu::ShaderModule, fs: &str, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(fs),
            layout: None,
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("vs_fullscreen"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some(fs),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }

    fn bind(&self, pipeline: &wgpu::RenderPipeline, entries: &[wgpu::BindingResource]) -> wgpu::BindGroup {
        let pairs: Vec<(u32, wgpu::BindingResource)> =
            entries.iter().enumerate().map(|(i, r)| (i as u32, r.clone())).collect();
        self.bind_at(pipeline, &pairs)
    }

    /// Automatic layouts only contain the bindings an entry point uses, so some groups skip numbers.
    fn bind_at(&self, pipeline: &wgpu::RenderPipeline, entries: &[(u32, wgpu::BindingResource)]) -> wgpu::BindGroup {
        let entries: Vec<wgpu::BindGroupEntry> = entries
            .iter()
            .map(|(i, r)| wgpu::BindGroupEntry { binding: *i, resource: r.clone() })
            .collect();
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        })
    }

    fn pass(&self, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, pipeline: &wgpu::RenderPipeline, group: &wgpu::BindGroup) {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
}

// ---- meshes: position + normal, u16 indices ----------------------------------------------------

fn cube() -> (Vec<f32>, Vec<u16>) {
    let mut v = Vec::new();
    let mut i = Vec::new();
    let faces = [
        (Vec3::X, Vec3::Y, Vec3::Z),
        (Vec3::NEG_X, Vec3::Y, Vec3::NEG_Z),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (Vec3::NEG_Y, Vec3::NEG_Z, Vec3::X),
        (Vec3::Z, Vec3::Y, Vec3::NEG_X),
        (Vec3::NEG_Z, Vec3::Y, Vec3::X),
    ];
    for (n, up, _) in faces {
        let right = up.cross(n);
        let base = (v.len() / 6) as u16;
        for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p = (n + right * a + up * b) * 0.5;
            v.extend_from_slice(&[p.x, p.y, p.z, n.x, n.y, n.z]);
        }
        i.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (v, i)
}

fn sphere(rings: u16, segs: u16) -> (Vec<f32>, Vec<u16>) {
    let mut v = Vec::new();
    let mut i = Vec::new();
    for r in 0..=rings {
        let th = std::f32::consts::PI * r as f32 / rings as f32;
        for s in 0..=segs {
            let ph = std::f32::consts::TAU * s as f32 / segs as f32;
            let n = Vec3::new(th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin());
            let p = n * 0.5;
            v.extend_from_slice(&[p.x, p.y, p.z, n.x, n.y, n.z]);
        }
    }
    let row = segs + 1;
    for r in 0..rings {
        for s in 0..segs {
            let a = r * row + s;
            let b = a + row;
            i.extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    (v, i)
}

struct Mesh {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    count: u32,
    inst: wgpu::Buffer,
    inst_count: u32,
}

fn instance(m: Mat4, color: [f32; 3], rough: f32, metal: f32, model: f32) -> Vec<f32> {
    let mut out = m.to_cols_array().to_vec();
    out.extend_from_slice(&[color[0], color[1], color[2], 1.0, rough, metal, 0.0, model]);
    out
}

fn make_mesh(gpu: &Gpu, data: (Vec<f32>, Vec<u16>), instances: &[Vec<f32>]) -> Mesh {
    let (v, mut i) = data;
    if i.len() % 2 == 1 {
        i.push(0);
    }
    let flat: Vec<f32> = instances.iter().flatten().copied().collect();
    Mesh {
        vb: gpu.buffer(&bytes_f32(&v), wgpu::BufferUsages::VERTEX),
        count: i.len() as u32,
        ib: gpu.buffer(&i.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>(), wgpu::BufferUsages::INDEX),
        inst: gpu.buffer(&bytes_f32(&flat), wgpu::BufferUsages::VERTEX),
        inst_count: instances.len() as u32,
    }
}

fn scene(gpu: &Gpu, model: f32) -> Vec<Mesh> {
    let t = |p: Vec3, r: Quat, s: Vec3| Mat4::from_scale_rotation_translation(s, r, p);
    let boxes = vec![
        instance(t(Vec3::new(0.0, -0.5, 0.0), Quat::IDENTITY, Vec3::new(30.0, 1.0, 30.0)), [0.18, 0.36, 0.14], 0.9, 0.0, model),
        instance(t(Vec3::new(-3.5, 1.0, 1.0), Quat::from_rotation_y(0.5), Vec3::splat(2.0)), [0.08, 0.2, 0.6], 0.5, 0.0, model),
        instance(t(Vec3::new(2.0, 2.0, -3.0), Quat::from_rotation_y(0.2), Vec3::new(1.2, 4.0, 1.2)), [0.75, 0.72, 0.68], 0.8, 0.0, model),
        instance(t(Vec3::new(-1.0, 0.35, 3.2), Quat::from_rotation_y(-0.3), Vec3::new(3.0, 0.7, 1.0)), [0.45, 0.25, 0.12], 0.7, 0.0, model),
    ];
    let balls = vec![
        instance(t(Vec3::new(0.0, 1.5, 0.0), Quat::IDENTITY, Vec3::splat(3.0)), [0.7, 0.08, 0.06], 0.35, 0.0, model),
        instance(t(Vec3::new(3.2, 1.0, 0.8), Quat::IDENTITY, Vec3::splat(2.0)), [1.0, 0.72, 0.3], 0.25, 1.0, model),
        instance(t(Vec3::new(-2.0, 0.6, -2.5), Quat::IDENTITY, Vec3::splat(1.2)), [0.5, 0.2, 0.55], 0.6, 0.0, model),
    ];
    // Low ring/segment counts for the flat look so facets read clearly.
    let (rings, segs) = if model == 3.0 { (8, 12) } else { (32, 48) };
    vec![make_mesh(gpu, cube(), &boxes), make_mesh(gpu, sphere(rings, segs), &balls)]
}

struct Look {
    name: &'static str,
    model: f32,
    size: (u32, u32),
    outline: Option<i32>,
    curve: u32,
    pixel: bool,
}

fn read_png(gpu: &Gpu, tex: &wgpu::Texture, path: &std::path::Path) -> Result<(), String> {
    let row = W * 4;
    let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(padded) * u64::from(H),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(H) },
        },
        wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
    );
    gpu.queue.submit(Some(enc.finish()));
    let (tx, rx) = std::sync::mpsc::channel();
    buf.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    let mapped = buf.slice(..).get_mapped_range().map_err(|e| e.to_string())?;
    let mut px = Vec::with_capacity((row * H) as usize);
    for r in mapped.chunks_exact(padded as usize) {
        px.extend_from_slice(&r[..row as usize]);
    }
    drop(mapped);
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(&px).map_err(|e| e.to_string())?;
    Ok(())
}

fn render(gpu: &Gpu, look: &Look, out_dir: &std::path::Path) -> Result<(), String> {
    let (w, h) = look.size;
    let aspect = w as f32 / h as f32;
    let eye = Vec3::new(8.0, 6.0, 10.0);
    let view = Mat4::look_at_rh(eye, Vec3::new(0.0, 1.0, 0.0), Vec3::Y);
    let proj = Mat4::perspective_rh(40f32.to_radians(), aspect, NEAR, FAR);
    let vp = proj * view;
    let sun = Vec3::new(0.45, 0.8, 0.35).normalize();
    let bands = if look.pixel { 2.0 } else { 3.0 };
    let mut frame = Vec::new();
    frame.extend_from_slice(&vp.to_cols_array());
    frame.extend_from_slice(&view.to_cols_array());
    frame.extend_from_slice(&[eye.x, eye.y, eye.z, 1.0]);
    frame.extend_from_slice(&[sun.x, sun.y, sun.z, 0.0]);
    frame.extend_from_slice(&[3.3, 3.1, 2.8, 0.0]); // sun colour * intensity
    frame.extend_from_slice(&[0.36, 0.45, 0.62, 0.0]); // sky ambient
    frame.extend_from_slice(&[0.16, 0.15, 0.12, 0.0]); // ground ambient
    frame.extend_from_slice(&[bands, 0.04, 0.12, 0.03]); // toon: bands, softness, rim, spec size
    frame.extend_from_slice(&[0.5, 0.48, 0.72, 0.0]); // toon shadow tint
    let frame_buf = gpu.buffer(&bytes_f32(&frame), wgpu::BufferUsages::UNIFORM);

    let usage_copy = wgpu::TextureUsages::COPY_SRC;
    let hdr = gpu.tex(w, h, HDR, usage_copy);
    let normals = gpu.tex(w, h, NORMALS, usage_copy);
    let depth = gpu.tex(w, h, DEPTH, usage_copy);
    let hdr_v = hdr.create_view(&Default::default());
    let normals_v = normals.create_view(&Default::default());
    let depth_v = depth.create_view(&Default::default());

    let surface = gpu.module("surface");
    let attrs0 = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
    let attrs1 = wgpu::vertex_attr_array![2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4];
    let scene_pipe = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("surface"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &surface,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout { array_stride: 24, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs0 }),
                Some(wgpu::VertexBufferLayout { array_stride: 96, step_mode: wgpu::VertexStepMode::Instance, attributes: &attrs1 }),
            ],
        },
        primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &surface,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[
                Some(wgpu::ColorTargetState { format: HDR, blend: None, write_mask: wgpu::ColorWrites::ALL }),
                Some(wgpu::ColorTargetState { format: NORMALS, blend: None, write_mask: wgpu::ColorWrites::ALL }),
            ],
        }),
        multiview_mask: None,
        cache: None,
    });
    let scene_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &scene_pipe.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: frame_buf.as_entire_binding() }],
    });
    let meshes = scene(gpu, look.model);

    let mut enc = gpu.device.create_command_encoder(&Default::default());
    {
        let sky = wgpu::Color { r: 0.42, g: 0.6, b: 0.92, a: 1.0 };
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {
                    view: &hdr_v,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(sky), store: wgpu::StoreOp::Store },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &normals_v,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.5, g: 0.5, b: 1.0, a: 0.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                }),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_v,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&scene_pipe);
        pass.set_bind_group(0, &scene_group, &[]);
        for m in &meshes {
            pass.set_vertex_buffer(0, m.vb.slice(..));
            pass.set_vertex_buffer(1, m.inst.slice(..));
            pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..m.count, 0, 0..m.inst_count);
        }
    }

    // Optional post outlines into a second HDR texture.
    let lit = if let Some(width) = look.outline {
        let outlined = gpu.tex(w, h, HDR, usage_copy);
        let module = gpu.module("outline_post");
        let pipe = gpu.fullscreen(&module, "fs_main", HDR);
        let mut p = bytes_f32(&[0.02, 0.015, 0.03, 1.0, 1.6, 1.6, 1.6, 0.6, NEAR, FAR, 0.04, 0.35]);
        p.extend(bytes_u32(&[width as u32, 0, 0, 0]));
        let ubuf = gpu.buffer(&p, wgpu::BufferUsages::UNIFORM);
        let group = gpu.bind(&pipe, &[
            ubuf.as_entire_binding(),
            wgpu::BindingResource::TextureView(&hdr_v),
            wgpu::BindingResource::TextureView(&depth_v),
            wgpu::BindingResource::TextureView(&normals_v),
        ]);
        let view = outlined.create_view(&Default::default());
        gpu.pass(&mut enc, &view, &pipe, &group);
        outlined
    } else {
        hdr
    };
    let lit_v = lit.create_view(&Default::default());

    // Identity 2x2x2 LUT (strength 0 here; a graded LUT plugs in the same way).
    let lut = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("lut"),
        size: wgpu::Extent3d { width: 2, height: 2, depth_or_array_layers: 2 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: LDR,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut lut_data = Vec::new();
    for b in 0..2u8 {
        for g in 0..2u8 {
            for r in 0..2u8 {
                lut_data.extend_from_slice(&[r * 255, g * 255, b * 255, 255]);
            }
        }
    }
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &lut, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        &lut_data,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(8), rows_per_image: Some(2) },
        wgpu::Extent3d { width: 2, height: 2, depth_or_array_layers: 2 },
    );
    let lut_v = lut.create_view(&Default::default());
    let lin = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    let final_tex = gpu.tex(W, H, if look.pixel { LDR } else { OUT_SRGB }, usage_copy);
    let final_v = final_tex.create_view(&Default::default());
    let tonemap = gpu.module("tonemap");
    // Pixel look: tonemap into a low-res UNORM target with the shader encoding sRGB itself.
    let (tm_format, encode) = if look.pixel { (LDR, 1u32) } else { (OUT_SRGB, 0u32) };
    let tm_pipe = gpu.fullscreen(&tonemap, "fs_main", tm_format);
    let mut g = bytes_f32(&[1.0]);
    g.extend(bytes_u32(&[look.curve]));
    g.extend(bytes_f32(&[1.05, 1.1, 1.0, 1.0, 1.0, 0.0, 0.0, 2.0]));
    g.extend(bytes_u32(&[encode, 0]));
    let gbuf = gpu.buffer(&g, wgpu::BufferUsages::UNIFORM);
    let tm_group = gpu.bind(&tm_pipe, &[
        gbuf.as_entire_binding(),
        wgpu::BindingResource::TextureView(&lit_v),
        wgpu::BindingResource::TextureView(&lut_v),
        wgpu::BindingResource::Sampler(&lin),
    ]);

    if look.pixel {
        let low = gpu.tex(w, h, LDR, usage_copy);
        let low_v = low.create_view(&Default::default());
        gpu.pass(&mut enc, &low_v, &tm_pipe, &tm_group);
        let quantized = gpu.tex(w, h, LDR, usage_copy);
        let quantized_v = quantized.create_view(&Default::default());
        let module = gpu.module("pixel_art");
        let q_pipe = gpu.fullscreen(&module, "fs_quantize", LDR);
        let u_pipe = gpu.fullscreen(&module, "fs_upscale", LDR);
        // A 16-colour palette made for this sample (display-space sRGB values).
        let palette: [[u8; 3]; 16] = [
            [20, 16, 28], [52, 44, 72], [92, 84, 120], [170, 200, 240],
            [230, 226, 214], [118, 160, 222], [70, 104, 176], [40, 64, 130],
            [44, 92, 40], [86, 140, 58], [150, 190, 86], [124, 64, 34],
            [196, 120, 60], [244, 196, 96], [176, 34, 30], [228, 86, 62],
        ];
        let mut q = bytes_u32(&[2]);
        q.extend(bytes_f32(&[0.0, 0.4]));
        q.extend(bytes_u32(&[4, 16, 0, 0, 0]));
        for i in 0..64 {
            let c = palette[i % 16];
            q.extend(bytes_f32(&[c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]));
        }
        let qbuf = gpu.buffer(&q, wgpu::BufferUsages::UNIFORM);
        let ubuf = gpu.buffer(&bytes_f32(&[w as f32, h as f32, W as f32, H as f32, 0.0, 0.0, 1.0, 0.0]), wgpu::BufferUsages::UNIFORM);
        let qg = gpu.bind_at(&q_pipe, &[(0, qbuf.as_entire_binding()), (2, wgpu::BindingResource::TextureView(&low_v))]);
        gpu.pass(&mut enc, &quantized_v, &q_pipe, &qg);
        let ug = gpu.bind_at(&u_pipe, &[
            (1, ubuf.as_entire_binding()),
            (2, wgpu::BindingResource::TextureView(&quantized_v)),
            (3, wgpu::BindingResource::Sampler(&lin)),
        ]);
        gpu.pass(&mut enc, &final_v, &u_pipe, &ug);
    } else {
        gpu.pass(&mut enc, &final_v, &tm_pipe, &tm_group);
    }
    gpu.queue.submit(Some(enc.finish()));
    read_png(gpu, &final_tex, &out_dir.join(format!("{}.png", look.name)))
}

fn main() -> ExitCode {
    let out_dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..wgpu::InstanceDescriptor::new_without_display_handle_from_env()
    });
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        eprintln!("no adapter");
        return ExitCode::FAILURE;
    };
    println!("adapter: {}", adapter.get_info().name);
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&Default::default())) else {
        eprintln!("no device");
        return ExitCode::FAILURE;
    };
    let gpu = Gpu { device, queue };
    let looks = [
        Look { name: "pbr", model: 0.0, size: (W, H), outline: None, curve: 0, pixel: false },
        Look { name: "toon", model: 1.0, size: (W, H), outline: Some(2), curve: 1, pixel: false },
        Look { name: "flat", model: 3.0, size: (W, H), outline: Some(1), curve: 1, pixel: false },
        Look { name: "pixel", model: 1.0, size: (320, 180), outline: Some(1), curve: 2, pixel: true },
    ];
    let mut failed = false;
    for look in &looks {
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let result = render(&gpu, look, &out_dir);
        let err = pollster::block_on(scope.pop());
        match (result, err) {
            (Ok(()), None) => println!("ok   {}", look.name),
            (r, e) => {
                failed = true;
                println!("FAIL {}: {:?} {:?}", look.name, r.err(), e.map(|e| e.to_string()));
            }
        }
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
