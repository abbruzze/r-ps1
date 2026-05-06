pub mod pixels;
mod text_renderer;

use crate::core::cdrom::{CDOperation, Region};
use crate::core::config::Config;
use crate::core::controllers::ControllerButton;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{mpsc, Arc};

#[derive(Debug, Clone)]
pub enum PS1Event {
    NewFrame(GPUFrameBuffer,u16),
    SplashScreen,
    WarpMode(bool),
    Paused(bool),
    CDROMAccess(CDOperation),
    Shutdown,
    SetRegion(Region),
    AudioMute(bool),
}

#[derive(Debug, Clone)]
pub enum GUIEvent {
    Controller(usize, ControllerButton, bool),
    WarpMode,
    Paused,
    VRAMDebugMode,
    Shutdown,
    Mute,
    InsertDisc(PathBuf),
    Cheat,
    Reset(bool),
    Ready,
}

/*
GPU Frame Buffer Structure
Holds a frame buffer to be rendered by the GPU renderer.
Each pixel is represented by 4 bytes (RGBA).
 */
#[derive(Debug, Clone)]
pub struct GPUFrameBuffer {
    frame: Arc<Vec<u8>>,
    crt_width: usize,
    crt_height: usize,
    visible_width:usize,
    visible_height:usize,
    debug_frame: bool,
}

impl GPUFrameBuffer {
    pub fn new(frame: Arc<Vec<u8>>, crt_width: usize, crt_height: usize, visible_width:usize,visible_height:usize,debug_frame:bool) -> GPUFrameBuffer {
        GPUFrameBuffer { frame, crt_width, crt_height,visible_width,visible_height,debug_frame }
    }
}

pub type EmuStarter<R> = fn(R,mpsc::Receiver<GUIEvent>,Config);

#[derive(Debug)]
pub struct MouseAccumulator {
    dx: AtomicI32,
    dy: AtomicI32,
    right_button_pressed: AtomicBool,
    left_button_pressed: AtomicBool,
}

impl MouseAccumulator {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            dx: AtomicI32::new(0),
            dy: AtomicI32::new(0),
            right_button_pressed: AtomicBool::new(false),
            left_button_pressed: AtomicBool::new(false),
        })
    }

    pub fn move_delta(&self, dx: i32, dy: i32) {
        self.dx.fetch_add(dx, Ordering::Relaxed);
        self.dy.fetch_add(dy, Ordering::Relaxed);
    }

    pub fn set_right_button_pressed(&self, pressed: bool) {
        self.right_button_pressed.store(pressed, Ordering::Relaxed);
    }
    pub fn set_left_button_pressed(&self, pressed: bool) {
        self.left_button_pressed.store(pressed, Ordering::Relaxed);
    }

    pub fn consume(&self) -> (i32,i32,bool,bool) {
        let x = self.dx.swap(0, Ordering::Relaxed);
        let y = self.dy.swap(0, Ordering::Relaxed);
        let right_button_pressed = self.right_button_pressed.load(Ordering::Relaxed);
        let left_button_pressed = self.left_button_pressed.load(Ordering::Relaxed);
        (x,y,left_button_pressed,right_button_pressed)
    }
}

pub trait Renderer {
    fn set_splash_screen(&mut self);
    fn render_frame(&mut self, frame: GPUFrameBuffer,last_performance:u16);
    fn set_warp_mode(&mut self,enabled:bool);
    fn set_paused(&mut self,paused:bool);
    fn set_last_cd_access(&mut self,access:CDOperation);
    fn shutdown(&mut self);
    fn set_region(&mut self,region:Region);
    fn set_audio_mute(&mut self,mute:bool);
    fn get_mouse_accumulator(&self) -> Arc<MouseAccumulator>;
}