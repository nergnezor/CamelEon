//! Game state and the frame loop shared by the window and terminal frontends.

use glam::{DQuat, DVec2, DVec3};
use vello::kurbo::{Affine, BezPath, Circle, Point, Rect, Stroke, Vec2};
use vello::peniko::{Color, Fill};
use vello::Scene;

use crate::camel_joe::{self, Animator, Look, Motion};
use crate::canvas3d::{Camera, Canvas3d, OUTLINE};
use crate::jungle::{self, WorldView};
use crate::level::{self, Level};
use crate::player::{Controls, Player, State, TongueTarget, MOUTH_HEIGHT};
use crate::rig::{Skeleton, Solved};

/// Distance from the camera to the gameplay plane.
const CAMERA_DISTANCE: f64 = 11.0;
/// World units visible vertically in the gameplay plane: standing still, and
/// at full running speed (the camera zooms out the faster Joe goes).
const VIEW_HEIGHT: f64 = 8.5;
const VIEW_HEIGHT_FAST: f64 = 13.0;
const MAX_STEP: f64 = 1.0 / 120.0;
const TONGUE: Color = Color::from_rgb8(0xe8, 0x5f, 0x8a);

/// Which controls are currently held down, plus analog stick axes.
#[derive(Default, Clone, Copy)]
pub struct Input {
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub jump: bool,
    pub tongue: bool,
    /// Analog stick, −1..1 (x right, y up); zero when no stick is used.
    pub stick_x: f64,
    pub stick_y: f64,
}

impl Input {
    /// Combines two input sources: a button is down if it's down in either,
    /// and the stick that's pushed further wins.
    pub fn merge(a: Input, b: Input) -> Input {
        let pick = |x: f64, y: f64| if x.abs() > y.abs() { x } else { y };
        Input {
            left: a.left || b.left,
            right: a.right || b.right,
            up: a.up || b.up,
            down: a.down || b.down,
            jump: a.jump || b.jump,
            tongue: a.tongue || b.tongue,
            stick_x: pick(a.stick_x, b.stick_x),
            stick_y: pick(a.stick_y, b.stick_y),
        }
    }

    /// Horizontal and vertical axes: the stick if it's pushed, else the keys.
    fn axes(&self) -> (f64, f64) {
        let keys = |neg: bool, pos: bool| pos as i32 as f64 - neg as i32 as f64;
        let pick = |stick: f64, key: f64| if stick.abs() > key.abs() { stick } else { key };
        (
            pick(self.stick_x, keys(self.left, self.right)),
            pick(self.stick_y, keys(self.down, self.up)),
        )
    }
}

struct Particle {
    pos: DVec3,
    vel: DVec3,
    life: f64,
    color: Color,
    size: f64,
}

pub struct Game {
    /// Keyboard state, set by the frontend.
    pub input: Input,
    /// Gamepad state, set by the frontend.
    pub pad: Input,
    prev_jump: bool,
    prev_tongue: bool,
    level: Level,
    player: Player,
    fly_homes: Vec<DVec2>,
    flies: Vec<DVec2>,
    caught: Vec<bool>,
    checkpoint: usize,
    skeleton: Skeleton,
    animator: Animator,
    /// Smoothed facing (−1..1) and "facing into the screen" (0..1) for turning.
    turn: f64,
    away: f64,
    squash: f64,
    squash_vel: f64,
    camo: f64,
    camera: DVec2,
    view_height: f64,
    time: f64,
    won_at: Option<f64>,
    flash: f64,
    particles: Vec<Particle>,
}

impl Game {
    pub fn new() -> Self {
        let level = level::jungle();
        let start = level.checkpoints[0];
        let fly_homes = level.flies.clone();
        Self {
            input: Input::default(),
            pad: Input::default(),
            prev_jump: false,
            prev_tongue: false,
            player: Player::new(start),
            flies: fly_homes.clone(),
            caught: vec![false; fly_homes.len()],
            fly_homes,
            level,
            checkpoint: 0,
            skeleton: camel_joe::skeleton(),
            animator: Animator::default(),
            turn: 1.0,
            away: 0.0,
            squash: 0.0,
            squash_vel: 0.0,
            camo: 0.0,
            camera: start + DVec2::new(2.0, 1.7),
            view_height: VIEW_HEIGHT,
            time: 0.0,
            won_at: None,
            flash: 0.0,
            particles: Vec::new(),
        }
    }

    /// Advances the game by `dt` seconds, in small fixed-size steps.
    pub fn update(&mut self, dt: f64) {
        let mut left = dt.min(0.1);
        while left > 0.0 {
            let step = left.min(MAX_STEP);
            self.step(step);
            left -= step;
        }
    }

    fn step(&mut self, dt: f64) {
        self.time += dt;
        let (k, p) = (self.input, self.pad);
        let (kx, ky) = k.axes();
        let (px, py) = p.axes();
        let pick = |a: f64, b: f64| if a.abs() > b.abs() { a } else { b };
        let (x, y) = (pick(kx, px), pick(ky, py));
        let input = Input {
            up: y > 0.3,
            down: y < -0.3,
            jump: k.jump || p.jump,
            tongue: k.tongue || p.tongue,
            ..Input::default()
        };
        let mut controls = Controls {
            x,
            y,
            up: input.up,
            down: input.down,
            jump: input.jump,
            jump_pressed: input.jump && !self.prev_jump,
            tongue_pressed: input.tongue && !self.prev_tongue,
        };
        self.prev_jump = input.jump;
        self.prev_tongue = input.tongue;

        for (i, home) in self.fly_homes.iter().enumerate() {
            self.flies[i] = jungle::fly_position(*home, self.time, i);
        }

        if let Some(won) = self.won_at {
            if self.time - won > 6.0 {
                *self = Game::new();
                return;
            }
            // Victory: ignore the player and jump for joy.
            let since = self.time - won;
            controls = Controls {
                jump: true,
                jump_pressed: since % 0.9 < dt,
                ..Controls::default()
            };
        }

        let events = self.player.update(dt, &controls, &self.level, &self.flies, &self.caught);
        if let Some(i) = events.caught_fly {
            self.caught[i] = true;
            let p = self.flies[i];
            self.burst(DVec3::new(p.x, p.y, 0.0), Color::from_rgb8(0xff, 0xe0, 0x60), 14);
        }
        if let Some(speed) = events.landed {
            if speed > 5.0 {
                self.squash_vel -= speed * 0.25;
                let feet = DVec3::new(self.player.pos.x, self.player.pos.y, 0.0);
                self.burst(feet, Color::from_rgb8(0x9a, 0x7a, 0x50), (speed as usize / 3).min(10));
            }
        }
        if events.jumped {
            self.squash_vel += 4.0;
        }
        if events.died {
            let at = self.level.checkpoints[self.checkpoint];
            self.player.respawn(at);
            self.flash = 1.0;
        }
        let reached = (self.checkpoint + 1..self.level.checkpoints.len()).rev().find(|&i| {
            self.player.on_ground && self.player.pos.x >= self.level.checkpoints[i].x - 0.5
        });
        if let Some(i) = reached {
            self.checkpoint = i;
            let cp = self.level.checkpoints[i];
            self.burst(DVec3::new(cp.x, cp.y + 2.0, 0.7), Color::from_rgb8(0xf2, 0x7a, 0x2e), 12);
        }
        let center = self.player.pos + DVec2::new(0.0, 0.8);
        if self.won_at.is_none() && center.distance(self.level.goal + DVec2::new(0.0, 2.2)) < 2.0 {
            self.won_at = Some(self.time);
            let g = self.level.goal;
            self.burst(DVec3::new(g.x, g.y + 2.2, 0.3), Color::from_rgb8(0xff, 0xd0, 0x40), 60);
        }

        // Squash and stretch spring.
        let accel = -self.squash * 220.0 - self.squash_vel * 14.0;
        self.squash_vel += accel * dt;
        self.squash = (self.squash + self.squash_vel * dt).clamp(-0.35, 0.35);

        // Turning: facing changes swing Joe round through the camera.
        let k = 1.0 - (-dt * 10.0).exp();
        self.turn += (self.player.facing - self.turn) * k;
        let climbing = matches!(self.player.state, State::Climbing(_));
        self.away += (climbing as u8 as f64 - self.away) * k;

        // Chameleon camouflage when standing still.
        let camo_target = if self.player.idle > 1.2 { 1.0 } else { 0.0 };
        self.camo += (camo_target - self.camo) * (1.0 - (-dt * 2.0).exp());

        let motion = self.motion();
        self.animator.update(dt, &motion);

        // Zoom out with speed (slowly, so it breathes rather than pumps).
        let speed = (self.player.vel.length() / 17.0).min(1.0);
        let target_view = VIEW_HEIGHT + (VIEW_HEIGHT_FAST - VIEW_HEIGHT) * speed;
        self.view_height += (target_view - self.view_height) * (1.0 - (-dt * 1.2).exp());

        // Camera follows with a little look-ahead.
        let target = self.player.pos + DVec2::new(self.turn * 2.0 + self.player.vel.x * 0.3, 1.7);
        let ck = DVec2::new(1.0 - (-dt * 3.5).exp(), 1.0 - (-dt * 2.5).exp());
        self.camera += (target - self.camera) * ck;
        self.camera.y = self.camera.y.max(self.level.kill_y + 6.0);

        self.update_effects(dt);
    }

    fn update_effects(&mut self, dt: f64) {
        self.flash = (self.flash - dt * 1.5).max(0.0);
        for p in &mut self.particles {
            p.vel.y -= 12.0 * dt;
            p.pos += p.vel * dt;
            p.life -= dt;
        }
        self.particles.retain(|p| p.life > 0.0);
    }

    fn burst(&mut self, at: DVec3, color: Color, count: usize) {
        for i in 0..count {
            let a = i as f64 * 2.399 + self.time * 7.0;
            let speed = 2.0 + (i % 5) as f64;
            self.particles.push(Particle {
                pos: at,
                vel: DVec3::new(a.cos() * speed, 3.0 + a.sin().abs() * speed, (a * 1.7).sin() * 1.5),
                life: 0.6 + (i % 3) as f64 * 0.2,
                color,
                size: 0.06 + (i % 4) as f64 * 0.02,
            });
        }
    }

    fn motion(&self) -> Motion {
        let p = &self.player;
        let head = p.pos + DVec2::new(0.0, 1.45);
        let look_at = |t: DVec2| DVec3::new(t.x - head.x, t.y - head.y, -1.2);
        // The front eye watches the nearest fly; the back eye does its own thing.
        let nearest = self
            .flies
            .iter()
            .zip(&self.caught)
            .filter(|(_, c)| !**c)
            .map(|(f, _)| *f)
            .min_by(|a, b| a.distance(head).total_cmp(&b.distance(head)));
        let front = match (&p.tongue, nearest) {
            (Some(t), _) => look_at(t.tip),
            (None, Some(f)) if f.distance(head) < 8.0 => look_at(f),
            _ => DVec3::new(p.facing * 0.5, 0.1, -1.0),
        };
        let back = DVec3::new((self.time * 0.7).sin() * 0.8, (self.time * 1.3).cos() * 0.5, -1.0);
        let (left, right) = if p.facing > 0.0 { (back, front) } else { (front, back) };
        Motion {
            time: self.time,
            stride: p.stride,
            run: (p.vel.x.abs() / 7.0).min(1.0),
            vel: DVec3::new(p.vel.x, p.vel.y, 0.0),
            heading: camel_joe::heading(self.turn, self.away),
            airborne: !p.on_ground,
            climbing: matches!(p.state, State::Climbing(_)),
            swinging: matches!(p.state, State::Swinging { .. }),
            gaze: [left, right],
        }
    }

    /// The hook the tongue would grab, for highlighting.
    fn hook_hint(&self) -> Option<usize> {
        if let Some(t) = &self.player.tongue {
            if let TongueTarget::Hook(i) = t.target {
                return Some(i);
            }
        }
        let mouth = self.player.mouth();
        self.level.hooks.iter().position(|h| {
            h.y > mouth.y + 0.5 && h.distance(mouth) < 7.5 && (h.x - mouth.x) * self.player.facing > -1.0
        })
    }

    /// Moves Joe to a checkpoint, for the snapshot tool.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn warp(&mut self, checkpoint: usize) {
        self.checkpoint = checkpoint.min(self.level.checkpoints.len() - 1);
        let at = self.level.checkpoints[self.checkpoint];
        self.player.respawn(at);
        self.camera = at + DVec2::new(2.0, 1.7);
    }

    /// A one-line summary of the player's state, for the snapshot tool.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn status(&self) -> String {
        let p = &self.player;
        let state = match p.state {
            State::Normal if p.on_ground => "ground".to_string(),
            State::Normal => "air".to_string(),
            State::Climbing(i) => format!("climbing {i}"),
            State::Swinging { hook, length } => format!("swinging {hook} len {length:.1}"),
        };
        let caught = self.caught.iter().filter(|c| **c).count();
        format!("pos ({:.2}, {:.2}) vel ({:.1}, {:.1}) {state} flies {caught} checkpoint {}", p.pos.x, p.pos.y, p.vel.x, p.vel.y, self.checkpoint)
    }

    pub fn draw(&self, scene: &mut Scene, w: f64, h: f64) {
        let camera = Camera {
            eye: DVec3::new(self.camera.x, self.camera.y, -CAMERA_DISTANCE),
            focal: h / self.view_height * CAMERA_DISTANCE,
            center: Point::new(w / 2.0, h / 2.0),
        };
        crate::paint::set_zoom(h / self.view_height / 85.0);
        jungle::draw_background(scene, &camera, w, h, self.time);

        let mut canvas = Canvas3d::new(camera);
        jungle::draw_world(
            &mut canvas,
            &self.level,
            &WorldView {
                time: self.time,
                flies: &self.flies,
                caught: &self.caught,
                checkpoint: self.checkpoint,
                hook_hint: self.hook_hint(),
                screen_width: w,
            },
        );
        self.draw_joe(&mut canvas);
        for p in &self.particles {
            let pr = camera.project(p.pos);
            let r = p.size * pr.scale * p.life.min(1.0);
            let color = p.color;
            canvas.push(pr.depth, move |scene| {
                scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &Circle::new(pr.pos, r));
            });
        }
        canvas.finish(scene);

        self.draw_hud(scene, w, h);
        crate::paint::paper(scene, w, h);
        if self.flash > 0.0 {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Color::BLACK.with_alpha(self.flash as f32),
                None,
                &Rect::new(0.0, 0.0, w, h),
            );
        }
    }

    fn draw_joe(&self, canvas: &mut Canvas3d) {
        let motion = self.motion();
        let pose = self.animator.pose(&motion);
        let p = &self.player;
        let mut rot = motion.heading;
        let mut feet = DVec3::new(p.pos.x, p.pos.y, 0.0);
        if let State::Swinging { hook, .. } = p.state {
            // Hang from the mouth, tilted towards the hook.
            let mouth = p.mouth();
            let d = self.level.hooks[hook] - mouth;
            let tilt = DQuat::from_rotation_z(-d.x.atan2(d.y));
            rot = tilt * rot;
            feet = DVec3::new(mouth.x, mouth.y, 0.0) - tilt * DVec3::new(0.0, MOUTH_HEIGHT, 0.0);
        }
        let root = camel_joe::root(feet, rot, self.squash);
        let solved = Solved::solve(&self.skeleton, &pose, root);
        let look = Look {
            time: self.time,
            camo: self.camo,
            tongue_out: p.tongue.is_some(),
        };
        let anchors = camel_joe::draw(canvas, &solved, &look);

        if let Some(t) = &p.tongue {
            let cam = canvas.camera;
            let a = cam.project(anchors.mouth);
            let b = cam.project(DVec3::new(t.tip.x, t.tip.y, 0.0));
            let mid = a.pos.midpoint(b.pos) + Vec2::new(0.0, (a.pos - b.pos).hypot() * 0.08);
            let width = 0.085 * a.scale;
            let tip_r = 0.11 * b.scale;
            canvas.push(a.depth - 0.05, move |scene| {
                let mut path = BezPath::new();
                path.move_to(a.pos);
                path.quad_to(mid, b.pos);
                let round = |w: f64| Stroke::new(w).with_caps(vello::kurbo::Cap::Round);
                scene.stroke(&round(width + 3.0), Affine::IDENTITY, OUTLINE, None, &path);
                scene.stroke(&round(width), Affine::IDENTITY, TONGUE, None, &path);
                let tip = Circle::new(b.pos, tip_r);
                scene.fill(Fill::NonZero, Affine::IDENTITY, TONGUE, None, &tip);
                scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, OUTLINE, None, &tip);
            });
        }
    }

    fn draw_hud(&self, scene: &mut Scene, w: f64, h: f64) {
        // One fly icon per fly: filled when caught.
        let r = (h * 0.014).max(4.0);
        for (i, &caught) in self.caught.iter().enumerate() {
            let c = Point::new(r * 2.0 + i as f64 * r * 2.6, r * 2.0);
            let dot = Circle::new(c, r);
            if caught {
                scene.fill(Fill::NonZero, Affine::IDENTITY, Color::from_rgb8(0xff, 0xd8, 0x50), None, &dot);
            } else {
                scene.fill(Fill::NonZero, Affine::IDENTITY, Color::BLACK.with_alpha(0.25), None, &dot);
            }
            scene.stroke(&Stroke::new(1.5), Affine::IDENTITY, OUTLINE, None, &dot);
        }
        if let Some(won) = self.won_at {
            // A big celebratory star.
            let t = ((self.time - won) * 2.0).min(1.0);
            let c = Point::new(w / 2.0, h * 0.25);
            let size = h * 0.18 * t;
            let mut star = BezPath::new();
            for k in 0..10 {
                let a = -std::f64::consts::FRAC_PI_2 + k as f64 * std::f64::consts::PI / 5.0 + self.time;
                let rr = if k % 2 == 0 { size } else { size * 0.45 };
                let p = c + Vec2::new(a.cos(), a.sin()) * rr;
                if k == 0 { star.move_to(p) } else { star.line_to(p) }
            }
            star.close_path();
            scene.fill(Fill::NonZero, Affine::IDENTITY, Color::from_rgb8(0xff, 0xd0, 0x40), None, &star);
            scene.stroke(&Stroke::new(4.0), Affine::IDENTITY, OUTLINE, None, &star);
        }
    }
}
