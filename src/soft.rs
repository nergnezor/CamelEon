//! Soft, living shapes, simulated every frame and drawn in whatever shape
//! they're in (which is where vector graphics shines: nothing is baked):
//!
//! - Jellies: a ring of points held in shape by shape matching (each point
//!   is pulled towards where the rest shape, moved and turned to fit the
//!   ring, says it should be) plus a pressure that keeps their volume, so
//!   they squash, bulge and wobble back. They breathe, hop now and then, look
//!   at Konrad, and bounce him high when he lands on them.
//! - Banners: cloth as a grid of verlet points in 3D, held together by
//!   distance constraints, pinned along the top. The wind makes them flap and
//!   Konrad parts them when he runs through; each cell is shaded by how it
//!   faces the light, which is what shows the folds.

use glam::{DVec2, DVec3};
use vello::kurbo::{Affine, BezPath, Circle, Ellipse, Point, Stroke, Vec2};
use vello::peniko::{Color, Fill, Gradient};

use crate::canvas3d::{darken, lighten, mix, Canvas3d};
use crate::level::{BannerSpec, Level};
use crate::noise::{hash, noise1};

/// Points round a jelly's rim.
const RIM_POINTS: usize = 18;
const GRAVITY: f64 = 20.0;
/// How hard points are pulled towards the matched rest shape, and damped.
const SHAPE_STIFFNESS: f64 = 170.0;
const DAMPING: f64 = 3.5;
/// How hard the jelly pushes back against losing volume.
const PRESSURE: f64 = 45.0;

/// A jelly creature.
pub struct Jelly {
    home: DVec2,
    /// Rim points, their velocities, and the rest shape (offsets from the
    /// centre).
    pts: Vec<DVec2>,
    vel: Vec<DVec2>,
    rest: Vec<DVec2>,
    rest_area: f64,
    /// Turn of the body relative to the rest shape (from shape matching).
    angle: f64,
    seed: u64,
    color: Color,
    /// Seconds until the next hop.
    hop_in: f64,
    /// How squashed it is right now, 0 (at rest) .. 1: for its face.
    squash: f64,
    /// Its height at rest.
    rest_height: f64,
}

/// Where the rest shape puts rim point `i`: a dome with a flat bottom, a
/// little lopsided so no two look alike.
fn rest_point(i: usize, seed: u64) -> DVec2 {
    let a = i as f64 / RIM_POINTS as f64 * std::f64::consts::TAU;
    let lump = 1.0 + 0.08 * noise1(a * 1.2 + seed as f64, seed);
    DVec2::new(a.cos() * 0.72 * lump, (a.sin() * 0.7 * lump).max(-0.42))
}

fn area(pts: &[DVec2]) -> f64 {
    let n = pts.len();
    (0..n).map(|i| pts[i].perp_dot(pts[(i + 1) % n])).sum::<f64>() * 0.5
}

impl Jelly {
    pub fn new(home: DVec2, color: Color, seed: u64) -> Self {
        let rest: Vec<DVec2> = (0..RIM_POINTS).map(|i| rest_point(i, seed)).collect();
        let rest_height = rest.iter().map(|r| r.y).fold(f64::MIN, f64::max) - rest.iter().map(|r| r.y).fold(f64::MAX, f64::min);
        let center = home + DVec2::new(0.0, 0.45);
        Self {
            home,
            pts: rest.iter().map(|r| center + *r).collect(),
            vel: vec![DVec2::ZERO; RIM_POINTS],
            rest_area: area(&rest),
            rest,
            angle: 0.0,
            seed,
            color,
            hop_in: 2.0 + 3.0 * hash(seed as i64, 1),
            squash: 0.0,
            rest_height,
        }
    }

    fn center(&self) -> DVec2 {
        self.pts.iter().copied().sum::<DVec2>() / self.pts.len() as f64
    }

    /// The height of its top at `x`, if `x` is over it.
    fn surface(&self, x: f64) -> Option<f64> {
        let n = self.pts.len();
        let mut top: Option<f64> = None;
        for i in 0..n {
            let (a, b) = (self.pts[i], self.pts[(i + 1) % n]);
            if (a.x - x) * (b.x - x) <= 0.0 && (a.x - b.x).abs() > 1e-9 {
                let y = a.y + (b.y - a.y) * (x - a.x) / (b.x - a.x);
                top = Some(top.map_or(y, |t: f64| t.max(y)));
            }
        }
        top
    }

    /// Konrad falling onto it at `vel_y`: dents it where he lands and
    /// returns how fast he's thrown back up (higher with jump held).
    pub fn stomp(&mut self, feet: DVec2, vel_y: f64, jump_held: bool) -> Option<f64> {
        if vel_y > -1.0 {
            return None;
        }
        let top = self.surface(feet.x)?;
        if feet.y > top + 0.05 || feet.y < top - 0.45 {
            return None;
        }
        for (p, v) in self.pts.iter().zip(&mut self.vel) {
            let near = (1.0 - (p.x - feet.x).abs() / 0.9).max(0.0);
            let upper = if p.y > top - 0.5 { 1.0 } else { 0.3 };
            v.y += vel_y * 0.55 * near * upper;
            v.x += (p.x - feet.x).signum() * near * 1.5;
        }
        let boost = if jump_held { 1.25 } else { 1.0 };
        Some(((-vel_y * 0.9).max(14.0) * boost).min(25.0))
    }

    /// A shot hitting it at `at`: the points near it are knocked along
    /// `push`. Returns whether it was hit.
    pub fn poke(&mut self, at: DVec2, push: DVec2) -> bool {
        if self.center().distance(at) > self.rest_height * 0.6 {
            return false;
        }
        for (p, v) in self.pts.iter().zip(&mut self.vel) {
            let near = (1.0 - p.distance(at) / 1.0).max(0.0);
            *v += push * near;
        }
        true
    }

    pub fn update(&mut self, dt: f64, level: &Level, time: f64, player: (DVec2, DVec2)) {
        let c = self.center();
        // Shape matching: the rotation that best fits the rest shape to
        // where the points are now.
        let (mut s, mut k) = (0.0, 0.0);
        for (p, r) in self.pts.iter().zip(&self.rest) {
            let q = *p - c;
            s += r.perp_dot(q);
            k += r.dot(q);
        }
        self.angle = s.atan2(k);
        let rot = DVec2::from_angle(self.angle);
        // Breathing: the rest shape swells and shrinks a little.
        let breath = 1.0 + 0.035 * (time * 2.1 + self.seed as f64).sin();
        // Pressure: push out along the normals when squeezed.
        let lost = (self.rest_area * breath * breath - area(&self.pts)) / self.rest_area;
        let n = self.pts.len();
        let normals: Vec<DVec2> = (0..n)
            .map(|i| {
                let t = self.pts[(i + 1) % n] - self.pts[(i + n - 1) % n];
                DVec2::new(t.y, -t.x).normalize_or_zero()
            })
            .collect();
        for i in 0..n {
            let goal = c + rot.rotate(self.rest[i] * breath);
            let v = &mut self.vel[i];
            *v += ((goal - self.pts[i]) * SHAPE_STIFFNESS + normals[i] * lost * PRESSURE - DVec2::Y * GRAVITY) * dt;
            *v *= (-DAMPING * dt).exp();
        }
        // Now and then it hops, drifting back towards home.
        let grounded = self.pts.iter().any(|p| on_block(level, *p));
        self.hop_in -= dt;
        if self.hop_in <= 0.0 && grounded {
            self.hop_in = 2.5 + 4.0 * hash((time * 10.0) as i64, self.seed);
            let toward = ((self.home.x - c.x) * 1.2).clamp(-2.5, 2.5);
            for (p, v) in self.pts.iter().zip(&mut self.vel) {
                // The bottom kicks off harder, so it stretches as it leaves.
                let kick = if p.y < c.y { 8.5 } else { 6.5 };
                v.y += kick;
                v.x += toward;
            }
        }
        // Konrad brushing past (he passes in front of it) sets it jiggling.
        let (feet, pvel) = player;
        for (p, v) in self.pts.iter().zip(&mut self.vel) {
            let dx = p.x - feet.x;
            if dx.abs() < 0.4 && p.y > feet.y + 0.1 && p.y < feet.y + 1.7 {
                let near = 1.0 - dx.abs() / 0.4;
                v.x += (pvel.x * 0.12 + dx.signum() * 1.2) * near * dt * 30.0;
            }
        }
        for (p, v) in self.pts.iter_mut().zip(&mut self.vel) {
            *p += *v * dt;
            collide(level, p, v);
        }
        // Back home if it ever falls off the world.
        if self.center().y < level.kill_y {
            *self = Jelly::new(self.home, self.color, self.seed);
            return;
        }
        let height = self.pts.iter().map(|p| p.y).fold(f64::MIN, f64::max) - self.pts.iter().map(|p| p.y).fold(f64::MAX, f64::min);
        // Sagging a little under its own weight is resting, not squashed.
        let target = ((0.88 - height / self.rest_height) * 4.0).clamp(0.0, 1.0);
        self.squash += (target - self.squash) * (1.0 - (-dt * 20.0).exp());
    }
}

/// Whether `p` is resting on (or just in) the top of a block or a hill.
fn on_block(level: &Level, p: DVec2) -> bool {
    level.blocks.iter().any(|b| p.x >= b.x0 && p.x <= b.x1 && p.y <= b.y1 + 0.03 && p.y > b.y1 - 0.3)
        || level.hills.iter().any(|h| h.contains(p.x) && (p.y - h.height(p.x)).abs() < 0.03 + 0.15)
}

/// Pushes a point out of the level's blocks and hills, with some friction.
fn collide(level: &Level, p: &mut DVec2, v: &mut DVec2) {
    for h in &level.hills {
        if !h.contains(p.x) {
            continue;
        }
        let top = h.height(p.x);
        if p.y < top && p.y > top - 1.5 {
            p.y = top;
            v.y = v.y.max(0.0);
            v.x *= 0.85;
        }
    }
    for b in &level.blocks {
        if p.x <= b.x0 || p.x >= b.x1 || p.y <= b.y0 || p.y >= b.y1 {
            continue;
        }
        let up = b.y1 - p.y;
        if b.one_way() && (v.y > 0.0 || up > 0.3) {
            continue;
        }
        let (left, right, down) = (p.x - b.x0, b.x1 - p.x, p.y - b.y0);
        let least = up.min(left).min(right).min(down);
        if least == up {
            p.y = b.y1;
            v.y = v.y.max(0.0);
            v.x *= 0.85;
        } else if least == left {
            p.x = b.x0;
            v.x = v.x.min(0.0);
        } else if least == right {
            p.x = b.x1;
            v.x = v.x.max(0.0);
        } else {
            p.y = b.y0;
            v.y = v.y.min(0.0);
        }
    }
}

/// How a jelly or banner is lit: the sun's direction on screen and in the
/// world, and the colour of its light on edges.
#[derive(Clone, Copy)]
pub struct Light {
    pub screen: Vec2,
    pub world: DVec3,
    pub rim: Color,
}

/// Draws a jelly: see-through, with a darker nucleus and bubbles inside, a
/// highlight, sunlight on its rim, and a face that watches Konrad.
pub fn draw_jelly(canvas: &mut Canvas3d, jelly: &Jelly, time: f64, light: Light, konrad: DVec2) {
    let cam = canvas.camera;
    let z = 0.15;
    let pts: Vec<Point> = jelly.pts.iter().map(|p| cam.point(DVec3::new(p.x, p.y, z))).collect();
    let body = crate::konrad::smooth_closed(&pts);
    let c = jelly.center();
    let pc = cam.project(DVec3::new(c.x, c.y, z));
    let s = pc.scale;
    let rot = DVec2::from_angle(jelly.angle);
    // Points on the body in its own frame, so the face turns and squashes
    // with it.
    let squash = 1.0 - 0.45 * jelly.squash;
    let local = |x: f64, y: f64| {
        let q = c + rot.rotate(DVec2::new(x * (1.0 + 0.3 * jelly.squash), y * squash));
        cam.point(DVec3::new(q.x, q.y, z - 0.01))
    };
    let look = (konrad + DVec2::new(0.0, 1.2) - c).normalize_or_zero();
    let blink = (time * 0.9 + jelly.seed as f64 * 0.7).rem_euclid(4.0) < 0.12;
    let eyes: Vec<(Point, Point)> = [-0.2, 0.2]
        .iter()
        .map(|&ex| {
            let e = local(ex, 0.12);
            (e, e + Vec2::new(look.x, -look.y) * 0.035 * s)
        })
        .collect();
    // A small mouth: a smile, a surprised "o" when squashed.
    let mouth_c = local(0.0, -0.08);
    let open = jelly.squash;
    let bubbles: Vec<(Point, f64)> = (0..3)
        .map(|k| {
            let t = (time * (0.25 + 0.1 * k as f64) + hash(k, jelly.seed)).fract();
            let x = (hash(k, jelly.seed + 1) - 0.5) * 0.7 + (time * 1.3 + k as f64).sin() * 0.05;
            (local(x, -0.3 + t * 0.6), (0.025 + 0.02 * hash(k, jelly.seed + 2)) * s * (1.0 - t * 0.5))
        })
        .collect();
    let nucleus = local(0.05 * (time * 0.7).sin(), -0.1);
    let hot = pc.pos + light.screen * 0.3 * s + Vec2::new(0.0, -0.25 * s);
    let color = jelly.color;
    let shadow_c = cam.point(DVec3::new(c.x, jelly.pts.iter().map(|p| p.y).fold(f64::MAX, f64::min), z));
    let span = (jelly.pts.iter().map(|p| p.x).fold(f64::MIN, f64::max) - jelly.pts.iter().map(|p| p.x).fold(f64::MAX, f64::min)) * s;
    canvas.push(pc.depth, move |scene| {
        let id = Affine::IDENTITY;
        let contact = Gradient::new_radial(shadow_c, (span * 0.55) as f32).with_stops([Color::BLACK.with_alpha(0.3), Color::BLACK.with_alpha(0.0)]);
        scene.fill(Fill::NonZero, id, &contact, Some(Affine::translate((0.0, 0.0))), &Ellipse::new(shadow_c, (span * 0.55, span * 0.12), 0.0));
        // The body: brighter where the light comes through.
        let fill = Gradient::new_two_point_radial(hot, 0.0_f32, pc.pos, (0.85 * s) as f32).with_stops([
            (0.0, lighten(color, 0.55).with_alpha(0.9)),
            (0.45, color.with_alpha(0.78)),
            (1.0, darken(color, 0.3).with_alpha(0.92)),
        ]);
        scene.fill(Fill::NonZero, id, &fill, None, &body);
        scene.push_clip_layer(Fill::NonZero, id, &body);
        scene.fill(Fill::NonZero, id, darken(color, 0.35).with_alpha(0.35), None, &Ellipse::new(nucleus, (0.22 * s, 0.15 * s), 0.2));
        for &(b, r) in &bubbles {
            scene.stroke(&Stroke::new(0.012 * s), id, lighten(color, 0.7).with_alpha(0.7), None, &Circle::new(b, r));
        }
        // Light on the rim facing the sun.
        let rim_w = 0.09 * s;
        scene.stroke(&Stroke::new(rim_w), Affine::translate(-light.screen * rim_w * 0.8), light.rim.with_alpha(0.75), None, &body);
        scene.pop_layer();
        scene.stroke(&Stroke::new(0.02 * s), id, darken(color, 0.45).with_alpha(0.6), None, &body);
        scene.fill(Fill::NonZero, id, Color::WHITE.with_alpha(0.75), None, &Ellipse::new(hot, (0.1 * s, 0.05 * s), -0.4));
        // The face.
        for &(e, pupil) in &eyes {
            if blink {
                let mut lid = BezPath::new();
                lid.move_to(e - Vec2::new(0.06 * s, 0.0));
                lid.line_to(e + Vec2::new(0.06 * s, 0.0));
                scene.stroke(&Stroke::new(0.02 * s), id, darken(color, 0.7), None, &lid);
            } else {
                scene.fill(Fill::NonZero, id, Color::WHITE, None, &Ellipse::new(e, (0.07 * s, 0.085 * s), 0.0));
                scene.fill(Fill::NonZero, id, Color::from_rgb8(0x18, 0x10, 0x20), None, &Circle::new(pupil, 0.038 * s));
                scene.fill(Fill::NonZero, id, Color::WHITE, None, &Circle::new(pupil + Vec2::new(-0.012 * s, -0.015 * s), 0.012 * s));
            }
        }
        if open > 0.25 {
            scene.fill(Fill::NonZero, id, darken(color, 0.7), None, &Ellipse::new(mouth_c, (0.04 * s, 0.05 * s * open), 0.0));
        } else {
            let mut smile = BezPath::new();
            smile.move_to(mouth_c + Vec2::new(-0.06 * s, 0.0));
            smile.quad_to(mouth_c + Vec2::new(0.0, 0.05 * s), mouth_c + Vec2::new(0.06 * s, 0.0));
            scene.stroke(&Stroke::new(0.018 * s), id, darken(color, 0.7), None, &smile);
        }
    });
}

impl Jelly {
    /// Its outline in 3D, a little thick, to cast a shadow.
    pub fn caster(&self) -> Vec<DVec3> {
        self.pts.iter().flat_map(|p| [DVec3::new(p.x, p.y, -0.2), DVec3::new(p.x, p.y, 0.5)]).collect()
    }

    /// Its bottom and top, for its shadow's fade.
    pub fn span(&self) -> (DVec3, DVec3) {
        span(self.pts.iter().map(|p| DVec3::new(p.x, p.y, 0.15)))
    }
}

/// The lowest and highest of some points (their middle in x and z).
fn span(pts: impl Iterator<Item = DVec3>) -> (DVec3, DVec3) {
    let (mut lo, mut hi, mut sum, mut n) = (f64::MAX, f64::MIN, DVec3::ZERO, 0.0_f64);
    for p in pts {
        lo = lo.min(p.y);
        hi = hi.max(p.y);
        sum += p;
        n += 1.0;
    }
    let mid = sum / n.max(1.0);
    (DVec3::new(mid.x, lo, mid.z), DVec3::new(mid.x, hi, mid.z))
}

/// A cloth banner hanging from a catwalk.
pub struct Banner {
    cols: usize,
    rows: usize,
    pos: Vec<DVec3>,
    prev: Vec<DVec3>,
    rest_dx: f64,
    rest_dy: f64,
    colors: [Color; 2],
    seed: u64,
}

impl Banner {
    pub fn new(spec: &BannerSpec, seed: u64) -> Self {
        let cols = 7;
        let rows = ((spec.length / 0.22).round() as usize).max(4);
        let rest_dx = (spec.x1 - spec.x0) / (cols - 1) as f64;
        let rest_dy = spec.length / (rows - 1) as f64;
        let pos: Vec<DVec3> = (0..rows)
            .flat_map(|r| (0..cols).map(move |c| DVec3::new(spec.x0 + c as f64 * rest_dx, spec.top - r as f64 * rest_dy, spec.z)))
            .collect();
        Self { cols, rows, prev: pos.clone(), pos, rest_dx, rest_dy, colors: spec.colors, seed }
    }

    fn at(&self, c: usize, r: usize) -> usize {
        r * self.cols + c
    }

    /// Steps the cloth: verlet with wind and gravity, then the lengths
    /// between neighbours restored a few times over. `player` is Konrad's
    /// feet and velocity; he pushes the cloth aside.
    pub fn update(&mut self, dt: f64, wind: f64, time: f64, player: (DVec2, DVec2)) {
        let damping = 0.985;
        let (feet, pvel) = player;
        for i in self.cols..self.pos.len() {
            let p = self.pos[i];
            // Gusts ripple through the cloth: noise in time and height.
            let gust = noise1(time * 0.9 + p.y * 0.4 + self.seed as f64, self.seed);
            let flutter = noise1(time * 3.1 + p.x * 1.7 + p.y * 0.9, self.seed + 1);
            let force = DVec3::new(1.2 + 3.0 * wind * (0.6 + gust), -GRAVITY * 0.6, (1.5 + 2.5 * wind) * flutter);
            let next = p + (p - self.prev[i]) * damping + force * dt * dt;
            self.prev[i] = p;
            self.pos[i] = next;
        }
        for _ in 0..4 {
            for r in 0..self.rows {
                for c in 0..self.cols {
                    if c + 1 < self.cols {
                        self.satisfy(self.at(c, r), self.at(c + 1, r), self.rest_dx);
                    }
                    if r + 1 < self.rows {
                        self.satisfy(self.at(c, r), self.at(c, r + 1), self.rest_dy);
                    }
                }
            }
        }
        // Konrad goes through: the cloth is pushed back behind him, dragged
        // along a little, and swings back once he's past.
        // (In verlet a moved point keeps the move as velocity, so the
        // previous positions go along with most of it, or the cloth would be
        // flung away.)
        for i in self.cols..self.pos.len() {
            let (p, prev) = (&mut self.pos[i], &mut self.prev[i]);
            let dx = p.x - feet.x;
            if dx.abs() < 0.4 && p.y > feet.y && p.y < feet.y + 1.9 && p.z < 0.55 {
                let push = DVec3::new(pvel.x * dt * 0.15, 0.0, (0.55 - p.z) * 0.3);
                *p += push;
                *prev += push * 0.8;
            }
        }
    }

    fn satisfy(&mut self, a: usize, b: usize, rest: f64) {
        let d = self.pos[b] - self.pos[a];
        let len = d.length();
        if len < 1e-9 {
            return;
        }
        let fix = d * ((len - rest) / len);
        // The top row is pinned to the catwalk.
        match (a < self.cols, b < self.cols) {
            (true, true) => {}
            (true, false) => self.pos[b] -= fix,
            (false, true) => self.pos[a] += fix,
            (false, false) => {
                self.pos[a] += fix * 0.5;
                self.pos[b] -= fix * 0.5;
            }
        }
    }

    /// Its points, to cast a shadow.
    pub fn caster(&self) -> Vec<DVec3> {
        self.pos.clone()
    }

    /// Its bottom and top, for its shadow's fade.
    pub fn span(&self) -> (DVec3, DVec3) {
        span(self.pos.iter().copied())
    }
}

/// Draws a banner: two-colour stripes, each cell shaded by how squarely it
/// faces the light (the folds), the back side darker, a rod along the top and
/// tassels along the bottom.
pub fn draw_banner(canvas: &mut Canvas3d, banner: &Banner, light: Light) {
    const SHADES: usize = 7;
    let cam = canvas.camera;
    // One path per stripe colour and shade, so the whole banner is a
    // handful of fills.
    let mut cells: Vec<BezPath> = vec![BezPath::new(); 2 * SHADES];
    let l = light.world.normalize();
    for r in 0..banner.rows - 1 {
        for c in 0..banner.cols - 1 {
            let (a, b, cc, d) = (banner.pos[banner.at(c, r)], banner.pos[banner.at(c + 1, r)], banner.pos[banner.at(c + 1, r + 1)], banner.pos[banner.at(c, r + 1)]);
            let normal = (b - a).cross(d - a).normalize_or_zero();
            // Lit from either side (thin cloth lets light through), but the
            // side turned away from the camera is in its own shade.
            let lit = normal.dot(l).abs() * if normal.z > 0.0 { 0.6 } else { 1.0 };
            let shade = ((lit * SHADES as f64) as usize).min(SHADES - 1);
            let stripe = usize::from(c % 3 == 1);
            let path = &mut cells[stripe * SHADES + shade];
            path.move_to(cam.point(a));
            path.line_to(cam.point(b));
            path.line_to(cam.point(cc));
            path.line_to(cam.point(d));
            path.close_path();
        }
    }
    let first = banner.pos[0];
    let last = banner.pos[banner.cols - 1];
    let mut rod = BezPath::new();
    rod.move_to(cam.point(first - DVec3::X * 0.1));
    rod.line_to(cam.point(last + DVec3::X * 0.1));
    let mut tassels = BezPath::new();
    for c in 0..banner.cols {
        let p = banner.pos[banner.at(c, banner.rows - 1)];
        let above = banner.pos[banner.at(c, banner.rows - 2)];
        tassels.move_to(cam.point(p));
        tassels.line_to(cam.point(p + (p - above).normalize_or_zero() * 0.15));
    }
    let s = cam.project(first).scale;
    let colors = banner.colors;
    let mid = banner.pos[banner.pos.len() / 2];
    let depth = canvas.depth_of(DVec3::new(mid.x, mid.y, mid.z.max(0.05)));
    canvas.push(depth, move |scene| {
        let id = Affine::IDENTITY;
        for (i, path) in cells.iter().enumerate() {
            let (stripe, shade) = (i / SHADES, i % SHADES);
            let k = shade as f64 / (SHADES - 1) as f64;
            let color = mix(darken(colors[stripe], 0.55), lighten(colors[stripe], 0.15), k);
            scene.fill(Fill::NonZero, id, color, None, path);
            // Seams between the cells would show as hairlines otherwise.
            scene.stroke(&Stroke::new(0.6), id, color, None, path);
        }
        scene.stroke(&Stroke::new(0.03 * s), id, darken(colors[0], 0.5), None, &tassels);
        scene.stroke(&Stroke::new(0.07 * s), id, Color::from_rgb8(0x2e, 0x24, 0x36), None, &rod);
    });
}
