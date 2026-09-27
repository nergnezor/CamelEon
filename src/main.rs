mod konrad;
mod canvas3d;
mod game;
mod gamepad;
mod hair;
mod jungle;
mod level;
mod paint;
mod player;
mod rig;
mod stats;
#[cfg(not(target_arch = "wasm32"))]
mod snapshot;
#[cfg(not(target_arch = "wasm32"))]
mod terminal;
mod touch;
mod window;

#[cfg(not(target_arch = "wasm32"))]
const USAGE: &str = "\
Usage: camel-eon [MODE]

  (none)        Play in a window
  --terminal    Play inside the terminal (kitty graphics protocol)
  --direct      Like --terminal, but sends frames through the terminal stream
                instead of shared memory (works over SSH)
  --snapshot OUT.ppm [SCRIPT]
                Developer tool: run with scripted input (e.g. -:1 R:1.2 RJ:0.3)
                and save the last frame

Keys V, F and H toggle vsync, the FPS overlay and shader hair (window mode).

Controls:
  ← → / A D     run
  ↑ ↓ / W S     climb (and shorten/lengthen the tongue while swinging)
  Space / Z     jump (let go while swinging)
  X / J         tongue: catch flies, grab flowers to swing
  Esc (or q in the terminal) quits.

Gamepad: left stick or d-pad to move and climb, A/Cross to jump,
X/Square, B/Circle or right trigger for the tongue.";

fn main() {
    #[cfg(target_arch = "wasm32")]
    {
        std::panic::set_hook(Box::new(|info| {
            console_error_panic_hook::hook(info);
            window::show_error(&format!("crash: {info}"));
        }));
        window::run();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        #[cfg(feature = "hotpatch")]
        dioxus_devtools::connect_subsecond();
        let arg = std::env::args().nth(1);
        match arg.as_deref() {
            None => window::run(),
            Some("--terminal") => terminal::run(terminal::Transfer::default()),
            Some("--direct") => terminal::run(terminal::Transfer::Direct),
            Some("--snapshot") => {
                let args: Vec<String> = std::env::args().skip(2).collect();
                let out = args.first().map(String::as_str).unwrap_or("snapshot.ppm");
                snapshot::run(out, args.get(1).map(String::as_str).unwrap_or(""));
            }
            Some(_) => {
                eprintln!("{USAGE}");
                std::process::exit(2);
            }
        }
    }
}
