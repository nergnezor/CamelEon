//! Shader-rendered hair. The game describes hair as strands (screen-space
//! polylines); a small wgpu pass draws them as tapered, soft-edged ribbons
//! with Kajiya–Kay style anisotropic highlights (a sharp white band and a
//! softer tinted one) into a texture, which Vello then draws as an image in
//! the right layer of the scene.

use vello::kurbo::{Point, Vec2};
use vello::peniko::{ImageAlphaType, ImageData};
use vello::wgpu::{self, util::DeviceExt};
use vello::Renderer;

/// How Konrad's hair is drawn: shader strands, vector locks, or both (the
/// strands over the locks).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HairMode {
    Shader,
    Both,
    Vector,
}

impl HairMode {
    pub fn next(self) -> Self {
        match self {
            Self::Shader => Self::Both,
            Self::Both => Self::Vector,
            Self::Vector => Self::Shader,
        }
    }
}

/// What the game needs to draw the hair: the image the shader renders into
/// (when strands are on) and whether to draw the vector locks.
#[derive(Clone, Copy, Default)]
pub struct HairStyle<'a> {
    pub image: Option<&'a ImageData>,
    pub locks: bool,
}

/// Side of the square hair texture, in pixels.
pub const TEXTURE_SIZE: u32 = 512;

/// One hair strand, in screen pixels.
pub struct Strand {
    pub points: Vec<Point>,
    /// Width at the root, in pixels; strands taper towards the tip.
    pub width: f64,
    pub color: [f32; 3],
    /// Per-strand randomness for the highlight and streaks, 0..1.
    pub seed: f32,
}

/// Everything the hair pass needs for one frame.
pub struct HairFrame {
    pub strands: Vec<Strand>,
    /// Screen-space square the texture covers: top-left corner and side.
    pub origin: Point,
    pub size: f64,
    /// Centre of the head, for the strands' outward normals.
    pub center: Point,
    /// Direction towards the light, in screen space.
    pub light: Vec2,
}

impl HairFrame {
    /// Where the hair texture goes on screen.
    pub fn transform(&self) -> vello::kurbo::Affine {
        vello::kurbo::Affine::translate(self.origin.to_vec2()) * vello::kurbo::Affine::scale(self.size / TEXTURE_SIZE as f64)
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    pos: [f32; 2],
    along_across: [f32; 2],
    tangent: [f32; 2],
    normal: [f32; 2],
    color: [f32; 3],
    seed: f32,
}

const SHADER: &str = r#"
struct Uniforms {
    light: vec2<f32>,
    pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

struct VertexIn {
    @location(0) pos: vec2<f32>,
    @location(1) along_across: vec2<f32>,
    @location(2) tangent: vec2<f32>,
    @location(3) normal: vec2<f32>,
    @location(4) color: vec3<f32>,
    @location(5) seed: f32,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) along_across: vec2<f32>,
    @location(1) tangent: vec2<f32>,
    @location(2) normal: vec2<f32>,
    @location(3) color: vec3<f32>,
    @location(4) seed: f32,
};

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(v.pos, 0.0, 1.0);
    out.along_across = v.along_across;
    out.tangent = v.tangent;
    out.normal = v.normal;
    out.color = v.color;
    out.seed = v.seed;
    return out;
}

// Kajiya–Kay: highlight strength from the sine between the hair tangent and
// the half vector; shifting the tangent along the normal moves the band.
fn strand_spec(t: vec3<f32>, h: vec3<f32>, exponent: f32) -> f32 {
    let d = dot(t, h);
    return pow(sqrt(max(1.0 - d * d, 0.0)), exponent);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let along = in.along_across.x;
    let across = abs(in.along_across.y);
    // Soft ribbon edges, and tips that fade out.
    let alpha = (1.0 - smoothstep(0.3, 1.0, across)) * (1.0 - 0.6 * smoothstep(0.75, 1.0, along));

    // Screen y points down; flip to a y-up frame with z towards the viewer.
    let t = normalize(vec3<f32>(in.tangent.x, -in.tangent.y, 0.0));
    let n = normalize(vec3<f32>(in.normal.x, -in.normal.y, 0.75));
    let l = normalize(vec3<f32>(u.light.x, -u.light.y, 0.9));
    let h = normalize(l + vec3<f32>(0.0, 0.0, 1.0));
    let shift = (in.seed - 0.5) * 0.35;
    let primary = strand_spec(normalize(t + n * (0.35 + shift)), h, 80.0);
    let secondary = strand_spec(normalize(t + n * (-0.1 + shift)), h, 18.0);

    let diffuse = mix(0.35, 1.0, clamp(dot(n, l) * 0.5 + 0.5, 0.0, 1.0));
    let root = mix(0.5, 1.0, smoothstep(0.0, 0.45, along));
    // Fine streaks so it reads as many hairs, not one ribbon.
    let streak = 0.85 + 0.3 * fract(sin(in.seed * 91.7 + floor(across * 3.0)) * 43758.5);
    var color = in.color * diffuse * root * streak;
    color += vec3<f32>(0.85, 0.93, 1.0) * primary * 0.5 * root;
    color += in.color * 1.8 * secondary * 0.3;
    return vec4<f32>(color * alpha, alpha);
}
"#;

pub struct HairRenderer {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// The texture as a Vello image, to draw in the scene.
    pub image: ImageData,
}

impl HairRenderer {
    pub fn new(device: &wgpu::Device, renderer: &mut Renderer) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hair"),
            size: wgpu::Extent3d { width: TEXTURE_SIZE, height: TEXTURE_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut image = renderer.register_texture(texture.clone());
        // The pass writes premultiplied colour.
        image.alpha_type = ImageAlphaType::AlphaPremultiplied;

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hair uniforms"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hair"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hair"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hair"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hair"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let premultiplied = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hair"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x2, 4 => Float32x3, 5 => Float32
                    ],
                }],
            },
            primitive: wgpu::PrimitiveState::default(),
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
        Self { texture, view, pipeline, uniforms, bind_group, image }
    }

    /// Draws the strands into the hair texture. Submit this before Vello
    /// renders the scene that uses the image.
    pub fn render(&self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &HairFrame) {
        let _ = &self.texture;
        let to_tex = TEXTURE_SIZE as f64 / frame.size.max(1.0);
        let ndc = |p: Point| {
            let x = (p.x - frame.origin.x) * to_tex;
            let y = (p.y - frame.origin.y) * to_tex;
            [(x / TEXTURE_SIZE as f64 * 2.0 - 1.0) as f32, (1.0 - y / TEXTURE_SIZE as f64 * 2.0) as f32]
        };
        let mut vertices: Vec<Vertex> = Vec::new();
        for strand in &frame.strands {
            let n = strand.points.len();
            if n < 2 {
                continue;
            }
            let side_at = |i: usize| {
                let a = strand.points[i.saturating_sub(1)];
                let b = strand.points[(i + 1).min(n - 1)];
                let t = b - a;
                let t = if t.hypot() > 1e-9 { t.normalize() } else { Vec2::new(1.0, 0.0) };
                (t, Vec2::new(-t.y, t.x))
            };
            let corner = |i: usize, across: f64| {
                let along = i as f64 / (n - 1) as f64;
                let (tangent, side) = side_at(i);
                let half = strand.width * 0.5 * (1.0 - 0.85 * along);
                let p = strand.points[i] + side * half * across;
                let normal = (strand.points[i] - frame.center).normalize();
                Vertex {
                    pos: ndc(p),
                    along_across: [along as f32, across as f32],
                    tangent: [tangent.x as f32, tangent.y as f32],
                    normal: [normal.x as f32, normal.y as f32],
                    color: strand.color,
                    seed: strand.seed,
                }
            };
            for i in 0..n - 1 {
                let (a0, a1, b0, b1) = (corner(i, -1.0), corner(i, 1.0), corner(i + 1, -1.0), corner(i + 1, 1.0));
                vertices.extend_from_slice(&[a0, a1, b0, a1, b1, b0]);
            }
        }
        let light = frame.light.normalize();
        queue.write_buffer(&self.uniforms, 0, &f32s_as_bytes(&[light.x as f32, light.y as f32, 0.0, 0.0]));
        let bytes = unsafe { std::slice::from_raw_parts(vertices.as_ptr() as *const u8, std::mem::size_of_val(vertices.as_slice())) };
        let vertex_buffer = (!vertices.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("hair strands"),
                contents: bytes,
                usage: wgpu::BufferUsages::VERTEX,
            })
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("hair") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hair"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(buffer) = &vertex_buffer {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..vertices.len() as u32, 0..1);
            }
        }
        queue.submit([encoder.finish()]);
    }
}

fn f32s_as_bytes(values: &[f32; 4]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (i, v) in values.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}
