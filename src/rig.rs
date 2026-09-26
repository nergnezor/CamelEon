//! Skeletal rig: bones with rest offsets, poses as per-bone rotations, and
//! forward kinematics. Poses blend with slerp, which is what makes it easy to
//! cross-fade between animations smoothly.

use glam::{DQuat, DVec3};

pub struct Bone {
    /// Must come before this bone in the skeleton.
    pub parent: Option<usize>,
    /// Position relative to the parent bone, in the parent's rest frame.
    pub offset: DVec3,
}

pub struct Skeleton {
    pub bones: Vec<Bone>,
}

/// Local rotation of every bone relative to its rest orientation.
#[derive(Clone)]
pub struct Pose {
    pub rot: Vec<DQuat>,
}

impl Pose {
    pub fn rest(bones: usize) -> Self {
        Self {
            rot: vec![DQuat::IDENTITY; bones],
        }
    }

    /// Composes an extra rotation onto a bone.
    pub fn rotate(&mut self, bone: usize, q: DQuat) {
        self.rot[bone] = self.rot[bone] * q;
    }

    /// Blends towards `other` by `w` (0 = self, 1 = other).
    pub fn blend(&mut self, other: &Pose, w: f64) {
        if w <= 0.0 {
            return;
        }
        for (a, b) in self.rot.iter_mut().zip(&other.rot) {
            *a = a.slerp(*b, w.min(1.0));
        }
    }
}

/// Where Joe is in the world: position of the feet, heading and squash/stretch.
#[derive(Clone, Copy)]
pub struct Root {
    pub pos: DVec3,
    pub rot: DQuat,
    /// Non-uniform scale in model space, applied around the feet.
    pub scale: DVec3,
}

/// Bone frames in world space after forward kinematics.
pub struct Solved {
    pub pos: Vec<DVec3>,
    pub rot: Vec<DQuat>,
    pub root: Root,
}

impl Solved {
    pub fn solve(skeleton: &Skeleton, pose: &Pose, root: Root) -> Self {
        let n = skeleton.bones.len();
        let mut model_pos = vec![DVec3::ZERO; n];
        let mut model_rot = vec![DQuat::IDENTITY; n];
        for (i, bone) in skeleton.bones.iter().enumerate() {
            match bone.parent {
                Some(p) => {
                    model_pos[i] = model_pos[p] + model_rot[p] * bone.offset;
                    model_rot[i] = model_rot[p] * pose.rot[i];
                }
                None => {
                    model_pos[i] = bone.offset;
                    model_rot[i] = pose.rot[i];
                }
            }
        }
        let pos = model_pos
            .iter()
            .map(|&p| root.pos + root.rot * (p * root.scale))
            .collect();
        let rot = model_rot.iter().map(|&r| root.rot * r).collect();
        Self { pos, rot, root }
    }

    /// A point given in a bone's local frame, in world space.
    pub fn at(&self, bone: usize, local: DVec3) -> DVec3 {
        self.pos[bone] + self.root.rot * (self.model_dir(bone, local) * self.root.scale)
    }

    /// A direction given in a bone's local frame, in world space (unscaled).
    pub fn dir(&self, bone: usize, local: DVec3) -> DVec3 {
        self.rot[bone] * local
    }

    fn model_dir(&self, bone: usize, local: DVec3) -> DVec3 {
        self.root.rot.inverse() * (self.rot[bone] * local)
    }
}
