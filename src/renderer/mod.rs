pub mod pixels;

use std::sync::{mpsc, Arc};
use crate::core::cdrom::{CDOperation, Region};
use crate::core::config::Config;
use crate::core::controllers::ControllerButton;

#[derive(Debug, Clone)]
pub enum PS1Event {
    NewFrame(GPUFrameBuffer,u8),
    WarpMode(bool),
    Paused(bool),
    CDROMAccess(CDOperation),
    Shutdown,
    SetRegion(Region),
    AudioMute(bool),
}

#[derive(Debug, Clone)]
pub enum GUIEvent {
    Control(usize,ControllerButton,bool),
    WarpMode,
    Paused,
    VRAMDebugMode,
    Shutdown,
    Mute,
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

pub trait Renderer {
    fn render_frame(&mut self, frame: GPUFrameBuffer,last_performance:u8);
    fn set_warp_mode(&mut self,enabled:bool);
    fn set_paused(&mut self,paused:bool);
    fn set_last_cd_access(&mut self,access:CDOperation);
    fn shutdown(&mut self);
    fn set_region(&mut self,region:Region);
    fn set_audio_mute(&mut self,mute:bool);
}