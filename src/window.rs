//! Window frontend: renders straight to a window via winit and wgpu.

use std::sync::Arc;
use std::time::Instant;

use vello::peniko::Color;
use vello::util::{RenderContext, RenderSurface};
use vello::wgpu::{self, CurrentSurfaceTexture};
use vello::{AaConfig, Renderer, RendererOptions, Scene};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::game::Game;
use crate::gamepad::Gamepads;

struct RenderState {
    surface: RenderSurface<'static>,
    window: Arc<Window>,
    renderer: Renderer,
}

struct App {
    context: RenderContext,
    state: Option<RenderState>,
    scene: Scene,
    game: Game,
    gamepads: Gamepads,
    last_frame: Instant,
    fps_timer: Instant,
    frames: u32,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("CamelJon"))
                .expect("failed to create window"),
        );
        let size = window.inner_size();
        let surface = pollster::block_on(self.context.create_surface(
            window.clone(),
            size.width.max(1),
            size.height.max(1),
            wgpu::PresentMode::AutoVsync,
        ))
        .expect("failed to create surface");
        let device = &self.context.devices[surface.dev_id].device;
        let renderer = Renderer::new(
            device,
            RendererOptions {
                antialiasing_support: vello::AaSupport::area_only(),
                ..Default::default()
            },
        )
        .expect("failed to create renderer");
        self.state = Some(RenderState {
            surface,
            window,
            renderer,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = &mut self.state else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state.is_pressed();
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::ArrowLeft | KeyCode::KeyA) => {
                        self.game.input.left = pressed;
                    }
                    PhysicalKey::Code(KeyCode::ArrowRight | KeyCode::KeyD) => {
                        self.game.input.right = pressed;
                    }
                    PhysicalKey::Code(KeyCode::ArrowUp | KeyCode::KeyW) => self.game.input.up = pressed,
                    PhysicalKey::Code(KeyCode::ArrowDown | KeyCode::KeyS) => self.game.input.down = pressed,
                    PhysicalKey::Code(KeyCode::Space | KeyCode::KeyZ) => self.game.input.jump = pressed,
                    PhysicalKey::Code(KeyCode::KeyX | KeyCode::KeyJ) => self.game.input.tongue = pressed,
                    PhysicalKey::Code(KeyCode::Escape) => event_loop.exit(),
                    _ => {}
                }
            }
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                self.context
                    .resize_surface(&mut state.surface, size.width, size.height);
                state.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let width = state.surface.config.width;
                let height = state.surface.config.height;
                self.game.pad = self.gamepads.poll();
                let now = Instant::now();
                self.game
                    .update(now.duration_since(self.last_frame).as_secs_f64());
                self.last_frame = now;

                self.scene.reset();
                self.game.draw(&mut self.scene, width as f64, height as f64);

                let handle = &self.context.devices[state.surface.dev_id];
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
                        self.context.configure_surface(&state.surface);
                        state.window.request_redraw();
                        return;
                    }
                    _ => {
                        state.window.request_redraw();
                        return;
                    }
                };
                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                let mut encoder = handle
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                state.surface.blitter.copy(
                    &handle.device,
                    &mut encoder,
                    &state.surface.target_view,
                    &view,
                );
                handle.queue.submit([encoder.finish()]);
                state.window.pre_present_notify();
                frame.present();

                self.frames += 1;
                let elapsed = self.fps_timer.elapsed().as_secs_f64();
                if elapsed >= 1.0 {
                    let fps = self.frames as f64 / elapsed;
                    state.window.set_title(&format!("CamelJon — {fps:.0} FPS"));
                    self.frames = 0;
                    self.fps_timer = Instant::now();
                }
                state.window.request_redraw();
            }
            _ => {}
        }
    }
}

pub fn run() {
    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut app = App {
        context: RenderContext::new(),
        state: None,
        scene: Scene::new(),
        game: Game::new(),
        gamepads: Gamepads::new(),
        last_frame: Instant::now(),
        fps_timer: Instant::now(),
        frames: 0,
    };
    event_loop.run_app(&mut app).expect("event loop failed");
}
