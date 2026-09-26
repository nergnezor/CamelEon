//! Watercolour look: translucent washes with uneven, wobbly edges, darker
//! "wet edges" where pigment pools, broken sepia ink lines and a paper texture
//! multiplied over the whole picture.
//!
//! Wobble is driven by a per-shape seed and the position along the outline, so
//! a shape keeps the same irregular edge from frame to frame.

use std::f64::consts::TAU;
use std::sync::OnceLock;

use vello::kurbo::{flatten, Affine, BezPath, PathEl, Point, Rect, Stroke, Vec2};
use vello::peniko::{Blob, BlendMode, Brush, Color, Extend, Fill, Gradient, ImageAlphaType, ImageBrush, ImageData, ImageFormat, Mix};
use vello::Scene;

pub const INK: Color = Color::from_rgb8(0x3b, 0x2a, 0x1e);

pub fn darken(c: Color, amount: f32) -> Color {
    let [r, g, b, a] = c.components;
    let k = 1.0 - amount;
    Color::new([r * k, g * k, b * k, a])
}

pub fn lighten(c: Color, amount: f32) -> Color {
    let [r, g, b, a] = c.components;
    Color::new([r + (1.0 - r) * amount, g + (1.0 - g) * amount, b + (1.0 - b) * amount, a])
}

/// A stable seed from anything hashable-ish: mix a few numbers together.
pub fn seed(values: &[f64]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in values {
        h ^= (v * 1000.0) as i64 as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn rand01(seed: u64, k: u64) -> f64 {
    let mut x = seed ^ k.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// Returns `path` with its outline displaced along the normals by smooth noise
/// of amplitude `amp` pixels.
pub fn wobble(path: &BezPath, amp: f64, seed: u64) -> BezPath {
    let mut polys: Vec<(Vec<Point>, bool)> = Vec::new();
    flatten(path, 0.4, |el| match el {
        PathEl::MoveTo(p) => polys.push((vec![p], false)),
        PathEl::LineTo(p) => {
            if let Some((poly, _)) = polys.last_mut() {
                poly.push(p);
            }
        }
        PathEl::ClosePath => {
            if let Some((_, closed)) = polys.last_mut() {
                *closed = true;
            }
        }
        _ => {}
    });

    let mut out = BezPath::new();
    for (poly, closed) in polys {
        if poly.len() < 2 {
            continue;
        }
        // Resample so long straight edges wobble too.
        let mut pts = Vec::with_capacity(poly.len() * 2);
        let n = poly.len();
        let segs = if closed { n } else { n - 1 };
        let perimeter: f64 = (0..segs).map(|i| (poly[(i + 1) % n] - poly[i]).hypot()).sum();
        let step = (perimeter / 400.0).max(5.0);
        for i in 0..segs {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            let len = (b - a).hypot();
            let parts = (len / step).ceil().max(1.0) as usize;
            for k in 0..parts {
                pts.push(a.lerp(b, k as f64 / parts as f64));
            }
        }
        if !closed {
            pts.push(poly[n - 1]);
        }

        // Periodic noise along the outline, with a few octaves.
        let waves = [(60.0, 1.0), (23.0, 0.45), (9.0, 0.2)].map(|(wavelength, weight)| {
            let freq = (perimeter / wavelength).round().max(1.0);
            (freq, weight, rand01(seed, freq as u64) * TAU)
        });
        let m = pts.len();
        let mut s = 0.0;
        for i in 0..m {
            if i > 0 {
                s += (pts[i] - pts[i - 1]).hypot();
            }
            let prev = if i == 0 { if closed { pts[m - 1] } else { pts[0] } } else { pts[i - 1] };
            let next = if i + 1 == m { if closed { pts[0] } else { pts[m - 1] } } else { pts[i + 1] };
            let t = next - prev;
            let normal = if t.hypot() > 1e-9 { Vec2::new(-t.y, t.x).normalize() } else { Vec2::ZERO };
            let phase = s / perimeter.max(1e-9) * TAU;
            let noise: f64 = waves.iter().map(|(f, w, p)| w * (phase * f + p).sin()).sum();
            let p = pts[i] + normal * noise * amp;
            if i == 0 {
                out.move_to(p);
            } else {
                out.line_to(p);
            }
        }
        if closed {
            out.close_path();
        }
    }
    out
}

/// Paints a watercolour wash: two translucent, differently wobbled layers of
/// pigment, an optional shading glaze, and a darker wet edge. `size` is the
/// shape's rough size in pixels and scales the irregularity.
pub fn wash(scene: &mut Scene, path: &BezPath, color: Color, glaze: Option<&Gradient>, size: f64, seed: u64) {
    wash_with_edge(scene, path, color, glaze, size, seed, 0.55);
}

/// Like `wash`, with control over how strongly pigment pools at the edge
/// (distant things get softer edges).
pub fn wash_with_edge(
    scene: &mut Scene,
    path: &BezPath,
    color: Color,
    glaze: Option<&Gradient>,
    size: f64,
    seed: u64,
    edge_alpha: f32,
) {
    let amp = (size * 0.035).clamp(0.3, 6.0);
    scene.fill(Fill::NonZero, Affine::IDENTITY, color.with_alpha(0.78), None, &wobble(path, amp, seed));
    scene.fill(Fill::NonZero, Affine::IDENTITY, color.with_alpha(0.4), None, &wobble(path, amp * 1.8, seed ^ 0x55));
    if let Some(g) = glaze {
        scene.fill(Fill::NonZero, Affine::IDENTITY, g, None, &wobble(path, amp * 0.6, seed ^ 0x77));
    }
    let edge = (size * 0.03).clamp(0.8, 3.5);
    scene.stroke(
        &Stroke::new(edge),
        Affine::IDENTITY,
        darken(color, 0.3).with_alpha(edge_alpha),
        None,
        &wobble(path, amp * 0.8, seed ^ 0x33),
    );
}

/// A shading glaze for round things: clear towards the light, pigment on the
/// shadow side.
pub fn ball_glaze(center: Point, radius: f64, color: Color) -> Gradient {
    let hot = center + Vec2::new(-0.45, -0.55) * radius * 0.6;
    let shadow = darken(color, 0.35);
    Gradient::new_two_point_radial(hot, 0.0_f32, center, (radius * 1.05) as f32).with_stops([
        (0.0, Color::WHITE.with_alpha(0.35)),
        (0.3, shadow.with_alpha(0.0)),
        (0.75, shadow.with_alpha(0.15)),
        (1.0, shadow.with_alpha(0.5)),
    ])
}

/// A shading glaze across a tube.
pub fn tube_glaze(mid: Point, axis: Vec2, width: f64, color: Color) -> Gradient {
    let mut n = Vec2::new(-axis.y, axis.x);
    n = if n.hypot() > 1e-9 { n.normalize() } else { Vec2::new(1.0, 0.0) };
    if n.dot(Vec2::new(-0.45, -0.55)) < 0.0 {
        n = -n;
    }
    let shadow = darken(color, 0.35);
    Gradient::new_linear(mid + n * width, mid - n * width).with_stops([
        (0.0, Color::WHITE.with_alpha(0.3)),
        (0.35, shadow.with_alpha(0.0)),
        (0.75, shadow.with_alpha(0.15)),
        (1.0, shadow.with_alpha(0.45)),
    ])
}

/// A loose sepia ink line with gaps, like a quick pen sketch over the paint.
pub fn ink(scene: &mut Scene, path: &BezPath, size: f64, seed: u64) {
    let width = (size * 0.05).clamp(0.8, 2.2);
    let dash = (size * 0.6).clamp(12.0, 80.0);
    let offset = rand01(seed, 1) * dash * 3.0;
    let stroke = Stroke::new(width).with_dashes(offset, [dash, dash * 0.12, dash * 0.7, dash * 0.2]);
    scene.stroke(&stroke, Affine::IDENTITY, INK.with_alpha(0.75), None, &wobble(path, width * 0.6, seed ^ 0x99));
}

/// Cold-pressed watercolour paper: a tileable grain texture.
fn paper_texture() -> &'static ImageData {
    static PAPER: OnceLock<ImageData> = OnceLock::new();
    PAPER.get_or_init(|| {
        const N: usize = 256;
        // Periodic value noise at a few scales, so the tile repeats seamlessly.
        let lattice = |cells: usize, oct: u64| {
            let mut grid = vec![0.0; cells * cells];
            for (i, g) in grid.iter_mut().enumerate() {
                *g = rand01(0xa11ce ^ oct, i as u64);
            }
            move |x: f64, y: f64| {
                let fx = x * cells as f64;
                let fy = y * cells as f64;
                let (x0, y0) = (fx.floor() as usize % cells, fy.floor() as usize % cells);
                let (x1, y1) = ((x0 + 1) % cells, (y0 + 1) % cells);
                let (tx, ty) = (fx.fract(), fy.fract());
                let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
                let a = grid[y0 * cells + x0] + (grid[y0 * cells + x1] - grid[y0 * cells + x0]) * sx;
                let b = grid[y1 * cells + x0] + (grid[y1 * cells + x1] - grid[y1 * cells + x0]) * sx;
                a + (b - a) * sy
            }
        };
        let octaves = [(lattice(8, 1), 0.35), (lattice(32, 2), 0.3), (lattice(64, 3), 0.2), (lattice(128, 4), 0.15)];
        let mut data = Vec::with_capacity(N * N * 4);
        for y in 0..N {
            for x in 0..N {
                let (u, v) = (x as f64 / N as f64, y as f64 / N as f64);
                let n: f64 = octaves.iter().map(|(f, w)| f(u, v) * w).sum();
                let speck = if rand01(0x5eed, (y * N + x) as u64) > 0.995 { 0.06 } else { 0.0 };
                let k = 0.88 + 0.12 * n - speck;
                let paper = [0.99, 0.965, 0.92];
                for c in paper {
                    data.push(((c * k).clamp(0.0, 1.0) * 255.0) as u8);
                }
                data.push(255);
            }
        }
        ImageData {
            data: Blob::from(data),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: N as u32,
            height: N as u32,
        }
    })
}

/// Multiplies paper grain and a soft vignette over everything drawn so far.
pub fn paper(scene: &mut Scene, w: f64, h: f64) {
    let rect = Rect::new(0.0, 0.0, w, h);
    scene.push_layer(Fill::NonZero, BlendMode::from(Mix::Multiply), 1.0, Affine::IDENTITY, &rect);
    let brush: Brush = ImageBrush::new(paper_texture().clone()).with_extend(Extend::Repeat).into();
    scene.fill(Fill::NonZero, Affine::IDENTITY, &brush, None, &rect);
    let vignette = Gradient::new_radial(Point::new(w / 2.0, h / 2.0), (w.max(h) * 0.75) as f32).with_stops([
        (0.0, Color::WHITE),
        (0.7, Color::WHITE),
        (1.0, Color::from_rgb8(0xd8, 0xcc, 0xb4)),
    ]);
    scene.fill(Fill::NonZero, Affine::IDENTITY, &vignette, None, &rect);
    scene.pop_layer();
}
