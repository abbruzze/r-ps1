use crate::core::clock::EventType;
use crate::core::clock::{ClockConfig, Event};
use crate::core::cpu::{disassembler, Cpu};
use crate::core::debugger;
use crate::core::debugger::{BreakPoints, DebuggerCommand};
use crate::core::debugger::{DebuggerResponse, RunMode};
use crate::core::dma::{DMAController, DmaDevice, DummyDMAChannel};
use crate::core::gpu::GPU;
use crate::core::interrupt::IrqHandler;
use crate::core::memory::bus::Bus;
use crate::core::memory::{ArrayMemory, Memory, ReadMemoryAccess};
use crate::log::Logger;
use crate::renderer::{GUIEvent, Renderer};
use std::cell::RefCell;
use std::process::exit;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};
use std::{fs, thread};
use thread::spawn;
use tracing::{error, info};

const THROTTLE_RES : u64 = 100;
const THROTTLE_ADJ_FACTOR : f32 = 1.8;

pub struct Emulator {
    cpu: Cpu,
    bus: Bus,
    gpu: Rc<RefCell<GPU>>,
    dma: Rc<RefCell<DMAController>>,
    just_entered_in_step_mode: bool,
    last_cycles: usize,
    run_mode: RunMode,
    logger: Logger,
    gui_event_rx: Receiver<GUIEvent>,
    new_frame: bool,
    warp_mode_enabled: bool,
    paused: bool,
    last_throttle_timestamp: Instant,
}

impl Emulator {
    pub fn new(bios:ArrayMemory,logger: Logger,renderer:Box<dyn Renderer>,gui_event_rx: Receiver<GUIEvent>) -> Self {
        info!("Building emulator ...");
        let cpu = Cpu::new();
        
        let mdec_in = Rc::new(RefCell::new(DummyDMAChannel {}));
        let mdec_out = Rc::new(RefCell::new(DummyDMAChannel {}));
        let gpu = Rc::new(RefCell::new(GPU::new(renderer)));
        let cdrom = Rc::new(RefCell::new(DummyDMAChannel {}));
        let spu = Rc::new(RefCell::new(DummyDMAChannel {}));
        let pio = Rc::new(RefCell::new(DummyDMAChannel {}));
        let otc = Rc::new(RefCell::new(DummyDMAChannel {}));
        
        let devices = [mdec_in,mdec_out,gpu.clone() as Rc<RefCell<dyn DmaDevice>>,cdrom,spu,pio,otc];
        
        let dma = Rc::new(RefCell::new(DMAController::new(&devices)));
        let bus = Bus::new(ClockConfig::NTSC,bios,&dma,&gpu);

        let emu = Self {
            cpu,bus,
            gpu,
            dma,
            just_entered_in_step_mode: false,
            last_cycles: 0,
            run_mode: RunMode::FreeMode,
            logger,
            gui_event_rx,
            new_frame: false,
            warp_mode_enabled: false,
            paused: false,
            last_throttle_timestamp: Instant::now(),
        };

        emu
    }

    fn load_exe(&self,file_name:&String) -> Result<Vec<u8>,String> {
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

    pub fn emulate(&mut self) {
        self.cpu.set_bios_tty_capture_enabled(true);

        // send first hblank event
        self.gpu.borrow_mut().send_first_hblank_event(self.bus.get_clock_mut());
        // send first throttle event
        self.reschedule_throttling();

        let (loop_tx_cmd, debugger_rx_cmd) = mpsc::channel::<DebuggerResponse>();
        let (debugger_tx_cmd, loop_rx_cmd) = mpsc::channel::<DebuggerCommand>();

        let mut debugger = debugger::Debugger::new(debugger_rx_cmd,debugger_tx_cmd);
        info!("Launching debugger ..");
        spawn(move || {
            debugger.execute();
        });

        let mut irq_handler = IrqHandler::new();

        const LOAD_EXE_PENDING: bool = true;
        let exe_path = String::from("C:\\Users\\ealeame\\OneDrive - Ericsson\\Desktop\\ps1\\resolution.exe");

        self.just_entered_in_step_mode = false;
        self.run_mode = RunMode::FreeMode;
        let mut dma_in_progress = false;

        if LOAD_EXE_PENDING {
            info!("Waiting to reach EXE loading point ...");

            while self.cpu.get_pc() != 0x80030000 {
                self.cpu.execute_next_instruction(&mut self.bus) as u64;
            }

            match self.load_exe(&exe_path) {
                Ok(exe) => {
                    info!("Loading EXE file {} ...",exe_path);
                    self.bus.load_exe(exe,&mut self.cpu);
                }
                Err(error) => {
                    error!("Error while loading EXE file {} : {}",exe_path,error);
                    exit(1);
                }
            }
        }

        'main_loop: loop {
            if self.just_entered_in_step_mode {
                self.send_cpu_info(&loop_tx_cmd);
                self.just_entered_in_step_mode = false;
            }

            while !self.bus.get_clock().has_ready_event() {
                let (send_step,skip_execution) = self.debug(&loop_rx_cmd,&loop_tx_cmd);
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
                self.last_cycles = self.cpu.execute_next_instruction(&mut self.bus);
                self.bus.get_clock_mut().advance_time(self.last_cycles as u64);

                if send_step {
                    self.send_cpu_info(&loop_tx_cmd);
                    continue 'main_loop;
                }
                // SIO0
                //self.bus.get_sio0_mut().tick(self.last_cycles, &mut irq_handler);
                // DMA
                dma_in_progress = self.dma.borrow_mut().do_dma_for_cpu_cycles(self.last_cycles, &mut self.bus, &mut irq_handler);
                // IRQs
                irq_handler.forward_to_controller(&mut self.bus);
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
            EventType::SIO0 => {
                let (sio0,clock) = self.bus.get_sio0_and_clock_mut();
                sio0.on_tx_transmitted(clock,irq_handler);
            }
            EventType::DoThrottle => {
                if !self.warp_mode_enabled {
                    let elapsed_micros = self.last_throttle_timestamp.elapsed().as_micros() as u64;
                    self.reschedule_throttling();
                    const EXPECTED_MICROS: u64 = 1_000_000 / THROTTLE_RES;

                    if elapsed_micros < EXPECTED_MICROS {
                        thread::sleep(Duration::from_micros(((EXPECTED_MICROS as f32 - elapsed_micros as f32) * THROTTLE_ADJ_FACTOR) as u64));
                    }
                }
            }
        }
    }

    fn reschedule_throttling(&mut self) {
        self.last_throttle_timestamp = Instant::now();
        let cpu_clock = self.bus.get_clock().get_clock_config().cpu_hz;
        self.bus.get_clock_mut().schedule(EventType::DoThrottle, cpu_clock / THROTTLE_RES);
    }

    fn cancel_throttling(&mut self) {
        self.bus.get_clock_mut().cancel(EventType::DoThrottle);
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
                    self.bus.get_clock_mut().cancel(EventType::DoThrottle);
                    if !self.warp_mode_enabled {
                        self.reschedule_throttling();
                    }
                    self.gpu.borrow_mut().get_renderer_mut().set_warp_mode(self.warp_mode_enabled);
                    info!("Throttling enabled: {}",!self.warp_mode_enabled);
                }
                GUIEvent::Paused => {
                    self.paused ^= true;
                    self.gpu.borrow_mut().get_renderer_mut().set_paused(self.paused);
                    if !self.paused {
                        self.reschedule_throttling();
                    }
                    else {
                        self.cancel_throttling();
                    }
                }
            }
        }
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
                Some(DebuggerResponse::CpuRegs(dis,debugger::CpuRegisters {pc,regs,lo,hi},self.last_cycles))
            },
            other => {
                error!("Unexpected fetching error {:?}",other);
                None
            }
        }
    }
}