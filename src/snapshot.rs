//! Developer tool: runs the game headless with scripted input and saves the
//! last frame as a PPM image. Useful for testing moves and taking screenshots.
//!
//! Script: space-separated steps `KEYS:seconds`, where KEYS is any of
//! L R U D J T (left, right, up, down, jump, tongue) or `-` for none.
//! A step `@N` starts from checkpoint N instead.
//! Example: `-:1 R:1.2 RJ:0.3 R:0.8`

use vello::peniko::Color;
use vello::util::RenderContext;
use vello::{AaConfig, Renderer, RendererOptions, Scene};

use crate::game::Game;
use crate::terminal::Target;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;

pub fn run(out: &str, script: &str) {
    if let Err(err) = run_inner(out, script) {
        eprintln!("cameljon: {err}");
        std::process::exit(1);
    }
}

fn run_inner(out: &str, script: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut game = Game::new();
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
    let target = Target::new(device, WIDTH, HEIGHT);
    let mut scene = Scene::new();
    // Time building the scene (the CPU side of a frame), averaged.
    let runs = 20;
    let start = web_time::Instant::now();
    for _ in 0..runs {
        scene.reset();
        game.draw(&mut scene, WIDTH as f64, HEIGHT as f64);
    }
    eprintln!("scene build: {:.2} ms", start.elapsed().as_secs_f64() * 1000.0 / runs as f64);
    renderer.render_to_texture(
        device,
        queue,
        &scene,
        &target.view,
        &vello::RenderParams {
            base_color: Color::BLACK,
            width: WIDTH,
            height: HEIGHT,
            antialiasing_method: AaConfig::Area,
        },
    )?;
    let rgba = target.read_pixels(device, queue)?;
    let mut ppm = format!("P6\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
    ppm.extend(rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]));
    std::fs::write(out, ppm)?;
    Ok(())
}
