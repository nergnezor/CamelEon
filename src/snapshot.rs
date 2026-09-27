//! Developer tool: runs the game headless with scripted input and saves the
//! last frame as a PPM image. Useful for testing moves and taking screenshots.
//!
//! Script: space-separated steps `KEYS:seconds`, where KEYS is any of
//! L R J (left, right, jump) or `-` for none.
//! A step `@N` starts from checkpoint N instead. Environment variables:
//! `CAMEL_EON_SIZE=WxH`, `CAMEL_EON_DETAIL=0..2`, `CAMEL_EON_GPU_BENCH=1`,
//! `CAMEL_EON_HAIR=shader|vector|both`, `CAMEL_EON_WEATHER=rain|clear`,
//! `CAMEL_EON_WAV=out.wav` (also records the sound of the whole script),
//! `CAMEL_EON_LEVEL=N` (0 = the dusk city, 1 = the jungle).
//! Example: `-:1 R:1.2 RJ:0.3 R:0.8`

use vello::util::RenderContext;

use crate::audio::Synth;
use crate::frame::{FrameRenderer, Layers};
use crate::game::Game;
use crate::hair::{HairMode, HairStyle};

/// Default image size; override with e.g. `CAMEL_EON_SIZE=1080x2400`.
const DEFAULT_SIZE: (u32, u32) = (1280, 720);

pub fn run(out: &str, script: &str) {
    if let Err(err) = run_inner(out, script) {
        eprintln!("camel-eon: {err}");
        std::process::exit(1);
    }
}

fn run_inner(out: &str, script: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Level: `CAMEL_EON_LEVEL=1` for the jungle.
    let level = std::env::var("CAMEL_EON_LEVEL").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut game = Game::with_level(level);
    // Detail level to test the low-detail modes: `CAMEL_EON_DETAIL=2`.
    if let Some(level) = std::env::var("CAMEL_EON_DETAIL").ok().and_then(|v| v.parse().ok()) {
        crate::paint::set_detail(level);
    }
    // Weather: `CAMEL_EON_WEATHER=rain` or `clear` (default: showers come and go).
    match std::env::var("CAMEL_EON_WEATHER").as_deref() {
        Ok("rain") => {
            game.weather.mode = crate::weather::Mode::Rain;
            game.weather.rain = 1.0;
        }
        Ok("clear") => game.weather.mode = crate::weather::Mode::Clear,
        _ => {}
    }
    // Sound: `CAMEL_EON_WAV=out.wav` renders what the script would sound like.
    let wav = std::env::var("CAMEL_EON_WAV").ok();
    let mut synth = Synth::new(AUDIO_RATE as f32);
    let mut samples: Vec<(f32, f32)> = Vec::new();
    for step in script.split_whitespace() {
        if let Some(n) = step.strip_prefix('@') {
            game.warp(n.parse()?);
            continue;
        }
        let (keys, secs) = step.split_once(':').ok_or("script steps look like KEYS:seconds")?;
        let secs: f64 = secs.parse()?;
        game.input.left = keys.contains('L');
        game.input.right = keys.contains('R');
        game.input.jump = keys.contains('J');
        let frames = (secs * 60.0).round() as usize;
        for _ in 0..frames {
            game.update(1.0 / 60.0);
            if wav.is_some() {
                for sfx in game.sounds.drain(..) {
                    synth.play(sfx);
                }
                synth.ambience = game.ambience();
                samples.extend((0..AUDIO_RATE / 60).map(|_| synth.next()));
            }
        }
        eprintln!("{step:>8} -> {}", game.status());
    }

    if let Some(path) = &wav {
        write_wav(path, &samples)?;
    }

    let mut context = RenderContext::new();
    let dev_id = pollster::block_on(context.device(None)).ok_or("no compatible GPU found")?;
    let device = &context.devices[dev_id].device;
    let queue = &context.devices[dev_id].queue;
    let mut renderer = FrameRenderer::new(device)?;
    // Hair: `CAMEL_EON_HAIR=shader`, `vector` or `both` (the default).
    renderer.hair_mode = match std::env::var("CAMEL_EON_HAIR").as_deref() {
        Ok("shader") => HairMode::Shader,
        Ok("vector") => HairMode::Vector,
        _ => HairMode::Both,
    };
    let (width, height) = std::env::var("CAMEL_EON_SIZE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or(DEFAULT_SIZE);
    if std::env::var("CAMEL_EON_GPU_BENCH").is_ok() {
        gpu_bench(&game, &mut renderer, device, queue, width, height)?;
    }
    // Time building the layers (the CPU side of a frame), averaged.
    let runs = 20;
    let mut layers = Layers::default();
    let start = web_time::Instant::now();
    for _ in 0..runs {
        layers = Layers::default();
        let image = renderer.hair_image();
        game.draw(&mut layers, width as f64, height as f64, HairStyle { image: image.as_ref(), locks: renderer.hair_locks() });
    }
    drop(layers);
    eprintln!("scene build: {:.2} ms", start.elapsed().as_secs_f64() * 1000.0 / runs as f64);
    // Render a fresh game first, like the window's earlier frames, so stale
    // GPU caches (e.g. the hair image in Vello's atlas) show up here too.
    let fresh = Game::with_level(level);
    renderer.render(device, queue, width, height, |layers, hair| fresh.draw(layers, width as f64, height as f64, hair))?;
    renderer.render(device, queue, width, height, |layers, hair| game.draw(layers, width as f64, height as f64, hair))?;
    let rgba = renderer.read_pixels(device, queue)?;
    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    ppm.extend(rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]));
    std::fs::write(out, ppm)?;
    Ok(())
}

const AUDIO_RATE: u32 = 48_000;

/// Saves stereo samples as a 16-bit WAV file and prints their levels.
fn write_wav(path: &str, samples: &[(f32, f32)]) -> Result<(), Box<dyn std::error::Error>> {
    let peak = samples.iter().map(|&(l, r)| l.abs().max(r.abs())).fold(0.0, f32::max);
    let rms = (samples.iter().map(|&(l, r)| (l * l + r * r) as f64 / 2.0).sum::<f64>() / samples.len().max(1) as f64).sqrt();
    eprintln!("sound: {:.1} s, peak {peak:.2}, rms {rms:.3}", samples.len() as f64 / AUDIO_RATE as f64);
    let bytes = samples.len() as u32 * 4;
    let mut out = Vec::with_capacity(44 + bytes as usize);
    out.extend(b"RIFF");
    out.extend((36 + bytes).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes()); // PCM
    out.extend(2u16.to_le_bytes()); // stereo
    out.extend(AUDIO_RATE.to_le_bytes());
    out.extend((AUDIO_RATE * 4).to_le_bytes());
    out.extend(4u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(bytes.to_le_bytes());
    for &(l, r) in samples {
        for v in [l, r] {
            out.extend(((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
    }
    std::fs::write(path, out)?;
    Ok(())
}

/// Times a whole frame on the GPU, and with parts left out, to see what the
/// GPU spends its time on.
fn gpu_bench(
    game: &Game,
    renderer: &mut FrameRenderer,
    device: &vello::wgpu::Device,
    queue: &vello::wgpu::Queue,
    width: u32,
    height: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::game::{SKIP, SKIP_BACKGROUND, SKIP_GRADE, SKIP_JOE, SKIP_WORLD};
    use std::sync::atomic::Ordering;
    let variants = [
        ("full frame", 0),
        ("without background", SKIP_BACKGROUND),
        ("without platforms/props", SKIP_WORLD),
        ("without Konrad", SKIP_JOE),
        ("without the grade", SKIP_GRADE),
        ("empty", SKIP_BACKGROUND | SKIP_WORLD | SKIP_JOE | SKIP_GRADE),
    ];
    for (name, skip) in variants {
        SKIP.store(skip, Ordering::Relaxed);
        let mut frame = || -> Result<(), Box<dyn std::error::Error>> {
            renderer.render(device, queue, width, height, |layers, hair| game.draw(layers, width as f64, height as f64, hair))?;
            device.poll(vello::wgpu::PollType::wait_indefinitely())?;
            Ok(())
        };
        frame()?; // Warm-up.
        let runs = 20;
        let start = web_time::Instant::now();
        for _ in 0..runs {
            frame()?;
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / runs as f64;
        eprintln!("gpu {name:>26}: {ms:6.2} ms");
    }
    SKIP.store(0, Ordering::Relaxed);
    Ok(())
}
