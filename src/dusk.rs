//! The dusk city: a future metropolis at sunset, in the spirit of Flashback's
//! New Washington. Everything is vector shapes bent by noise, so hardly
//! anything is straight or regular: terraced mesas, a jagged skyline,
//! chipped concrete, stains, rubble and weeds.
//!
//! A low sun on the right lights the faces turned towards it and throws long
//! shadows to the left. Shadows are real geometry: each caster's corners are
//! projected along the sunlight onto the platform tops (see `shadow_path`),
//! so they move with the running figure and stretch over the roofs.
//!
//! The sky itself is drawn by a shader (see `frame`).
//!
//! On slow devices the detail levels (`paint::detail`) thin things out:
//! level 1 drops small clutter (windows in the skylines, rubble, weeds,
//! stains, creepers, dust, half the traffic), level 2 also the airship,
//! the traffic, the foreground and the industry's lights and steam.

use std::f64::consts::TAU;

use glam::{DMat3, DVec2, DVec3};
use vello::kurbo::{Affine, BezPath, Cap, Circle, Ellipse, Join, Point, Rect, Shape, Stroke, Vec2};
use vello::peniko::{Color, Fill, Gradient};
use vello::Scene;

use crate::canvas3d::{darken, lighten, mix, Camera, Canvas3d};
use crate::jungle::{block_depth, top_face, WorldView};
use crate::level::{Block, BlockKind, Level, Prop, PropKind};
use crate::noise::{fbm1, fbm2, hash, noise1, ridge1};
use crate::rig::Solved;

/// Where the sun sits on screen: low, just above the horizon, on the right.
pub const SUN: (f64, f64) = (0.8, 0.37);
/// The sun's direction on screen, for rim light on edges facing it.
pub const SUN_SCREEN_DIR: Vec2 = Vec2::new(0.96, -0.28);

/// Towards the sun, for shadows: low (about 11°), from the right and a
/// little from behind, so shadows fall to the left and slightly forwards.
fn sun_dir() -> DVec3 {
    DVec3::new(0.9, 0.19, 0.36).normalize()
}

const TOWER: Color = Color::from_rgb8(0x3a, 0x22, 0x4c);
/// Haze near the horizon: distant things fade into it.
const HAZE: Color = Color::from_rgb8(0xe4, 0x8a, 0x88);
const SUNLIT: Color = Color::from_rgb8(0xff, 0x98, 0x60);
const RIM: Color = Color::from_rgb8(0xff, 0xcc, 0x8e);
const WINDOW: Color = Color::from_rgb8(0xff, 0xc2, 0x76);
const NEON_PINK: Color = Color::from_rgb8(0xff, 0x4a, 0xaa);
const NEON_CYAN: Color = Color::from_rgb8(0x5a, 0xee, 0xff);
const SHADOW: Color = Color::from_rgb8(0x2a, 0x10, 0x3a);
const CONCRETE_TOP: Color = Color::from_rgb8(0xae, 0x78, 0x76);
const CONCRETE_FRONT: Color = Color::from_rgb8(0x3c, 0x2a, 0x4a);
const CONCRETE_LIT: Color = Color::from_rgb8(0xee, 0x92, 0x6a);
const CONCRETE_DARK: Color = Color::from_rgb8(0x2c, 0x1e, 0x3a);
const GLASS: Color = Color::from_rgb8(0x2e, 0x2e, 0x52);
const STEEL: Color = Color::from_rgb8(0x3c, 0x36, 0x4c);
const STEEL_TOP: Color = Color::from_rgb8(0x7e, 0x62, 0x6e);
const WEED: Color = Color::from_rgb8(0x4a, 0x52, 0x32);
/// Cargo container paints: rust red, teal, ochre and navy.
const PAINTS: [Color; 4] = [
    Color::from_rgb8(0x8e, 0x3e, 0x34),
    Color::from_rgb8(0x2c, 0x5c, 0x66),
    Color::from_rgb8(0x9a, 0x76, 0x38),
    Color::from_rgb8(0x36, 0x3c, 0x62),
];
/// Camouflage colours on a roof: concrete in shade, and lit.
pub const CAMO: (Color, Color) = (CONCRETE_FRONT, CONCRETE_TOP);

/// A closed polygon through 3D points, on screen.
fn poly(cam: &Camera, pts: &[DVec3]) -> BezPath {
    let mut path = BezPath::new();
    for (i, p) in pts.iter().enumerate() {
        let q = cam.point(*p);
        if i == 0 { path.move_to(q) } else { path.line_to(q) }
    }
    if !pts.is_empty() {
        path.close_path();
    }
    path
}

/// A line turned into a fillable shape of the given width.
fn outline(path: &BezPath, width: f64) -> BezPath {
    let style = Stroke::new(width).with_caps(Cap::Round);
    vello::kurbo::stroke(path.iter(), &style, &vello::kurbo::StrokeOpts::default(), 0.1)
}

fn rect_path(a: Point, b: Point) -> BezPath {
    Rect::from_points(a, b).to_path(0.1)
}

/// The parts of a silhouette's upper edge that face the sun, as a path to
/// stroke with rim light. `pts` run left to right along the top.
fn sunny_edges(pts: &[Point], min: f64) -> BezPath {
    let mut path = BezPath::new();
    let mut open = false;
    for pair in pts.windows(2) {
        let d = pair[1] - pair[0];
        let len = d.hypot();
        if len < 1e-6 {
            continue;
        }
        let n = Vec2::new(d.y, -d.x) / len;
        if n.dot(SUN_SCREEN_DIR) > min {
            if !open {
                path.move_to(pair[0]);
                open = true;
            }
            path.line_to(pair[1]);
        } else {
            open = false;
        }
    }
    path
}

/// A soft round glow.
fn glow(scene: &mut Scene, at: Point, r: f64, color: Color, alpha: f32) {
    if r < 0.5 || alpha <= 0.0 {
        return;
    }
    let g = Gradient::new_radial(at, r as f32).with_stops([(0.0, color.with_alpha(alpha)), (0.4, color.with_alpha(alpha * 0.35)), (1.0, color.with_alpha(0.0))]);
    scene.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &Circle::new(at, r));
}

/// A puff of steam: rises, grows and fades over `phase` 0..1; lit pink on
/// the side facing the sun.
fn steam(scene: &mut Scene, at: Point, r: f64, phase: f64) {
    let alpha = (0.3 * (1.0 - phase) * (phase * 6.0).min(1.0)) as f32;
    if alpha <= 0.01 {
        return;
    }
    let lit = at + SUN_SCREEN_DIR * r * 0.45;
    let g = Gradient::new_two_point_radial(lit, 0.0_f32, at, r as f32).with_stops([
        (0.0, Color::from_rgb8(0xff, 0xc8, 0xb0).with_alpha(alpha)),
        (0.6, Color::from_rgb8(0xc8, 0x8a, 0xa0).with_alpha(alpha * 0.7)),
        (1.0, Color::from_rgb8(0x9a, 0x6a, 0x90).with_alpha(0.0)),
    ]);
    scene.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &Circle::new(at, r));
}

// ---------------------------------------------------------------------------
// Background

/// The far and middle distance: mesas, two skylines, an airship, the maglev
/// line, air traffic and the industry behind the roofs. `cam`, `w` and `h`
/// describe the (half-resolution) layer images.
pub fn draw_background(far: &mut Scene, mid: &mut Scene, cam: &Camera, w: f64, h: f64, time: f64) {
    let detail = crate::paint::detail();
    draw_mesas(far, cam, w, h);
    draw_skyline(far, cam, w, time, &SKYLINES[0]);
    if detail < 2 {
        draw_airship(far, cam, time);
    }
    draw_skyline(far, cam, w, time, &SKYLINES[1]);
    if detail < 2 {
        draw_traffic(mid, cam, w, time);
    }
    draw_maglev(mid, cam, w, time);
    draw_industry(mid, cam, w, h, time);
}

/// Height of the mesas: flat tops, steep flanks and terraces of harder
/// rock, with a ridged crust.
fn mesa_height(x: f64) -> f64 {
    let broad = fbm1(x * 0.0045, 3, 71) * 0.5 + 0.5;
    let t = ((broad - 0.42) * 4.5).clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t);
    let terraced = t * 0.7 + (t * 4.0).floor() / 4.0 * 0.3;
    -10.0 + 38.0 * terraced + 4.0 * ridge1(x * 0.02, 3, 72)
}

fn draw_mesas(scene: &mut Scene, cam: &Camera, w: f64, h: f64) {
    let z = 330.0;
    let (x0, x1) = cam.visible_x(z, w);
    let step = 3.0;
    let top: Vec<Point> = ((x0 / step).floor() as i64..=(x1 / step).ceil() as i64)
        .map(|i| {
            let x = i as f64 * step;
            cam.point(DVec3::new(x, mesa_height(x), z))
        })
        .collect();
    let mut body = BezPath::new();
    body.move_to(top[0]);
    for p in &top[1..] {
        body.line_to(*p);
    }
    body.line_to((w + 50.0, h + 50.0));
    body.line_to((-50.0, h + 50.0));
    body.close_path();
    let horizon = cam.point(DVec3::new(0.0, cam.eye.y, z)).y;
    let peak = cam.point(DVec3::new(0.0, cam.eye.y + 40.0, z)).y;
    let rock = Color::from_rgb8(0x8a, 0x48, 0x6c);
    let fill = Gradient::new_linear((0.0, peak), (0.0, horizon)).with_stops([mix(rock, HAZE, 0.35), mix(rock, HAZE, 0.8)]);
    scene.fill(Fill::NonZero, Affine::IDENTITY, &fill, None, &body);
    scene.stroke(&Stroke::new(1.3), Affine::IDENTITY, SUNLIT.with_alpha(0.6), None, &sunny_edges(&top, 0.3));
}

struct Skyline {
    z: f64,
    spacing: f64,
    height: f64,
    fog: f64,
    seed: u64,
    /// Neon advertising strips on some towers.
    ads: bool,
}

const SKYLINES: [Skyline; 2] = [
    Skyline { z: 210.0, spacing: 10.0, height: 58.0, fog: 0.62, seed: 5, ads: false },
    Skyline { z: 105.0, spacing: 8.0, height: 40.0, fog: 0.36, seed: 6, ads: true },
];

/// A tower's pieces in its own plane: quads (bottom left, bottom right, top
/// right, top left) and the tip, where a beacon blinks.
struct TowerShape {
    quads: Vec<[DVec2; 4]>,
    tip: DVec2,
}

fn tower_shape(x: f64, base: f64, width: f64, height: f64, i: i64, seed: u64) -> TowerShape {
    let p = DVec2::new;
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64| [p(x0, y0), p(x1, y0), p(x1, y1), p(x0, y1)];
    let top = base + height;
    let hw = width / 2.0;
    let mut quads = Vec::new();
    let tip;
    match (hash(i, seed + 5) * 5.0) as u32 {
        0 => {
            // Setbacks, not quite centred on each other.
            let tiers = 2 + (hash(i, seed + 6) * 3.0) as i64;
            let (mut y, mut half, mut cx) = (base, hw, x);
            for t in 0..tiers {
                let frac = if t + 1 == tiers { 1.0 } else { 0.5 + 0.25 * hash(i * 7 + t, seed + 7) };
                let y1 = y + (top - y) * frac;
                quads.push(rect(cx - half, y, cx + half, y1));
                y = y1;
                cx += (hash(i * 7 + t, seed + 8) - 0.5) * half * 0.5;
                half *= 0.55 + 0.25 * hash(i * 7 + t, seed + 9);
            }
            tip = p(cx, top);
        }
        1 => {
            // A tapering spire with a needle.
            quads.push([p(x - hw, base), p(x + hw, base), p(x + hw * 0.3, top), p(x - hw * 0.3, top)]);
            let needle = top + height * 0.22;
            quads.push([p(x - hw * 0.06, top), p(x + hw * 0.06, top), p(x + 0.03, needle), p(x - 0.03, needle)]);
            tip = p(x, needle);
        }
        2 => {
            // A slim shaft carrying a saucer deck.
            quads.push(rect(x - hw * 0.5, base, x + hw * 0.5, top));
            let deck = top - height * 0.16;
            let t = height * 0.045;
            quads.push([p(x - hw * 1.6, deck), p(x + hw * 1.6, deck), p(x + hw * 1.1, deck + t), p(x - hw * 1.1, deck + t)]);
            quads.push([p(x - hw * 1.0, deck - t * 0.8), p(x + hw * 1.0, deck - t * 0.8), p(x + hw * 1.6, deck), p(x - hw * 1.6, deck)]);
            tip = p(x, top);
        }
        3 => {
            // A roof sloping up towards the sun.
            let drop = width * (0.5 + 0.9 * hash(i, seed + 10));
            quads.push([p(x - hw, base), p(x + hw, base), p(x + hw, top), p(x - hw, top - drop)]);
            tip = p(x + hw, top);
        }
        _ => {
            // Twin towers with a skybridge.
            let gap = width * 0.16;
            let second = height * (0.7 + 0.2 * hash(i, seed + 11));
            quads.push(rect(x - hw, base, x - gap, top));
            quads.push(rect(x + gap, base, x + hw, base + second));
            let bridge = base + second * 0.72;
            quads.push(rect(x - gap, bridge, x + gap, bridge + height * 0.035));
            tip = p(x - (hw + gap) / 2.0, top);
        }
    }
    TowerShape { quads, tip }
}

fn draw_skyline(scene: &mut Scene, cam: &Camera, w: f64, time: f64, sky: &Skyline) {
    let (x0, x1) = cam.visible_x(sky.z, w);
    let base = -30.0;
    let s = sky.seed;
    let pt = |q: DVec2| cam.point(DVec3::new(q.x, q.y, sky.z));
    let mut bodies = BezPath::new();
    let mut lit = BezPath::new();
    let mut windows = BezPath::new();
    let mut beacons = Vec::new();
    let mut ads = Vec::new();
    for i in (x0 / sky.spacing).floor() as i64 - 2..=(x1 / sky.spacing).ceil() as i64 + 2 {
        if hash(i, s) < 0.18 {
            continue;
        }
        let x = (i as f64 + 0.5 * hash(i, s + 1)) * sky.spacing;
        // Districts: clusters of tall towers, and lower sprawl between.
        let district = fbm1(x * 0.005, 2, s + 2) * 0.5 + 0.5;
        let height = sky.height * (0.25 + 1.3 * district * district) * (0.45 + 0.75 * hash(i, s + 3));
        let width = sky.spacing * (0.35 + 0.5 * hash(i, s + 4));
        let shape = tower_shape(x, base, width, height, i, s);
        for (k, q) in shape.quads.iter().enumerate() {
            bodies.move_to(pt(q[0]));
            for c in &q[1..] {
                bodies.line_to(pt(*c));
            }
            bodies.close_path();
            // The side facing the sun: a lit sliver down the right.
            let (db, dt) = ((q[1] - q[0]) * 0.13, (q[2] - q[3]) * 0.13);
            lit.move_to(pt(q[1] - db));
            lit.line_to(pt(q[1]));
            lit.line_to(pt(q[2]));
            lit.line_to(pt(q[2] - dt));
            lit.close_path();
            // Lit windows, scattered.
            let (qy0, qy1) = (q[0].y.max(cam.eye.y - 40.0), q[3].y.min(q[2].y));
            let mut y = if crate::paint::detail() > 0 { qy1 } else { qy0 + 1.0 };
            let mut row = 0i64;
            while y < qy1 - 1.0 {
                let t = (y - q[0].y) / (q[3].y - q[0].y).max(1e-6);
                let (left, right) = (q[0].x + (q[3].x - q[0].x) * t, q[1].x + (q[2].x - q[1].x) * t);
                let mut wx = left + 0.6;
                let mut col = 0i64;
                while wx < right - 0.9 {
                    let key = i * 10_007 + k as i64 * 1_009 + row * 101 + col;
                    if hash(key, s + 20) < 0.14 + 0.12 * hash(i, s + 21) {
                        windows.move_to(pt(DVec2::new(wx, y)));
                        windows.line_to(pt(DVec2::new(wx + 0.5, y)));
                        windows.line_to(pt(DVec2::new(wx + 0.5, y + 0.7)));
                        windows.line_to(pt(DVec2::new(wx, y + 0.7)));
                        windows.close_path();
                    }
                    wx += 1.25;
                    col += 1;
                }
                y += 2.0;
                row += 1;
            }
        }
        // Aircraft warning lights blink on the tallest.
        if height > sky.height * 0.6 {
            let on = ((time * 0.8 + hash(i, s + 30) * 3.0).fract() < 0.18) as i32 as f32;
            beacons.push((pt(shape.tip), on));
        }
        if sky.ads && hash(i, s + 40) < 0.2 {
            let q = shape.quads[0];
            let (ax, ay) = (q[0].x + (q[1].x - q[0].x) * 0.2, q[0].y + (q[3].y - q[0].y) * (0.45 + 0.3 * hash(i, s + 41)));
            let color = if hash(i, s + 42) < 0.5 { NEON_PINK } else { NEON_CYAN };
            let flicker = if noise1(time * 3.0 + i as f64, s + 43) > 0.75 { 0.3 } else { 1.0 };
            ads.push((rect_path(pt(DVec2::new(ax, ay)), pt(DVec2::new(ax + width * 0.14, ay + height * 0.18))), color, flicker));
        }
    }
    scene.fill(Fill::NonZero, Affine::IDENTITY, mix(TOWER, HAZE, sky.fog), None, &bodies);
    scene.fill(Fill::NonZero, Affine::IDENTITY, mix(SUNLIT, HAZE, sky.fog * 0.6).with_alpha(0.9), None, &lit);
    scene.fill(Fill::NonZero, Affine::IDENTITY, mix(WINDOW, HAZE, sky.fog * 0.5).with_alpha(0.85), None, &windows);
    let px = cam.project(DVec3::new(0.0, 0.0, sky.z)).scale;
    for (path, color, flicker) in ads {
        scene.fill(Fill::NonZero, Affine::IDENTITY, mix(color, HAZE, sky.fog * 0.4).with_alpha(0.85 * flicker), None, &path);
    }
    for (at, on) in beacons {
        glow(scene, at, 2.5 * px, Color::from_rgb8(0xff, 0x40, 0x40), 0.9 * on);
    }
}

/// A big, slow airship with a glowing billboard, drifting across the city.
fn draw_airship(scene: &mut Scene, cam: &Camera, time: f64) {
    let z = 160.0;
    let x = 420.0 - (time * 1.8).rem_euclid(560.0);
    let y = 34.0 + (time * 0.35).sin() * 0.6;
    let c = cam.project(DVec3::new(x, y, z));
    let s = c.scale;
    if c.pos.x < -40.0 * s || c.pos.x > cam.center.x * 2.0 + 40.0 * s {
        return;
    }
    let hull = Ellipse::new(c.pos, (15.0 * s, 3.8 * s), 0.0);
    let body = mix(TOWER, HAZE, 0.5);
    // Tail fins at the back (it flies to the left).
    let mut fins = BezPath::new();
    let tail = c.pos + Vec2::new(12.0 * s, 0.0);
    fins.move_to(tail + Vec2::new(-2.0 * s, -1.5 * s));
    fins.line_to(tail + Vec2::new(4.2 * s, -5.0 * s));
    fins.line_to(tail + Vec2::new(4.5 * s, -0.5 * s));
    fins.line_to(tail + Vec2::new(4.2 * s, 4.0 * s));
    fins.line_to(tail + Vec2::new(-2.0 * s, 1.5 * s));
    fins.close_path();
    scene.fill(Fill::NonZero, Affine::IDENTITY, darken(body, 0.1), None, &fins);
    scene.fill(Fill::NonZero, Affine::IDENTITY, body, None, &hull);
    // Sunlight on its upper right: a crescent.
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &hull);
    scene.fill(Fill::NonZero, Affine::IDENTITY, mix(SUNLIT, HAZE, 0.35), None, &hull);
    scene.fill(Fill::NonZero, Affine::IDENTITY, body, None, &Ellipse::new(c.pos + Vec2::new(-1.4 * s, 0.9 * s), (15.0 * s, 3.8 * s), 0.0));
    scene.pop_layer();
    // Gondola and billboard, with bars of text scrolling across.
    let board = Rect::new(c.pos.x - 5.0 * s, c.pos.y + 3.8 * s, c.pos.x + 5.0 * s, c.pos.y + 6.8 * s);
    scene.fill(Fill::NonZero, Affine::IDENTITY, darken(body, 0.3), None, &board.inflate(0.3 * s, 0.3 * s));
    let hue = if (time / 6.0).floor() as i64 % 2 == 0 { NEON_CYAN } else { NEON_PINK };
    scene.fill(Fill::NonZero, Affine::IDENTITY, mix(hue, HAZE, 0.3).with_alpha(0.75), None, &board);
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &board);
    let mut bars = BezPath::new();
    for k in 0..8i64 {
        let len = (1.0 + 2.0 * hash(k, 7)) * s;
        let bx = board.x1 - ((time * 3.0 * s + k as f64 * 2.6 * s) % (board.width() + 4.0 * s)) + 2.0 * s;
        let by = board.y0 + (0.6 + 1.1 * (k % 2) as f64) * s;
        bars.extend(rect_path(Point::new(bx, by), Point::new(bx + len, by + 0.5 * s)).iter());
    }
    scene.fill(Fill::NonZero, Affine::IDENTITY, Color::WHITE.with_alpha(0.7), None, &bars);
    scene.pop_layer();
    glow(scene, board.center(), 9.0 * s, hue, 0.25);
    let blink = ((time * 1.3).fract() < 0.15) as i32 as f32;
    glow(scene, c.pos + Vec2::new(-15.0 * s, 0.0), 2.0 * s, Color::from_rgb8(0xff, 0x50, 0x50), blink);
    glow(scene, tail + Vec2::new(4.3 * s, -4.8 * s), 1.6 * s, Color::from_rgb8(0x80, 0xff, 0x90), 1.0 - blink);
}

/// Hover cars in lanes at different heights and depths, each with a
/// headlight beam ahead, a red tail light and a thruster glow underneath.
fn draw_traffic(scene: &mut Scene, cam: &Camera, w: f64, time: f64) {
    // Height, depth and speed (negative: to the left) of each lane.
    const LANES: [(f64, f64, f64); 4] = [(12.5, 60.0, -17.0), (21.0, 52.0, 24.0), (17.0, 36.0, -21.0), (14.0, 29.0, 15.0)];
    let loop_len = 150.0;
    for (li, &(y, z, speed)) in LANES.iter().enumerate() {
        let (x0, x1) = cam.visible_x(z, w);
        let fog = ((z - 20.0) / 60.0).clamp(0.0, 0.6);
        let body = mix(Color::from_rgb8(0x24, 0x18, 0x30), HAZE, fog);
        let seed = 300 + li as u64;
        let cars = if crate::paint::detail() > 0 { 3 } else { 6 };
        for k in 0..cars {
            if hash(k, seed) < 0.3 {
                continue;
            }
            let offset = (time * speed + (k as f64 + 0.6 * hash(k, seed + 1)) * loop_len / 6.0).rem_euclid(loop_len);
            let mut rep = (x0 / loop_len).floor();
            while rep * loop_len < x1 + loop_len {
                let x = rep * loop_len + offset;
                rep += 1.0;
                if x < x0 - 5.0 || x > x1 + 5.0 {
                    continue;
                }
                let bob = (time * 1.7 + k as f64 * 2.3).sin() * 0.2;
                let c = cam.project(DVec3::new(x, y + bob + (hash(k, seed + 2) - 0.5) * 2.0, z));
                let s = c.scale;
                let dir = speed.signum();
                // The body: a low wedge with a canopy bump.
                let mut car = BezPath::new();
                let front = Vec2::new(dir * 1.1 * s, 0.0);
                car.move_to(c.pos - front + Vec2::new(0.0, 0.2 * s));
                car.line_to(c.pos + front + Vec2::new(0.0, 0.15 * s));
                car.line_to(c.pos + front * 1.1 + Vec2::new(0.0, -0.05 * s));
                car.quad_to(c.pos + front * 0.3 + Vec2::new(0.0, -0.55 * s), c.pos - front * 0.4 + Vec2::new(0.0, -0.35 * s));
                car.line_to(c.pos - front + Vec2::new(0.0, -0.15 * s));
                car.close_path();
                let beam_tip = c.pos + front * 6.0 + Vec2::new(0.0, 0.6 * s);
                let mut beam = BezPath::new();
                beam.move_to(c.pos + front);
                beam.line_to(beam_tip + Vec2::new(0.0, -0.9 * s));
                beam.line_to(beam_tip + Vec2::new(0.0, 0.9 * s));
                beam.close_path();
                let light = Gradient::new_linear(c.pos + front, beam_tip).with_stops([
                    mix(WINDOW, HAZE, fog).with_alpha(0.35),
                    mix(WINDOW, HAZE, fog).with_alpha(0.0),
                ]);
                scene.fill(Fill::NonZero, Affine::IDENTITY, &light, None, &beam);
                glow(scene, c.pos + Vec2::new(0.0, 0.35 * s), 0.9 * s, NEON_CYAN, 0.5);
                scene.fill(Fill::NonZero, Affine::IDENTITY, body, None, &car);
                scene.stroke(&Stroke::new(0.12 * s), Affine::IDENTITY, mix(SUNLIT, HAZE, fog).with_alpha(0.8), None, &sunny_edges(&[c.pos - front * 0.4 + Vec2::new(0.0, -0.35 * s), c.pos + front * 0.3 + Vec2::new(0.0, -0.5 * s), c.pos + front * 1.1], -0.2));
                glow(scene, c.pos - front, 0.6 * s, Color::from_rgb8(0xff, 0x30, 0x40), 0.9);
                glow(scene, c.pos + front, 0.5 * s, Color::from_rgb8(0xff, 0xf0, 0xd0), 0.9);
            }
        }
    }
}

/// An elevated maglev guideway on pylons, and a train gliding along it now
/// and then.
fn draw_maglev(scene: &mut Scene, cam: &Camera, w: f64, time: f64) {
    let z = 44.0;
    let y = 10.0;
    let (x0, x1) = cam.visible_x(z, w);
    let color = mix(Color::from_rgb8(0x30, 0x20, 0x3c), HAZE, 0.3);
    let p = |x: f64, y: f64| cam.point(DVec3::new(x, y, z));
    // Pylons, each a slightly different taper.
    let spacing = 26.0;
    let mut pylons = BezPath::new();
    let mut pylon_lit = BezPath::new();
    for i in (x0 / spacing).floor() as i64 - 1..=(x1 / spacing).ceil() as i64 + 1 {
        let x = i as f64 * spacing + 6.0 * hash(i, 401);
        let (top, bottom) = (0.6 + 0.3 * hash(i, 402), 1.2 + 0.6 * hash(i, 403));
        pylons.extend(poly(cam, &[DVec3::new(x - bottom, -30.0, z), DVec3::new(x + bottom, -30.0, z), DVec3::new(x + top, y, z), DVec3::new(x - top, y, z)]).iter());
        pylon_lit.extend(poly(cam, &[DVec3::new(x + bottom * 0.7, -30.0, z), DVec3::new(x + bottom, -30.0, z), DVec3::new(x + top, y, z), DVec3::new(x + top * 0.7, y, z)]).iter());
    }
    let beam = rect_path(p(x0, y), p(x1, y + 1.0));
    scene.fill(Fill::NonZero, Affine::IDENTITY, darken(color, 0.1), None, &pylons);
    scene.fill(Fill::NonZero, Affine::IDENTITY, mix(SUNLIT, HAZE, 0.35).with_alpha(0.7), None, &pylon_lit);
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &beam);
    let mut rim = BezPath::new();
    rim.move_to(p(x0, y + 1.0));
    rim.line_to(p(x1, y + 1.0));
    scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, mix(RIM, HAZE, 0.3), None, &rim);

    // A train every 20 s, in alternating directions.
    let period = 20.0;
    let cycle = (time / period).floor() as i64;
    let t = time - cycle as f64 * period;
    let dir = if cycle % 2 == 0 { 1.0 } else { -1.0 };
    let span = x1 - x0 + 120.0;
    let head = if dir > 0.0 { x0 - 60.0 + t * 40.0 } else { x1 + 60.0 - t * 40.0 };
    if t * 40.0 > span + 60.0 {
        return;
    }
    let (car_len, cars) = (7.0, 5);
    let s = cam.project(DVec3::new(head, y, z)).scale;
    let paint = mix(Color::from_rgb8(0xd8, 0xcc, 0xd8), HAZE, 0.3);
    for k in 0..cars {
        let back = head - dir * (k as f64 * (car_len + 0.4));
        let (a, b) = if dir > 0.0 { (back - car_len, back) } else { (back, back + car_len) };
        let bottom = y + 1.15;
        let top = y + 2.7;
        let mut car = BezPath::new();
        car.move_to(p(a, bottom));
        car.line_to(p(b, bottom));
        if k == 0 {
            // The nose: a long curve down to the rail.
            let (tip, root) = if dir > 0.0 { (b + 2.2, b) } else { (a - 2.2, a) };
            if dir > 0.0 {
                car.line_to(p(tip, bottom));
                car.quad_to(p(root + 0.6, top), p(root - 0.8, top));
                car.line_to(p(a, top));
            } else {
                car.line_to(p(b, top));
                car.line_to(p(root + 0.8, top));
                car.quad_to(p(root - 0.6, top), p(tip, bottom));
            }
        } else {
            car.line_to(p(b, top));
            car.line_to(p(a, top));
        }
        car.close_path();
        scene.fill(Fill::NonZero, Affine::IDENTITY, paint, None, &car);
        // A band of lit windows, and the sun along the roof.
        scene.fill(Fill::NonZero, Affine::IDENTITY, mix(WINDOW, HAZE, 0.25), None, &rect_path(p(a + 0.4, y + 1.9), p(b - 0.4, y + 2.3)));
        let mut roof = BezPath::new();
        roof.move_to(p(a, top));
        roof.line_to(p(b, top));
        scene.stroke(&Stroke::new(0.25 * s), Affine::IDENTITY, mix(RIM, HAZE, 0.2), None, &roof);
    }
    // A glow on the rail under the train.
    let tail = head - dir * cars as f64 * (car_len + 0.4);
    let g = Gradient::new_linear(p(tail, y), p(head, y)).with_stops([NEON_CYAN.with_alpha(0.0), NEON_CYAN.with_alpha(0.5)]);
    scene.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &rect_path(p(tail.min(head), y + 0.9), p(tail.max(head), y + 1.15)));
}

/// Industry behind the roofs: water tanks, cooling towers, cranes and stacks,
/// on rubble whose line is never straight.
fn draw_industry(scene: &mut Scene, cam: &Camera, w: f64, h: f64, time: f64) {
    let z = 20.0;
    let (x0, x1) = cam.visible_x(z, w);
    let dark = mix(Color::from_rgb8(0x2c, 0x1a, 0x38), HAZE, 0.22);
    let lit_color = mix(SUNLIT, HAZE, 0.15);
    let p = |x: f64, y: f64| cam.point(DVec3::new(x, y, z));
    let s = cam.project(DVec3::new(0.0, 0.0, z)).scale;
    let ground = |x: f64| -3.0 + 1.6 * fbm1(x * 0.08, 4, 501);
    let mut body = BezPath::new();
    let mut lit = BezPath::new();
    let mut lines = BezPath::new();
    let mut lights = Vec::new();
    let mut puffs = Vec::new();

    // The rubble line along the bottom.
    let step = 0.8;
    let first = (x0 / step).floor() as i64;
    let last = (x1 / step).ceil() as i64;
    let tops: Vec<Point> = (first..=last).map(|i| p(i as f64 * step, ground(i as f64 * step))).collect();
    body.move_to(tops[0]);
    for q in &tops[1..] {
        body.line_to(*q);
    }
    body.line_to((w + 50.0, h + 50.0));
    body.line_to((-50.0, h + 50.0));
    body.close_path();
    lines.extend(sunny_edges(&tops, 0.35).iter());

    let spacing = 15.0;
    for i in (x0 / spacing).floor() as i64 - 1..=(x1 / spacing).ceil() as i64 + 1 {
        if hash(i, 510) < 0.25 {
            continue;
        }
        let x = (i as f64 + 0.3 * hash(i, 511)) * spacing;
        let g = ground(x) - 0.5;
        match (hash(i, 512) * 4.0) as u32 {
            0 => {
                // A water tower: a tank with a conical roof on braced legs.
                let (r, top) = (1.8 + hash(i, 513), 9.0 + 4.0 * hash(i, 514));
                let bottom = top - r * 1.4;
                for leg in [-0.9, -0.3, 0.3, 0.9] {
                    lines.move_to(p(x + leg * r * 1.1, g));
                    lines.line_to(p(x + leg * r * 0.9, bottom));
                }
                let bays = 3;
                for k in 0..bays {
                    let (ya, yb) = (g + (bottom - g) * k as f64 / bays as f64, g + (bottom - g) * (k + 1) as f64 / bays as f64);
                    lines.move_to(p(x - r, ya));
                    lines.line_to(p(x + r, yb));
                    lines.move_to(p(x + r, ya));
                    lines.line_to(p(x - r, yb));
                }
                body.extend(poly(cam, &[DVec3::new(x - r, bottom, z), DVec3::new(x + r, bottom, z), DVec3::new(x + r, top, z), DVec3::new(x, top + r * 0.55, z), DVec3::new(x - r, top, z)]).iter());
                lit.extend(poly(cam, &[DVec3::new(x + r * 0.72, bottom, z), DVec3::new(x + r, bottom, z), DVec3::new(x + r, top, z), DVec3::new(x, top + r * 0.55, z), DVec3::new(x + r * 0.2, top + r * 0.5, z), DVec3::new(x + r * 0.72, top, z)]).iter());
                lights.push((p(x, top + r * 0.6), Color::from_rgb8(0xff, 0x40, 0x40), (time * 0.9 + i as f64).fract() < 0.2));
            }
            1 => {
                // A cooling tower (hyperbolic), steaming.
                let (r, hgt) = (3.2 + hash(i, 515), 11.0 + 5.0 * hash(i, 516));
                let mut tower = BezPath::new();
                tower.move_to(p(x - r, g));
                tower.quad_to(p(x - r * 0.35, g + hgt * 0.6), p(x - r * 0.6, g + hgt));
                tower.line_to(p(x + r * 0.6, g + hgt));
                tower.quad_to(p(x + r * 0.35, g + hgt * 0.6), p(x + r, g));
                tower.close_path();
                body.extend(tower.iter());
                let mut side = BezPath::new();
                side.move_to(p(x + r * 0.7, g));
                side.quad_to(p(x + r * 0.2, g + hgt * 0.6), p(x + r * 0.45, g + hgt));
                side.line_to(p(x + r * 0.6, g + hgt));
                side.quad_to(p(x + r * 0.35, g + hgt * 0.6), p(x + r, g));
                side.close_path();
                lit.extend(side.iter());
                for k in 0..6 {
                    let phase = (time * 0.12 + k as f64 / 6.0 + hash(i, 517)).fract();
                    let at = p(x + phase * 5.0 - 0.5, g + hgt + phase * 9.0);
                    puffs.push((at, (1.6 + 3.5 * phase) * s, phase));
                }
            }
            2 => {
                // A tower crane, its jib slowly slewing round (seen side on,
                // the arm grows and shrinks as it turns).
                let mast = 16.0 + 6.0 * hash(i, 518);
                let (mx, top) = (x, g + mast);
                for side in [-0.5, 0.5] {
                    lines.move_to(p(mx + side, g));
                    lines.line_to(p(mx + side, top));
                }
                let mut y = g;
                let mut flip = -0.5;
                while y < top - 1.0 {
                    lines.move_to(p(mx + flip, y));
                    lines.line_to(p(mx - flip, y + 1.0));
                    y += 1.0;
                    flip = -flip;
                }
                let slew = (time * 0.05 + hash(i, 519) * TAU).cos();
                let (jib, counter) = (14.0 * slew, -4.5 * slew);
                lines.move_to(p(mx + counter, top));
                lines.line_to(p(mx + jib, top));
                lines.move_to(p(mx, top + 3.0));
                lines.line_to(p(mx + jib, top));
                lines.move_to(p(mx, top + 3.0));
                lines.line_to(p(mx + counter, top));
                body.extend(rect_path(p(mx + counter - 1.2 * slew.signum(), top - 1.2), p(mx + counter, top)).iter());
                // The hook, hanging from a trolley that runs along the jib.
                let trolley = mx + jib * (0.5 + 0.4 * (time * 0.1 + i as f64).sin());
                let hook = top - 6.0 - 3.0 * (time * 0.13 + i as f64).cos();
                lines.move_to(p(trolley, top));
                lines.line_to(p(trolley, hook));
                body.extend(rect_path(p(trolley - 0.6, hook - 0.8), p(trolley + 0.6, hook)).iter());
                lights.push((p(mx + jib, top), Color::from_rgb8(0xff, 0x40, 0x40), (time * 1.1 + i as f64 * 0.37).fract() < 0.15));
            }
            _ => {
                // A block of flats with windows lit here and there.
                let (hw, hgt) = (3.0 + 2.0 * hash(i, 520), 8.0 + 8.0 * hash(i, 521));
                let lean = (hash(i, 522) - 0.5) * 1.2;
                body.extend(poly(cam, &[DVec3::new(x - hw, g, z), DVec3::new(x + hw, g, z), DVec3::new(x + hw, g + hgt + lean, z), DVec3::new(x - hw, g + hgt - lean, z)]).iter());
                lit.extend(poly(cam, &[DVec3::new(x + hw * 0.82, g, z), DVec3::new(x + hw, g, z), DVec3::new(x + hw, g + hgt + lean, z), DVec3::new(x + hw * 0.82, g + hgt + lean * 0.82, z)]).iter());
                let mut wy = g + 1.2;
                let mut row = 0;
                while wy < g + hgt - 1.5 {
                    let mut wx = x - hw + 0.8;
                    let mut col = 0;
                    while wx < x + hw * 0.7 {
                        if hash(i * 1000 + row * 31 + col, 523) < 0.25 {
                            lights.push((p(wx + 0.3, wy + 0.4), WINDOW, true));
                        }
                        wx += 1.3;
                        col += 1;
                    }
                    wy += 1.8;
                    row += 1;
                }
                // A stack on the roof, smoking.
                if hash(i, 524) < 0.6 {
                    let sx = x - hw * 0.5;
                    let top = g + hgt + 5.0;
                    body.extend(poly(cam, &[DVec3::new(sx - 0.6, g + hgt, z), DVec3::new(sx + 0.6, g + hgt, z), DVec3::new(sx + 0.4, top, z), DVec3::new(sx - 0.4, top, z)]).iter());
                    for k in 0..5 {
                        let phase = (time * 0.15 + k as f64 / 5.0 + hash(i, 525)).fract();
                        puffs.push((p(sx + phase * 4.0, top + phase * 7.0), (0.7 + 2.5 * phase) * s, phase));
                    }
                }
            }
        }
    }
    scene.fill(Fill::NonZero, Affine::IDENTITY, dark, None, &body);
    scene.fill(Fill::NonZero, Affine::IDENTITY, lit_color.with_alpha(0.75), None, &lit);
    scene.stroke(&Stroke::new((0.22 * s).max(1.0)), Affine::IDENTITY, dark, None, &lines);
    if crate::paint::detail() >= 2 {
        return;
    }
    for (at, r, phase) in puffs {
        steam(scene, at, r, phase);
    }
    for (at, color, on) in lights {
        if on {
            let r = if color == WINDOW { 0.35 * s } else { 1.4 * s };
            if color == WINDOW {
                scene.fill(Fill::NonZero, Affine::IDENTITY, mix(WINDOW, HAZE, 0.2), None, &Rect::from_center_size(at, (0.55 * s, 0.75 * s)));
            } else {
                glow(scene, at, r, color, 0.9);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shadows

/// A shadow caster: convex pieces, each given as points whose hull is the
/// piece (a box's corners, a limb's ends puffed out by its thickness).
pub type Caster = Vec<Vec<DVec3>>;

/// Points of a ball, for a caster piece.
fn ball(c: DVec3, r: f64) -> [DVec3; 6] {
    [c + DVec3::X * r, c - DVec3::X * r, c + DVec3::Y * r, c - DVec3::Y * r, c + DVec3::Z * r, c - DVec3::Z * r]
}

fn box_corners(x0: f64, x1: f64, y0: f64, y1: f64, z0: f64, z1: f64) -> Vec<DVec3> {
    let mut v = Vec::with_capacity(8);
    for x in [x0, x1] {
        for y in [y0, y1] {
            for z in [z0, z1] {
                v.push(DVec3::new(x, y, z));
            }
        }
    }
    v
}

fn convex_hull(mut pts: Vec<DVec2>) -> Vec<DVec2> {
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup_by(|a, b| (*a - *b).length_squared() < 1e-12);
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: DVec2, a: DVec2, b: DVec2| (a - o).perp_dot(b - o);
    let mut hull: Vec<DVec2> = Vec::with_capacity(pts.len() * 2);
    for &p in &pts {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
            hull.pop();
        }
        hull.push(p);
    }
    let lower = hull.len() + 1;
    for &p in pts.iter().rev().skip(1) {
        while hull.len() >= lower && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
            hull.pop();
        }
        hull.push(p);
    }
    hull.pop();
    hull
}

/// The shadow `caster` throws onto the horizontal plane at height `ground`,
/// on screen: each piece's points slid along the sunlight down to the
/// plane, and the hull of them. Parts below the plane cast nothing.
fn shadow_path(cam: &Camera, caster: &[Vec<DVec3>], ground: f64) -> BezPath {
    let l = sun_dir();
    let mut path = BezPath::new();
    for piece in caster {
        if piece.iter().all(|p| p.y <= ground + 0.02) {
            continue;
        }
        let pts = piece
            .iter()
            .map(|p| {
                let y = p.y.max(ground);
                let q = DVec3::new(p.x, y, p.z) - l * ((y - ground) / l.y);
                DVec2::new(q.x, q.z)
            })
            .collect();
        let hull = convex_hull(pts);
        if hull.len() < 3 {
            continue;
        }
        for (i, q) in hull.iter().enumerate() {
            let s = cam.point(DVec3::new(q.x, ground, q.y));
            if i == 0 { path.move_to(s) } else { path.line_to(s) }
        }
        path.close_path();
    }
    path
}

/// How far to the left a caster of this height can throw its shadow.
fn shadow_reach(height: f64) -> f64 {
    let l = sun_dir();
    height.max(0.0) * l.x / l.y + 1.0
}

/// Draws shadows onto a platform's top (clipped to it), just above it in
/// the drawing order. `fade` runs from `(start, end)` on screen, dark to
/// lighter, as the shadow softens away from its caster.
fn push_shadow(canvas: &mut Canvas3d, b: &Block, path: BezPath, fade: Option<(Point, Point)>) {
    if path.elements().is_empty() || canvas.camera.eye.y <= b.y1 {
        return;
    }
    let top = top_face(&canvas.camera, b);
    let depth = block_depth(&canvas.camera, b) - 0.004;
    canvas.push(depth, move |scene| {
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &top);
        match fade {
            Some((a, e)) => {
                let g = Gradient::new_linear(a, e).with_stops([SHADOW.with_alpha(0.72), SHADOW.with_alpha(0.38)]);
                scene.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &path);
            }
            None => scene.fill(Fill::NonZero, Affine::IDENTITY, SHADOW.with_alpha(0.62), None, &path),
        }
        scene.pop_layer();
    });
}

/// The block a thing standing at `(x, y)` stands on.
fn block_under(level: &Level, x: f64, y: f64) -> Option<&Block> {
    level.blocks.iter().find(|b| b.x0 <= x && x <= b.x1 && (b.y1 - y).abs() < 0.05)
}

/// Konrad's long shadow, cast from his posed skeleton onto every roof it
/// reaches: it runs, jumps and waves its hair along with him.
pub fn draw_body_shadow(canvas: &mut Canvas3d, level: &Level, s: &Solved) {
    use crate::konrad::bone::*;
    // Seen from a low camera the shadow's width (along z) is squashed thin,
    // so it's cast a little fatter than he is, to read.
    let fat = |pts: [DVec3; 6], c: DVec3| pts.map(|p| c + (p - c) * DVec3::new(1.0, 1.0, 2.2));
    let ball = |c: DVec3, r: f64| fat(ball(c, r), c);
    let limb = |a: usize, b: usize, r: f64| {
        let mut v = ball(s.pos[a], r).to_vec();
        v.extend(ball(s.pos[b], r));
        v
    };
    let head = s.at(HEAD, DVec3::new(0.0, 0.1, 0.02));
    let hair = s.at(HEAD, DVec3::new(0.0, 0.22, -0.06));
    let caster: Caster = vec![
        limb(PELVIS, CHEST, 0.17),
        limb(CHEST, NECK, 0.14),
        limb(HIP_L, HIP_R, 0.14),
        ball(head, 0.13).to_vec(),
        ball(hair, 0.3).to_vec(),
        limb(SHOULDER_L, SHOULDER_R, 0.08),
        limb(SHOULDER_L, ELBOW_L, 0.065),
        limb(ELBOW_L, HAND_L, 0.055),
        limb(SHOULDER_R, ELBOW_R, 0.065),
        limb(ELBOW_R, HAND_R, 0.055),
        limb(HIP_L, KNEE_L, 0.085),
        limb(KNEE_L, FOOT_L, 0.07),
        limb(HIP_R, KNEE_R, 0.085),
        limb(KNEE_R, FOOT_R, 0.07),
        limb(FOOT_L, FOOT_L, 0.07).into_iter().map(|p| p + s.root.rot * DVec3::Z * 0.1).collect(),
        limb(FOOT_R, FOOT_R, 0.07).into_iter().map(|p| p + s.root.rot * DVec3::Z * 0.1).collect(),
    ];
    let feet = s.pos[FOOT_L].lerp(s.pos[FOOT_R], 0.5);
    let reach = shadow_reach(hair.y + 0.3 - feet.y + 12.0);
    let cam = canvas.camera;
    let receivers: Vec<Block> = level
        .blocks
        .iter()
        .filter(|b| b.y1 <= feet.y + 0.05 && b.x1 > feet.x - reach && b.x0 < feet.x + 1.0)
        .copied()
        .collect();
    let l = sun_dir();
    for b in receivers {
        let path = shadow_path(&cam, &caster, b.y1);
        let slide = |p: DVec3| {
            let y = p.y.max(b.y1);
            cam.point(DVec3::new(p.x, y, p.z) - l * ((y - b.y1) / l.y))
        };
        push_shadow(canvas, &b, path, Some((slide(feet), slide(hair))));
    }
}

// ---------------------------------------------------------------------------
// The world

pub fn draw_world(canvas: &mut Canvas3d, level: &Level, view: &WorldView) {
    let cam = canvas.camera;
    let (vx0, vx1) = cam.visible_x(-1.5, view.screen_width);
    // Things to the right, out of view, can still throw shadows into it.
    let near = |x0: f64, x1: f64, extra: f64| x1 > vx0 - 2.0 && x0 < vx1 + 2.0 + extra;
    let mut rubble: Vec<Caster> = vec![Vec::new(); level.blocks.len()];
    for (i, b) in level.blocks.iter().enumerate() {
        if near(b.x0, b.x1, 0.0) {
            rubble[i] = draw_block(canvas, &level.blocks, b, (vx0, vx1), view.time);
        }
    }
    // Long shadows onto every roof in view.
    for (ri, r) in level.blocks.iter().enumerate() {
        if !near(r.x0, r.x1, 0.0) || cam.eye.y <= r.y1 {
            continue;
        }
        let mut caster: Caster = std::mem::take(&mut rubble[ri]);
        for c in &level.blocks {
            if c.y1 <= r.y1 + 0.05 || c.x1 < r.x0 || c.x0 > r.x1 + shadow_reach(c.y1 - r.y1) {
                continue;
            }
            caster.push(box_corners(c.x0, c.x1, c.y0.max(r.y1), c.y1, c.z0, c.z1));
        }
        for prop in &level.props {
            if prop.y >= r.y1 - 0.05 && prop.x > r.x0 - 1.0 && prop.x < r.x1 + shadow_reach(prop.y + 6.0 - r.y1) {
                caster.extend(prop_caster(prop, view.time));
            }
        }
        let path = shadow_path(&cam, &caster, r.y1);
        push_shadow(canvas, r, path, None);
    }
    for prop in level.props.iter().filter(|p| near(p.x, p.x, 0.0)) {
        draw_prop(canvas, level, prop, view.time);
    }
    for (i, &f) in view.flies.iter().enumerate() {
        if !view.caught[i] {
            draw_cell(canvas, f, view.time, i);
        }
    }
    for (i, &c) in level.checkpoints.iter().enumerate().skip(1) {
        draw_beacon(canvas, c, i <= view.checkpoint, view.time);
    }
    draw_teleporter(canvas, level.goal, view.time);
    let detail = crate::paint::detail();
    if detail < 1 {
        draw_motes(canvas, view.screen_width, view.time);
    }
    if detail < 2 {
        draw_foreground(canvas, view.screen_width, view.time);
    }
}

/// Draws a platform: a building's roof and facade, a cargo container, a
/// steel catwalk or a plinth. Returns the rubble lying on its top, which
/// casts little shadows of its own.
fn draw_block(canvas: &mut Canvas3d, blocks: &[Block], b: &Block, visible: (f64, f64), time: f64) -> Caster {
    let cam = canvas.camera;
    let y0 = b.y0.max(cam.eye.y - 25.0);
    let p = |x: f64, y: f64, z: f64| cam.point(DVec3::new(x, y, z));
    let seed = crate::paint::seed(&[b.x0, b.y1]);
    let building = b.y0 < -20.0;
    let catwalk = b.kind == BlockKind::Log;
    let plinth = !building && !catwalk && b.y1 - b.y0 < 0.7;
    let glass = building && b.kind == BlockKind::Stone;
    // Small clutter goes first on slow devices.
    let clutter = crate::paint::detail() == 0;
    let paint = PAINTS[(hash(seed as i64, 3) * 4.0) as usize % 4];
    let (top, front, lit_side, dark_side) = if building {
        (CONCRETE_TOP, if glass { GLASS } else { CONCRETE_FRONT }, CONCRETE_LIT, CONCRETE_DARK)
    } else if catwalk || plinth {
        (STEEL_TOP, STEEL, mix(STEEL, SUNLIT, 0.55), darken(STEEL, 0.3))
    } else {
        (mix(paint, SUNLIT, 0.35), darken(mix(paint, CONCRETE_FRONT, 0.35), 0.1), mix(paint, SUNLIT, 0.6), darken(paint, 0.45))
    };
    let scale = cam.project(DVec3::new((b.x0 + b.x1) / 2.0, b.y1, b.z0)).scale;
    let depth = block_depth(&cam, b);
    let width = b.x1 - b.x0;

    // Broken corners: how far the facade's edge is bitten in at height y
    // (the roof's edge itself stays whole, to stand on).
    let chip = |side: u64, y: f64| -> f64 {
        if !building || glass {
            return 0.0;
        }
        let ramp = ((b.y1 - y - 0.3) / 1.2).clamp(0.0, 1.0);
        let n = fbm1(y * 0.45 + side as f64 * 17.0, 3, seed ^ side) * 0.5 + 0.5;
        (n * n * n * 2.6 * ramp).min(width * 0.3)
    };
    let mut ys = vec![b.y1];
    let mut y = b.y1 - 0.3;
    while y > y0 {
        ys.push(y);
        y -= 0.4;
    }
    ys.push(y0);

    // The roof's front edge crumbles: chunks are broken out of it, deeper
    // here and there. `bite(x)` is how far back the top is broken (in
    // depth); the facade's top edge drops by about as much.
    let (vis0, vis1) = (b.x0.max(visible.0 - 2.0), b.x1.min(visible.1 + 2.0));
    let bite = |x: f64| -> f64 {
        if !building || glass {
            return 0.0;
        }
        let n = fbm1(x * 0.8, 3, seed ^ 80) * 0.5 + 0.5;
        let big = ((n - 0.6) / 0.2).clamp(0.0, 1.0);
        let fine = (noise1(x * 4.0, seed ^ 81) * 0.5 + 0.5) * 0.05;
        // The corners are left to the chips.
        let fade = ((x - b.x0).min(b.x1 - x) / 0.6).clamp(0.0, 1.0);
        (big * big * 0.34 + fine) * fade
    };
    let mut edge_xs = vec![b.x0];
    if building && vis1 > vis0 {
        let mut x = (vis0 / 0.18).ceil() * 0.18;
        while x < vis1 {
            if x > b.x0 && x < b.x1 {
                edge_xs.push(x);
            }
            x += 0.18;
        }
    }
    edge_xs.push(b.x1);
    let top_edge: Vec<Point> = edge_xs.iter().map(|&x| p(x, b.y1, b.z0 + bite(x))).collect();
    let front_edge: Vec<Point> = edge_xs.iter().map(|&x| p(x, b.y1 - bite(x) * 1.1, b.z0)).collect();
    // The broken faces between the two.
    let mut broken = BezPath::new();
    if building {
        broken.move_to(top_edge[0]);
        for q in &top_edge[1..] {
            broken.line_to(*q);
        }
        for q in front_edge.iter().rev() {
            broken.line_to(*q);
        }
        broken.close_path();
    }

    // The front face, with its corners chipped away.
    let mut front_face = BezPath::new();
    front_face.move_to(front_edge[0]);
    for q in &front_edge[1..] {
        front_face.line_to(*q);
    }
    for &y in &ys[1..] {
        front_face.line_to(p(b.x1 - chip(1, y), y, b.z0));
    }
    for &y in ys[1..].iter().rev() {
        front_face.line_to(p(b.x0 + chip(0, y), y, b.z0));
    }
    front_face.close_path();

    // The sides: the one facing the sun is lit, down to where the next
    // building's shadow cuts across it.
    let mut sides: Vec<(BezPath, Gradient)> = Vec::new();
    for (x, side, sunny) in [(b.x1, 1u64, true), (b.x0, 0u64, false)] {
        let seen = if sunny { cam.eye.x > b.x1 } else { cam.eye.x < b.x0 };
        if !seen {
            continue;
        }
        let inward = if sunny { -1.0 } else { 1.0 };
        let mut face = BezPath::new();
        for (k, &y) in ys.iter().enumerate() {
            let q = p(x + inward * chip(side, y), y, b.z0);
            if k == 0 { face.move_to(q) } else { face.line_to(q) }
        }
        for &y in ys.iter().rev() {
            face.line_to(p(x + inward * chip(side, y) * 0.3, y, b.z1));
        }
        face.close_path();
        let a = p(x, b.y1, b.z0);
        let e = p(x, y0.min(b.y1 - 0.01), b.z0);
        let fill = if sunny {
            let cut = sun_line(blocks, b);
            let t = ((b.y1 - cut) / (b.y1 - y0).max(0.01)).clamp(0.0, 1.0) as f32;
            if t <= 0.0 {
                Gradient::new_linear(a, e).with_stops([dark_side, darken(dark_side, 0.3)])
            } else {
                Gradient::new_linear(a, e).with_stops([
                    (0.0, lighten(lit_side, 0.15)),
                    (t * 0.999, lit_side),
                    (t, dark_side),
                    (1.0, darken(dark_side, 0.3)),
                ])
            }
        } else {
            Gradient::new_linear(a, e).with_stops([dark_side, darken(dark_side, 0.35)])
        };
        sides.push((face, fill));
    }

    // Facade details, clipped to the front face.
    let mut dark_lines = BezPath::new();
    let mut frames = BezPath::new();
    let mut sills = BezPath::new();
    let mut vines = BezPath::new();
    let mut leaves = BezPath::new();
    let mut lit_leaves = BezPath::new();
    let mut streaks: Vec<(BezPath, Gradient)> = Vec::new();
    let mut lit_windows = BezPath::new();
    let mut cyan_windows = BezPath::new();
    let mut dark_windows = BezPath::new();
    let mut boxes = BezPath::new();
    let mut fans = BezPath::new();
    let mut neon: Vec<(BezPath, Color, f32)> = Vec::new();
    let mut rebar = BezPath::new();
    let mut steel_rims = BezPath::new();
    let mut lamps: Vec<(Point, Color, f32)> = Vec::new();
    let (vx0, vx1) = (b.x0.max(visible.0 - 2.0), b.x1.min(visible.1 + 2.0));
    if building {
        // Floors of windows; a few lit, some flickering.
        let (col_step, row_step) = if glass { (1.25, 1.5) } else { (1.9, 2.6) };
        let mut row = 0i64;
        let mut wy = b.y1 - 1.5;
        while wy - 1.2 > y0 && row < 12 {
            let mut wx = b.x0 + 1.0 + (hash(row, seed) - 0.5) * 0.4;
            let mut col = 0i64;
            while wx + 1.0 < b.x1 - 0.8 {
                let key = row * 1000 + col;
                let inside = wx > b.x0 + chip(0, wy - 0.6) + 0.4 && wx + 1.0 < b.x1 - chip(1, wy - 0.6) - 0.4;
                if inside && wx > vx0 - 1.0 && wx < vx1 + 1.0 {
                    let (ww, wh) = if glass { (1.05, 1.25) } else { (0.95, 1.15) };
                    let r = rect_path(p(wx, wy - wh, b.z0), p(wx + ww, wy, b.z0));
                    if !glass {
                        frames.move_to(p(wx + ww * 0.5, wy, b.z0));
                        frames.line_to(p(wx + ww * 0.5, wy - wh, b.z0));
                        frames.move_to(p(wx, wy - wh * 0.38, b.z0));
                        frames.line_to(p(wx + ww, wy - wh * 0.38, b.z0));
                        frames.extend(rect_path(p(wx, wy - wh, b.z0), p(wx + ww, wy, b.z0)).iter());
                        sills.extend(rect_path(p(wx - 0.06, wy - wh - 0.09, b.z0), p(wx + ww + 0.06, wy - wh, b.z0)).iter());
                    }
                    let h = hash(key, seed ^ 11);
                    let flicker = h < 0.04 && noise1(time * 0.8 + key as f64, seed) > 0.2;
                    if (h < 0.26 && !flicker) || (h < 0.04 && !flicker) {
                        if hash(key, seed ^ 12) < 0.15 {
                            cyan_windows.extend(r.iter());
                        } else {
                            lit_windows.extend(r.iter());
                        }
                    } else {
                        dark_windows.extend(r.iter());
                    }
                    // Grime running down from the sill.
                    if clutter && !glass && hash(key, seed ^ 13) < 0.35 {
                        let sx = wx + 0.2 + 0.5 * hash(key, seed ^ 14);
                        let len = 0.5 + 1.8 * hash(key, seed ^ 15);
                        let streak = rect_path(p(sx, wy - wh - len, b.z0), p(sx + 0.07 + 0.08 * hash(key, seed ^ 16), wy - wh, b.z0));
                        let g = Gradient::new_linear(p(sx, wy - wh, b.z0), p(sx, wy - wh - len, b.z0))
                            .with_stops([CONCRETE_DARK.with_alpha(0.45), CONCRETE_DARK.with_alpha(0.0)]);
                        streaks.push((streak, g));
                    }
                }
                wx += col_step + if glass { 0.0 } else { (hash(key, seed ^ 17) - 0.5) * 0.3 };
                col += 1;
            }
            // The floor slab: a seam, not quite level.
            if !glass {
                let sy = wy - 1.45;
                let n = ((b.x1 - b.x0) / 1.5).ceil() as i64;
                for k in 0..=n {
                    let x = b.x0 + (b.x1 - b.x0) * k as f64 / n as f64;
                    let q = p(x, sy + noise1(x * 0.7, seed ^ row as u64) * 0.06, b.z0);
                    if k == 0 { dark_lines.move_to(q) } else { dark_lines.line_to(q) }
                }
            }
            wy -= row_step;
            row += 1;
        }
        if glass {
            // Mullions between the panes of a curtain wall.
            let mut x = b.x0 + 0.95;
            while x < b.x1 {
                dark_lines.move_to(p(x, b.y1 - 0.3, b.z0));
                dark_lines.line_to(p(x, y0, b.z0));
                x += 1.25;
            }
        }
        // Air conditioners with spinning fans, a downpipe, a neon sign.
        let mut k = (vx0 / 9.0).floor() as i64;
        while (k as f64) * 9.0 < vx1 {
            let x = k as f64 * 9.0 + 4.0 * hash(k, seed ^ 20);
            let floor = (hash(k, seed ^ 21) * 3.0).floor();
            let y = b.y1 - 2.6 - floor * 2.6;
            let fits = x > b.x0 + chip(0, y) + 0.8 && x + 0.9 < b.x1 - chip(1, y) - 0.8 && y - 0.6 > y0;
            if fits && !glass {
                match (hash(k, seed ^ 22) * 3.0) as u32 {
                    0 | 1 => {
                        boxes.extend(rect_path(p(x, y - 0.65, b.z0 - 0.02), p(x + 0.9, y, b.z0 - 0.02)).iter());
                        let c = p(x + 0.45, y - 0.32, b.z0 - 0.03);
                        let r = 0.24 * scale;
                        let spin = time * (9.0 + 4.0 * hash(k, seed ^ 23)) + k as f64;
                        for blade in 0..3 {
                            let a = spin + blade as f64 * TAU / 3.0;
                            let tip = c + Vec2::new(a.cos(), a.sin()) * r;
                            let side = Vec2::new(-a.sin(), a.cos()) * r * 0.28;
                            fans.move_to(c);
                            fans.quad_to(c + (tip - c) * 0.5 + side, tip);
                            fans.line_to(c + (tip - c) * 0.8 - side * 0.4);
                            fans.close_path();
                        }
                        steel_rims.extend(Circle::new(c, r).to_path(0.1).iter());
                    }
                    _ => {
                        // A vertical neon sign jutting out from the wall.
                        let color = if hash(k, seed ^ 24) < 0.5 { NEON_PINK } else { NEON_CYAN };
                        let on = if noise1(time * 2.0 + k as f64 * 3.0, seed ^ 25) > 0.7 { 0.25 } else { 1.0 };
                        let sign = rect_path(p(x + 0.2, y - 2.2, b.z0 - 0.3), p(x + 0.65, y, b.z0 - 0.3));
                        let mut glyphs = BezPath::new();
                        for g in 0..5 {
                            let gy = y - 0.3 - g as f64 * 0.4;
                            let gw = 0.1 + 0.2 * hash(k * 10 + g, seed ^ 26);
                            glyphs.extend(rect_path(p(x + 0.28, gy - 0.22, b.z0 - 0.31), p(x + 0.28 + gw, gy, b.z0 - 0.31)).iter());
                        }
                        neon.push((sign, color, on));
                        neon.push((glyphs, Color::WHITE, on * 0.9));
                        lamps.push((p(x + 0.42, y - 1.1, b.z0 - 0.3), color, on * 0.5));
                    }
                }
            }
            if hash(k, seed ^ 27) < 0.3 && !glass {
                let px = x + 3.0;
                if px > b.x0 + 1.0 && px < b.x1 - 1.0 {
                    boxes.extend(rect_path(p(px, y0, b.z0 - 0.05), p(px + 0.14, b.y1 - 0.25, b.z0 - 0.05)).iter());
                }
            }
            k += 1;
        }
        // Creepers hanging from the parapet, swaying a little.
        for i in (vis0 / 2.1).floor() as i64..=(vis1 / 2.1).ceil() as i64 {
            let hs = seed ^ 90;
            if !clutter || hash(i, hs) < 0.62 {
                continue;
            }
            let x0 = (i as f64 + hash(i, hs + 1)) * 2.1;
            if x0 < b.x0 + 0.5 || x0 > b.x1 - 0.5 {
                continue;
            }
            let len = 0.8 + 3.4 * hash(i, hs + 2) * hash(i, hs + 3);
            let top = b.y1 - bite(x0) * 1.1 - 0.05;
            let steps = (len / 0.12).ceil() as i64;
            for k in 0..=steps {
                let t = k as f64 / steps as f64;
                let sway = (time * 0.9 + x0).sin() * 0.12 * t * t;
                let at = DVec3::new(x0 + noise1(t * len * 1.8, hs + 4) * 0.18 * t + sway, top - t * len, b.z0 - 0.03);
                let q = cam.point(at);
                if k == 0 { vines.move_to(q) } else { vines.line_to(q) }
                if k % 2 == 1 {
                    let side = if k % 4 == 1 { 1.0 } else { -1.0 };
                    let c = cam.point(at + DVec3::new(side * 0.07, -0.02, 0.0));
                    let leaf = Ellipse::new(c, (0.075 * scale, 0.04 * scale), side * 0.5).to_path(0.1);
                    if side > 0.0 && hash(i * 100 + k, hs + 5) < 0.5 { lit_leaves.extend(leaf.iter()) } else { leaves.extend(leaf.iter()) }
                }
            }
        }
        // Rebar sticking out of the broken edges.
        for (side, x, out) in [(0u64, b.x0, -1.0), (1u64, b.x1, 1.0)] {
            for (i, &y) in ys.iter().enumerate().skip(1) {
                let bite = chip(side, y);
                if bite > 0.35 && hash(i as i64, seed ^ (30 + side)) < 0.5 {
                    let edge = x - out * bite;
                    let len = bite * (0.4 + 0.6 * hash(i as i64, seed ^ (32 + side)));
                    let droop = (hash(i as i64, seed ^ (34 + side)) - 0.5) * 0.4;
                    rebar.move_to(p(edge, y, b.z0 + 0.1));
                    rebar.quad_to(p(edge + out * len * 0.6, y + 0.05, b.z0 + 0.1), p(edge + out * len, y + droop, b.z0 + 0.1));
                }
            }
        }
    } else if catwalk {
        // Lightening holes along the beam, and amber marker lights that
        // chase along it.
        let mut x = (vx0 / 0.8).ceil() * 0.8;
        let yc = (b.y0 + b.y1) / 2.0;
        while x < vx1 && x < b.x1 - 0.2 {
            if x > b.x0 + 0.2 {
                let c = cam.project(DVec3::new(x, yc, b.z0));
                dark_windows.extend(Ellipse::new(c.pos, (0.1 * c.scale, 0.08 * c.scale), 0.0).to_path(0.1).iter());
            }
            x += 0.8;
        }
        let n = (width / 2.0).floor().max(1.0) as i64;
        for k in 0..=n {
            let x = b.x0 + 0.3 + (width - 0.6) * k as f64 / n as f64;
            let on = ((time * 2.0 - k as f64 * 0.25).fract() < 0.3) as i32 as f32;
            lamps.push((p(x, b.y0 + 0.06, b.z0 - 0.02), Color::from_rgb8(0xff, 0xb0, 0x40), 0.3 + 0.7 * on));
        }
    } else if plinth {
        let mut strip = BezPath::new();
        strip.move_to(p(b.x0 + 0.1, (b.y0 + b.y1) / 2.0, b.z0 - 0.01));
        strip.line_to(p(b.x1 - 0.1, (b.y0 + b.y1) / 2.0, b.z0 - 0.01));
        neon.push((outline(&strip, 0.05 * scale), NEON_CYAN, 0.6 + 0.4 * (time * 3.0).sin().abs() as f32));
    } else {
        // A cargo container: corrugated, framed, stencilled; or a machine
        // housing with hazard stripes and a status light.
        let mut x = b.x0 + 0.15;
        while x < b.x1 - 0.15 {
            dark_lines.move_to(p(x, b.y0 + 0.1, b.z0));
            dark_lines.line_to(p(x, b.y1 - 0.1, b.z0));
            x += 0.2;
        }
        steel_rims.extend(rect_path(p(b.x0 + 0.04, b.y0 + 0.04, b.z0), p(b.x1 - 0.04, b.y1 - 0.04, b.z0)).iter());
        if hash(seed as i64, 4) < 0.4 && b.y1 - b.y0 > 1.0 {
            let band_y = b.y1 - 0.45;
            let mut stripes = BezPath::new();
            let mut sx = b.x0;
            while sx < b.x1 {
                stripes.extend(poly(&cam, &[DVec3::new(sx, band_y - 0.2, b.z0), DVec3::new(sx + 0.2, band_y - 0.2, b.z0), DVec3::new(sx + 0.4, band_y, b.z0), DVec3::new(sx + 0.2, band_y, b.z0)]).iter());
                sx += 0.4;
            }
            neon.push((stripes, Color::from_rgb8(0xf0, 0xb0, 0x30), 0.9));
            let on = ((time * 0.7 + hash(seed as i64, 5)).fract() < 0.5) as i32 as f32;
            lamps.push((p(b.x1 - 0.35, b.y0 + 0.35, b.z0 - 0.02), Color::from_rgb8(0x60, 0xff, 0x80), 0.3 + 0.7 * on));
        } else {
            let sy = b.y1 - 0.35;
            for (g, len) in [0.5, 0.25, 0.4].iter().enumerate() {
                let gx = b.x0 + 0.3 + g as f64 * 0.6;
                if gx + len < b.x1 - 0.2 {
                    neon.push((rect_path(p(gx, sy - 0.14, b.z0 - 0.01), p(gx + len, sy, b.z0 - 0.01)), lighten(paint, 0.55), 0.8));
                }
            }
        }
    }

    // The roof's top: warm grazing light, expansion joints, stains, puddles
    // reflecting the sky, weeds in the cracks and rubble.
    let top_visible = cam.eye.y > b.y1;
    let mut joints = BezPath::new();
    let mut stains: Vec<(BezPath, Color)> = Vec::new();
    let mut weeds = BezPath::new();
    let mut rubble_paths: Vec<(BezPath, BezPath)> = Vec::new();
    let mut rubble: Caster = Vec::new();
    let mut puddles: Vec<BezPath> = Vec::new();
    if top_visible && building {
        let (tx0, tx1) = (b.x0.max(visible.0 - 1.0), b.x1.min(visible.1 + 1.0));
        let spacing = 3.1;
        for i in (tx0 / spacing).ceil() as i64..=(tx1 / spacing).floor() as i64 {
            let x = i as f64 * spacing + (hash(i, seed ^ 40) - 0.5) * 0.3;
            if x > b.x0 + 0.3 && x < b.x1 - 0.3 {
                // A joint, cracked and wandering.
                for k in 0..=8 {
                    let z = b.z0 + bite(x) + (b.z1 - b.z0 - bite(x)) * k as f64 / 8.0;
                    let q = p(x + noise1(z * 2.5 + i as f64 * 3.0, seed ^ 41) * 0.12, b.y1, z);
                    if k == 0 { joints.move_to(q) } else { joints.line_to(q) }
                }
            }
        }
        let zj = b.z0 + 1.6;
        joints.move_to(p(tx0.max(b.x0), b.y1, zj));
        joints.line_to(p(tx1.min(b.x1), b.y1, zj));
        // Cracks: short random walks.
        for i in (tx0 / 4.3).floor() as i64..=(tx1 / 4.3).ceil() as i64 {
            let hs = seed ^ 45;
            if hash(i, hs) < 0.4 {
                continue;
            }
            let (mut x, mut z) = ((i as f64 + hash(i, hs + 1)) * 4.3, b.z0 + 0.3 + (b.z1 - b.z0 - 0.6) * hash(i, hs + 2));
            let mut heading = hash(i, hs + 3) * TAU;
            joints.move_to(p(x, b.y1, z));
            for k in 0..7 {
                heading += noise1(k as f64 * 0.7 + i as f64, hs + 4) * 1.2;
                x += heading.cos() * 0.28;
                z = (z + heading.sin() * 0.16).clamp(b.z0 + 0.1, b.z1 - 0.1);
                if x < b.x0 + 0.1 || x > b.x1 - 0.1 {
                    break;
                }
                joints.line_to(p(x, b.y1, z));
            }
        }
        // Stains: blobs with noisy outlines.
        for i in (tx0 / 3.7).floor() as i64..=(tx1 / 3.7).ceil() as i64 {
            let hs = seed ^ 50;
            if !clutter || hash(i, hs) < 0.35 {
                continue;
            }
            let cx = (i as f64 + hash(i, hs + 1)) * 3.7;
            let cz = b.z0 + 0.5 + (b.z1 - b.z0 - 1.0) * hash(i, hs + 2);
            let (rx, rz) = (0.5 + 1.3 * hash(i, hs + 3), 0.2 + 0.5 * hash(i, hs + 4));
            let mut pts = Vec::new();
            for k in 0..14 {
                let a = k as f64 / 14.0 * TAU;
                // Noise round the circle (sampled in 2D so it closes up).
                let wob = 1.0 + 0.6 * fbm2(a.cos() * 1.4 + i as f64 * 5.0, a.sin() * 1.4, 3, hs + 5);
                let x = (cx + a.cos() * rx * wob).clamp(b.x0 + 0.05, b.x1 - 0.05);
                let z = (cz + a.sin() * rz * wob).clamp(b.z0 + 0.05, b.z1 - 0.05);
                pts.push(DVec3::new(x, b.y1, z));
            }
            let dark = hash(i, hs + 6) < 0.65;
            let color = if dark { darken(top, 0.35).with_alpha(0.3) } else { lighten(top, 0.2).with_alpha(0.25) };
            if hash(i, hs + 7) < 0.12 {
                puddles.push(poly(&cam, &pts));
            } else {
                stains.push((poly(&cam, &pts), color));
            }
        }
        // Weeds where the joints crack, and rubble.
        for i in (tx0 / 1.1).floor() as i64..=(tx1 / 1.1).ceil() as i64 {
            let hs = seed ^ 60;
            let x = (i as f64 + hash(i, hs)) * 1.1;
            if !clutter || x < b.x0 + 0.2 || x > b.x1 - 0.2 {
                continue;
            }
            let z = b.z0 + 0.15 + (b.z1 - b.z0 - 0.3) * hash(i, hs + 1);
            let roll = hash(i, hs + 2);
            if roll < 0.22 {
                let sway = (time * 1.4 + x).sin() * 0.05;
                for k in 0..4 {
                    let a = (k as f64 - 1.5) * 0.35 + sway;
                    let len = 0.12 + 0.2 * hash(i * 4 + k, hs + 3);
                    let base = DVec3::new(x + k as f64 * 0.03, b.y1, z);
                    let tip = base + DVec3::new(a.sin() * len, a.cos() * len, 0.0);
                    weeds.move_to(cam.point(base - DVec3::X * 0.02));
                    weeds.quad_to(cam.point(base.lerp(tip, 0.5) + DVec3::X * 0.02), cam.point(tip));
                    weeds.quad_to(cam.point(base.lerp(tip, 0.5) + DVec3::X * 0.04), cam.point(base + DVec3::X * 0.02));
                    weeds.close_path();
                }
            } else if roll < 0.45 {
                // A chunk of concrete: an irregular little block.
                let size = 0.06 + 0.14 * hash(i, hs + 4);
                let hgt = size * (0.6 + 0.8 * hash(i, hs + 5));
                let skew = (hash(i, hs + 6) - 0.5) * size;
                let base = DVec3::new(x, b.y1, z);
                let body = poly(&cam, &[base - DVec3::X * size, base + DVec3::X * size, base + DVec3::new(size * 0.6 + skew, hgt, 0.0), base + DVec3::new(-size * 0.5 + skew, hgt * 0.8, 0.0)]);
                let lit = poly(&cam, &[base + DVec3::X * size * 0.3, base + DVec3::X * size, base + DVec3::new(size * 0.6 + skew, hgt, 0.0), base + DVec3::new(size * 0.15 + skew, hgt * 0.95, 0.0)]);
                rubble_paths.push((body, lit));
                rubble.push(box_corners(x - size, x + size, b.y1, b.y1 + hgt, z - size, z + size));
            }
        }
    }

    // The top face and its front edge, lit by the grazing sun.
    let top_face = top_visible.then(|| {
        let mut face = BezPath::new();
        face.move_to(top_edge[0]);
        for q in &top_edge[1..] {
            face.line_to(*q);
        }
        face.line_to(p(b.x1, b.y1, b.z1));
        face.line_to(p(b.x0, b.y1, b.z1));
        face.close_path();
        let xm = (b.x0 + b.x1) / 2.0;
        let shade = Gradient::new_linear(p(xm, b.y1, b.z0), p(xm, b.y1, b.z1)).with_stops([
            (0.0, lighten(top, 0.12)),
            (0.5, top),
            (1.0, mix(top, HAZE, 0.25)),
        ]);
        (face, shade)
    });
    let mut edge = BezPath::new();
    edge.move_to(top_edge[0]);
    for q in &top_edge[1..] {
        edge.line_to(*q);
    }
    // A parapet lip along the top of the facade, its lower edge ragged.
    let lip = building.then(|| {
        let mut lip = BezPath::new();
        lip.move_to(front_edge[0]);
        for q in &front_edge[1..] {
            lip.line_to(*q);
        }
        let n = (width / 0.3).ceil() as i64;
        for k in (0..=n).rev() {
            let x = b.x0 + width * k as f64 / n as f64;
            let rag = bite(x) * 1.1 + 0.22 + 0.1 * noise1(x * 2.1, seed ^ 70) + if hash(k, seed ^ 71) < 0.08 { 0.15 } else { 0.0 };
            lip.line_to(p(x, b.y1 - rag, b.z0));
        }
        lip.close_path();
        lip
    });
    // Railing and suspension rods of a catwalk, behind the walkway.
    let railing = catwalk.then(|| {
        let mut rails = BezPath::new();
        let zr = b.z1 - 0.08;
        let n = (width / 1.3).ceil().max(1.0) as i64;
        for k in 0..=n {
            let x = b.x0 + 0.1 + (width - 0.2) * k as f64 / n as f64;
            rails.move_to(p(x, b.y1, zr));
            rails.line_to(p(x, b.y1 + 1.0, zr));
        }
        for h in [0.5, 1.0] {
            rails.move_to(p(b.x0 + 0.1, b.y1 + h, zr));
            rails.line_to(p(b.x1 - 0.1, b.y1 + h, zr));
        }
        for x in [b.x0 + 0.3, b.x1 - 0.3] {
            rails.move_to(p(x, b.y1 + 1.0, zr));
            rails.line_to(p(x + 0.4, b.y1 + 18.0, zr));
        }
        let mut grate = BezPath::new();
        let mut x = b.x0 + 0.15;
        while x < b.x1 {
            grate.move_to(p(x, b.y1, b.z0));
            grate.line_to(p(x, b.y1, b.z1));
            x += 0.25;
        }
        (rails, grate)
    });

    let ink = darken(front, 0.45);
    let a = p(b.x0, b.y1, b.z0);
    let (rails, grate) = railing.unzip();
    canvas.push(depth, move |scene| {
        let id = Affine::IDENTITY;
        for (face, fill) in &sides {
            scene.fill(Fill::NonZero, id, fill, None, face);
        }
        // The front is in shade, lit only by the sky: brighter near the top.
        scene.fill(Fill::NonZero, id, front, None, &front_face);
        scene.push_clip_layer(Fill::NonZero, id, &front_face);
        let bounce = Gradient::new_linear(a, a + Vec2::new(0.0, 7.0 * scale)).with_stops([
            (0.0, Color::from_rgb8(0xd8, 0x80, 0x90).with_alpha(0.18)),
            (1.0, Color::from_rgb8(0x10, 0x08, 0x20).with_alpha(0.35)),
        ]);
        scene.fill(Fill::NonZero, id, &bounce, None, &front_face);
        scene.fill(Fill::NonZero, id, darken(front, 0.35), None, &dark_windows);
        if !dark_windows.elements().is_empty() {
            // Dark glass reflects a streak of the sky.
            let sheen = Gradient::new_linear(a, a + Vec2::new(3.0 * scale, 3.0 * scale)).with_stops([
                (0.0, Color::from_rgb8(0xe0, 0x88, 0xa0).with_alpha(0.0)),
                (0.5, Color::from_rgb8(0xe0, 0x88, 0xa0).with_alpha(0.22)),
                (1.0, Color::from_rgb8(0xe0, 0x88, 0xa0).with_alpha(0.0)),
            ]);
            let sheen = sheen.with_extend(vello::peniko::Extend::Repeat);
            scene.fill(Fill::NonZero, id, &sheen, None, &dark_windows);
        }
        scene.fill(Fill::NonZero, id, WINDOW, None, &lit_windows);
        scene.fill(Fill::NonZero, id, lighten(NEON_CYAN, 0.3), None, &cyan_windows);
        scene.stroke(&Stroke::new(0.06 * scale), id, darken(front, 0.45), None, &frames);
        scene.fill(Fill::NonZero, id, lighten(front, 0.14), None, &sills);
        for (streak, g) in &streaks {
            scene.fill(Fill::NonZero, id, g, None, streak);
        }
        scene.stroke(&Stroke::new(0.05 * scale), id, ink.with_alpha(0.7), None, &dark_lines);
        scene.fill(Fill::NonZero, id, darken(front, 0.2), None, &boxes);
        scene.fill(Fill::NonZero, id, darken(front, 0.5), None, &fans);
        scene.stroke(&Stroke::new(0.04 * scale), id, lighten(front, 0.2), None, &steel_rims);
        if let Some(lip) = &lip {
            scene.fill(Fill::NonZero, id, lighten(front, 0.12), None, lip);
        }
        scene.pop_layer();
        scene.stroke(&Stroke::new(0.035 * scale).with_caps(Cap::Round), id, Color::from_rgb8(0x6a, 0x3a, 0x30), None, &rebar);
        scene.stroke(&Stroke::new(0.025 * scale), id, darken(WEED, 0.3), None, &vines);
        scene.fill(Fill::NonZero, id, darken(WEED, 0.15), None, &leaves);
        scene.fill(Fill::NonZero, id, mix(WEED, SUNLIT, 0.35), None, &lit_leaves);
        for (path, color, alpha) in &neon {
            scene.fill(Fill::NonZero, id, color.with_alpha(*alpha), None, path);
        }
        for &(at, color, alpha) in &lamps {
            glow(scene, at, 0.5 * scale, color, alpha * 0.8);
            scene.fill(Fill::NonZero, id, lighten(color, 0.5).with_alpha(alpha), None, &Circle::new(at, 0.04 * scale));
        }
        // Broken concrete where the edge crumbled away.
        scene.fill(Fill::NonZero, id, mix(top, front, 0.55), None, &broken);
        if let Some((face, shade)) = &top_face {
            scene.fill(Fill::NonZero, id, top, None, face);
            scene.fill(Fill::NonZero, id, shade, None, face);
            scene.push_clip_layer(Fill::NonZero, id, face);
            for (stain, color) in &stains {
                scene.fill(Fill::NonZero, id, *color, None, stain);
            }
            for puddle in &puddles {
                // Still water mirrors the sunset.
                let bb = puddle.bounding_box();
                let g = Gradient::new_linear((bb.x0, bb.y0), (bb.x0, bb.y1)).with_stops([Color::from_rgb8(0xff, 0xb0, 0x80), Color::from_rgb8(0xc0, 0x5a, 0x80)]);
                scene.fill(Fill::NonZero, id, &g, None, puddle);
            }
            scene.stroke(&Stroke::new(0.03 * scale), id, darken(top, 0.4).with_alpha(0.5), None, &joints);
            if let Some(grate) = &grate {
                scene.stroke(&Stroke::new(0.025 * scale), id, darken(top, 0.45).with_alpha(0.6), None, grate);
            }
            scene.pop_layer();
            scene.fill(Fill::NonZero, id, WEED, None, &weeds);
            for (body, lit) in &rubble_paths {
                scene.fill(Fill::NonZero, id, darken(top, 0.2), None, body);
                scene.fill(Fill::NonZero, id, CONCRETE_LIT, None, lit);
            }
        }
        // Sunlight skimming the roof catches the front edge.
        scene.stroke(&Stroke::new(0.05 * scale), id, RIM.with_alpha(0.85), None, &edge);
    });
    if let Some(rails) = rails {
        let color = darken(STEEL, 0.1);
        canvas.push(depth - 0.01, move |scene| {
            scene.stroke(&Stroke::new(0.05 * scale).with_caps(Cap::Round), Affine::IDENTITY, color, None, &rails);
            // The sun catches the rails' right sides.
            scene.stroke(&Stroke::new(0.015 * scale), Affine::translate((0.02 * scale, 0.0)), RIM.with_alpha(0.5), None, &rails);
        });
    }
    rubble
}

/// Where the shadow of the next building towards the sun cuts across this
/// one's sunny side: sunlight only reaches above this height.
fn sun_line(blocks: &[Block], b: &Block) -> f64 {
    let l = sun_dir();
    let slope = l.y / l.x;
    blocks
        .iter()
        .filter(|n| n.x0 >= b.x1 - 0.01 && n.x0 - b.x1 < 40.0 && n.y1 > b.y0)
        .map(|n| n.y1 - (n.x0 - b.x1) * slope)
        .fold(f64::NEG_INFINITY, f64::max)
}

// ---------------------------------------------------------------------------
// Props

/// The pieces of a prop that throw shadows.
fn prop_caster(prop: &Prop, time: f64) -> Caster {
    let base = DVec3::new(prop.x, prop.y, prop.z);
    let pole = |r: f64, h: f64| box_corners(prop.x - r, prop.x + r, prop.y, prop.y + h, prop.z - r, prop.z + r);
    match prop.kind {
        PropKind::Lamp => vec![pole(0.07, 3.3), box_corners(prop.x - 0.9, prop.x + 0.05, prop.y + 3.15, prop.y + 3.4, prop.z - 0.15, prop.z + 0.15)],
        PropKind::Antenna => {
            let top = base + DVec3::Y * 5.6;
            let dish = dish_angle(prop, time);
            let face = DVec3::new(dish.cos(), 0.0, dish.sin());
            let side = DVec3::new(-dish.sin(), 0.0, dish.cos());
            let c = top + DVec3::Y * 0.5;
            vec![
                vec![base - DVec3::X * 0.5, base + DVec3::X * 0.5, base - DVec3::Z * 0.4, base + DVec3::Z * 0.4, top],
                vec![c + side * 0.9, c - side * 0.9, c + DVec3::Y * 0.9, c - DVec3::Y * 0.9, c + face * 0.3],
            ]
        }
        PropKind::Vent => vec![box_corners(prop.x - 0.6, prop.x + 0.6, prop.y, prop.y + 0.9, prop.z - 0.4, prop.z + 0.4)],
        PropKind::Sign => vec![pole(0.06, 2.4)],
    }
}

fn dish_angle(prop: &Prop, time: f64) -> f64 {
    time * 0.7 + prop.x
}

/// A post from `a` up to `b`, `r` thick: dark, with a lit strip down the
/// side facing the sun.
fn post_paths(cam: &Camera, a: DVec3, b: DVec3, r: f64) -> (BezPath, BezPath) {
    let body = poly(cam, &[a - DVec3::X * r, a + DVec3::X * r, b + DVec3::X * r, b - DVec3::X * r]);
    let lit = poly(cam, &[a + DVec3::X * r * 0.2, a + DVec3::X * r, b + DVec3::X * r, b + DVec3::X * r * 0.2]);
    (body, lit)
}

fn draw_prop(canvas: &mut Canvas3d, level: &Level, prop: &Prop, time: f64) {
    let cam = canvas.camera;
    let base = DVec3::new(prop.x, prop.y, prop.z);
    let pr = cam.project(base);
    let s = pr.scale;
    let mut depth = pr.depth;
    // Things behind the middle of their roof are drawn just after it.
    if let Some(b) = block_under(level, prop.x, prop.y) {
        depth = depth.min(block_depth(&cam, b) - 0.01);
    }
    let seed = (prop.x * 10.0) as i64;
    let metal = Color::from_rgb8(0x2e, 0x24, 0x36);
    match prop.kind {
        PropKind::Lamp => {
            let top = base + DVec3::Y * 3.3;
            let (pole, lit) = post_paths(&cam, base, top, 0.07);
            let head = top + DVec3::new(-0.75, -0.05, 0.0);
            let mut arm = BezPath::new();
            arm.move_to(cam.point(top));
            arm.quad_to(cam.point(top + DVec3::new(-0.1, 0.25, 0.0)), cam.point(head + DVec3::Y * 0.1));
            let hood = poly(&cam, &[head + DVec3::new(-0.25, 0.05, 0.0), head + DVec3::new(0.25, 0.15, 0.0), head + DVec3::new(0.2, -0.05, 0.0), head + DVec3::new(-0.3, -0.08, 0.0)]);
            // Street lights come on at dusk, one sometimes stuttering.
            let stutter = hash(seed, 1) < 0.3 && noise1(time * 4.0, seed as u64) > 0.55;
            let on = if stutter { 0.15 } else { 1.0 };
            let bulb = cam.point(head - DVec3::Y * 0.08);
            let ground = cam.point(DVec3::new(head.x, prop.y, head.z));
            let pool = canvas.project_ellipsoid(DVec3::new(head.x, prop.y + 0.01, head.z), DMat3::from_cols(DVec3::X * 1.6, DVec3::Z * 0.9, DVec3::Y * 1e-3));
            let mut cone = BezPath::new();
            cone.move_to(bulb + Vec2::new(-0.12 * s, 0.0));
            cone.line_to(ground + Vec2::new(-1.3 * s, 0.0));
            cone.line_to(ground + Vec2::new(1.3 * s, 0.0));
            cone.line_to(bulb + Vec2::new(0.12 * s, 0.0));
            cone.close_path();
            canvas.push(depth, move |scene| {
                let id = Affine::IDENTITY;
                let beam = Gradient::new_linear(bulb, ground).with_stops([WINDOW.with_alpha(0.3 * on), WINDOW.with_alpha(0.0)]);
                scene.fill(Fill::NonZero, id, &beam, None, &cone);
                let pool_fill = Gradient::new_radial(pool.center(), pool.radii().x as f32).with_stops([WINDOW.with_alpha(0.3 * on), WINDOW.with_alpha(0.0)]);
                scene.fill(Fill::NonZero, id, &pool_fill, None, &pool);
                scene.fill(Fill::NonZero, id, metal, None, &pole);
                scene.fill(Fill::NonZero, id, SUNLIT.with_alpha(0.8), None, &lit);
                scene.stroke(&Stroke::new(0.09 * s).with_caps(Cap::Round), id, metal, None, &arm);
                scene.fill(Fill::NonZero, id, metal, None, &hood);
                glow(scene, bulb, 0.9 * s, WINDOW, 0.7 * on);
                scene.fill(Fill::NonZero, id, Color::from_rgb8(0xff, 0xf4, 0xd8).with_alpha(on), None, &Ellipse::new(bulb, (0.18 * s, 0.05 * s), 0.0));
            });
        }
        PropKind::Antenna => {
            // A lattice mast, a turning dish and a warning light.
            let top = base + DVec3::Y * 5.6;
            let mut lattice = BezPath::new();
            let legs = [(-0.5, 0.0), (0.5, 0.0)];
            for (dx, _) in legs {
                lattice.move_to(cam.point(base + DVec3::X * dx));
                lattice.line_to(cam.point(top + DVec3::X * dx * 0.15));
            }
            let mut y = 0.0;
            let mut flip = 1.0;
            while y < 5.4 {
                let w0 = 0.5 - 0.425 * y / 5.6;
                let w1 = 0.5 - 0.425 * (y + 0.6) / 5.6;
                lattice.move_to(cam.point(base + DVec3::new(-w0 * flip, y, 0.0)));
                lattice.line_to(cam.point(base + DVec3::new(w1 * flip, y + 0.6, 0.0)));
                y += 0.6;
                flip = -flip;
            }
            let a = dish_angle(prop, time);
            let c = top + DVec3::Y * 0.5;
            // The dish turns about the mast: seen from the front it's an
            // ellipse whose width follows the turn.
            let dish = canvas.project_ellipsoid(
                c,
                DMat3::from_cols(DVec3::new(-a.sin(), 0.0, a.cos()) * 0.9, DVec3::Y * 0.9, DVec3::new(a.cos(), 0.0, a.sin()) * 0.08),
            );
            let facing = a.sin();
            let feed = cam.point(c + DVec3::new(a.cos(), 0.0, a.sin()) * 0.7);
            let blink = ((time * 0.9 + hash(seed, 2)).fract() < 0.15) as i32 as f32;
            let tip = cam.point(c + DVec3::Y * 1.1);
            canvas.push(depth, move |scene| {
                let id = Affine::IDENTITY;
                scene.stroke(&Stroke::new(0.06 * s).with_caps(Cap::Round), id, metal, None, &lattice);
                scene.stroke(&Stroke::new(0.02 * s), Affine::translate((0.03 * s, 0.0)), SUNLIT.with_alpha(0.6), None, &lattice);
                let inside = if facing < 0.0 { lighten(metal, 0.5) } else { metal };
                scene.fill(Fill::NonZero, id, inside, None, &dish);
                scene.stroke(&Stroke::new(0.05 * s), id, lighten(metal, 0.2), None, &dish);
                // The rim facing the sun glows.
                scene.push_clip_layer(Fill::NonZero, id, &dish);
                scene.stroke(&Stroke::new(0.12 * s), Affine::translate((-0.06 * s, 0.02 * s)), SUNLIT.with_alpha(0.7), None, &dish);
                scene.pop_layer();
                let mut arm = BezPath::new();
                arm.move_to(dish.center());
                arm.line_to(feed);
                scene.stroke(&Stroke::new(0.04 * s), id, metal, None, &arm);
                let mut mast_tip = BezPath::new();
                mast_tip.move_to(dish.center());
                mast_tip.line_to(tip);
                scene.stroke(&Stroke::new(0.04 * s), id, metal, None, &mast_tip);
                glow(scene, tip, 0.6 * s, Color::from_rgb8(0xff, 0x40, 0x40), blink);
            });
        }
        PropKind::Vent => {
            let (x0, x1, y1, z0, z1) = (prop.x - 0.6, prop.x + 0.6, prop.y + 0.9, prop.z - 0.4, prop.z + 0.4);
            let front = poly(&cam, &[DVec3::new(x0, prop.y, z0), DVec3::new(x1, prop.y, z0), DVec3::new(x1, y1, z0), DVec3::new(x0, y1, z0)]);
            let top = poly(&cam, &[DVec3::new(x0, y1, z0), DVec3::new(x1, y1, z0), DVec3::new(x1, y1, z1), DVec3::new(x0, y1, z1)]);
            let side = (cam.eye.x > x1).then(|| poly(&cam, &[DVec3::new(x1, prop.y, z0), DVec3::new(x1, prop.y, z1), DVec3::new(x1, y1, z1), DVec3::new(x1, y1, z0)]));
            let c = cam.point(DVec3::new(prop.x, prop.y + 0.45, z0 - 0.01));
            let r = 0.32 * s;
            let spin = time * 11.0 + prop.x;
            let mut blades = BezPath::new();
            for k in 0..5 {
                let a = spin + k as f64 * TAU / 5.0;
                let tip = c + Vec2::new(a.cos(), a.sin()) * r;
                let side = Vec2::new(-a.sin(), a.cos()) * r * 0.35;
                blades.move_to(c);
                blades.quad_to(c + (tip - c) * 0.5 + side, tip);
                blades.line_to(c + (tip - c) * 0.85 - side * 0.5);
                blades.close_path();
            }
            let mut grille = BezPath::new();
            for k in -2..=2 {
                let y = c.y + k as f64 * r * 0.4;
                let half = (r * r - (k as f64 * r * 0.4).powi(2)).max(0.0).sqrt();
                grille.move_to((c.x - half, y));
                grille.line_to((c.x + half, y));
            }
            let puffs: Vec<(Point, f64, f64)> = (0..5)
                .map(|k| {
                    let phase = (time * 0.35 + k as f64 / 5.0 + hash(seed, 3)).fract();
                    let at = cam.point(DVec3::new(prop.x - phase * 1.2, y1 + 0.2 + phase * 2.4, prop.z));
                    (at, (0.25 + 0.7 * phase) * s, phase)
                })
                .collect();
            canvas.push(depth, move |scene| {
                let id = Affine::IDENTITY;
                let body = Color::from_rgb8(0x5a, 0x4a, 0x62);
                if let Some(side) = &side {
                    scene.fill(Fill::NonZero, id, SUNLIT, None, side);
                }
                scene.fill(Fill::NonZero, id, body, None, &front);
                scene.fill(Fill::NonZero, id, lighten(body, 0.3), None, &top);
                scene.fill(Fill::NonZero, id, darken(body, 0.6), None, &Circle::new(c, r));
                scene.fill(Fill::NonZero, id, darken(body, 0.2), None, &blades);
                scene.stroke(&Stroke::new(0.03 * s), id, lighten(body, 0.1), None, &grille);
                scene.stroke(&Stroke::new(0.04 * s), id, lighten(body, 0.15), None, &Circle::new(c, r));
                for &(at, r, phase) in &puffs {
                    steam(scene, at, r, phase);
                }
            });
        }
        PropKind::Sign => {
            // A hologram on a post: see-through, flickering, glitching sideways.
            let top = base + DVec3::Y * 2.4;
            let (pole, lit) = post_paths(&cam, base, top, 0.06);
            let color = if hash(seed, 4) < 0.5 { NEON_PINK } else { NEON_CYAN };
            let glitch = if noise1(time * 3.0, seed as u64 ^ 5) > 0.8 { 0.15 * (time * 60.0).sin() } else { 0.0 };
            let flicker = (0.75 + 0.25 * (time * 37.0).sin() * noise1(time * 2.0, seed as u64 ^ 6)) as f32;
            let c = top + DVec3::new(glitch, 0.75, 0.0);
            let panel = poly(&cam, &[c + DVec3::new(-1.0, -0.6, 0.0), c + DVec3::new(1.0, -0.6, 0.0), c + DVec3::new(1.0, 0.6, 0.0), c + DVec3::new(-1.0, 0.6, 0.0)]);
            let mut glyphs = BezPath::new();
            for row in 0..3 {
                let mut x = -0.85;
                let mut k = 0;
                while x < 0.8 {
                    let gw = 0.1 + 0.2 * hash(seed * 100 + row * 10 + k, 7);
                    let gy = c.y + 0.3 - row as f64 * 0.32;
                    if x + gw < 0.85 {
                        glyphs.extend(poly(&cam, &[DVec3::new(c.x + x, gy - 0.16, c.z), DVec3::new(c.x + x + gw, gy - 0.16, c.z), DVec3::new(c.x + x + gw, gy, c.z), DVec3::new(c.x + x, gy, c.z)]).iter());
                    }
                    x += gw + 0.08;
                    k += 1;
                }
            }
            let mut scan = BezPath::new();
            let mut y = -0.55;
            while y < 0.6 {
                scan.move_to(cam.point(c + DVec3::new(-1.0, y, 0.0)));
                scan.line_to(cam.point(c + DVec3::new(1.0, y, 0.0)));
                y += 0.12;
            }
            let center = cam.point(c);
            canvas.push(depth, move |scene| {
                let id = Affine::IDENTITY;
                scene.fill(Fill::NonZero, id, metal, None, &pole);
                scene.fill(Fill::NonZero, id, SUNLIT.with_alpha(0.8), None, &lit);
                glow(scene, center, 2.2 * s, color, 0.3 * flicker);
                scene.fill(Fill::NonZero, id, color.with_alpha(0.22 * flicker), None, &panel);
                scene.stroke(&Stroke::new(0.04 * s), id, color.with_alpha(0.8 * flicker), None, &panel);
                scene.fill(Fill::NonZero, id, lighten(color, 0.6).with_alpha(0.85 * flicker), None, &glyphs);
                scene.stroke(&Stroke::new(0.02 * s), id, Color::BLACK.with_alpha(0.25), None, &scan);
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Pickups, checkpoints, the goal and the air

/// An energy cell to collect: a glowing core in two spinning rings.
fn draw_cell(canvas: &mut Canvas3d, pos: DVec2, time: f64, i: usize) {
    let c = DVec3::new(pos.x, pos.y, -0.05);
    let pr = canvas.camera.project(c);
    let s = pr.scale;
    let spin = time * 2.2 + i as f64;
    let rings = [
        canvas.project_ellipsoid(c, DMat3::from_cols(DVec3::new(spin.cos(), 0.0, spin.sin()) * 0.28, DVec3::Y * 0.28, DVec3::new(-spin.sin(), 0.0, spin.cos()) * 0.01)),
        canvas.project_ellipsoid(c, DMat3::from_cols(DVec3::X * 0.24, DVec3::new(0.0, spin.cos(), spin.sin()) * 0.24, DVec3::new(0.0, -spin.sin(), spin.cos()) * 0.01)),
    ];
    let pulse = 0.8 + 0.2 * (time * 5.0 + i as f64).sin();
    canvas.push(pr.depth, move |scene| {
        glow(scene, pr.pos, 0.7 * s * pulse, Color::from_rgb8(0xff, 0xb0, 0x50), 0.55);
        for ring in &rings {
            scene.stroke(&Stroke::new(0.035 * s), Affine::IDENTITY, Color::from_rgb8(0xff, 0xe0, 0xa0), None, ring);
        }
        scene.fill(Fill::NonZero, Affine::IDENTITY, Color::from_rgb8(0xff, 0xf6, 0xd8), None, &Circle::new(pr.pos, 0.09 * s * pulse));
    });
}

/// A checkpoint beacon: a slim pylon whose light turns from red to cyan
/// when reached, sending rings over the roof.
fn draw_beacon(canvas: &mut Canvas3d, at: DVec2, reached: bool, time: f64) {
    let cam = canvas.camera;
    let base = DVec3::new(at.x, at.y, 0.9);
    let top = base + DVec3::Y * 2.1;
    let (pole, lit) = post_paths(&cam, base, top, 0.05);
    let color = if reached { NEON_CYAN } else { Color::from_rgb8(0xff, 0x48, 0x48) };
    let on = if reached { 1.0 } else { ((time * 1.5).fract() < 0.5) as i32 as f32 * 0.8 + 0.2 };
    let light = cam.point(top);
    let s = cam.project(top).scale;
    let rings: Vec<(Ellipse, f32)> = if reached {
        (0..2)
            .map(|k| {
                let t = (time * 0.6 + k as f64 * 0.5).fract();
                let r = 0.3 + 1.6 * t;
                (canvas.project_ellipsoid(base + DVec3::Y * 0.02, DMat3::from_cols(DVec3::X * r, DVec3::Z * r * 0.6, DVec3::Y * 1e-3)), (1.0 - t) as f32 * 0.6)
            })
            .collect()
    } else {
        Vec::new()
    };
    let depth = canvas.depth_of(base) - 0.01;
    canvas.push(depth, move |scene| {
        let id = Affine::IDENTITY;
        for (ring, alpha) in &rings {
            scene.stroke(&Stroke::new(0.04 * s), id, color.with_alpha(*alpha), None, ring);
        }
        scene.fill(Fill::NonZero, id, Color::from_rgb8(0x2e, 0x24, 0x36), None, &pole);
        scene.fill(Fill::NonZero, id, SUNLIT.with_alpha(0.8), None, &lit);
        glow(scene, light, 0.8 * s, color, 0.8 * on);
        scene.fill(Fill::NonZero, id, lighten(color, 0.5).with_alpha(on), None, &Rect::from_center_size(light, (0.16 * s, 0.26 * s)));
    });
}

/// The goal: a teleporter, three rings spinning over its plinth round a
/// column of light, with sparks rising through it.
fn draw_teleporter(canvas: &mut Canvas3d, at: DVec2, time: f64) {
    let cam = canvas.camera;
    let base = DVec3::new(at.x, at.y, 0.6);
    let s = cam.project(base).scale;
    let mut rings = Vec::new();
    for k in 0..3 {
        let y = 0.35 + k as f64 * 1.1 + (time * 1.5 + k as f64).sin() * 0.06;
        let tilt = (time * (0.9 + 0.3 * k as f64) + k as f64 * 2.0).sin() * 0.25;
        let c = base + DVec3::Y * y;
        let e = canvas.project_ellipsoid(c, DMat3::from_cols(DVec3::X * 1.1, DVec3::new(0.0, tilt.sin(), tilt.cos()) * 1.1, DVec3::Y * 0.02));
        let pulse = (0.6 + 0.4 * (time * 3.0 - k as f64).sin()) as f32;
        rings.push((e, pulse));
    }
    let bottom = cam.point(base);
    let top = cam.point(base + DVec3::Y * 4.0);
    let column = Rect::new(bottom.x - 0.7 * s, top.y, bottom.x + 0.7 * s, bottom.y);
    let sparks: Vec<Point> = (0..14)
        .map(|k| {
            let t = (time * 0.4 + hash(k, 90)).fract();
            let a = time * 2.0 + k as f64 * 2.4;
            cam.point(base + DVec3::new(a.cos() * 0.5 * (1.0 - t), t * 4.0, a.sin() * 0.5))
        })
        .collect();
    let depth = canvas.depth_of(base) - 0.3;
    canvas.push(depth, move |scene| {
        let id = Affine::IDENTITY;
        // A radial gradient squeezed tall and thin: bright at the base,
        // fading sideways and upwards.
        let beam = Gradient::new_radial((0.0, 0.0), 1.0).with_stops([
            (0.0, NEON_CYAN.with_alpha(0.55)),
            (0.5, NEON_CYAN.with_alpha(0.25)),
            (1.0, NEON_CYAN.with_alpha(0.0)),
        ]);
        let squeeze = Affine::translate(bottom.to_vec2()) * Affine::scale_non_uniform(column.width() / 2.0, column.height());
        scene.fill(Fill::NonZero, id, &beam, Some(squeeze), &column);
        for &p in &sparks {
            scene.fill(Fill::NonZero, id, Color::from_rgb8(0xd8, 0xff, 0xff), None, &Circle::new(p, 0.04 * s));
        }
        for (ring, pulse) in &rings {
            scene.stroke(&Stroke::new(0.16 * s), id, Color::from_rgb8(0x30, 0x2a, 0x44), None, ring);
            scene.stroke(&Stroke::new(0.07 * s), id, NEON_CYAN.with_alpha(*pulse), None, ring);
        }
        glow(scene, bottom, 1.5 * s, NEON_CYAN, 0.4);
    });
}

/// Dust drifting in the air, glittering where the sun catches it.
fn draw_motes(canvas: &mut Canvas3d, w: f64, time: f64) {
    let cam = canvas.camera;
    let spacing = 1.4;
    let (x0, x1) = cam.visible_x(0.5, w);
    for i in (x0 / spacing).floor() as i64 - 1..=(x1 / spacing).ceil() as i64 + 1 {
        if hash(i, 81) < 0.5 {
            continue;
        }
        let t = time * (0.1 + 0.1 * hash(i, 82)) + hash(i, 83) * TAU;
        let x = (i as f64 + hash(i, 84)) * spacing + t.sin() * 1.2 - time * 0.15;
        let y = cam.eye.y - 3.0 + 7.0 * hash(i, 85) + (t * 1.3).cos() * 0.5;
        let z = -1.0 + 3.0 * hash(i, 86);
        let twinkle = ((time * (1.0 + hash(i, 87)) + hash(i, 88) * 10.0).sin() * 0.5 + 0.5).powi(4);
        let pr = cam.project(DVec3::new(x, y, z));
        let r = (0.02 * pr.scale).max(0.8);
        let alpha = (0.25 + 0.6 * twinkle) as f32;
        canvas.push(pr.depth, move |scene| {
            scene.fill(Fill::NonZero, Affine::IDENTITY, Color::from_rgb8(0xff, 0xd8, 0xa8).with_alpha(alpha), None, &Circle::new(pr.pos, r));
        });
    }
}

/// Near the camera, out of focus: poles with sagging cables, and girders
/// jutting up from below, their edges lit by the sun.
fn draw_foreground(canvas: &mut Canvas3d, w: f64, time: f64) {
    let cam = canvas.camera;
    let z = -5.0;
    let (x0, x1) = cam.visible_x(z, w);
    let spacing = 13.0;
    let dark = Color::from_rgb8(0x18, 0x0c, 0x20);
    for i in (x0 / spacing).floor() as i64 - 1..=(x1 / spacing).ceil() as i64 + 1 {
        let x = (i as f64 + 0.4 * hash(i, 91)) * spacing;
        let kind = hash(i, 92);
        let mut shape = BezPath::new();
        let mut rim = BezPath::new();
        if kind < 0.45 {
            // A pole, and cables sagging to the next one (catenary-ish).
            let top = 7.5 + 1.5 * hash(i, 93);
            let (pole, lit) = post_paths(&cam, DVec3::new(x, -6.0, z), DVec3::new(x, top, z), 0.12);
            shape.extend(pole.iter());
            rim.extend(lit.iter());
            let next = (i as f64 + 1.0 + 0.4 * hash(i + 1, 91)) * spacing;
            let next_top = 7.5 + 1.5 * hash(i + 1, 93);
            for k in 0..2 {
                let (ya, yb) = (top - 0.3 - k as f64 * 0.6, next_top - 0.3 - k as f64 * 0.6);
                let sag = 2.2 + k as f64 * 0.4 + (time * 0.7 + i as f64).sin() * 0.05;
                let mut cable = BezPath::new();
                for j in 0..=16 {
                    let t = j as f64 / 16.0;
                    let q = cam.point(DVec3::new(x + (next - x) * t, ya + (yb - ya) * t - sag * 4.0 * t * (1.0 - t), z));
                    if j == 0 { cable.move_to(q) } else { cable.line_to(q) }
                }
                shape.extend(outline(&cable, 0.05 * cam.project(DVec3::new(x, 0.0, z)).scale).iter());
            }
        } else if kind < 0.7 {
            // A girder with lightening holes, broken off at the top.
            let top = -2.5 + 2.0 * hash(i, 94);
            let lean = (hash(i, 95) - 0.5) * 0.8;
            shape.extend(poly(&cam, &[DVec3::new(x - 0.5, -8.0, z), DVec3::new(x + 0.5, -8.0, z), DVec3::new(x + 0.5 + lean, top - 0.4, z), DVec3::new(x + lean, top, z), DVec3::new(x - 0.5 + lean, top - 0.25, z)]).iter());
            rim.extend(poly(&cam, &[DVec3::new(x + 0.35, -8.0, z), DVec3::new(x + 0.5, -8.0, z), DVec3::new(x + 0.5 + lean, top - 0.4, z), DVec3::new(x + 0.35 + lean, top - 0.3, z)]).iter());
        } else {
            continue;
        }
        let s = cam.project(DVec3::new(x, 0.0, z)).scale;
        let depth = cam.project(DVec3::new(x, 0.0, z)).depth;
        canvas.push(depth, move |scene| {
            let halo = Stroke::new(0.12 * s).with_join(Join::Round);
            scene.stroke(&halo, Affine::IDENTITY, dark.with_alpha(0.3), None, &shape);
            scene.fill(Fill::NonZero, Affine::IDENTITY, dark.with_alpha(0.95), None, &shape);
            scene.fill(Fill::NonZero, Affine::IDENTITY, SUNLIT.with_alpha(0.55), None, &rim);
        });
    }
}
