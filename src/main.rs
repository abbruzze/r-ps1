use std::env;
use std::path::{Path, PathBuf};
use std::process::exit;
use clap::{Parser, ValueEnum};
use tracing::info;
use crate::core::config::{Config, RegionPolicyConfig};
use crate::core::emu::{EMU_BUILD_DATE_TIME, EMU_NAME, EMU_VERSION};

mod core;
mod log;
mod renderer;
mod audio;
mod cheats;

#[derive(Parser)]
#[command(version, about = "Rust Playstation 1 emulator", long_about = None)]
struct Args {
    /// Path to bios file
    #[arg(long, value_name = "FILE")]
    bios: Option<PathBuf>,
    /// Path to disc image file or EXE file
    #[arg(long, value_name = "FILE")]
    disc: Option<PathBuf>,
    /// Path configuration file
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Region
    #[arg(long, value_name = "REGION")]
    region: Option<ArgRegion>,
    /// Debugger enabled
    #[arg(long)]
    debugger: bool,
    /// Log level
    #[arg(long, value_name = "LEVEL")]
    log_level: Option<LogLevel>,
    /// Log file
    #[arg(long, value_name = "FILE")]
    log_file: Option<PathBuf>,
    /// Full screen enabled
    #[arg(long)]
    full_screen: bool,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Debug)]
enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum ArgRegion {
    /// America
    Usa,
    /// Europe
    Europe,
    /// Japan
    Japan,
    /// Automatic: the region will be the same of the disc
    Auto,
}

const DEFAULT_CONFIG_FILE_NAME : &str = "config.yaml";

fn main() {
    let emu_exe = env::current_exe().unwrap();
    let emu_dir = emu_exe.parent().unwrap();

    let args = Args::parse();
    let mut config = if let Some(config_path) = args.config {
        println!("Loading config file '{}' ...",config_path.display());
        Config::load_or_default(&config_path)
    }
    else {
        // check if there's a config file in the same directory
        let config_file = emu_dir.join(DEFAULT_CONFIG_FILE_NAME);
        if config_file.exists() {
            println!("Try default loading config file '{}'",config_file.display());
            Config::load_or_default(&config_file)
        }
        else {
            println!("No config file found, using default config");
            Config::default()
        }
    };

    // bios
    if args.bios.is_none() && config.bios_path.is_none() {
        println!("No bios file specified");
        exit(1);
    }
    if let Some(bios_path) = args.bios {
        // override bios path in config file
        config.bios_path = Some(String::from(bios_path.to_str().unwrap()));
    }
    // disc
    if let Some(disc_path) = args.disc {
        // override disc path in config file
        config.disc_path = Some(String::from(disc_path.to_str().unwrap()));
    }
    // region
    if let Some(region) = args.region {
        let region = match region {
            ArgRegion::Usa => RegionPolicyConfig::Usa,
            ArgRegion::Europe => RegionPolicyConfig::Europe,
            ArgRegion::Japan => RegionPolicyConfig::Japan,
            ArgRegion::Auto => RegionPolicyConfig::Auto,
        };
        config.region_policy = region;
    }
    // debugger
    if args.debugger {
        config.debugger_enabled = true;
    }
    // full screen
    if args.full_screen {
        config.gpu_config.start_full_screen = true;
    }

    let bios_path = Path::new(config.bios_path.as_deref().unwrap());
    if !bios_path.exists() {
        println!("Bios file '{}' not found",bios_path.display());
        exit(1);
    }
    
    // log
    if let Some(log_level) = args.log_level {
        config.log_config.log_severity = format!("{:?}",log_level);
    }
    if let Some(log_file) = args.log_file {
        config.log_config.log_file = Some(log_file);
    }

    if config.file_config.is_none() {
        let config_file = emu_dir.join(DEFAULT_CONFIG_FILE_NAME);
        match config.save(&config_file) {
            Ok(_) => println!("Config file saved to '{}'",config_file.display()),
            Err(e) => println!("Error saving config file: {}",e),
        }
    }
    
    renderer::pixels::run_loop(|renderer, gui_event_rx, config| {
        let logger = log::Logger::new(config.log_config.log_file.clone(),config.log_config.log_severity.clone());


        info!("Welcome to {} v{} compiled on {}",EMU_NAME,EMU_VERSION,EMU_BUILD_DATE_TIME);
        info!("Starting emulator from bios at {}",config.bios_path.as_ref().unwrap());
        
        let mut emu = core::emu::Emulator::new(config,logger,Box::new(renderer),gui_event_rx);
        
        emu.emulate();
    }, config);
}
