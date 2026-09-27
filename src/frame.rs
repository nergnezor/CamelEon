//! Frame rendering shared by every frontend (window, web, terminal, snapshots).
//!
//! The game draws three layers with Vello: the far background (sky, distant
//! hills) and the middle distance at half resolution, and the foreground at
//! full resolution. Two small GPU passes then finish the picture:
//!
//! 1. Light, at quarter resolution: volumetric light shafts marched from the
//!    sun through the gaps between the mid and front layers, plus bloom from
//!    the foreground's brightest pixels.
//! 2. Composite: the background layers blurred for depth of field (the far
//!    one more), the sharp foreground over them, the light added, and the
//!    grade (sunlight fading to dusk, vignette, sun glow, rain).
//!
//! In the dusk city the sky itself is a shader too: a sunset gradient, the
//! low sun and drifting noise clouds lit from below, drawn wherever the far
//! layer leaves a gap.

use vello::peniko::{Color, ImageData};
use vello::wgpu;
use vello::{AaConfig, Renderer, RendererOptions, Scene};

use crate::hair::{HairFrame, HairMode, HairRenderer, HairStyle};
use crate::level::Theme;

/// What the game draws into, each frame.
#[derive(Default)]
pub struct Layers {
    /// Sky and distant background, drawn at half resolution (see `Game::draw`).
    pub far: Scene,
    /// Middle distance, half resolution.
    pub mid: Scene,
    /// Everything in focus, full resolution.
    pub front: Scene,
}

impl Layers {
    fn reset(&mut self) {
        self.far.reset();
        self.mid.reset();
        self.front.reset();
    }
}

/// Look settings for the finishing passes.
#[derive(Clone, Copy)]
pub struct Post {
    /// The sun's position on screen, 0..1.
    pub sun: [f32; 2],
    /// 0 = dry, 1 = pouring.
    pub rain: f32,
    /// Strength of the light shafts.
    pub rays: f32,
    /// Whether to apply the grade (off for the GPU benchmark).
    pub grade: bool,
    /// Which world's sky and grade: the jungle's, or the dusk city's.
    pub theme: Theme,
    /// Seconds of game time, for drifting clouds.
    pub time: f32,
    /// How far the camera has panned (world units), so the clouds move
    /// with a little parallax.
    pub pan: f32,
}

impl Default for Post {
    fn default() -> Self {
        Self { sun: [0.72, 0.18], rain: 0.0, rays: 1.0, grade: true, theme: Theme::Jungle, time: 0.0, pan: 0.0 }
    }
}

/// What the game returns from drawing a frame.
#[derive(Default)]
pub struct FrameInfo {
    pub hair: Option<HairFrame>,
    pub post: Post,
}

const SHARED_WGSL: &str = r#"
struct Uniforms {
    sun: vec2<f32>,
    // 1 / size of the half-resolution layers.
    half_texel: vec2<f32>,
    aspect: f32,
    rain: f32,
    rays: f32,
    grade: f32,
    // 0 = jungle, 1 = dusk city.
    theme: f32,
    time: f32,
    pan: f32,
    spare: f32,
};
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var far_tex: texture_2d<f32>;
@group(0) @binding(3) var mid_tex: texture_2d<f32>;
@group(0) @binding(4) var front_tex: texture_2d<f32>;
@group(0) @binding(5) var light_tex: texture_2d<f32>;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VertexOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VertexOut;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.3, 0.59, 0.11));
}

// ---- Dusk sky ----
// The camera looks straight ahead, so the horizon is the middle of the screen.
const HORIZON: f32 = 0.5;

fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q += dot(q, q + 45.32);
    return fract(q.x * q.y);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < 5; i++) {
        sum += value_noise(q) * amp;
        // Rotate each octave so the lattice doesn't show.
        q = mat2x2<f32>(1.6, 1.2, -1.2, 1.6) * q + vec2<f32>(3.1, 1.7);
        amp *= 0.5;
    }
    return sum;
}

// Distance to the sun in screen heights.
fn sun_distance(uv: vec2<f32>) -> f32 {
    return length((uv - u.sun) * vec2<f32>(u.aspect, 1.0));
}

// The clear sky: a sunset gradient by height above the horizon, warmer and
// brighter towards the sun, and the sun's disc.
fn dusk_gradient(uv: vec2<f32>) -> vec3<f32> {
    let e = HORIZON - uv.y;
    let gold = vec3<f32>(1.0, 0.76, 0.47);
    let coral = vec3<f32>(1.0, 0.55, 0.43);
    let pink = vec3<f32>(0.9, 0.4, 0.53);
    let violet = vec3<f32>(0.55, 0.27, 0.52);
    let night = vec3<f32>(0.2, 0.13, 0.33);
    var col = mix(gold, coral, smoothstep(0.0, 0.07, e));
    col = mix(col, pink, smoothstep(0.05, 0.2, e));
    col = mix(col, violet, smoothstep(0.17, 0.36, e));
    col = mix(col, night, smoothstep(0.3, 0.55, e));
    // Below the horizon: haze over the city.
    col = mix(col, vec3<f32>(0.93, 0.56, 0.5), smoothstep(0.0, -0.08, e));
    let d = sun_distance(uv);
    // The glow round the sun, and the horizon brightening towards it.
    col += vec3<f32>(1.0, 0.62, 0.35) * (0.55 * exp(-d * d * 18.0) + 0.25 * exp(-d * 3.5));
    col += vec3<f32>(1.0, 0.7, 0.45) * 0.3 * exp(-abs(e) * 30.0) * exp(-abs(uv.x - u.sun.x) * u.aspect * 1.6);
    return col;
}

fn sun_disc(uv: vec2<f32>) -> f32 {
    // Flattened a little by the thick air near the horizon.
    let p = (uv - u.sun) * vec2<f32>(u.aspect, 1.12);
    return smoothstep(0.036, 0.031, length(p));
}

// Cloud density at a screen point: noise on a plane high above, seen in
// perspective, so clouds stretch out and crowd together near the horizon.
fn cloud_density(uv: vec2<f32>) -> f32 {
    let e = max(HORIZON - uv.y, 0.012);
    let plane = vec2<f32>((uv.x - 0.5) * u.aspect / e, 1.0 / e);
    let drift = vec2<f32>(u.pan * 0.05 + u.time * 0.06, 0.0);
    let q = (plane + drift) * vec2<f32>(0.22, 0.9);
    let n = fbm(q + vec2<f32>(0.0, 7.0));
    // Long bands of stratus, thinning out high up and at the horizon.
    let band = smoothstep(0.02, 0.09, e) * (1.0 - smoothstep(0.3, 0.46, e));
    return smoothstep(0.5, 0.78, n) * band;
}

fn dusk_sky(uv: vec2<f32>) -> vec3<f32> {
    var col = dusk_gradient(uv);
    col = mix(col, vec3<f32>(1.0, 0.95, 0.82), sun_disc(uv));
    let density = cloud_density(uv);
    if density > 0.001 {
        // Lit from below by the low sun: the cloud's underside glows where
        // it thins out towards the sun, and everything near the sun burns.
        let toward = normalize((u.sun - uv) * vec2<f32>(u.aspect, 1.0) + vec2<f32>(0.0, 0.25));
        let ahead = cloud_density(uv + toward * vec2<f32>(1.0 / u.aspect, 1.0) * 0.012);
        let edge = clamp((density - ahead) * 2.5 + 0.35, 0.0, 1.0);
        let near = exp(-sun_distance(uv) * 2.2);
        let shade = mix(vec3<f32>(0.42, 0.22, 0.42), vec3<f32>(0.78, 0.36, 0.48), smoothstep(0.1, 0.35, HORIZON - uv.y) * 0.6);
        let lit = mix(vec3<f32>(1.0, 0.56, 0.4), vec3<f32>(1.0, 0.84, 0.56), near);
        let cloud = mix(shade, lit, clamp(edge * (0.45 + 0.9 * near), 0.0, 1.0));
        col = mix(col, cloud, density * 0.92);
    }
    return col;
}

// Sky brightness for the light shafts: the gradient and sun without clouds
// (it's sampled many times per pixel).
fn sky_light(uv: vec2<f32>) -> f32 {
    if u.theme > 0.5 {
        let d = sun_distance(uv);
        return 0.4 * luma(dusk_gradient(uv)) + 1.2 * exp(-d * d * 60.0);
    }
    return 0.0;
}
"#;

const LIGHT_WGSL: &str = r#"
// How much sunlight gets through at `uv`: the gaps in the mid and front
// layers, weighted by how bright the sky behind them is.
fn shaft_source(uv: vec2<f32>) -> f32 {
    let mid = textureSampleLevel(mid_tex, smp, uv, 0.0).a;
    let front = textureSampleLevel(front_tex, smp, uv, 0.0).a;
    let far = textureSampleLevel(far_tex, smp, uv, 0.0);
    // In the dusk city the far layer only holds the skyline: the sky is
    // the shader's, behind it.
    let sky = luma(far.rgb) * select(1.0, far.a, u.theme > 0.5) + sky_light(uv) * (1.0 - far.a);
    return (1.0 - max(mid, front)) * smoothstep(0.25, 0.75, sky);
}

@fragment
fn fs_light(in: VertexOut) -> @location(0) vec4<f32> {
    // Light shafts: march from the pixel towards the sun, gathering light.
    let steps = 28;
    let delta = (in.uv - u.sun) * (0.95 / f32(steps));
    var coord = in.uv;
    var decay = 1.0;
    var shafts = 0.0;
    for (var i = 0; i < steps; i++) {
        coord -= delta;
        shafts += shaft_source(coord) * decay;
        decay *= 0.93;
    }
    shafts *= u.rays * 0.032 * (1.0 - 0.7 * u.rain);
    // Only below the canopy edge do shafts read as shafts, not as sky glow.
    shafts *= smoothstep(0.0, 0.35, in.uv.y - u.sun.y + 0.1);
    // At dusk the rays hang in the air above the roofs; the buildings
    // themselves stand in their own shade.
    if u.theme > 0.5 {
        shafts *= 1.0 - 0.85 * textureSampleLevel(front_tex, smp, in.uv, 0.0).a;
    }
    // A low evening sun throws long, strong, orange rays.
    let warm = select(vec3<f32>(1.0, 0.86, 0.62), vec3<f32>(1.0, 0.6, 0.4) * 1.6, u.theme > 0.5);

    // Bloom from the foreground's brightest pixels.
    var bloom = vec3<f32>(0.0);
    let o = u.half_texel;
    for (var j = 0; j < 4; j++) {
        let off = vec2<f32>(select(-1.0, 1.0, (j & 1) == 1), select(-1.0, 1.0, (j & 2) == 2)) * o;
        let c = textureSampleLevel(front_tex, smp, in.uv + off, 0.0);
        bloom += max(c.rgb * c.a - vec3<f32>(0.72), vec3<f32>(0.0));
    }
    bloom *= 0.9;
    return vec4<f32>(warm * shafts + bloom, 1.0);
}
"#;

const COMPOSITE_WGSL: &str = r#"
// Blur for depth of field, keeping transparent areas from darkening edges
// (the layers store straight alpha, so blend premultiplied).
fn blurred(tex: texture_2d<f32>, uv: vec2<f32>, radius: f32) -> vec4<f32> {
    var acc = vec4<f32>(0.0);
    var total = 0.0;
    for (var i = 0; i < 9; i++) {
        var off = vec2<f32>(0.0);
        var w = 0.2;
        if i > 0 {
            let a = f32(i) * 0.785398 + 0.3;
            off = vec2<f32>(cos(a), sin(a)) * radius * u.half_texel * select(1.0, 0.55, (i & 1) == 0);
            w = 0.1;
        }
        let c = textureSampleLevel(tex, smp, uv + off, 0.0);
        acc += vec4<f32>(c.rgb * c.a, c.a) * w;
        total += w;
    }
    return acc / total;
}

@fragment
fn fs_composite(in: VertexOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let far = blurred(far_tex, uv, 2.6);
    let mid = blurred(mid_tex, uv, 1.1);
    var back = far.rgb;
    if u.theme > 0.5 && far.a < 0.999 {
        back += dusk_sky(uv) * (1.0 - far.a);
    }
    var col = back * (1.0 - mid.a) + mid.rgb;
    let front = textureSampleLevel(front_tex, smp, uv, 0.0);
    col = front.rgb * front.a + col * (1.0 - front.a);
    col += textureSampleLevel(light_tex, smp, uv, 0.0).rgb;

    if u.grade > 0.5 && u.theme > 0.5 {
        // Evening: warm near the sun, rose and lilac away from it, a
        // rose-tinted vignette and a burning glow round the low sun.
        let s = sun_distance(uv);
        let warmth = exp(-s * 1.4);
        col *= mix(vec3<f32>(0.86, 0.74, 0.9), vec3<f32>(1.06, 0.96, 0.9), warmth);
        let p = (uv - 0.5) * vec2<f32>(u.aspect, 1.0) / max(u.aspect, 1.0);
        let v = smoothstep(0.4, 0.8, length(p));
        col *= mix(vec3<f32>(1.0), vec3<f32>(0.62, 0.46, 0.62), v);
        let glow = vec3<f32>(1.0, 0.6, 0.38) * (0.4 * exp(-s * s * 14.0) + 0.1 * exp(-s * 2.5));
        col = 1.0 - (1.0 - col) * (1.0 - glow);
    } else if u.grade > 0.5 {
        // Sunlight falling off into violet dusk away from the sun.
        let away = vec2<f32>(1.0 - u.sun.x - 0.3, 1.1);
        let d = away - u.sun;
        let t = clamp(dot(uv - u.sun, d) / dot(d, d), 0.0, 1.0);
        let c0 = vec3<f32>(0.902, 0.878, 0.871);
        let c1 = vec3<f32>(0.8, 0.737, 0.776);
        let c2 = vec3<f32>(0.408, 0.337, 0.518);
        let dusk = select(mix(c1, c2, (t - 0.4) / 0.6), mix(c0, c1, t / 0.4), t < 0.4);
        col *= dusk;
        // Vignette.
        let p = (uv - 0.5) * vec2<f32>(u.aspect, 1.0) / max(u.aspect, 1.0);
        let v = smoothstep(0.42, 0.75, length(p));
        col *= mix(vec3<f32>(1.0), vec3<f32>(0.72, 0.64, 0.66), v);
        // Warm glow round the sun.
        let s = length((uv - u.sun) * vec2<f32>(u.aspect, 1.0));
        let glow = vec3<f32>(1.0, 0.78, 0.56) * 0.18 * exp(-s * s * 12.0) * (1.0 - 0.8 * u.rain);
        col = 1.0 - (1.0 - col) * (1.0 - glow);
        // Rain: cooler, greyer and darker.
        let grey = vec3<f32>(luma(col)) * vec3<f32>(0.85, 0.95, 1.1);
        col = mix(col, grey, 0.4 * u.rain) * (1.0 - 0.22 * u.rain);
    }
    return vec4<f32>(clamp(col, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
"#;

struct Targets {
    width: u32,
    height: u32,
    far: wgpu::TextureView,
    mid: wgpu::TextureView,
    front: wgpu::TextureView,
    light: wgpu::TextureView,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    out_texture: wgpu::Texture,
    out: wgpu::TextureView,
    /// The light pass can't read the texture it writes, so it gets its own
    /// bind group (with the far layer standing in for the light texture).
    light_group: wgpu::BindGroup,
    composite_group: wgpu::BindGroup,
}

pub struct FrameRenderer {
    pub renderer: Renderer,
    hair: HairRenderer,
    pub hair_mode: HairMode,
    layers: Layers,
    targets: Option<Targets>,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    light: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
}

fn half(v: u32) -> u32 {
    v.div_ceil(2).max(1)
}

impl FrameRenderer {
    pub fn new(device: &wgpu::Device) -> Result<Self, vello::Error> {
        let mut renderer = Renderer::new(
            device,
            RendererOptions {
                antialiasing_support: vello::AaSupport::area_only(),
                ..Default::default()
            },
        )?;
        let hair = HairRenderer::new(device, &mut renderer);

        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture_entry(2),
                texture_entry(3),
                texture_entry(4),
                texture_entry(5),
            ],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("post uniforms"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |label: &str, body: &str, entry: &str| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(format!("{SHARED_WGSL}{body}").into()),
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let light = pipeline("light", LIGHT_WGSL, "fs_light");
        let composite = pipeline("composite", COMPOSITE_WGSL, "fs_composite");
        Ok(Self {
            renderer,
            hair,
            hair_mode: HairMode::Both,
            layers: Layers::default(),
            targets: None,
            layout,
            uniforms,
            sampler,
            light,
            composite,
        })
    }

    /// The hair image to pass to `Game::draw`, if shader hair is on.
    pub fn hair_image(&self) -> Option<ImageData> {
        (self.hair_mode != HairMode::Vector).then(|| self.hair.image.clone())
    }

    /// Whether the game should draw the vector hair locks.
    pub fn hair_locks(&self) -> bool {
        self.hair_mode != HairMode::Shader
    }

    fn targets(&mut self, device: &wgpu::Device, width: u32, height: u32) -> &Targets {
        if !matches!(&self.targets, Some(t) if t.width == width && t.height == height) {
            let texture = |label, w, h, usage| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage,
                    view_formats: &[],
                })
            };
            let vello_usage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
            let pass_usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
            let view = |t: wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
            let far = view(texture("far", half(width), half(height), vello_usage));
            let mid = view(texture("mid", half(width), half(height), vello_usage));
            let front = view(texture("front", width, height, vello_usage));
            let light = view(texture("light", width.div_ceil(4).max(1), height.div_ceil(4).max(1), pass_usage));
            let out_texture = texture("frame", width, height, pass_usage | wgpu::TextureUsages::COPY_SRC);
            let out = out_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let group = |light_input: &wgpu::TextureView| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("post"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: self.uniforms.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&far) },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&mid) },
                        wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&front) },
                        wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(light_input) },
                    ],
                })
            };
            let light_group = group(&far);
            let composite_group = group(&light);
            self.targets = Some(Targets { width, height, far, mid, front, light, out_texture, out, light_group, composite_group });
        }
        self.targets.as_ref().expect("targets were just created")
    }

    /// Renders a frame of `width`×`height`: `draw` fills the layers (it's
    /// given the hair image when shader hair is on) and returns the frame's
    /// settings. Returns the finished picture.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        draw: impl FnOnce(&mut Layers, HairStyle) -> FrameInfo,
    ) -> Result<&wgpu::TextureView, vello::Error> {
        self.layers.reset();
        let hair_image = self.hair_image();
        let locks = self.hair_locks();
        let info = draw(&mut self.layers, HairStyle { image: hair_image.as_ref(), locks });
        if let Some(frame) = &info.hair {
            self.hair.render(device, queue, frame);
            // Vello caches images in its atlas; without this it keeps
            // showing the first frame's strands.
            self.renderer.mark_override_image_dirty(&self.hair.image);
        }
        self.targets(device, width, height);
        let t = self.targets.as_ref().expect("targets exist");
        let params = |w, h, base: Color| vello::RenderParams { base_color: base, width: w, height: h, antialiasing_method: AaConfig::Area };
        let clear = Color::TRANSPARENT;
        // The dusk sky is drawn by the composite shader behind the far layer.
        let far_base = if info.post.theme == Theme::Dusk { clear } else { Color::BLACK };
        self.renderer.render_to_texture(device, queue, &self.layers.far, &t.far, &params(half(width), half(height), far_base))?;
        self.renderer.render_to_texture(device, queue, &self.layers.mid, &t.mid, &params(half(width), half(height), clear))?;
        self.renderer.render_to_texture(device, queue, &self.layers.front, &t.front, &params(width, height, clear))?;

        let p = info.post;
        let values: [f32; 12] = [
            p.sun[0],
            p.sun[1],
            1.0 / half(width) as f32,
            1.0 / half(height) as f32,
            width as f32 / height.max(1) as f32,
            p.rain,
            p.rays,
            if p.grade { 1.0 } else { 0.0 },
            if p.theme == Theme::Dusk { 1.0 } else { 0.0 },
            p.time,
            p.pan,
            0.0,
        ];
        let mut bytes = [0u8; 48];
        for (i, v) in values.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        queue.write_buffer(&self.uniforms, 0, &bytes);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("post") });
        for (pipeline, view, group) in [(&self.light, &t.light, &t.light_group), (&self.composite, &t.out, &t.composite_group)] {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
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
        queue.submit([encoder.finish()]);
        Ok(&t.out)
    }

    /// Copies the last frame to the CPU as tightly packed RGBA pixels.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn read_pixels(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let t = self.targets.as_ref().ok_or("nothing rendered yet")?;
        let padded_row = (t.width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: padded_row as u64 * t.height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &t.out_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded_row), rows_per_image: None },
            },
            wgpu::Extent3d { width: t.width, height: t.height, depth_or_array_layers: 1 },
        );
        queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely())?;
        let row = (t.width * 4) as usize;
        let mut pixels = Vec::with_capacity(row * t.height as usize);
        {
            let mapped = slice.get_mapped_range();
            for chunk in mapped.chunks(padded_row as usize) {
                pixels.extend_from_slice(&chunk[..row]);
            }
        }
        buffer.unmap();
        Ok(pixels)
    }
}
