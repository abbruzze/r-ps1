pub mod pixels;

use std::sync::{mpsc, Arc};
use crate::core::cdrom::disc::DiscTime;
use crate::core::controllers::ControllerButton;

#[derive(Debug, Clone)]
pub enum PS1Event {
    NewFrame(GPUFrameBuffer,u8),
    WarpMode(bool),
    Paused(bool),
    CDROMAccess(CDAccess),
}

#[derive(Debug, Clone)]
pub enum GUIEvent {
    Control(usize,ControllerButton,bool),
    WarpMode,
    Paused,
    VRAMDebugMode,
}
#[derive(Debug, Clone)]
pub enum CDOperation {
    Reading,
    Playing,
    Idle,
}

#[derive(Debug, Clone)]
pub struct CDAccess {
    operation: CDOperation,
    position: Option<DiscTime>,
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

pub type EmuStarter<R> = fn(R,mpsc::Receiver<GUIEvent>);

pub trait Renderer {
    fn render_frame(&mut self, frame: GPUFrameBuffer,last_performance:u8);
    fn set_warp_mode(&mut self,enabled:bool);
    fn set_paused(&mut self,paused:bool);
    fn set_last_cd_access(&mut self,access:CDAccess);
}