use crate::audio::cpal::CpalAudioDevice;
use crate::audio::{AudioDevice, AudioSample};
use crate::core::cdrom::{CDOperation, CDRom, Region};
use crate::core::clock::EventType;
use crate::core::clock::{ClockConfig, Event};
use crate::core::config::{Config, RegionPolicyConfig};
use crate::core::cpu::{disassembler, Cpu};
use crate::core::debugger;
use crate::core::debugger::{BreakPoints, DebuggerCommand};
use crate::core::debugger::{DebuggerResponse, RunMode};
use crate::core::dma::{DMAController, DmaDevice, DummyDMAChannel};
use crate::core::gpu::{VideoMode, GPU};
use crate::core::interrupt::IrqHandler;
use crate::core::mdec::{MDec, MDecIn, MDecOut};
use crate::core::memory::bus::Bus;
use crate::core::memory::{ArrayMemory, Memory, ReadMemoryAccess, BIOS_LEN};
use crate::core::spu::{AdpcmInterpolation, Spu};
use crate::log::Logger;
use crate::renderer::{GUIEvent, Renderer};
use build_time::build_time_local;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};
use std::{fs, thread};
use std::path::{Path, PathBuf};
use thread::spawn;
use regex::Regex;
use tracing::{error, info, warn};
use crate::cheats::Cheats;
use crate::core::bios::PS1_BIOS_SET;

pub const EMU_NAME : &str = env!("CARGO_PKG_NAME");
pub const EMU_VERSION : &str = env!("CARGO_PKG_VERSION");
pub const EMU_BUILD_DATE_TIME : &str = build_time_local!("%d/%m/%Y %H:%M:%S");

struct Perf {
    last_timestamp: Instant,
    last_cycles: u64,
    initialized: bool,
    duration: Duration,
}

impl Perf {
    pub fn new(duration: Duration) -> Self {
        Self {
            last_timestamp: Instant::now(),
            last_cycles: 0,
            initialized: false,
            duration,
        }
    }

    pub fn throttle(&mut self,elapsed_cycles:u64,clock_config:&ClockConfig,warp_mode:bool) -> u16 {
        if !self.initialized {
            self.initialized = true;
            self.last_cycles = elapsed_cycles;
            self.last_timestamp = Instant::now();
            return 0;
        }
        let elapsed_micros = self.last_timestamp.elapsed().as_micros() as u64;
        let emulated_micros = (((elapsed_cycles - self.last_cycles) as f64 / clock_config.cpu_hz as f64) * 1_000_000.0) as u64;

        if !warp_mode && emulated_micros > elapsed_micros {
            //println!("Sleeping for {} micros. elapsed={} emulated={}",emulated_micros - elapsed_micros,elapsed_micros,emulated_micros);
            thread::sleep(Duration::from_micros(emulated_micros - elapsed_micros));
        }

        if elapsed_micros > self.duration.as_micros() as u64 {
            self.last_cycles = elapsed_cycles;
            self.last_timestamp = Instant::now();
        }

        ((emulated_micros as f32 / elapsed_micros as f32) * 100.0).ceil() as u16
    }
}

pub struct Emulator {
    cpu: Cpu,
    bus: Bus,
    gpu: Rc<RefCell<GPU>>,
    cdrom: Rc<RefCell<CDRom>>,
    dma: Rc<RefCell<DMAController>>,
    spu: Rc<RefCell<Spu>>,
    audio_device: Option<Box<dyn AudioDevice>>,
    just_entered_in_step_mode: bool,
    last_cycles: usize,
    run_mode: RunMode,
    logger: Logger,
    gui_event_rx: Receiver<GUIEvent>,
    new_frame: bool,
    warp_mode_enabled: bool,
    audio_muted:bool,
    paused: bool,
    debug_vram_mode: bool,
    dma_in_progress:bool,
    perf: Perf,
    last_cd_op: CDOperation,
    config:Config,
    cheats: Cheats,
    cheats_on: bool,
}

impl Emulator {
    pub fn new(config:Config,logger: Logger,renderer:Box<dyn Renderer>,gui_event_rx: Receiver<GUIEvent>) -> Self {
        info!("Loading bios ...");

        let bios = ArrayMemory::load_from_file(config.bios_path.as_deref().unwrap(),BIOS_LEN,true,0,0).unwrap();
        match PS1_BIOS_SET.get(bios.md5.to_lowercase().as_str()) {
            Some(bios_info) => {
                info!("Bios MD5 found. Name: {} region: {:?} release-date: {}",bios_info.redump_name,bios_info.region,bios_info.date);
            }
            None => {
                warn!("Bios MD5 '{}' not found",bios.md5.to_lowercase());
            }
        }
        info!("Bios MD5: {}",bios.md5);

        info!("Building emulator ...");
        let cpu = Cpu::new(&config);

        let mdec = Rc::new(RefCell::new(MDec::new()));
        let mdec_in = Rc::new(RefCell::new(MDecIn::new(&mdec)));
        let mdec_out = Rc::new(RefCell::new(MDecOut::new(&mdec)));
        let gpu = Rc::new(RefCell::new(GPU::new(&config,renderer)));
        let cdrom = Rc::new(RefCell::new(CDRom::new()));
        let spu = Rc::new(RefCell::new(Spu::new(AdpcmInterpolation::default())));
        let pio = Rc::new(RefCell::new(DummyDMAChannel {}));
        let otc = Rc::new(RefCell::new(DummyDMAChannel {}));
        
        let devices = [
            mdec_in as Rc<RefCell<dyn DmaDevice>>,
            mdec_out,
            gpu.clone(),
            cdrom.clone(),
            spu.clone(),
            pio,
            otc
        ];
        
        let dma = Rc::new(RefCell::new(DMAController::new(&devices)));
        let bus = Bus::new(ClockConfig::NTSC,&config,bios,&dma,&gpu,&cdrom,&mdec,&spu);

        let mut emu = Self {
            cpu,bus,
            gpu,
            cdrom,
            dma,
            spu,
            audio_device: None,
            just_entered_in_step_mode: false,
            last_cycles: 0,
            run_mode: RunMode::FreeMode,
            logger,
            gui_event_rx,
            new_frame: false,
            warp_mode_enabled: false,
            audio_muted: false,
            paused: false,
            debug_vram_mode: false,
            dma_in_progress: false,
            perf: Perf::new(Duration::from_millis(4000)),
            last_cd_op: CDOperation::Idle,
            config,
            cheats: Cheats::new(),
            cheats_on: false,
        };

        // cheats
        if emu.config.cheats_config.cheats_enabled {
            info!("Cheats enabled:");
            let cheats = Cheats::parse(&emu.config.cheats_config.cheats_codes);
            for cheat in cheats.cheats.iter() {
                info!("{:?}",cheat);
            }
            
            emu.cheats = cheats;
        }

        emu
    }

    fn load_exe(&self,file_name:&str) -> Result<Vec<u8>,String> {
        match fs::read(file_name) {
            Ok(exe) => {
                let magic = &exe[0..8];
                if magic != b"PS-X EXE" {
                    return Err("Invalid magic number".to_string());
                }
                Ok(exe)
            }
            Err(e) => {
                Err(format!("{:?}",e))
            }
        }
    }

    fn load_disc(&mut self,disc_path:&String,allow_exe:bool) {
        let load_exe_pending = allow_exe && disc_path.to_uppercase().ends_with("EXE");

        if load_exe_pending {
            let exe_path = disc_path.as_str();

            info!("Loading exe '{}', waiting CPU to reach loading point ...",exe_path);

            while self.cpu.get_pc() != 0x80030000 {
                self.cpu.execute_next_instruction(&mut self.bus,false);
            }

            match self.load_exe(exe_path) {
                Ok(exe) => {
                    info!("Loading EXE file {} ...",exe_path);
                    self.bus.load_exe(exe,&mut self.cpu);
                }
                Err(error) => {
                    error!("Error while loading EXE file {} : {}",exe_path,error);
                }
            }
        }
        else {
            info!("Loading disc '{}' ...",disc_path);
            match crate::core::cdrom::disc::Disc::new(&disc_path) {
                Ok(disc) => {
                    let region = match self.config.region_policy {
                        RegionPolicyConfig::Auto => {
                            if disc.get_region().is_none() {
                                warn!("Cannot found region on disc {}, default to USA",disc.get_cue_file_name());
                            }
                            disc.get_region().unwrap_or(Region::USA)
                        },
                        RegionPolicyConfig::Usa => Region::USA,
                        RegionPolicyConfig::Japan => Region::Japan,
                        RegionPolicyConfig::Europe => Region::Europe,
                    };

                    let (clock_config,video_mode) = match region {
                        Region::USA | Region::Japan => (ClockConfig::NTSC,VideoMode::Ntsc),
                        Region::Europe => (ClockConfig::PAL,VideoMode::Pal),
                    };
                    info!("Setting region to {:?} and clock to {:?}",region,clock_config);
                    self.bus.get_clock_mut().set_clock_config(clock_config);
                    self.gpu.borrow_mut().get_renderer_mut().set_region(region);
                    self.gpu.borrow_mut().set_video_mode(video_mode);

                    self.cdrom.borrow_mut().insert_disk(disc);
                    let disc_name = PathBuf::from(disc_path);
                    let name = Path::new(&disc_name)
                        .file_stem()
                        .unwrap()
                        .to_string_lossy()
                        .to_string();
                    let re = Regex::new(r"\([^)]*\)").unwrap();
                    let name_without_parenthesis = re.replace_all(&name, "").to_string();
                    self.gpu.borrow_mut().get_renderer_mut().set_last_cd_access(CDOperation::DiscLoading(name_without_parenthesis));
                }
                Err(e) => {
                    error!("Error while loading disc: {:?}",e);
                }
            }
        }
    }

    pub fn emulate(&mut self) {
        self.cpu.set_bios_tty_capture_enabled(self.config.tty_enabled);

        // send first hblank event
        self.gpu.borrow_mut().send_first_hblank_event(self.bus.get_clock_mut());

        let (loop_tx_cmd, debugger_rx_cmd) = mpsc::channel::<DebuggerResponse>();
        let (debugger_tx_cmd, loop_rx_cmd) = mpsc::channel::<DebuggerCommand>();

        if self.config.debugger_enabled {
            let mut debugger = debugger::Debugger::new(debugger_rx_cmd, debugger_tx_cmd);
            info!("Launching debugger ..");
            spawn(move || debugger.execute() );
        }

        // controllers & memory cards
        self.bus.get_sio0_mut().get_controller_mut(0).set_connected(self.config.controllers.controller_1.controller_enabled);
        self.bus.get_sio0_mut().get_controller_mut(1).set_connected(self.config.controllers.controller_2.controller_enabled);
        if let Some(card_path) = self.config.controllers.controller_1.memory_card_path.as_deref() {
            self.bus.get_sio0_mut().get_controller_mut(0).get_memory_card_mut().set_file_name(String::from(card_path)).unwrap_or_else(|e| error!("Cannot read memory card '{card_path}' for controller 1: {:?}",e));
        }
        if let Some(card_path) = self.config.controllers.controller_2.memory_card_path.as_deref() {
            self.bus.get_sio0_mut().get_controller_mut(1).get_memory_card_mut().set_file_name(String::from(card_path)).unwrap_or_else(|e| error!("Cannot read memory card '{card_path}' for controller 2: {:?}",e));
        }

        let mut irq_handler = IrqHandler::new();

        if let Some(disc_path) = self.config.disc_path.clone() {
            self.load_disc(&disc_path,false);
        }

        self.just_entered_in_step_mode = false;
        self.run_mode = RunMode::FreeMode;

        // starting audio device
        let mut audio_cpal = CpalAudioDevice::new(self.config.audio_config.buffer_capacity_in_millis);
        if let Ok(()) = audio_cpal.start() {
            self.audio_device = Some(Box::new(audio_cpal));
        }
        // schedule first audio event
        self.bus.get_clock_mut().schedule_audio_sample();

        let debugger_enabled = self.config.debugger_enabled;

        'main_loop: loop {
            if self.just_entered_in_step_mode {
                self.send_cpu_info(&loop_tx_cmd);
                self.just_entered_in_step_mode = false;
            }

            while !self.bus.get_clock().has_ready_event() {
                let (send_step,skip_execution) = if debugger_enabled { self.debug(&loop_rx_cmd,&loop_tx_cmd) } else { (false,false) };

                if skip_execution {
                    continue 'main_loop;
                }
                if self.paused {
                    if send_step {
                        self.send_cpu_info(&loop_tx_cmd);
                        continue 'main_loop;
                    }
                    self.check_input();
                    thread::sleep(Duration::from_millis(100));
                }
                self.last_cycles = self.cpu.execute_next_instruction(&mut self.bus,self.dma_in_progress);

                // DMA
                self.dma_in_progress = self.dma.borrow_mut().do_dma_for_cpu_cycles(self.last_cycles, &mut self.bus,&mut irq_handler);
                // IRQs
                irq_handler.forward_to_controller(&mut self.bus);

                if send_step {
                    self.send_cpu_info(&loop_tx_cmd);
                }

                self.bus.get_clock_mut().advance_time(self.last_cycles as u64);
            }

            let events_to_process = self.bus.get_clock_mut().next_events();
            for event in events_to_process {
                self.process_event(event,&mut irq_handler);
                irq_handler.forward_to_controller(&mut self.bus);
            }
        }
    }

    fn process_event(&mut self,event: Event,irq_handler:&mut IrqHandler) {
        match event.event_type {
            EventType::HBlankEnd => {
                self.gpu.borrow_mut().on_hblank_end(event.over_cycles,&mut self.bus);
            }
            EventType::HBlankStart => {
                self.gpu.borrow_mut().on_hblank_start(&mut self.bus,irq_handler,event.over_cycles);
            }
            EventType::RasterLineEnd => {
                self.new_frame = self.gpu.borrow_mut().on_raster_line_end(&mut self.bus, irq_handler, event.over_cycles);

                if self.new_frame {
                    self.check_input();
                    self.gpu.borrow_mut().get_renderer_mut().set_last_cd_access(self.last_cd_op.clone());
                    // cheats
                    if self.config.cheats_config.cheats_enabled && self.cheats_on {
                        self.cheats.apply(&mut self.bus);
                    }
                }
            }
            EventType::Timer0 => {
                let (timer0,clock) = self.bus.get_timer0_and_clock_mut();
                timer0.on_timer_expired(clock,irq_handler);
            }
            EventType::Timer1 => {
                let (timer1,clock) = self.bus.get_timer1_and_clock_mut();
                timer1.on_timer_expired(clock,irq_handler);
            }
            EventType::Timer2 => {
                let (timer2,clock) = self.bus.get_timer2_and_clock_mut();
                timer2.on_timer_expired(clock,irq_handler);
            }
            e@(EventType::SIO0Byte | EventType::SIO0Ack) => {
                let (sio0,clock) = self.bus.get_sio0_and_clock_mut();
                sio0.on_event(e,clock,irq_handler);
            }
            EventType::GPUCommandCompleted => {
                self.gpu.borrow_mut().command_completed(self.bus.get_clock_mut(), irq_handler);
            }
            EventType::Audio44100 => {
                let mut cdrom = self.cdrom.borrow_mut();
                self.last_cd_op = cdrom.clock_44100hz(irq_handler);
                let sample = AudioSample::new_lr(self.spu.borrow_mut().clock(&cdrom,irq_handler));
                if let Some(audio_device) = self.audio_device.as_mut() {
                    if !self.warp_mode_enabled && !self.audio_muted {
                        audio_device.play_sample(sample);
                    }
                    // reschedule event
                    self.bus.get_clock_mut().schedule_audio_sample();
                }
                let clock = self.bus.get_clock();
                let perf = self.perf.throttle(clock.current_time(),clock.get_clock_config(),self.warp_mode_enabled);
                self.gpu.borrow_mut().set_last_cpu_perf(perf);
            }
        }
    }

    fn check_input(&mut self) {
        if let Ok(event) = self.gui_event_rx.try_recv(){
            match event {
                GUIEvent::Control(controller_id,button, pressed) => {
                    //println!("Button {:?} pressed: {}",button,pressed);
                    self.bus.get_sio0_mut().get_controller_mut(controller_id).on_key(button,pressed);
                }
                GUIEvent::WarpMode => {
                    self.warp_mode_enabled ^= true;
                    self.gpu.borrow_mut().get_renderer_mut().set_warp_mode(self.warp_mode_enabled);
                    info!("Throttling enabled: {}",!self.warp_mode_enabled);
                }
                GUIEvent::Mute => {
                    self.audio_muted ^= true;
                    self.gpu.borrow_mut().get_renderer_mut().set_audio_mute(self.audio_muted);
                }
                GUIEvent::Paused => {
                    self.paused ^= true;
                    self.gpu.borrow_mut().get_renderer_mut().set_paused(self.paused);
                }
                GUIEvent::VRAMDebugMode => {
                    self.debug_vram_mode ^= true;
                    self.gpu.borrow_mut().set_show_vram(self.debug_vram_mode);
                }
                GUIEvent::Shutdown => {
                    self.shutdown();
                    self.gpu.borrow_mut().get_renderer_mut().shutdown();
                }
                GUIEvent::InsertDisc(disc_path) => {
                    self.load_disc(&disc_path.to_string_lossy().to_string(),false);
                }
                GUIEvent::Cheat => {
                    self.cheats_on ^= true;
                    info!("Cheating is {}",self.cheats_on);
                }
            }
        }
    }
    
    fn shutdown(&mut self) {
        info!("Shutting down ...");
        
        self.bus.get_sio0_mut().get_controller_mut(0).save();
        self.bus.get_sio0_mut().get_controller_mut(1).save();
    }

    #[inline(always)]
    fn debug(&mut self,loop_rx_cmd:&Receiver<DebuggerCommand>,loop_tx_cmd:&Sender<DebuggerResponse>) -> (bool,bool) {
        match self.run_mode {
            RunMode::StepByStepMode => {
                self.handle_step_by_step_mode(&loop_rx_cmd,&loop_tx_cmd)
            },
            RunMode::BreakMode(ref breaks) => {
                if self.handle_break_mode(&breaks,&loop_tx_cmd) {
                    self.run_mode = RunMode::StepByStepMode;
                    self.just_entered_in_step_mode = true;
                    (false,true)
                }
                else {
                    (false,false)
                }
            },
            RunMode::FreeMode => {
                if self.new_frame || self.paused {
                    while let Ok(cmd) = loop_rx_cmd.try_recv() {
                        match cmd {
                            DebuggerCommand::RunModeChanged(mode) => {
                                self.run_mode = mode;
                                if self.run_mode == RunMode::StepByStepMode {
                                    self.just_entered_in_step_mode = true;
                                    return (true, false)
                                }
                            },
                            DebuggerCommand::Step => {
                                self.just_entered_in_step_mode = true;
                                self.run_mode = RunMode::StepByStepMode;
                                return (true, true)
                            }
                            _ => {
                                todo!("Unimplemented command {:?}", cmd);
                            }
                        }
                    }
                    (false,false)
                }
                else {
                    (false,false)
                }
            }
        }
    }

    fn handle_break_mode(&self,breaks:&BreakPoints,loop_tx_cmd:&Sender<DebuggerResponse>) -> bool {
        let pc = self.cpu.get_pc();
        if let Some(opcode) = breaks.opcode && self.cpu.get_last_opcode() == opcode {
            info!("Break on opcode {:08X} at {:08X}",opcode,pc - 4);
            loop_tx_cmd.send(DebuggerResponse::BreakAt(pc)).unwrap();
            true
        }
        else if breaks.execute.contains(&pc) {
            //*hits += 1;
            //if *hits == 0x100 {
                info!("Break on execute at {:08X}",pc);
                loop_tx_cmd.send(DebuggerResponse::BreakAt(pc)).unwrap();
                true
            // }
            // else {
            //     false
            // }
        }
        else if let Some(break_read_addr) = self.cpu.get_last_mem_read_address() && breaks.read.contains(&break_read_addr) {
            info!("Break on read at {:08X}. Read value {:08X}",break_read_addr,self.cpu.get_last_mem_rw_value());
            loop_tx_cmd.send(DebuggerResponse::BreakAt(break_read_addr)).unwrap();
            true
        }
        else if let Some(break_write_addr) = self.cpu.get_last_mem_write_address() && breaks.write.contains(&break_write_addr) {
            info!("Break on write at {:08X}. Written value {:08X}",break_write_addr,self.cpu.get_last_mem_rw_value());
            loop_tx_cmd.send(DebuggerResponse::BreakAt(break_write_addr)).unwrap();
            true
        }
        else {
            false
        }
    }

    fn handle_step_by_step_mode(&mut self,loop_rx_cmd:&Receiver<DebuggerCommand>,loop_tx_cmd:&Sender<DebuggerResponse>) -> (bool,bool) {
        match loop_rx_cmd.recv().unwrap() {
            DebuggerCommand::Log(level) => {
                self.logger.set_log_level(level.as_str());
                (false,true)
            },
            DebuggerCommand::Step => {
                // ok, step
                (true,false)
            },
            DebuggerCommand::ReqCpuRegs => {
                match self.get_step_info() {
                    Some(info) => {
                        loop_tx_cmd.send(info).unwrap();
                    },
                    None => {}
                }
                (false,true)
            },
            DebuggerCommand::ReqCop0Regs => {
                let regs = self.bus.get_cop0().get_regs().clone();
                loop_tx_cmd.send(DebuggerResponse::Cop0Regs(debugger::Cop0Registers { regs })).unwrap();
                (false,true)
            },
            DebuggerCommand::RunModeChanged(mode) => {
                self.run_mode = mode;
                (false,false)
            },
            DebuggerCommand::ReadMemory(address, length, size) => {
                let step = (size >> 3) as u32;
                let base_address = address;
                let mut address = address;
                let mut buffer = Vec::with_capacity(length);
                for _ in 0..length {
                    let e = match size {
                        8 => self.bus.peek::<8>(address),
                        16 => self.bus.peek::<16>(address),
                        32 => self.bus.peek::<32>(address),
                        _ => None
                    };
                    buffer.push(e.or_else(|| Some(0xFFFFFFFFu32)).unwrap());
                    address += step;
                };
                loop_tx_cmd.send(debugger::DebuggerResponse::Memory(base_address, buffer)).unwrap();
                (false,true)
            },
        }
    }

    fn send_cpu_info(&mut self,loop_tx_cmd: &Sender<DebuggerResponse>) {
        // send initial info
        match self.get_step_info() {
            Some(info) => {
                loop_tx_cmd.send(info).unwrap();
            },
            None => {}
        }
    }

    fn get_step_info(&mut self) -> Option<DebuggerResponse> {
        let pc = self.cpu.get_pc();
        match self.bus.read::<32>(pc,false) {
            ReadMemoryAccess::Read(instruction,_) => {
                let dis = disassembler::disassemble(pc,instruction);
                let regs = self.cpu.get_registers().clone();
                let lo = self.cpu.get_lo();
                let hi = self.cpu.get_hi();
                Some(DebuggerResponse::CpuRegs(dis,debugger::CpuRegisters {pc,regs,lo,hi,dma_in_progress: self.dma_in_progress},self.last_cycles))
            },
            other => {
                error!("Unexpected fetching error {:?}",other);
                None
            }
        }
    }
}