//! Camel Joe: a camel standing on two legs, with exaggerated chameleon traits.
//! Camel: hump, long S-curved neck, long snout with droopy split lips, sleepy
//! lidded eyes with long lashes, small ears, knobbly knees, two-toed feet and a
//! tufted tail. Chameleon: turret eyes that look in different directions, a
//! sticky tongue, a curling prehensile tail, a spiky crest and camouflage.
//!
//! Joe is a 3D rig (see `rig`) drawn with vector primitives (see `canvas3d`).
//! Model space: x = Joe's right, y = up, z = forward. Feet at the origin.

use std::f64::consts::PI;

use glam::{DMat3, DQuat, DVec2, DVec3};
use vello::kurbo::{Affine, BezPath, Ellipse, Point, RoundedRect, Shape, Stroke};
use vello::peniko::{Color, Fill, Gradient};

use crate::canvas3d::{darken, mix, outline, Canvas3d, OUTLINE};
use crate::rig::{Bone, Pose, Root, Skeleton, Solved};

/// Bone indices.
pub mod bone {
    pub const ROOT: usize = 0;
    pub const PELVIS: usize = 1;
    pub const CHEST: usize = 2;
    /// The neck is two bones so it can make a camel's S-curve.
    pub const NECK: usize = 3;
    pub const NECK2: usize = 4;
    pub const HEAD: usize = 5;
    pub const HAT: [usize; 4] = [6, 7, 8, 9];
    pub const EYE_L: usize = 10;
    pub const EYE_R: usize = 11;
    pub const SHOULDER_L: usize = 12;
    pub const ELBOW_L: usize = 13;
    pub const HAND_L: usize = 14;
    pub const SHOULDER_R: usize = 15;
    pub const ELBOW_R: usize = 16;
    pub const HAND_R: usize = 17;
    pub const HIP_L: usize = 18;
    pub const KNEE_L: usize = 19;
    pub const FOOT_L: usize = 20;
    pub const HIP_R: usize = 21;
    pub const KNEE_R: usize = 22;
    pub const FOOT_R: usize = 23;
    pub const TAIL: [usize; 8] = [24, 25, 26, 27, 28, 29, 30, 31];
    pub const FLAP_L: usize = 32;
    pub const FLAP_R: usize = 33;
    pub const COUNT: usize = 34;
}
use bone::*;

const FUR: Color = Color::from_rgb8(0xd9, 0xab, 0x6e);
const FUR_SHADE: Color = Color::from_rgb8(0xbc, 0x8a, 0x50);
/// Shaggy hair on the hump, throat and tail tuft.
const HAIR: Color = Color::from_rgb8(0x9c, 0x6a, 0x3c);
/// Knee and elbow calluses, foot pads.
const PAD: Color = Color::from_rgb8(0x7e, 0x5e, 0x44);
const CAMO: Color = Color::from_rgb8(0x4f, 0x8f, 0x3a);
const CAMO_SHADE: Color = Color::from_rgb8(0x35, 0x6b, 0x2a);
const MUZZLE: Color = Color::from_rgb8(0xf0, 0xcf, 0xa0);
const SCARF: Color = Color::from_rgb8(0x5b, 0x3f, 0x8c);
const STRAW: Color = Color::from_rgb8(0xd8, 0xab, 0x3c);
const POMPOM: Color = Color::from_rgb8(0x6b, 0x5b, 0x8a);
const CREST: Color = Color::from_rgb8(0xe8, 0x5d, 0x3f);
const EYEBALL: Color = Color::from_rgb8(0xfb, 0xf4, 0xe4);

/// The hat's colour bands, from the brim up.
const HAT_BANDS: [Color; 6] = [
    Color::from_rgb8(0xe8, 0x5d, 0x3f),
    Color::from_rgb8(0xf2, 0xc1, 0x4e),
    Color::from_rgb8(0x3c, 0x8d, 0x5a),
    Color::from_rgb8(0xe9, 0x8a, 0xa8),
    Color::from_rgb8(0x4a, 0x6f, 0xc9),
    Color::from_rgb8(0xf2, 0xc1, 0x4e),
];

pub fn skeleton() -> Skeleton {
    let b = |parent: usize, x: f64, y: f64, z: f64| Bone {
        parent: Some(parent),
        offset: DVec3::new(x, y, z),
    };
    let mut bones = vec![
        Bone { parent: None, offset: DVec3::ZERO }, // ROOT
        b(ROOT, 0.0, 0.62, 0.0),                    // PELVIS
        b(PELVIS, 0.0, 0.26, 0.0),                  // CHEST
        b(CHEST, 0.0, 0.16, 0.1),                   // NECK
        b(NECK, 0.0, 0.25, 0.17),                   // NECK2
        b(NECK2, 0.0, 0.25, 0.0),                   // HEAD
        b(HEAD, 0.0, 0.13, -0.03),                  // HAT[0]: brim
        b(HAT[0], 0.0, 0.18, 0.0),                  // HAT[1]
        b(HAT[1], 0.0, 0.16, 0.0),                  // HAT[2]
        b(HAT[2], 0.0, 0.12, 0.0),                  // HAT[3]: tip
        b(HEAD, -0.24, 0.07, 0.05),                 // EYE_L
        b(HEAD, 0.24, 0.07, 0.05),                  // EYE_R
        b(CHEST, -0.2, 0.1, 0.06),                  // SHOULDER_L
        b(SHOULDER_L, 0.0, -0.27, 0.0),             // ELBOW_L
        b(ELBOW_L, 0.0, -0.25, 0.0),                // HAND_L
        b(CHEST, 0.2, 0.1, 0.06),                   // SHOULDER_R
        b(SHOULDER_R, 0.0, -0.27, 0.0),             // ELBOW_R
        b(ELBOW_R, 0.0, -0.25, 0.0),                // HAND_R
        b(PELVIS, -0.12, -0.02, 0.0),               // HIP_L
        b(HIP_L, 0.0, -0.3, 0.0),                   // KNEE_L
        b(KNEE_L, 0.0, -0.3, 0.0),                  // FOOT_L
        b(PELVIS, 0.12, -0.02, 0.0),                // HIP_R
        b(HIP_R, 0.0, -0.3, 0.0),                   // KNEE_R
        b(KNEE_R, 0.0, -0.3, 0.0),                  // FOOT_R
    ];
    // The tail grows backwards (−z) from the pelvis.
    bones.push(b(PELVIS, 0.0, 0.0, -0.17));
    for i in 1..TAIL.len() {
        bones.push(b(TAIL[i - 1], 0.0, 0.0, -0.1));
    }
    bones.push(b(HEAD, -0.16, 0.05, -0.03)); // FLAP_L
    bones.push(b(HEAD, 0.16, 0.05, -0.03)); // FLAP_R
    debug_assert_eq!(bones.len(), COUNT);
    Skeleton { bones }
}

/// What Joe is doing this frame; input to the animator.
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
    /// Gaze directions of the left and right eye, in world space.
    pub gaze: [DVec3; 2],
}

/// A damped 2D spring used for secondary motion (hat, tail, ear flaps).
#[derive(Default)]
struct Spring {
    pos: DVec2,
    vel: DVec2,
}

impl Spring {
    fn update(&mut self, target: DVec2, dt: f64, stiffness: f64, damping: f64) {
        let accel = (target - self.pos) * stiffness - self.vel * damping;
        self.vel += accel * dt;
        self.pos += self.vel * dt;
    }
}

/// Blends between animation clips and runs secondary motion.
#[derive(Default)]
pub struct Animator {
    run: f64,
    air: f64,
    climb: f64,
    swing: f64,
    hat: Spring,
    tail: Spring,
    flaps: Spring,
    prev_vel: DVec3,
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
        let k = 1.0 - (-dt * 12.0).exp();
        self.run += (run - self.run) * k;
        self.air += (air - self.air) * k;
        self.climb += (climb - self.climb) * k;
        self.swing += (swing - self.swing) * k;

        // Secondary motion reacts to acceleration in model space: the hat
        // lags behind when Joe speeds up and flops forward when Joe stops.
        let accel = (m.vel - self.prev_vel) / dt;
        self.prev_vel = m.vel;
        let a = m.heading.inverse() * accel.clamp_length_max(60.0);
        let push = DVec2::new(a.z + a.y * 0.4, -a.x);
        self.hat.update(push * 0.02, dt, 90.0, 7.0);
        self.flaps.update(push * 0.015, dt, 140.0, 8.0);
        self.tail.update(push * 0.01, dt, 60.0, 5.0);
    }

    pub fn pose(&self, m: &Motion) -> Pose {
        let mut pose = idle(m);
        pose.blend(&run(m), self.run);
        pose.blend(&air(m), self.air);
        pose.blend(&climb(m), self.climb);
        pose.blend(&swing(m), self.swing);

        for (i, &b) in HAT.iter().enumerate().skip(1) {
            let w = i as f64 * 0.6;
            pose.rotate(b, rx(self.hat.pos.x * w) * rz(self.hat.pos.y * w));
        }
        for (i, &b) in TAIL.iter().enumerate() {
            let w = i as f64 * 0.25;
            pose.rotate(b, rx(self.tail.pos.x * w) * ry(self.tail.pos.y * w));
        }
        for b in [FLAP_L, FLAP_R] {
            pose.rotate(b, rx(self.flaps.pos.x * 2.0) * rz(self.flaps.pos.y * 2.0));
        }

        // Turret eyes: each points independently along its gaze.
        let head_inv = m.heading.inverse();
        for (eye, gaze, side) in [(EYE_L, m.gaze[0], -1.0), (EYE_R, m.gaze[1], 1.0)] {
            let mut dir = (head_inv * gaze).normalize_or(DVec3::Z);
            // Keep each eye on its own side of the head.
            dir.x = if side < 0.0 { dir.x.min(0.2) } else { dir.x.max(-0.2) };
            pose.rot[eye] = DQuat::from_rotation_arc(DVec3::Z, dir.normalize());
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

// Animation clips. Conventions: limbs hang along −y, so a negative rotation
// about x swings them forward; a positive rotation about z swings them to +x.
//
// Joe walks on all fours by default (idle, run, air) and rears up on the hind
// legs to climb and swing; blending between the two makes Joe stand up.

/// Tips the body forward onto all fours: the spine becomes horizontal and the
/// legs, neck and tail are turned back to hang/point the right way.
fn on_all_fours() -> Pose {
    let mut p = Pose::rest(COUNT);
    let upright = PI / 2.0;
    p.rotate(PELVIS, rx(upright));
    for b in [SHOULDER_L, SHOULDER_R, HIP_L, HIP_R] {
        p.rotate(b, rx(-upright));
    }
    p.rotate(NECK, rx(-upright + 0.12));
    p.rotate(TAIL[0], rx(-upright + 0.2));
    p
}

fn idle(m: &Motion) -> Pose {
    let mut p = on_all_fours();
    let breath = (m.time * 2.2).sin();
    p.rotate(CHEST, rx(0.02 * breath));
    p.rotate(NECK2, rx(-0.05 - 0.04 * breath));
    // Looking around lazily.
    p.rotate(HEAD, rx(0.1 - 0.04 * breath) * ry(0.25 * (m.time * 0.4).sin()) * rz(0.08 * (m.time * 0.7).sin()));
    p.rotate(TAIL[0], ry(-0.4));
    for (i, &b) in TAIL.iter().enumerate() {
        p.rotate(b, rx(0.42 + 0.06 * (m.time * 1.5 + i as f64 * 0.5).sin()));
    }
    p
}

fn run(m: &Motion) -> Pose {
    let mut p = on_all_fours();
    let s = m.stride;
    // Camels pace: both legs on one side move together, which makes the body
    // rock from side to side.
    p.rotate(PELVIS, ry(s.sin() * 0.1));
    let bob = (s * 2.0).sin();
    p.rotate(NECK, rx(0.25 + bob * 0.08));
    p.rotate(NECK2, rx(-0.2 - bob * 0.1));
    p.rotate(HEAD, rx(0.12));
    for (hip, knee, shoulder, elbow, phase) in [
        (HIP_L, KNEE_L, SHOULDER_L, ELBOW_L, 0.0),
        (HIP_R, KNEE_R, SHOULDER_R, ELBOW_R, PI),
    ] {
        let swing = (s + phase).sin();
        // The foot lifts while the leg swings forward.
        let lift = (s + phase).cos().max(0.0);
        p.rotate(hip, rx(-swing * 0.55));
        p.rotate(knee, rx(0.1 + lift * 1.0));
        p.rotate(shoulder, rx(-swing * 0.55));
        p.rotate(elbow, rx(0.1 + lift * 1.1));
    }
    for (i, &b) in TAIL.iter().enumerate() {
        p.rotate(b, rx(0.2) * ry((s + i as f64 * 0.6).sin() * 0.12));
    }
    p
}

fn air(m: &Motion) -> Pose {
    let mut p = on_all_fours();
    // A leap: stretched out while rising, legs reaching down while falling.
    let fall = (-m.vel.y / 10.0).clamp(0.0, 1.0);
    let stretch = 1.0 - fall;
    p.rotate(PELVIS, rx(-0.15 * stretch));
    p.rotate(NECK, rx(-0.15 * stretch));
    p.rotate(NECK2, rx(-0.25));
    p.rotate(HEAD, rx(0.2 * fall));
    for (hip, knee, shoulder, elbow) in [(HIP_L, KNEE_L, SHOULDER_L, ELBOW_L), (HIP_R, KNEE_R, SHOULDER_R, ELBOW_R)] {
        p.rotate(hip, rx(0.8 * stretch - 0.2 * fall));
        p.rotate(knee, rx(0.3 + 0.3 * fall));
        p.rotate(shoulder, rx(-0.9 * stretch + 0.1 * fall));
        p.rotate(elbow, rx(0.9 * stretch + 0.3 * fall));
    }
    for &b in &TAIL {
        p.rotate(b, rx(0.5));
    }
    p
}

fn climb(m: &Motion) -> Pose {
    let mut p = Pose::rest(COUNT);
    let s = m.stride;
    p.rotate(NECK, rx(-0.25));
    p.rotate(HEAD, rx(-0.2));
    for (shoulder, elbow, hip, knee, phase, side) in [
        (SHOULDER_L, ELBOW_L, HIP_L, KNEE_L, 0.0, -1.0),
        (SHOULDER_R, ELBOW_R, HIP_R, KNEE_R, PI, 1.0),
    ] {
        let reach = (s + phase).sin();
        p.rotate(shoulder, rx(-2.5 - reach * 0.45) * rz(side * 0.35));
        p.rotate(elbow, rx(-0.5 + reach * 0.4));
        p.rotate(hip, rx(-0.9 + reach * 0.45) * rz(side * 0.35));
        p.rotate(knee, rx(1.2 - reach * 0.4));
    }
    for &b in &TAIL {
        p.rotate(b, rx(0.62));
    }
    p
}

fn swing(m: &Motion) -> Pose {
    let mut p = Pose::rest(COUNT);
    let sway = (m.vel.x * 0.08).clamp(-0.8, 0.8);
    p.rotate(NECK, rx(-0.2));
    p.rotate(NECK2, rx(-0.2));
    p.rotate(HEAD, rx(0.1));
    for (shoulder, elbow, hip, knee, side) in [
        (SHOULDER_L, ELBOW_L, HIP_L, KNEE_L, -1.0),
        (SHOULDER_R, ELBOW_R, HIP_R, KNEE_R, 1.0),
    ] {
        p.rotate(shoulder, rz(side * 2.0) * rx(-0.4));
        p.rotate(elbow, rx(-0.5 + (m.time * 6.0 + side).sin() * 0.2));
        p.rotate(hip, rx(sway - 0.3));
        p.rotate(knee, rx(0.5));
    }
    for &b in &TAIL {
        p.rotate(b, rx(0.35) * ry(sway * 0.3));
    }
    p
}

/// Appearance that isn't part of the skeleton.
#[derive(Clone, Copy, Default)]
pub struct Look {
    pub time: f64,
    /// 0 = normal fur, 1 = fully blended into the jungle.
    pub camo: f64,
    pub tongue_out: bool,
}

/// Points on Joe that the game needs, e.g. where the tongue starts.
pub struct Anchors {
    pub mouth: DVec3,
}

/// A point on an ellipsoid surface: `u` = angle around y (0 = +z), `v` = latitude.
fn on_ellipsoid(center: DVec3, rot: DQuat, radii: DVec3, u: f64, v: f64) -> DVec3 {
    let dir = DVec3::new(u.sin() * v.cos(), v.sin(), u.cos() * v.cos());
    center + rot * (dir * radii)
}

fn axes(rot: DQuat, radii: DVec3, scale: DVec3) -> DMat3 {
    DMat3::from_quat(rot) * DMat3::from_diagonal(radii * scale)
}

pub fn draw(canvas: &mut Canvas3d, s: &Solved, look: &Look) -> Anchors {
    let fur = mix(FUR, CAMO, look.camo);
    let shade = mix(FUR_SHADE, CAMO_SHADE, look.camo);
    let k = (s.root.scale.x + s.root.scale.y + s.root.scale.z) / 3.0;
    let cam = canvas.camera;

    // Tail: a chain of tapering capsules.
    let mut prev = s.at(PELVIS, DVec3::new(0.0, 0.0, -0.1));
    for (i, &b) in TAIL.iter().enumerate() {
        let r0 = 0.06 * (1.0 - i as f64 / 10.0) * k;
        let r1 = 0.06 * (1.0 - (i + 1) as f64 / 10.0) * k;
        canvas.capsule(prev, r0, s.pos[b], r1, shade);
        prev = s.pos[b];
    }
    let tip_dir = (s.pos[TAIL[7]] - s.pos[TAIL[6]]).normalize_or(DVec3::Y);
    canvas.capsule(prev, 0.04 * k, prev + tip_dir * 0.14 * k, 0.015 * k, HAIR);

    // Legs with pincer feet (two fused toe bundles each).
    for (hip, knee, foot) in [(HIP_L, KNEE_L, FOOT_L), (HIP_R, KNEE_R, FOOT_R)] {
        canvas.capsule(s.pos[hip], 0.085 * k, s.pos[knee], 0.05 * k, shade);
        canvas.capsule(s.pos[knee], 0.05 * k, s.pos[foot], 0.042 * k, shade);
        canvas.sphere(s.at(knee, DVec3::new(0.0, 0.0, 0.035)), 0.058 * k, PAD);
        canvas.ellipsoid(s.at(foot, DVec3::new(0.0, 0.0, 0.04)), axes(s.rot[foot], DVec3::new(0.08, 0.03, 0.1), s.root.scale), PAD);
        for side in [-1.0, 1.0] {
            let toe = s.at(foot, DVec3::new(side * 0.042, 0.03, 0.09));
            canvas.ellipsoid(toe, axes(s.rot[foot], DVec3::new(0.042, 0.038, 0.085), s.root.scale), fur);
        }
    }

    // Torso with belly scales and a back crest.
    let torso_c = s.at(PELVIS, DVec3::new(0.0, 0.2, 0.0));
    let torso_r = DVec3::new(0.23, 0.28, 0.2);
    let torso_rot = s.rot[PELVIS].slerp(s.rot[CHEST], 0.5);
    canvas.ellipsoid(torso_c, axes(torso_rot, torso_r, s.root.scale), fur);
    let belly_n = torso_rot * DVec3::Z;
    if cam.faces(torso_c + belly_n * 0.2, belly_n) {
        let map = |u: f64, v: f64| on_ellipsoid(torso_c, torso_rot, torso_r * s.root.scale * 1.01, u, v);
        let belly = canvas.surface_path(&Ellipse::new((0.0, -0.1), (0.75, 0.85), 0.0).to_path(0.01), map);
        let bands: Vec<BezPath> = (0..5)
            .map(|i| {
                let v = -0.7 + i as f64 * 0.3;
                let mut band = BezPath::new();
                band.move_to((-0.7, v));
                band.quad_to((0.0, v - 0.12), (0.7, v));
                canvas.surface_path(&band, map)
            })
            .collect();
        let belly_color = mix(MUZZLE, fur, look.camo * 0.6);
        let depth = canvas.camera.project(torso_c).depth;
        canvas.push(depth - 1e-4, move |scene| {
            scene.fill(Fill::NonZero, Affine::IDENTITY, belly_color, None, &belly);
            scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &belly);
            for band in &bands {
                scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, darken(belly_color, 0.2), None, band);
            }
            scene.pop_layer();
        });
    }
    // The hump, with shaggy hair along its top.
    let hump_rot = s.rot[CHEST] * rx(-0.35);
    let hump_c = s.at(CHEST, DVec3::new(0.0, 0.1, -0.21));
    let hump_r = DVec3::new(0.2, 0.25, 0.2);
    canvas.ellipsoid(hump_c, axes(hump_rot, hump_r, s.root.scale), fur);
    for i in 0..9 {
        let v = 0.15 + i as f64 * 0.13;
        let u = PI + (i as f64 * 1.7).sin() * 0.35;
        let base = on_ellipsoid(hump_c, hump_rot, hump_r * s.root.scale, u, v);
        let normal = (base - hump_c).normalize();
        let tip = base + (normal + hump_rot * DVec3::new(0.0, -0.3, -0.4)).normalize() * 0.09 * k;
        canvas.capsule(base, 0.035 * k, tip, 0.008 * k, HAIR);
    }

    // Arms with pincer hands.
    for (shoulder, elbow, hand) in [(SHOULDER_L, ELBOW_L, HAND_L), (SHOULDER_R, ELBOW_R, HAND_R)] {
        canvas.capsule(s.pos[shoulder], 0.06 * k, s.pos[elbow], 0.042 * k, fur);
        canvas.capsule(s.pos[elbow], 0.042 * k, s.pos[hand], 0.038 * k, fur);
        // Camels have callused pads on their front knees.
        canvas.sphere(s.at(elbow, DVec3::new(0.0, 0.0, 0.03)), 0.055 * k, PAD);
        canvas.ellipsoid(s.at(hand, DVec3::new(0.0, -0.02, 0.04)), axes(s.rot[hand], DVec3::new(0.075, 0.028, 0.095), s.root.scale), PAD);
        for side in [-1.0, 1.0] {
            let toe = s.at(hand, DVec3::new(side * 0.04, -0.01, 0.085));
            canvas.ellipsoid(toe, axes(s.rot[hand], DVec3::new(0.04, 0.036, 0.08), s.root.scale), fur);
        }
    }

    // Neck and scarf.
    let neck_base = s.at(CHEST, DVec3::new(0.0, 0.12, 0.04));
    let neck_top = s.at(HEAD, DVec3::new(0.0, -0.06, -0.08));
    let neck_pts = [neck_base, s.pos[NECK], s.pos[NECK2], neck_top];
    let neck_r = [0.13, 0.1, 0.085, 0.08].map(|r| r * k);
    for i in 0..3 {
        canvas.capsule(neck_pts[i], neck_r[i], neck_pts[i + 1], neck_r[i + 1], fur);
    }
    for i in 0..6 {
        let f = i as f64 / 5.0;
        let seg = ((f * 2.999) as usize).min(2);
        let t = f * 3.0 - seg as f64;
        let center = neck_pts[seg].lerp(neck_pts[seg + 1], t);
        let r = neck_r[seg] + (neck_r[seg + 1] - neck_r[seg]) * t;
        let bone = [NECK, NECK, NECK2][seg];
        let back = s.dir(bone, DVec3::new(0.0, 0.2, -1.0)).normalize();
        let front = s.dir(bone, DVec3::new(0.0, -0.5, 1.0)).normalize();
        canvas.capsule(center + back * r * 0.8, 0.035 * k, center + back * (r + 0.1 * k), 0.004, CREST);
        if i > 0 {
            canvas.capsule(center + front * r * 0.8, 0.03 * k, center + front * (r + 0.07 * k) - DVec3::Y * 0.04 * k, 0.006, HAIR);
        }
    }
    let scarf_c = s.at(NECK, DVec3::new(0.0, -0.1, -0.04));
    let (front, back) = canvas.loop_halves(
        scarf_c,
        s.dir(NECK, DVec3::X) * 0.14 * k,
        s.dir(NECK, DVec3::Z) * 0.135 * k,
    );
    let scarf_px = 0.08 * cam.project(scarf_c).scale;
    let scarf_depth = cam.project(scarf_c).depth;
    let band = Stroke::new(scarf_px).with_caps(vello::kurbo::Cap::Round);
    let band2 = band.clone();
    canvas.push(scarf_depth + 0.12, move |scene| {
        scene.stroke(&band, Affine::IDENTITY, darken(SCARF, 0.2), None, &back);
    });
    canvas.push(scarf_depth - 0.12, move |scene| {
        scene.stroke(&Stroke::new(scarf_px + 3.0).with_caps(vello::kurbo::Cap::Round), Affine::IDENTITY, OUTLINE, None, &front);
        scene.stroke(&band2, Affine::IDENTITY, SCARF, None, &front);
    });
    // The scarf's loose end flutters behind Joe.
    let mut tail_pts = vec![s.at(NECK, DVec3::new(-0.05, -0.1, -0.16))];
    for i in 1..6 {
        let f = i as f64;
        let flutter = (look.time * 7.0 - f * 0.9).sin() * 0.04 * f;
        tail_pts.push(s.at(NECK, DVec3::new(-0.05 + flutter * 0.5, -0.1 - 0.03 * f + flutter, -0.16 - 0.08 * f)));
    }
    canvas.line(&tail_pts, 0.07 * k, SCARF);

    // Head and snout.
    let head_c = s.pos[HEAD];
    let head_rot = s.rot[HEAD];
    let head_r = DVec3::new(0.155, 0.165, 0.18) * s.root.scale;
    canvas.ellipsoid(head_c, axes(head_rot, head_r, DVec3::ONE), fur);
    let head_depth = cam.project(head_c).depth;
    for side in [-1.0, 1.0] {
        let cheek = on_ellipsoid(head_c, head_rot, head_r, side * 1.0, -0.3);
        canvas.push(head_depth - 1e-4, {
            let e = canvas.project_ellipsoid(cheek, axes(head_rot, DVec3::new(0.05, 0.08, 0.01), s.root.scale));
            let n = head_rot * DVec3::new(side, 0.0, 0.3);
            let visible = cam.faces(cheek, n);
            move |scene| {
                if visible {
                    scene.fill(Fill::NonZero, Affine::IDENTITY, shade.with_alpha(0.6), None, &e);
                }
            }
        });
    }

    let snout_c = s.at(HEAD, DVec3::new(0.0, -0.06, 0.22));
    let snout_r = DVec3::new(0.105, 0.1, 0.2) * s.root.scale;
    let muzzle = mix(MUZZLE, fur, look.camo * 0.6);
    canvas.ellipsoid(snout_c, axes(head_rot, snout_r, DVec3::ONE), fur);
    // Droopy split upper lip and a hanging lower lip.
    for side in [-1.0, 1.0] {
        let lip = s.at(HEAD, DVec3::new(side * 0.045, -0.11, 0.39));
        canvas.ellipsoid(lip, axes(head_rot, DVec3::new(0.055, 0.05, 0.055), s.root.scale), muzzle);
    }
    let lower_lip = s.at(HEAD, DVec3::new(0.0, -0.17, 0.34));
    canvas.ellipsoid(lower_lip, axes(head_rot * rx(0.3), DVec3::new(0.06, 0.035, 0.07), s.root.scale), muzzle);
    // Buck teeth peeking out under the lip.
    for side in [-1.0, 1.0] {
        let top = s.at(HEAD, DVec3::new(side * 0.018, -0.13, 0.425));
        canvas.capsule(top, 0.017 * k, top - head_rot * DVec3::Y * 0.06 * k, 0.017 * k, Color::WHITE);
    }
    let snout_map = |u: f64, v: f64| on_ellipsoid(snout_c, head_rot, snout_r * 1.01, u, v);
    let mouth = s.at(HEAD, DVec3::new(0.0, -0.14, 0.4));
    let snout_depth = cam.project(snout_c).depth;
    let snout_front = head_rot * DVec3::Z;
    if cam.faces(snout_c + snout_front * 0.1, snout_front) {
        let nostrils: Vec<BezPath> = [-1.0, 1.0]
            .iter()
            .map(|side| canvas.surface_path(&Ellipse::new((side * 0.28, 0.3), (0.1, 0.035), side * -0.5).to_path(0.01), snout_map))
            .collect();
        let mut mouth_line = BezPath::new();
        mouth_line.move_to((-1.0, -0.55));
        mouth_line.quad_to((0.0, -0.75), (1.0, -0.55));
        let mouth_line = canvas.surface_path(&mouth_line, snout_map);
        let open = look
            .tongue_out
            .then(|| canvas.surface_path(&Ellipse::new((0.0, -0.55), (0.3, 0.2), 0.0).to_path(0.01), snout_map));
        canvas.push(snout_depth - 1e-4, move |scene| {
            for n in &nostrils {
                scene.fill(Fill::NonZero, Affine::IDENTITY, OUTLINE, None, n);
            }
            if let Some(open) = &open {
                scene.fill(Fill::NonZero, Affine::IDENTITY, Color::from_rgb8(0x5a, 0x12, 0x20), None, open);
            }
            scene.stroke(&Stroke::new(1.5), Affine::IDENTITY, OUTLINE.with_alpha(0.7), None, &mouth_line);
        });
    }
    if !look.tongue_out {
        let corner = s.at(HEAD, DVec3::new(-0.07, -0.13, 0.33));
        let wiggle = (look.time * 4.0).sin() * 0.03;
        let tip = corner + head_rot * DVec3::new(-0.28, -0.04 + wiggle, 0.12);
        canvas.line(&[corner, corner.lerp(tip, 0.5) + DVec3::Y * 0.015, tip], 0.014, STRAW);
    }

    // Small camel ears poking out under the hat, flicking now and then.
    for side in [-1.0, 1.0] {
        let flick = ((look.time * 0.9 + side).sin().max(0.93) - 0.93) * 6.0;
        let base = s.at(HEAD, DVec3::new(side * 0.1, 0.11, -0.1));
        let tip = s.at(HEAD, DVec3::new(side * (0.2 + flick * 0.05), 0.2 + flick * 0.05, -0.2));
        canvas.capsule(base, 0.045 * k, tip, 0.012 * k, fur);
    }

    // Ear flaps hanging from the hat, with tassels.
    for (flap, side) in [(FLAP_L, -1.0), (FLAP_R, 1.0)] {
        let top = s.pos[flap];
        let bottom = s.at(flap, DVec3::new(side * 0.02, -0.17, 0.0));
        canvas.capsule(top, 0.06 * k, bottom, 0.035 * k, HAT_BANDS[0]);
        let tassel = s.at(flap, DVec3::new(side * 0.02, -0.3, 0.0));
        canvas.line(&[bottom, tassel], 0.01, OUTLINE);
        canvas.sphere(tassel, 0.035 * k, POMPOM);
    }

    draw_hat(canvas, s, head_depth, k);
    draw_goggles(canvas, s, head_c, head_rot, head_r, head_depth, look.time);
    draw_eyes(canvas, s, fur, shade, k, look.time);

    Anchors { mouth }
}

fn draw_hat(canvas: &mut Canvas3d, s: &Solved, head_depth: f64, k: f64) {
    let cam = canvas.camera;
    let radii = [0.19, 0.14, 0.085, 0.04].map(|r| r * k);
    let pts = HAT.map(|b| s.pos[b]);
    let projected = pts.map(|p| cam.project(p));

    // Silhouette: the union of capsules along the bent chain.
    let mut silhouette = BezPath::new();
    for i in 0..3 {
        silhouette.extend(crate::canvas3d::capsule_path(
            projected[i].pos,
            radii[i] * projected[i].scale,
            projected[i + 1].pos,
            radii[i + 1] * projected[i + 1].scale,
        ));
    }

    // Colour bands: strips of the hat's surface between rings around its axis.
    // Back strips first, then front strips over them (painter's order on a
    // convex shape).
    let segments = HAT_BANDS.len();
    let steps = 32;
    let ring = |b: usize| -> Vec<(Point, bool)> {
        let t = b as f64 / segments as f64 * 3.0;
        let i = (t.floor() as usize).min(2);
        let f = t - i as f64;
        let c = pts[i].lerp(pts[i + 1], f);
        let r = radii[i] + (radii[i + 1] - radii[i]) * f;
        let rot = s.rot[HAT[i]];
        (0..=steps)
            .map(|j| {
                let a = j as f64 / steps as f64 * std::f64::consts::TAU;
                let p = c + rot * DVec3::new(a.cos() * r, 0.0, a.sin() * r);
                (cam.point(p), p.z < c.z)
            })
            .collect()
    };
    let rings: Vec<_> = (0..=segments).map(ring).collect();
    let mut strips: Vec<(BezPath, Color, bool)> = Vec::new();
    for front in [false, true] {
        for band in 0..segments {
            let (lo, hi) = (&rings[band], &rings[band + 1]);
            let mut strip = BezPath::new();
            let mut open = false;
            for j in 0..=steps {
                if lo[j].1 == front {
                    if open { strip.line_to(lo[j].0) } else { strip.move_to(lo[j].0) }
                    open = true;
                }
            }
            for j in (0..=steps).rev() {
                if hi[j].1 == front {
                    if open { strip.line_to(hi[j].0) } else { strip.move_to(hi[j].0) }
                    open = true;
                }
            }
            if open {
                strip.close_path();
                strips.push((strip, HAT_BANDS[band], front));
            }
        }
    }
    // A zigzag-ish knitted pattern as dashes along a middle ring.
    let (zig, _) = canvas.loop_halves(
        pts[1],
        s.rot[HAT[1]] * DVec3::X * radii[1],
        s.rot[HAT[1]] * DVec3::Z * radii[1],
    );
    let zig_px = 0.03 * cam.project(pts[1]).scale;
    let size = radii[0] * projected[0].scale;
    let anchor = projected[0].pos;

    canvas.push(head_depth - 0.002, move |scene| {
        crate::paint::wash(scene, &silhouette, HAT_BANDS[5], None, size * 2.0, 17, anchor);
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &silhouette);
        for (i, (strip, color, front)) in strips.iter().enumerate() {
            let color = if *front { *color } else { darken(*color, 0.2) };
            crate::paint::wash(scene, strip, color, None, size, 100 + i as u64, anchor);
        }
        scene.stroke(
            &Stroke::new(zig_px).with_dashes(0.0, [zig_px, zig_px * 1.2]),
            Affine::IDENTITY,
            Color::WHITE.with_alpha(0.85),
            None,
            &zig,
        );
        scene.pop_layer();
        crate::paint::ink(scene, &silhouette, size * 2.0, 17, anchor);
    });
    canvas.sphere(s.at(HAT[3], DVec3::new(0.0, 0.03, 0.0)), 0.055 * k, POMPOM);
}

fn draw_goggles(canvas: &mut Canvas3d, s: &Solved, head_c: DVec3, head_rot: DQuat, head_r: DVec3, head_depth: f64, time: f64) {
    let cam = canvas.camera;
    let _ = s;
    // Strap around the head: only the half facing away shows beside the goggles.
    let strap_v: f64 = 0.28;
    let strap_c = head_c + head_rot * DVec3::new(0.0, head_r.y * strap_v.sin(), 0.0);
    let (_, back) = canvas.loop_halves(
        strap_c,
        head_rot * DVec3::X * head_r.x * 1.04 * strap_v.cos(),
        head_rot * DVec3::Z * head_r.z * 1.04 * strap_v.cos(),
    );
    let strap_px = 0.07 * cam.project(strap_c).scale;
    canvas.push(head_depth - 0.0015, move |scene| {
        scene.stroke(&Stroke::new(strap_px), Affine::IDENTITY, OUTLINE, None, &back);
    });

    let front = head_rot * DVec3::Z;
    if !cam.faces(head_c + front * head_r.z, front) {
        return;
    }
    // The goggles are drawn on the (slightly inflated) head surface; path
    // coordinates are (u, v) angles.
    let map = |u: f64, v: f64| on_ellipsoid(head_c, head_rot, head_r * 1.08, u, v);
    let frame = canvas.surface_path(&RoundedRect::new(-1.05, 0.0, 1.05, 0.6, 0.25).to_path(0.01), map);
    let lens = canvas.surface_path(&RoundedRect::new(-0.95, 0.07, 0.95, 0.53, 0.2).to_path(0.01), map);
    let lens_top = cam.point(map(0.0, 0.53));
    let lens_bottom = cam.point(map(0.0, 0.07));

    // Reflected mountains slide across the lens.
    let offset = (time * 0.25) % 0.8;
    let mut mountains = BezPath::new();
    let base = 0.22;
    mountains.move_to((-1.2 - offset, base));
    let mut u = -1.2 - offset;
    let peaks = [(0.14, 0.12), (0.08, 0.16), (0.12, 0.1), (0.09, 0.14)];
    while u < 1.2 {
        for (h, half) in peaks {
            mountains.line_to((u + half, base + h));
            u += half * 2.0;
            mountains.line_to((u, base));
        }
    }
    mountains.line_to((u, 0.0));
    mountains.line_to((-1.2 - offset, 0.0));
    mountains.close_path();
    let mountains = canvas.surface_path(&mountains, map);
    let glints: Vec<BezPath> = [(-0.45, 0.12), (-0.25, 0.05), (0.4, 0.09)]
        .iter()
        .map(|&(u0, w)| {
            let mut g = BezPath::new();
            g.move_to((u0, 0.6));
            g.line_to((u0 + w, 0.6));
            g.line_to((u0 + w - 0.2, 0.0));
            g.line_to((u0 - 0.2, 0.0));
            g.close_path();
            canvas.surface_path(&g, map)
        })
        .collect();
    let size = head_r.x * cam.project(head_c).scale;

    canvas.push(head_depth - 0.003, move |scene| {
        scene.fill(Fill::NonZero, Affine::IDENTITY, Color::from_rgb8(0x1b, 0x1b, 0x22), None, &frame);
        let sky = Gradient::new_linear(lens_top, lens_bottom).with_stops([
            Color::from_rgb8(0x2e, 0x3d, 0x8f),
            Color::from_rgb8(0x6a, 0x8c, 0xe0),
            Color::from_rgb8(0x2b, 0x25, 0x5e),
        ]);
        scene.fill(Fill::NonZero, Affine::IDENTITY, &sky, None, &lens);
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &lens);
        scene.fill(Fill::NonZero, Affine::IDENTITY, Color::from_rgb8(0xe8, 0xec, 0xff), None, &mountains);
        for g in &glints {
            scene.fill(Fill::NonZero, Affine::IDENTITY, Color::WHITE.with_alpha(0.2), None, g);
        }
        scene.pop_layer();
        scene.stroke(&outline(size), Affine::IDENTITY, OUTLINE, None, &frame);
    });
}

fn draw_eyes(canvas: &mut Canvas3d, s: &Solved, fur: Color, shade: Color, k: f64, time: f64) {
    let cam = canvas.camera;
    // Slow, sleepy blinks, each eye on its own schedule.
    let blink = |phase: f64| {
        let t = (time * 0.37 + phase) % 1.0;
        if t < 0.04 { 1.0 - (t / 0.02 - 1.0).abs() } else { 0.0 }
    };
    for (eye, side, phase) in [(EYE_L, -1.0, 0.0), (EYE_R, 1.0, 0.43)] {
        let center = s.pos[eye];
        let base = s.at(HEAD, DVec3::new(side * 0.1, 0.06, 0.04));
        canvas.capsule(base, 0.08 * k, center, 0.12 * k, shade);
        canvas.sphere(center, 0.14 * k, fur);

        let gaze = s.rot[eye] * DVec3::Z;
        if !cam.faces(center + gaze * 0.14, gaze) {
            continue;
        }
        // The eye is drawn in a small plane on the eyeball facing along the
        // gaze, with "up" aligned to the head so the lids sit right.
        let (u, v) = {
            let up = s.dir(HEAD, DVec3::Y);
            let u = up.cross(gaze).normalize_or(DVec3::X);
            (u, gaze.cross(u))
        };
        let size = 0.115 * k;
        let at = move |x: f64, y: f64| cam.point(center + gaze * 0.135 * k + (u * x + v * y) * size);
        let scale = (at(1.0, 0.0) - at(0.0, 0.0)).hypot();
        let eye_center = at(0.0, 0.0);

        // Almond-shaped opening; the heavy upper lid comes down when blinking.
        let open = 1.0 - blink(phase);
        let top = 0.2 + 0.75 * open;
        let mut opening = BezPath::new();
        opening.move_to(at(-1.15, -0.05));
        opening.curve_to(at(-0.6, top), at(0.5, top + 0.05), at(1.15, 0.05));
        opening.curve_to(at(0.6, -0.72), at(-0.55, -0.7), at(-1.15, -0.05));
        opening.close_path();
        let mut lid_edge = BezPath::new();
        lid_edge.move_to(at(-1.15, -0.05));
        lid_edge.curve_to(at(-0.6, top), at(0.5, top + 0.05), at(1.15, 0.05));
        let mut crease = BezPath::new();
        crease.move_to(at(-1.1, 0.35));
        crease.curve_to(at(-0.5, top + 0.55), at(0.5, top + 0.6), at(1.1, 0.45));
        let mut lower_rim = BezPath::new();
        lower_rim.move_to(at(-1.1, -0.1));
        lower_rim.curve_to(at(-0.55, -0.72), at(0.6, -0.74), at(1.12, 0.02));

        // Big dark iris with a horizontal oval pupil, like a real camel's.
        let iris_c = at(0.02, -0.06);
        let iris = canvas.project_ellipsoid(
            center + gaze * 0.136 * k + (u * 0.02 - v * 0.06) * size,
            DMat3::from_cols(u * 0.72 * size, v * 0.72 * size, gaze * 1e-4),
        );
        let pupil = canvas.project_ellipsoid(
            center + gaze * 0.137 * k + (u * 0.02 - v * 0.08) * size,
            DMat3::from_cols(u * 0.34 * size, v * 0.15 * size, gaze * 1e-4),
        );
        let streaks: Vec<BezPath> = (0..14)
            .map(|i| {
                let a = i as f64 / 14.0 * std::f64::consts::TAU;
                let mut l = BezPath::new();
                l.move_to(at(0.02 + a.cos() * 0.38, -0.06 + a.sin() * 0.38));
                l.line_to(at(0.02 + a.cos() * 0.66, -0.06 + a.sin() * 0.66));
                l
            })
            .collect();

        // Long, thick lashes: two rows on the upper lid, short ones below.
        let mut lashes: Vec<(BezPath, f64)> = Vec::new();
        for row in 0..2 {
            let n = 7;
            for i in 0..n {
                let t = (i as f64 + 0.5 * row as f64) / (n as f64 - 0.5);
                let x = -1.0 + 2.0 * t;
                let y = (1.0 - x * x) * (top - 0.05) - 0.02;
                let len = (0.55 + 0.35 * (1.0 - x.abs())) * (1.0 - 0.25 * row as f64);
                let out = 0.45 * x + 0.3;
                let mut lash = BezPath::new();
                lash.move_to(at(x, y));
                lash.quad_to(at(x + out * 0.4, y + len * 0.8), at(x + out * 0.9 + 0.25, y + len * 0.85));
                lashes.push((lash, if row == 0 { 1.6 } else { 1.1 }));
            }
        }
        for i in 0..5 {
            let x = -0.6 + i as f64 * 0.3;
            let y = -0.62 * (1.0 - x * x * 0.8);
            let mut lash = BezPath::new();
            lash.move_to(at(x, y));
            lash.quad_to(at(x * 1.1, y - 0.2), at(x * 1.25 + 0.1, y - 0.3));
            lashes.push((lash, 0.8));
        }

        let depth = cam.project(center).depth;
        canvas.push(depth - 1e-4, move |scene| {
            let id = Affine::IDENTITY;
            // Sclera: barely visible, ivory with pink corners.
            let sclera = Gradient::new_radial(eye_center, (1.2 * scale) as f32).with_stops([
                (0.0, EYEBALL),
                (0.7, EYEBALL),
                (1.0, Color::from_rgb8(0xd9, 0x9a, 0x8c)),
            ]);
            scene.fill(Fill::NonZero, id, &sclera, None, &opening);
            scene.push_clip_layer(Fill::NonZero, id, &opening);
            let brown = Gradient::new_radial(iris_c, (0.72 * scale) as f32).with_stops([
                (0.0, Color::from_rgb8(0x1a, 0x10, 0x0a)),
                (0.45, Color::from_rgb8(0x4a, 0x2c, 0x18)),
                (0.85, Color::from_rgb8(0x3a, 0x22, 0x12)),
                (1.0, Color::from_rgb8(0x14, 0x0c, 0x08)),
            ]);
            scene.fill(Fill::NonZero, id, &brown, None, &iris);
            for streak in &streaks {
                scene.stroke(&Stroke::new(0.8), id, Color::from_rgb8(0x7a, 0x4e, 0x2a).with_alpha(0.5), None, streak);
            }
            scene.fill(Fill::NonZero, id, Color::from_rgb8(0x08, 0x05, 0x04), None, &pupil);
            // The lid casts a soft shadow onto the top of the eye.
            let shadow = Gradient::new_linear(at(0.0, top), at(0.0, top - 0.6)).with_stops([
                Color::from_rgb8(0x20, 0x12, 0x0a).with_alpha(0.6),
                Color::from_rgb8(0x20, 0x12, 0x0a).with_alpha(0.0),
            ]);
            scene.fill(Fill::NonZero, id, &shadow, None, &opening);
            // Wet highlights: a soft sky reflection and a sharp glint.
            let sky = Gradient::new_radial(at(-0.3, 0.1), (0.45 * scale) as f32).with_stops([
                Color::WHITE.with_alpha(0.35),
                Color::WHITE.with_alpha(0.0),
            ]);
            scene.fill(Fill::NonZero, id, &sky, None, &vello::kurbo::Circle::new(at(-0.3, 0.1), 0.45 * scale));
            scene.fill(Fill::NonZero, id, Color::WHITE.with_alpha(0.95), None, &vello::kurbo::Circle::new(at(-0.28, 0.12), 0.11 * scale));
            scene.pop_layer();
            // Lids: pink lower rim, dark upper lid line and the skin crease.
            scene.stroke(&Stroke::new((0.08 * scale).max(1.0)), id, Color::from_rgb8(0xb8, 0x74, 0x66), None, &lower_rim);
            scene.stroke(&Stroke::new((0.12 * scale).max(1.4)).with_caps(vello::kurbo::Cap::Round), id, OUTLINE, None, &lid_edge);
            scene.stroke(&Stroke::new((0.06 * scale).max(0.8)), id, darken(shade, 0.35), None, &crease);
            for (lash, w) in &lashes {
                scene.stroke(&Stroke::new(w * (scale / 12.0).clamp(0.6, 1.6)).with_caps(vello::kurbo::Cap::Round), id, OUTLINE, None, lash);
            }
        });
    }
}

/// Model-to-world rotation for a heading: `facing` 1 = right, −1 = left, with
/// values in between turning through the camera (a 3/4 view at the ends).
pub fn heading(facing: f64, away: f64) -> DQuat {
    // Yaw that points model +z at world (±1, 0, −0.45); `away` turns Joe to
    // face into the screen (for climbing).
    let side = DQuat::from_rotation_y(PI - facing * 1.15);
    side.slerp(DQuat::from_rotation_y(0.0), away)
}

pub fn root(pos: DVec3, rot: DQuat, squash: f64) -> Root {
    Root {
        pos,
        rot,
        scale: DVec3::new(1.0 - squash * 0.5, 1.0 + squash, 1.0 - squash * 0.5),
    }
}

