//! Konrad's movement: running with a momentum boost, and jumping (charged,
//! with coyote time and jump buffering). He catches flies by running or
//! jumping into them.
//!
//! On rolling hills he runs along the slope: gravity pulls him down it, speed
//! gained downhill is kept, and over a crest he takes off when he's too fast
//! for the ground to curve away under him. In a loop he runs round the
//! inside, and drops off if he's too slow at the top.

use glam::DVec2;

use std::f64::consts::{FRAC_PI_2, TAU};

use crate::level::Level;

const HALF_WIDTH: f64 = 0.35;
const HEIGHT: f64 = 1.8;

const RUN_SPEED: f64 = 14.0;
/// Top speed multiplier gained by running flat out for a while.
const MAX_BOOST: f64 = 1.45;
/// How fast the boost builds while running flat out (per second).
const BOOST_RATE: f64 = 0.28;
const GROUND_ACCEL: f64 = 120.0;
const AIR_ACCEL: f64 = 56.0;
const GRAVITY: f64 = 32.0;
const JUMP_SPEED: f64 = 13.5;
/// Holding jump on the ground crouches deeper for up to this long (seconds);
/// he leaps when it's let go, or by himself once fully charged. The deeper
/// the crouch, the higher the jump: from `JUMP_MIN` to `JUMP_MAX` times
/// `JUMP_SPEED`.
const CHARGE_TIME: f64 = 0.2;
const JUMP_MIN: f64 = 0.8;
const JUMP_MAX: f64 = 1.35;
const MAX_FALL: f64 = 22.0;
const COYOTE_TIME: f64 = 0.1;
const JUMP_BUFFER: f64 = 0.12;
/// How close a fly must come to the middle of his body to be caught.
const CATCH_RANGE: f64 = 1.0;
/// Hard speed limit, however long the downhill.
const MAX_SPEED: f64 = 58.0;
/// Speed above the running top speed fades this fast (per second) while he
/// keeps running, so a downhill's speed lasts.
const MOMENTUM_DRAG: f64 = 3.0;
/// Pushing forward in a loop: much weaker than on flat ground, so it takes
/// a run-up to get round.
const LOOP_ACCEL: f64 = 5.0;
/// How much harder than gravity alone the ground holds him over a crest
/// (his grip, as it were): only sharp crests throw him into the air.
const GROUND_GRIP: f64 = 2.5;
/// How far the ground may rise under him in one step and still be walked
/// onto, rather than being a wall.
const STEP_UP: f64 = 0.5;

/// Running round a loop (`Level::loops[index]`).
#[derive(Clone, Copy)]
struct LoopRun {
    index: usize,
    /// Where he is on the loop: the angle from its centre (−π/2 at the bottom).
    angle: f64,
    /// Speed along the loop, and which way round he goes (1 anticlockwise,
    /// entered running right; −1 clockwise).
    speed: f64,
    dir: f64,
}

#[derive(Default, Clone, Copy)]
pub struct Controls {
    /// Analog horizontal input, −1 (left) to 1 (right).
    pub x: f64,
    pub jump: bool,
    pub jump_pressed: bool,
}

/// Things that happened during an update, for effects and scoring.
#[derive(Default)]
pub struct Events {
    pub landed: Option<f64>,
    pub jumped: bool,
    pub caught_fly: Option<usize>,
    pub died: bool,
}

pub struct Player {
    /// Position of the feet.
    pub pos: DVec2,
    pub vel: DVec2,
    pub facing: f64,
    pub on_ground: bool,
    coyote: f64,
    jump_buffer: f64,
    /// Seconds jump has been held while crouching to jump, if he is.
    charge: Option<f64>,
    /// Seconds spent standing still; drives camouflage.
    pub idle: f64,
    /// Advances with distance moved; drives the run cycle.
    pub stride: f64,
    /// 0..1: builds up while running flat out, raising top speed and
    /// acceleration, so Joe goes faster and faster.
    pub boost: f64,
    /// Which hill he's running on, if any.
    hill: Option<usize>,
    looping: Option<LoopRun>,
    /// How far he's rotated from upright (radians, anticlockwise): the
    /// ground's angle under him, all the way round in a loop.
    pub tilt: f64,
}

impl Player {
    pub fn new(pos: DVec2) -> Self {
        Self {
            pos,
            vel: DVec2::ZERO,
            facing: 1.0,
            on_ground: false,
            coyote: 0.0,
            jump_buffer: 0.0,
            charge: None,
            idle: 0.0,
            stride: 0.0,
            boost: 0.0,
            hill: None,
            looping: None,
            tilt: 0.0,
        }
    }

    /// How deep he's crouching to jump, 0..1 (0 when not crouching): a quick
    /// dip at once, then deeper with the charge.
    pub fn crouch(&self) -> f64 {
        self.charge.map_or(0.0, |t| 0.3 + 0.7 * (t / CHARGE_TIME).min(1.0))
    }

    /// Which way is up for him: tilted with the ground.
    pub fn up(&self) -> DVec2 {
        DVec2::new(-self.tilt.sin(), self.tilt.cos())
    }

    /// Whether he's running round a loop.
    pub fn in_loop(&self) -> bool {
        self.looping.is_some()
    }

    /// Leaps with the power of a crouch held for `charge` seconds.
    fn leap(&mut self, charge: f64, events: &mut Events) {
        let power = (charge / CHARGE_TIME).min(1.0);
        let speed = JUMP_SPEED * (JUMP_MIN + (JUMP_MAX - JUMP_MIN) * power);
        if self.looping.take().is_some() {
            // Off the loop's track, away from it.
            self.vel += self.up() * speed * 0.8;
            self.on_ground = false;
        } else {
            // Running up a slope carries him higher.
            self.vel.y = speed + self.vel.y.max(0.0) * 0.5;
        }
        self.hill = None;
        self.charge = None;
        self.jump_buffer = 0.0;
        self.coyote = 0.0;
        events.jumped = true;
    }

    pub fn respawn(&mut self, pos: DVec2) {
        let facing = self.facing;
        *self = Player::new(pos);
        self.facing = facing;
    }

    pub fn update(&mut self, dt: f64, c: &Controls, level: &Level, flies: &[DVec2], caught: &[bool]) -> Events {
        let mut events = Events::default();
        let dir = c.x.clamp(-1.0, 1.0);
        if dir.abs() > 0.1 {
            self.facing = dir.signum();
        }
        self.jump_buffer = if c.jump_pressed { JUMP_BUFFER } else { (self.jump_buffer - dt).max(0.0) };
        self.update_normal(dt, dir, c, level, &mut events);

        let body = self.pos + self.up() * (HEIGHT / 2.0);
        events.caught_fly = (0..flies.len()).find(|&i| !caught[i] && flies[i].distance(body) < CATCH_RANGE);

        let moving = self.vel.length() > 0.2 || !self.on_ground;
        self.idle = if moving { 0.0 } else { self.idle + dt };
        if self.pos.y < level.kill_y {
            events.died = true;
        }
        events
    }

    fn update_normal(&mut self, dt: f64, dir: f64, c: &Controls, level: &Level, events: &mut Events) {
        // Momentum: keep pushing the same way at speed and the boost builds;
        // stopping, turning or hitting a wall loses it (only on the ground,
        // so a jump keeps the speed).
        let flat_out = dir.abs() > 0.6 && dir * self.vel.x > RUN_SPEED * 0.85;
        if flat_out && self.on_ground {
            self.boost = (self.boost + BOOST_RATE * dt * (1.0 + self.boost)).min(1.0);
        } else if self.on_ground && !flat_out {
            self.boost = (self.boost - 1.5 * dt).max(0.0);
        }
        let top = RUN_SPEED * (1.0 + MAX_BOOST * self.boost);
        let accel = if self.on_ground { GROUND_ACCEL } else { AIR_ACCEL } * (1.0 + self.boost);
        if let Some(i) = self.hill.filter(|_| self.on_ground) {
            // Along the slope: gravity pulls him down it, and running uphill
            // is slower.
            let tangent = DVec2::new(1.0, level.hills[i].slope(self.pos.x)).normalize();
            let uphill = (dir * tangent.y).max(0.0);
            let mut speed = self.vel.dot(tangent) - GRAVITY * tangent.y * dt;
            speed = drive(speed, dir, top * (1.0 - 0.35 * uphill), accel * dt, dt);
            self.vel = tangent * speed;
        } else if self.looping.is_none() {
            self.vel.x = drive(self.vel.x, dir, top, accel * dt, dt);
        }
        self.coyote = if self.on_ground { COYOTE_TIME } else { (self.coyote - dt).max(0.0) };

        if let Some(held) = self.charge {
            // Crouching to jump: leap on release or at full charge. Off an
            // edge he can still leap within the coyote time, after that the
            // crouch is lost.
            let held = held + dt;
            if !self.on_ground && self.coyote <= 0.0 {
                self.charge = None;
            } else if !c.jump || held >= CHARGE_TIME {
                self.leap(held, events);
            } else {
                self.charge = Some(held);
            }
        } else if self.jump_buffer > 0.0 && self.coyote > 0.0 {
            if self.on_ground && c.jump {
                self.charge = Some(0.0);
                self.jump_buffer = 0.0;
            } else {
                // Already let go, or just off an edge: a quick hop.
                self.leap(0.0, events);
            }
        }
        if let Some(run) = self.looping {
            self.run_loop(run, dt, dir, level);
        } else {
            self.vel.y = (self.vel.y - GRAVITY * dt).max(-MAX_FALL);
            self.vel = self.vel.clamp_length_max(MAX_SPEED);
            self.move_and_collide(dt, level, events);
        }
        let speed = if self.on_ground { self.vel.length() } else { self.vel.x.abs() };
        self.stride += speed.min(16.0) * dt * 1.6 + speed * dt * 0.3;
        // Lean with the ground (only partly on hills, so he still looks like
        // he's running upright), and settle back upright in the air.
        let target = match (self.looping, self.hill.filter(|_| self.on_ground)) {
            (Some(run), _) => run.angle + FRAC_PI_2,
            (None, Some(i)) => level.hills[i].slope(self.pos.x).atan() * 0.5,
            _ => 0.0,
        };
        if self.looping.is_some() {
            self.tilt = target;
        } else {
            // Unwind the long way round a loop left behind.
            self.tilt = (self.tilt + std::f64::consts::PI).rem_euclid(TAU) - std::f64::consts::PI;
            self.tilt += (target - self.tilt) * (1.0 - (-dt * 12.0).exp());
        }
    }

    /// One step round a loop: gravity slows him on the way up and speeds
    /// him up on the way down; pushing on helps a little. He leaves it at
    /// the bottom, or drops off when too slow to stay on at the top.
    fn run_loop(&mut self, mut run: LoopRun, dt: f64, dir: f64, level: &Level) {
        let l = level.loops[run.index];
        let (sin, cos) = run.angle.sin_cos();
        run.speed -= run.dir * GRAVITY * cos * dt;
        let push = dir * run.dir;
        let top = RUN_SPEED * (1.0 + MAX_BOOST * self.boost);
        if push > 0.1 && run.speed < top {
            run.speed = (run.speed + LOOP_ACCEL * push * dt).min(top);
        } else if push < -0.1 {
            run.speed -= LOOP_ACCEL * dt;
        }
        run.angle += run.dir * run.speed / l.radius * dt;
        let progress = run.dir * (run.angle + FRAC_PI_2);
        let bottom = DVec2::new(l.center.x, l.center.y - l.radius);
        let tangent = DVec2::new(-run.angle.sin(), run.angle.cos()) * run.dir;
        if progress >= TAU || progress <= 0.0 {
            // Back on the ground at the bottom, going on or back.
            let way = if progress >= TAU { run.dir } else { -run.dir };
            self.pos = bottom + DVec2::new(way * 0.01, 0.0);
            self.vel = DVec2::new(way * run.speed.abs(), 0.0);
            self.looping = None;
            self.hill = level.hill_below(self.pos.x, self.pos.y + 0.1).map(|(i, _)| i);
            self.on_ground = true;
            return;
        }
        self.pos = l.center + DVec2::new(run.angle.cos(), run.angle.sin()) * l.radius;
        self.vel = tangent * run.speed;
        if run.speed * run.speed < GRAVITY * l.radius * sin.max(0.0) || run.speed < 0.0 && sin > 0.0 {
            // Too slow to be held to the track: he falls.
            self.looping = None;
            self.on_ground = false;
            return;
        }
        self.looping = Some(run);
    }

    fn move_and_collide(&mut self, dt: f64, level: &Level, events: &mut Events) {
        let was_on_ground = self.on_ground;
        let fall_speed = -self.vel.y;
        let prev = self.pos;

        self.pos.x += self.vel.x * dt;
        for b in level.blocks.iter().filter(|b| !b.one_way()) {
            if self.overlaps(b.x0, b.x1, b.y0, b.y1) {
                if self.vel.x > 0.0 {
                    self.pos.x = b.x0 - HALF_WIDTH;
                } else if self.vel.x < 0.0 {
                    self.pos.x = b.x1 + HALF_WIDTH;
                }
                self.vel.x = 0.0;
            }
        }
        // Running into a hill's cliff from the side.
        for h in &level.hills {
            let entering = !h.contains(prev.x) && h.contains(self.pos.x);
            if entering && h.height(self.pos.x) > prev.y + STEP_UP {
                self.pos.x = if prev.x < h.x0() { h.x0() - 1e-3 } else { h.x1() + 1e-3 };
                self.vel.x = 0.0;
            }
        }

        // Crossing the bottom of a loop on the ground: into the loop.
        if was_on_ground {
            for (index, l) in level.loops.iter().enumerate() {
                let crossed = (prev.x - l.center.x) * (self.pos.x - l.center.x) <= 0.0 && prev.x != l.center.x;
                let bottom = l.center.y - l.radius;
                if crossed && (self.pos.y - bottom).abs() < 0.6 && self.vel.x.abs() > 1.0 {
                    let dir = self.vel.x.signum();
                    let angle = -FRAC_PI_2 + dir * (self.pos.x - l.center.x).abs() / l.radius;
                    self.looping = Some(LoopRun { index, angle, speed: self.vel.length(), dir });
                    self.hill = None;
                    self.on_ground = true;
                    return;
                }
            }
        }

        // On a hill: follow the ground, unless it falls away faster than
        // gravity can pull him down (over a crest at speed).
        if let Some(i) = self.hill.filter(|_| was_on_ground) {
            let h = &level.hills[i];
            if h.contains(self.pos.x) {
                let lift = -h.bend(self.pos.x) * self.vel.x * self.vel.x;
                if lift > GRAVITY * GROUND_GRIP {
                    self.pos.y += self.vel.y * dt;
                    self.hill = None;
                    self.on_ground = false;
                    return;
                }
                self.pos.y = h.height(self.pos.x);
                let tangent = DVec2::new(1.0, h.slope(self.pos.x)).normalize();
                self.vel = tangent * self.vel.dot(tangent);
                self.on_ground = true;
                return;
            }
            self.hill = None;
        }

        let prev_y = self.pos.y;
        self.pos.y += self.vel.y * dt;
        self.on_ground = false;
        for b in &level.blocks {
            if !self.overlaps(b.x0, b.x1, b.y0, b.y1) {
                continue;
            }
            if self.vel.y <= 0.0 && prev_y >= b.y1 - 1e-6 {
                self.pos.y = b.y1;
                self.vel.y = 0.0;
                self.on_ground = true;
            } else if !b.one_way() && self.vel.y > 0.0 {
                self.pos.y = b.y0 - HEIGHT;
                self.vel.y = 0.0;
            }
        }
        // Landing on a hill keeps the speed along its slope.
        for (i, h) in level.hills.iter().enumerate() {
            if !h.contains(self.pos.x) || self.on_ground {
                continue;
            }
            let top = h.height(self.pos.x);
            if self.pos.y <= top && prev.y >= h.height(prev.x) - STEP_UP - self.vel.length() * dt {
                let tangent = DVec2::new(1.0, h.slope(self.pos.x)).normalize();
                let normal = DVec2::new(-tangent.y, tangent.x);
                if self.vel.dot(normal) > 0.0 {
                    continue;
                }
                self.pos.y = top;
                self.vel = tangent * self.vel.dot(tangent);
                self.hill = Some(i);
                self.on_ground = true;
            }
        }
        if self.on_ground && !was_on_ground {
            events.landed = Some(fall_speed);
        }
    }

    fn overlaps(&self, x0: f64, x1: f64, y0: f64, y1: f64) -> bool {
        self.pos.x + HALF_WIDTH > x0 && self.pos.x - HALF_WIDTH < x1 && self.pos.y + HEIGHT > y0 && self.pos.y < y1
    }
}

/// Running: towards `dir` times `top`, but speed beyond `top` in the
/// direction he's pushing is kept (slowly fading), so a downhill's speed
/// carries on.
fn drive(v: f64, dir: f64, top: f64, step: f64, dt: f64) -> f64 {
    if dir.abs() > 0.1 && v * dir > top {
        (v.abs() - MOMENTUM_DRAG * dt).max(top) * v.signum()
    } else {
        approach(v, dir * top, step)
    }
}

fn approach(value: f64, target: f64, step: f64) -> f64 {
    if value < target {
        (value + step).min(target)
    } else {
        (value - step).max(target)
    }
}
