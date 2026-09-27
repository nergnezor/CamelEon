//! Game state and the frame loop shared by the window and terminal frontends.

use glam::{DMat3, DVec2, DVec3};
use vello::kurbo::{Affine, BezPath, Circle, Point, Rect, Stroke, Vec2};
use vello::peniko::{Color, Fill, Gradient};
use vello::Scene;

use crate::audio::{Ambience, Sfx};
use crate::konrad::{self as hero, Animator, Look, Motion};
use crate::canvas3d::{Camera, Canvas3d, OUTLINE};
use crate::jungle::{self, WorldView};
use crate::dusk;
use crate::level::{self, Level, Theme};
use crate::player::{Controls, Player};
use crate::frame::{FrameInfo, Layers, Post};
use crate::hair::{HairFrame, HairStyle};
use crate::weather::{self, Weather};
use crate::rig::Skeleton;

/// Profiling switch: parts of the frame to leave out (`SKIP_*` bit flags).
/// Only set by the snapshot tool's GPU benchmark.
pub static SKIP: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
pub const SKIP_BACKGROUND: u8 = 1;
pub const SKIP_WORLD: u8 = 2;
pub const SKIP_JOE: u8 = 4;
pub const SKIP_GRADE: u8 = 8;

/// Distance from the camera to the gameplay plane.
const CAMERA_DISTANCE: f64 = 11.0;
/// World units visible vertically in the gameplay plane: standing still, and
/// at full running speed (the camera zooms out the faster Joe goes).
const VIEW_HEIGHT: f64 = 8.5;
const VIEW_HEIGHT_FAST: f64 = 16.0;
/// World units always visible across the screen (matters in portrait).
const MIN_VIEW_WIDTH: f64 = 10.0;
const MAX_STEP: f64 = 1.0 / 120.0;
/// Camouflage colours (ground, blotches): the jungle behind him, or bark
/// in front of a tree trunk.
const CAMO_LEAVES: (Color, Color) = (Color::from_rgb8(0x1c, 0x3e, 0x30), Color::from_rgb8(0x3a, 0x6a, 0x3e));
const CAMO_BARK: (Color, Color) = (Color::from_rgb8(0x4a, 0x36, 0x2a), Color::from_rgb8(0x6e, 0x50, 0x38));
/// Seconds standing still before the camouflage comes on, and how long it
/// takes to cover him and to drop when he moves.
const CAMO_DELAY: f64 = 0.8;
const CAMO_ON: f64 = 1.4;
const CAMO_OFF: f64 = 0.3;

/// Which controls are currently held down, plus analog stick axes.
#[derive(Default, Clone, Copy)]
pub struct Input {
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    /// Analog stick, −1..1 (right is positive); zero when no stick is used.
    pub stick_x: f64,
}

impl Input {
    /// Combines two input sources: a button is down if it's down in either,
    /// and the stick that's pushed further wins.
    pub fn merge(a: Input, b: Input) -> Input {
        let pick = |x: f64, y: f64| if x.abs() > y.abs() { x } else { y };
        Input {
            left: a.left || b.left,
            right: a.right || b.right,
            jump: a.jump || b.jump,
            stick_x: pick(a.stick_x, b.stick_x),
        }
    }

    /// The horizontal axis: the stick if it's pushed, else the keys.
    fn axis(&self) -> f64 {
        let keys = self.right as i32 as f64 - self.left as i32 as f64;
        if self.stick_x.abs() > keys.abs() { self.stick_x } else { keys }
    }
}

struct Particle {
    pos: DVec3,
    vel: DVec3,
    life: f64,
    color: Color,
    size: f64,
    /// A puff of dust: floats instead of falling, grows and fades.
    puff: bool,
}

pub struct Game {
    /// Keyboard state, set by the frontend.
    pub input: Input,
    /// Gamepad state, set by the frontend.
    pub pad: Input,
    prev_jump: bool,
    /// Which of `level::all()` is being played.
    level_index: usize,
    level: Level,
    player: Player,
    fly_homes: Vec<DVec2>,
    flies: Vec<DVec2>,
    caught: Vec<bool>,
    checkpoint: usize,
    skeleton: Skeleton,
    animator: Animator,
    /// Smoothed facing (−1..1), so he turns round through the camera.
    turn: f64,
    squash: f64,
    squash_vel: f64,
    camera: DVec2,
    view_height: f64,
    /// Breathing: the phase of the breath cycle, and how out of breath he is
    /// (0..1), which builds up running and fades while he rests.
    breath_phase: f64,
    exertion: f64,
    /// How far the view leads ahead of him, −1..1 as a share of the lead
    /// room; it grows with speed so there's more to see the faster he runs.
    lead: f64,
    pub weather: Weather,
    /// Hair simulation: the tips' swing (a damped spring driven by his
    /// head's acceleration) and the hair's lagging facing when he turns.
    hair_swing: DVec2,
    hair_swing_vel: DVec2,
    hair_facing: f64,
    hair_facing_vel: f64,
    prev_head_vel: DVec2,
    time: f64,
    won_at: Option<f64>,
    flash: f64,
    particles: Vec<Particle>,
    /// Sound effects since the frontend last collected them.
    pub sounds: Vec<Sfx>,
    /// Footfall counter, from the stride: a step sounds when it changes.
    footfall: i64,
    /// Camouflage, 0..1 (see `konrad::Look::camo`).
    camo: f64,
}

impl Game {
    pub fn new() -> Self {
        Self::with_level(0)
    }

    /// A fresh game on level `index` of `level::all()` (wrapping round).
    pub fn with_level(index: usize) -> Self {
        let levels = level::all();
        let level_index = index % levels.len();
        let level = levels[level_index]();
        let start = level.checkpoints[0];
        let fly_homes = level.flies.clone();
        Self {
            input: Input::default(),
            pad: Input::default(),
            prev_jump: false,
            level_index,
            player: Player::new(start),
            flies: fly_homes.clone(),
            caught: vec![false; fly_homes.len()],
            fly_homes,
            level,
            checkpoint: 0,
            skeleton: hero::skeleton(),
            animator: Animator::default(),
            turn: 1.0,
            squash: 0.0,
            squash_vel: 0.0,
            camera: start + DVec2::new(2.0, 1.7),
            view_height: VIEW_HEIGHT,
            lead: 0.0,
            breath_phase: 0.0,
            exertion: 0.0,
            weather: Weather::new(),
            hair_swing: DVec2::ZERO,
            hair_swing_vel: DVec2::ZERO,
            hair_facing: 1.0,
            hair_facing_vel: 0.0,
            prev_head_vel: DVec2::ZERO,
            time: 0.0,
            won_at: None,
            flash: 0.0,
            particles: Vec::new(),
            sounds: Vec::new(),
            footfall: 0,
            camo: 0.0,
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
        let (kx, px) = (k.axis(), p.axis());
        let jump = k.jump || p.jump;
        let mut controls = Controls {
            x: if kx.abs() > px.abs() { kx } else { px },
            jump,
            jump_pressed: jump && !self.prev_jump,
        };
        self.prev_jump = jump;

        for (i, home) in self.fly_homes.iter().enumerate() {
            self.flies[i] = jungle::fly_position(*home, self.time, i);
        }

        if let Some(won) = self.won_at {
            if self.time - won > 6.0 {
                self.next_level();
                return;
            }
            // Victory: ignore the player and jump for joy.
            let since = self.time - won;
            // Hold jump a moment each time, so he crouches and leaps.
            controls = Controls {
                jump: since % 0.9 < 0.2,
                jump_pressed: since % 0.9 < dt,
                ..Controls::default()
            };
        }

        let events = self.player.update(dt, &controls, &self.level, &self.flies, &self.caught);
        self.step_sounds();
        if let Some(i) = events.caught_fly {
            self.sounds.push(Sfx::Gulp);
            self.caught[i] = true;
            let p = self.flies[i];
            self.burst(DVec3::new(p.x, p.y, 0.0), Color::from_rgb8(0xff, 0xe0, 0x60), 14);
        }
        if let Some(speed) = events.landed {
            if speed > 2.0 {
                self.sounds.push(Sfx::Land { speed: speed as f32 });
            }
            if speed > 5.0 {
                self.squash_vel -= speed * 0.25;
                let feet = DVec3::new(self.player.pos.x, self.player.pos.y, 0.0);
                let (dust, bits) = match self.level.theme {
                    Theme::Jungle => (Color::from_rgb8(0x9a, 0x7a, 0x50), Color::from_rgb8(0x6a, 0x9a, 0x4a)),
                    Theme::Dusk => (Color::from_rgb8(0xae, 0x80, 0x78), Color::from_rgb8(0x5a, 0x44, 0x60)),
                };
                self.burst(feet, dust, (speed as usize / 3).min(10));
                self.burst(feet, bits, (speed as usize / 4).min(6));
            }
        }
        if events.jumped {
            self.squash_vel += 4.0;
            let power = ((self.player.vel.y - 11.0) / 7.0).clamp(0.0, 1.0);
            self.sounds.push(Sfx::Jump { power: power as f32 });
        }
        if events.died {
            let at = self.level.checkpoints[self.checkpoint];
            self.player.respawn(at);
            self.flash = 1.0;
            self.sounds.push(Sfx::Fall);
        }
        let reached = (self.checkpoint + 1..self.level.checkpoints.len()).rev().find(|&i| {
            self.player.on_ground && self.player.pos.x >= self.level.checkpoints[i].x - 0.5
        });
        if let Some(i) = reached {
            self.checkpoint = i;
            self.sounds.push(Sfx::Checkpoint);
            let cp = self.level.checkpoints[i];
            self.burst(DVec3::new(cp.x, cp.y + 2.0, 0.7), Color::from_rgb8(0xf2, 0x7a, 0x2e), 12);
        }
        let center = self.player.pos + DVec2::new(0.0, 0.8);
        if self.won_at.is_none() && center.distance(self.level.goal + DVec2::new(0.0, 2.2)) < 2.0 {
            self.won_at = Some(self.time);
            self.sounds.push(Sfx::Win);
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

        let motion = self.motion();
        self.animator.update(dt, &motion);
        self.update_hair(dt);
        let effort = (self.player.vel.x.abs() / 20.0).min(1.0);
        let rate = if effort > self.exertion { 0.4 } else { 0.08 };
        self.exertion += (effort - self.exertion) * (1.0 - (-dt * rate).exp());
        // One breath about every four seconds at rest, panting when winded.
        self.breath_phase += dt * (1.5 + 3.5 * self.exertion);
        self.weather.update(dt, self.time);
        if self.level.theme == Theme::Dusk {
            // A clear evening in the city.
            self.weather.rain = 0.0;
        }
        self.update_camo(dt);

        // Zoom out with speed (slowly, so it breathes rather than pumps).
        let speed = (self.player.vel.length() / 34.0).min(1.0);
        let target_view = VIEW_HEIGHT + (VIEW_HEIGHT_FAST - VIEW_HEIGHT) * speed;
        self.view_height += (target_view - self.view_height) * (1.0 - (-dt * 1.2).exp());

        // Camera follows him; the look-ahead (`lead`) is applied in `draw`,
        // where the width of the view is known. It eases in slowly so turning
        // round doesn't whip the view across.
        let target_lead = (self.player.vel.x / 30.0).clamp(-1.0, 1.0);
        self.lead += (target_lead - self.lead) * (1.0 - (-dt * 2.0).exp());
        let follow = DVec2::new(6.0, 3.0);
        // Aim ahead by the distance the smoothing lags behind at this speed,
        // so the camera keeps up with him instead of eating the look-ahead.
        let lag = 0.75 * self.player.vel.x / follow.x;
        let target = self.player.pos + DVec2::new(self.turn * 1.5 + lag, 1.7);
        let ck = DVec2::new(1.0 - (-dt * follow.x).exp(), 1.0 - (-dt * follow.y).exp());
        self.camera += (target - self.camera) * ck;
        self.camera.y = self.camera.y.max(self.level.kill_y + 6.0);

        self.update_effects(dt);
    }

    /// Footfalls: a sound, and dust kicked up at speed.
    fn step_sounds(&mut self) {
        let p = &self.player;
        // A foot lands twice per run cycle, just after each leg's forward
        // swing (see `konrad::run`).
        let footfall = ((p.stride - 2.0) / std::f64::consts::PI).floor() as i64;
        if footfall != self.footfall {
            self.footfall = footfall;
            if p.on_ground && p.vel.x.abs() > 1.0 {
                self.sounds.push(Sfx::Step { speed: (p.vel.x.abs() / 30.0).min(1.0) as f32 });
                if p.vel.x.abs() > 10.0 {
                    let (feet, vx) = (DVec3::new(p.pos.x, p.pos.y + 0.05, 0.1), p.vel.x);
                    self.kick_up(feet, vx);
                }
            }
        }
    }

    /// Starts the next level (after the last, the first again).
    pub fn next_level(&mut self) {
        let weather = self.weather.mode;
        *self = Game::with_level(self.level_index + 1);
        self.weather.mode = weather;
    }

    /// Standing still, his tracksuit slowly blends into the surroundings;
    /// moving drops the camouflage at once.
    fn update_camo(&mut self, dt: f64) {
        let p = &self.player;
        let hiding = p.on_ground && p.idle > CAMO_DELAY;
        if hiding && self.camo == 0.0 {
            self.sounds.push(Sfx::Camo);
        }
        self.camo = if hiding { (self.camo + dt / CAMO_ON).min(1.0) } else { (self.camo - dt / CAMO_OFF).max(0.0) };
    }

    /// What the camouflage blends into: bark in front of a tree trunk,
    /// otherwise the leaves.
    fn camo_colors(&self) -> (Color, Color) {
        if self.level.theme == Theme::Dusk {
            return dusk::CAMO;
        }
        let p = self.player.pos;
        let trunk = self.level.trees.iter().any(|t| (p.x - t.x).abs() < 1.0 && p.y < t.y);
        if trunk { CAMO_BARK } else { CAMO_LEAVES }
    }

    /// A footfall at speed kicks up a puff of dust, or splashes in the rain.
    fn kick_up(&mut self, feet: DVec3, vx: f64) {
        let wet = self.weather.rain;
        let dust = Color::from_rgb8(0x9a, 0x8c, 0x72);
        if wet < 0.5 {
            for k in 0..2 {
                let spread = (self.time * 13.0 + k as f64 * 2.1).sin();
                self.particles.push(Particle {
                    pos: feet + DVec3::new(-vx.signum() * 0.2, 0.05, spread * 0.2),
                    vel: DVec3::new(-vx * 0.06 + spread * 0.3, 0.5 + 0.3 * k as f64, spread * 0.4),
                    life: 0.7,
                    color: crate::canvas3d::mix(dust, Color::from_rgb8(0x6a, 0x74, 0x60), wet * 2.0),
                    size: 0.12 + 0.05 * k as f64,
                    puff: true,
                });
            }
        } else {
            for k in 0..5 {
                let a = self.time * 17.0 + k as f64 * 1.3;
                self.particles.push(Particle {
                    pos: feet,
                    vel: DVec3::new(-vx * 0.1 + a.cos() * 1.2, 2.0 + a.sin().abs() * 1.5, a.sin() * 0.8),
                    life: 0.35,
                    color: Color::from_rgb8(0xd0, 0xe0, 0xec).with_alpha(0.7),
                    size: 0.035,
                    puff: false,
                });
            }
        }
    }

    /// The surroundings, for the ambient sound.
    pub fn ambience(&self) -> Ambience {
        let p = &self.player;
        // Air rushes past when he runs flat out, flies or swings fast.
        let speed = if p.on_ground { p.vel.x.abs() - 16.0 } else { p.vel.length() - 9.0 };
        Ambience {
            rain: self.weather.rain as f32,
            wind: self.weather.wind as f32,
            rush: (speed / 24.0).clamp(0.0, 1.0) as f32,
        }
    }

    /// Hair has inertia: when his head speeds up, stops, bobs, lands or
    /// turns, the hair lags behind and springs back.
    fn update_hair(&mut self, dt: f64) {
        let p = &self.player;
        // The head moves with the body, plus the run's bob and the idle nods.
        let bob = if p.on_ground { (p.stride * 2.0).cos() * 0.6 * (p.vel.x.abs() / 14.0).min(1.0) } else { 0.0 };
        let nod = (self.time * 0.31).cos() * 0.15;
        let head_vel = p.vel + DVec2::new(0.0, bob + nod);
        let accel = ((head_vel - self.prev_head_vel) / dt.max(1e-4)).clamp_length_max(120.0);
        self.prev_head_vel = head_vel;
        // Spring back to rest; pushed the opposite way to the acceleration.
        let force = -self.hair_swing * 45.0 - self.hair_swing_vel * 5.0 - accel * 0.07;
        self.hair_swing_vel += force * dt;
        self.hair_swing = (self.hair_swing + self.hair_swing_vel * dt).clamp_length_max(0.14);
        // Facing swings round after the head, overshooting a little.
        let target = self.turn.clamp(-1.0, 1.0);
        let pull = (target - self.hair_facing) * 60.0 - self.hair_facing_vel * 9.0;
        self.hair_facing_vel += pull * dt;
        self.hair_facing = (self.hair_facing + self.hair_facing_vel * dt).clamp(-1.15, 1.15);
    }

    fn update_effects(&mut self, dt: f64) {
        self.flash = (self.flash - dt * 1.5).max(0.0);
        for p in &mut self.particles {
            if p.puff {
                p.vel *= (-dt * 3.0).exp();
            } else {
                p.vel.y -= 12.0 * dt;
            }
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
                puff: false,
            });
        }
    }

    /// How far into a breath he is: 0 (out) to 1 (in), deeper when winded.
    fn breath(&self) -> f64 {
        (0.5 - 0.5 * self.breath_phase.cos()) * (1.0 + 0.5 * self.exertion)
    }

    fn motion(&self) -> Motion {
        let p = &self.player;
        Motion {
            time: self.time,
            stride: p.stride,
            run: (p.vel.x.abs() / 14.0).min(1.0),
            vel: DVec3::new(p.vel.x, p.vel.y, 0.0),
            heading: hero::heading(self.turn),
            airborne: !p.on_ground,
            crouch: p.crouch(),
            breath: self.breath(),
        }
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
        let state = if p.on_ground { "ground" } else { "air" };
        let caught = self.caught.iter().filter(|c| **c).count();
        format!("pos ({:.2}, {:.2}) vel ({:.1}, {:.1}) {state} flies {caught} checkpoint {}", p.pos.x, p.pos.y, p.vel.x, p.vel.y, self.checkpoint)
    }

    /// Draws a frame into the renderer's layers (see `frame`). With an image
    /// in `hair_style`, Konrad's hair is shader-rendered and its strands are
    /// returned for the hair pass.
    pub fn draw(&self, layers: &mut Layers, w: f64, h: f64, hair_style: HairStyle) -> FrameInfo {
        // In portrait, zoom out so there's still room to see ahead.
        let view_height = self.view_height.max(MIN_VIEW_WIDTH * h / w.max(1.0));
        // At full speed he runs in the back fifth of the screen.
        let lead_room = view_height * w / h.max(1.0) * 0.28;
        let camera = Camera {
            // The extra height in portrait goes mostly above him, not into the ground.
            eye: DVec3::new(self.camera.x + self.lead * lead_room, self.camera.y + (view_height - self.view_height) * 0.3, -CAMERA_DISTANCE),
            focal: h / view_height * CAMERA_DISTANCE,
            center: Point::new(w / 2.0, h / 2.0),
        };
        // The background layers are rendered at half resolution.
        let half = Camera { focal: camera.focal * 0.5, center: Point::new(w / 4.0, h / 4.0), ..camera };
        let (rain, wind) = (self.weather.rain, self.weather.wind);
        jungle::set_wind(wind);
        let skip = SKIP.load(std::sync::atomic::Ordering::Relaxed);
        let dusk = self.level.theme == Theme::Dusk;
        if skip & SKIP_BACKGROUND == 0 {
            crate::paint::set_view(w / 2.0, h / 2.0, h / view_height / 170.0);
            if dusk {
                dusk::draw_background(&mut layers.far, &mut layers.mid, &half, w / 2.0, h / 2.0, self.time);
            } else {
                jungle::draw_background(&mut layers.far, &mut layers.mid, &half, w / 2.0, h / 2.0, self.time);
                weather::draw_birds(&mut layers.far, &half, w / 2.0, h / 2.0, self.time, rain);
                weather::draw_fog(&mut layers.mid, &half, w / 2.0, self.time, rain, wind);
                let mut mid = Canvas3d::new(half);
                weather::draw_leaves(&mut mid, w / 2.0, self.time, wind, (6.0, 14.0), 70);
                mid.finish(&mut layers.mid);
            }
        }
        crate::paint::set_view(w, h, h / view_height / 85.0);

        let scene = &mut layers.front;
        if skip & SKIP_WORLD == 0 {
            // Mist in the jungle's pits; dusky haze deep between the buildings.
            let tint = if dusk { Color::from_rgb8(0x8e, 0x4a, 0x70) } else { Color::from_rgb8(0x6e, 0x86, 0x94) };
            weather::draw_pit_mist(scene, &camera, w, h, rain, tint);
        }
        let mut canvas = Canvas3d::new(camera);
        if skip & SKIP_WORLD == 0 {
            let view = WorldView {
                time: self.time,
                flies: &self.flies,
                caught: &self.caught,
                checkpoint: self.checkpoint,
                screen_width: w,
                player: self.player.pos,
                grounded: self.player.on_ground,
                rain,
            };
            if dusk {
                dusk::draw_world(&mut canvas, &self.level, &view);
            } else {
                jungle::draw_world(&mut canvas, &self.level, &view);
                weather::draw_leaves(&mut canvas, w, self.time, wind, (-2.5, 2.0), 90);
                weather::draw_fireflies(&mut canvas, w, self.time, rain);
                weather::draw_rain(&mut canvas, &self.level, w, h, self.time, rain, wind);
            }
        }
        let mut hair = None;
        if skip & SKIP_JOE == 0 {
            hair = self.draw_joe(&mut canvas, hair_style);
        }
        for p in &self.particles {
            let pr = camera.project(p.pos);
            if p.puff {
                // A soft puff that grows as it fades.
                let r = p.size * pr.scale * (1.0 + 2.5 * (0.7 - p.life).max(0.0));
                let alpha = (0.45 * p.life / 0.7).min(0.45) as f32;
                let soft = Gradient::new_radial(pr.pos, r as f32).with_stops([
                    (0.0, p.color.with_alpha(alpha)),
                    (1.0, p.color.with_alpha(0.0)),
                ]);
                canvas.push(pr.depth, move |scene| {
                    scene.fill(Fill::NonZero, Affine::IDENTITY, &soft, None, &Circle::new(pr.pos, r));
                });
                continue;
            }
            let r = p.size * pr.scale * p.life.min(1.0);
            let color = p.color;
            canvas.push(pr.depth, move |scene| {
                scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &Circle::new(pr.pos, r));
            });
        }
        canvas.finish(scene);

        self.draw_hud(scene, w, h);
        if self.flash > 0.0 {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Color::BLACK.with_alpha(self.flash as f32),
                None,
                &Rect::new(0.0, 0.0, w, h),
            );
        }
        FrameInfo {
            hair,
            post: Post {
                sun: if dusk { [dusk::SUN.0 as f32, dusk::SUN.1 as f32] } else { [crate::paint::SUN.0 as f32, crate::paint::SUN.1 as f32] },
                rain: rain as f32,
                rays: if dusk { 0.5 } else { 1.0 },
                grade: skip & SKIP_GRADE == 0,
                theme: self.level.theme,
                time: self.time as f32,
                pan: camera.eye.x as f32,
            },
        }
    }

    /// How much sunlight reaches `pos`: less under the tree canopies and
    /// overhead platforms, and in the rain.
    fn light_at(&self, pos: DVec2) -> f64 {
        let mut light: f64 = 1.0;
        for t in &self.level.trees {
            if (pos.x - t.x).abs() < 3.0 {
                light = light.min(0.45 + 0.55 * ((pos.x - t.x).abs() / 3.0).powi(2));
            }
        }
        for b in &self.level.blocks {
            if b.y0 > pos.y + 1.0 && b.y0 < pos.y + 7.0 && pos.x > b.x0 - 0.3 && pos.x < b.x1 + 0.3 {
                light = light.min(0.65);
            }
        }
        light * (1.0 - 0.35 * self.weather.rain)
    }

    fn draw_joe(&self, canvas: &mut Canvas3d, hair_style: HairStyle) -> Option<HairFrame> {
        let motion = self.motion();
        let pose = self.animator.pose(&motion);
        let p = &self.player;
        let rot = motion.heading;
        let feet = DVec3::new(p.pos.x, p.pos.y, 0.0);
        // A soft violet contact shadow on the ground below, shrinking and
        // fading as Joe gets higher.
        let ground = self
            .level
            .blocks
            .iter()
            .filter(|b| b.x0 <= p.pos.x && p.pos.x <= b.x1 && b.y1 <= p.pos.y + 0.05)
            .map(|b| b.y1)
            .fold(f64::NEG_INFINITY, f64::max);
        let fade = (1.0 - (p.pos.y - ground) / 6.0).clamp(0.0, 1.0);
        if fade > 0.0 {
            let center = DVec3::new(p.pos.x, ground, 0.15);
            let size = 0.6 + 0.4 * fade;
            let ellipse = canvas.project_ellipsoid(
                center,
                DMat3::from_cols(DVec3::X * 0.8 * size, DVec3::Z * 0.45 * size, DVec3::Y * 1e-3),
            );
            let shape = Affine::translate(ellipse.center().to_vec2())
                * Affine::rotate(ellipse.rotation())
                * Affine::scale_non_uniform(ellipse.radii().x, ellipse.radii().y);
            let shadow = Gradient::new_radial((0.0, 0.0), 1.0).with_stops([
                (0.0, Color::from_rgb8(0x2e, 0x22, 0x4a).with_alpha(0.5 * fade as f32)),
                (0.6, Color::from_rgb8(0x2e, 0x22, 0x4a).with_alpha(0.3 * fade as f32)),
                (1.0, Color::from_rgb8(0x2e, 0x22, 0x4a).with_alpha(0.0)),
            ]);
            // A long shadow cast away from the sun, fainter in the shade.
            let light = self.light_at(p.pos);
            let cast_center = DVec3::new(p.pos.x - 1.1 * fade, ground, 0.2);
            let cast = canvas.project_ellipsoid(
                cast_center,
                DMat3::from_cols(DVec3::X * 1.5 * size, DVec3::Z * 0.3 * size, DVec3::Y * 1e-3),
            );
            let cast_shape = Affine::translate(cast.center().to_vec2())
                * Affine::rotate(cast.rotation())
                * Affine::scale_non_uniform(cast.radii().x, cast.radii().y);
            // (In the dusk city the long shadow is cast from his skeleton.)
            let cast_alpha = if self.level.theme == Theme::Dusk { 0.0 } else { (0.35 * fade * light * light) as f32 };
            let cast_fill = Gradient::new_radial((0.0, 0.0), 1.0).with_stops([
                (0.0, Color::from_rgb8(0x14, 0x12, 0x2a).with_alpha(cast_alpha)),
                (1.0, Color::from_rgb8(0x14, 0x12, 0x2a).with_alpha(0.0)),
            ]);
            let depth = canvas.depth_of(DVec3::new(p.pos.x, ground, 0.6));
            canvas.push(depth, move |scene| {
                scene.fill(Fill::NonZero, Affine::IDENTITY, &cast_fill, Some(cast_shape), &cast);
                scene.fill(Fill::NonZero, Affine::IDENTITY, &shadow, Some(shape), &ellipse);
            });
        }

        // Cartoon speed stretch: the faster Joe goes, the longer and thinner.
        let stretch = ((p.vel.x.abs() - 12.0) / 22.0).clamp(0.0, 1.0) * 0.7;
        let root = hero::root(feet, rot, self.squash, stretch);
        let standing = p.on_ground;
        let solved = hero::plant(&self.skeleton, &pose, root, standing);
        let dusk = self.level.theme == Theme::Dusk;
        if dusk {
            dusk::draw_body_shadow(canvas, &self.level, &solved);
        }
        let look = Look {
            time: self.time,
            vel: DVec3::new(p.vel.x, p.vel.y, 0.0),
            stride: p.stride,
            hair_swing: self.hair_swing,
            hair_facing: self.hair_facing,
            light: self.light_at(p.pos),
            wind: self.weather.wind,
            breath: self.breath(),
            camo: self.camo,
            camo_colors: self.camo_colors(),
            // The dusk sun is low on the right, and rims him in orange.
            sun_dir: if dusk { dusk::SUN_SCREEN_DIR } else { Vec2::new(0.55, -0.83) },
            rim: if dusk { Color::from_rgb8(0xff, 0xa4, 0x68) } else { hero::RIM },
        };
        // The hero is drawn as one group, sorted as a whole against the world
        // (no outline: the flat, outline-free style of the era).
        let hero_depth = canvas.depth_of(DVec3::new(p.pos.x, p.pos.y, 0.0));
        let mut figure = Canvas3d::group(canvas.camera, 0.0);
        let hair = hero::draw(&mut figure, &solved, &look, hair_style);
        canvas.push(hero_depth, figure.into_group());
        hair
    }

    fn draw_hud(&self, scene: &mut Scene, w: f64, h: f64) {
        // One fly icon per fly: filled when caught.
        let n = self.caught.len() as f64;
        let r = (h * 0.014).max(4.0).min(w / (n * 2.6 + 2.0));
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
