//! Developer tool: runs the game headless with scripted input and saves the
//! last frame as a PPM image. Useful for testing moves and taking screenshots.
//!
//! Script: space-separated steps `KEYS:seconds`, where KEYS is any of
//! L R U D J T (left, right, up, down, jump, tongue) or `-` for none.
//! A step `@N` starts from checkpoint N instead. Environment variables:
//! `CAMEL_EON_SIZE=WxH`, `CAMEL_EON_DETAIL=0..2`, `CAMEL_EON_GPU_BENCH=1`,
//! `CAMEL_EON_HAIR=vector`.
//! Example: `-:1 R:1.2 RJ:0.3 R:0.8`

use vello::peniko::Color;
use vello::util::RenderContext;
use vello::{AaConfig, Renderer, RendererOptions, Scene};

use crate::game::Game;
use crate::terminal::Target;

/// Default image size; override with e.g. `CAMEL_EON_SIZE=1080x2400`.
const DEFAULT_SIZE: (u32, u32) = (1280, 720);

pub fn run(out: &str, script: &str) {
    if let Err(err) = run_inner(out, script) {
        eprintln!("camel-eon: {err}");
        std::process::exit(1);
    }
}

fn run_inner(out: &str, script: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut game = Game::new();
    // Detail level to test the low-detail modes: `CAMEL_EON_DETAIL=2`.
    if let Some(level) = std::env::var("CAMEL_EON_DETAIL").ok().and_then(|v| v.parse().ok()) {
        crate::paint::set_detail(level);
    }
    for step in script.split_whitespace() {
        if let Some(n) = step.strip_prefix('@') {
            game.warp(n.parse()?);
            continue;
        }
        let (keys, secs) = step.split_once(':').ok_or("script steps look like KEYS:seconds")?;
        let secs: f64 = secs.parse()?;
        game.input.left = keys.contains('L');
        game.input.right = keys.contains('R');
        game.input.up = keys.contains('U');
        game.input.down = keys.contains('D');
        game.input.jump = keys.contains('J');
        game.input.tongue = keys.contains('T');
        let frames = (secs * 60.0).round() as usize;
        for _ in 0..frames {
            game.update(1.0 / 60.0);
        }
        eprintln!("{step:>8} -> {}", game.status());
    }

    let mut context = RenderContext::new();
    let dev_id = pollster::block_on(context.device(None)).ok_or("no compatible GPU found")?;
    let device = &context.devices[dev_id].device;
    let queue = &context.devices[dev_id].queue;
    let mut renderer = Renderer::new(
        device,
        RendererOptions {
            antialiasing_support: vello::AaSupport::area_only(),
            ..Default::default()
        },
    )?;
    let (width, height) = std::env::var("CAMEL_EON_SIZE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or(DEFAULT_SIZE);
    let target = Target::new(device, width, height);
    // Shader hair unless `CAMEL_EON_HAIR=vector`.
    let hair = (std::env::var("CAMEL_EON_HAIR").as_deref() != Ok("vector"))
        .then(|| crate::hair::HairRenderer::new(device, &mut renderer));
    if std::env::var("CAMEL_EON_GPU_BENCH").is_ok() {
        gpu_bench(&mut game, &mut renderer, device, queue, &target, width, height)?;
    }
    let mut scene = Scene::new();
    // Time building the scene (the CPU side of a frame), averaged.
    let runs = 20;
    let start = web_time::Instant::now();
    for _ in 0..runs {
        scene.reset();
        game.draw(&mut scene, width as f64, height as f64, hair.as_ref().map(|h| &h.image));
    }
    eprintln!("scene build: {:.2} ms", start.elapsed().as_secs_f64() * 1000.0 / runs as f64);
    scene.reset();
    if let Some(frame) = game.draw(&mut scene, width as f64, height as f64, hair.as_ref().map(|h| &h.image)) {
        hair.as_ref().expect("hair frames only come with a hair renderer").render(device, queue, &frame);
    }
    renderer.render_to_texture(
        device,
        queue,
        &scene,
        &target.view,
        &vello::RenderParams {
            base_color: Color::BLACK,
            width,
            height,
            antialiasing_method: AaConfig::Area,
        },
    )?;
    let rgba = target.read_pixels(device, queue)?;
    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    ppm.extend(rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]));
    std::fs::write(out, ppm)?;
    Ok(())
}

/// Times the GPU side of a frame for the full scene and with parts left out,
/// to see what the GPU spends its time on.
fn gpu_bench(
    game: &mut Game,
    renderer: &mut Renderer,
    device: &vello::wgpu::Device,
    queue: &vello::wgpu::Queue,
    target: &Target,
    width: u32,
    height: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::game::{SKIP, SKIP_BACKGROUND, SKIP_GRADE, SKIP_JOE, SKIP_WORLD};
    use std::sync::atomic::Ordering;
    let variants = [
        ("full frame", 0),
        ("without background", SKIP_BACKGROUND),
        ("without platforms/props", SKIP_WORLD),
        ("without Joe", SKIP_JOE),
        ("without lighting grade", SKIP_GRADE),
        ("empty", SKIP_BACKGROUND | SKIP_WORLD | SKIP_JOE | SKIP_GRADE),
    ];
    let mut scene = Scene::new();
    for (name, skip) in variants {
        SKIP.store(skip, Ordering::Relaxed);
        scene.reset();
        game.draw(&mut scene, width as f64, height as f64, None);
        let params = vello::RenderParams {
            base_color: Color::BLACK,
            width,
            height,
            antialiasing_method: AaConfig::Area,
        };
        let mut render = || -> Result<(), Box<dyn std::error::Error>> {
            renderer.render_to_texture(device, queue, &scene, &target.view, &params)?;
            device.poll(vello::wgpu::PollType::wait_indefinitely())?;
            Ok(())
        };
        render()?; // Warm-up.
        let runs = 20;
        let start = web_time::Instant::now();
        for _ in 0..runs {
            render()?;
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / runs as f64;
        eprintln!("gpu {name:>26}: {ms:6.2} ms");
    }
    SKIP.store(0, Ordering::Relaxed);
    Ok(())
}
