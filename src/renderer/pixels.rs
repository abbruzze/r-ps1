use super::GUIEvent;
use super::{EmuStarter, GPUEvent, GPUFrameBuffer, Renderer};
use pixels::{wgpu, Pixels, PixelsBuilder, SurfaceTexture};
use std::sync::mpsc;
use std::thread;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};
use crate::core::config::Config;

const DEFAULT_WIDTH: usize = 640;
const DEFAULT_HEIGHT: usize = 480;
const DEFAULT_SCALE: usize = 1;

const FPS_PERIOD : f64 = 2.0;

pub struct GPUPixelsRenderer {
    event_proxy: EventLoopProxy<GPUEvent>,
}

impl GPUPixelsRenderer {
    pub fn new(event_proxy: EventLoopProxy<GPUEvent>) -> Self {
        Self { event_proxy }
    }
}

impl Renderer for GPUPixelsRenderer {
    fn render_frame(&mut self, frame: GPUFrameBuffer) {
        let _ = self.event_proxy.send_event(GPUEvent::NewFrame(frame));
    }
    fn set_warp_mode(&mut self,enabled:bool) {
        let _ = self.event_proxy.send_event(GPUEvent::WarpMode(enabled));
    }
    fn set_paused(&mut self,paused:bool) {
        let _ = self.event_proxy.send_event(GPUEvent::Paused(paused));
    }
}

pub fn run_loop(start:EmuStarter<GPUPixelsRenderer>,config:Config) {
    let event_loop = EventLoop::<GPUEvent>::with_user_event()
        .build()
        .unwrap();

    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();

    let (gui_event_tx, gui_event_rx) = mpsc::channel::<GUIEvent>();

    thread::spawn(move || start(GPUPixelsRenderer::new(proxy),gui_event_rx));

    // start gui
    let mut gui = PixelsRenderer::new(DEFAULT_WIDTH, DEFAULT_HEIGHT, DEFAULT_SCALE,gui_event_tx,config);
    event_loop.run_app(&mut gui).unwrap();
}

struct PixelsRenderer {
    window: Option<&'static Window>,
    pixels: Option<Pixels<'static>>,
    width: usize,
    height: usize,
    scale: usize,
    fps_last: Instant,
    fps_frames: u32,
    gui_event_tx: mpsc::Sender<GUIEvent>,
    config: Config,
    last_key_repeat_state: bool,
    warp_mode: bool,
    paused: bool,
}

impl PixelsRenderer {
    pub fn new(width: usize, height: usize, scale: usize,gui_event_tx: mpsc::Sender<GUIEvent>,config: Config) -> Self {
        Self {
            window: None,
            pixels: None,
            width,
            height,
            scale,
            fps_last: Instant::now(),
            fps_frames: 0,
            gui_event_tx,
            config,
            last_key_repeat_state: false,
            warp_mode: false,
            paused: false,
        }
    }

    fn update_fps(&mut self,update_now:bool) {
        self.fps_frames += 1;
        let duration = self.fps_last.elapsed().as_secs_f64();
        if duration >= FPS_PERIOD || update_now {
            let fps = self.fps_frames as f64 / duration;
            if let Some(window) = self.window {
                if self.warp_mode || self.paused {
                    let mut info = String::new();
                    if self.warp_mode {
                        info.push_str(" - warp mode");
                    }
                    if self.paused {
                        info.push_str(" - paused");
                    }
                    window.set_title(&format!("PS1 Emulator - FPS: {:.2}{info} [{}x{}]", fps,self.width,self.height));
                }
                else {
                    window.set_title(&format!("PS1 Emulator - FPS: {:.2} [{}x{}]", fps,self.width,self.height));
                }
            }
            self.fps_frames = 0;
            self.fps_last = Instant::now();
        }
    }

    fn new_frame(&mut self, frame: &GPUFrameBuffer) {
        if let Some(pixels) = &mut self.pixels {
            if frame.crt_width != self.width || frame.crt_height != self.height {
                println!("PixelsRenderer: Frame size changed from {}x{} to {}x{}", self.width, self.height, frame.visible_width, frame.visible_height);
                self.width = frame.crt_width;
                self.height = frame.crt_height;
                //let new_size = winit::dpi::PhysicalSize::new(self.width as u32, self.height as u32);
                //let _ = self.window.unwrap().request_inner_size(new_size);
                if pixels.resize_buffer(self.width as u32, self.height as u32).is_err() {
                    println!("Pixels buffer resize error");
                }
            }
            let frame_buffer = pixels.frame_mut();
            frame_buffer.copy_from_slice(&frame.frame);

            if pixels.render().is_err() {
                println!("Pixels render error");
            }
            self.window.unwrap().request_redraw();
        }
    }
}

impl ApplicationHandler<GPUEvent> for PixelsRenderer {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attrs = Window::default_attributes()
            .with_title("PS1 Emulator - ApplicationHandler Demo")
            .with_inner_size(winit::dpi::LogicalSize::new(
                (self.width * self.scale) as u32,
                (self.height * self.scale) as u32,
            ))
            .with_resizable(true);

        let window = event_loop.create_window(window_attrs).unwrap();
        let window_ref: &'static Window = Box::leak(Box::new(window));

        // Crea pixels
        let window_size = window_ref.inner_size();
        let surface_texture = SurfaceTexture::new(window_size.width, window_size.height,window_ref);
        let mut builder = PixelsBuilder::new(DEFAULT_WIDTH as u32, DEFAULT_HEIGHT as u32, surface_texture);
        builder = builder.request_adapter_options(wgpu::RequestAdapterOptions {
            // 1 - GPU: Pick one or the other
            //power_preference: wgpu::PowerPreference::LowPower,
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        });
        let mut pixels = builder.build().expect("create pixels");
        pixels.set_present_mode(wgpu::PresentMode::Immediate); // can be changed to Fifo for VSync

        self.window = Some(window_ref);
        self.pixels = Some(pixels);

        // Init FPS timer
        self.fps_last = Instant::now();
        self.fps_frames = 0;

        window_ref.request_redraw();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: GPUEvent) {
        match event {
            GPUEvent::NewFrame(frame) => {
                self.new_frame(&frame);
            }
            GPUEvent::WarpMode(on) => {
                self.warp_mode = on;
            }
            GPUEvent::Paused(paused) => {
                self.paused = paused;
                self.update_fps(true);
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(new_size) => {
                if let Some(pixels) = &mut self.pixels {
                    if pixels.resize_surface(new_size.width, new_size.height).is_err() {
                        println!("Pixels surface resize error");
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                self.update_fps(false);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // if event.repeat {
                //     self.last_key_repeat_state ^= true;
                // }
                // else {
                //     self.last_key_repeat_state = event.state.is_pressed();
                // }
                self.last_key_repeat_state = event.state.is_pressed();
                if let PhysicalKey::Code(keycode) = event.physical_key {
                    // check warp mode
                    if keycode == KeyCode::F1 && !self.last_key_repeat_state {
                        let _ = self.gui_event_tx.send(GUIEvent::WarpMode);
                        return;
                    }
                    // check pause mode
                    if keycode == KeyCode::Space && !self.last_key_repeat_state {
                        let _ = self.gui_event_tx.send(GUIEvent::Paused);
                        return;
                    }
                    match self.config.controller_1_config.map_key(keycode) {
                        Some(button) => {
                            let _ = self.gui_event_tx.send(GUIEvent::Control(0,button, self.last_key_repeat_state));
                            //println!("Button {:?} [{}]",button,self.last_key_repeat_state);
                        }
                        None => {
                            match self.config.controller_2_config.map_key(keycode) {
                                Some(button) => {
                                    let _ = self.gui_event_tx.send(GUIEvent::Control(1,button, self.last_key_repeat_state));
                                }
                                None => {}
                            }
                        }
                    }
                }
            }
                _ => {}
        }
    }
}