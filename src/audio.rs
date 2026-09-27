//! Sound, all synthesised on the fly (no audio files): the game reports
//! one-shot effects (`Sfx`) and the state of the surroundings (`Ambience`),
//! and a small synth turns them into rain, wind, insects, birds, footsteps,
//! jumps and chimes. `Audio` plays it through cpal (Web Audio on the web).

use std::f32::consts::{PI, TAU};
use std::f64::consts::TAU as TAU64;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

/// A one-shot sound effect.
#[derive(Clone, Copy, Debug)]
pub enum Sfx {
    /// A footfall; `speed` 0..1 from a jog to flat out.
    Step { speed: f32 },
    /// Leaving the ground; `power` 0..1 from a hop to a full charge.
    Jump { power: f32 },
    /// Touching down at `speed` (world units per second, downwards).
    Land { speed: f32 },
    /// A fly caught.
    Gulp,
    Checkpoint,
    /// The camouflage coming on: a soft rising shimmer.
    Camo,
    /// Fell into a pit.
    Fall,
    Win,
}

/// The continuous sounds of the surroundings, each 0..1.
#[derive(Clone, Copy, Default, Debug)]
pub struct Ambience {
    pub rain: f32,
    pub wind: f32,
    /// Air rushing past when he runs fast or flies through the air.
    pub rush: f32,
}

/// Plays the synth on the sound device. Silent if there is none.
pub struct Audio {
    shared: Arc<Mutex<Shared>>,
    stream: Option<cpal::Stream>,
    started: bool,
}

/// Handed from the game to the audio callback.
#[derive(Default)]
struct Shared {
    queue: Vec<Sfx>,
    ambience: Ambience,
    muted: bool,
}

impl Audio {
    pub fn new() -> Self {
        Self { shared: Arc::default(), stream: None, started: false }
    }

    /// Opens the sound device the first time it's called. Browsers only
    /// allow sound once the player has pressed something, so on the web this
    /// is called from input events.
    pub fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        match open(self.shared.clone()) {
            Ok(stream) => self.stream = Some(stream),
            Err(err) => eprintln!("camel-eon: no sound: {err}"),
        }
    }

    /// Passes this frame's ambience and effects (draining `sounds`) on.
    pub fn feed(&self, ambience: Ambience, sounds: &mut Vec<Sfx>) {
        let Ok(mut shared) = self.shared.lock() else { return };
        if self.stream.is_some() {
            shared.queue.append(sounds);
            // Don't pile up if the callback has stalled (a hidden tab).
            if shared.queue.len() > 64 {
                shared.queue.clear();
            }
        } else {
            sounds.clear();
        }
        shared.ambience = ambience;
    }

    /// Stops and restarts the sound device (while the app is in the
    /// background).
    pub fn set_paused(&self, paused: bool) {
        let Some(stream) = &self.stream else { return };
        let result = if paused { stream.pause() } else { stream.play() };
        if let Err(err) = result {
            eprintln!("camel-eon: sound: {err}");
        }
    }

    pub fn toggle_mute(&self) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.muted = !shared.muted;
        }
    }
}

fn open(shared: Arc<Mutex<Shared>>) -> Result<cpal::Stream, Box<dyn std::error::Error>> {
    let device = cpal::default_host().default_output_device().ok_or("no output device")?;
    let supported = device.default_output_config()?;
    #[allow(unused_mut)]
    let mut config = supported.config();
    // Small buffers keep the effects in time with the picture. (The web
    // backend runs on the main thread, where bigger ones are safer.)
    #[cfg(not(target_arch = "wasm32"))]
    if let cpal::SupportedBufferSize::Range { min, max } = *supported.buffer_size() {
        config.buffer_size = cpal::BufferSize::Fixed(512u32.clamp(min, max));
    }
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, config, shared)?,
        SampleFormat::I16 => build::<i16>(&device, config, shared)?,
        SampleFormat::U16 => build::<u16>(&device, config, shared)?,
        other => return Err(format!("unsupported sample format {other}").into()),
    };
    stream.play()?;
    Ok(stream)
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: StreamConfig,
    shared: Arc<Mutex<Shared>>,
) -> Result<cpal::Stream, cpal::Error> {
    let channels = config.channels.max(1) as usize;
    let mut synth = Synth::new(config.sample_rate as f32);
    let mut muted = false;
    device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            // Never wait for the game: if it holds the lock, catch up next time.
            if let Ok(mut s) = shared.try_lock() {
                for sfx in s.queue.drain(..) {
                    synth.play(sfx);
                }
                synth.ambience = s.ambience;
                muted = s.muted;
            }
            for frame in data.chunks_mut(channels) {
                let (l, r) = if muted { (0.0, 0.0) } else { synth.next() };
                for (i, out) in frame.iter_mut().enumerate() {
                    *out = T::from_sample(match i {
                        0 => l,
                        1 => r,
                        _ => (l + r) * 0.5,
                    });
                }
            }
        },
        |err| eprintln!("camel-eon: sound error: {err}"),
        None,
    )
}

/// Cheap deterministic noise (xorshift).
struct Rng(u32);

impl Rng {
    /// 0..1.
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }

    /// −1..1, white noise.
    fn noise(&mut self) -> f32 {
        self.next() * 2.0 - 1.0
    }

    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.next()
    }
}

/// State-variable filter coefficients (the "TPT" form, stable when swept).
#[derive(Clone, Copy, Default)]
struct Coef {
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl Coef {
    fn new(freq: f32, q: f32, rate: f32) -> Self {
        let g = (PI * freq.clamp(10.0, rate * 0.45) / rate).tan();
        let k = 1.0 / q;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        Self { a1, a2, a3: g * a2, k }
    }
}

#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    /// Low-pass and band-pass (unity gain at the centre) outputs.
    fn run(&mut self, c: &Coef, x: f32) -> (f32, f32) {
        let v3 = x - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, v1 * c.k)
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
enum Wave {
    #[default]
    Sine,
    Triangle,
    /// Inharmonic partials, like struck metal or glass.
    Bell,
}

/// Partial frequency ratios and levels of `Wave::Bell`.
const BELL: [(f32, f32); 4] = [(1.0, 1.0), (2.76, 0.5), (5.4, 0.25), (8.93, 0.12)];

/// A one-shot sound: a tone gliding from `f0` to `f1` and/or band-passed
/// noise gliding from `n0` to `n1`, under an attack/decay envelope.
#[derive(Clone, Copy)]
struct Voice {
    /// Seconds before it starts.
    delay: f32,
    t: f32,
    /// Seconds it lasts.
    len: f32,
    attack: f32,
    /// Decay time constant, seconds.
    decay: f32,
    wave: Wave,
    tone: f32,
    f0: f32,
    f1: f32,
    /// Seconds the glides take.
    glide: f32,
    vibrato: f32,
    vibrato_rate: f32,
    noise: f32,
    n0: f32,
    n1: f32,
    q: f32,
    /// −1 (left) .. 1 (right).
    pan: f32,
    phase: [f32; 4],
    filter: Svf,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            delay: 0.0,
            t: 0.0,
            len: 0.3,
            attack: 0.003,
            decay: 0.1,
            wave: Wave::Sine,
            tone: 0.0,
            f0: 440.0,
            f1: 440.0,
            glide: 0.1,
            vibrato: 0.0,
            vibrato_rate: 0.0,
            noise: 0.0,
            n0: 1000.0,
            n1: 1000.0,
            q: 1.0,
            pan: 0.0,
            phase: [0.0; 4],
            filter: Svf::default(),
        }
    }
}

impl Voice {
    /// The next sample, or `None` once it has finished.
    fn next(&mut self, rate: f32, rng: &mut Rng) -> Option<(f32, f32)> {
        let dt = 1.0 / rate;
        if self.delay > 0.0 {
            self.delay -= dt;
            return Some((0.0, 0.0));
        }
        let t = self.t;
        if t >= self.len {
            return None;
        }
        self.t += dt;
        let k = (t / self.glide).min(1.0);
        let env = (t / self.attack).min(1.0) * (-t / self.decay).exp();
        let mut out = 0.0;
        if self.tone > 0.0 {
            let vib = 1.0 + self.vibrato * (TAU * self.vibrato_rate * t).sin();
            let f = self.f0 * (self.f1 / self.f0).powf(k) * vib;
            let partials = if self.wave == Wave::Bell { BELL.len() } else { 1 };
            for (i, &(ratio, level)) in BELL.iter().enumerate().take(partials) {
                self.phase[i] = (self.phase[i] + f * ratio * dt).fract();
                let p = self.phase[i];
                out += self.tone
                    * match self.wave {
                        Wave::Sine => (TAU * p).sin(),
                        Wave::Triangle => 1.0 - 4.0 * (p - 0.5).abs(),
                        // Higher partials die away faster.
                        Wave::Bell => (TAU * p).sin() * level * (-t * i as f32 * 6.0).exp(),
                    };
            }
        }
        if self.noise > 0.0 {
            let c = Coef::new(self.n0 * (self.n1 / self.n0).powf(k), self.q, rate);
            out += self.noise * self.filter.run(&c, rng.noise()).1;
        }
        let out = out * env;
        Some((out * (1.0 - self.pan.max(0.0)), out * (1.0 + self.pan.min(0.0))))
    }
}

/// A raindrop: a noise tick ringing a resonator.
#[derive(Clone, Copy, Default)]
struct Drop {
    coef: Coef,
    filter: Svf,
    env: f32,
    left: f32,
    right: f32,
}

/// Voices beyond this are dropped, oldest first.
const MAX_VOICES: usize = 32;
/// Filter coefficients of the ambience are updated this often (samples).
const CONTROL_EVERY: u32 = 64;

/// The synthesiser: ambience plus the one-shot voices.
pub struct Synth {
    rate: f32,
    rng: Rng,
    pub ambience: Ambience,
    /// Smoothed ambience, so changes fade instead of clicking.
    rain: f32,
    wind: f32,
    rush: f32,
    /// Seconds played (f64: f32 loses the insects' phase within minutes).
    time: f64,
    counter: u32,
    voices: Vec<Voice>,
    hiss: [Svf; 2],
    hiss_c: Coef,
    patter: [Svf; 2],
    patter_c: Coef,
    gust: [Svf; 2],
    gust_c: Coef,
    rumble: [Svf; 2],
    rumble_c: Coef,
    air: [Svf; 2],
    air_c: Coef,
    drops: [Drop; 8],
    next_drop: usize,
    drop_decay: f32,
    /// Seconds until the next bird call.
    bird_in: f32,
}

impl Synth {
    pub fn new(rate: f32) -> Self {
        Self {
            rate,
            rng: Rng(0x2545_f491),
            ambience: Ambience::default(),
            rain: 0.0,
            wind: 0.0,
            rush: 0.0,
            time: 0.0,
            counter: 0,
            voices: Vec::with_capacity(MAX_VOICES),
            hiss: [Svf::default(); 2],
            hiss_c: Coef::new(5000.0, 0.5, rate),
            patter: [Svf::default(); 2],
            patter_c: Coef::new(900.0, 0.6, rate),
            gust: [Svf::default(); 2],
            gust_c: Coef::default(),
            rumble: [Svf::default(); 2],
            rumble_c: Coef::new(90.0, 0.7, rate),
            air: [Svf::default(); 2],
            air_c: Coef::default(),
            drops: [Drop::default(); 8],
            next_drop: 0,
            drop_decay: (-1.0 / (0.0015 * rate)).exp(),
            bird_in: 3.0,
        }
    }

    fn add(&mut self, voice: Voice) {
        if self.voices.len() >= MAX_VOICES {
            self.voices.remove(0);
        }
        self.voices.push(voice);
    }

    pub fn play(&mut self, sfx: Sfx) {
        let r = &mut self.rng;
        match sfx {
            Sfx::Step { speed } => {
                // A soft thump and the crunch of the forest floor.
                let pitch = r.range(0.85, 1.15);
                let crunch = r.range(700.0, 1300.0);
                self.add(Voice { len: 0.12, decay: 0.035, tone: 0.2 + 0.15 * speed, f0: 120.0 * pitch, f1: 55.0, glide: 0.05, ..Voice::default() });
                self.add(Voice { len: 0.1, decay: 0.025, noise: 0.18 + 0.15 * speed, n0: crunch, n1: crunch * 0.6, q: 1.3, glide: 0.05, ..Voice::default() });
            }
            Sfx::Jump { power } => {
                self.add(Voice { len: 0.25, decay: 0.08, wave: Wave::Triangle, tone: 0.13, f0: 180.0, f1: 420.0 + 300.0 * power, glide: 0.12, ..Voice::default() });
                self.add(Voice { len: 0.25, attack: 0.02, decay: 0.08, noise: 0.25 + 0.15 * power, n0: 500.0, n1: 1800.0, q: 1.2, glide: 0.15, ..Voice::default() });
            }
            Sfx::Land { speed } => {
                let hard = ((speed - 3.0) / 18.0).clamp(0.0, 1.0);
                self.add(Voice { len: 0.3, decay: 0.07 + 0.05 * hard, tone: 0.25 + 0.45 * hard, f0: 140.0, f1: 45.0, glide: 0.09, ..Voice::default() });
                self.add(Voice { len: 0.2, decay: 0.04 + 0.03 * hard, noise: 0.2 + 0.45 * hard, n0: 900.0, n1: 350.0, q: 0.8, glide: 0.1, ..Voice::default() });
            }
            Sfx::Gulp => {
                // Blip-bloop, and a sparkle.
                self.add(Voice { len: 0.12, decay: 0.05, tone: 0.18, f0: 520.0, f1: 880.0, glide: 0.06, ..Voice::default() });
                self.add(Voice { delay: 0.07, len: 0.18, decay: 0.07, tone: 0.18, f0: 880.0, f1: 1320.0, glide: 0.06, ..Voice::default() });
                self.add(Voice { delay: 0.12, len: 0.6, decay: 0.15, wave: Wave::Bell, tone: 0.07, f0: 2640.0, f1: 2640.0, ..Voice::default() });
            }
            Sfx::Checkpoint => {
                for (i, f) in [523.3, 659.3, 784.0, 1046.5].into_iter().enumerate() {
                    let pan = (i as f32 - 1.5) * 0.2;
                    self.add(Voice { delay: i as f32 * 0.09, len: 0.9, decay: 0.25, wave: Wave::Bell, tone: 0.12, f0: f, f1: f, pan, ..Voice::default() });
                }
            }
            Sfx::Camo => {
                self.add(Voice { len: 1.4, attack: 0.5, decay: 0.5, noise: 0.07, n0: 1800.0, n1: 6500.0, q: 3.0, glide: 1.2, ..Voice::default() });
                self.add(Voice { len: 1.4, attack: 0.4, decay: 0.45, tone: 0.035, f0: 660.0, f1: 1320.0, glide: 1.2, vibrato: 0.01, vibrato_rate: 7.0, ..Voice::default() });
            }
            Sfx::Fall => {
                self.add(Voice { len: 0.9, attack: 0.02, decay: 0.35, wave: Wave::Triangle, tone: 0.16, f0: 520.0, f1: 110.0, glide: 0.7, vibrato: 0.04, vibrato_rate: 9.0, ..Voice::default() });
            }
            Sfx::Win => {
                let notes = [(0.0, 523.3), (0.12, 659.3), (0.24, 784.0), (0.36, 1046.5), (0.6, 784.0), (0.72, 1046.5)];
                for (i, (at, f)) in notes.into_iter().enumerate() {
                    let pan = if i % 2 == 0 { -0.3 } else { 0.3 };
                    self.add(Voice { delay: at, len: 0.6, decay: 0.18, wave: Wave::Triangle, tone: 0.12, f0: f, f1: f, pan, ..Voice::default() });
                }
                // A closing chord that rings out.
                for f in [523.3, 659.3, 784.0, 1046.5] {
                    self.add(Voice { delay: 0.9, len: 2.5, attack: 0.02, decay: 0.8, wave: Wave::Bell, tone: 0.08, f0: f, f1: f, vibrato: 0.004, vibrato_rate: 5.0, ..Voice::default() });
                }
            }
        }
    }

    /// A bird somewhere in the canopy: a few quick whistled chirps.
    fn bird(&mut self) {
        let r = &mut self.rng;
        let pan = r.range(-0.8, 0.8);
        let base = r.range(2200.0, 3800.0);
        let up = r.next() < 0.5;
        let count = 2 + (r.next() * 4.0) as usize;
        let gap = r.range(0.09, 0.16);
        let level = r.range(0.025, 0.05);
        for i in 0..count {
            let f = base * (1.0 + 0.04 * i as f32);
            let (f0, f1) = if up { (f * 0.75, f * 1.15) } else { (f * 1.15, f * 0.7) };
            self.add(Voice {
                delay: i as f32 * gap,
                len: 0.09,
                attack: 0.01,
                decay: 0.04,
                tone: level,
                f0,
                f1,
                glide: 0.07,
                vibrato: 0.03,
                vibrato_rate: 38.0,
                pan,
                ..Voice::default()
            });
        }
    }

    /// 0..1: the wind's gusts, rising and falling over a few seconds.
    fn gust(&self) -> f32 {
        let t = self.time as f32 % 1e4;
        0.5 + 0.5 * ((t * 0.37).sin() * 0.6 + (t * 1.13 + 1.0).sin() * 0.4)
    }

    /// Recomputes what changes slowly: smoothing, the wind's filters, drops
    /// and birds. Runs every `CONTROL_EVERY` samples.
    fn control(&mut self) {
        let dt = CONTROL_EVERY as f32 / self.rate;
        let ease = |value: &mut f32, target: f32, secs: f32| *value += (target - *value) * (1.0 - (-dt / secs).exp());
        let a = self.ambience;
        ease(&mut self.rain, a.rain.clamp(0.0, 1.0), 0.5);
        ease(&mut self.wind, a.wind.clamp(0.0, 1.0), 0.5);
        ease(&mut self.rush, a.rush.clamp(0.0, 1.0), 0.15);
        // Gusts: the wind's pitch rises as it blows harder.
        let gust = self.gust();
        self.gust_c = Coef::new(250.0 + 650.0 * gust * self.wind, 2.5, self.rate);
        self.air_c = Coef::new(350.0 + 1100.0 * self.rush, 0.9, self.rate);

        self.bird_in -= dt;
        if self.bird_in <= 0.0 {
            self.bird_in = self.rng.range(2.5, 8.0);
            if self.rng.next() > self.rain * 1.5 {
                self.bird();
            }
        }
    }

    /// The next stereo sample.
    pub fn next(&mut self) -> (f32, f32) {
        if self.counter == 0 {
            self.control();
        }
        self.counter = (self.counter + 1) % CONTROL_EVERY;
        self.time += 1.0 / self.rate as f64;
        let t = self.time;
        let gust = self.gust();
        let r = &mut self.rng;
        let (rain, wind, rush) = (self.rain, self.wind, self.rush);

        let mut out = [0.0f32; 2];
        for (ch, out) in out.iter_mut().enumerate() {
            // Each side gets its own noise, which makes the sound wide.
            let hiss = self.hiss[ch].run(&self.hiss_c, r.noise()).1;
            let patter = self.patter[ch].run(&self.patter_c, r.noise()).1;
            let gusty = self.gust[ch].run(&self.gust_c, r.noise()).1;
            let rumble = self.rumble[ch].run(&self.rumble_c, r.noise()).0;
            let air = self.air[ch].run(&self.air_c, r.noise()).1;
            *out = rain * (0.07 * hiss + 0.09 * patter)
                + wind * (0.35 + 0.65 * gust) * (0.9 * gusty + 0.6 * rumble)
                + rush * rush * 0.3 * air;
        }

        // Raindrops on leaves.
        if r.next() < rain * rain * 700.0 / self.rate {
            self.next_drop = (self.next_drop + 1) % self.drops.len();
            let d = &mut self.drops[self.next_drop];
            d.coef = Coef::new(r.range(1200.0, 4200.0), 10.0, self.rate);
            d.env = r.range(0.3, 1.0);
            let pan = r.range(-1.0, 1.0);
            (d.left, d.right) = (1.0 - pan.max(0.0), 1.0 + pan.min(0.0));
        }
        for d in &mut self.drops {
            if d.env < 1e-4 && d.filter.ic1.abs() < 1e-5 {
                continue;
            }
            let ring = d.filter.run(&d.coef, r.noise() * d.env).1 * 0.9;
            d.env *= self.drop_decay;
            out[0] += ring * d.left;
            out[1] += ring * d.right;
        }

        // Insects: two chirping crickets, quiet in the rain.
        let life = (1.0 - rain * 1.3).max(0.0) * 0.02;
        if life > 0.0 {
            for (ch, (freq, rate, offset)) in [(4300.0, 1.7, 0.0), (4700.0, 1.3, 0.4)].into_iter().enumerate() {
                let cycle = (t * rate + offset).fract();
                if cycle < 0.3 {
                    let pulse = ((TAU64 * t * 15.0).sin() as f32).powi(8);
                    out[ch] += life * pulse * (TAU64 * freq * t).sin() as f32;
                }
            }
        }

        let rate = self.rate;
        self.voices.retain_mut(|v| match v.next(rate, &mut self.rng) {
            Some((l, r)) => {
                out[0] += l;
                out[1] += r;
                true
            }
            None => false,
        });
        // Soft clipping keeps a pile-up of sounds from crackling.
        (out[0].tanh() * 0.8, out[1].tanh() * 0.8)
    }
}
