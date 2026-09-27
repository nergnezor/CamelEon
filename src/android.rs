//! Android entry point: the crate root of the Android library
//! (`android/Cargo.toml`). It shares the game's modules with `main.rs`, minus
//! the terminal and snapshot tools; `android_main` is called by the
//! NativeActivity glue.

// Some code only serves the desktop's snapshot tool.
#![allow(dead_code)]

mod audio;
mod canvas3d;
mod frame;
mod game;
mod gamepad;
mod hair;
mod jungle;
mod konrad;
mod level;
mod paint;
mod player;
mod rig;
mod stats;
mod touch;
mod weather;
mod window;

#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    window::run_android(app);
}
