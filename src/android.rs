//! Android entry point: the crate root of the Android library
//! (`android/Cargo.toml`). It shares the game's modules with `main.rs`, minus
//! the terminal and snapshot tools; `android_main` is called by the
//! NativeActivity glue.

// Some code only serves the desktop's snapshot tool.
#![allow(dead_code)]

mod audio;
mod canvas3d;
mod dusk;
mod frame;
mod game;
mod gamepad;
mod hair;
mod jungle;
mod konrad;
mod level;
mod noise;
mod paint;
mod player;
mod rig;
mod stats;
mod touch;
mod weather;
mod window;

#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    // Panics would otherwise vanish without a trace; send them to logcat.
    std::panic::set_hook(Box::new(|info| log_error(&format!("crash: {info}"))));
    window::run_android(app);
}

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(priority: i32, tag: *const std::ffi::c_char, text: *const std::ffi::c_char) -> i32;
}

/// Writes an error to logcat (see it with `adb logcat -s camel-eon`).
pub fn log_error(message: &str) {
    const ANDROID_LOG_ERROR: i32 = 6;
    let text = std::ffi::CString::new(message.replace('\0', " ")).unwrap_or_default();
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe {
        __android_log_write(ANDROID_LOG_ERROR, c"camel-eon".as_ptr(), text.as_ptr());
    }
}
