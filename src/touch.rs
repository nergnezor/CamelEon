//! On-screen touch controls for phones and tablets: a floating analog stick
//! on the left half of the screen (it appears where the thumb lands) and
//! a jump button on the right (anywhere on the right half jumps).

use vello::kurbo::{Affine, BezPath, Circle, Point, Stroke, Vec2};
use vello::peniko::{Color, Fill};
use vello::Scene;
use winit::event::TouchPhase;

use crate::game::Input;

struct Stick {
    id: u64,
    origin: Point,
    pos: Point,
}

#[derive(Default)]
pub struct TouchControls {
    stick: Option<Stick>,
    jump: Option<u64>,
    /// The overlay only shows once the screen has been touched.
    used: bool,
    size: (f64, f64),
}

impl TouchControls {
    fn radius(&self) -> f64 {
        self.size.0.min(self.size.1) * 0.085
    }

    fn jump_button(&self) -> Point {
        let r = self.radius();
        Point::new(self.size.0 - r * 1.5, self.size.1 - r * 1.6)
    }

    pub fn resize(&mut self, w: f64, h: f64) {
        self.size = (w, h);
    }

    pub fn event(&mut self, id: u64, phase: TouchPhase, pos: Point) {
        self.used = true;
        match phase {
            TouchPhase::Started => {
                if pos.x < self.size.0 * 0.5 {
                    if self.stick.is_none() {
                        self.stick = Some(Stick { id, origin: pos, pos });
                    }
                } else {
                    self.jump = Some(id);
                }
            }
            TouchPhase::Moved => {
                if let Some(stick) = &mut self.stick {
                    if stick.id == id {
                        stick.pos = pos;
                    }
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                if self.stick.as_ref().is_some_and(|s| s.id == id) {
                    self.stick = None;
                }
                if self.jump == Some(id) {
                    self.jump = None;
                }
            }
        }
    }

    pub fn input(&self) -> Input {
        let mut input = Input {
            jump: self.jump.is_some(),
            ..Input::default()
        };
        if let Some(stick) = &self.stick {
            let d = (stick.pos - stick.origin) / self.radius();
            let d = if d.hypot() > 1.0 { d.normalize() } else { d };
            // A small deadzone so resting the thumb doesn't creep.
            if d.hypot() > 0.12 {
                input.stick_x = d.x;
            }
        }
        input
    }

    pub fn draw(&self, scene: &mut Scene) {
        if !self.used {
            return;
        }
        let r = self.radius();
        let id = Affine::IDENTITY;
        let ink = Color::from_rgb8(0x3b, 0x2a, 0x1e);
        let ring = |scene: &mut Scene, c: Point, radius: f64, pressed: bool| {
            let circle = Circle::new(c, radius);
            let fill = if pressed { 0.45 } else { 0.22 };
            scene.fill(Fill::NonZero, id, Color::from_rgb8(0xfb, 0xf4, 0xe4).with_alpha(fill), None, &circle);
            scene.stroke(&Stroke::new(2.5), id, ink.with_alpha(0.6), None, &circle);
        };

        if let Some(stick) = &self.stick {
            ring(scene, stick.origin, r, false);
            let d = stick.pos - stick.origin;
            let knob = stick.origin + if d.hypot() > r { d.normalize() * r } else { d };
            ring(scene, knob, r * 0.45, true);
        } else {
            // A faint hint of where to put the thumb.
            let hint = Point::new(r * 2.0, self.size.1 - r * 2.0);
            scene.stroke(&Stroke::new(2.0).with_dashes(0.0, [8.0, 8.0]), id, ink.with_alpha(0.35), None, &Circle::new(hint, r));
        }

        // Jump: an up arrow.
        let j = self.jump_button();
        ring(scene, j, r, self.jump.is_some());
        let mut arrow = BezPath::new();
        arrow.move_to(j + Vec2::new(-r * 0.35, r * 0.15));
        arrow.line_to(j + Vec2::new(0.0, -r * 0.3));
        arrow.line_to(j + Vec2::new(r * 0.35, r * 0.15));
        scene.stroke(&Stroke::new(r * 0.12), id, ink.with_alpha(0.75), None, &arrow);
    }
}
