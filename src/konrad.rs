//! Konrad: the hero, an agent in the style of early-90s cinematic platformers
//! (think Flashback): realistic proportions, smooth lifelike movement, and flat
//! colour shading without outlines. Brown hair, an off-white long-sleeved
//! shirt, grey-blue jeans, boots and a belt with a holster. He fires a
//! grappling line from his right hand.
//!
//! Konrad is a 3D rig (see `rig`) drawn with vector primitives (see `canvas3d`).
//! Model space: x = his right, y = up, z = forward. Feet at the origin.

use std::f64::consts::PI;

use glam::{DMat3, DQuat, DVec3};
use vello::kurbo::{Affine, BezPath, Ellipse, Shape, Stroke};
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

/// Standing: slow breathing, a weight shift, a glance around.
fn idle(m: &Motion) -> Pose {
    let mut p = Pose::rest(COUNT);
    let breath = (m.time * 1.6).sin();
    let shift = (m.time * 0.45).sin();
    p.rotate(PELVIS, rz(0.03 * shift));
    p.rotate(SPINE, rx(0.02 * breath) * rz(-0.03 * shift));
    p.rotate(CHEST, rx(0.015 * breath));
    p.rotate(HEAD, ry(0.35 * (m.time * 0.3).sin().powi(3)) * rx(0.05));
    for (hip, knee, side) in [(HIP_L, KNEE_L, -1.0), (HIP_R, KNEE_R, 1.0)] {
        p.rotate(hip, rz(side * 0.05) * rx(-0.04));
        p.rotate(knee, rx(0.06 + 0.04 * (shift * side).max(0.0)));
    }
    for (shoulder, elbow, side) in [(SHOULDER_L, ELBOW_L, -1.0), (SHOULDER_R, ELBOW_R, 1.0)] {
        p.rotate(shoulder, rz(side * 0.1) * rx(0.05 * breath));
        p.rotate(elbow, rx(-0.25));
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

fn on_ellipsoid(center: DVec3, rot: DQuat, radii: DVec3, u: f64, v: f64) -> DVec3 {
    let dir = DVec3::new(u.sin() * v.cos(), v.sin(), u.cos() * v.cos());
    center + rot * (dir * radii)
}

fn axes(rot: DQuat, radii: DVec3, scale: DVec3) -> DMat3 {
    DMat3::from_quat(rot) * DMat3::from_diagonal(radii * scale)
}

pub fn draw(canvas: &mut Canvas3d, s: &Solved, look: &Look) -> Anchors {
    let k = (s.root.scale.x + s.root.scale.y + s.root.scale.z) / 3.0;
    let sc = s.root.scale;

    // Legs: jeans and boots.
    for (hip, knee, foot) in [(HIP_L, KNEE_L, FOOT_L), (HIP_R, KNEE_R, FOOT_R)] {
        canvas.capsule(s.pos[hip], 0.078 * k, s.pos[knee], 0.058 * k, JEANS);
        canvas.capsule(s.pos[knee], 0.058 * k, s.at(foot, DVec3::new(0.0, 0.06, 0.0)), 0.048 * k, JEANS);
        canvas.ellipsoid(s.at(foot, DVec3::new(0.0, -0.005, 0.05)), axes(s.rot[foot], DVec3::new(0.05, 0.045, 0.12), sc), BOOT);
    }

    // Hips, belt and holster.
    let pelvis_rot = s.rot[PELVIS];
    canvas.ellipsoid(s.pos[PELVIS], axes(pelvis_rot, DVec3::new(0.165, 0.11, 0.105), sc), JEANS);
    canvas.ellipsoid(s.at(PELVIS, DVec3::new(0.0, 0.09, 0.0)), axes(pelvis_rot, DVec3::new(0.168, 0.028, 0.108), sc), BELT);
    canvas.ellipsoid(s.at(PELVIS, DVec3::new(0.165, 0.0, 0.02)), axes(pelvis_rot, DVec3::new(0.035, 0.085, 0.05), sc), BELT);

    // Torso: waist and chest in the shirt.
    canvas.capsule(s.at(PELVIS, DVec3::new(0.0, 0.1, 0.0)), 0.14 * k, s.at(CHEST, DVec3::new(0.0, 0.0, 0.0)), 0.155 * k, SHIRT);
    canvas.ellipsoid(s.at(CHEST, DVec3::new(0.0, 0.06, 0.0)), axes(s.rot[CHEST], DVec3::new(0.19, 0.16, 0.115), sc), SHIRT);

    // Arms in long sleeves, bare hands.
    for (shoulder, elbow, hand) in [(SHOULDER_L, ELBOW_L, HAND_L), (SHOULDER_R, ELBOW_R, HAND_R)] {
        canvas.capsule(s.pos[shoulder], 0.058 * k, s.pos[elbow], 0.047 * k, SHIRT);
        canvas.capsule(s.pos[elbow], 0.047 * k, s.at(hand, DVec3::new(0.0, 0.02, 0.0)), 0.038 * k, SHIRT);
        canvas.ellipsoid(s.at(hand, DVec3::new(0.0, -0.055, 0.0)), axes(s.rot[hand], DVec3::new(0.035, 0.065, 0.028), sc), SKIN);
    }

    // Neck and head.
    canvas.capsule(s.at(CHEST, DVec3::new(0.0, 0.14, 0.0)), 0.05 * k, s.at(HEAD, DVec3::new(0.0, -0.08, -0.01)), 0.045 * k, SKIN);
    let head_c = s.pos[HEAD];
    let head_rot = s.rot[HEAD];
    let head_r = DVec3::new(0.092, 0.118, 0.104) * sc;
    // Hair: a cap just behind and above the head, so it shows on top and at
    // the back while the face covers its front; plus a fringe.
    canvas.ellipsoid(s.at(HEAD, DVec3::new(0.0, 0.035, -0.025)), axes(head_rot, DVec3::new(0.1, 0.1, 0.105), sc), HAIR);
    canvas.ellipsoid(head_c, axes(head_rot, head_r, DVec3::ONE), SKIN);
    canvas.capsule(s.at(HEAD, DVec3::new(-0.06, 0.085, 0.06)), 0.03 * k, s.at(HEAD, DVec3::new(0.05, 0.1, 0.055)), 0.028 * k, HAIR);
    canvas.ellipsoid(s.at(HEAD, DVec3::new(0.0, -0.005, 0.105)), axes(head_rot, DVec3::new(0.018, 0.03, 0.025), sc), SKIN);
    for side in [-1.0, 1.0] {
        canvas.ellipsoid(s.at(HEAD, DVec3::new(side * 0.09, 0.0, -0.005)), axes(head_rot, DVec3::new(0.015, 0.03, 0.02), sc), SKIN);
    }
    draw_face(canvas, head_c, head_rot, head_r, look);

    Anchors {
        hand: s.at(HAND_R, DVec3::new(0.0, -0.09, 0.0)),
    }
}

/// Small, understated features: eyes, brows, mouth.
fn draw_face(canvas: &mut Canvas3d, head_c: DVec3, head_rot: DQuat, head_r: DVec3, look: &Look) {
    let cam = canvas.camera;
    let front = head_rot * DVec3::Z;
    if !cam.faces(head_c + front * head_r.z, front) {
        return;
    }
    let map = move |u: f64, v: f64| on_ellipsoid(head_c, head_rot, head_r * 1.01, u, v);
    let blink = {
        let t = (look.time * 0.23) % 1.0;
        t < 0.03
    };
    let mut paths: Vec<(BezPath, f64)> = Vec::new();
    for side in [-1.0, 1.0] {
        let eye = if blink {
            let mut l = BezPath::new();
            l.move_to((side * 0.38 - 0.08, 0.12));
            l.line_to((side * 0.38 + 0.08, 0.12));
            (l, 0.0)
        } else {
            (Ellipse::new((side * 0.38, 0.12), (0.05, 0.07), 0.0).to_path(0.01), 1.0)
        };
        paths.push((canvas.surface_path(&eye.0, map), eye.1));
        let mut brow = BezPath::new();
        brow.move_to((side * 0.26, 0.3));
        brow.line_to((side * 0.52, 0.28));
        paths.push((canvas.surface_path(&brow, map), 0.0));
    }
    let mut mouth = BezPath::new();
    mouth.move_to((-0.14, -0.42));
    mouth.line_to((0.14, -0.42));
    paths.push((canvas.surface_path(&mouth, map), 0.0));
    let px = (cam.project(head_c).scale * 0.012).max(1.0);
    let depth = cam.project(head_c).depth;
    canvas.push(depth - 1e-4, move |scene| {
        for (path, fill) in &paths {
            if *fill > 0.0 {
                scene.fill(Fill::NonZero, Affine::IDENTITY, FEATURE, None, path);
            } else {
                scene.stroke(&Stroke::new(px), Affine::IDENTITY, FEATURE, None, path);
            }
        }
    });
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

