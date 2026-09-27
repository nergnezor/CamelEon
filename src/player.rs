//! Konrad's movement: running with a momentum boost, and jumping (charged,
//! with coyote time and jump buffering). He catches flies by running or
//! jumping into them.

use glam::DVec2;

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

        let body = self.pos + DVec2::new(0.0, HEIGHT / 2.0);
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
        self.vel.x = approach(self.vel.x, dir * top, accel * dt);
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
        self.vel.y = (self.vel.y - GRAVITY * dt).max(-MAX_FALL);
        self.stride += self.vel.x.abs().min(16.0) * dt * 1.6 + self.vel.x.abs() * dt * 0.3;
        self.move_and_collide(dt, level, events);
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
