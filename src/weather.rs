//! Weather and ambient life: rain that comes and goes (with splashes where it
//! lands), wind, drifting fog, mist in the pits, falling leaves, fireflies
//! and the odd flock of birds. Everything is procedural from time and
//! position, so it needs no per-object state.

use std::f64::consts::TAU;

use glam::DVec3;
use vello::kurbo::{Affine, BezPath, Circle, Ellipse, Line, Point, Rect, Stroke, Vec2};
use vello::peniko::{Color, Fill, Gradient};
use vello::Scene;

use crate::canvas3d::{Camera, Canvas3d};
use crate::level::Level;

/// How the weather is chosen.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    /// Showers come and go.
    Auto,
    Clear,
    Rain,
}

pub struct Weather {
    pub mode: Mode,
    /// 0 = dry, 1 = pouring; eases towards the target.
    pub rain: f64,
    /// 0 calm .. 1 gusty.
    pub wind: f64,
}

impl Weather {
    pub fn new() -> Self {
        Self { mode: Mode::Auto, rain: 0.0, wind: 0.4 }
    }

    /// Clear → rain → auto.
    pub fn cycle_mode(&mut self) {
        self.mode = match self.mode {
            Mode::Auto => Mode::Clear,
            Mode::Clear => Mode::Rain,
            Mode::Rain => Mode::Auto,
        };
    }

    pub fn update(&mut self, dt: f64, time: f64) {
        let target = match self.mode {
            Mode::Clear => 0.0,
            Mode::Rain => 1.0,
            // A shower for about 30 s out of every 90, the first after a while.
            Mode::Auto => {
                let phase = ((time + 25.0) / 90.0).fract();
                if phase > 0.62 { 1.0 } else { 0.0 }
            }
        };
        // Rain takes a few seconds to build up or clear.
        let k = 1.0 - (-dt / 4.0).exp();
        self.rain += (target - self.rain) * k;
        // Gusts on top of a breeze that picks up with the rain.
        let gust = 0.5 + 0.5 * ((time * 0.23).sin() * 0.6 + (time * 0.61).sin() * 0.4);
        self.wind = (0.25 + 0.35 * gust + 0.4 * self.rain).clamp(0.0, 1.0);
    }
}

/// Deterministic pseudo-random number in 0..1.
fn hash(i: i64, seed: u64) -> f64 {
    let mut x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ seed.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// Birds crossing the far sky now and then (far layer, half resolution).
pub fn draw_birds(scene: &mut Scene, cam: &Camera, w: f64, h: f64, time: f64, rain: f64) {
    // One flock every 40 s, crossing in about 14 s; none in heavy rain.
    let period = 40.0;
    let t = time % period;
    if t > 14.0 || rain > 0.6 {
        return;
    }
    let flock = (time / period).floor() as i64;
    let dir = if hash(flock, 1) < 0.5 { 1.0 } else { -1.0 };
    let progress = t / 14.0;
    let y = h * (0.12 + 0.2 * hash(flock, 2));
    let x0 = if dir > 0.0 { -0.1 * w } else { 1.1 * w };
    let lead = Point::new(x0 + dir * progress * 1.2 * w - cam.eye.x * 0.3, y);
    let size = h * 0.012;
    let mut birds = BezPath::new();
    for b in 0..7 {
        // V formation.
        let rank = ((b + 1) / 2) as f64;
        let side = if b % 2 == 0 { 1.0 } else { -1.0 };
        let p = lead + Vec2::new(-dir * rank * size * 3.0, side * rank * size * 1.8);
        let flap = (time * 9.0 + b as f64 * 1.3).sin();
        let wing = Vec2::new(size, -size * 0.9 * flap);
        birds.move_to(p - Vec2::new(wing.x, 0.0) + Vec2::new(0.0, wing.y));
        birds.quad_to(p + Vec2::new(-wing.x * 0.3, -size * 0.2), p);
        birds.quad_to(p + Vec2::new(wing.x * 0.3, -size * 0.2), p + Vec2::new(wing.x, wing.y));
    }
    scene.stroke(&Stroke::new(size * 0.35), Affine::IDENTITY, Color::from_rgb8(0x1a, 0x24, 0x34).with_alpha(0.8), None, &birds);
}

/// Fog banks drifting with the wind through the middle distance.
pub fn draw_fog(scene: &mut Scene, cam: &Camera, w: f64, time: f64, rain: f64, wind: f64) {
    let amount = 0.2 + 0.8 * rain;
    let z = 12.0;
    let (x0, x1) = cam.visible_x(z, w);
    let spacing = 9.0;
    let drift = time * (0.4 + wind * 1.4);
    let first = ((x0 - drift) / spacing).floor() as i64 - 1;
    let last = ((x1 - drift) / spacing).ceil() as i64 + 1;
    for i in first..=last {
        let x = (i as f64 + hash(i, 11)) * spacing + drift;
        let y = -1.5 + 3.0 * hash(i, 12);
        let p = cam.project(DVec3::new(x, y, z));
        let rx = (4.0 + 3.0 * hash(i, 13)) * p.scale;
        let ry = rx * 0.28;
        let alpha = (0.22 + 0.2 * hash(i, 14)) * amount;
        let tint = Color::from_rgb8(0x8c, 0xa2, 0xb0);
        let fog = Gradient::new_radial((0.0, 0.0), 1.0).with_stops([
            (0.0, tint.with_alpha(alpha as f32)),
            (0.6, tint.with_alpha((alpha * 0.5) as f32)),
            (1.0, tint.with_alpha(0.0)),
        ]);
        let shape = Affine::translate(p.pos.to_vec2()) * Affine::scale_non_uniform(rx, ry);
        scene.fill(Fill::NonZero, Affine::IDENTITY, &fog, Some(shape), &Ellipse::new(p.pos, (rx, ry), 0.0));
    }
}

/// Mist pooling in the pits, below ground level (front layer, drawn before
/// the platforms so it only shows where there's no ground).
pub fn draw_pit_mist(scene: &mut Scene, cam: &Camera, w: f64, h: f64, rain: f64) {
    let top = cam.point(DVec3::new(0.0, -0.3, 0.5)).y;
    let bottom = cam.point(DVec3::new(0.0, -7.0, 0.5)).y;
    if top > h {
        return;
    }
    let tint = Color::from_rgb8(0x6e, 0x86, 0x94);
    let mist = Gradient::new_linear((0.0, top), (0.0, bottom)).with_stops([
        (0.0, tint.with_alpha(0.0)),
        (0.35, tint.with_alpha((0.35 + 0.2 * rain) as f32)),
        (1.0, tint.with_alpha((0.8 + 0.1 * rain) as f32)),
    ]);
    scene.fill(Fill::NonZero, Affine::IDENTITY, &mist, None, &Rect::new(0.0, top, w, h.max(bottom)));
}

/// Leaves drifting down, tumbling and swaying with the wind.
pub fn draw_leaves(canvas: &mut Canvas3d, w: f64, time: f64, wind: f64, z_range: (f64, f64), seed: u64) {
    let cam = canvas.camera;
    let spacing = 3.5;
    let z = (z_range.0 + z_range.1) / 2.0;
    let (x0, x1) = cam.visible_x(z, w);
    let colors = [
        Color::from_rgb8(0x5a, 0x7a, 0x32),
        Color::from_rgb8(0x8a, 0x7a, 0x2e),
        Color::from_rgb8(0x9a, 0x52, 0x2a),
        Color::from_rgb8(0x3e, 0x62, 0x36),
    ];
    for i in (x0 / spacing).floor() as i64 - 2..=(x1 / spacing).ceil() as i64 + 2 {
        if hash(i, seed) < 0.45 {
            continue;
        }
        // Each leaf falls from the canopy to the ground over its period.
        let period = 9.0 + 6.0 * hash(i, seed + 1);
        let phase = ((time + hash(i, seed + 2) * period) / period).fract();
        let fall = 11.0;
        let y = 9.0 - phase * fall;
        let drift = phase * fall * wind * 0.8;
        let x = (i as f64 + hash(i, seed + 3)) * spacing + (time * 1.3 + i as f64).sin() * 0.6 + drift;
        let lz = z_range.0 + (z_range.1 - z_range.0) * hash(i, seed + 4);
        let pr = cam.project(DVec3::new(x, y, lz));
        let size = 0.13 * pr.scale;
        let spin = time * (1.5 + hash(i, seed + 5) * 2.0) + i as f64;
        let color = colors[(hash(i, seed + 6) * 4.0) as usize % 4];
        // Tumbling: the leaf's width flips as it turns.
        let leaf = Ellipse::new(pr.pos, (size, size * 0.45 * spin.cos().abs().max(0.15)), spin * 0.7);
        canvas.push(pr.depth, move |scene| {
            scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &leaf);
        });
    }
}

/// Fireflies near the ground, glowing on and off.
pub fn draw_fireflies(canvas: &mut Canvas3d, w: f64, time: f64, rain: f64) {
    let cam = canvas.camera;
    let spacing = 2.2;
    let (x0, x1) = cam.visible_x(0.5, w);
    for i in (x0 / spacing).floor() as i64 - 1..=(x1 / spacing).ceil() as i64 + 1 {
        if hash(i, 31) < 0.4 {
            continue;
        }
        let t = time * (0.35 + 0.2 * hash(i, 32)) + hash(i, 33) * TAU;
        let x = (i as f64 + hash(i, 34)) * spacing + t.sin() * 0.9;
        let y = 0.5 + 2.2 * hash(i, 35) + (t * 1.7).cos() * 0.35;
        let z = -0.6 + 1.8 * hash(i, 36);
        // Slow blinks, dimmer in the rain.
        let blink = ((time * (0.8 + hash(i, 37)) + hash(i, 38) * 10.0).sin() * 0.5 + 0.5).powi(3) * (1.0 - 0.7 * rain);
        if blink < 0.02 {
            continue;
        }
        let pr = cam.project(DVec3::new(x, y, z));
        let r = 0.28 * pr.scale;
        canvas.push(pr.depth, move |scene| {
            let glow = Gradient::new_radial(pr.pos, r as f32).with_stops([
                Color::from_rgb8(0xe8, 0xff, 0x8a).with_alpha((0.55 * blink) as f32),
                Color::from_rgb8(0xe8, 0xff, 0x8a).with_alpha(0.0),
            ]);
            scene.fill(Fill::NonZero, Affine::IDENTITY, &glow, None, &Circle::new(pr.pos, r));
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Color::from_rgb8(0xfa, 0xff, 0xd8).with_alpha(blink as f32),
                None,
                &Circle::new(pr.pos, (r * 0.12).max(1.0)),
            );
        });
    }
}

/// Falling rain in three depths (screen-space streaks that parallax with the
/// camera), plus splashes on the tops of the platforms.
pub fn draw_rain(canvas: &mut Canvas3d, level: &Level, w: f64, h: f64, time: f64, rain: f64, wind: f64) {
    if rain < 0.02 {
        return;
    }
    let cam = canvas.camera;
    let slant = 0.15 + wind * 0.45;
    // Streaks, near to far: faster, longer and brighter up close.
    for (layer, depth) in [(0u64, 0.6), (1, 1.0), (2, 1.8)] {
        let count = (140.0 * rain) as i64;
        let speed = h * 1.6 / depth;
        let len = h * 0.05 / depth;
        let parallax = cam.eye.x * cam.focal / (12.0 * depth);
        let mut streaks = BezPath::new();
        for i in 0..count {
            let seed = layer * 1000;
            let x = ((hash(i, seed + 1) * (w + 200.0) - parallax + time * speed * slant).rem_euclid(w + 200.0)) - 100.0;
            let y = (hash(i, seed + 2) * (h + len) + time * speed).rem_euclid(h + len) - len;
            let a = Point::new(x, y);
            streaks.move_to(a);
            streaks.line_to(a + Vec2::new(len * slant, len));
        }
        let alpha = (0.18 + 0.12 / depth) * rain;
        let color = Color::from_rgb8(0xc8, 0xd8, 0xe8).with_alpha(alpha as f32);
        let width = (1.6 / depth).max(0.6);
        // Far rain behind the world, near rain in front of everything.
        let order = if depth < 1.0 { -100.0 } else { 1000.0 };
        canvas.push(order, move |scene| {
            scene.stroke(&Stroke::new(width), Affine::IDENTITY, color, None, &streaks);
        });
    }

    // Splashes: little rings popping up on the tops of the platforms.
    let (vx0, vx1) = cam.visible_x(0.0, w);
    for (bi, b) in level.blocks.iter().enumerate() {
        let (x0, x1) = (b.x0.max(vx0), b.x1.min(vx1));
        if x1 <= x0 {
            continue;
        }
        let count = ((x1 - x0) * 1.2 * rain) as i64;
        for i in 0..count {
            let cycle = 0.45;
            let slot = ((time + hash(i, bi as u64 + 51) * cycle) / cycle).floor() as i64;
            let t = ((time + hash(i, bi as u64 + 51) * cycle) / cycle).fract();
            let x = x0 + (x1 - x0) * hash(slot * 131 + i, bi as u64 + 52);
            let z = b.z0 + (b.z1 - b.z0).min(1.5) * hash(slot * 131 + i, bi as u64 + 53);
            let pr = cam.project(DVec3::new(x, b.y1, z));
            let r = (0.06 + 0.18 * t) * pr.scale;
            let alpha = (1.0 - t) * 0.6;
            let ring = Ellipse::new(pr.pos, (r, r * 0.3), 0.0);
            let drop = Line::new(pr.pos - Vec2::new(0.0, 0.25 * pr.scale * (1.0 - t)), pr.pos - Vec2::new(0.0, 0.05 * pr.scale));
            canvas.push(pr.depth - 0.01, move |scene| {
                let color = Color::from_rgb8(0xd8, 0xe6, 0xf0).with_alpha(alpha as f32);
                scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, color, None, &ring);
                if t < 0.35 {
                    scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, color, None, &drop);
                }
            });
        }
    }
}
