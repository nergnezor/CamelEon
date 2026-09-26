//! Window frontend: renders straight to a window via winit and wgpu. On the
//! web the "window" is a full-page canvas.

use std::sync::Arc;

use vello::kurbo::Point;
use vello::peniko::Color;
use vello::util::{RenderContext, RenderSurface};
use vello::wgpu::{self, CurrentSurfaceTexture};
use vello::{AaConfig, Renderer, RendererOptions, Scene};
use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::game::{Game, Input};
use crate::gamepad::Gamepads;
use crate::stats::FrameStats;
use crate::touch::TouchControls;

struct RenderState {
    surface: RenderSurface<'static>,
    window: Arc<Window>,
    renderer: Renderer,
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
    scene: Scene,
    game: Game,
    gamepads: Gamepads,
    touch: TouchControls,
    stats: FrameStats,
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
    let renderer = Renderer::new(
        device,
        RendererOptions {
            antialiasing_support: vello::AaSupport::area_only(),
            ..Default::default()
        },
    )
    .map_err(|e| format!("failed to create renderer: {e}"))?;
    Ok((context, RenderState { surface, window, renderer }))
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Some(context) = self.context.take() else { return };
        #[allow(unused_mut)]
        let mut attributes = Window::default_attributes().with_title("CamelJon");
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

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Ready(ready) => {
                let (context, state) = *ready;
                let size = state.window.inner_size();
                self.touch.resize(size.width as f64, size.height as f64);
                state.window.request_redraw();
                self.context = Some(context);
                self.state = Some(state);
            }
            UserEvent::Failed(err) => {
                show_error(&err);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let (Some(state), Some(context)) = (&mut self.state, &self.context) else { return };
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
                state.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let width = state.surface.config.width;
                let height = state.surface.config.height;
                let work_start = Instant::now();
                self.game.pad = Input::merge(self.gamepads.poll(), self.touch.input());
                let now = Instant::now();
                let dt = now.duration_since(self.last_frame).as_secs_f64();
                self.last_frame = now;

                // With the `hotpatch` feature, code changes are patched into
                // these calls while the game runs.
                let (game, scene) = (&mut self.game, &mut self.scene);
                hot(|| game.update(dt));
                scene.reset();
                hot(|| game.draw(scene, width as f64, height as f64));
                self.touch.draw(scene);
                self.stats.draw(scene, width as f64, height as f64, self.vsync);

                let handle = &context.devices[state.surface.dev_id];
                state
                    .renderer
                    .render_to_texture(
                        &handle.device,
                        &handle.queue,
                        &self.scene,
                        &state.surface.target_view,
                        &vello::RenderParams {
                            base_color: Color::from_rgb8(0x10, 0x12, 0x1c),
                            width,
                            height,
                            antialiasing_method: AaConfig::Area,
                        },
                    )
                    .expect("rendering failed");

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
                state
                    .surface
                    .blitter
                    .copy(&handle.device, &mut encoder, &state.surface.target_view, &view);
                handle.queue.submit([encoder.finish()]);
                let work = work_start.elapsed().as_secs_f64();
                state.window.pre_present_notify();
                frame.present();

                if let Some(fps) = self.stats.frame(work) {
                    state.window.set_title(&format!(
                        "CamelJon — {fps:.0} FPS (max {:.0})",
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

fn show_error(err: &str) {
    eprintln!("cameljon: {err}");
    #[cfg(target_arch = "wasm32")]
    if let Some(body) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.body()) {
        body.set_inner_html(&format!(
            "<div style=\"font:18px sans-serif;padding:2em;color:#3b2a1e;max-width:40em\">\
             <p>Camel Joe needs WebGPU, which this browser doesn't have turned on.</p>\
             <p>It works out of the box in Chrome, Edge and Safari on Windows, macOS, Android and iOS.</p>\
             <p>On Linux, Chromium-based browsers (Chrome, Vivaldi, Brave, Edge) need two flags: \
             open <code>chrome://flags</code> (or <code>vivaldi://flags</code>), enable \
             <b>Unsafe WebGPU Support</b> and <b>Vulkan</b>, then restart the browser.</p>\
             <p><small>{err}</small></p></div>"
        ));
    }
}

pub fn run() {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("failed to create event loop");
    let app = App {
        context: Some(RenderContext::new()),
        state: None,
        proxy: event_loop.create_proxy(),
        scene: Scene::new(),
        game: Game::new(),
        gamepads: Gamepads::new(),
        touch: TouchControls::default(),
        stats: FrameStats::default(),
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
        event_loop.run_app(&mut app).expect("event loop failed");
    }
}
