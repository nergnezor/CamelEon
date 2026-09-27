//! Watercolour look: translucent washes with uneven, wobbly edges, darker
//! "wet edges" where pigment pools and broken sepia ink lines (the lighting
//! grade now happens in `frame`).
//!
//! Wobble is a noise field anchored to each object, so a shape keeps the same
//! irregular edge from frame to frame while the camera moves.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use vello::kurbo::{flatten, Affine, BezPath, PathEl, Point, Stroke, Vec2};
use vello::peniko::{Color, Fill, Gradient};
use vello::Scene;

/// Flat style (early-90s cinematic platformer): solid colour shapes with no
/// outlines, no watercolour edges and no paper. Set to false for the old
/// watercolour-and-ink look.
pub const FLAT: bool = true;

pub const INK: Color = Color::from_rgb8(0x1f, 0x16, 0x1e);
/// Shadows lean towards a cool violet, lit sides towards warm peach.
const SHADOW_TINT: Color = Color::from_rgb8(0x4a, 0x3c, 0x7a);
/// Where the sun sits on screen (fraction of width and height).
pub const SUN: (f64, f64) = (0.72, 0.18);

fn mix(a: Color, b: Color, t: f32) -> Color {
    let [ar, ag, ab, aa] = a.components;
    let [br, bg, bb, ba] = b.components;
    Color::new([ar + (br - ar) * t, ag + (bg - ag) * t, ab + (bb - ab) * t, aa + (ba - aa) * t])
}

fn cool_shadow(color: Color) -> Color {
    mix(darken(color, 0.25), SHADOW_TINT, 0.4)
}

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

/// Current camera zoom relative to the default, so edge irregularities keep
/// their size on the object when the camera zooms. Set once per frame.
static ZOOM: AtomicU64 = AtomicU64::new(0x3FF0_0000_0000_0000); // 1.0

/// Screen size, so outline segments entirely off screen can be skipped.
static VIEW_W: AtomicU64 = AtomicU64::new(0x40A0_0000_0000_0000); // 2048.0
static VIEW_H: AtomicU64 = AtomicU64::new(0x40A0_0000_0000_0000);

pub fn set_view(w: f64, h: f64, zoom: f64) {
    ZOOM.store(zoom.to_bits(), Ordering::Relaxed);
    VIEW_W.store(w.to_bits(), Ordering::Relaxed);
    VIEW_H.store(h.to_bits(), Ordering::Relaxed);
}

/// Detail level, lowered on slow devices to keep 60 FPS:
/// 0 = full, 1 = single-layer washes and no paper, 2 = straight edges too
/// (no wobble), fewer background trees and no sun bloom.
static DETAIL: AtomicU8 = AtomicU8::new(0);
pub const MAX_DETAIL_DROP: u8 = 2;

pub fn set_detail(level: u8) {
    DETAIL.store(level.min(MAX_DETAIL_DROP), Ordering::Relaxed);
}

pub fn detail() -> u8 {
    DETAIL.load(Ordering::Relaxed)
}

fn zoom() -> f64 {
    f64::from_bits(ZOOM.load(Ordering::Relaxed))
}

/// Smooth 2D value noise in [-1, 1].
fn noise2(x: f64, y: f64, seed: u64) -> f64 {
    let (xi, yi) = (x.floor(), y.floor());
    let (tx, ty) = (x - xi, y - yi);
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let corner = |dx: f64, dy: f64| {
        let k = ((xi + dx) as i64 as u64).wrapping_mul(0x8DA6_B343) ^ ((yi + dy) as i64 as u64).wrapping_mul(0xD816_3841);
        rand01(seed, k) * 2.0 - 1.0
    };
    let a = corner(0.0, 0.0) + (corner(1.0, 0.0) - corner(0.0, 0.0)) * sx;
    let b = corner(0.0, 1.0) + (corner(1.0, 1.0) - corner(0.0, 1.0)) * sx;
    a + (b - a) * sy
}

/// Returns `path` with its outline displaced along the normals by smooth noise
/// of amplitude `amp` pixels.
///
/// The noise is a field anchored at `anchor` (a screen point that moves with
/// the object) and scaled with the zoom, so the irregular edge sticks to the
/// object instead of shimmering when the camera pans or zooms.
pub fn wobble(path: &BezPath, amp: f64, seed: u64, anchor: Point) -> BezPath {
    if FLAT || detail() >= 2 {
        return path.clone();
    }
    let mut polys: Vec<(Vec<Point>, bool)> = Vec::new();
    flatten(path, 0.8, |el| match el {
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

    let z = zoom();
    let field = |p: Point| {
        let q = (p - anchor) / z;
        noise2(q.x / 38.0, q.y / 38.0, seed) + 0.4 * noise2(q.x / 15.0, q.y / 15.0, seed ^ 0xabc)
    };
    let mut out = BezPath::new();
    for (poly, closed) in polys {
        if poly.len() < 2 {
            continue;
        }
        // Resample from each corner so long straight edges wobble too; the
        // samples stay put relative to the corners.
        let n = poly.len();
        let segs = if closed { n } else { n - 1 };
        let perimeter: f64 = (0..segs).map(|i| (poly[(i + 1) % n] - poly[i]).hypot()).sum();
        // The noise's finest detail is ~15 px, so ~6 px steps are plenty.
        let step = (perimeter / 600.0).max(6.0);
        let (vw, vh) = (f64::from_bits(VIEW_W.load(Ordering::Relaxed)), f64::from_bits(VIEW_H.load(Ordering::Relaxed)));
        let margin = 40.0;
        let off = |a: Point, b: Point| {
            (a.x < -margin && b.x < -margin)
                || (a.y < -margin && b.y < -margin)
                || (a.x > vw + margin && b.x > vw + margin)
                || (a.y > vh + margin && b.y > vh + margin)
        };
        let mut pts = Vec::with_capacity((perimeter / step) as usize + n);
        for i in 0..segs {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            // Nobody sees a wobble off screen: keep just the corner.
            let parts = if off(a, b) { 1 } else { ((b - a).hypot() / step).ceil().max(1.0) as usize };
            for k in 0..parts {
                pts.push(a.lerp(b, k as f64 / parts as f64));
            }
        }
        if !closed {
            pts.push(poly[n - 1]);
        }
        let m = pts.len();
        for i in 0..m {
            let prev = if i == 0 { if closed { pts[m - 1] } else { pts[0] } } else { pts[i - 1] };
            let next = if i + 1 == m { if closed { pts[0] } else { pts[m - 1] } } else { pts[i + 1] };
            let t = next - prev;
            let normal = if t.hypot() > 1e-9 { Vec2::new(-t.y, t.x).normalize() } else { Vec2::ZERO };
            let p = pts[i] + normal * field(pts[i]) * amp;
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
pub fn wash(scene: &mut Scene, path: &BezPath, color: Color, glaze: Option<&Gradient>, size: f64, seed: u64, anchor: Point) {
    wash_with_edge(scene, path, color, glaze, size, seed, anchor, 0.55);
}

/// Like `wash`, with control over how strongly pigment pools at the edge
/// (distant things get softer edges).
#[allow(clippy::too_many_arguments)]
pub fn wash_with_edge(
    scene: &mut Scene,
    path: &BezPath,
    color: Color,
    glaze: Option<&Gradient>,
    size: f64,
    seed: u64,
    anchor: Point,
    edge_alpha: f32,
) {
    if FLAT {
        // Flat colour, like a 16-bit era painted background.
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, path);
        if let Some(g) = glaze {
            scene.fill(Fill::NonZero, Affine::IDENTITY, g, None, path);
        }
        let _ = (size, seed, anchor, edge_alpha);
        return;
    }
    let amp = (size * 0.035).clamp(0.3, 6.0);
    let main = wobble(path, amp, seed, anchor);
    if size < 40.0 || detail() >= 1 {
        // Small shapes: a single layer looks the same and costs half.
        scene.fill(Fill::NonZero, Affine::IDENTITY, color.with_alpha(0.9), None, &main);
    } else {
        scene.fill(Fill::NonZero, Affine::IDENTITY, color.with_alpha(0.78), None, &main);
        scene.fill(Fill::NonZero, Affine::IDENTITY, color.with_alpha(0.4), None, &wobble(path, amp * 1.8, seed ^ 0x55, anchor));
    }
    if let Some(g) = glaze {
        let glazed = if detail() >= 1 { main.clone() } else { wobble(path, amp * 0.6, seed ^ 0x77, anchor) };
        scene.fill(Fill::NonZero, Affine::IDENTITY, g, None, &glazed);
    }
    let edge = (size * 0.03).clamp(0.8, 3.5);
    scene.stroke(
        &Stroke::new(edge),
        Affine::IDENTITY,
        darken(color, 0.3).with_alpha(edge_alpha),
        None,
        &main,
    );
}

/// A shading glaze for round things: clear towards the light, pigment on the
/// shadow side.
pub fn ball_glaze(center: Point, radius: f64, color: Color) -> Gradient {
    let hot = center + Vec2::new(-0.45, -0.55) * radius * 0.6;
    let shadow = cool_shadow(color);
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
    let shadow = cool_shadow(color);
    Gradient::new_linear(mid + n * width, mid - n * width).with_stops([
        (0.0, Color::WHITE.with_alpha(0.3)),
        (0.35, shadow.with_alpha(0.0)),
        (0.75, shadow.with_alpha(0.15)),
        (1.0, shadow.with_alpha(0.45)),
    ])
}

/// A bold, confident ink outline, slightly uneven like a hand-inked cel.
pub fn ink(scene: &mut Scene, path: &BezPath, size: f64, seed: u64, anchor: Point) {
    if FLAT {
        return;
    }
    let width = (size * 0.055).clamp(1.2, 4.0);
    let stroke = Stroke::new(width).with_join(vello::kurbo::Join::Round).with_caps(vello::kurbo::Cap::Round);
    scene.stroke(&stroke, Affine::IDENTITY, INK, None, &wobble(path, width * 0.25, seed ^ 0x99, anchor));
}

/// Cartoon cel painting for characters and props: a flat colour with a crisp
/// shadow shape on the side away from the light, a small highlight and a bold
/// ink outline, in the style of 1930s animation.
pub fn cel(scene: &mut Scene, path: &BezPath, color: Color, shadow: Option<&Gradient>, size: f64, seed: u64, anchor: Point) {
    let body = cel_fill(scene, path, color, shadow, size, seed, anchor);
    let width = (size * 0.055).clamp(1.2, 4.0);
    let stroke = Stroke::new(width).with_join(vello::kurbo::Join::Round).with_caps(vello::kurbo::Cap::Round);
    scene.stroke(&stroke, Affine::IDENTITY, INK, None, &body);
}

/// The fill part of `cel`, without the outline (for figures that get one
/// shared outline around their whole silhouette). Returns the painted shape.
pub fn cel_fill(scene: &mut Scene, path: &BezPath, color: Color, shadow: Option<&Gradient>, size: f64, seed: u64, anchor: Point) -> BezPath {
    let body = wobble(path, (size * 0.012).clamp(0.3, 1.5), seed, anchor);
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &body);
    if let Some(g) = shadow {
        scene.fill(Fill::NonZero, Affine::IDENTITY, g, None, &body);
    }
    body
}

/// Crisp cel shadow along a tube.
pub fn tube_cel(mid: Point, axis: Vec2, width: f64, color: Color) -> Gradient {
    let mut n = Vec2::new(-axis.y, axis.x);
    n = if n.hypot() > 1e-9 { n.normalize() } else { Vec2::new(1.0, 0.0) };
    if n.dot(Vec2::new(-0.45, -0.55)) < 0.0 {
        n = -n;
    }
    let shadow = cool_shadow(color);
    Gradient::new_linear(mid + n * width, mid - n * width).with_stops([
        (0.0, shadow.with_alpha(0.0)),
        (0.62, shadow.with_alpha(0.0)),
        (0.63, shadow.with_alpha(0.85)),
        (1.0, shadow.with_alpha(0.85)),
    ])
}
