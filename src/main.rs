use std::env;
use tracing::info;
use crate::core::config::Config;
use crate::core::memory::{ArrayMemory, BIOS_LEN};

mod core;
mod log;
pub mod renderer;

fn main() {
    let config = Config::default();
    
    renderer::pixels::run_loop(|renderer, gui_event_rx| {
        let args: Vec<String> = env::args().collect();
        let bios_path = if args.len() > 1 {
            args[1].clone()
        }
        else {
            String::from("/Users/ealeame/OneDrive - Ericsson/Desktop/ps1/SCPH1001.BIN")
        };

        println!("Starting emulator from bios at {}",bios_path);


        let logger = log::Logger::new("info");
        
        info!("Emulator is being started ...");
        let bios = ArrayMemory::load_from_file(bios_path.as_str(),BIOS_LEN,true,0,0).unwrap();
        println!("Bios MD5: {}",bios.md5);
        
        let mut emu = core::emu::Emulator::new(bios, logger,Box::new(renderer),gui_event_rx);
        
        emu.emulate();
    }, config);
}
