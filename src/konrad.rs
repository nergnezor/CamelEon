//! Konrad: the hero, an agent in the style of early-90s cinematic platformers
//! (think Flashback): realistic proportions, smooth lifelike movement, and flat
//! colour shading without outlines. Brown hair, an off-white long-sleeved
//! shirt, grey-blue jeans, boots and a belt with a holster. He fires a
//! grappling line from his right hand.
//!
//! Konrad is a 3D rig (see `rig`) drawn as a few smooth 2D body shapes over
//! the projected skeleton, in a fixed back-to-front order.
//! Model space: x = his right, y = up, z = forward. Feet at the origin.

use std::f64::consts::PI;

use glam::{DQuat, DVec3};
use vello::kurbo::{Affine, BezPath, Ellipse, Point, Shape, Stroke, Vec2};
use vello::peniko::{Color, Fill};

use crate::canvas3d::Canvas3d;
use crate::rig::{Bone, Pose, Root, Skeleton, Solved};

/// Bone indices.
pub mod bone {
    pub const ROOT: usize = 0;
    pub const PELVIS: usize = 1;
    pub const SPINE: usize = 2;
    pub const CHEST: usize = 3;
    pub const NECK: usize = 4;
    pub const HEAD: usize = 5;
    pub const SHOULDER_L: usize = 6;
    pub const ELBOW_L: usize = 7;
    pub const HAND_L: usize = 8;
    pub const SHOULDER_R: usize = 9;
    pub const ELBOW_R: usize = 10;
    pub const HAND_R: usize = 11;
    pub const HIP_L: usize = 12;
    pub const KNEE_L: usize = 13;
    pub const FOOT_L: usize = 14;
    pub const HIP_R: usize = 15;
    pub const KNEE_R: usize = 16;
    pub const FOOT_R: usize = 17;
    pub const COUNT: usize = 18;
}
use bone::*;

const SHIRT: Color = Color::from_rgb8(0xd6, 0xd2, 0xc6);
const JEANS: Color = Color::from_rgb8(0x4b, 0x58, 0x74);
const BOOT: Color = Color::from_rgb8(0x4a, 0x32, 0x22);
const BELT: Color = Color::from_rgb8(0x2e, 0x20, 0x18);
const SKIN: Color = Color::from_rgb8(0xd4, 0x9c, 0x7a);
const HAIR: Color = Color::from_rgb8(0x4a, 0x2c, 0x1a);
const FEATURE: Color = Color::from_rgb8(0x2a, 0x1c, 0x18);

/// Height of the soles below the ankle bone.
const SOLE: f64 = 0.035;

pub fn skeleton() -> Skeleton {
    let b = |parent: usize, x: f64, y: f64, z: f64| Bone {
        parent: Some(parent),
        offset: DVec3::new(x, y, z),
    };
    let bones = vec![
        Bone { parent: None, offset: DVec3::ZERO }, // ROOT
        b(ROOT, 0.0, 0.95, 0.0),                    // PELVIS
        b(PELVIS, 0.0, 0.18, 0.0),                  // SPINE
        b(SPINE, 0.0, 0.2, 0.0),                    // CHEST
        b(CHEST, 0.0, 0.2, 0.01),                   // NECK
        b(NECK, 0.0, 0.14, 0.02),                   // HEAD
        b(CHEST, -0.19, 0.14, 0.0),                 // SHOULDER_L
        b(SHOULDER_L, 0.0, -0.29, 0.0),             // ELBOW_L
        b(ELBOW_L, 0.0, -0.27, 0.0),                // HAND_L
        b(CHEST, 0.19, 0.14, 0.0),                  // SHOULDER_R
        b(SHOULDER_R, 0.0, -0.29, 0.0),             // ELBOW_R
        b(ELBOW_R, 0.0, -0.27, 0.0),                // HAND_R
        b(PELVIS, -0.095, -0.04, 0.0),              // HIP_L
        b(HIP_L, 0.0, -0.44, 0.0),                  // KNEE_L
        b(KNEE_L, 0.0, -0.44, 0.0),                 // FOOT_L
        b(PELVIS, 0.095, -0.04, 0.0),               // HIP_R
        b(HIP_R, 0.0, -0.44, 0.0),                  // KNEE_R
        b(KNEE_R, 0.0, -0.44, 0.0),                 // FOOT_R
    ];
    debug_assert_eq!(bones.len(), COUNT);
    Skeleton { bones }
}

/// What Konrad is doing this frame; input to the animator.
#[derive(Clone, Copy, Default)]
pub struct Motion {
    pub time: f64,
    /// Advances with distance walked or climbed; drives limb cycles.
    pub stride: f64,
    /// 0 when standing still, 1 at full running speed.
    pub run: f64,
    /// World-space velocity.
    pub vel: DVec3,
    pub heading: DQuat,
    pub airborne: bool,
    pub climbing: bool,
    pub swinging: bool,
    /// The grappling line is out (the right arm points along it).
    pub grappling: bool,
    /// Where the grappling line goes, relative to the shoulder, in world space.
    pub aim: DVec3,
}

/// Blends between animation clips.
#[derive(Default)]
pub struct Animator {
    run: f64,
    air: f64,
    climb: f64,
    swing: f64,
    aim: f64,
}

impl Animator {
    pub fn update(&mut self, dt: f64, m: &Motion) {
        if dt <= 0.0 {
            return;
        }
        let (air, climb, swing) = (
            (m.airborne && !m.climbing && !m.swinging) as u8 as f64,
            m.climbing as u8 as f64,
            m.swinging as u8 as f64,
        );
        let run = if air + climb + swing > 0.0 { 0.0 } else { m.run };
        let k = 1.0 - (-dt * 10.0).exp();
        self.run += (run - self.run) * k;
        self.air += (air - self.air) * k;
        self.climb += (climb - self.climb) * k;
        self.swing += (swing - self.swing) * k;
        let aim = (m.grappling && !m.climbing) as u8 as f64;
        self.aim += (aim - self.aim) * (1.0 - (-dt * 20.0).exp());
    }

    pub fn pose(&self, m: &Motion) -> Pose {
        let mut pose = idle(m);
        pose.blend(&run(m), self.run);
        pose.blend(&air(m), self.air);
        pose.blend(&climb(m), self.climb);
        pose.blend(&swing(m), self.swing);
        if self.aim > 0.0 {
            // Point the right arm along the grappling line.
            let local = (m.heading.inverse() * m.aim).normalize_or(DVec3::Y);
            let mut aimed = pose.clone();
            aimed.rot[SHOULDER_R] = DQuat::from_rotation_arc(DVec3::NEG_Y, local);
            aimed.rot[ELBOW_R] = rx(-0.1);
            pose.blend(&aimed, self.aim);
        }
        pose
    }
}

fn rx(a: f64) -> DQuat {
    DQuat::from_rotation_x(a)
}
fn ry(a: f64) -> DQuat {
    DQuat::from_rotation_y(a)
}
fn rz(a: f64) -> DQuat {
    DQuat::from_rotation_z(a)
}

// Animation clips. Limbs hang along −y: a negative rotation about x swings
// them forward, a positive rotation about z swings them out to +x.

/// Breathing: 0..1..0, one breath about every four seconds.
fn breath(time: f64) -> f64 {
    0.5 - 0.5 * (time * 1.5).cos()
}

/// Standing: the whole body is alive, not just the arms. He breathes (chest
/// rises and fills out), sways slowly from the ankles, rests on one leg with
/// the other knee relaxed and changes legs now and then, glances up and
/// down, and his arms hang loosely and follow the body a beat late.
fn idle(m: &Motion) -> Pose {
    let mut p = Pose::rest(COUNT);
    let t = m.time;
    let b = breath(t);
    let sway = (t * 0.5).sin();
    let lagged_sway = (t * 0.5 - 0.6).sin();
    // Weight on the left leg when > 0, eased so it settles on each side.
    let weight = {
        let w = (t * 0.45).sin();
        w.signum() * w.abs().powf(0.4)
    };
    // Every so often he glances down at the ground for a moment.
    let glance = {
        let c = (t * 0.13) % 1.0;
        if c < 0.12 { (c / 0.12 * PI).sin() } else { 0.0 }
    };

    // Lean from the ankles: the pelvis tips, the hips counter so the feet stay.
    p.rotate(PELVIS, rx(0.05 * sway) * rz(0.03 * weight));
    p.rotate(SPINE, rx(-0.03 * b - 0.02 * sway));
    p.rotate(CHEST, rx(-0.05 * b));
    p.rotate(NECK, rx(0.04 * b + 0.1 * glance));
    p.rotate(HEAD, rx(0.08 * (t * 0.31).sin() + 0.35 * glance));
    for (hip, knee, side) in [(HIP_L, KNEE_L, -1.0), (HIP_R, KNEE_R, 1.0)] {
        // The leg without the weight relaxes: knee forward, hip a bit bent.
        let relaxed = ((-weight * side) as f64).max(0.0);
        p.rotate(hip, rx(-0.05 * sway - 0.22 * relaxed) * rz(side * 0.04));
        p.rotate(knee, rx(0.05 + 0.45 * relaxed));
    }
    for (shoulder, elbow, side) in [(SHOULDER_L, ELBOW_L, -1.0), (SHOULDER_R, ELBOW_R, 1.0)] {
        // Arms hang loose; they trail the sway and open a touch on each breath.
        p.rotate(shoulder, rz(side * (0.09 + 0.03 * b)) * rx(0.06 * lagged_sway + 0.04));
        p.rotate(elbow, rx(-0.22 - 0.06 * b));
    }
    p
}

/// A natural run: lean, full leg swing with the trailing knee folding up,
/// arms swinging opposite with bent elbows, shoulders counter-rotating.
fn run(m: &Motion) -> Pose {
    let mut p = Pose::rest(COUNT);
    let s = m.stride;
    let bounce = (s * 2.0).cos();
    p.rotate(PELVIS, rx(0.12) * ry(s.sin() * 0.12));
    p.rotate(SPINE, rx(0.06));
    p.rotate(CHEST, ry(-s.sin() * 0.22) * rx(0.03 * bounce));
    p.rotate(HEAD, ry(s.sin() * 0.08) * rx(-0.1));
    for (hip, knee, shoulder, elbow, phase) in [
        (HIP_L, KNEE_L, SHOULDER_L, ELBOW_L, 0.0),
        (HIP_R, KNEE_R, SHOULDER_R, ELBOW_R, PI),
    ] {
        let swing = (s + phase).sin();
        // The knee folds most while the leg swings back and through.
        let fold = (s + phase + 1.2).cos().max(0.0);
        p.rotate(hip, rx(-swing * 0.75 - 0.15));
        p.rotate(knee, rx(0.2 + 1.5 * fold));
        p.rotate(shoulder, rx(swing * 0.7));
        p.rotate(elbow, rx(-0.9 - 0.3 * swing.max(0.0)));
    }
    p
}

/// Jumping: one knee drawn up, the other leg trailing, arms forward.
fn air(m: &Motion) -> Pose {
    let mut p = Pose::rest(COUNT);
    let fall = (-m.vel.y / 12.0).clamp(0.0, 1.0);
    p.rotate(PELVIS, rx(0.1));
    p.rotate(SPINE, rx(0.08 - 0.1 * fall));
    p.rotate(HIP_L, rx(-1.1 + 0.6 * fall));
    p.rotate(KNEE_L, rx(1.4 - 0.9 * fall));
    p.rotate(HIP_R, rx(0.35 - 0.2 * fall));
    p.rotate(KNEE_R, rx(0.9 - 0.5 * fall));
    for (shoulder, elbow, side) in [(SHOULDER_L, ELBOW_L, -1.0), (SHOULDER_R, ELBOW_R, 1.0)] {
        p.rotate(shoulder, rx(-0.9 + 0.5 * fall) * rz(side * (0.3 + 0.5 * fall)));
        p.rotate(elbow, rx(-0.5));
    }
    p
}

/// Hand over hand, feet finding holds.
fn climb(m: &Motion) -> Pose {
    let mut p = Pose::rest(COUNT);
    let s = m.stride;
    p.rotate(HEAD, rx(-0.25));
    for (shoulder, elbow, hip, knee, phase, side) in [
        (SHOULDER_L, ELBOW_L, HIP_L, KNEE_L, 0.0, -1.0),
        (SHOULDER_R, ELBOW_R, HIP_R, KNEE_R, PI, 1.0),
    ] {
        let reach = (s + phase).sin();
        p.rotate(shoulder, rx(-2.7 - reach * 0.35) * rz(side * 0.25));
        p.rotate(elbow, rx(-0.4 + reach * 0.5));
        p.rotate(hip, rx(-0.7 + reach * 0.45) * rz(side * 0.2));
        p.rotate(knee, rx(1.0 - reach * 0.5));
    }
    p
}

/// Hanging from the line: legs trailing with the swing.
fn swing(m: &Motion) -> Pose {
    let mut p = Pose::rest(COUNT);
    let sway = (m.vel.x * 0.04).clamp(-0.6, 0.6);
    p.rotate(SPINE, rx(-sway * 0.3));
    p.rotate(SHOULDER_L, rz(-0.5) * rx(-0.4));
    p.rotate(ELBOW_L, rx(-0.6));
    for (hip, knee, side) in [(HIP_L, KNEE_L, -1.0), (HIP_R, KNEE_R, 1.0)] {
        p.rotate(hip, rx(sway - 0.25 + (m.time * 3.0 + side).sin() * 0.1));
        p.rotate(knee, rx(0.4));
    }
    p
}

/// Solves the pose and, when Konrad is standing, moves him down or up so the
/// soles rest on the ground (bent knees would otherwise lift the feet).
pub fn plant(skeleton: &Skeleton, pose: &Pose, root: Root, grounded: bool) -> Solved {
    let solved = Solved::solve(skeleton, pose, root);
    if !grounded {
        return solved;
    }
    let lowest = solved.pos[FOOT_L].y.min(solved.pos[FOOT_R].y) - SOLE * root.scale.y;
    let mut root = root;
    root.pos.y -= lowest - root.pos.y;
    Solved::solve(skeleton, pose, root)
}

/// Appearance that isn't part of the skeleton.
#[derive(Clone, Copy, Default)]
pub struct Look {
    pub time: f64,
}

/// Points on Konrad that the game needs.
pub struct Anchors {
    /// Where the grappling line leaves his hand.
    pub hand: DVec3,
}

/// Colour for the limbs on the far side: a touch darker and cooler.
fn far(c: Color) -> Color {
    crate::canvas3d::mix(c, Color::from_rgb8(0x14, 0x18, 0x28), 0.28)
}

/// A smooth closed shape through `points` (Catmull-Rom spline).
fn smooth_closed(points: &[Point]) -> BezPath {
    let n = points.len();
    let mut path = BezPath::new();
    path.move_to(points[0]);
    for i in 0..n {
        let (p0, p1, p2, p3) = (points[(i + n - 1) % n], points[i], points[(i + 1) % n], points[(i + 2) % n]);
        let c1 = p1 + (p2 - p0) / 6.0;
        let c2 = p2 - (p3 - p1) / 6.0;
        path.curve_to(c1, c2, p2);
    }
    path.close_path();
    path
}

/// A body part as one smooth shape: a spline through the joint `points`
/// (screen space) with the given `widths` (pixels) at each point, and
/// rounded ends.
fn limb(points: &[Point], widths: &[f64]) -> BezPath {
    let n = points.len();
    let point = |i: isize| points[i.clamp(0, n as isize - 1) as usize];
    // Sample the centreline spline.
    let mut centre: Vec<(Point, f64)> = Vec::new();
    let steps = 6;
    for i in 0..n - 1 {
        let (p0, p1, p2, p3) = (point(i as isize - 1), point(i as isize), point(i as isize + 1), point(i as isize + 2));
        for k in 0..steps {
            let t = k as f64 / steps as f64;
            let (t2, t3) = (t * t, t * t * t);
            let p = Point::new(
                0.5 * (2.0 * p1.x + (-p0.x + p2.x) * t + (2.0 * p0.x - 5.0 * p1.x + 4.0 * p2.x - p3.x) * t2 + (-p0.x + 3.0 * p1.x - 3.0 * p2.x + p3.x) * t3),
                0.5 * (2.0 * p1.y + (-p0.y + p2.y) * t + (2.0 * p0.y - 5.0 * p1.y + 4.0 * p2.y - p3.y) * t2 + (-p0.y + 3.0 * p1.y - 3.0 * p2.y + p3.y) * t3),
            );
            centre.push((p, widths[i] + (widths[i + 1] - widths[i]) * t));
        }
    }
    centre.push((points[n - 1], widths[n - 1]));
    // Offset both sides along the normals.
    let m = centre.len();
    let normal = |j: usize| {
        let a = centre[j.saturating_sub(1)].0;
        let b = centre[(j + 1).min(m - 1)].0;
        let t = b - a;
        let len = t.hypot().max(1e-9);
        Vec2::new(-t.y / len, t.x / len)
    };
    let mut outline: Vec<Point> = Vec::with_capacity(m * 2 + 12);
    for j in 0..m {
        outline.push(centre[j].0 + normal(j) * centre[j].1 * 0.5);
    }
    // Rounded end cap.
    let (end, w_end, n_end) = (centre[m - 1].0, centre[m - 1].1 * 0.5, normal(m - 1));
    let tangent = Vec2::new(n_end.y, -n_end.x);
    for k in 1..6 {
        let a = std::f64::consts::PI * k as f64 / 6.0;
        outline.push(end + (n_end * a.cos() + tangent * a.sin()) * w_end);
    }
    for j in (0..m).rev() {
        outline.push(centre[j].0 - normal(j) * centre[j].1 * 0.5);
    }
    let (start, w_start, n_start) = (centre[0].0, centre[0].1 * 0.5, normal(0));
    let back = Vec2::new(-n_start.y, n_start.x);
    for k in 1..6 {
        let a = std::f64::consts::PI * k as f64 / 6.0;
        outline.push(start - (n_start * a.cos() - back * a.sin()) * w_start);
    }
    let mut path = BezPath::new();
    path.move_to(outline[0]);
    for p in &outline[1..] {
        path.line_to(*p);
    }
    path.close_path();
    path
}

/// A straight band with flat ends from `a` to `b`, `width` pixels wide.
fn band(a: Point, b: Point, width: f64) -> BezPath {
    let d = b - a;
    let n = if d.hypot() > 1e-9 { Vec2::new(-d.y, d.x).normalize() * width * 0.5 } else { Vec2::new(width * 0.5, 0.0) };
    let mut path = BezPath::new();
    path.move_to(a + n);
    path.line_to(b + n);
    path.line_to(b - n);
    path.line_to(a - n);
    path.close_path();
    path
}

/// A shape drawn in a local 2D frame (x forward, y up, in world units) and
/// placed on screen at `origin` along `forward` and `up`.
fn placed(points: &[(f64, f64)], origin: Point, forward: Vec2, up: Vec2, scale: f64) -> Vec<Point> {
    points.iter().map(|&(x, y)| origin + (forward * x + up * y) * scale).collect()
}

/// Head in profile, facing +x, in world units around the head centre.
const HEAD_SHAPE: [(f64, f64); 17] = [
    (-0.07, -0.06),
    (-0.105, 0.02),
    (-0.09, 0.1),
    (-0.02, 0.135),
    (0.05, 0.12),
    (0.085, 0.07),
    (0.09, 0.035),
    (0.084, 0.018),
    (0.106, -0.012),
    (0.09, -0.03),
    (0.096, -0.044),
    (0.09, -0.058),
    (0.088, -0.078),
    (0.062, -0.1),
    (0.015, -0.088),
    (-0.01, -0.07),
    (-0.04, -0.075),
];
/// Hair: over the top and back, with a fringe falling forward.
const HAIR_SHAPE: [(f64, f64); 11] = [
    (-0.078, -0.03),
    (-0.113, 0.03),
    (-0.098, 0.112),
    (-0.02, 0.152),
    (0.06, 0.135),
    (0.098, 0.088),
    (0.072, 0.072),
    (0.035, 0.095),
    (-0.02, 0.09),
    (-0.05, 0.05),
    (-0.045, 0.0),
];
/// Hips in jeans, around the pelvis: x across, y up.
const HIPS_SHAPE: [(f64, f64); 7] = [
    (-0.105, 0.1),
    (0.105, 0.1),
    (0.115, 0.01),
    (0.1, -0.055),
    (0.0, -0.075),
    (-0.1, -0.055),
    (-0.115, 0.01),
];
/// Boot, around the ankle: x forward, y up.
const BOOT_SHAPE: [(f64, f64); 9] = [
    (-0.055, -0.035),
    (0.1, -0.035),
    (0.14, -0.02),
    (0.13, 0.015),
    (0.06, 0.035),
    (0.035, 0.1),
    (-0.045, 0.1),
    (-0.06, 0.03),
    (-0.065, -0.01),
];
/// Hand, from the wrist along the forearm: x along the hand, y across.
const HAND_SHAPE: [(f64, f64); 7] = [
    (0.0, -0.028),
    (0.05, -0.034),
    (0.095, -0.02),
    (0.105, 0.005),
    (0.07, 0.026),
    (0.03, 0.038),
    (0.0, 0.026),
];

pub fn draw(canvas: &mut Canvas3d, s: &Solved, look: &Look) -> Anchors {
    let cam = canvas.camera;
    let pt = |p: DVec3| cam.point(p);
    let k = (s.root.scale.x + s.root.scale.y + s.root.scale.z) / 3.0;
    let px = cam.project(s.pos[PELVIS]).scale * k;
    // Which side is towards the camera decides the drawing order (only
    // changes when he turns, so parts never swap while he stands).
    let right_near = (s.root.rot * DVec3::X).z < 0.0;
    let forward_x = cam.project_dir(s.pos[HEAD], s.root.rot * DVec3::Z).x;
    let facing = if forward_x >= 0.0 { 1.0 } else { -1.0 };

    let leg = |hip: usize, knee: usize, foot: usize, color: Color| -> Vec<(BezPath, Color)> {
        let (h, kn, a) = (s.pos[hip], s.pos[knee], s.pos[foot]);
        let thigh = h.lerp(kn, 0.45);
        let calf = kn.lerp(a, 0.35) + (s.rot[knee] * DVec3::NEG_Z) * 0.012;
        let jeans = limb(
            &[pt(h), pt(thigh), pt(kn), pt(calf), pt(a)],
            &[0.15, 0.135, 0.1, 0.105, 0.08].map(|w| w * px),
        );
        let fwd = cam.project_dir(a, s.rot[foot] * DVec3::Z);
        let fwd = if fwd.hypot() > 1e-6 { fwd.normalize() } else { Vec2::new(facing, 0.0) };
        let up = if Vec2::new(-fwd.y, fwd.x).y < 0.0 { Vec2::new(-fwd.y, fwd.x) } else { Vec2::new(fwd.y, -fwd.x) };
        let boot = smooth_closed(&placed(&BOOT_SHAPE, pt(a), fwd, up, px));
        vec![(jeans, color), (boot, if color == JEANS { BOOT } else { far(BOOT) })]
    };
    let arm = |shoulder: usize, elbow: usize, hand: usize, sleeve: Color, skin: Color| -> Vec<(BezPath, Color)> {
        let (sh, el, wr) = (s.pos[shoulder], s.pos[elbow], s.at(hand, DVec3::new(0.0, 0.02, 0.0)));
        let sleeve_path = limb(
            &[pt(sh), pt(sh.lerp(el, 0.5)), pt(el), pt(el.lerp(wr, 0.5)), pt(wr)],
            &[0.11, 0.1, 0.085, 0.085, 0.07].map(|w| w * px),
        );
        let dir = pt(s.at(hand, DVec3::new(0.0, -0.1, 0.0))) - pt(wr);
        let along = if dir.hypot() > 1e-6 { dir.normalize() } else { Vec2::new(0.0, 1.0) };
        let across = Vec2::new(-along.y, along.x) * facing;
        let hand_path = smooth_closed(&placed(&HAND_SHAPE, pt(wr), along, across, px));
        vec![(sleeve_path, sleeve), (hand_path, skin)]
    };

    let (near_leg, far_leg) = if right_near { ((HIP_R, KNEE_R, FOOT_R), (HIP_L, KNEE_L, FOOT_L)) } else { ((HIP_L, KNEE_L, FOOT_L), (HIP_R, KNEE_R, FOOT_R)) };
    let (near_arm, far_arm) = if right_near {
        ((SHOULDER_R, ELBOW_R, HAND_R), (SHOULDER_L, ELBOW_L, HAND_L))
    } else {
        ((SHOULDER_L, ELBOW_L, HAND_L), (SHOULDER_R, ELBOW_R, HAND_R))
    };

    let mut shapes: Vec<(BezPath, Color)> = Vec::new();
    shapes.extend(arm(far_arm.0, far_arm.1, far_arm.2, far(SHIRT), far(SKIN)));
    shapes.extend(leg(far_leg.0, far_leg.1, far_leg.2, far(JEANS)));

    // Torso: one shape from the hips up through the chest to the shoulders.
    let pelvis = s.pos[PELVIS];
    // It starts above the belt so its rounded end stays hidden.
    let torso = limb(
        &[
            pt(s.at(PELVIS, DVec3::new(0.0, 0.1, 0.0))),
            pt(s.pos[SPINE]),
            pt(s.pos[CHEST]),
            pt(s.at(CHEST, DVec3::new(0.0, 0.06, 0.0))),
        ],
        // Seen from the side: chest deeper than the waist, narrowing to the
        // shoulders so the top ends below the chin.
        &[0.22, 0.2, 0.25 * (1.0 + 0.05 * breath(look.time)), 0.2].map(|w| w * px),
    );
    shapes.push((torso, SHIRT));
    // Jeans round the hips, with the belt.
    // Jeans round the hips: a drawn pelvis shape (waist, hips, crotch) in
    // the pelvis's own frame, so it tilts with him.
    let hip_up = cam.project_dir(pelvis, s.rot[PELVIS] * DVec3::Y);
    let hip_up = if hip_up.hypot() > 1e-6 { hip_up.normalize() } else { Vec2::new(0.0, -1.0) };
    let hip_side = Vec2::new(-hip_up.y, hip_up.x);
    shapes.push((smooth_closed(&placed(&HIPS_SHAPE, pt(pelvis), hip_side, hip_up, px)), JEANS));
    let belt = band(pt(s.at(PELVIS, DVec3::new(0.0, 0.07, 0.0))), pt(s.at(PELVIS, DVec3::new(0.0, 0.1, 0.0))), 0.225 * px);
    shapes.push((belt, BELT));

    shapes.extend(leg(near_leg.0, near_leg.1, near_leg.2, JEANS));
    if right_near {
        // Holster on the right hip.
        let top = s.at(PELVIS, DVec3::new(0.13, 0.06, 0.02));
        let bottom = s.at(PELVIS, DVec3::new(0.135, -0.1, 0.03));
        shapes.push((limb(&[pt(top), pt(bottom)], &[0.075 * px, 0.06 * px]), BELT));
    }
    let near = arm(near_arm.0, near_arm.1, near_arm.2, SHIRT, SKIN);
    // A faint crease line so the near sleeve reads against the shirt.
    let sleeve_line = near[0].0.clone();
    shapes.extend(near);

    // The near sleeve is the second-to-last shape so far (then its hand).
    let sleeve_index = shapes.len() - 1;

    // Neck, then the head in profile with hair, ear and face.
    let neck = limb(
        &[pt(s.at(CHEST, DVec3::new(0.0, 0.12, 0.0))), pt(s.at(HEAD, DVec3::new(-0.01, -0.07, -0.01)))],
        &[0.1 * px, 0.09 * px],
    );
    shapes.push((neck, SKIN));
    let head_c = pt(s.pos[HEAD]);
    let up_screen = cam.project_dir(s.pos[HEAD], s.rot[HEAD] * DVec3::Y);
    let up = if up_screen.hypot() > 1e-6 { up_screen.normalize() } else { Vec2::new(0.0, -1.0) };
    let fwd = Vec2::new(-up.y, up.x) * facing;
    let head_px = cam.project(s.pos[HEAD]).scale * k;
    shapes.push((smooth_closed(&placed(&HEAD_SHAPE, head_c, fwd, up, head_px)), SKIN));
    shapes.push((smooth_closed(&placed(&HAIR_SHAPE, head_c, fwd, up, head_px)), HAIR));
    let ear = Ellipse::new(head_c + (fwd * -0.02 + up * 0.005) * head_px, (0.018 * head_px, 0.028 * head_px), up.atan2() + std::f64::consts::FRAC_PI_2);
    shapes.push((ear.to_path(0.1), crate::canvas3d::darken(SKIN, 0.12)));

    // Face details as thin strokes and a small eye.
    let blink = (look.time * 0.23) % 1.0 < 0.03;
    let at = |x: f64, y: f64| head_c + (fwd * x + up * y) * head_px;
    let mut details = BezPath::new();
    details.move_to(at(0.045, 0.05));
    details.line_to(at(0.08, 0.046));
    details.move_to(at(0.074, -0.052));
    details.line_to(at(0.089, -0.051));
    if blink {
        details.move_to(at(0.052, 0.03));
        details.line_to(at(0.07, 0.03));
    }
    let eye = (!blink).then(|| Ellipse::new(at(0.062, 0.03), (0.01 * head_px, 0.007 * head_px), up.atan2() + std::f64::consts::FRAC_PI_2));
    let line_w = (0.008 * head_px).max(1.0);

    // Everything in one fixed order, as a single item in the world.
    let depth = canvas.depth_of(pelvis);
    canvas.push(depth, move |scene| {
        for (i, (path, color)) in shapes.iter().enumerate() {
            scene.fill(Fill::NonZero, Affine::IDENTITY, *color, None, path);
            if i + 1 == sleeve_index {
                scene.stroke(&Stroke::new(line_w * 0.8), Affine::IDENTITY, crate::canvas3d::darken(SHIRT, 0.3), None, &sleeve_line);
            }
        }
        if let Some(eye) = &eye {
            scene.fill(Fill::NonZero, Affine::IDENTITY, FEATURE, None, eye);
        }
        scene.stroke(&Stroke::new(line_w).with_caps(vello::kurbo::Cap::Round), Affine::IDENTITY, FEATURE, None, &details);
    });

    Anchors {
        hand: s.at(HAND_R, DVec3::new(0.0, -0.09, 0.0)),
    }
}

/// Model-to-world rotation for a heading: `facing` 1 = right, −1 = left, with
/// values in between turning through the camera (a 3/4 view at the ends).
pub fn heading(facing: f64, away: f64) -> DQuat {
    let side = DQuat::from_rotation_y(PI - facing * 1.2);
    side.slerp(DQuat::from_rotation_y(0.0), away)
}

/// Konrad's root: `squash` > 0 stretches up (jumping), < 0 squashes (landing);
/// `stretch` pulls him out along his heading when running fast.
pub fn root(pos: DVec3, rot: DQuat, squash: f64, stretch: f64) -> Root {
    let thin = 1.0 / (1.0 + stretch).sqrt();
    Root {
        pos,
        rot,
        scale: DVec3::new(
            (1.0 - squash * 0.5) * thin,
            (1.0 + squash) * thin,
            (1.0 - squash * 0.5) * (1.0 + stretch),
        ),
    }
}

