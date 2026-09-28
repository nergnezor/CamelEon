//! Shader-rendered grass. The game describes each row of grass as a patch of
//! blades (curved, tapering, in screen pixels); a wgpu pass draws all the
//! blades, instanced, into one atlas texture, and each patch is then drawn
//! by Vello as an image in its place in the scene's depth order (so grass in
//! front of Konrad still hides his feet).

use vello::kurbo::{Affine, Point, Rect, Vec2};
use vello::peniko::{ImageAlphaType, ImageBrush, ImageData};
use vello::wgpu::{self, util::DeviceExt};
use vello::{Renderer, Scene};

/// Size of the atlas the patches are packed into, in pixels.
pub const ATLAS_WIDTH: u32 = 2048;
pub const ATLAS_HEIGHT: u32 = 1024;
/// Empty pixels round each patch, so filtering doesn't pick up a neighbour.
const PADDING: f64 = 2.0;
/// Segments along each blade.
const SEGMENTS: u32 = 6;

/// One blade of grass: a quadratic curve from `base` through `ctrl` to
/// `tip` (atlas pixels), `width` pixels wide at the base.
#[repr(C)]
#[derive(Clone, Copy)]
struct Blade {
    base: [f32; 2],
    ctrl: [f32; 2],
    tip: [f32; 2],
    width: f32,
    seed: f32,
    root_color: [f32; 4],
    tip_color: [f32; 4],
}

/// A blade in screen space, as the game describes it.
pub struct BladeShape {
    pub base: Point,
    /// A point halfway along the blade; it curves through it.
    pub mid: Point,
    pub tip: Point,
    pub width: f64,
    pub root_color: [f32; 3],
    pub tip_color: [f32; 3],
    pub seed: f32,
}

/// Where a patch went in the atlas, and how to draw it.
#[derive(Clone)]
pub struct Patch {
    image: ImageData,
    /// The patch's area in the atlas.
    atlas: Rect,
    /// Atlas to screen.
    transform: Affine,
}

impl Patch {
    pub fn draw(&self, scene: &mut Scene) {
        scene.fill(vello::peniko::Fill::NonZero, self.transform, &ImageBrush::new(self.image.clone()), None, &self.atlas);
    }
}

/// This frame's grass: the blades of all patches, packed into the atlas in
/// rows ("shelves").
#[derive(Default)]
pub struct GrassFrame {
    /// The atlas as a Vello image; `None` turns grass off.
    pub image: Option<ImageData>,
    blades: Vec<Blade>,
    shelf: (f64, f64, f64),
    /// Direction towards the sun, in screen space, and how strongly it shines
    /// through the blades (less in the rain).
    pub light: Vec2,
    pub backlight: f32,
}

impl GrassFrame {
    pub fn reset(&mut self) {
        self.blades.clear();
        self.shelf = (0.0, 0.0, 0.0);
    }

    /// Adds a patch of blades and returns how to draw it, or `None` if
    /// there's no room left (or grass is off).
    pub fn patch(&mut self, blades: &[BladeShape]) -> Option<Patch> {
        let image = self.image.clone()?;
        let mut bounds = Rect::from_points(blades.first()?.base, blades[0].tip);
        for b in blades {
            let pad = b.width;
            for p in [b.base, b.mid, b.tip] {
                bounds = bounds.union(Rect::new(p.x - pad, p.y - pad, p.x + pad, p.y + pad));
            }
        }
        // Wider than the atlas: render it smaller and scale it back up.
        let scale = ((ATLAS_WIDTH as f64 - 2.0 * PADDING) / bounds.width()).min(1.0);
        let (w, h) = (bounds.width() * scale + 2.0 * PADDING, bounds.height() * scale + 2.0 * PADDING);
        let (mut x, mut y, mut shelf_h) = self.shelf;
        if x + w > ATLAS_WIDTH as f64 {
            (x, y, shelf_h) = (0.0, y + shelf_h, 0.0);
        }
        if y + h > ATLAS_HEIGHT as f64 {
            return None;
        }
        self.shelf = (x + w, y, shelf_h.max(h));
        let origin = Vec2::new(x + PADDING, y + PADDING);
        let to_atlas = |p: Point| {
            let q = origin + (p - bounds.origin()) * scale;
            [q.x as f32, q.y as f32]
        };
        for b in blades {
            // The control point that makes the curve pass through `mid`.
            let ctrl = b.mid + (b.mid - b.base.midpoint(b.tip));
            let rgba = |c: [f32; 3]| [c[0], c[1], c[2], 1.0];
            self.blades.push(Blade {
                base: to_atlas(b.base),
                ctrl: to_atlas(ctrl),
                tip: to_atlas(b.tip),
                width: (b.width * scale) as f32,
                seed: b.seed,
                root_color: rgba(b.root_color),
                tip_color: rgba(b.tip_color),
            });
        }
        Some(Patch {
            image,
            atlas: Rect::new(x, y, x + w, y + h),
            transform: Affine::translate(bounds.origin().to_vec2())
                * Affine::scale(1.0 / scale)
                * Affine::translate(-origin),
        })
    }
}

const SHADER: &str = r#"
struct Uniforms {
    light: vec2<f32>,
    backlight: f32,
    pad: f32,
    atlas: vec2<f32>,
    pad2: vec2<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

struct Blade {
    @location(0) base: vec2<f32>,
    @location(1) ctrl: vec2<f32>,
    @location(2) tip: vec2<f32>,
    @location(3) width: f32,
    @location(4) seed: f32,
    @location(5) root_color: vec4<f32>,
    @location(6) tip_color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    // Along the blade 0..1, and pixels from its centre line.
    @location(0) along: f32,
    @location(1) across: f32,
    @location(2) half_width: f32,
    @location(3) facing: f32,
    @location(4) root_color: vec3<f32>,
    @location(5) tip_color: vec3<f32>,
    @location(6) seed: f32,
};

const SEGMENTS: f32 = 6.0;

@vertex
fn vs_main(@builtin(vertex_index) i: u32, b: Blade) -> VertexOut {
    // A triangle strip: pairs of vertices on either side of the curve.
    let t = f32(i / 2u) / SEGMENTS;
    let side = f32(i % 2u) * 2.0 - 1.0;
    let s = 1.0 - t;
    let p = s * s * b.base + 2.0 * s * t * b.ctrl + t * t * b.tip;
    let d = 2.0 * s * (b.ctrl - b.base) + 2.0 * t * (b.tip - b.ctrl);
    let dir = normalize(d + vec2<f32>(1e-5, 0.0));
    let normal = vec2<f32>(-dir.y, dir.x);
    // Taper to a point; half a pixel extra on each side for antialiasing.
    let half_width = 0.5 * b.width * pow(s, 0.8);
    let across = side * (half_width + 0.75);
    let q = p + normal * across;
    var out: VertexOut;
    out.position = vec4<f32>(q.x / u.atlas.x * 2.0 - 1.0, 1.0 - q.y / u.atlas.y * 2.0, 0.0, 1.0);
    out.along = t;
    out.across = across;
    out.half_width = half_width;
    // Which way the blade's face turns towards the sun (screen y is down).
    out.facing = dot(normal, vec2<f32>(u.light.x, u.light.y));
    out.root_color = b.root_color.rgb;
    out.tip_color = b.tip_color.rgb;
    out.seed = b.seed;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Coverage from the distance to the edge, in pixels.
    let coverage = clamp(in.half_width - abs(in.across) + 0.5, 0.0, 1.0);
    if (coverage <= 0.0) {
        discard;
    }
    let t = in.along;
    var color = mix(in.root_color, in.tip_color, smoothstep(0.0, 1.0, t));
    // Shade down among the other blades.
    color *= mix(0.6, 1.0, smoothstep(0.0, 0.4, t));
    // A lighter midrib, and a darker edge on the side away from the sun.
    let rel = in.across / max(in.half_width, 0.5);
    color *= 1.0 + 0.12 * (1.0 - abs(rel)) - 0.12 * max(-rel * sign(in.facing), 0.0);
    // Sunlight shining through the upper blade.
    let glow = u.backlight * smoothstep(0.35, 1.0, t) * (0.55 + 0.45 * abs(in.facing)) * (0.7 + 0.6 * in.seed);
    color += in.tip_color * vec3<f32>(1.1, 1.05, 0.6) * glow * 0.55;
    return vec4<f32>(color * coverage, coverage);
}
"#;

/// Draws the frame's grass into the atlas.
pub struct GrassRenderer {
    view: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// The atlas as a Vello image, to draw the patches from.
    pub image: ImageData,
}

impl GrassRenderer {
    pub fn new(device: &wgpu::Device, renderer: &mut Renderer) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("grass"),
            size: wgpu::Extent3d { width: ATLAS_WIDTH, height: ATLAS_HEIGHT, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut image = renderer.register_texture(texture);
        // The pass writes premultiplied colour.
        image.alpha_type = ImageAlphaType::AlphaPremultiplied;

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grass uniforms"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grass"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grass"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grass"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grass"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let premultiplied = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grass"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Blade>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32, 4 => Float32, 5 => Float32x4, 6 => Float32x4
                    ],
                }],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState { color: premultiplied, alpha: premultiplied }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { view, pipeline, uniforms, bind_group, image }
    }

    /// Draws the frame's blades into the atlas. Returns false if there were
    /// none (then the atlas is left as it was). Submit before Vello renders
    /// the scene that uses it.
    pub fn render(&self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &GrassFrame) -> bool {
        if frame.blades.is_empty() {
            return false;
        }
        let light = if frame.light.hypot() > 1e-9 { frame.light.normalize() } else { Vec2::new(0.55, -0.83) };
        let values: [f32; 8] = [light.x as f32, light.y as f32, frame.backlight, 0.0, ATLAS_WIDTH as f32, ATLAS_HEIGHT as f32, 0.0, 0.0];
        let mut bytes = [0u8; 32];
        for (i, v) in values.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        queue.write_buffer(&self.uniforms, 0, &bytes);
        // SAFETY: `Blade` is `repr(C)` and made of plain `f32`s.
        let data = unsafe { std::slice::from_raw_parts(frame.blades.as_ptr() as *const u8, std::mem::size_of_val(frame.blades.as_slice())) };
        let instances = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grass blades"),
            contents: data,
            usage: wgpu::BufferUsages::VERTEX,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("grass") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("grass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, instances.slice(..));
            pass.draw(0..(SEGMENTS + 1) * 2, 0..frame.blades.len() as u32);
        }
        queue.submit([encoder.finish()]);
        true
    }
}
