//! Drawing the jungle: sky, fogged background layers, platforms as 3D boxes,
//! vines, trees, swing flowers, flies, checkpoints, the goal and foreground
//! foliage.

use std::f64::consts::{PI, TAU};

use glam::{DVec2, DVec3};
use vello::kurbo::{Affine, BezPath, Circle, Ellipse, Point, Rect, Shape, Stroke, Vec2};
use vello::peniko::{Color, Fill, Gradient};
use vello::Scene;

use crate::canvas3d::{darken, lighten, mix, Camera, Canvas3d, OUTLINE};
use crate::paint::{self, wash};
use crate::level::{Block, BlockKind, ClimbKind, Level};

const SKY_TOP: Color = Color::from_rgb8(0x6f, 0xa8, 0xb4);
const SKY_HORIZON: Color = Color::from_rgb8(0xf4, 0xe6, 0xbe);
const FOG: Color = Color::from_rgb8(0xcf, 0xdf, 0xc0);
const LEAF: Color = Color::from_rgb8(0x5a, 0x9a, 0x4a);
const LEAF_DARK: Color = Color::from_rgb8(0x2c, 0x55, 0x33);
const BARK: Color = Color::from_rgb8(0x8e, 0x62, 0x3c);

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
    Layer { z: 90.0, base: -6.0, amp: 22.0, color: Color::from_rgb8(0x5d, 0x8c, 0x7a), fog: 0.55, tree_spacing: 0.0, tree_size: 0.0, seed: 1 },
    Layer { z: 40.0, base: -3.0, amp: 4.0, color: Color::from_rgb8(0x3f, 0x77, 0x52), fog: 0.45, tree_spacing: 6.0, tree_size: 3.2, seed: 2 },
    Layer { z: 18.0, base: -2.5, amp: 2.5, color: Color::from_rgb8(0x2f, 0x62, 0x3e), fog: 0.28, tree_spacing: 4.5, tree_size: 2.4, seed: 3 },
    Layer { z: 7.0, base: -3.0, amp: 1.5, color: Color::from_rgb8(0x24, 0x4c, 0x30), fog: 0.12, tree_spacing: 3.8, tree_size: 2.0, seed: 4 },
];

/// Sky and background layers, drawn straight into the scene (always behind).
pub fn draw_background(scene: &mut Scene, cam: &Camera, w: f64, h: f64, time: f64) {
    let sky = Gradient::new_linear((0.0, 0.0), (0.0, h)).with_stops([SKY_TOP, SKY_HORIZON, SKY_HORIZON]);
    scene.fill(Fill::NonZero, Affine::IDENTITY, &sky, None, &Rect::new(0.0, 0.0, w, h));
    let sun = Point::new(w * 0.72, h * 0.18);
    let glow = Gradient::new_radial(sun, (h * 0.5) as f32).with_stops([
        Color::from_rgb8(0xff, 0xf6, 0xd0).with_alpha(0.9),
        Color::from_rgb8(0xff, 0xf6, 0xd0).with_alpha(0.0),
    ]);
    scene.fill(Fill::NonZero, Affine::IDENTITY, &glow, None, &Rect::new(0.0, 0.0, w, h));
    // Watercolour blooms: soft blotches where the wash dried unevenly.
    for k in 0..7 {
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
        let blotch = paint::wobble(&Circle::new(c, r).to_path(1.0), r * 0.12, k as u64);
        scene.fill(Fill::NonZero, Affine::IDENTITY, &bloom, None, &blotch);
    }

    for (li, layer) in LAYERS.iter().enumerate() {
        let color = mix(layer.color, FOG, layer.fog);
        let (x0, x1) = cam.visible_x(layer.z, w);
        let height = |x: f64| {
            let n = wave(x, layer.seed as f64 * 1.7);
            if li == 0 {
                // Jagged mountains.
                layer.base + layer.amp * (n * n + 0.25 * (x * 0.9).sin().abs())
            } else {
                layer.base + layer.amp * n
            }
        };
        let steps = 90;
        let mut hills = BezPath::new();
        for i in 0..=steps {
            let x = x0 + (x1 - x0) * i as f64 / steps as f64;
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
        paint::wash_with_edge(scene, &hills, color, None, 160.0, li as u64 + 7, 0.1 + 0.08 * li as f32);

        if layer.tree_spacing > 0.0 {
            let first = (x0 / layer.tree_spacing).floor() as i64 - 1;
            let last = (x1 / layer.tree_spacing).ceil() as i64 + 1;
            for i in first..=last {
                let x = (i as f64 + hash(i, layer.seed)) * layer.tree_spacing;
                let size = layer.tree_size * (0.7 + 0.6 * hash(i, layer.seed + 10));
                let p = cam.project(DVec3::new(x, height(x) - 0.3, layer.z));
                let tf = Affine::translate(p.pos.to_vec2()) * Affine::scale_non_uniform(p.scale * size, -p.scale * size);
                let sway = (time * 0.8 + i as f64).sin() * 0.04;
                let seed = (i as u64).wrapping_mul(31) ^ layer.seed;
                let size_px = p.scale * size;
                if hash(i, layer.seed + 20) < 0.4 {
                    draw_palm(scene, tf, color, sway, size_px, seed);
                } else {
                    draw_tree(scene, tf, color, hash(i, layer.seed + 30), size_px, seed);
                }
            }
        }

        // Sun rays between the layers.
        if li == 1 {
            for k in 0..4 {
                let x = w * (0.15 + 0.25 * k as f64) + (cam.eye.x * -2.0) % (w * 0.25);
                let spread = w * 0.06 * (1.0 + (time * 0.3 + k as f64).sin() * 0.3);
                let mut ray = BezPath::new();
                ray.move_to((x, -10.0));
                ray.line_to((x + spread, -10.0));
                ray.line_to((x + spread * 3.0 - w * 0.2, h));
                ray.line_to((x - w * 0.2, h));
                ray.close_path();
                let g = Gradient::new_linear((0.0, 0.0), (0.0, h)).with_stops([
                    Color::WHITE.with_alpha(0.18),
                    Color::WHITE.with_alpha(0.0),
                ]);
                scene.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &ray);
            }
        }
    }
}

/// A broadleaf tree in local units (y up, base at the origin, ~1 unit tall).
fn draw_tree(scene: &mut Scene, tf: Affine, color: Color, variant: f64, size: f64, seed: u64) {
    let trunk = tf * Rect::new(-0.06, 0.0, 0.06, 0.6).to_path(0.1);
    paint::wash_with_edge(scene, &trunk, darken(color, 0.25), None, size * 0.2, seed, 0.15);
    let blobs = [(0.0, 0.75, 0.3), (-0.22, 0.62, 0.22), (0.22, 0.64, 0.24), (0.05 * variant, 0.95, 0.22)];
    for (i, (x, y, r)) in blobs.iter().enumerate() {
        let c = if i % 2 == 0 { color } else { lighten(color, 0.08) };
        let blob = tf * Circle::new((*x, *y), *r).to_path(0.01);
        paint::wash_with_edge(scene, &blob, c, None, size * r * 2.0, seed ^ i as u64, 0.2);
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
    paint::wash_with_edge(scene, &(tf * trunk), darken(color, 0.2), None, size * 0.1, seed, 0.15);
    for k in 0..7 {
        let a = PI * (0.05 + k as f64 / 6.0 * 0.9) + sway;
        let tip = top + Vec2::new(a.cos() * 0.45, a.sin() * 0.18 - 0.2);
        let mid = top.midpoint(tip) + Vec2::new(0.0, 0.14);
        let mut frond = BezPath::new();
        frond.move_to(top);
        frond.quad_to(mid + Vec2::new(0.0, 0.06), tip);
        frond.quad_to(mid - Vec2::new(0.0, 0.02), top);
        paint::wash_with_edge(scene, &(tf * frond), lighten(color, 0.04), None, size * 0.3, seed ^ k as u64, 0.2);
    }
}

/// Per-frame state the world needs for drawing.
pub struct WorldView<'a> {
    pub time: f64,
    pub flies: &'a [DVec2],
    pub caught: &'a [bool],
    pub checkpoint: usize,
    /// The hook the tongue would grab right now, if any.
    pub hook_hint: Option<usize>,
    pub screen_width: f64,
}

pub fn draw_world(canvas: &mut Canvas3d, level: &Level, view: &WorldView) {
    let cam = canvas.camera;
    let (vx0, vx1) = cam.visible_x(-1.5, view.screen_width);
    for b in &level.blocks {
        if b.x1 < vx0 - 2.0 || b.x0 > vx1 + 2.0 {
            continue;
        }
        draw_block(canvas, b);
    }
    for c in &level.climbables {
        match c.kind {
            ClimbKind::Vine => draw_vine(canvas, c.x, c.y0, c.y1 + 14.0, view.time),
            ClimbKind::Trunk => draw_trunk(canvas, c.x, c.y1),
        }
    }
    for (i, &h) in level.hooks.iter().enumerate() {
        draw_hook(canvas, h, view.time, view.hook_hint == Some(i));
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

fn draw_block(canvas: &mut Canvas3d, b: &Block) {
    let cam = canvas.camera;
    let (top, front, side) = match b.kind {
        BlockKind::Ground => (Color::from_rgb8(0x8c, 0xc0, 0x63), Color::from_rgb8(0xa9, 0x78, 0x4a), Color::from_rgb8(0x86, 0x5c, 0x38)),
        BlockKind::Stone => (Color::from_rgb8(0xa4, 0xb8, 0x86), Color::from_rgb8(0x9a, 0xa3, 0xab), Color::from_rgb8(0x7a, 0x82, 0x8c)),
        BlockKind::Log => (Color::from_rgb8(0xb5, 0x82, 0x4e), Color::from_rgb8(0x9c, 0x6a, 0x3a), Color::from_rgb8(0xe0, 0xbc, 0x8a)),
    };
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
    let side_face = |x: f64| quad(p(x, y0, b.z0), p(x, b.y1, b.z0), p(x, b.y1, b.z1), p(x, y0, b.z1));
    if cam.eye.x < b.x0 {
        faces.push((side_face(b.x0), side));
    }
    if cam.eye.x > b.x1 {
        faces.push((side_face(b.x1), side));
    }
    if cam.eye.y > b.y1 {
        faces.push((quad(p(b.x0, b.y1, b.z0), p(b.x1, b.y1, b.z0), p(b.x1, b.y1, b.z1), p(b.x0, b.y1, b.z1)), top));
    }
    let front_face = quad(p(b.x0, y0, b.z0), p(b.x1, y0, b.z0), p(b.x1, b.y1, b.z0), p(b.x0, b.y1, b.z0));

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
            let rocks = ((b.x1 - b.x0) * 0.8) as i64;
            for i in 0..rocks {
                let x = b.x0 + 0.3 + hash(seed + i, 7) * (b.x1 - b.x0 - 0.6);
                let y = b.y1 - 0.5 - hash(seed + i, 8) * 4.0;
                if y <= y0 + 0.2 {
                    continue;
                }
                let c = cam.project(DVec3::new(x, y, b.z0));
                let r = (0.1 + 0.15 * hash(seed + i, 9)) * c.scale;
                details.push((Ellipse::new(c.pos, (r * 1.4, r), 0.0).to_path(0.1), lighten(front, 0.12), None));
            }
            // Grass or moss hanging over the front edge.
            let fringe_color = if b.kind == BlockKind::Ground { top } else { Color::from_rgb8(0x6a, 0x9a, 0x4a) };
            let mut fringe = BezPath::new();
            fringe.move_to(p(b.x0, b.y1, b.z0));
            let mut x = b.x0;
            let step = 0.22;
            let mut i = 0;
            while x < b.x1 {
                let len = 0.15 + 0.25 * hash(seed + i, 3);
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
    let center = DVec3::new((b.x0 + b.x1) / 2.0, (y0.max(cam.eye.y - 8.0) + b.y1) / 2.0, (b.z0 + b.z1) / 2.0);
    // Boxes further away in the xy plane are drawn first.
    let dx = (b.x0 - cam.eye.x).max(cam.eye.x - b.x1).max(0.0);
    let dy = (b.y0 - cam.eye.y).max(cam.eye.y - b.y1).max(0.0);
    let depth = canvas.depth_of(center) + (dx + dy) * 0.001;
    let seed = paint::seed(&[b.x0, b.y1]);
    canvas.push(depth, move |scene| {
        for (i, (face, color)) in faces.iter().enumerate() {
            wash(scene, face, *color, None, scale * 2.0, seed ^ i as u64);
            paint::ink(scene, face, scale * 2.0, seed ^ i as u64);
        }
        wash(scene, &front_face, front, None, scale * 2.0, seed ^ 0xf);
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &front_face);
        for (i, (path, color, stroke)) in details.iter().enumerate() {
            match stroke {
                Some(w) => scene.stroke(&Stroke::new(w * scale), Affine::IDENTITY, color.with_alpha(0.6), None, path),
                None => wash(scene, path, *color, None, scale * 0.3, seed ^ (i as u64 + 20)),
            }
        }
        scene.pop_layer();
        paint::ink(scene, &front_face, scale * 2.0, seed);
    });
}

fn draw_vine(canvas: &mut Canvas3d, x: f64, y0: f64, y1: f64, time: f64) {
    let cam = canvas.camera;
    let z = 0.45;
    let sway = |y: f64| (y * 1.3 + time * 1.5).sin() * 0.06 + (y1 - y) * 0.0;
    let mut stem = BezPath::new();
    let n = ((y1 - y0) / 0.4).ceil() as usize;
    for i in 0..=n {
        let y = y0 + (y1 - y0) * i as f64 / n as f64;
        let pt = cam.point(DVec3::new(x + sway(y), y, z));
        if i == 0 { stem.move_to(pt) } else { stem.line_to(pt) }
    }
    let mut leaves = Vec::new();
    let mut y = y0 + 0.3;
    let mut side = 1.0;
    while y < y1 {
        let base = DVec3::new(x + sway(y), y, z);
        let pr = cam.project(base);
        let r = 0.2 * pr.scale;
        let angle = if side > 0.0 { -0.5 } else { PI + 0.5 };
        let c = pr.pos + Vec2::new(angle.cos(), angle.sin()) * r;
        leaves.push(Ellipse::new(c, (r, r * 0.45), angle));
        y += 0.55;
        side = -side;
    }
    let pr = cam.project(DVec3::new(x, (y0 + y1) / 2.0, z));
    canvas.push(pr.depth, move |scene| {
        let w = 0.09 * pr.scale;
        let stem = paint::wobble(&stem, w * 0.3, 5);
        scene.stroke(&Stroke::new(w), Affine::IDENTITY, Color::from_rgb8(0x5e, 0x86, 0x3a).with_alpha(0.85), None, &stem);
        scene.stroke(&Stroke::new(1.2), Affine::IDENTITY, OUTLINE.with_alpha(0.5), None, &stem);
        for (i, leaf) in leaves.iter().enumerate() {
            let path = leaf.to_path(0.1);
            wash(scene, &path, LEAF, None, leaf.radii().x * 2.0, i as u64);
            paint::ink(scene, &path, leaf.radii().x * 2.0, i as u64);
        }
    });
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
        wash(scene, &body, BARK, Some(&glaze), width * 2.0, 41);
        for g in &grain {
            scene.stroke(&Stroke::new(1.5), Affine::IDENTITY, darken(BARK, 0.3).with_alpha(0.6), None, g);
        }
        paint::ink(scene, &body, width * 2.0, 41);
        for (i, &(p, r)) in canopy.iter().enumerate() {
            let c = Circle::new(p, r).to_path(0.1);
            let color = if i % 2 == 0 { LEAF } else { lighten(LEAF, 0.1) };
            wash(scene, &c, color, Some(&paint::ball_glaze(p, r, color)), r * 2.0, 50 + i as u64);
        }
    });
}

fn draw_hook(canvas: &mut Canvas3d, h: DVec2, time: f64, hint: bool) {
    let cam = canvas.camera;
    let center = DVec3::new(h.x, h.y, 0.0);
    let pr = cam.project(center);
    let stem_top = cam.point(DVec3::new(h.x + 0.2, h.y + 9.0, 0.05));
    let s = pr.scale;
    let spin = time * 0.6;
    let pulse = if hint { 1.0 + (time * 8.0).sin() * 0.12 } else { 1.0 };
    canvas.push(pr.depth, move |scene| {
        let mut stem = BezPath::new();
        stem.move_to(stem_top);
        stem.quad_to(pr.pos + Vec2::new(0.4 * s, -3.0 * s), pr.pos);
        scene.stroke(&Stroke::new(0.07 * s), Affine::IDENTITY, Color::from_rgb8(0x5e, 0x86, 0x3a).with_alpha(0.85), None, &stem);
        scene.stroke(&Stroke::new(1.2), Affine::IDENTITY, OUTLINE.with_alpha(0.5), None, &stem);
        if hint {
            let glow = Gradient::new_radial(pr.pos, (0.9 * s) as f32).with_stops([
                Color::from_rgb8(0xff, 0xf0, 0x80).with_alpha(0.7),
                Color::from_rgb8(0xff, 0xf0, 0x80).with_alpha(0.0),
            ]);
            scene.fill(Fill::NonZero, Affine::IDENTITY, &glow, None, &Circle::new(pr.pos, 0.9 * s));
        }
        for k in 0..5 {
            let a = spin + k as f64 * TAU / 5.0;
            let c = pr.pos + Vec2::new(a.cos(), a.sin()) * 0.22 * s * pulse;
            let petal = Ellipse::new(c, (0.2 * s * pulse, 0.11 * s * pulse), a);
            let petal = petal.to_path(0.1);
            wash(scene, &petal, Color::from_rgb8(0xe0, 0x5f, 0x9e), None, 0.4 * s, k as u64);
            paint::ink(scene, &petal, 0.4 * s, k as u64);
        }
        let bud = Circle::new(pr.pos, 0.12 * s);
        wash(scene, &bud.to_path(0.1), Color::from_rgb8(0xf6, 0xd0, 0x4a), None, 0.24 * s, 9);
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
    let flap = (time * 50.0 + i as f64).sin().abs();
    canvas.push(pr.depth, move |scene| {
        for side in [-1.0, 1.0] {
            let wing = Ellipse::new(
                pr.pos + Vec2::new(side * 0.07 * s, -0.08 * s),
                (0.09 * s, 0.05 * s * (0.3 + flap)),
                side * 0.5,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, Color::from_rgb8(0xe8, 0xf4, 0xff).with_alpha(0.75), None, &wing);
            scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, OUTLINE, None, &wing);
        }
        let body = Ellipse::new(pr.pos, (0.1 * s, 0.07 * s), 0.0);
        scene.fill(Fill::NonZero, Affine::IDENTITY, Color::from_rgb8(0x2a, 0x33, 0x2a), None, &body);
        scene.fill(Fill::NonZero, Affine::IDENTITY, Color::from_rgb8(0xd0, 0x30, 0x30), None, &Circle::new(pr.pos + Vec2::new(0.07 * s, -0.02 * s), 0.035 * s));
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
        flag.line_to(cam.point(top + DVec3::new(f * 0.9, -0.05 + wave, 0.0)));
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
        wash(scene, &star, Color::from_rgb8(0xf2, 0xc2, 0x3a), Some(&gold), 1.4 * s, 3);
        paint::ink(scene, &star, 1.4 * s, 3);
    });
}

fn draw_foreground(canvas: &mut Canvas3d, w: f64, time: f64) {
    let cam = canvas.camera;
    // Ferns close to the camera, rising from below the screen.
    let z = -3.6;
    let (x0, x1) = cam.visible_x(z, w);
    let spacing = 4.5;
    for i in (x0 / spacing).floor() as i64 - 1..=(x1 / spacing).ceil() as i64 + 1 {
        if hash(i, 50) < 0.35 {
            continue;
        }
        let x = (i as f64 + hash(i, 51)) * spacing;
        let base = DVec3::new(x, -2.2 - hash(i, 52) * 1.2, z);
        let pr = cam.project(base);
        let s = pr.scale * (1.1 + hash(i, 53) * 0.8);
        let sway = (time * 1.1 + i as f64).sin() * 0.05;
        let color = if hash(i, 54) < 0.5 { LEAF_DARK } else { darken(LEAF, 0.4) };
        canvas.push(pr.depth, move |scene| {
            for k in 0..7 {
                let a = -PI / 2.0 + (k as f64 - 3.0) * 0.32 + sway;
                let len = s * (1.6 - (k as f64 - 3.0).abs() * 0.18);
                let tip = pr.pos + Vec2::new(a.cos(), a.sin()) * len;
                let bend = Vec2::new(-(a.sin()), a.cos()) * len * 0.2 * if k < 3 { -1.0 } else { 1.0 };
                let mid = pr.pos.midpoint(tip) + bend;
                let normal = Vec2::new(-(a.sin()), a.cos()) * s * 0.18;
                let mut frond = BezPath::new();
                frond.move_to(pr.pos);
                frond.quad_to(mid + normal, tip);
                frond.quad_to(mid - normal, pr.pos);
                wash(scene, &frond, color, None, len * 0.4, (i as u64) << 4 | k as u64);
            }
        });
    }
}
