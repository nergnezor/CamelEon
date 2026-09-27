//! Window frontend: renders straight to a window via winit and wgpu. On the
//! web the "window" is a full-page canvas.

use std::sync::Arc;

use vello::kurbo::{Affine, Point};
use vello::util::{RenderContext, RenderSurface};
use vello::wgpu::{self, CurrentSurfaceTexture};
use vello::Scene;
use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::audio::Audio;
use crate::game::{Game, Input};
use crate::frame::FrameRenderer;
use crate::gamepad::Gamepads;
use crate::stats::FrameStats;
use crate::touch::TouchControls;

struct RenderState {
    surface: RenderSurface<'static>,
    window: Arc<Window>,
    /// Renders frames, possibly below screen resolution; they're then scaled
    /// up onto the screen with linear filtering.
    frame: FrameRenderer,
    upscaler: wgpu::util::TextureBlitter,
}

/// Keeps the game at 60 FPS on slow devices. If frames are slow and the CPU
/// is the limit, the drawing detail drops (see `paint::set_detail`); if the
/// CPU has time to spare, the GPU is the limit and the render resolution
/// drops instead. Each drop is checked: if it didn't help, it's undone and
/// left alone for a while. Quality comes back when there's headroom.
struct Resolution {
    scale: f64,
    detail: u8,
    /// Scale at which frames last got too slow; we stay a bit below it.
    ceiling: f64,
    /// Smoothed frame rate.
    avg: f64,
    good_streak: u32,
    /// After a drop: the frame rate, scale and detail before it.
    check: Option<(f64, f64, u8)>,
    /// Measurements are ignored until then (startup and resizes are slow).
    settle_until: Instant,
    /// No more drops until then (the last one didn't help).
    hold_until: Instant,
}

impl Resolution {
    const MIN: f64 = 0.35;
    /// Rendering more pixels than this at startup starts scaled down.
    const START_BUDGET: f64 = 3_500_000.0;
    /// Below this we act; the goal is at least 60.
    const SLOW_FPS: f64 = 57.0;
    /// CPU time per frame above which the CPU counts as the limit (of 16.7 ms).
    const CPU_BOUND_MS: f64 = 11.0;

    fn initial(pixels: f64) -> Self {
        let now = Instant::now();
        Self {
            scale: (Self::START_BUDGET / pixels.max(1.0)).sqrt().clamp(Self::MIN, 1.0),
            detail: 0,
            ceiling: 1.0,
            avg: 0.0,
            good_streak: 0,
            check: None,
            settle_until: now + web_time::Duration::from_secs(2),
            hold_until: now,
        }
    }

    fn settle(&mut self, secs: f64) {
        self.settle_until = Instant::now() + web_time::Duration::from_secs_f64(secs);
        self.good_streak = 0;
        self.avg = 0.0;
    }

    /// Called twice a second with the measured frame rate and CPU time.
    fn adapt(&mut self, fps: f64, cpu_ms: f64, vsync: bool) {
        let now = Instant::now();
        if !vsync || now < self.settle_until {
            return; // Benchmarking, or still warming up.
        }
        self.avg = if self.avg == 0.0 { fps } else { self.avg * 0.6 + fps * 0.4 };

        if let Some((before, scale, detail)) = self.check.take() {
            if self.avg < before * 1.06 && self.avg < Self::SLOW_FPS {
                // That didn't make it faster: undo it.
                self.scale = scale;
                self.detail = detail;
                crate::paint::set_detail(detail);
                self.hold_until = now + web_time::Duration::from_secs(15);
                self.settle(1.0);
                return;
            }
        }
        if self.avg < Self::SLOW_FPS && now >= self.hold_until {
            let (scale, detail) = (self.scale, self.detail);
            let cpu_bound = cpu_ms > Self::CPU_BOUND_MS;
            if (cpu_bound || self.scale <= Self::MIN) && self.detail < crate::paint::MAX_DETAIL_DROP {
                self.detail += 1;
                crate::paint::set_detail(self.detail);
            } else if self.scale > Self::MIN {
                // GPU time scales with pixel count, i.e. with scale²: aim for 60.
                self.ceiling = scale;
                self.scale = (scale * (self.avg / 60.0).sqrt() * 0.95).clamp(Self::MIN, 1.0);
            } else {
                return; // Nothing left to give.
            }
            self.check = Some((self.avg, scale, detail));
            self.settle(1.5);
        } else if self.avg > 58.5 && cpu_ms < Self::CPU_BOUND_MS * 0.7 {
            self.good_streak += 1;
            if self.good_streak >= 4 {
                self.good_streak = 0;
                if self.scale < 1.0 {
                    // Step up, but not quite back to where it got slow; the
                    // ceiling slowly relaxes so we retry now and then.
                    self.ceiling = (self.ceiling + 0.03).min(1.0);
                    self.scale = (self.scale * 1.1).min(self.ceiling * 0.97).max(self.scale).min(1.0);
                } else if self.detail > 0 {
                    self.detail -= 1;
                    crate::paint::set_detail(self.detail);
                }
            }
        }
    }
}

/// GPU setup finished (it's asynchronous on the web).
enum UserEvent {
    Ready(Box<(RenderContext, RenderState)>),
    Failed(String),
}

struct App {
    context: Option<RenderContext>,
    state: Option<RenderState>,
    proxy: EventLoopProxy<UserEvent>,
    game: Game,
    audio: Audio,
    gamepads: Gamepads,
    touch: TouchControls,
    stats: FrameStats,
    resolution: Resolution,
    vsync: bool,
    last_frame: Instant,
}

async fn init(mut context: RenderContext, window: Arc<Window>) -> Result<(RenderContext, RenderState), String> {
    let size = window.inner_size();
    let surface = context
        .create_surface(window.clone(), size.width.max(1), size.height.max(1), wgpu::PresentMode::AutoVsync)
        .await
        .map_err(|e| format!("failed to create surface: {e}"))?;
    let device = &context.devices[surface.dev_id].device;
    // GPU validation errors would otherwise vanish silently (a blank screen).
    device.on_uncaptured_error(Arc::new(|err: wgpu::Error| show_error(&format!("GPU error: {err}"))));
    let upscaler = wgpu::util::TextureBlitterBuilder::new(device, surface.format)
        .sample_type(wgpu::FilterMode::Linear)
        .build();
    let frame = FrameRenderer::new(device).map_err(|e| format!("failed to create renderer: {e}"))?;
    Ok((context, RenderState { surface, window, frame, upscaler }))
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Some(context) = self.context.take() else { return };
        #[allow(unused_mut)]
        let mut attributes = Window::default_attributes().with_title("Camel Eon");
        #[cfg(target_arch = "wasm32")]
        {
            use winit::platform::web::WindowAttributesExtWebSys;
            attributes = attributes.with_append(true);
        }
        let window = Arc::new(event_loop.create_window(attributes).expect("failed to create window"));
        #[cfg(target_arch = "wasm32")]
        {
            use winit::platform::web::WindowExtWebSys;
            if let Some(canvas) = window.canvas() {
                let _ = canvas.set_attribute("style", "width:100%;height:100%;display:block;touch-action:none;outline:none");
            }
        }

        let proxy = self.proxy.clone();
        let setup = async move {
            let event = match init(context, window).await {
                Ok(ready) => UserEvent::Ready(Box::new(ready)),
                Err(err) => UserEvent::Failed(err),
            };
            let _ = proxy.send_event(event);
        };
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(setup);
        #[cfg(not(target_arch = "wasm32"))]
        pollster::block_on(setup);
    }

    /// Android takes the window away when the app goes to the background:
    /// drop everything tied to it (it's rebuilt in `resumed`) and go quiet.
    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        if self.state.take().is_some() {
            self.audio.set_paused(true);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Ready(ready) => {
                let (context, state) = *ready;
                #[cfg(target_os = "android")]
                request_high_refresh_rate(&state.window);
                let size = state.window.inner_size();
                self.touch.resize(size.width as f64, size.height as f64);
                self.resolution = Resolution::initial(size.width as f64 * size.height as f64);
                state.window.request_redraw();
                self.context = Some(context);
                self.state = Some(state);
                self.audio.set_paused(false);
                // Don't simulate the time spent in the background.
                self.last_frame = Instant::now();
            }
            UserEvent::Failed(err) => {
                show_error(&err);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let (Some(state), Some(context)) = (&mut self.state, &self.context) else { return };
        // Browsers only allow sound after a key press, click or touch.
        if let WindowEvent::KeyboardInput { .. } | WindowEvent::MouseInput { .. } | WindowEvent::Touch(_) = event {
            self.audio.start();
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state.is_pressed();
                let input = &mut self.game.input;
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::ArrowLeft | KeyCode::KeyA) => input.left = pressed,
                    PhysicalKey::Code(KeyCode::ArrowRight | KeyCode::KeyD) => input.right = pressed,
                    PhysicalKey::Code(KeyCode::ArrowUp | KeyCode::KeyW) => input.up = pressed,
                    PhysicalKey::Code(KeyCode::ArrowDown | KeyCode::KeyS) => input.down = pressed,
                    PhysicalKey::Code(KeyCode::Space | KeyCode::KeyZ) => input.jump = pressed,
                    PhysicalKey::Code(KeyCode::KeyX | KeyCode::KeyJ) => input.tongue = pressed,
                    PhysicalKey::Code(KeyCode::Escape) if !cfg!(target_arch = "wasm32") => event_loop.exit(),
                    PhysicalKey::Code(KeyCode::KeyH) if pressed && !event.repeat => {
                        state.frame.hair_mode = state.frame.hair_mode.next();
                    }
                    PhysicalKey::Code(KeyCode::KeyC) if pressed && !event.repeat => {
                        self.game.weather.cycle_mode();
                    }
                    PhysicalKey::Code(KeyCode::KeyM) if pressed && !event.repeat => self.audio.toggle_mute(),
                    PhysicalKey::Code(KeyCode::KeyF) if pressed && !event.repeat => {
                        self.stats.visible = !self.stats.visible;
                    }
                    PhysicalKey::Code(KeyCode::KeyV) if pressed && !event.repeat => {
                        self.vsync = !self.vsync;
                        let mode = if self.vsync {
                            wgpu::PresentMode::AutoVsync
                        } else {
                            wgpu::PresentMode::AutoNoVsync
                        };
                        context.set_present_mode(&mut state.surface, mode);
                    }
                    _ => {}
                }
            }
            WindowEvent::Touch(touch) => {
                let at = Point::new(touch.location.x, touch.location.y);
                self.touch.event(touch.id, touch.phase, at);
            }
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                context.resize_surface(&mut state.surface, size.width, size.height);
                self.touch.resize(size.width as f64, size.height as f64);
                self.resolution.settle(2.0);
                state.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let (screen_w, screen_h) = (state.surface.config.width, state.surface.config.height);
                let scale = self.resolution.scale;
                let width = ((screen_w as f64 * scale).round() as u32).max(1);
                let height = ((screen_h as f64 * scale).round() as u32).max(1);
                let work_start = Instant::now();
                self.game.pad = Input::merge(self.gamepads.poll(), self.touch.input());
                let now = Instant::now();
                let dt = now.duration_since(self.last_frame).as_secs_f64();
                self.last_frame = now;

                // With the `hotpatch` feature, code changes are patched into
                // these calls while the game runs.
                let game = &mut self.game;
                hot(|| game.update(dt));
                self.audio.feed(game.ambience(), &mut game.sounds);
                let (touch, stats, vsync) = (&self.touch, &self.stats, self.vsync);
                let handle = &context.devices[state.surface.dev_id];
                let rendered = state.frame.render(&handle.device, &handle.queue, width, height, |layers, hair| {
                    let info = hot(|| game.draw(layers, width as f64, height as f64, hair));
                    // Touch controls are laid out in screen pixels.
                    let mut overlay = Scene::new();
                    touch.draw(&mut overlay);
                    layers.front.append(&overlay, Some(Affine::scale(scale)));
                    stats.draw(&mut layers.front, width as f64, height as f64, vsync, scale);
                    info
                });
                let target_view = match rendered {
                    Ok(view) => view,
                    Err(err) => {
                        show_error(&format!("rendering failed: {err}"));
                        event_loop.exit();
                        return;
                    }
                };

                let frame = match state.surface.surface.get_current_texture() {
                    CurrentSurfaceTexture::Success(f) | CurrentSurfaceTexture::Suboptimal(f) => f,
                    CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Lost => {
                        context.configure_surface(&state.surface);
                        state.window.request_redraw();
                        return;
                    }
                    _ => {
                        state.window.request_redraw();
                        return;
                    }
                };
                let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
                let mut encoder = handle
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                state.upscaler.copy(&handle.device, &mut encoder, target_view, &view);
                handle.queue.submit([encoder.finish()]);
                let work = work_start.elapsed().as_secs_f64();
                state.window.pre_present_notify();
                frame.present();

                if let Some(fps) = self.stats.frame(work) {
                    self.resolution.adapt(fps, self.stats.cpu_ms(), self.vsync);
                    state.window.set_title(&format!(
                        "Camel Eon — {fps:.0} FPS (max {:.0})",
                        self.stats.possible_fps()
                    ));
                }
                state.window.request_redraw();
            }
            _ => {}
        }
    }
}

/// Runs `f`, through the hot-patching engine when that's enabled.
fn hot<R>(f: impl FnMut() -> R) -> R {
    #[cfg(all(feature = "hotpatch", not(target_arch = "wasm32")))]
    {
        subsecond::call(f)
    }
    #[cfg(not(all(feature = "hotpatch", not(target_arch = "wasm32"))))]
    {
        let mut f = f;
        f()
    }
}

pub(crate) fn show_error(err: &str) {
    eprintln!("camel-eon: {err}");
    #[cfg(target_arch = "wasm32")]
    if let Some(body) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.body()) {
        let help = if err.starts_with("failed to create") {
            "<p>Camel Eon needs WebGPU, which this browser doesn't have turned on.</p>\
             <p>It works out of the box in Chrome, Edge and Safari on Windows, macOS, Android and iOS.</p>\
             <p>On Linux, Chromium-based browsers (Chrome, Vivaldi, Brave, Edge) need two flags: \
             open <code>chrome://flags</code> (or <code>vivaldi://flags</code>), enable \
             <b>Unsafe WebGPU Support</b> and <b>Vulkan</b>, then restart the browser.</p>"
        } else {
            "<p>Something went wrong. Please report this message:</p>"
        };
        body.set_inner_html(&format!(
            "<div style=\"font:18px sans-serif;padding:2em;color:#3b2a1e;max-width:40em;\
             overflow-wrap:anywhere\">{help}<pre style=\"white-space:pre-wrap\">{err}</pre></div>"
        ));
    }
}

/// Asks Android for 120 Hz. Many phones run apps at 60 Hz unless the app
/// votes for more; the game itself runs at whatever rate vsync gives it.
/// `ANativeWindow_setFrameRate` needs Android 11, so it's looked up at run
/// time and older versions keep their default.
#[cfg(target_os = "android")]
fn request_high_refresh_rate(window: &Window) {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    type SetFrameRate = unsafe extern "C" fn(*mut std::ffi::c_void, f32, i8) -> i32;
    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::AndroidNdk(handle) = handle.as_raw() else { return };
    // SAFETY: dlsym with a NUL-terminated name; the symbol, when present,
    // has the signature above (from <android/native_window.h>), and the
    // window pointer is alive while winit's window is.
    unsafe {
        let symbol = libc::dlsym(libc::RTLD_DEFAULT, c"ANativeWindow_setFrameRate".as_ptr());
        if symbol.is_null() {
            return;
        }
        let set_frame_rate: SetFrameRate = std::mem::transmute(symbol);
        // ANATIVEWINDOW_FRAME_RATE_COMPATIBILITY_DEFAULT
        set_frame_rate(handle.a_native_window.as_ptr(), 120.0, 0);
    }
}

#[cfg(not(target_os = "android"))]
pub fn run() {
    run_with(EventLoop::<UserEvent>::with_user_event());
}

#[cfg(target_os = "android")]
pub fn run_android(app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;
    let mut builder = EventLoop::<UserEvent>::with_user_event();
    builder.with_android_app(app);
    run_with(builder);
}

fn run_with(mut builder: winit::event_loop::EventLoopBuilder<UserEvent>) {
    let event_loop = builder.build().expect("failed to create event loop");
    let app = App {
        context: Some(RenderContext::new()),
        state: None,
        proxy: event_loop.create_proxy(),
        game: Game::new(),
        audio: Audio::new(),
        gamepads: Gamepads::new(),
        touch: TouchControls::default(),
        stats: FrameStats::default(),
        resolution: Resolution::initial(1.0),
        vsync: true,
        last_frame: Instant::now(),
    };
    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        event_loop.spawn_app(app);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut app = app;
        app.audio.start();
        event_loop.run_app(&mut app).expect("event loop failed");
    }
}
