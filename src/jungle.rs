//! Drawing the jungle: sky, fogged background layers, platforms as 3D boxes,
//! trees, flies, checkpoints, the goal and foreground
//! foliage.

use std::f64::consts::PI;

use glam::{DVec2, DVec3};
use vello::kurbo::{Affine, BezPath, Circle, Ellipse, Point, Rect, Shape, Stroke, Vec2};
use vello::peniko::{Color, Fill, Gradient};
use vello::Scene;

use crate::canvas3d::{darken, lighten, mix, Camera, Canvas3d, OUTLINE};
use crate::paint::{self, wash};
use crate::grass::{BladeShape, GrassFrame};
use crate::level::{Block, BlockKind, Level};

const SKY_TOP: Color = Color::from_rgb8(0x16, 0x22, 0x3c);
const SKY_HORIZON: Color = Color::from_rgb8(0x5a, 0x7a, 0x8e);
/// Distance haze: a dusty peach-lilac, so far things take on the sky's colour.
const FOG: Color = Color::from_rgb8(0x3e, 0x5a, 0x6a);
const LEAF: Color = Color::from_rgb8(0x2e, 0x5e, 0x3e);
const LEAF_DARK: Color = Color::from_rgb8(0x0e, 0x20, 0x1c);
const BARK: Color = Color::from_rgb8(0x4a, 0x36, 0x2a);
/// Blossom and foliage accents mixed into some trees.
const ACCENTS: [Color; 4] = [
    Color::from_rgb8(0x1e, 0x4a, 0x44),
    Color::from_rgb8(0x2c, 0x3e, 0x5c),
    Color::from_rgb8(0x3a, 0x5e, 0x2e),
    Color::from_rgb8(0x24, 0x3a, 0x30),
];

/// Deterministic pseudo-random number in [0, 1) for an integer cell.
fn hash(i: i64, seed: u64) -> f64 {
    let mut x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ seed.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// Smooth 1D noise from a few sines.
fn wave(x: f64, seed: f64) -> f64 {
    ((x * 0.31 + seed).sin() * 0.5 + (x * 0.17 + seed * 2.3).sin() * 0.35 + (x * 0.73 + seed * 0.7).sin() * 0.15) * 0.5 + 0.5
}

struct Layer {
    z: f64,
    base: f64,
    amp: f64,
    color: Color,
    fog: f64,
    tree_spacing: f64,
    tree_size: f64,
    seed: u64,
}

const LAYERS: [Layer; 4] = [
    Layer { z: 90.0, base: -6.0, amp: 22.0, color: Color::from_rgb8(0x22, 0x34, 0x4c), fog: 0.35, tree_spacing: 0.0, tree_size: 0.0, seed: 1 },
    Layer { z: 40.0, base: -3.0, amp: 4.0, color: Color::from_rgb8(0x1a, 0x34, 0x38), fog: 0.3, tree_spacing: 6.0, tree_size: 3.2, seed: 2 },
    Layer { z: 18.0, base: -2.5, amp: 2.5, color: Color::from_rgb8(0x14, 0x2c, 0x26), fog: 0.18, tree_spacing: 4.5, tree_size: 2.4, seed: 3 },
    Layer { z: 7.0, base: -3.0, amp: 1.5, color: Color::from_rgb8(0x0e, 0x20, 0x1a), fog: 0.06, tree_spacing: 3.8, tree_size: 2.0, seed: 4 },
];

/// Sky and background layers, drawn straight into the scene (always behind).
/// Wind strength, 0 calm .. 1 gusty: sways trees, ferns and grass. Set once
/// per frame by the game from the weather.
static WIND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0x3FE0_0000_0000_0000); // 0.5

pub fn set_wind(wind: f64) {
    WIND.store(wind.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

fn wind() -> f64 {
    f64::from_bits(WIND.load(std::sync::atomic::Ordering::Relaxed))
}

/// Sky and background hills. The two far layers go into `far` and the two
/// nearer ones into `mid`; the renderer blurs them for depth of field.
/// `cam`, `w` and `h` describe the (half-resolution) layer images.
pub fn draw_background(far: &mut Scene, mid: &mut Scene, cam: &Camera, w: f64, h: f64, time: f64) {
    let scene = &mut *far;
    let sky = Gradient::new_linear((0.0, 0.0), (0.0, h)).with_stops([SKY_TOP, SKY_HORIZON, SKY_HORIZON]);
    scene.fill(Fill::NonZero, Affine::IDENTITY, &sky, None, &Rect::new(0.0, 0.0, w, h));
    let sun = Point::new(w * paint::SUN.0, h * paint::SUN.1);
    let glow = Gradient::new_radial(sun, (h * 0.45) as f32).with_stops([
        Color::from_rgb8(0xff, 0xf6, 0xd0).with_alpha(0.65),
        Color::from_rgb8(0xff, 0xf6, 0xd0).with_alpha(0.0),
    ]);
    scene.fill(Fill::NonZero, Affine::IDENTITY, &glow, None, &Rect::new(0.0, 0.0, w, h));
    // Watercolour blooms: soft blotches where the wash dried unevenly.
    for k in 0..4 {
        let fx = hash(k, 90);
        let fy = hash(k, 91);
        let drift = (cam.eye.x * 0.01 + fx * 10.0) % 1.4 - 0.2;
        let c = Point::new(w * drift, h * (0.05 + fy * 0.45));
        let r = h * (0.18 + 0.2 * hash(k, 92));
        let tint = if k % 2 == 0 { Color::from_rgb8(0x5d, 0x8f, 0xb0) } else { Color::from_rgb8(0xe8, 0xc9, 0x86) };
        let bloom = Gradient::new_radial(c, r as f32).with_stops([
            (0.0, tint.with_alpha(0.1)),
            (0.7, tint.with_alpha(0.07)),
            (1.0, tint.with_alpha(0.0)),
        ]);
        let blotch = paint::wobble(&Circle::new(c, r).to_path(1.0), r * 0.12, k as u64, c);
        scene.fill(Fill::NonZero, Affine::IDENTITY, &bloom, None, &blotch);
    }

    for (li, layer) in LAYERS.iter().enumerate() {
        let scene: &mut Scene = if li < 2 { &mut *far } else { &mut *mid };
        let color = mix(layer.color, FOG, layer.fog);
        let (x0, x1) = cam.visible_x(layer.z, w);
        let height = |x: f64| {
            let n = wave(x, layer.seed as f64 * 1.7);
            if li == 0 {
                // Mountains: broad peaks with gentle ridges (no fine detail,
                // which would flicker as the camera moves).
                layer.base + layer.amp * (n * n + 0.12 * (x * 0.3).sin().abs())
            } else {
                layer.base + layer.amp * n
            }
        };
        // Sample at fixed points in the world, so the outline stays put as the
        // camera pans and zooms instead of being resampled every frame.
        let step = if li == 0 { 1.5 } else { 0.75 };
        let first = (x0 / step).floor() as i64;
        let steps = ((x1 - x0) / step).ceil() as i64 + 1;
        let mut hills = BezPath::new();
        for i in 0..=steps {
            let x = (first + i) as f64 * step;
            let p = cam.point(DVec3::new(x, height(x), layer.z));
            if i == 0 {
                hills.move_to(p);
            } else {
                hills.line_to(p);
            }
        }
        hills.line_to((w + 100.0, h + 100.0));
        hills.line_to((-100.0, h + 100.0));
        hills.close_path();
        let anchor = cam.point(DVec3::new(0.0, layer.base, layer.z));
        paint::wash_with_edge(scene, &hills, color, None, 160.0, li as u64 + 7, anchor, 0.1 + 0.08 * li as f32);

        // At the lowest detail level only the nearest layer keeps its trees.
        if layer.tree_spacing > 0.0 && (paint::detail() < 2 || li == LAYERS.len() - 1) {
            let first = (x0 / layer.tree_spacing).floor() as i64 - 1;
            let last = (x1 / layer.tree_spacing).ceil() as i64 + 1;
            for i in first..=last {
                let x = (i as f64 + hash(i, layer.seed)) * layer.tree_spacing;
                let size = layer.tree_size * (0.7 + 0.6 * hash(i, layer.seed + 10));
                let p = cam.project(DVec3::new(x, height(x) - 0.3, layer.z));
                let tf = Affine::translate(p.pos.to_vec2()) * Affine::scale_non_uniform(p.scale * size, -p.scale * size);
                let sway = (time * 0.8 + i as f64).sin() * 0.04 * (0.4 + wind()) + wind() * 0.04;
                // The whole tree leans with the wind.
                let tf = tf * Affine::skew(sway * 0.6, 0.0);
                let seed = (i as u64).wrapping_mul(31) ^ layer.seed;
                // Some trees blossom or turn: pastel accents, hazed with distance.
                let color = if hash(i, layer.seed + 40) < 0.45 {
                    let accent = ACCENTS[(hash(i, layer.seed + 41) * 4.0) as usize % 4];
                    mix(mix(layer.color, accent, 0.55), FOG, layer.fog)
                } else {
                    color
                };
                let size_px = p.scale * size;
                if hash(i, layer.seed + 20) < 0.4 {
                    draw_palm(scene, tf, color, sway, size_px, seed);
                } else {
                    draw_tree(scene, tf, color, hash(i, layer.seed + 30), size_px, seed);
                }
            }
        }

    }
}

/// A broadleaf tree in local units (y up, base at the origin, ~1 unit tall).
fn draw_tree(scene: &mut Scene, tf: Affine, color: Color, variant: f64, size: f64, seed: u64) {
    let trunk = tf * Rect::new(-0.06, 0.0, 0.06, 0.6).to_path(0.1);
    let anchor = tf * Point::ORIGIN;
    paint::wash_with_edge(scene, &trunk, darken(color, 0.25), None, size * 0.2, seed, anchor, 0.15);
    let blobs = [(0.0, 0.75, 0.3), (-0.22, 0.62, 0.22), (0.22, 0.64, 0.24), (0.05 * variant, 0.95, 0.22)];
    for (i, (x, y, r)) in blobs.iter().enumerate() {
        let c = if i % 2 == 0 { color } else { lighten(color, 0.08) };
        let blob = tf * Circle::new((*x, *y), *r).to_path(0.01);
        paint::wash_with_edge(scene, &blob, c, None, size * r * 2.0, seed ^ i as u64, anchor, 0.2);
    }
}

/// A palm in local units (y up, base at the origin, ~1 unit tall).
fn draw_palm(scene: &mut Scene, tf: Affine, color: Color, sway: f64, size: f64, seed: u64) {
    let top = Point::new(0.15 + sway, 1.0);
    let mut trunk = BezPath::new();
    trunk.move_to((-0.04, 0.0));
    trunk.quad_to(Point::new(0.1, 0.5), top + Vec2::new(-0.025, 0.0));
    trunk.line_to(top + Vec2::new(0.025, 0.0));
    trunk.quad_to(Point::new(0.14, 0.5), Point::new(0.04, 0.0));
    trunk.close_path();
    let anchor = tf * Point::ORIGIN;
    paint::wash_with_edge(scene, &(tf * trunk), darken(color, 0.2), None, size * 0.1, seed, anchor, 0.15);
    for k in 0..7 {
        let a = PI * (0.05 + k as f64 / 6.0 * 0.9) + sway;
        let tip = top + Vec2::new(a.cos() * 0.45, a.sin() * 0.18 - 0.2);
        let mid = top.midpoint(tip) + Vec2::new(0.0, 0.14);
        let mut frond = BezPath::new();
        frond.move_to(top);
        frond.quad_to(mid + Vec2::new(0.0, 0.06), tip);
        frond.quad_to(mid - Vec2::new(0.0, 0.02), top);
        paint::wash_with_edge(scene, &(tf * frond), lighten(color, 0.04), None, size * 0.3, seed ^ k as u64, anchor, 0.2);
    }
}

/// Per-frame state the world needs for drawing.
pub struct WorldView<'a> {
    pub time: f64,
    pub flies: &'a [DVec2],
    pub caught: &'a [bool],
    pub checkpoint: usize,
    pub screen_width: f64,
    /// Konrad's feet, and whether he's on the ground: grass bends around him.
    pub player: DVec2,
    pub grounded: bool,
    /// 0 dry .. 1 pouring: the ground darkens, gets a sheen and puddles.
    pub rain: f64,
}

pub fn draw_world(canvas: &mut Canvas3d, level: &Level, view: &WorldView, grass: &mut GrassFrame) {
    let cam = canvas.camera;
    let (vx0, vx1) = cam.visible_x(-1.5, view.screen_width);
    for b in &level.blocks {
        if b.x1 < vx0 - 2.0 || b.x0 > vx1 + 2.0 {
            continue;
        }
        draw_block(canvas, b, (vx0, vx1), view, grass);
    }
    for t in &level.trees {
        draw_trunk(canvas, t.x, t.y);
    }
    for (i, &f) in view.flies.iter().enumerate() {
        if !view.caught[i] {
            draw_fly(canvas, f, view.time, i);
        }
    }
    for (i, &c) in level.checkpoints.iter().enumerate().skip(1) {
        draw_checkpoint(canvas, c, i <= view.checkpoint, view.time);
    }
    draw_goal(canvas, level.goal, view.time);
    draw_foreground(canvas, view.screen_width, view.time);
}

/// The sky as reflected in wet ground and puddles.
const SKY_REFLECTION: Color = Color::from_rgb8(0x8c, 0xa6, 0xb8);

/// Shadow colour for ambient occlusion on the platforms.
const OCCLUSION: Color = Color::from_rgb8(0x0c, 0x0a, 0x14);

/// Draws a platform. `visible` is the range of x on screen, where grass
/// tufts are drawn.
fn draw_block(canvas: &mut Canvas3d, b: &Block, visible: (f64, f64), view: &WorldView, grass: &mut GrassFrame) {
    let cam = canvas.camera;
    let (top, front, side) = match b.kind {
        BlockKind::Ground => (Color::from_rgb8(0x3e, 0x62, 0x36), Color::from_rgb8(0x3a, 0x2a, 0x22), Color::from_rgb8(0x2a, 0x1e, 0x1a)),
        BlockKind::Stone => (Color::from_rgb8(0x4a, 0x62, 0x44), Color::from_rgb8(0x4e, 0x58, 0x64), Color::from_rgb8(0x36, 0x3e, 0x4a)),
        BlockKind::Log => (Color::from_rgb8(0x6a, 0x4a, 0x34), Color::from_rgb8(0x54, 0x3a, 0x2a), Color::from_rgb8(0x9a, 0x7a, 0x56)),
    };
    // Wet ground is darker.
    let wet = view.rain.clamp(0.0, 1.0);
    let (top, front, side) = (darken(top, 0.25 * wet as f32), darken(front, 0.3 * wet as f32), darken(side, 0.3 * wet as f32));
    // Don't bother drawing far below the screen.
    let y0 = b.y0.max(cam.eye.y - 25.0);
    let p = |x: f64, y: f64, z: f64| cam.point(DVec3::new(x, y, z));
    let quad = |a: Point, b: Point, c: Point, d: Point| {
        let mut path = BezPath::new();
        path.move_to(a);
        path.line_to(b);
        path.line_to(c);
        path.line_to(d);
        path.close_path();
        path
    };
    let mut faces: Vec<(BezPath, Color)> = Vec::new();
    if cam.eye.y < y0 {
        faces.push((quad(p(b.x0, y0, b.z0), p(b.x1, y0, b.z0), p(b.x1, y0, b.z1), p(b.x0, y0, b.z1)), darken(side, 0.3)));
    }
    let side_face = |x: f64| quad(p(x, b.y1, b.z0), p(x, b.y1, b.z1), p(x, y0, b.z1), p(x, y0, b.z0));
    if cam.eye.x < b.x0 {
        faces.push((side_face(b.x0), side));
    }
    if cam.eye.x > b.x1 {
        faces.push((side_face(b.x1), side));
    }
    if cam.eye.y > b.y1 {
        faces.push((quad(p(b.x0, b.y1, b.z0), p(b.x1, b.y1, b.z0), p(b.x1, b.y1, b.z1), p(b.x0, b.y1, b.z1)), top));
    }
    let front_face = quad(p(b.x0, b.y1, b.z0), p(b.x1, b.y1, b.z0), p(b.x1, y0, b.z0), p(b.x0, y0, b.z0));
    let anchor = p(b.x0, b.y1, b.z0);

    // Details on the front face.
    let mut details: Vec<(BezPath, Color, Option<f64>)> = Vec::new();
    let seed = (b.x0 * 13.0) as i64;
    match b.kind {
        BlockKind::Ground | BlockKind::Stone => {
            for (k, depth) in [0.8, 1.9, 3.3, 5.0].iter().enumerate() {
                let y = b.y1 - depth;
                if y <= y0 {
                    break;
                }
                let mut line = BezPath::new();
                let n = ((b.x1 - b.x0) * 2.0).ceil() as usize;
                for i in 0..=n {
                    let x = b.x0 + (b.x1 - b.x0) * i as f64 / n as f64;
                    let wobble = (x * 2.3 + k as f64).sin() * 0.08;
                    let pt = p(x, y + wobble, b.z0);
                    if i == 0 { line.move_to(pt) } else { line.line_to(pt) }
                }
                details.push((line, darken(front, 0.18), Some(0.04)));
            }
            // Stones in the soil, lit from the upper right, each with a
            // shadow under it.
            let rocks = ((b.x1 - b.x0) * 0.8) as i64;
            for i in 0..rocks {
                let x = b.x0 + 0.3 + hash(seed + i, 7) * (b.x1 - b.x0 - 0.6);
                let y = b.y1 - 0.5 - hash(seed + i, 8) * 4.0;
                if y <= y0 + 0.2 {
                    continue;
                }
                let c = cam.project(DVec3::new(x, y, b.z0));
                let r = (0.1 + 0.15 * hash(seed + i, 9)) * c.scale;
                let tilt = (hash(seed + i, 10) - 0.5) * 0.6;
                details.push((Ellipse::new(c.pos + Vec2::new(-0.15, 0.3) * r, (r * 1.45, r * 1.05), tilt).to_path(0.1), darken(front, 0.35), None));
                details.push((Ellipse::new(c.pos, (r * 1.4, r), tilt).to_path(0.1), lighten(front, 0.1), None));
                details.push((Ellipse::new(c.pos + Vec2::new(0.35, -0.35) * r, (r * 0.7, r * 0.4), tilt).to_path(0.1), lighten(front, 0.3), None));
            }
            // Roots hanging out of the soil under the grass.
            if b.kind == BlockKind::Ground {
                let roots = ((b.x1 - b.x0) * 0.35) as i64;
                for i in 0..roots {
                    let x = b.x0 + 0.4 + hash(seed + i, 11) * (b.x1 - b.x0 - 0.8);
                    let len = 0.5 + 1.4 * hash(seed + i, 12);
                    let mut root = BezPath::new();
                    root.move_to(p(x, b.y1 - 0.1, b.z0));
                    let bend = (hash(seed + i, 13) - 0.5) * 0.6;
                    root.curve_to(p(x + bend, b.y1 - len * 0.35, b.z0), p(x - bend, b.y1 - len * 0.7, b.z0), p(x + bend * 0.5, b.y1 - len, b.z0));
                    details.push((root, mix(front, Color::from_rgb8(0x7a, 0x5a, 0x40), 0.6), Some(0.03 + 0.03 * hash(seed + i, 14))));
                }
            }
            // Grass or moss hanging over the front edge, in uneven clumps.
            let fringe_color = if b.kind == BlockKind::Ground { top } else { Color::from_rgb8(0x6a, 0x9a, 0x4a) };
            let mut fringe = BezPath::new();
            fringe.move_to(p(b.x0, b.y1, b.z0));
            let mut x = b.x0;
            let mut i = 0;
            while x < b.x1 {
                let step = 0.12 + 0.2 * hash(seed + i, 4);
                let clump = 0.5 + (x * 0.9 + seed as f64).sin() * 0.5;
                let len = (0.08 + 0.3 * hash(seed + i, 3)) * (0.5 + clump);
                fringe.line_to(p((x + step / 2.0).min(b.x1), b.y1 - len, b.z0));
                x += step;
                fringe.line_to(p(x.min(b.x1), b.y1, b.z0));
                i += 1;
            }
            fringe.close_path();
            details.push((fringe, fringe_color, None));
        }
        BlockKind::Log => {
            // Bark stripes along the log.
            for k in 1..4 {
                let y = b.y0 + (b.y1 - b.y0) * k as f64 / 4.0;
                let mut line = BezPath::new();
                line.move_to(p(b.x0 + 0.1, y, b.z0));
                line.line_to(p(b.x1 - 0.1, y, b.z0));
                details.push((line, darken(front, 0.2), Some(0.03)));
            }
        }
    }
    // Tree rings on a visible log end.
    if b.kind == BlockKind::Log {
        for x in [b.x0, b.x1] {
            if (x == b.x0 && cam.eye.x < b.x0) || (x == b.x1 && cam.eye.x > b.x1) {
                let c = DVec3::new(x, (b.y0 + b.y1) / 2.0, (b.z0 + b.z1) / 2.0);
                for r in [0.3, 0.6, 0.85] {
                    let e = canvas.project_ellipsoid(
                        c,
                        glam::DMat3::from_cols(
                            DVec3::new(0.0, (b.y1 - b.y0) / 2.0 * r, 0.0),
                            DVec3::new(0.0, 0.0, (b.z1 - b.z0) / 2.0 * r),
                            DVec3::new(1e-4, 0.0, 0.0),
                        ),
                    );
                    details.push((e.to_path(0.1), darken(side, 0.25), Some(0.02)));
                }
            }
        }
    }

    let scale = cam.project(DVec3::new(b.x0, b.y1, b.z0)).scale;
    let depth = block_depth(&cam, b);
    // Ambient occlusion on the front: a shadow under the grass, then darker
    // with depth and towards the corners.
    let occlusion = |a: f32| OCCLUSION.with_alpha(a);
    let fall = Gradient::new_linear(p(b.x0, b.y1, b.z0), p(b.x0, b.y1 - 6.0, b.z0)).with_stops([
        (0.0, occlusion(0.45)),
        (0.05, occlusion(0.0)),
        (0.3, occlusion(0.12)),
        (1.0, occlusion(0.55)),
    ]);
    let edge = (0.9 / (b.x1 - b.x0)).min(0.3) as f32;
    let corners = Gradient::new_linear(p(b.x0, b.y1, b.z0), p(b.x1, b.y1, b.z0)).with_stops([
        (0.0, occlusion(0.4)),
        (edge, occlusion(0.0)),
        (1.0 - edge, occlusion(0.0)),
        (1.0, occlusion(0.4)),
    ]);
    // The top: sunlit along the front edge, darker further back.
    let top_face = (cam.eye.y > b.y1).then(|| {
        let xm = (b.x0 + b.x1) / 2.0;
        // In the rain the front edge shines with the sky's reflection.
        let sheen = mix(lighten(top, 0.25), SKY_REFLECTION, wet);
        let shade = Gradient::new_linear(p(xm, b.y1, b.z0), p(xm, b.y1, b.z1)).with_stops([
            (0.0, sheen.with_alpha((0.35 + 0.2 * wet) as f32)),
            (0.25, top.with_alpha(0.0)),
            (1.0, occlusion(0.35)),
        ]);
        let mut rim = BezPath::new();
        rim.move_to(p(b.x0, b.y1, b.z0));
        rim.line_to(p(b.x1, b.y1, b.z0));
        (quad(p(b.x0, b.y1, b.z0), p(b.x1, b.y1, b.z0), p(b.x1, b.y1, b.z1), p(b.x0, b.y1, b.z1)), shade, rim, lighten(top, 0.35))
    });
    if b.kind == BlockKind::Ground && wet > 0.05 && cam.eye.y > b.y1 {
        draw_puddles(canvas, b, visible, view, depth);
    }
    if b.kind != BlockKind::Log {
        draw_grass(canvas, b, top, visible, view, grass);
    }
    let seed = paint::seed(&[b.x0, b.y1]);
    canvas.push(depth, move |scene| {
        // Platforms are solid: an opaque base under the translucent washes,
        // so nothing behind shows through.
        for (i, (face, color)) in faces.iter().enumerate() {
            scene.fill(Fill::NonZero, Affine::IDENTITY, *color, None, face);
            wash(scene, face, *color, None, scale * 2.0, seed ^ i as u64, anchor);
            paint::ink(scene, face, scale * 2.0, seed ^ i as u64, anchor);
        }
        scene.fill(Fill::NonZero, Affine::IDENTITY, front, None, &front_face);
        wash(scene, &front_face, front, None, scale * 2.0, seed ^ 0xf, anchor);
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &front_face);
        for (i, (path, color, stroke)) in details.iter().enumerate() {
            match stroke {
                Some(w) => scene.stroke(&Stroke::new(w * scale), Affine::IDENTITY, color.with_alpha(0.6), None, path),
                None => wash(scene, path, *color, None, scale * 0.3, seed ^ (i as u64 + 20), anchor),
            }
        }
        scene.fill(Fill::NonZero, Affine::IDENTITY, &fall, None, &front_face);
        scene.fill(Fill::NonZero, Affine::IDENTITY, &corners, None, &front_face);
        scene.pop_layer();
        paint::ink(scene, &front_face, scale * 2.0, seed, anchor);
        if let Some((face, shade, rim, rim_color)) = &top_face {
            scene.fill(Fill::NonZero, Affine::IDENTITY, shade, None, face);
            scene.stroke(&Stroke::new(0.04 * scale), Affine::IDENTITY, rim_color.with_alpha(0.5), None, rim);
        }
    });
}

/// Where a platform is sorted among the things drawn in front of the camera.
pub fn block_depth(cam: &Camera, b: &Block) -> f64 {
    // Boxes further away in the xy plane are drawn first.
    let dx = (b.x0 - cam.eye.x).max(cam.eye.x - b.x1).max(0.0);
    let dy = (b.y0 - cam.eye.y).max(cam.eye.y - b.y1).max(0.0);
    (b.z0 + b.z1) / 2.0 - cam.eye.z + (dx + dy) * 0.001
}

/// The top face of a platform on screen.
pub fn top_face(cam: &Camera, b: &Block) -> BezPath {
    let p = |x: f64, z: f64| cam.point(DVec3::new(x, b.y1, z));
    let mut path = BezPath::new();
    path.move_to(p(b.x0, b.z0));
    path.line_to(p(b.x1, b.z0));
    path.line_to(p(b.x1, b.z1));
    path.line_to(p(b.x0, b.z1));
    path.close_path();
    path
}

/// Puddles on top of a platform in the rain: they grow as it keeps
/// raining, reflect the sky and ripple where drops land. `depth` is the
/// platform's, so they're drawn right on top of it.
fn draw_puddles(canvas: &mut Canvas3d, b: &Block, visible: (f64, f64), view: &WorldView, depth: f64) {
    let cam = canvas.camera;
    let grow = (view.rain * 1.4 - 0.1).clamp(0.0, 1.0);
    let spacing = 3.2;
    let (x0, x1) = (b.x0.max(visible.0 - 2.0), b.x1.min(visible.1 + 2.0));
    let mut puddles = Vec::new();
    for i in (x0 / spacing).floor() as i64..=(x1 / spacing).ceil() as i64 {
        if hash(i, 81) < 0.45 {
            continue;
        }
        let rx = (0.5 + 0.9 * hash(i, 82)) * grow;
        let x = (i as f64 + hash(i, 83)) * spacing;
        if x - rx < b.x0 + 0.2 || x + rx > b.x1 - 0.2 || rx < 0.05 {
            continue;
        }
        let z = b.z0 + 0.4 + (b.z1 - b.z0 - 1.2) * hash(i, 84);
        let rz = (0.25 + 0.2 * hash(i, 85)) * grow;
        let center = DVec3::new(x, b.y1 + 0.004, z);
        let flat = |sx: f64, sz: f64| canvas.project_ellipsoid(center, glam::DMat3::from_cols(DVec3::X * sx, DVec3::Z * sz, DVec3::Y * 1e-4));
        let shape = flat(rx, rz);
        // The far side reflects the bright sky, the near side the dark
        // canopy.
        let far = cam.point(center + DVec3::Z * rz);
        let near = cam.point(center - DVec3::Z * rz);
        let fill = Gradient::new_linear(far, near).with_stops([
            (0.0, SKY_REFLECTION.with_alpha(0.55)),
            (1.0, mix(SKY_REFLECTION, LEAF_DARK, 0.6).with_alpha(0.5)),
        ]);
        // Ripples: rings that spread and fade, a few at a time.
        let mut ripples = Vec::new();
        for k in 0..3i64 {
            let cycle = 0.9 + 0.4 * hash(i * 3 + k, 86);
            let t = (view.time / cycle + hash(i * 3 + k, 87)).fract();
            let n = (view.time / cycle + hash(i * 3 + k, 87)).floor() as i64;
            let at = DVec3::new(x + (hash(n * 7 + k, 88) - 0.5) * rx, center.y, z + (hash(n * 7 + k, 89) - 0.5) * rz);
            let r = 0.05 + 0.3 * t;
            let ring = canvas.project_ellipsoid(at, glam::DMat3::from_cols(DVec3::X * r, DVec3::Z * r, DVec3::Y * 1e-4));
            ripples.push((ring, (1.0 - t) * 0.5 * view.rain));
        }
        puddles.push((shape, fill, ripples));
    }
    if puddles.is_empty() {
        return;
    }
    let scale = cam.project(DVec3::new((x0 + x1) / 2.0, b.y1, b.z0)).scale;
    canvas.push(depth - 0.005, move |scene| {
        for (shape, fill, ripples) in &puddles {
            scene.fill(Fill::NonZero, Affine::IDENTITY, fill, None, shape);
            scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, shape);
            for (ring, alpha) in ripples {
                scene.stroke(&Stroke::new(0.015 * scale), Affine::IDENTITY, Color::WHITE.with_alpha(*alpha as f32), None, ring);
            }
            scene.pop_layer();
        }
    });
}

/// Grass (moss on stone) along the top of a platform: dense blades in rows
/// from the front edge back, clumped, swaying in the wind and bending round
/// Konrad, drawn by the grass shader. The front row is sorted on its own, so
/// it can hide his feet as he walks behind it.
fn draw_grass(canvas: &mut Canvas3d, b: &Block, top: Color, visible: (f64, f64), view: &WorldView, grass: &mut GrassFrame) {
    let cam = canvas.camera;
    let time = view.time;
    // Konrad pushes the grass aside where he stands or walks.
    let on_top = view.grounded && (view.player.y - b.y1).abs() < 0.3;
    let stone = b.kind == BlockKind::Stone;
    let (x0, x1) = (b.x0.max(visible.0 - 1.0), b.x1.min(visible.1 + 1.0));
    if x1 <= x0 {
        return;
    }
    let (height, spacing) = if stone { (0.14, 0.07) } else { (0.36, 0.04) };
    let (root, tip) = if stone {
        (Color::from_rgb8(0x2e, 0x46, 0x26), Color::from_rgb8(0x86, 0xb0, 0x58))
    } else {
        (darken(top, 0.15), lighten(top, 0.3))
    };
    // Some blades yellower, some bluer.
    let (dry, lush) = (Color::from_rgb8(0xb4, 0xb8, 0x5c), Color::from_rgb8(0x4a, 0x86, 0x66));
    let sway = 0.08 + 0.3 * wind();
    let rgb = |c: Color| [c.components[0], c.components[1], c.components[2]];
    for (row, dz) in [(0u64, 0.06), (1, 0.45), (2, 0.95), (3, 1.6)] {
        let z = b.z0 + dz;
        if z > b.z1 || (row > 0 && cam.eye.y < b.y1) {
            continue;
        }
        let step = spacing * if row == 0 { 1.0 } else { 1.4 };
        let fog = 0.1 * row as f32;
        let mut blades = Vec::new();
        for i in (x0 / step).floor() as i64..=(x1 / step).ceil() as i64 {
            let seed = row * 1000 + 60;
            let x = (i as f64 + hash(i, seed)) * step;
            if x < b.x0 + 0.03 || x > b.x1 - 0.03 {
                continue;
            }
            // Clumps: the grass grows in uneven patches.
            let clump = 0.5 + 0.3 * (x * 1.3 + row as f64).sin() + 0.2 * (x * 3.7 + 1.0).sin();
            let len = height * clump.max(0.15) * (0.55 + 0.7 * hash(i, seed + 1));
            let gust = ((time * 1.7 + x * 0.8).sin() + 0.35 * (time * 3.1 + x * 2.3).sin()) * sway;
            let mut lean = (hash(i, seed + 2) - 0.5) * 0.9 * len + gust * len;
            let away = x - view.player.x;
            if on_top && away.abs() < 0.7 {
                lean += away.signum() * (1.0 - away.abs() / 0.7) * len * 0.9;
            }
            // Spread in depth, so the rows don't show as lines.
            let z = (z + (hash(i, seed + 6) - 0.5) * 0.4).clamp(b.z0 + 0.02, b.z1);
            let base = DVec3::new(x, b.y1 - 0.03, z);
            let tip_at = DVec3::new(x + lean, b.y1 + len, z);
            let mid = DVec3::new(x + lean * 0.35, b.y1 + len * 0.55, z);
            let hue = hash(i, seed + 3);
            let tint = if hue < 0.3 { mix(tip, dry, 0.5) } else if hue > 0.8 { mix(tip, lush, 0.5) } else { tip };
            let scale = cam.project(base).scale;
            blades.push(BladeShape {
                base: cam.point(base),
                mid: cam.point(mid),
                tip: cam.point(tip_at),
                width: 0.03 * scale * (0.7 + 0.6 * hash(i, seed + 4)),
                root_color: rgb(darken(root, fog)),
                tip_color: rgb(darken(tint, fog)),
                seed: hash(i, seed + 5) as f32,
            });
        }
        let Some(patch) = grass.patch(&blades) else { continue };
        let depth = canvas.depth_of(DVec3::new((x0 + x1) / 2.0, b.y1, z));
        // The back rows are sorted with the platform, just in front of it.
        let depth = if row == 0 { depth } else { depth.min(canvas.depth_of(DVec3::new((b.x0 + b.x1) / 2.0, b.y1, (b.z0 + b.z1) / 2.0)) - 0.01) };
        canvas.push(depth, move |scene| patch.draw(scene));
    }
}

fn draw_trunk(canvas: &mut Canvas3d, x: f64, top: f64) {
    let cam = canvas.camera;
    let z = 1.1;
    let r = 0.65;
    let y_top = top + 4.0;
    let a = cam.project(DVec3::new(x - r, -1.0, z));
    let b = cam.project(DVec3::new(x + r, -1.0, z));
    let c = cam.project(DVec3::new(x + r * 0.8, y_top, z));
    let d = cam.project(DVec3::new(x - r * 0.8, y_top, z));
    let mut body = BezPath::new();
    body.move_to(a.pos);
    body.line_to(b.pos);
    body.line_to(c.pos);
    body.line_to(d.pos);
    body.close_path();
    let mid = a.pos.midpoint(c.pos);
    let width = (b.pos.x - a.pos.x) / 2.0;
    let mut grain = Vec::new();
    for k in 0..5 {
        let fx = -0.6 + k as f64 * 0.3;
        let mut line = BezPath::new();
        let n = 20;
        for i in 0..=n {
            let y = -1.0 + (y_top + 1.0) * i as f64 / n as f64;
            let wob = (y * 1.7 + k as f64 * 2.0).sin() * 0.05;
            let pt = cam.point(DVec3::new(x + (fx + wob) * r, y, z - 0.01));
            if i == 0 { line.move_to(pt) } else { line.line_to(pt) }
        }
        grain.push(line);
    }
    let canopy: Vec<(Point, f64)> = [(-1.4, 0.3, 1.6), (1.2, 0.5, 1.7), (0.0, 1.4, 2.0), (-0.3, -0.4, 1.3)]
        .iter()
        .map(|&(dx, dy, rr)| {
            let pr = cam.project(DVec3::new(x + dx, y_top + dy, z + 0.3));
            (pr.pos, rr * pr.scale)
        })
        .collect();
    canvas.push(a.depth + 0.01, move |scene| {
        let glaze = paint::tube_glaze(mid, Vec2::new(0.0, 1.0), width, BARK);
        wash(scene, &body, BARK, Some(&glaze), width * 2.0, 41, a.pos);
        for g in &grain {
            scene.stroke(&Stroke::new(1.5), Affine::IDENTITY, darken(BARK, 0.3).with_alpha(0.6), None, g);
        }
        paint::ink(scene, &body, width * 2.0, 41, a.pos);
        for (i, &(p, r)) in canopy.iter().enumerate() {
            let c = Circle::new(p, r).to_path(0.1);
            let color = if i % 2 == 0 { LEAF } else { lighten(LEAF, 0.1) };
            wash(scene, &c, color, Some(&paint::ball_glaze(p, r, color)), r * 2.0, 50 + i as u64, p);
        }
    });
    draw_trunk_base(canvas, x, z, r);
}

/// Where a trunk meets the ground: it flares out into roots that snake
/// over the ground towards the camera, with a soft shadow round the base.
fn draw_trunk_base(canvas: &mut Canvas3d, x: f64, z: f64, r: f64) {
    let cam = canvas.camera;
    let seed = (x * 7.0) as i64;
    let shadow = canvas.project_ellipsoid(DVec3::new(x, 0.0, z - 0.2), glam::DMat3::from_cols(DVec3::X * 2.4, DVec3::Z * 1.3, DVec3::Y * 1e-3));
    let shadow_shape = Affine::translate(shadow.center().to_vec2()) * Affine::rotate(shadow.rotation()) * Affine::scale_non_uniform(shadow.radii().x, shadow.radii().y);
    let shadow_fill = Gradient::new_radial((0.0, 0.0), 1.0).with_stops([
        (0.0, OCCLUSION.with_alpha(0.5)),
        (0.5, OCCLUSION.with_alpha(0.25)),
        (1.0, OCCLUSION.with_alpha(0.0)),
    ]);
    // The flare: wider at the ground, curving into the trunk.
    let mut flare = BezPath::new();
    flare.move_to(cam.point(DVec3::new(x - r * 1.7, -0.05, z)));
    flare.quad_to(cam.point(DVec3::new(x - r * 0.95, 0.1, z)), cam.point(DVec3::new(x - r * 0.97, 1.6, z)));
    flare.line_to(cam.point(DVec3::new(x + r * 0.97, 1.6, z)));
    flare.quad_to(cam.point(DVec3::new(x + r * 0.95, 0.1, z)), cam.point(DVec3::new(x + r * 1.7, -0.05, z)));
    flare.close_path();
    // It fades out upwards into the trunk.
    let flare_fill = Gradient::new_linear(cam.point(DVec3::new(x, 0.3, z)), cam.point(DVec3::new(x, 1.6, z)))
        .with_stops([(0.0, darken(BARK, 0.05)), (1.0, darken(BARK, 0.05).with_alpha(0.0))]);
    // Roots: each a curve from the flare out over the ground, thinning out.
    let mut roots: Vec<([Point; 4], f64)> = Vec::new();
    for k in 0..5i64 {
        let side = if k % 2 == 0 { 1.0 } else { -1.0 };
        let reach = 1.0 + 1.3 * hash(seed + k, 71);
        let toward = 0.2 + 1.0 * hash(seed + k, 72);
        let start = DVec3::new(x + side * r * (0.5 + 0.15 * k as f64), 0.5, z);
        let end = DVec3::new(x + side * (r + reach), -0.02, z - toward);
        let c1 = DVec3::new(start.x + side * reach * 0.3, 0.25, z - toward * 0.2);
        let c2 = DVec3::new(end.x - side * reach * 0.3, 0.05, z - toward * 0.8);
        let scale = cam.project(start).scale;
        roots.push(([cam.point(start), cam.point(c1), cam.point(c2), cam.point(end)], (0.2 + 0.12 * hash(seed + k, 73)) * scale));
    }
    let base = cam.project(DVec3::new(x, 0.0, z - 0.6));
    let bark_light = lighten(BARK, 0.18);
    canvas.push(base.depth, move |scene| {
        scene.fill(Fill::NonZero, Affine::IDENTITY, &shadow_fill, Some(shadow_shape), &shadow);
        scene.fill(Fill::NonZero, Affine::IDENTITY, &flare_fill, None, &flare);
        for (pts, width) in &roots {
            // Drawn in pieces, each thinner, so the root tapers.
            for piece in 0..4 {
                let (t0, t1) = (piece as f64 / 4.0, (piece + 1) as f64 / 4.0);
                let seg = vello::kurbo::ParamCurve::subsegment(&vello::kurbo::CubicBez::new(pts[0], pts[1], pts[2], pts[3]), t0..t1);
                let w = width * (1.0 - 0.22 * piece as f64);
                let cap = Stroke::new(w).with_caps(vello::kurbo::Cap::Round);
                scene.stroke(&cap, Affine::IDENTITY, darken(BARK, 0.08), None, &seg);
                // Light along the top of the root.
                let lit = Stroke::new(w * 0.3).with_caps(vello::kurbo::Cap::Round);
                scene.stroke(&lit, Affine::translate((0.0, -w * 0.25)), bark_light.with_alpha(0.5), None, &seg);
            }
        }
    });
}

/// Where fly `i` is right now: buzzing around its home.
pub fn fly_position(home: DVec2, time: f64, i: usize) -> DVec2 {
    let f = i as f64 * 1.37;
    home + DVec2::new((time * 1.9 + f).sin() * 0.35, (time * 2.7 + f * 2.0).cos() * 0.22)
}

fn draw_fly(canvas: &mut Canvas3d, pos: DVec2, time: f64, i: usize) {
    let cam = canvas.camera;
    let pr = cam.project(DVec3::new(pos.x, pos.y, -0.05));
    let s = pr.scale;
    let pulse = 0.85 + 0.15 * (time * 5.0 + i as f64).sin();
    let turn = (time * 2.0 + i as f64).cos();
    canvas.push(pr.depth, move |scene| {
        let glow = Gradient::new_radial(pr.pos, (0.45 * s * pulse) as f32).with_stops([
            Color::from_rgb8(0x7a, 0xe8, 0xff).with_alpha(0.55),
            Color::from_rgb8(0x7a, 0xe8, 0xff).with_alpha(0.0),
        ]);
        scene.fill(Fill::NonZero, Affine::IDENTITY, &glow, None, &Circle::new(pr.pos, 0.45 * s * pulse));
        // A spinning diamond: the width follows the turn.
        let (w, h) = (0.1 * s * turn.abs().max(0.25), 0.16 * s);
        let mut gem = BezPath::new();
        gem.move_to(pr.pos + Vec2::new(0.0, -h));
        gem.line_to(pr.pos + Vec2::new(w, 0.0));
        gem.line_to(pr.pos + Vec2::new(0.0, h));
        gem.line_to(pr.pos + Vec2::new(-w, 0.0));
        gem.close_path();
        scene.fill(Fill::NonZero, Affine::IDENTITY, Color::from_rgb8(0x4a, 0xc8, 0xe8), None, &gem);
        let mut facet = BezPath::new();
        facet.move_to(pr.pos + Vec2::new(0.0, -h));
        facet.line_to(pr.pos + Vec2::new(w * turn.signum(), 0.0));
        facet.line_to(pr.pos);
        facet.close_path();
        scene.fill(Fill::NonZero, Affine::IDENTITY, Color::from_rgb8(0xd8, 0xfa, 0xff), None, &facet);
    });
}

fn draw_checkpoint(canvas: &mut Canvas3d, at: DVec2, reached: bool, time: f64) {
    let cam = canvas.camera;
    let base = DVec3::new(at.x, at.y, 0.7);
    let top = base + DVec3::Y * 1.8;
    canvas.capsule(base, 0.06, top, 0.05, BARK);
    let color = if reached { Color::from_rgb8(0xf2, 0x7a, 0x2e) } else { Color::from_rgb8(0x9a, 0x9a, 0x90) };
    let mut flag = BezPath::new();
    let n = 8;
    for i in 0..=n {
        let f = i as f64 / n as f64;
        let wave = (time * 5.0 - f * 4.0).sin() * 0.08 * f;
        let p = cam.point(top + DVec3::new(f * 0.9, -0.05 + wave, 0.0));
        if i == 0 { flag.move_to(p) } else { flag.line_to(p) }
    }
    for i in (0..=n).rev() {
        let f = i as f64 / n as f64;
        let wave = (time * 5.0 - f * 4.0).sin() * 0.08 * f;
        flag.line_to(cam.point(top + DVec3::new(f * 0.9, -0.5 + f * 0.2 + wave, 0.0)));
    }
    flag.close_path();
    let depth = canvas.depth_of(top) - 0.01;
    canvas.push(depth, move |scene| {
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &flag);
        scene.stroke(&Stroke::new(1.5), Affine::IDENTITY, OUTLINE, None, &flag);
    });
}

fn draw_goal(canvas: &mut Canvas3d, at: DVec2, time: f64) {
    let cam = canvas.camera;
    // A golden star spinning above its pedestal (a block in the level).
    let c = DVec3::new(at.x, at.y + 2.2 + (time * 2.0).sin() * 0.15, 0.3);
    let pr = cam.project(c);
    let s = pr.scale;
    let spin = (time * 1.5).cos();
    canvas.push(pr.depth, move |scene| {
        let glow = Gradient::new_radial(pr.pos, (1.6 * s) as f32).with_stops([
            Color::from_rgb8(0xff, 0xe0, 0x60).with_alpha(0.6),
            Color::from_rgb8(0xff, 0xe0, 0x60).with_alpha(0.0),
        ]);
        scene.fill(Fill::NonZero, Affine::IDENTITY, &glow, None, &Circle::new(pr.pos, 1.6 * s));
        let mut star = BezPath::new();
        for k in 0..10 {
            let a = -PI / 2.0 + k as f64 * PI / 5.0;
            let r = if k % 2 == 0 { 0.7 } else { 0.3 } * s;
            let p = pr.pos + Vec2::new(a.cos() * r * spin, a.sin() * r);
            if k == 0 { star.move_to(p) } else { star.line_to(p) }
        }
        star.close_path();
        let gold = Gradient::new_linear(pr.pos - Vec2::new(0.0, 0.7 * s), pr.pos + Vec2::new(0.0, 0.7 * s))
            .with_stops([Color::from_rgb8(0xff, 0xf2, 0x9a), Color::from_rgb8(0xe0, 0xa0, 0x20)]);
        wash(scene, &star, Color::from_rgb8(0xf2, 0xc2, 0x3a), Some(&gold), 1.4 * s, 3, pr.pos);
        paint::ink(scene, &star, 1.4 * s, 3, pr.pos);
    });
}

/// Plants close to the camera, framing the view: ferns rising from below
/// and leafy branches hanging from above. They're out of focus (a soft halo
/// round a dark silhouette) and move fastest with the camera, which gives
/// the scene depth.
fn draw_foreground(canvas: &mut Canvas3d, w: f64, time: f64) {
    let cam = canvas.camera;
    let z = -5.0;
    let (x0, x1) = cam.visible_x(z, w);
    let spacing = 4.0;
    for i in (x0 / spacing).floor() as i64 - 1..=(x1 / spacing).ceil() as i64 + 1 {
        let x = (i as f64 + hash(i, 51)) * spacing;
        let sway = (time * 0.9 + i as f64).sin() * 0.04 * (0.4 + wind()) + 0.06 * wind();
        let color = mix(LEAF_DARK, ACCENTS[(hash(i, 54) * 4.0) as usize % 4], 0.25);
        let mut shape = BezPath::new();
        let kind = hash(i, 50);
        if kind < 0.5 {
            // A fern: fronds of paired leaflets, fanning up from below.
            let base = DVec3::new(x, -2.0 - hash(i, 52) * 0.6, z);
            let fronds = 4 + (hash(i, 53) * 3.0) as usize;
            for k in 0..fronds {
                let spread = (k as f64 / (fronds - 1) as f64 - 0.5) * 1.6 + sway;
                let len = 2.6 + 1.4 * hash(i * 8 + k as i64, 55);
                frond(&mut shape, &cam, base, spread, len, 0.32);
            }
        } else if kind < 0.7 {
            // A branch hanging into view from above, leaves along it.
            let top = DVec3::new(x, 4.6 + 1.0 * hash(i, 56), z);
            let len = 2.2 + 1.2 * hash(i, 57);
            frond(&mut shape, &cam, top, std::f64::consts::PI + sway * 2.0 + (hash(i, 58) - 0.5) * 0.5, len, 0.26);
        } else {
            continue;
        }
        let blur = 0.09 * cam.project(DVec3::new(x, 0.0, z)).scale;
        let depth = cam.project(DVec3::new(x, 0.0, z)).depth;
        canvas.push(depth, move |scene| {
            // Out of focus: a faint halo round the shape.
            let halo = Stroke::new(blur * 1.5).with_join(vello::kurbo::Join::Round);
            scene.stroke(&halo, Affine::IDENTITY, color.with_alpha(0.3), None, &shape);
            scene.fill(Fill::NonZero, Affine::IDENTITY, color.with_alpha(0.92), None, &shape);
        });
    }
}

/// Adds a frond to `path`: a curving stem from `base` at `angle` (0 = up,
/// positive leaning right) of `len` world units, with leaflets in pairs
/// that shrink towards the tip; `leaf` is the largest leaflet's length.
fn frond(path: &mut BezPath, cam: &Camera, base: DVec3, angle: f64, len: f64, leaf: f64) {
    let dir = |a: f64| DVec3::new(a.sin(), a.cos(), 0.0);
    // The stem arcs over: its direction turns further as it goes.
    let at = |t: f64| {
        let a = angle + angle.signum() * t * t * 0.6;
        base + dir(angle) * len * t * 0.5 + dir(a) * len * t * 0.5
    };
    let steps = 12;
    for j in 1..steps {
        let t = j as f64 / steps as f64;
        let p = at(t);
        let along = (at(t + 0.02) - at(t - 0.02)).normalize();
        let size = leaf * (1.0 - t * 0.75);
        for side in [-1.0, 1.0] {
            // Leaflets point outwards and a little towards the tip.
            let out = DVec3::new(-along.y, along.x, 0.0) * side;
            let tip = p + (out * 0.85 + along * 0.5) * size;
            let n = along * size * 0.18;
            path.move_to(cam.point(p - n));
            path.quad_to(cam.point(p.lerp(tip, 0.5) + along * size * 0.2), cam.point(tip));
            path.quad_to(cam.point(p.lerp(tip, 0.5) - along * size * 0.1), cam.point(p + n));
            path.close_path();
        }
    }
    // The stem itself, a thin tapered sliver.
    let w = leaf * 0.06;
    path.move_to(cam.point(base - DVec3::X * w));
    for j in 1..=steps {
        path.line_to(cam.point(at(j as f64 / steps as f64)));
    }
    path.line_to(cam.point(base + DVec3::X * w));
    path.close_path();
}
