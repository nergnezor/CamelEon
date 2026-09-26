//! A tiny "vector 3D" renderer: 3D primitives are projected with a perspective
//! camera and drawn as 2D vector shapes (with outlines and gradient shading),
//! sorted back to front (painter's algorithm).
//!
//! World space: x right, y up, z away from the camera.

use std::f64::consts::{PI, TAU};

use glam::{DMat3, DVec3};
use vello::kurbo::{Affine, Arc, BezPath, Circle, Ellipse, Join, Point, Shape, Stroke, Vec2};
use vello::peniko::Color;
use vello::Scene;

pub use crate::paint::{darken, lighten, INK as OUTLINE};
use crate::paint;

#[derive(Clone, Copy)]
pub struct Camera {
    /// Camera position in world space. It always looks along +z.
    pub eye: DVec3,
    /// Pixels per world unit at distance 1.
    pub focal: f64,
    /// Screen position of the optical center.
    pub center: Point,
}

/// A projected point: screen position, depth along the view axis and the
/// local scale (pixels per world unit at that depth).
#[derive(Clone, Copy)]
pub struct Projected {
    pub pos: Point,
    pub depth: f64,
    pub scale: f64,
}

impl Camera {
    pub fn project(&self, p: DVec3) -> Projected {
        let rel = p - self.eye;
        let depth = rel.z.max(0.05);
        let scale = self.focal / depth;
        Projected {
            pos: Point::new(self.center.x + rel.x * scale, self.center.y - rel.y * scale),
            depth,
            scale,
        }
    }

    pub fn point(&self, p: DVec3) -> Point {
        self.project(p).pos
    }

    /// The affine approximation of the projection around `p`: maps a 3D offset
    /// (in world units) to a 2D screen offset.
    fn jacobian(&self, p: DVec3) -> [[f64; 3]; 2] {
        let rel = p - self.eye;
        let z = rel.z.max(0.05);
        let k = self.focal / z;
        [[k, 0.0, -k * rel.x / z], [0.0, -k, k * rel.y / z]]
    }

    /// Projects a 3D direction at `p` to a screen-space vector.
    pub fn project_dir(&self, p: DVec3, d: DVec3) -> Vec2 {
        let j = self.jacobian(p);
        Vec2::new(
            j[0][0] * d.x + j[0][1] * d.y + j[0][2] * d.z,
            j[1][0] * d.x + j[1][1] * d.y + j[1][2] * d.z,
        )
    }

    /// Whether a surface at `p` with normal `n` faces the camera.
    pub fn faces(&self, p: DVec3, n: DVec3) -> bool {
        (self.eye - p).dot(n) > 0.0
    }

    /// The range of world x visible at depth `z` (world z), with some margin.
    pub fn visible_x(&self, z: f64, screen_width: f64) -> (f64, f64) {
        let depth = (z - self.eye.z).max(0.05);
        let margin = 80.0;
        let to_world = |sx: f64| self.eye.x + (sx - self.center.x) * depth / self.focal;
        (to_world(-margin), to_world(screen_width + margin))
    }
}

type DrawFn<'a> = Box<dyn FnOnce(&mut Scene) + 'a>;

/// Collects draw commands with a depth and emits them far to near.
pub struct Canvas3d<'a> {
    pub camera: Camera,
    items: Vec<(f64, u32, DrawFn<'a>)>,
    order: u32,
}

impl<'a> Canvas3d<'a> {
    pub fn new(camera: Camera) -> Self {
        Self {
            camera,
            items: Vec::new(),
            order: 0,
        }
    }

    /// Queues a custom draw closure at the given view depth. Items with equal
    /// depth are drawn in submission order.
    pub fn push(&mut self, depth: f64, draw: impl FnOnce(&mut Scene) + 'a) {
        self.items.push((depth, self.order, Box::new(draw)));
        self.order += 1;
    }

    pub fn depth_of(&self, p: DVec3) -> f64 {
        p.z - self.camera.eye.z
    }

    pub fn finish(mut self, scene: &mut Scene) {
        self.items
            .sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, _, draw) in self.items {
            draw(scene);
        }
    }

    pub fn sphere(&mut self, center: DVec3, radius: f64, color: Color) {
        let p = self.camera.project(center);
        let r = radius * p.scale;
        let seed = color_seed(color, radius);
        self.push(p.depth, move |scene| {
            let circle = Circle::new(p.pos, r).to_path(0.1);
            paint::wash(scene, &circle, color, Some(&paint::ball_glaze(p.pos, r, color)), r * 2.0, seed, p.pos);
            paint::ink(scene, &circle, r * 2.0, seed, p.pos);
        });
    }

    /// An ellipsoid whose semi-axes are the columns of `axes` (world space).
    pub fn ellipsoid(&mut self, center: DVec3, axes: DMat3, color: Color) {
        let p = self.camera.project(center);
        let ellipse = self.project_ellipsoid(center, axes);
        let r = ellipse.radii().x.max(ellipse.radii().y);
        let seed = color_seed(color, axes.x_axis.length() + axes.y_axis.length());
        self.push(p.depth, move |scene| {
            let path = ellipse.to_path(0.1);
            paint::wash(scene, &path, color, Some(&paint::ball_glaze(p.pos, r, color)), r * 2.0, seed, p.pos);
            paint::ink(scene, &path, r * 2.0, seed, p.pos);
        });
    }

    /// The screen-space silhouette of an ellipsoid (affine approximation).
    pub fn project_ellipsoid(&self, center: DVec3, axes: DMat3) -> Ellipse {
        let p = self.camera.project(center);
        let cols = [axes.x_axis, axes.y_axis, axes.z_axis].map(|a| self.camera.project_dir(center, a));
        // Silhouette = image of the unit sphere under A (2x3); its shape is A·Aᵀ.
        let (mut a, mut b, mut c) = (0.0, 0.0, 0.0);
        for v in cols {
            a += v.x * v.x;
            b += v.x * v.y;
            c += v.y * v.y;
        }
        let tr = a + c;
        let det = a * c - b * b;
        let disc = (tr * tr / 4.0 - det).max(0.0).sqrt();
        let l1 = tr / 2.0 + disc;
        let l2 = (tr / 2.0 - disc).max(0.0);
        let angle = if b.abs() < 1e-12 {
            if a >= c { 0.0 } else { PI / 2.0 }
        } else {
            (l1 - a).atan2(b)
        };
        Ellipse::new(p.pos, (l1.sqrt(), l2.sqrt()), angle)
    }

    /// A tapered capsule: two spheres joined by their common tangents.
    pub fn capsule(&mut self, a: DVec3, ra: f64, b: DVec3, rb: f64, color: Color) {
        let pa = self.camera.project(a);
        let pb = self.camera.project(b);
        let path = capsule_path(pa.pos, ra * pa.scale, pb.pos, rb * pb.scale);
        let width = (ra * pa.scale).max(rb * pb.scale);
        let axis = pb.pos - pa.pos;
        let depth = (pa.depth + pb.depth) / 2.0;
        let seed = color_seed(color, ra + rb * 3.0);
        self.push(depth, move |scene| {
            let glaze = paint::tube_glaze(pa.pos.midpoint(pb.pos), axis, width, color);
            paint::wash(scene, &path, color, Some(&glaze), width * 2.0, seed, pa.pos);
            paint::ink(scene, &path, width * 2.0, seed, pa.pos);
        });
    }

    /// A flat 3D line with a fixed world-space width, without outline.
    pub fn line(&mut self, points: &[DVec3], width: f64, color: Color) {
        if points.len() < 2 {
            return;
        }
        let mut path = BezPath::new();
        let mut depth = 0.0;
        let mut scale = 0.0;
        for (i, &p) in points.iter().enumerate() {
            let pr = self.camera.project(p);
            depth += pr.depth;
            scale += pr.scale;
            if i == 0 {
                path.move_to(pr.pos);
            } else {
                path.line_to(pr.pos);
            }
        }
        let n = points.len() as f64;
        let stroke = Stroke::new(width * scale / n)
            .with_caps(vello::kurbo::Cap::Round)
            .with_join(Join::Round);
        self.push(depth / n, move |scene| {
            scene.stroke(&stroke, Affine::IDENTITY, color, None, &path);
        });
    }

    /// Draws a 2D path mapped onto a surface. `map` turns path coordinates into
    /// world points; the path is flattened and every vertex projected.
    pub fn surface_path(&self, path: &BezPath, map: impl Fn(f64, f64) -> DVec3) -> BezPath {
        let mut out = BezPath::new();
        vello::kurbo::flatten(path, 0.02, |el| {
            use vello::kurbo::PathEl;
            match el {
                PathEl::MoveTo(p) => out.move_to(self.camera.point(map(p.x, p.y))),
                PathEl::LineTo(p) => out.line_to(self.camera.point(map(p.x, p.y))),
                PathEl::ClosePath => out.close_path(),
                _ => {}
            }
        });
        out
    }

    /// Screen path of a closed 3D ellipse `c + a·cos t + b·sin t`, split into
    /// the half facing the camera and the half facing away.
    pub fn loop_halves(&self, c: DVec3, a: DVec3, b: DVec3) -> (BezPath, BezPath) {
        let steps = 48;
        let mut front = BezPath::new();
        let mut back = BezPath::new();
        let mut prev_front: Option<bool> = None;
        for i in 0..=steps {
            let t = i as f64 / steps as f64 * TAU;
            let p = c + a * t.cos() + b * t.sin();
            let is_front = p.z < c.z;
            let screen = self.camera.point(p);
            let path = if is_front { &mut front } else { &mut back };
            if prev_front != Some(is_front) {
                path.move_to(screen);
            } else {
                path.line_to(screen);
            }
            // Also continue the other half up to this point so there is no gap.
            if let Some(was) = prev_front {
                let other = if was { &mut front } else { &mut back };
                if was != is_front && !other.elements().is_empty() {
                    other.line_to(screen);
                }
            }
            prev_front = Some(is_front);
        }
        (front, back)
    }
}

/// A stable per-part seed so the watercolour edges don't shimmer.
fn color_seed(color: Color, size: f64) -> u64 {
    let [r, g, b, _] = color.components;
    paint::seed(&[r as f64, g as f64, b as f64, size])
}

/// Outline width that scales gently with the shape's size.
pub fn outline(size: f64) -> Stroke {
    Stroke::new((size * 0.08).clamp(1.0, 4.0)).with_join(Join::Round)
}

pub fn mix(a: Color, b: Color, t: f64) -> Color {
    let t = t.clamp(0.0, 1.0) as f32;
    let [ar, ag, ab, aa] = a.components;
    let [br, bg, bb, ba] = b.components;
    Color::new([ar + (br - ar) * t, ag + (bg - ag) * t, ab + (bb - ab) * t, aa + (ba - aa) * t])
}

/// The 2D hull of two circles: a tapered capsule outline.
pub fn capsule_path(c0: Point, r0: f64, c1: Point, r1: f64) -> BezPath {
    let d = c1 - c0;
    let len = d.hypot();
    if len <= (r0 - r1).abs() + 1e-6 {
        let (c, r) = if r0 >= r1 { (c0, r0) } else { (c1, r1) };
        return Circle::new(c, r).to_path(0.1);
    }
    let base = d.atan2();
    let phi = ((r0 - r1) / len).clamp(-1.0, 1.0).acos();
    let at = |c: Point, r: f64, a: f64| c + Vec2::new(a.cos(), a.sin()) * r;
    let mut path = BezPath::new();
    path.move_to(at(c0, r0, base + phi));
    path.line_to(at(c1, r1, base + phi));
    path.extend(Arc::new(c1, (r1, r1), base + phi, -2.0 * phi, 0.0).append_iter(0.1));
    path.line_to(at(c0, r0, base - phi));
    path.extend(Arc::new(c0, (r0, r0), base - phi, -(TAU - 2.0 * phi), 0.0).append_iter(0.1));
    path.close_path();
    path
}
