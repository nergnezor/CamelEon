//! The wilds: an alien planet in soft pastels, in the spirit of Scavengers
//! Reign. Rolling hills of mint moss over coral and plum strata, strange
//! flora (glowing bulb stalks, ribbed fans, pods, tube worms, fiddleheads),
//! loops grown from pearly shell with a glowing seam, and far off a ringed
//! planet, spires, drifting sky jellies, parasol trees, a crashed ship and
//! the ribs of something enormous.
//!
//! On slow devices the detail levels (`paint::detail`) thin things out:
//! level 1 drops the glowing specks in the moss, the drifting spores and
//! the far sky jellies, level 2 also the foreground and half the flora.

use std::f64::consts::{PI, TAU};

use glam::{DVec2, DVec3};
use vello::kurbo::{Affine, Arc, BezPath, Cap, Circle, Ellipse, Join, Point, Rect, Stroke, Vec2};
use vello::peniko::{Color, Fill, Gradient};
use vello::Scene;

use crate::canvas3d::{darken, lighten, mix, Camera, Canvas3d};
use crate::grass::{BladeShape, GrassFrame};
use crate::jungle::WorldView;
use crate::level::{Block, BlockKind, Hill, Level, Loop};
use crate::noise::{fbm1, hash, noise1};

/// Where the sun sits on screen: high on the right, pale in the haze.
pub const SUN: (f64, f64) = (0.7, 0.18);
/// Towards the sun on screen, for rim light.
pub const SUN_SCREEN_DIR: Vec2 = Vec2::new(0.55, -0.83);
/// The colour of sunlight on edges.
pub const RIM: Color = Color::from_rgb8(0xff, 0xf0, 0xd0);

const SKY_TOP: Color = Color::from_rgb8(0x5e, 0xa0, 0xa4);
const SKY_MID: Color = Color::from_rgb8(0xd8, 0xcc, 0xb4);
const SKY_HORIZON: Color = Color::from_rgb8(0xe6, 0xae, 0x94);
/// Distance haze: far things fade into the cream-peach of the horizon.
const HAZE: Color = Color::from_rgb8(0xdc, 0xbc, 0xac);

/// The ground: mint moss on top, then coral, rust and plum strata.
const MOSS: Color = Color::from_rgb8(0x7c, 0xc8, 0xa4);
const MOSS_LIGHT: Color = Color::from_rgb8(0xc4, 0xf0, 0xc8);
const MOSS_DEEP: Color = Color::from_rgb8(0x3a, 0x86, 0x7c);
const STRATA: [Color; 4] = [
    Color::from_rgb8(0xe0, 0x84, 0x70),
    Color::from_rgb8(0xb0, 0x52, 0x5c),
    Color::from_rgb8(0x74, 0x36, 0x58),
    Color::from_rgb8(0x3c, 0x22, 0x44),
];
/// The loops' shell: pearly, lilac in shade, with a glowing seam.
const PEARL: Color = Color::from_rgb8(0xe0, 0xd2, 0xde);
const PEARL_SHADE: Color = Color::from_rgb8(0x94, 0x80, 0xb4);
const SEAM: Color = Color::from_rgb8(0x6a, 0xf0, 0xe0);
/// Bioluminescence: warm bulbs and cool specks.
const GLOW_WARM: Color = Color::from_rgb8(0xff, 0xd0, 0x7a);
const GLOW_PINK: Color = Color::from_rgb8(0xff, 0x9a, 0xc8);
const GLOW_COOL: Color = Color::from_rgb8(0x8a, 0xf6, 0xff);
const CORAL: Color = Color::from_rgb8(0xf0, 0x8e, 0x6e);
const LILAC: Color = Color::from_rgb8(0xb8, 0x9e, 0xe0);

/// Camouflage colours in the wilds: moss in shade, and lit.
pub const CAMO: (Color, Color) = (MOSS_DEEP, MOSS);

/// Depth of the hills (they're drawn as slabs from `Z0` to `Z1`).
const Z0: f64 = -1.4;
const Z1: f64 = 3.0;

/// The sun, for things lit in the wilds.
pub fn light() -> crate::soft::Light {
    crate::soft::Light { screen: SUN_SCREEN_DIR, world: DVec3::new(0.4, 0.8, -0.45), rim: RIM }
}

/// A soft round glow.
fn glow(scene: &mut Scene, at: Point, r: f64, color: Color, alpha: f32) {
    if r < 0.5 || alpha <= 0.0 {
        return;
    }
    let g = Gradient::new_radial(at, r as f32).with_stops([(0.0, color.with_alpha(alpha)), (0.4, color.with_alpha(alpha * 0.35)), (1.0, color.with_alpha(0.0))]);
    scene.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &Circle::new(at, r));
}

/// A tapered strip along `pts`, `w0` wide at the first point and `w1` at the
/// last.
fn taper(pts: &[Point], w0: f64, w1: f64) -> BezPath {
    let n = pts.len();
    let mut path = BezPath::new();
    if n < 2 {
        return path;
    }
    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(n);
    for i in 0..n {
        let d = pts[(i + 1).min(n - 1)] - pts[i.saturating_sub(1)];
        let len = d.hypot().max(1e-6);
        let normal = Vec2::new(-d.y, d.x) / len;
        let w = (w0 + (w1 - w0) * i as f64 / (n - 1) as f64) * 0.5;
        left.push(pts[i] + normal * w);
        right.push(pts[i] - normal * w);
    }
    path.move_to(left[0]);
    for p in &left[1..] {
        path.line_to(*p);
    }
    for p in right.iter().rev() {
        path.line_to(*p);
    }
    path.close_path();
    path
}

/// A closed ring of points round `center` in the plane at depth `z`.
fn circle3(cam: &Camera, center: DVec2, r: f64, z: f64, path: &mut BezPath) {
    let n = 56;
    for i in 0..n {
        let a = i as f64 / n as f64 * TAU;
        let p = cam.point(DVec3::new(center.x + r * a.cos(), center.y + r * a.sin(), z));
        if i == 0 { path.move_to(p) } else { path.line_to(p) }
    }
    path.close_path();
}

/// A stalk bending under its own weight and the breeze: points from `base`
/// up `height`, the tip leaning `lean`.
fn stalk_points(cam: &Camera, base: DVec3, height: f64, lean: f64) -> Vec<Point> {
    (0..=8)
        .map(|k| {
            let t = k as f64 / 8.0;
            cam.point(base + DVec3::new(lean * t * t, height * t, 0.0))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Background

/// A ridge line across the view at depth `z`, filled down to the bottom of
/// the image. Sampled at fixed world x, so it stays put while panning.
fn ridge(cam: &Camera, w: f64, h: f64, z: f64, step: f64, height: impl Fn(f64) -> f64) -> BezPath {
    let (x0, x1) = cam.visible_x(z, w);
    let first = (x0 / step).floor() as i64;
    let last = (x1 / step).ceil() as i64;
    let mut path = BezPath::new();
    for i in first..=last {
        let x = i as f64 * step;
        let p = cam.point(DVec3::new(x, height(x), z));
        if i == first { path.move_to(p) } else { path.line_to(p) }
    }
    path.line_to((w + 100.0, h + 100.0));
    path.line_to((-100.0, h + 100.0));
    path.close_path();
    path
}

/// The sky, a ringed planet, spires, sky jellies and rolling hills with
/// parasol trees go into `far`; the nearer hills with the wrecks, the ribs
/// and the bulb thickets into `mid`. `cam`, `w` and `h` describe the
/// (half-resolution) layer images.
pub fn draw_background(far: &mut Scene, mid: &mut Scene, cam: &Camera, w: f64, h: f64, time: f64) {
    let detail = crate::paint::detail();
    let id = Affine::IDENTITY;
    // The horizon sits where the camera's eye level is.
    let horizon = cam.center.y + cam.eye.y * cam.focal / 400.0;
    let sky = Gradient::new_linear((0.0, 0.0), (0.0, h)).with_stops([
        (0.0, SKY_TOP),
        (0.4, SKY_MID),
        ((horizon / h).clamp(0.45, 0.9) as f32, SKY_HORIZON),
        (1.0, SKY_HORIZON),
    ]);
    far.fill(Fill::NonZero, id, &sky, None, &Rect::new(0.0, 0.0, w, h));
    let sun = Point::new(w * SUN.0, h * SUN.1);
    glow(far, sun, h * 0.3, Color::from_rgb8(0xff, 0xf0, 0xd0), 0.35);
    far.fill(Fill::NonZero, id, Color::from_rgb8(0xff, 0xfc, 0xf0), None, &Circle::new(sun, h * 0.035));

    draw_planet(far, w, h);
    if detail < 1 {
        draw_sky_jellies(far, cam, w, time);
    }

    // Far spires: soft rolling land broken by tall, thin termite-like
    // towers.
    let spires = |x: f64| {
        let mut y = -6.0 + 10.0 * (0.5 + 0.5 * fbm1(x * 0.012, 3, 11));
        let cell = (x / 22.0).floor() as i64;
        for i in cell - 1..=cell + 1 {
            let c = (i as f64 + hash(i, 12)) * 22.0;
            let tall = 40.0 * hash(i, 13).powi(3);
            let d = (x - c) / (1.5 + 2.0 * hash(i, 14));
            y += tall * (-d * d).exp();
        }
        y
    };
    let color = mix(Color::from_rgb8(0x8c, 0x7a, 0xbc), HAZE, 0.5);
    far.fill(Fill::NonZero, id, color, None, &ridge(cam, w, h, 170.0, 1.2, spires));

    // Rolling hills with parasol trees: a thin stem, a wide flat cap and a
    // glowing rim under it.
    let hills = |x: f64| -3.0 + 6.0 * (0.5 + 0.5 * fbm1(x * 0.04, 3, 21));
    let z = 70.0;
    let color = mix(Color::from_rgb8(0x5a, 0x92, 0xa4), HAZE, 0.32);
    let (x0, x1) = cam.visible_x(z, w);
    let spacing = 9.0;
    for i in (x0 / spacing).floor() as i64 - 1..=(x1 / spacing).ceil() as i64 + 1 {
        if hash(i, 22) < 0.35 {
            continue;
        }
        let x = (i as f64 + hash(i, 23)) * spacing;
        let size = 5.0 + 7.0 * hash(i, 24);
        let base = DVec3::new(x, hills(x) - 0.5, z);
        let sway = (time * 0.3 + i as f64).sin() * 0.3;
        let stem = stalk_points(cam, base, size, size * 0.15 + sway);
        let top = *stem.last().unwrap();
        let s = cam.project(base).scale;
        let cap = Ellipse::new(top, (size * 0.45 * s, size * 0.12 * s), 0.0);
        let tone = mix(color, lighten(mix(CORAL, LILAC, hash(i, 25)), 0.2), 0.35);
        far.fill(Fill::NonZero, id, darken(color, 0.05), None, &taper(&stem, 0.35 * s, 0.2 * s));
        far.fill(Fill::NonZero, id, tone, None, &cap);
        let rim = Ellipse::new(top + Vec2::new(0.0, size * 0.06 * s), (size * 0.4 * s, size * 0.04 * s), 0.0);
        far.fill(Fill::NonZero, id, mix(GLOW_WARM, HAZE, 0.3).with_alpha(0.8), None, &rim);
    }
    far.fill(Fill::NonZero, id, color, None, &ridge(cam, w, h, z, 1.0, hills));

    // Nearer: a crashed ship and giant ribs half buried in the hills.
    let near_hills = |x: f64| -2.5 + 3.5 * (0.5 + 0.5 * fbm1(x * 0.07, 3, 31));
    let z = 30.0;
    let color = mix(Color::from_rgb8(0x3e, 0x74, 0x80), HAZE, 0.2);
    for &x in &[40.0, 210.0, 355.0] {
        draw_wreck(mid, cam, x, near_hills(x), z, time, color);
    }
    for &x in &[125.0, 290.0] {
        draw_ribs(mid, cam, x, near_hills(x), z, color);
    }
    mid.fill(Fill::NonZero, id, color, None, &ridge(cam, w, h, z, 0.8, near_hills));

    // The nearest background: a thicket of bulb stalks on a low bank.
    let bank = |x: f64| -3.2 + 1.8 * (0.5 + 0.5 * fbm1(x * 0.11, 2, 41));
    let z = 12.0;
    let color = mix(Color::from_rgb8(0x36, 0x66, 0x6c), HAZE, 0.12);
    let (x0, x1) = cam.visible_x(z, w);
    let spacing = 2.2;
    for i in (x0 / spacing).floor() as i64 - 1..=(x1 / spacing).ceil() as i64 + 1 {
        if hash(i, 42) < 0.3 || (detail >= 2 && i % 2 == 0) {
            continue;
        }
        let x = (i as f64 + hash(i, 43)) * spacing;
        let height = 1.5 + 3.0 * hash(i, 44);
        let sway = (time * 0.7 + i as f64 * 1.3).sin() * 0.2;
        let stem = stalk_points(cam, DVec3::new(x, bank(x) - 0.3, z), height, sway + (hash(i, 45) - 0.5));
        let s = cam.project(DVec3::new(x, 0.0, z)).scale;
        let top = *stem.last().unwrap();
        mid.fill(Fill::NonZero, id, color, None, &taper(&stem, 0.16 * s, 0.06 * s));
        let bulb = Ellipse::new(top, (0.22 * s, 0.3 * s), 0.0);
        let hue = if hash(i, 46) < 0.5 { GLOW_WARM } else { GLOW_PINK };
        let pulse = 0.7 + 0.3 * (time * 1.5 + i as f64).sin();
        glow(mid, top, 0.9 * s, hue, (0.35 * pulse) as f32);
        mid.fill(Fill::NonZero, id, mix(color, hue, 0.7 * pulse), None, &bulb);
    }
    mid.fill(Fill::NonZero, id, color, None, &ridge(cam, w, h, z, 0.5, bank));
}

/// A ringed gas giant hanging in the sky, pale in the haze.
fn draw_planet(scene: &mut Scene, w: f64, h: f64) {
    let id = Affine::IDENTITY;
    let c = Point::new(w * 0.2, h * 0.2);
    let r = h * 0.16;
    let tilt = -0.32;
    let ring = |scene: &mut Scene, from: f64| {
        let arc = Arc::new(c, (r * 1.9, r * 0.42), from, PI, tilt);
        scene.stroke(&Stroke::new(r * 0.12), id, Color::from_rgb8(0xf4, 0xe4, 0xe8).with_alpha(0.55), None, &arc);
        scene.stroke(&Stroke::new(r * 0.04), id, Color::from_rgb8(0xd8, 0xc0, 0xd8).with_alpha(0.6), None, &Arc::new(c, (r * 1.62, r * 0.35), from, PI, tilt));
    };
    // The back of the ring first, then the planet, then the front.
    ring(scene, PI);
    let disc = Circle::new(c, r);
    let body = Gradient::new_linear(c - Vec2::new(r, r), c + Vec2::new(r, r)).with_stops([
        Color::from_rgb8(0xf8, 0xe0, 0xe0),
        Color::from_rgb8(0xd8, 0xb8, 0xd8),
    ]);
    scene.fill(Fill::NonZero, id, &body, None, &disc);
    scene.push_clip_layer(Fill::NonZero, id, &disc);
    for k in 0..4 {
        let y = c.y - r * 0.6 + k as f64 * r * 0.4;
        let band = Ellipse::new(Point::new(c.x, y), (r * 1.2, r * 0.07 * (1.0 + k as f64 * 0.3)), tilt * 0.4);
        scene.fill(Fill::NonZero, id, Color::from_rgb8(0xc4, 0xa0, 0xc8).with_alpha(0.35), None, &band);
    }
    // The night side, away from the sun.
    scene.fill(Fill::NonZero, id, Color::from_rgb8(0x6a, 0x8e, 0xa8).with_alpha(0.4), None, &Circle::new(c + Vec2::new(-r * 0.55, r * 0.35), r * 1.05));
    scene.pop_layer();
    ring(scene, 0.0);
    // Haze in front of it.
    scene.fill(Fill::NonZero, id, SKY_MID.with_alpha(0.3), None, &disc);
}

/// Enormous jellies drifting high up, trailing long tentacles.
fn draw_sky_jellies(scene: &mut Scene, cam: &Camera, w: f64, time: f64) {
    let id = Affine::IDENTITY;
    let z = 150.0;
    let (x0, x1) = cam.visible_x(z, w);
    let spacing = 140.0;
    for i in (x0 / spacing).floor() as i64 - 1..=(x1 / spacing).ceil() as i64 + 1 {
        let x = (i as f64 + hash(i, 61)) * spacing + time * 0.8;
        let y = 28.0 + 14.0 * hash(i, 62) + (time * 0.2 + i as f64).sin() * 1.5;
        let size = 7.0 + 5.0 * hash(i, 63);
        let pr = cam.project(DVec3::new(x, y, z));
        let s = pr.scale * size;
        let c = pr.pos;
        let pulse = (time * 0.6 + i as f64).sin();
        let color = mix(GLOW_PINK, HAZE, 0.55);
        // Tentacles first, waving slowly.
        let mut tentacles = BezPath::new();
        for k in 0..7 {
            let fx = (k as f64 / 6.0 - 0.5) * 1.3;
            let mut p = c + Vec2::new(fx * s, 0.05 * s);
            tentacles.move_to(p);
            for j in 1..=10 {
                let t = j as f64 / 10.0;
                let sway = (time * 0.8 + k as f64 + t * 4.0).sin() * 0.12 * t;
                p = c + Vec2::new((fx * (1.0 - 0.3 * t) + sway) * s, (0.05 + t * 2.2) * s);
                tentacles.line_to(p);
            }
        }
        scene.stroke(&Stroke::new(0.04 * s).with_caps(Cap::Round), id, color.with_alpha(0.35), None, &tentacles);
        // The bell: a dome, flattening and swelling as it swims.
        let (rx, ry) = (s * (0.8 + 0.05 * pulse), s * (0.55 - 0.05 * pulse));
        let mut bell = BezPath::new();
        bell.move_to(c + Vec2::new(-rx, 0.0));
        bell.curve_to(c + Vec2::new(-rx, -ry * 1.3), c + Vec2::new(rx, -ry * 1.3), c + Vec2::new(rx, 0.0));
        for k in (0..6).rev() {
            let fx = (k as f64 / 6.0) * 2.0 - 1.0;
            bell.quad_to(c + Vec2::new((fx + 1.0 / 6.0) * rx, ry * 0.18), c + Vec2::new(fx * rx, 0.0));
        }
        bell.close_path();
        scene.fill(Fill::NonZero, id, color.with_alpha(0.5), None, &bell);
        glow(scene, c + Vec2::new(0.0, -ry * 0.4), s * 0.6, GLOW_WARM, 0.25);
    }
}

/// A crashed ship, nose down in the hills, overgrown with moss.
fn draw_wreck(scene: &mut Scene, cam: &Camera, x: f64, ground: f64, z: f64, time: f64, color: Color) {
    let id = Affine::IDENTITY;
    let p = |dx: f64, dy: f64| cam.point(DVec3::new(x + dx, ground + dy, z));
    let s = cam.project(DVec3::new(x, ground, z)).scale;
    if p(-20.0, 0.0).x > 4000.0 || p(20.0, 0.0).x < -4000.0 {
        return;
    }
    let hull_color = mix(Color::from_rgb8(0x8c, 0x96, 0xa8), color, 0.45);
    // A long hull tipped over, its tail up in the air.
    let mut hull = BezPath::new();
    hull.move_to(p(-11.0, -1.0));
    hull.line_to(p(-8.0, 3.2));
    hull.line_to(p(5.0, 9.0));
    hull.line_to(p(9.5, 10.5));
    hull.line_to(p(10.5, 9.0));
    hull.line_to(p(8.0, 5.0));
    hull.line_to(p(-2.0, -1.0));
    hull.close_path();
    scene.fill(Fill::NonZero, id, hull_color, None, &hull);
    // A tail fin and a broken engine ring.
    let mut fin = BezPath::new();
    fin.move_to(p(6.0, 8.5));
    fin.line_to(p(4.5, 13.0));
    fin.line_to(p(7.5, 12.5));
    fin.line_to(p(9.0, 9.8));
    fin.close_path();
    scene.fill(Fill::NonZero, id, darken(hull_color, 0.12), None, &fin);
    scene.stroke(&Stroke::new(0.5 * s), id, darken(hull_color, 0.2), None, &Ellipse::new(p(10.0, 9.7), (1.3 * s, 0.7 * s), -0.6));
    // Panel lines and a row of windows, a few still lit.
    let mut seams = BezPath::new();
    for k in 0..5 {
        let t = k as f64 / 5.0;
        seams.move_to(p(-9.0 + 14.0 * t, 1.0 + 7.0 * t - 0.2));
        seams.line_to(p(-4.0 + 14.0 * t, -1.0 + 7.0 * t + 0.4));
    }
    scene.stroke(&Stroke::new(0.08 * s), id, darken(hull_color, 0.25), None, &seams);
    for k in 0..6 {
        let t = k as f64 / 6.0;
        let lit = hash(k + x as i64, 71) < 0.4;
        let flicker = if lit { 0.6 + 0.4 * ((time * 3.0 + k as f64).sin() > -0.6) as i32 as f64 } else { 0.0 };
        let wp = p(-7.0 + 13.0 * t, 2.2 + 6.5 * t);
        let win = Rect::from_center_size(wp, (0.5 * s, 0.35 * s));
        scene.fill(Fill::NonZero, id, mix(darken(hull_color, 0.4), GLOW_COOL, flicker), None, &win);
        if lit {
            glow(scene, wp, 1.2 * s, GLOW_COOL, 0.3 * flicker as f32);
        }
    }
    // Moss draped over its back.
    let mut moss = BezPath::new();
    moss.move_to(p(-8.3, 2.6));
    for k in 0..=10 {
        let t = k as f64 / 10.0;
        let drip = 0.4 + 0.5 * hash(k, 72 + x as u64);
        moss.line_to(p(-8.0 + 13.0 * t, 3.2 + 5.8 * t + 0.3));
        moss.line_to(p(-7.4 + 13.0 * t, 3.2 + 5.8 * t - drip));
    }
    moss.line_to(p(5.0, 9.0));
    moss.close_path();
    scene.fill(Fill::NonZero, id, mix(MOSS, color, 0.4), None, &moss);
}

/// The ribs of some vast dead creature arching out of a hill.
fn draw_ribs(scene: &mut Scene, cam: &Camera, x: f64, ground: f64, z: f64, color: Color) {
    let bone = mix(Color::from_rgb8(0xf2, 0xe8, 0xd8), color, 0.35);
    let s = cam.project(DVec3::new(x, ground, z)).scale;
    let mut ribs = BezPath::new();
    let mut spine = Vec::new();
    for k in 0..7 {
        let t = k as f64 / 6.0;
        let rx = x - 10.0 + t * 20.0;
        let height = 9.0 * (1.0 - (t - 0.35).powi(2) * 1.6);
        let arc: Vec<Point> = (0..=10)
            .map(|j| {
                let a = j as f64 / 10.0 * PI;
                cam.point(DVec3::new(rx - 3.0 * a.cos() * 0.5 + 1.5, ground - 0.8 + height * a.sin(), z))
            })
            .collect();
        spine.push(arc[5]);
        ribs.extend(taper(&arc, 0.9 * s, 0.9 * s).iter());
    }
    ribs.extend(taper(&spine, 1.1 * s, 0.6 * s).iter());
    scene.fill(Fill::NonZero, Affine::IDENTITY, bone, None, &ribs);
}

// ---------------------------------------------------------------------------
// The world

pub fn draw_world(canvas: &mut Canvas3d, level: &Level, view: &WorldView, grass: &mut GrassFrame) {
    let cam = canvas.camera;
    let (vx0, vx1) = cam.visible_x(Z0, view.screen_width);
    // Hill tops recede to Z1, where the view is wider than at the path: cut
    // them to the path's view and their far edge ends short of the screen's.
    let far = cam.visible_x(Z1, view.screen_width);
    let detail = crate::paint::detail();
    draw_mist(canvas, view.screen_width);
    for hill in &level.hills {
        if hill.x1() < far.0 - 2.0 || hill.x0() > far.1 + 2.0 {
            continue;
        }
        draw_hill(canvas, hill, far, view);
        draw_moss(canvas, hill, far, view, grass);
        draw_flora(canvas, level, hill, far, view.time, detail);
    }
    for b in &level.blocks {
        if b.x1 < vx0 - 2.0 || b.x0 > vx1 + 2.0 {
            continue;
        }
        if b.kind == BlockKind::Log {
            draw_shelf(canvas, b);
        } else {
            draw_pillar(canvas, b);
        }
    }
    for l in &level.loops {
        if l.center.x + l.radius > vx0 - 2.0 && l.center.x - l.radius < vx1 + 2.0 {
            draw_loop(canvas, l, view.time);
        }
    }
    for (i, &f) in view.flies.iter().enumerate() {
        if !view.caught[i] {
            draw_spore(canvas, f, view.time, i);
        }
    }
    for (i, &c) in level.checkpoints.iter().enumerate().skip(1) {
        draw_bud(canvas, c, i <= view.checkpoint, view.time);
    }
    draw_gate(canvas, level.goal, view.time);
    if detail < 1 {
        draw_drift(canvas, view.screen_width, view.time);
    }
    if detail < 2 {
        draw_foreground(canvas, view.screen_width, view.time);
    }
}

/// Haze filling the deep valleys and pits, behind everything in the world.
fn draw_mist(canvas: &mut Canvas3d, w: f64) {
    let cam = canvas.camera;
    let top = cam.point(DVec3::new(0.0, -8.0, 4.0)).y;
    let bottom = cam.point(DVec3::new(0.0, -16.0, 4.0)).y;
    let h = cam.center.y * 2.0;
    if top > h {
        return;
    }
    canvas.push(1000.0, move |scene| {
        let mist = Gradient::new_linear((0.0, top), (0.0, bottom)).with_stops([
            (0.0, HAZE.with_alpha(0.0)),
            (0.5, HAZE.with_alpha(0.55)),
            (1.0, mix(HAZE, LILAC, 0.4).with_alpha(0.9)),
        ]);
        scene.fill(Fill::NonZero, Affine::IDENTITY, &mist, None, &Rect::new(0.0, top, w, h.max(bottom)));
    });
}

/// The x positions to sample a hill at between `x0` and `x1`: fixed points
/// in the world, plus the ends.
fn samples(x0: f64, x1: f64, step: f64) -> Vec<f64> {
    let mut xs = vec![x0];
    let mut i = (x0 / step).floor() as i64 + 1;
    while (i as f64) * step < x1 {
        xs.push(i as f64 * step);
        i += 1;
    }
    xs.push(x1);
    xs
}

/// A hill: the mossy top, receding into the distance, and the front face
/// cut through its strata, with glowing specks. The top is sorted at the
/// back so plants can stand on it; the front just behind Konrad.
fn draw_hill(canvas: &mut Canvas3d, hill: &Hill, visible: (f64, f64), view: &WorldView) {
    let cam = canvas.camera;
    let (x0, x1) = (hill.x0().max(visible.0 - 1.0), hill.x1().min(visible.1 + 1.0));
    if x1 <= x0 {
        return;
    }
    let xs = samples(x0, x1, 0.4);
    let bottom = cam.eye.y - 25.0;
    let p = |x: f64, y: f64, z: f64| cam.point(DVec3::new(x, y, z));
    let profile: Vec<(f64, f64)> = xs.iter().map(|&x| (x, hill.height(x))).collect();

    // The top: between the front and back edges.
    let mut top = BezPath::new();
    for (i, &(x, y)) in profile.iter().enumerate() {
        if i == 0 { top.move_to(p(x, y, Z0)) } else { top.line_to(p(x, y, Z0)) }
    }
    for &(x, y) in profile.iter().rev() {
        top.line_to(p(x, y, Z1));
    }
    top.close_path();
    let mid_y = profile.iter().map(|q| q.1).sum::<f64>() / profile.len() as f64;
    let (front_y, back_y) = (p(0.0, mid_y, Z0).y, p(0.0, mid_y, Z1).y);
    let top_fill = Gradient::new_linear((0.0, front_y), (0.0, back_y)).with_stops([MOSS, mix(MOSS, HAZE, 0.35)]);
    // Glowing specks in the moss.
    let specks: Vec<(Point, f64, Color)> = if crate::paint::detail() < 1 {
        let step = 0.7;
        (((x0 / step).floor() as i64)..=((x1 / step).ceil() as i64))
            .filter(|&i| hash(i, 301) < 0.55)
            .filter_map(|i| {
                let x = (i as f64 + hash(i, 302)) * step;
                if !hill.contains(x) {
                    return None;
                }
                let z = Z0 + 0.2 + (Z1 - Z0 - 0.4) * hash(i, 303);
                let pr = cam.project(DVec3::new(x, hill.height(x) + 0.02, z));
                let pulse = 0.5 + 0.5 * (view.time * (1.0 + hash(i, 304)) + i as f64).sin();
                let color = if hash(i, 305) < 0.6 { GLOW_COOL } else { GLOW_PINK };
                Some((pr.pos, pr.scale * 0.035 * (0.5 + pulse), color))
            })
            .collect()
    } else {
        Vec::new()
    };
    let depth = canvas.depth_of(DVec3::new(0.0, 0.0, Z1));
    let see_top = cam.eye.y > hill.pts.iter().map(|q| q.y).fold(f64::MAX, f64::min);
    if see_top {
        canvas.push(depth, move |scene| {
            scene.fill(Fill::NonZero, Affine::IDENTITY, &top_fill, None, &top);
            for (c, r, color) in &specks {
                glow(scene, *c, r * 4.0, *color, 0.5);
                scene.fill(Fill::NonZero, Affine::IDENTITY, lighten(*color, 0.4), None, &Circle::new(*c, *r));
            }
        });
    }

    // The front face in bands: a mossy lip, then the strata, each edge
    // wavering with noise.
    let seed = (hill.x0() * 7.0) as u64;
    let band = |depth: f64, wobble: f64| -> Vec<Point> {
        profile
            .iter()
            .map(|&(x, y)| p(x, y - depth - wobble * (0.5 + 0.5 * noise1(x * 0.35, seed + depth as u64)), Z0))
            .collect()
    };
    let fill_below = |edge: &[Point]| {
        let mut path = BezPath::new();
        path.move_to(edge[0]);
        for q in &edge[1..] {
            path.line_to(*q);
        }
        path.line_to(p(x1, bottom, Z0));
        path.line_to(p(x0, bottom, Z0));
        path.close_path();
        path
    };
    let mut faces: Vec<(BezPath, Color)> = vec![(fill_below(&band(0.0, 0.0)), MOSS_DEEP)];
    for (k, (d, color)) in [(0.45, STRATA[0]), (1.4, STRATA[1]), (2.8, STRATA[2]), (4.6, STRATA[3])].into_iter().enumerate() {
        faces.push((fill_below(&band(d, 0.35 + 0.25 * k as f64)), color));
    }
    // Cliffs at the ends, seen from the side.
    let side = |x: f64| {
        let y = hill.height(x);
        let mut path = BezPath::new();
        path.move_to(p(x, y, Z0));
        path.line_to(p(x, y, Z1));
        path.line_to(p(x, bottom, Z1));
        path.line_to(p(x, bottom, Z0));
        path.close_path();
        path
    };
    let mut sides = Vec::new();
    if cam.eye.x < hill.x0() && x0 == hill.x0() {
        sides.push(side(hill.x0()));
    }
    if cam.eye.x > hill.x1() && x1 == hill.x1() {
        sides.push(side(hill.x1()));
    }
    let mut lip = BezPath::new();
    for (i, q) in band(0.0, 0.0).iter().enumerate() {
        if i == 0 { lip.move_to(*q) } else { lip.line_to(*q) }
    }
    let s = cam.project(DVec3::new(x0, 0.0, Z0)).scale;
    // Glowing veins in the deeper strata.
    let veins: Vec<(Point, f64)> = {
        let step = 1.3;
        (((x0 / step).floor() as i64)..=((x1 / step).ceil() as i64))
            .filter(|&i| hash(i, 311) < 0.5)
            .filter_map(|i| {
                let x = (i as f64 + hash(i, 312)) * step;
                hill.contains(x).then(|| (p(x, hill.height(x) - 1.6 - 2.5 * hash(i, 313), Z0), s * (0.04 + 0.05 * hash(i, 314))))
            })
            .collect()
    };
    let pulse = 0.6 + 0.4 * (view.time * 1.3).sin();
    let depth = canvas.depth_of(DVec3::new(0.0, 0.0, 0.1));
    canvas.push(depth, move |scene| {
        let id = Affine::IDENTITY;
        for path in &sides {
            scene.fill(Fill::NonZero, id, darken(STRATA[2], 0.3), None, path);
        }
        for (path, color) in &faces {
            scene.fill(Fill::NonZero, id, *color, None, path);
        }
        for (c, r) in &veins {
            glow(scene, *c, r * 5.0, GLOW_PINK, 0.35 * pulse as f32);
            scene.fill(Fill::NonZero, id, lighten(GLOW_PINK, 0.3), None, &Circle::new(*c, *r));
        }
        let stroke = Stroke::new(0.07 * s).with_join(Join::Round).with_caps(Cap::Round);
        scene.stroke(&stroke, id, MOSS_LIGHT.with_alpha(0.9), None, &lip);
    });
}

/// Moss tendrils along a hill's top, drawn by the grass shader in short
/// chunks (a long patch over a steep hill would waste the atlas).
fn draw_moss(canvas: &mut Canvas3d, hill: &Hill, visible: (f64, f64), view: &WorldView, grass: &mut GrassFrame) {
    let cam = canvas.camera;
    let time = view.time;
    let (x0, x1) = (hill.x0().max(visible.0 - 1.0), hill.x1().min(visible.1 + 1.0));
    if x1 <= x0 {
        return;
    }
    let on_top = view.grounded && hill.contains(view.player.x) && (view.player.y - hill.height(view.player.x)).abs() < 0.3;
    let rgb = |c: Color| [c.components[0], c.components[1], c.components[2]];
    let root = Color::from_rgb8(0x2e, 0x74, 0x6c);
    let tints = [MOSS_LIGHT, Color::from_rgb8(0xff, 0xb4, 0xcc), Color::from_rgb8(0xf8, 0xe8, 0x98), Color::from_rgb8(0x9c, 0xf0, 0xe0)];
    let chunk = 5.0;
    let see_top = cam.eye.y > hill.pts.iter().map(|q| q.y).fold(f64::MAX, f64::min);
    for (row, dz) in [(0u64, 0.08), (1, 0.7), (2, 1.6), (3, 2.5)] {
        if row > 0 && !see_top {
            continue;
        }
        let z = Z0 + dz;
        let spacing = if row == 0 { 0.06 } else { 0.09 };
        let fog = 0.08 * row as f32;
        let mut c0 = (x0 / chunk).floor() * chunk;
        while c0 < x1 {
            let (a, b) = (c0.max(x0), (c0 + chunk).min(x1));
            c0 += chunk;
            let mut blades = Vec::new();
            for i in (a / spacing).floor() as i64..=(b / spacing).ceil() as i64 {
                let seed = row * 1000 + 500;
                let x = (i as f64 + hash(i, seed)) * spacing;
                if x < a || x >= b || !hill.contains(x) {
                    continue;
                }
                let y = hill.height(x);
                let clump = 0.5 + 0.35 * (x * 0.9 + row as f64).sin() + 0.25 * (x * 2.9).sin();
                let len = 0.34 * clump.max(0.2) * (0.6 + 0.7 * hash(i, seed + 1));
                let gust = ((time * 1.3 + x * 0.6).sin() + 0.3 * (time * 2.7 + x * 1.9).sin()) * 0.12;
                let mut lean = (hash(i, seed + 2) - 0.5) * 0.8 * len + gust * len;
                let away = x - view.player.x;
                if on_top && away.abs() < 0.7 {
                    lean += away.signum() * (1.0 - away.abs() / 0.7) * len * 0.9;
                }
                let zz = (z + (hash(i, seed + 6) - 0.5) * 0.5).clamp(Z0 + 0.02, Z1);
                let base = DVec3::new(x, y - 0.03, zz);
                // Tendrils curl over at the tip.
                let tip = DVec3::new(x + lean * 1.2, y + len * 0.85, zz);
                let mid = DVec3::new(x + lean * 0.2, y + len * 0.7, zz);
                let tint = tints[(hash(i, seed + 3) * 4.0) as usize % 4];
                let tip_color = if hash(i, seed + 7) < 0.55 { MOSS } else { tint };
                let scale = cam.project(base).scale;
                blades.push(BladeShape {
                    base: cam.point(base),
                    mid: cam.point(mid),
                    tip: cam.point(tip),
                    width: 0.04 * scale * (0.7 + 0.6 * hash(i, seed + 4)),
                    root_color: rgb(darken(mix(root, HAZE, fog as f64 * 0.8), 0.0)),
                    tip_color: rgb(mix(tip_color, HAZE, fog as f64)),
                    seed: hash(i, seed + 5) as f32,
                });
            }
            if blades.is_empty() {
                continue;
            }
            let Some(patch) = grass.patch(&blades) else { continue };
            let mx = (a + b) / 2.0;
            let depth = if row == 0 {
                canvas.depth_of(DVec3::new(mx, 0.0, z))
            } else {
                // Just in front of the hill's top, behind the plants on it.
                canvas.depth_of(DVec3::new(mx, 0.0, Z1)) - 0.01 - 0.001 * (3 - row) as f64
            };
            canvas.push(depth, move |scene| patch.draw(scene));
        }
    }
}

/// Alien plants standing on a hill, most behind the path, a few small ones
/// in front of it: glowing bulb stalks, ribbed fans, pod clusters, tube
/// worms and fiddleheads.
fn draw_flora(canvas: &mut Canvas3d, level: &Level, hill: &Hill, visible: (f64, f64), time: f64, detail: u8) {
    let spacing = 2.4;
    let (x0, x1) = (hill.x0().max(visible.0 - 3.0), hill.x1().min(visible.1 + 3.0));
    for i in (x0 / spacing).floor() as i64..=(x1 / spacing).ceil() as i64 {
        if hash(i, 401) < 0.3 || (detail >= 2 && i % 2 == 0) {
            continue;
        }
        let x = (i as f64 + hash(i, 402)) * spacing;
        if x < hill.x0() + 0.6 || x > hill.x1() - 0.6 {
            continue;
        }
        // Keep the loops clear.
        let front = hash(i, 403) < 0.18;
        if front && level.loops.iter().any(|l| (x - l.center.x).abs() < l.radius + 1.0) {
            continue;
        }
        let z = if front { Z0 + 0.3 } else { 1.0 + 1.8 * hash(i, 404) };
        let base = DVec3::new(x, hill.height(x) - 0.05, z);
        let scale = if front { 0.5 } else { 0.7 + 0.7 * hash(i, 405) };
        let kind = (hash(i, 406) * 5.0) as u32;
        let sway = (time * 0.9 + i as f64 * 0.7).sin() * 0.15 + (time * 2.1 + i as f64).sin() * 0.04;
        match kind {
            0 => bulb_stalk(canvas, base, scale, sway, time, i),
            1 => fan(canvas, base, scale, sway, i),
            2 => pods(canvas, base, scale, time, i),
            3 => tubes(canvas, base, scale, time, i),
            _ => fiddlehead(canvas, base, scale, sway, i),
        }
    }
}

/// A tall bending stalk with a glowing, see-through bulb at its tip.
fn bulb_stalk(canvas: &mut Canvas3d, base: DVec3, scale: f64, sway: f64, time: f64, i: i64) {
    let cam = canvas.camera;
    let height = (2.0 + 2.2 * hash(i, 411)) * scale;
    let stem = stalk_points(&cam, base, height, sway * height * 0.3 + (hash(i, 412) - 0.5) * height * 0.3);
    let s = cam.project(base).scale;
    let top = *stem.last().unwrap();
    let hue = if hash(i, 413) < 0.5 { GLOW_WARM } else { GLOW_PINK };
    let pulse = 0.65 + 0.35 * (time * 1.7 + i as f64).sin();
    let r = (0.24 + 0.14 * hash(i, 414)) * scale * s;
    let stem_path = taper(&stem, 0.12 * scale * s, 0.05 * scale * s);
    let stem_color = mix(Color::from_rgb8(0x5a, 0x9c, 0x8a), HAZE, 0.1);
    canvas.push(cam.project(base).depth, move |scene| {
        let id = Affine::IDENTITY;
        scene.fill(Fill::NonZero, id, stem_color, None, &stem_path);
        glow(scene, top, r * 4.0, hue, (0.45 * pulse) as f32);
        let bulb = Ellipse::new(top - Vec2::new(0.0, r * 0.4), (r, r * 1.25), 0.0);
        let g = Gradient::new_radial(top - Vec2::new(r * 0.3, r * 0.8), (r * 1.6) as f32).with_stops([
            (0.0, Color::WHITE.with_alpha(0.95)),
            (0.35, lighten(hue, 0.3).with_alpha(0.9)),
            (1.0, hue.with_alpha(0.75)),
        ]);
        scene.fill(Fill::NonZero, id, &g, None, &bulb);
    });
}

/// A ribbed fan, like a sea fan or a gill, opening from the ground.
fn fan(canvas: &mut Canvas3d, base: DVec3, scale: f64, sway: f64, i: i64) {
    let cam = canvas.camera;
    let r = (0.9 + 0.8 * hash(i, 421)) * scale;
    let ribs = 9;
    let tilt = sway * 0.6;
    let mut shape = BezPath::new();
    let mut lines = BezPath::new();
    let root = cam.point(base);
    shape.move_to(root);
    for k in 0..=ribs * 2 {
        let a = PI * (0.12 + 0.76 * k as f64 / (ribs * 2) as f64) + tilt;
        let rr = r * if k % 2 == 0 { 1.0 } else { 0.9 };
        let q = cam.point(base + DVec3::new(-a.cos() * rr, a.sin() * rr, 0.0));
        shape.line_to(q);
        if k % 2 == 0 {
            lines.move_to(root);
            lines.line_to(q);
        }
    }
    shape.close_path();
    let s = cam.project(base).scale;
    let color = mix(CORAL, LILAC, hash(i, 422) * 0.6);
    canvas.push(cam.project(base).depth, move |scene| {
        let id = Affine::IDENTITY;
        let g = Gradient::new_radial(root, (r * s) as f32).with_stops([darken(color, 0.25), color, lighten(color, 0.35)]);
        scene.fill(Fill::NonZero, id, &g, None, &shape);
        scene.stroke(&Stroke::new(0.035 * s), id, darken(color, 0.3).with_alpha(0.7), None, &lines);
        scene.stroke(&Stroke::new(0.03 * s).with_join(Join::Round), id, lighten(color, 0.5), None, &shape);
    });
}

/// A cluster of speckled, bulging pods.
fn pods(canvas: &mut Canvas3d, base: DVec3, scale: f64, time: f64, i: i64) {
    let cam = canvas.camera;
    let s = cam.project(base).scale;
    let n = 2 + (hash(i, 431) * 3.0) as usize;
    let mut balls = Vec::new();
    for k in 0..n {
        let r = (0.2 + 0.25 * hash(i * 7 + k as i64, 432)) * scale;
        let dx = (k as f64 - (n as f64 - 1.0) / 2.0) * r * 1.2;
        let breathe = 1.0 + 0.04 * (time * 1.8 + k as f64 + i as f64).sin();
        let c = cam.point(base + DVec3::new(dx, r * 0.9, 0.0));
        balls.push((c, r * s * breathe));
    }
    balls.sort_by(|a, b| a.1.total_cmp(&b.1));
    let color = mix(LILAC, Color::from_rgb8(0xf0, 0xc0, 0xd8), hash(i, 433));
    canvas.push(cam.project(base).depth, move |scene| {
        let id = Affine::IDENTITY;
        for (k, (c, r)) in balls.iter().enumerate() {
            let hot = *c + Vec2::new(-0.35, -0.45) * *r;
            let g = Gradient::new_radial(hot, (*r * 1.5) as f32).with_stops([lighten(color, 0.3), color, darken(color, 0.35)]);
            scene.fill(Fill::NonZero, id, &g, None, &Circle::new(*c, *r));
            for j in 0..5 {
                let a = j as f64 * 2.4 + k as f64;
                let d = 0.55 * (0.3 + 0.7 * ((j * 37 + k * 11) % 10) as f64 / 10.0);
                scene.fill(Fill::NonZero, id, darken(color, 0.35).with_alpha(0.7), None, &Circle::new(*c + Vec2::new(a.cos(), a.sin()) * *r * d, *r * 0.08));
            }
        }
    });
}

/// Tube worms: a bundle of shell tubes with glowing mouths.
fn tubes(canvas: &mut Canvas3d, base: DVec3, scale: f64, time: f64, i: i64) {
    let cam = canvas.camera;
    let s = cam.project(base).scale;
    let n = 3 + (hash(i, 441) * 3.0) as usize;
    let mut parts = Vec::new();
    for k in 0..n {
        let dx = (k as f64 - (n as f64 - 1.0) / 2.0) * 0.22 * scale;
        let height = (0.6 + 1.4 * hash(i * 5 + k as i64, 442)) * scale;
        let lean = (hash(i * 5 + k as i64, 443) - 0.5) * 0.5 * scale;
        let pts = stalk_points(&cam, base + DVec3::new(dx, 0.0, 0.0), height, lean);
        let mouth = *pts.last().unwrap();
        parts.push((taper(&pts, 0.2 * scale * s, 0.16 * scale * s), mouth, (time * 2.0 + k as f64 + i as f64).sin()));
    }
    let shell = Color::from_rgb8(0xd4, 0xbe, 0xb8);
    canvas.push(cam.project(base).depth, move |scene| {
        let id = Affine::IDENTITY;
        for (tube, mouth, pulse) in &parts {
            scene.fill(Fill::NonZero, id, shell, None, tube);
            scene.stroke(&Stroke::new(0.02 * s), id, darken(shell, 0.25), None, tube);
            let r = 0.08 * scale * s;
            glow(scene, *mouth, r * 4.0, GLOW_COOL, (0.4 + 0.2 * pulse) as f32);
            scene.fill(Fill::NonZero, id, lighten(GLOW_COOL, 0.3), None, &Ellipse::new(*mouth, (r, r * 0.5), 0.0));
        }
    });
}

/// A fiddlehead: a stem that curls into a tight spiral.
fn fiddlehead(canvas: &mut Canvas3d, base: DVec3, scale: f64, sway: f64, i: i64) {
    let cam = canvas.camera;
    let s = cam.project(base).scale;
    let height = (1.2 + 1.3 * hash(i, 451)) * scale;
    let mut pts = stalk_points(&cam, base, height, sway * height * 0.4);
    let top = *pts.last().unwrap();
    let r0 = 0.35 * scale * s;
    let dir = if hash(i, 452) < 0.5 { 1.0 } else { -1.0 };
    for k in 1..=24 {
        let t = k as f64 / 24.0;
        let a = PI + dir * t * TAU * 1.3;
        let r = r0 * (1.0 - t * 0.85);
        pts.push(top + Vec2::new(r0 * dir + a.cos() * r * dir, -a.sin().abs() * 0.0 + a.sin() * r));
    }
    let color = mix(Color::from_rgb8(0x8a, 0xd8, 0x9a), Color::from_rgb8(0xe8, 0xf0, 0x90), hash(i, 453));
    let path = taper(&pts, 0.12 * scale * s, 0.03 * scale * s);
    canvas.push(cam.project(base).depth, move |scene| {
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &path);
    });
}

/// A fungal shelf: a bracket fungus to stand on, cream on top with gills
/// underneath.
fn draw_shelf(canvas: &mut Canvas3d, b: &Block) {
    let cam = canvas.camera;
    let cx = (b.x0 + b.x1) / 2.0;
    let half = (b.x1 - b.x0) / 2.0;
    let p = |x: f64, y: f64, z: f64| cam.point(DVec3::new(x, y, z));
    let n = 20;
    let mut top = BezPath::new();
    for k in 0..=n {
        let a = PI * k as f64 / n as f64;
        let q = p(cx - a.cos() * half, b.y1, b.z0 + (b.z1 - b.z0) * (1.0 - a.sin()) * 0.5);
        if k == 0 { top.move_to(q) } else { top.line_to(q) }
    }
    for k in 0..=n {
        let a = PI * (1.0 - k as f64 / n as f64);
        top.line_to(p(cx - a.cos() * half, b.y1, b.z1));
    }
    top.close_path();
    let mut front = BezPath::new();
    for k in 0..=n {
        let a = PI * k as f64 / n as f64;
        let q = p(cx - a.cos() * half, b.y1, b.z0 + (b.z1 - b.z0) * (1.0 - a.sin()) * 0.5);
        if k == 0 { front.move_to(q) } else { front.line_to(q) }
    }
    let mut gills = BezPath::new();
    for k in (0..=n).rev() {
        let a = PI * k as f64 / n as f64;
        let droop = b.y1 - b.y0 * 0.0 - (b.y1 - b.y0) * (0.4 + 0.6 * a.sin());
        let q = p(cx - a.cos() * half * 0.9, droop, b.z0 + (b.z1 - b.z0) * (1.0 - a.sin()) * 0.5);
        front.line_to(q);
        gills.move_to(q);
        gills.line_to(p(cx - a.cos() * half * 0.95, b.y1 - 0.05, b.z0 + (b.z1 - b.z0) * (1.0 - a.sin()) * 0.5));
    }
    front.close_path();
    let s = cam.project(DVec3::new(cx, b.y1, b.z0)).scale;
    let depth = canvas.depth_of(DVec3::new(cx, b.y1, 0.2));
    canvas.push(depth, move |scene| {
        let id = Affine::IDENTITY;
        scene.fill(Fill::NonZero, id, Color::from_rgb8(0xe6, 0xc8, 0xa8), None, &top);
        scene.fill(Fill::NonZero, id, Color::from_rgb8(0xe2, 0x9a, 0x74), None, &front);
        scene.stroke(&Stroke::new(0.03 * s), id, Color::from_rgb8(0xa8, 0x5c, 0x5a), None, &gills);
        scene.stroke(&Stroke::new(0.05 * s).with_join(Join::Round), id, Color::from_rgb8(0xf4, 0xe0, 0xc8), None, &top);
    });
}

/// A pillar of layered rock closing the valley at either end.
fn draw_pillar(canvas: &mut Canvas3d, b: &Block) {
    let cam = canvas.camera;
    let y0 = b.y0.max(cam.eye.y - 25.0);
    let p = |x: f64, y: f64| cam.point(DVec3::new(x, y, b.z0));
    let mut path = BezPath::new();
    let n = 24;
    path.move_to(p(b.x0, y0));
    for k in 1..=n {
        let y = y0 + (b.y1 + 6.0 - y0) * k as f64 / n as f64;
        path.line_to(p(b.x0 + 0.6 * noise1(y * 0.4, 81), y));
    }
    for k in (0..=n).rev() {
        let y = y0 + (b.y1 + 6.0 - y0) * k as f64 / n as f64;
        path.line_to(p(b.x1 + 0.6 * noise1(y * 0.4, 82), y));
    }
    path.close_path();
    let depth = canvas.depth_of(DVec3::new(b.x0, b.y1, 0.1));
    let (top, bottom) = (p(0.0, b.y1 + 6.0).y, p(0.0, y0).y);
    canvas.push(depth, move |scene| {
        let g = Gradient::new_linear((0.0, top), (0.0, bottom)).with_stops([STRATA[0], STRATA[1], STRATA[3]]);
        scene.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &path);
    });
}

/// A loop grown from pearly shell: the far rim and the inside of the track
/// behind Konrad, with a glowing seam and chevrons racing round it; the near
/// rim, ribbed, in front of him; roots clutching the ground.
fn draw_loop(canvas: &mut Canvas3d, l: &Loop, time: f64) {
    let cam = canvas.camera;
    let (c, r, t) = (l.center, l.radius, 0.55);
    let (zb, zf) = (0.9, -0.9);
    let mut back = BezPath::new();
    circle3(&cam, c, r + t, zb, &mut back);
    circle3(&cam, c, r, zb, &mut back);
    let mut track = BezPath::new();
    circle3(&cam, c, r, zb, &mut track);
    circle3(&cam, c, r, zf, &mut track);
    let mut seam = BezPath::new();
    circle3(&cam, c, r, 0.0, &mut seam);
    let s = cam.project(DVec3::new(c.x, c.y, 0.0)).scale;
    // Chevrons running round the track, anticlockwise.
    let mut chevrons = BezPath::new();
    let n = 14;
    for k in 0..n {
        let a = (k as f64 + (time * 1.2).fract()) / n as f64 * TAU;
        let (dir, out) = (DVec2::new(-a.sin(), a.cos()), DVec2::new(a.cos(), a.sin()));
        let at = c + out * (r - 0.01);
        let q = |d: f64, side: f64| cam.point((at + dir * d).extend(side));
        chevrons.move_to(q(-0.15, 0.35));
        chevrons.line_to(q(0.1, 0.0));
        chevrons.line_to(q(-0.15, -0.35));
    }
    let glow_c = cam.point(DVec3::new(c.x, c.y, 0.0));
    let pulse = 0.7 + 0.3 * (time * 2.0).sin();
    canvas.push(canvas.depth_of(DVec3::new(c.x, c.y, zb)), move |scene| {
        let id = Affine::IDENTITY;
        scene.fill(Fill::EvenOdd, id, PEARL_SHADE, None, &back);
        let g = Gradient::new_radial(glow_c, (r * 1.2 * s) as f32).with_stops([(0.8, PEARL), (1.0, mix(PEARL, PEARL_SHADE, 0.6))]);
        scene.fill(Fill::EvenOdd, id, &g, None, &track);
        scene.stroke(&Stroke::new(0.18 * s), id, SEAM.with_alpha(0.3 * pulse as f32), None, &seam);
        scene.stroke(&Stroke::new(0.05 * s), id, lighten(SEAM, 0.4), None, &seam);
        let chev = Stroke::new(0.06 * s).with_join(Join::Round).with_caps(Cap::Round);
        scene.stroke(&chev, id, SEAM.with_alpha(0.7), None, &chevrons);
    });

    let mut front = BezPath::new();
    circle3(&cam, c, r + t, zf, &mut front);
    circle3(&cam, c, r, zf, &mut front);
    let mut ribs = BezPath::new();
    for k in 0..24 {
        let a = k as f64 / 24.0 * TAU;
        let out = DVec2::new(a.cos(), a.sin());
        ribs.move_to(cam.point((c + out * (r + 0.05)).extend(zf)));
        ribs.line_to(cam.point((c + out * (r + t - 0.05)).extend(zf)));
    }
    // Roots gripping the ground either side of the base.
    let mut roots = BezPath::new();
    let bottom = c.y - r;
    for (k, side) in [(0, -1.0), (1, 1.0), (2, -1.0), (3, 1.0)] {
        let a = -PI / 2.0 + side * (0.35 + 0.18 * k as f64);
        let from = c + DVec2::new(a.cos(), a.sin()) * (r + t * 0.5);
        let reach = 1.0 + 0.5 * k as f64;
        let pts: Vec<Point> = (0..=8)
            .map(|j| {
                let u = j as f64 / 8.0;
                let x = from.x + side * reach * u;
                let y = from.y + (bottom - 0.25 - from.y) * (u * (2.0 - u));
                cam.point(DVec3::new(x, y, zf + 0.3 * u))
            })
            .collect();
        roots.extend(taper(&pts, 0.28 * s, 0.05 * s).iter());
    }
    let hot = cam.point((c + DVec2::new(-0.5, 0.8) * r).extend(zf));
    canvas.push(canvas.depth_of(DVec3::new(c.x, c.y, zf)), move |scene| {
        let id = Affine::IDENTITY;
        scene.fill(Fill::NonZero, id, mix(PEARL, PEARL_SHADE, 0.3), None, &roots);
        let g = Gradient::new_radial(hot, (r * 2.4 * s) as f32).with_stops([PEARL, mix(PEARL, PEARL_SHADE, 0.55)]);
        scene.fill(Fill::EvenOdd, id, &g, None, &front);
        scene.stroke(&Stroke::new(0.04 * s), id, PEARL_SHADE.with_alpha(0.8), None, &ribs);
        scene.stroke(&Stroke::new(0.04 * s), id, lighten(PEARL, 0.3).with_alpha(0.8), None, &front);
    });
}

/// A spore to catch: a glowing seed drifting under a little parachute of
/// filaments.
fn draw_spore(canvas: &mut Canvas3d, pos: DVec2, time: f64, i: usize) {
    let cam = canvas.camera;
    let pr = cam.project(DVec3::new(pos.x, pos.y, -0.05));
    let s = pr.scale;
    let pulse = 0.8 + 0.2 * (time * 4.0 + i as f64).sin();
    let hue = if i % 3 == 0 { GLOW_PINK } else { GLOW_WARM };
    let mut filaments = BezPath::new();
    for k in 0..7 {
        let a = -PI / 2.0 + (k as f64 / 6.0 - 0.5) * 2.0 + (time * 3.0 + k as f64).sin() * 0.08;
        let tip = pr.pos + Vec2::new(a.cos(), a.sin()) * 0.34 * s;
        filaments.move_to(pr.pos);
        filaments.quad_to(pr.pos + Vec2::new(a.cos() * 0.1, a.sin() * 0.25) * s, tip);
    }
    canvas.push(pr.depth, move |scene| {
        let id = Affine::IDENTITY;
        glow(scene, pr.pos, 0.55 * s * pulse, hue, 0.6);
        scene.stroke(&Stroke::new(0.015 * s).with_caps(Cap::Round), id, Color::WHITE.with_alpha(0.75), None, &filaments);
        let seed = Ellipse::new(pr.pos + Vec2::new(0.0, 0.04 * s), (0.07 * s, 0.1 * s), 0.0);
        scene.fill(Fill::NonZero, id, lighten(hue, 0.5), None, &seed);
        scene.fill(Fill::NonZero, id, Color::WHITE, None, &Circle::new(pr.pos + Vec2::new(-0.02 * s, 0.02 * s), 0.03 * s));
    });
}

/// A checkpoint: a bud on a stalk that bursts open into a glowing bloom
/// when reached.
fn draw_bud(canvas: &mut Canvas3d, at: DVec2, reached: bool, time: f64) {
    let cam = canvas.camera;
    let base = DVec3::new(at.x, at.y, 0.8);
    let sway = (time * 1.1 + at.x).sin() * 0.08;
    let stem = stalk_points(&cam, base, 2.0, sway);
    let top = *stem.last().unwrap();
    let s = cam.project(base).scale;
    let open = if reached { 1.0 } else { 0.0 };
    let color = if reached { GLOW_COOL } else { Color::from_rgb8(0xe8, 0xa8, 0x90) };
    let mut petals = BezPath::new();
    let n = 6;
    for k in 0..n {
        let spread = (k as f64 / (n - 1) as f64 - 0.5) * (0.5 + 2.2 * open);
        let a = -PI / 2.0 + spread + sway;
        let len = (0.45 + 0.2 * open) * s;
        let dir = Vec2::new(a.cos(), a.sin());
        let side = Vec2::new(-dir.y, dir.x) * 0.14 * s;
        petals.move_to(top);
        petals.quad_to(top + dir * len * 0.5 + side, top + dir * len);
        petals.quad_to(top + dir * len * 0.5 - side, top);
        petals.close_path();
    }
    let stem_path = taper(&stem, 0.1 * s, 0.06 * s);
    let pulse = 0.75 + 0.25 * (time * 2.5).sin();
    canvas.push(canvas.depth_of(base), move |scene| {
        let id = Affine::IDENTITY;
        scene.fill(Fill::NonZero, id, Color::from_rgb8(0x5a, 0x9c, 0x8a), None, &stem_path);
        if reached {
            glow(scene, top, 1.6 * s * pulse, GLOW_COOL, 0.6);
        }
        scene.fill(Fill::NonZero, id, color, None, &petals);
        scene.stroke(&Stroke::new(0.03 * s), id, lighten(color, 0.5), None, &petals);
        scene.fill(Fill::NonZero, id, if reached { Color::WHITE } else { darken(color, 0.3) }, None, &Circle::new(top, 0.1 * s));
    });
}

/// The goal: an upright ring of pearl shell with a shimmering membrane,
/// spores rising through it.
fn draw_gate(canvas: &mut Canvas3d, at: DVec2, time: f64) {
    let cam = canvas.camera;
    let c = DVec2::new(at.x, at.y + 2.4);
    let z = 0.6;
    let (rx, ry) = (1.7, 2.4);
    let ring = |grow: f64| {
        let mut path = BezPath::new();
        for k in 0..48 {
            let a = k as f64 / 48.0 * TAU;
            let wob = 1.0 + 0.03 * (a * 5.0 + time).sin();
            let q = cam.point(DVec3::new(c.x + a.cos() * (rx + grow) * wob, c.y + a.sin() * (ry + grow) * wob, z));
            if k == 0 { path.move_to(q) } else { path.line_to(q) }
        }
        path.close_path();
        path
    };
    let (outer, inner) = (ring(0.4), ring(0.0));
    let mut shell = outer.clone();
    shell.extend(inner.iter());
    let center = cam.point(c.extend(z));
    let s = cam.project(c.extend(z)).scale;
    let sparks: Vec<Point> = (0..12)
        .map(|k| {
            let t = (time * 0.35 + hash(k, 491)).fract();
            let a = time * 1.5 + k as f64 * 2.1;
            cam.point(DVec3::new(c.x + a.cos() * 1.2 * (1.0 - t), c.y - ry + t * ry * 2.0, z))
        })
        .collect();
    canvas.push(canvas.depth_of(c.extend(z)), move |scene| {
        let id = Affine::IDENTITY;
        let swirl = (time * 0.8).sin() as f32 * 0.1;
        let membrane = Gradient::new_radial(center, (ry * s) as f32).with_stops([
            (0.0, Color::WHITE.with_alpha(0.8)),
            (0.4 + swirl, GLOW_COOL.with_alpha(0.5)),
            (1.0, GLOW_PINK.with_alpha(0.35)),
        ]);
        scene.fill(Fill::NonZero, id, &membrane, None, &inner);
        for p in &sparks {
            scene.fill(Fill::NonZero, id, Color::WHITE, None, &Circle::new(*p, 0.05 * s));
        }
        glow(scene, center, 3.5 * s, GLOW_COOL, 0.3);
        scene.fill(Fill::EvenOdd, id, PEARL, None, &shell);
        scene.stroke(&Stroke::new(0.05 * s), id, PEARL_SHADE, None, &inner);
    });
}

/// Spores and seeds drifting in the air, catching the light.
fn draw_drift(canvas: &mut Canvas3d, w: f64, time: f64) {
    let cam = canvas.camera;
    let spacing = 1.2;
    let (x0, x1) = cam.visible_x(0.5, w);
    for i in (x0 / spacing).floor() as i64 - 1..=(x1 / spacing).ceil() as i64 + 1 {
        if hash(i, 501) < 0.5 {
            continue;
        }
        let t = time * (0.1 + 0.1 * hash(i, 502)) + hash(i, 503) * TAU;
        let x = (i as f64 + hash(i, 504)) * spacing + t.sin() * 1.2 + time * 0.2;
        let y = cam.eye.y - 4.0 + 9.0 * hash(i, 505) + (t * 1.3).cos() * 0.5 + (time * 0.15 * (0.5 + hash(i, 508))) % 3.0;
        let z = -1.0 + 3.5 * hash(i, 506);
        let pr = cam.project(DVec3::new(x, y, z));
        let r = (0.025 * pr.scale).max(0.8);
        let color = if hash(i, 507) < 0.5 { Color::WHITE } else { GLOW_WARM };
        canvas.push(pr.depth, move |scene| {
            scene.fill(Fill::NonZero, Affine::IDENTITY, color.with_alpha(0.7), None, &Circle::new(pr.pos, r));
        });
    }
}

/// Near the camera, out of focus: tall bulb stalks and curling fronds
/// rising from below, dark against the bright haze.
fn draw_foreground(canvas: &mut Canvas3d, w: f64, time: f64) {
    let cam = canvas.camera;
    let z = -5.0;
    let (x0, x1) = cam.visible_x(z, w);
    let spacing = 5.0;
    let dark = Color::from_rgb8(0x1e, 0x3a, 0x44);
    let base_y = cam.eye.y - 9.0;
    for i in (x0 / spacing).floor() as i64 - 1..=(x1 / spacing).ceil() as i64 + 1 {
        if hash(i, 511) < 0.45 {
            continue;
        }
        let x = (i as f64 + hash(i, 512)) * spacing;
        let sway = (time * 0.6 + i as f64).sin() * 0.3;
        let height = 4.0 + 3.0 * hash(i, 513);
        let stem = stalk_points(&cam, DVec3::new(x, base_y, z), height, sway + (hash(i, 514) - 0.5) * 2.0);
        let s = cam.project(DVec3::new(x, 0.0, z)).scale;
        let top = *stem.last().unwrap();
        let shape = taper(&stem, 0.3 * s, 0.1 * s);
        let bulb = hash(i, 515) < 0.5;
        let depth = cam.project(DVec3::new(x, 0.0, z)).depth;
        canvas.push(depth, move |scene| {
            let id = Affine::IDENTITY;
            let halo = Stroke::new(0.15 * s).with_join(Join::Round);
            scene.stroke(&halo, id, dark.with_alpha(0.3), None, &shape);
            scene.fill(Fill::NonZero, id, dark.with_alpha(0.95), None, &shape);
            if bulb {
                glow(scene, top, 1.4 * s, GLOW_PINK, 0.3);
                scene.fill(Fill::NonZero, id, mix(dark, GLOW_PINK, 0.35), None, &Ellipse::new(top, (0.35 * s, 0.45 * s), 0.0));
            }
        });
    }
}
