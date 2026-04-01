use std::collections::HashMap;
use super::{CDOperation, GUIEvent};
use super::{EmuStarter, PS1Event, GPUFrameBuffer, Renderer};
use pixels::{wgpu, Pixels, PixelsBuilder, SurfaceTexture};
use std::sync::mpsc;
use std::thread;
use std::time::Instant;
use gilrs::{Event, EventType, Gilrs};
use tracing::{debug, error, info};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};
use crate::core::cdrom::Region;
use crate::core::config::Config;
use crate::core::controllers::ControllerButton;

const DEFAULT_WIDTH: usize = 640;
const DEFAULT_SCALE: usize = 1;

const FPS_PERIOD : f64 = 2.0;

pub struct GPUPixelsRenderer {
    event_proxy: EventLoopProxy<PS1Event>,
}

impl GPUPixelsRenderer {
    pub fn new(event_proxy: EventLoopProxy<PS1Event>) -> Self {
        Self { event_proxy }
    }
}

impl Renderer for GPUPixelsRenderer {
    fn render_frame(&mut self, frame: GPUFrameBuffer,last_performance:u16) {
        let _ = self.event_proxy.send_event(PS1Event::NewFrame(frame, last_performance));
    }
    fn set_warp_mode(&mut self,enabled:bool) {
        let _ = self.event_proxy.send_event(PS1Event::WarpMode(enabled));
    }
    fn set_paused(&mut self,paused:bool) {
        let _ = self.event_proxy.send_event(PS1Event::Paused(paused));
    }
    fn set_last_cd_access(&mut self, access: CDOperation) {
        let _ = self.event_proxy.send_event(PS1Event::CDROMAccess(access));
    }
    fn shutdown(&mut self) {
        let _ = self.event_proxy.send_event(PS1Event::Shutdown);
    }
    fn set_region(&mut self,region:Region) {
        let _ = self.event_proxy.send_event(PS1Event::SetRegion(region));
    }
    fn set_audio_mute(&mut self,mute:bool) {
        let _ = self.event_proxy.send_event(PS1Event::AudioMute(mute));
    }
}

pub fn run_loop(start:EmuStarter<GPUPixelsRenderer>,config:Config) {
    let event_loop = EventLoop::<PS1Event>::with_user_event()
        .build()
        .unwrap();

    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();

    let (gui_event_tx, gui_event_rx) = mpsc::channel::<GUIEvent>();
    let usb_event_tx = gui_event_tx.clone();
    let usb_config = config.clone();
    let emu_config = config.clone();

    thread::spawn(move || start(GPUPixelsRenderer::new(proxy),gui_event_rx,emu_config));
    if config.controllers.controller_1.attach_to_usb || config.controllers.controller_2.attach_to_usb {
        thread::spawn(move || usb_controller_loop(usb_config, usb_event_tx));
    }
    else {
        info!("USB controller loop disabled");
    }

    // start gui
    let mut gui = PixelsRenderer::new(DEFAULT_SCALE, gui_event_tx, config);
    event_loop.run_app(&mut gui).unwrap();
}

struct USBDirections {
    left: bool,
    right: bool,
    up: bool,
    down: bool,
    resolution: f32,
}

impl USBDirections {
    pub fn new(resolution:f32) -> Self {
        Self {
            left: false,
            right: false,
            up: false,
            down: false,
            resolution,
        }
    }
    pub fn left_right_changed(&mut self,value:f32,gui_event_tx:&mpsc::Sender<GUIEvent>,controller_id:usize) {
        let left = value <= -self.resolution;
        let right = value >= self.resolution;

        if self.left != left {
            self.left = left;
            let _ = gui_event_tx.send(GUIEvent::Control(controller_id, ControllerButton::Left, left));
        }
        if self.right != right {
            self.right = right;
            let _ = gui_event_tx.send(GUIEvent::Control(controller_id, ControllerButton::Right, right));
        }
    }
    pub fn up_down_changed(&mut self,value:f32,gui_event_tx:&mpsc::Sender<GUIEvent>,controller_id:usize) {
        let down = value <= -self.resolution;
        let up = value >= self.resolution;

        if self.up != up {
            self.up = up;
            let _ = gui_event_tx.send(GUIEvent::Control(controller_id, ControllerButton::Up, up));
        }
        if self.down != down {
            self.down = down;
            let _ = gui_event_tx.send(GUIEvent::Control(controller_id, ControllerButton::Down, down));
        }
    }
}

fn usb_controller_loop(config:Config,gui_event_tx:mpsc::Sender<GUIEvent>) {
    let mut gilrs = match Gilrs::new() {
        Ok(gilrs) => {
            gilrs
        }
        Err(e) => {
            error!("Gilrs error: {}", e);
            return;
        }
    };

    let usb_buttons_map : HashMap<gilrs::Button,ControllerButton> = HashMap::from([
        (gilrs::Button::South,ControllerButton::Cross),
        (gilrs::Button::North,ControllerButton::Triangle),
        (gilrs::Button::East,ControllerButton::Circle),
        (gilrs::Button::West,ControllerButton::Square),
        (gilrs::Button::LeftTrigger,ControllerButton::L1),
        (gilrs::Button::LeftTrigger2,ControllerButton::L2),
        (gilrs::Button::RightTrigger,ControllerButton::R1),
        (gilrs::Button::RightTrigger2,ControllerButton::R2),
        (gilrs::Button::Start,ControllerButton::Start),
        (gilrs::Button::Select,ControllerButton::Select),
    ]);

    let mut directions = USBDirections::new(config.controllers.usb_direction_resolution);

    info!("Starting USB controller loop ...");

    let dpad_2_axis_button = |button:gilrs::Button| {
        match button {
            gilrs::Button::DPadLeft => Some(ControllerButton::Left),
            gilrs::Button::DPadRight => Some(ControllerButton::Right),
            gilrs::Button::DPadUp => Some(ControllerButton::Up),
            gilrs::Button::DPadDown => Some(ControllerButton::Left),
            _ => None,
        }
    };

    let mut controller_ids = [-1,-1];
    let c1_usb_enabled = config.controllers.controller_1.attach_to_usb;
    let c2_usb_enabled = config.controllers.controller_2.attach_to_usb;

    loop {
        while let Some(Event { id, event, time, .. }) = gilrs.next_event_blocking(Some(std::time::Duration::from_secs(1))) {
            debug!("{:?} New event from {}: {:?}", time, id, event);
            let id : usize = id.into();
            if controller_ids[0] == -1 && c1_usb_enabled && controller_ids[1] != id as i32 {
                controller_ids[0] = id as i32;
                info!("Controller #0 assigned to gamepad #{id}");
            }
            else if controller_ids[1] == -1 && c2_usb_enabled && controller_ids[0] != id as i32 {
                controller_ids[1] = id as i32;
                info!("Controller #1 assigned to gamepad #{id}");
            }
            let controller_id = if controller_ids[0] == id as i32 { 0 } else { 1 };

            match event {
                EventType::Connected => {
                    info!("New gamepad #{} connected", id);
                }
                EventType::Disconnected => {
                    if controller_ids[0] == id as i32 {
                        controller_ids[0] = -1;
                        info!("Gamepad #{} disconnected from controller #0", id);
                    }
                    else if controller_ids[1] == id as i32 {
                        controller_ids[1] = -1;
                        info!("Gamepad #{} disconnected from controller #1", id);
                    }
                }
                EventType::ButtonPressed(button,_code) => {
                    match usb_buttons_map.get(&button) {
                        Some(button) => {
                            let _ = gui_event_tx.send(GUIEvent::Control(controller_id, *button, true));
                        }
                        None => {
                            match dpad_2_axis_button(button) {
                                Some(button) => {
                                    let _ = gui_event_tx.send(GUIEvent::Control(controller_id, button, true));
                                }
                                None => {}
                            }
                        }
                    }
                }
                EventType::ButtonReleased(button,_code) => {
                    match usb_buttons_map.get(&button) {
                        Some(button) => {
                            let _ = gui_event_tx.send(GUIEvent::Control(controller_id, *button, false));
                        }
                        None => {
                            match dpad_2_axis_button(button) {
                                Some(button) => {
                                    let _ = gui_event_tx.send(GUIEvent::Control(controller_id, button, false));
                                }
                                None => {}
                            }
                        }
                    }
                }
                EventType::AxisChanged(axis,value,_code) => {
                    match axis {
                        gilrs::Axis::LeftStickX | gilrs::Axis::RightStickX=> {
                            directions.left_right_changed(value,&gui_event_tx,controller_id);
                        }
                        gilrs::Axis::LeftStickY | gilrs::Axis::RightStickY=> {
                           directions.up_down_changed(value,&gui_event_tx,controller_id);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
}

struct PixelsRenderer {
    window: Option<&'static Window>,
    pixels: Option<Pixels<'static>>,
    width: usize,
    height: usize,
    visible_width: usize,
    visible_height: usize,
    scale: usize,
    fps_last: Instant,
    fps_frames: u32,
    gui_event_tx: mpsc::Sender<GUIEvent>,
    config: Config,
    last_key: bool,
    warp_mode: bool,
    audio_muted:bool,
    paused: bool,
    debug_mode: bool,
    last_performance: u16,
    last_cd_access: Option<CDOperation>,
    region: Region,
    pending_region_change: Option<Region>,
}

impl PixelsRenderer {
    pub fn new(scale: usize,gui_event_tx: mpsc::Sender<GUIEvent>,config: Config) -> Self {
        Self {
            window: None,
            pixels: None,
            width : DEFAULT_WIDTH,
            height : Region::USA.get_crt_total_lines() * 2,
            visible_width: 0,
            visible_height: 0,
            scale,
            fps_last: Instant::now(),
            fps_frames: 0,
            gui_event_tx,
            config,
            last_key: false,
            warp_mode: false,
            audio_muted: false,
            paused: false,
            debug_mode: false,
            last_performance: 0,
            last_cd_access: None,
            region: Region::USA,
            pending_region_change: None,
        }
    }

    fn update_fps(&mut self,update_now:bool) {
        self.fps_frames += 1;
        let duration = self.fps_last.elapsed().as_secs_f64();
        if duration >= FPS_PERIOD || update_now {
            let fps = (self.fps_frames as f64 / duration).ceil();
            if let Some(window) = self.window {
                let cd_info : String = match &self.last_cd_access {
                    Some(access) => {
                        match access {
                            CDOperation::Reading(time) => {
                                format!("[CD R {:02}:{:02}:{:02}]",time.m(),time.s(),time.f())
                            },
                            CDOperation::Playing(time) => {
                                format!("[CD P {:02}:{:02}:{:02}]",time.m(),time.s(),time.f())
                            }
                            CDOperation::Idle => {
                                String::from("")
                            }
                        }
                    }
                    None => String::from("")
                };
                if self.warp_mode || self.paused || self.debug_mode  || self.audio_muted {
                    let mut info = String::new();
                    if self.warp_mode {
                        info.push_str(" (warp mode)");
                    }
                    if self.paused {
                        info.push_str(" (paused)");
                    }
                    if self.debug_mode {
                        info.push_str(" (debug mode)");
                    }
                    if self.audio_muted {
                        info.push_str(" (muted)");
                    }
                    window.set_title(&format!("r-ps1 - ({:?}) FPS: {:02}{info} CPU: {:3}% [{}x{}] {cd_info}",self.region,fps,self.last_performance,self.visible_width,self.visible_height));
                }
                else {
                    window.set_title(&format!("r-ps1 - ({:?}) FPS: {:02} CPU: {:3}% [{}x{}] {cd_info}",self.region,fps,self.last_performance,self.visible_width,self.visible_height));
                }
            }
            self.fps_frames = 0;
            self.fps_last = Instant::now();
        }
    }

    fn new_frame(&mut self, frame: &GPUFrameBuffer,last_performance:u16) {
        self.last_performance = last_performance;
        if let Some(pixels) = &mut self.pixels {
            self.visible_width = frame.visible_width;
            self.visible_height = frame.visible_height;
            if frame.crt_width != self.width || frame.crt_height != self.height || self.pending_region_change.is_some() {
                let mut region_changed = false;
                if let Some(region) = self.pending_region_change.take() && self.region != region {
                    info!("Region set to {:?}",region);
                    self.region = region;
                    region_changed = true;
                }
                info!("PixelsRenderer: Frame size changed from {}x{} to {}x{}", self.width, self.height, frame.visible_width, frame.visible_height);
                self.width = frame.crt_width;
                self.height = frame.crt_height;
                if self.debug_mode != frame.debug_frame || region_changed {
                    self.debug_mode = frame.debug_frame;
                    let new_size = if frame.debug_frame {
                        winit::dpi::PhysicalSize::new(self.width as u32, self.height as u32)
                    }
                    else {
                        winit::dpi::PhysicalSize::new(DEFAULT_WIDTH as u32, (self.region.get_crt_total_lines() * 2) as u32)
                    };
                    let _ = self.window.unwrap().request_inner_size(new_size);
                }

                if pixels.resize_buffer(self.width as u32, self.height as u32).is_err() {
                    println!("Pixels buffer resize error");
                }
                let window_size = self.window.unwrap().inner_size();
                pixels.resize_surface(window_size.width, window_size.height).unwrap();
            }
            let frame_buffer = pixels.frame_mut();
            frame_buffer.copy_from_slice(&frame.frame);

            if pixels.render().is_err() {
                println!("Pixels render error");
            }
            self.update_fps(false);
            self.window.unwrap().request_redraw();
        }
    }
}

impl ApplicationHandler<PS1Event> for PixelsRenderer {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attrs = Window::default_attributes()
            .with_title("r-ps1 - Starting up ...")
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
        let mut builder = PixelsBuilder::new(DEFAULT_WIDTH as u32, (self.region.get_crt_total_lines() * 2) as u32, surface_texture);
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

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: PS1Event) {
        match event {
            PS1Event::NewFrame(frame, last_performance) => {
                self.new_frame(&frame,last_performance);
            }
            PS1Event::WarpMode(on) => {
                self.warp_mode = on;
            }
            PS1Event::Paused(paused) => {
                self.paused = paused;
                self.update_fps(true);
            }
            PS1Event::CDROMAccess(access) => {
                self.last_cd_access = Some(access);
            }
            PS1Event::Shutdown => {
                info!("Shutting down GUI ...");
                event_loop.exit();
            }
            PS1Event::SetRegion(region) => {
                self.pending_region_change = Some(region);
            }
            PS1Event::AudioMute(on) => {
                self.audio_muted = on;
            }
        }
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                let _ = self.gui_event_tx.send(GUIEvent::Shutdown);
            },
            WindowEvent::Resized(new_size) => {
                if let Some(pixels) = &mut self.pixels {
                    if pixels.resize_surface(new_size.width, new_size.height).is_err() {
                        println!("Pixels surface resize error");
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                //self.update_fps(false);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.last_key = event.state.is_pressed();
                if let PhysicalKey::Code(keycode) = event.physical_key {
                    // check warp mode
                    if keycode == KeyCode::F1 && !self.last_key {
                        let _ = self.gui_event_tx.send(GUIEvent::WarpMode);
                        return;
                    }
                    // check pause mode
                    if keycode == KeyCode::Space && !self.last_key {
                        let _ = self.gui_event_tx.send(GUIEvent::Paused);
                        return;
                    }
                    // check vram debug mode
                    if keycode == KeyCode::F2 && !self.last_key {
                        let _ = self.gui_event_tx.send(GUIEvent::VRAMDebugMode);
                        return;
                    }
                    if keycode == KeyCode::F3 && !self.last_key {
                        let _ = self.gui_event_tx.send(GUIEvent::Mute);
                        return;
                    }
                    match self.config.controllers.controller_1.controller_keymap.map_key(keycode) {
                        Some(button) => {
                            let _ = self.gui_event_tx.send(GUIEvent::Control(0,button, self.last_key));
                            //println!("Button {:?} [{}]",button,self.last_key_repeat_state);
                        }
                        None => {
                            match self.config.controllers.controller_2.controller_keymap.map_key(keycode) {
                                Some(button) => {
                                    let _ = self.gui_event_tx.send(GUIEvent::Control(1,button, self.last_key));
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