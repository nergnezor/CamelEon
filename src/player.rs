//! Joe's movement: running, jumping (with coyote time and jump buffering),
//! climbing vines and trunks, and the sticky tongue that catches flies and
//! grapples onto flowers to swing.

use glam::DVec2;

use crate::level::Level;

const HALF_WIDTH: f64 = 0.35;
const HEIGHT: f64 = 1.8;
/// Height above the feet where the grappling line attaches (his raised hand).
pub const MOUTH_HEIGHT: f64 = 1.85;

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
/// he leaps when it's let go. The deeper the crouch, the higher the jump:
/// from `JUMP_MIN` to `JUMP_MAX` times `JUMP_SPEED`.
const CHARGE_TIME: f64 = 0.4;
const JUMP_MIN: f64 = 0.8;
const JUMP_MAX: f64 = 1.35;
const MAX_FALL: f64 = 22.0;
const COYOTE_TIME: f64 = 0.1;
const JUMP_BUFFER: f64 = 0.12;
const CLIMB_SPEED: f64 = 4.5;
const TONGUE_SPEED: f64 = 75.0;
const TONGUE_RANGE: f64 = 7.5;
const FLY_RANGE: f64 = 5.0;

#[derive(Default, Clone, Copy)]
pub struct Controls {
    /// Analog horizontal input, −1 (left) to 1 (right).
    pub x: f64,
    /// Analog vertical input, −1 (down) to 1 (up).
    pub y: f64,
    pub up: bool,
    pub down: bool,
    pub jump: bool,
    pub jump_pressed: bool,
    pub tongue_pressed: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum State {
    Normal,
    Climbing(usize),
    /// Hanging from a hook by the tongue.
    Swinging { hook: usize, length: f64 },
}

#[derive(Clone, Copy, PartialEq)]
pub enum TongueTarget {
    Hook(usize),
    Fly(usize),
    /// Missed: stretch to a point and come back.
    Point(DVec2),
}

#[derive(Clone, Copy)]
pub struct Tongue {
    pub tip: DVec2,
    pub target: TongueTarget,
    pub retracting: bool,
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
    pub state: State,
    pub on_ground: bool,
    pub tongue: Option<Tongue>,
    coyote: f64,
    jump_buffer: f64,
    /// Seconds jump has been held while crouching to jump, if he is.
    charge: Option<f64>,
    /// Seconds spent standing still; drives camouflage.
    pub idle: f64,
    /// Advances with distance moved; drives the walk and climb cycles.
    pub stride: f64,
    /// 0..1: builds up while running flat out, raising top speed and
    /// acceleration, so Joe goes faster and faster.
    pub boost: f64,
}

impl Player {
    pub fn new(pos: DVec2) -> Self {
        Self {
            pos,
            vel: DVec2::ZERO,
            facing: 1.0,
            state: State::Normal,
            on_ground: false,
            tongue: None,
            coyote: 0.0,
            jump_buffer: 0.0,
            charge: None,
            idle: 0.0,
            stride: 0.0,
            boost: 0.0,
        }
    }

    /// How deep he's crouching to jump, 0..1 (0 when not crouching): a quick
    /// dip at once, then deeper with the charge.
    pub fn crouch(&self) -> f64 {
        self.charge.map_or(0.0, |t| 0.3 + 0.7 * (t / CHARGE_TIME).min(1.0))
    }

    /// Leaps with the power of a crouch held for `charge` seconds.
    fn leap(&mut self, charge: f64, events: &mut Events) {
        let power = (charge / CHARGE_TIME).min(1.0);
        self.vel.y = JUMP_SPEED * (JUMP_MIN + (JUMP_MAX - JUMP_MIN) * power);
        self.charge = None;
        self.jump_buffer = 0.0;
        self.coyote = 0.0;
        events.jumped = true;
    }

    pub fn mouth(&self) -> DVec2 {
        self.pos + DVec2::new(0.0, MOUTH_HEIGHT)
    }

    pub fn respawn(&mut self, pos: DVec2) {
        let facing = self.facing;
        *self = Player::new(pos);
        self.facing = facing;
    }

    pub fn update(&mut self, dt: f64, c: &Controls, level: &Level, flies: &[DVec2], caught: &[bool]) -> Events {
        let mut events = Events::default();
        let dir = c.x.clamp(-1.0, 1.0);
        if dir.abs() > 0.1 && !matches!(self.state, State::Climbing(_)) {
            self.facing = dir.signum();
        }
        self.jump_buffer = if c.jump_pressed { JUMP_BUFFER } else { (self.jump_buffer - dt).max(0.0) };

        if c.tongue_pressed {
            self.use_tongue(level, flies, caught);
        }
        self.update_tongue(dt, level, flies, caught, &mut events);

        match self.state {
            State::Normal => self.update_normal(dt, dir, c, level, &mut events),
            State::Climbing(i) => self.update_climbing(dt, dir, c, level, i, &mut events),
            State::Swinging { hook, length } => self.update_swinging(dt, dir, c, level, hook, length, &mut events),
        }

        let moving = self.vel.length() > 0.2 || !self.on_ground;
        self.idle = if moving || self.tongue.is_some() { 0.0 } else { self.idle + dt };
        if self.pos.y < level.kill_y {
            events.died = true;
        }
        events
    }

    fn update_normal(&mut self, dt: f64, dir: f64, c: &Controls, level: &Level, events: &mut Events) {
        // Grab a climbable when pressing up or down on it.
        if c.up || (c.down && !self.on_ground) {
            if let Some(i) = self.climbable_at(level) {
                self.state = State::Climbing(i);
                self.vel = DVec2::ZERO;
                self.charge = None;
                return;
            }
        }

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
        self.vel.x = approach(self.vel.x, dir * top, accel * dt);
        self.coyote = if self.on_ground { COYOTE_TIME } else { (self.coyote - dt).max(0.0) };

        if let Some(held) = self.charge {
            // Crouching to jump: leap on release. Off an edge he can still
            // leap within the coyote time, after that the crouch is lost.
            let held = held + dt;
            if !self.on_ground && self.coyote <= 0.0 {
                self.charge = None;
            } else if !c.jump {
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
        self.vel.y = (self.vel.y - GRAVITY * dt).max(-MAX_FALL);
        self.stride += self.vel.x.abs().min(16.0) * dt * 1.6 + self.vel.x.abs() * dt * 0.3;
        self.move_and_collide(dt, level, events);
    }

    fn update_climbing(&mut self, dt: f64, dir: f64, c: &Controls, level: &Level, i: usize, events: &mut Events) {
        let climbable = level.climbables[i];
        let vy = c.y.clamp(-1.0, 1.0) * CLIMB_SPEED;
        self.vel = DVec2::new(0.0, vy);
        self.pos.x = approach(self.pos.x, climbable.x, 8.0 * dt);
        self.pos.y += vy * dt;
        self.stride += vy.abs() * dt * 2.2;

        if self.jump_buffer > 0.0 {
            self.jump_buffer = 0.0;
            self.state = State::Normal;
            if dir.abs() > 0.1 {
                self.facing = dir.signum();
            }
            self.vel = DVec2::new(dir * RUN_SPEED * 0.8, JUMP_SPEED * 0.8);
            events.jumped = true;
            return;
        }
        if self.pos.y >= climbable.y1 {
            self.state = State::Normal;
            // Step onto a ledge next to the top if there is one (preferring the
            // side Joe is pushing towards); otherwise pop off the top.
            let ledge = level
                .blocks
                .iter()
                .filter(|b| (b.y1 - climbable.y1).abs() < 0.8)
                .filter(|b| b.x0 - 1.2 < climbable.x && climbable.x < b.x1 + 1.2)
                .min_by(|a, b| {
                    let side = |b: &&crate::level::Block| ((b.x0 + b.x1) / 2.0 - climbable.x) * dir < 0.0;
                    side(a).cmp(&side(b))
                });
            match ledge {
                Some(b) => {
                    self.pos.x = self.pos.x.clamp(b.x0 + HALF_WIDTH, b.x1 - HALF_WIDTH);
                    self.pos.y = b.y1;
                    self.vel = DVec2::ZERO;
                    self.on_ground = true;
                    return;
                }
                None => {
                    self.pos.y = climbable.y1;
                    self.vel.y = 4.0;
                }
            }
        } else if self.pos.y <= climbable.y0 {
            self.pos.y = climbable.y0;
            self.state = State::Normal;
        }
        self.on_ground = false;
    }

    #[allow(clippy::too_many_arguments)]
    fn update_swinging(
        &mut self,
        dt: f64,
        dir: f64,
        c: &Controls,
        level: &Level,
        hook: usize,
        mut length: f64,
        events: &mut Events,
    ) {
        let anchor = level.hooks[hook];
        if self.jump_buffer > 0.0 {
            // Let go with a boost.
            self.jump_buffer = 0.0;
            self.state = State::Normal;
            self.vel = self.vel * 1.1 + DVec2::new(0.0, 6.0);
            self.tongue.as_mut().map(|t| t.retracting = true);
            events.jumped = true;
            return;
        }
        length = (length - c.y.clamp(-1.0, 1.0) * 3.0 * dt).clamp(1.5, TONGUE_RANGE);
        self.state = State::Swinging { hook, length };

        // Pendulum: integrate freely, then pull back onto the rope's circle and
        // remove the outward velocity.
        self.vel.y -= GRAVITY * dt;
        self.vel.x += dir * 10.0 * dt;
        let mut mouth = self.mouth() + self.vel * dt;
        let d = mouth - anchor;
        if d.length() > length {
            let n = d.normalize();
            mouth = anchor + n * length;
            let outward = self.vel.dot(n);
            if outward > 0.0 {
                self.vel -= n * outward;
            }
        }
        let target = mouth - DVec2::new(0.0, MOUTH_HEIGHT);
        let step = target - self.pos;
        // Resolve collisions along the way by moving with the equivalent velocity.
        let saved = self.vel;
        self.vel = step / dt;
        self.move_and_collide(dt, level, events);
        self.vel = if self.on_ground { DVec2::ZERO } else { saved };
        if self.on_ground {
            self.state = State::Normal;
            self.tongue.as_mut().map(|t| t.retracting = true);
        }
    }

    fn use_tongue(&mut self, level: &Level, flies: &[DVec2], caught: &[bool]) {
        if let State::Swinging { .. } = self.state {
            // Pressing again lets go.
            self.state = State::Normal;
            self.tongue.as_mut().map(|t| t.retracting = true);
            return;
        }
        if self.tongue.is_some() || matches!(self.state, State::Climbing(_)) {
            return;
        }
        let mouth = self.mouth();
        let ahead = |p: DVec2| (p.x - mouth.x) * self.facing > -1.0;
        let hook = level
            .hooks
            .iter()
            .enumerate()
            .filter(|&(_, &h)| h.y > mouth.y + 0.5 && ahead(h) && h.distance(mouth) < TONGUE_RANGE)
            .min_by(|a, b| a.1.distance(mouth).total_cmp(&b.1.distance(mouth)));
        let fly = flies
            .iter()
            .enumerate()
            .filter(|&(i, &f)| !caught[i] && ahead(f) && f.distance(mouth) < FLY_RANGE)
            .min_by(|a, b| a.1.distance(mouth).total_cmp(&b.1.distance(mouth)));
        // In the air a hook wins (it's usually a rescue); on the ground, snacks first.
        let target = match (fly, hook) {
            (_, Some((i, _))) if !self.on_ground => TongueTarget::Hook(i),
            (Some((i, _)), _) => TongueTarget::Fly(i),
            (None, Some((i, _))) => TongueTarget::Hook(i),
            (None, None) => TongueTarget::Point(mouth + DVec2::new(self.facing * 3.5, 1.2)),
        };
        self.tongue = Some(Tongue { tip: mouth, target, retracting: false });
    }

    fn update_tongue(&mut self, dt: f64, level: &Level, flies: &[DVec2], caught: &[bool], events: &mut Events) {
        let mouth = self.mouth();
        let Some(tongue) = &mut self.tongue else { return };
        if let State::Swinging { hook, .. } = self.state {
            tongue.tip = level.hooks[hook];
            return;
        }
        let goal = if tongue.retracting {
            mouth
        } else {
            match tongue.target {
                TongueTarget::Hook(i) => level.hooks[i],
                TongueTarget::Fly(i) => flies[i],
                TongueTarget::Point(p) => p,
            }
        };
        let d = goal - tongue.tip;
        let step = TONGUE_SPEED * dt;
        if d.length() > step {
            tongue.tip += d.normalize() * step;
            // A fly that got caught by something else, or out of range.
            if let TongueTarget::Fly(i) = tongue.target {
                if caught[i] {
                    tongue.retracting = true;
                }
            }
            if tongue.tip.distance(mouth) > TONGUE_RANGE * 1.2 {
                tongue.retracting = true;
            }
            return;
        }
        tongue.tip = goal;
        if tongue.retracting {
            self.tongue = None;
            return;
        }
        match tongue.target {
            TongueTarget::Hook(i) => {
                let length = mouth.distance(level.hooks[i]).max(1.5);
                self.state = State::Swinging { hook: i, length };
                self.on_ground = false;
            }
            TongueTarget::Fly(i) => {
                events.caught_fly = Some(i);
                tongue.retracting = true;
            }
            TongueTarget::Point(_) => tongue.retracting = true,
        }
    }

    fn climbable_at(&self, level: &Level) -> Option<usize> {
        level.climbables.iter().position(|c| {
            (self.pos.x - c.x).abs() < c.half_width + HALF_WIDTH
                && self.pos.y < c.y1 - 0.1
                && self.pos.y + HEIGHT > c.y0
        })
    }

    fn move_and_collide(&mut self, dt: f64, level: &Level, events: &mut Events) {
        let was_on_ground = self.on_ground;
        let fall_speed = -self.vel.y;

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
        if self.on_ground && !was_on_ground {
            events.landed = Some(fall_speed);
        }
    }

    fn overlaps(&self, x0: f64, x1: f64, y0: f64, y1: f64) -> bool {
        self.pos.x + HALF_WIDTH > x0 && self.pos.x - HALF_WIDTH < x1 && self.pos.y + HEIGHT > y0 && self.pos.y < y1
    }
}

fn approach(value: f64, target: f64, step: f64) -> f64 {
    if value < target {
        (value + step).min(target)
    } else {
        (value - step).max(target)
    }
}
