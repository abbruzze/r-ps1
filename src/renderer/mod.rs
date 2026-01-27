pub mod pixels;

use std::sync::{mpsc, Arc};
use crate::core::controllers::ControllerButton;

#[derive(Debug, Clone)]
pub enum GPUEvent {
    NewFrame(GPUFrameBuffer),
    WarpMode(bool),
    Paused(bool),
}

#[derive(Debug, Clone)]
pub enum GUIEvent {
    Control(usize,ControllerButton,bool),
    WarpMode,
    Paused,
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
    visible_height:usize
}

impl GPUFrameBuffer {
    pub fn new(frame: Arc<Vec<u8>>, crt_width: usize, crt_height: usize, visible_width:usize,visible_height:usize) -> GPUFrameBuffer {
        GPUFrameBuffer { frame, crt_width, crt_height,visible_width,visible_height }
    }
}

pub type EmuStarter<R> = fn(R,mpsc::Receiver<GUIEvent>);

pub trait Renderer {
    fn render_frame(&mut self, frame: GPUFrameBuffer);
    fn set_warp_mode(&mut self,enabled:bool);
    fn set_paused(&mut self,paused:bool);
}