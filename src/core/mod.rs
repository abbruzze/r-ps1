pub mod cpu;
pub mod memory;
pub mod debugger;
pub mod emu;
pub mod timer;
pub mod dma;
pub mod gpu;
mod interrupt;
pub mod controllers;
pub mod config;
mod sio;
pub mod clock;
pub mod cdrom;
mod mdec;

// CPU Clock   =  33.868800MHz (44100Hz*300h)
pub const CPU_CLOCK : usize = 33_868_800;