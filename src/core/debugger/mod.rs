use std::collections::HashSet;
use std::io;
use std::io::Write;
use std::sync::mpsc::{Receiver, Sender};
use tracing::{error, info};
use crate::core::cpu::disassembler::Disassembled;

const DUMP_MEMORY_COLUMNS : usize = 16;

#[derive(Debug,PartialEq)]
pub enum RunMode {
    FreeMode,
    StepByStepMode,
    BreakMode(BreakPoints),
}

#[derive(Debug,PartialEq,Clone)]
pub struct BreakPoints {
    pub execute: HashSet<u32>,
    pub read: HashSet<u32>,
    pub write: HashSet<u32>,
    pub opcode: Option<u32>,
}

impl BreakPoints {
    pub fn new() -> Self {
        Self {
            execute: HashSet::new(),
            read: HashSet::new(),
            write: HashSet::new(),
            opcode: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.execute.is_empty() && self.read.is_empty() && self.write.is_empty() && self.opcode.is_none()
    }
}

#[derive(Debug)]
pub struct CpuRegisters {
    pub pc : u32,
    pub regs: [u32;32],
    pub lo: u32,
    pub hi: u32,
    pub dma_in_progress: bool,
}
impl CpuRegisters {
    pub fn dump(&self) -> String {
        let mut dump = String::from("");
        dump.push_str(&format!("PC={:08X} LO={:08X} HI={:08X} DMA={}\n",self.pc,self.lo,self.hi,self.dma_in_progress));
        for r in 0..32 {
            dump.push_str(&format!("{:5}={:08X} ", crate::core::cpu::disassembler::register_alias(r), self.regs[r]));
            if (r & 7) == 7 {
                dump.push('\n');
            }
        }
        dump
    }
}
#[derive(Debug)]
pub struct Cop0Registers {
    pub regs: [u32;32],
}

impl Cop0Registers {
    pub fn dump(&self) -> String {
        let mut dump = String::from("");
        for r in 0..32 {
            dump.push_str(&format!("{:14}={:08X} ", crate::core::cpu::disassembler::cop_register_alias(0,r,false), self.regs[r]));
            if (r & 7) == 7 {
                dump.push('\n');
            }
        }
        dump
    }
}
#[derive(Debug)]
pub enum DebuggerCommand {
    RunModeChanged(RunMode),
    Step,
    ReqCop0Regs,
    ReqCpuRegs,
    ReadMemory(u32,usize,usize), // address, size, 8/16/32,
    Log(String),
}
#[derive(Debug)]
pub enum DebuggerResponse {
    CpuRegs(Disassembled,CpuRegisters,usize),
    Cop0Regs(Cop0Registers),
    Memory(u32,Vec<u32>),
    BreakAt(u32), // address,
}

pub struct Debugger {
    receiver: Receiver<DebuggerResponse>,
    sender: Sender<DebuggerCommand>,
    break_points: BreakPoints,
    step_by_step_mode: bool,
}

impl Debugger {
    pub fn new(receiver: Receiver<DebuggerResponse>, sender: Sender<DebuggerCommand>) -> Self {
        Self { receiver, sender, break_points: BreakPoints::new(), step_by_step_mode: true }
    }

    fn wait_resp(&mut self,wait:bool) {
        if wait {
            if let Ok(resp) = self.receiver.recv() {
                self.handle_response(resp, "r");
            }
        }
        else {
            while let Ok(resp) = self.receiver.try_recv() {
                self.handle_response(resp, "r");
            }
        }
    }

    pub fn execute(&mut self) {
        info!("Debugger is in step by step mode: {}",self.step_by_step_mode);
        loop {
            self.wait_resp(false);
            //if self.step_by_step_mode {
                let mut input = String::new();
                print!(">");
                io::stdout().flush().unwrap();
                io::stdin().read_line(&mut input).unwrap();
                let mut command_iter = input.trim().split_ascii_whitespace();
                let cmd = command_iter.next().or_else(|| Some("")).unwrap();

                match cmd {
                    "log" => {
                        let args = command_iter.collect::<Vec<&str>>();
                        if args.len() != 1 {
                            error!("Wrong number of arguments for 'log' command: expected <level>");
                        }
                        else {
                            self.sender.send(DebuggerCommand::Log(args[0].to_string())).unwrap();
                        }
                    },
                    cmd@"regs" => {
                        self.sender.send(DebuggerCommand::ReqCpuRegs).unwrap();
                        self.handle_response(self.receiver.recv().unwrap(),cmd);
                    },
                    cmd@(""|"r") => {
                        // step
                        self.sender.send(DebuggerCommand::Step).unwrap();
                        self.handle_response(self.receiver.recv().unwrap(),cmd);
                    },
                    cmd@"cop0" => {
                        // step
                        self.sender.send(DebuggerCommand::ReqCop0Regs).unwrap();
                        self.handle_response(self.receiver.recv().unwrap(),cmd);
                    },
                    "go" => {
                        if self.break_points.is_empty() {
                            self.sender.send(DebuggerCommand::RunModeChanged(RunMode::FreeMode)).unwrap();
                            info!("No breakpoints set, switching to Free Mode");
                        }
                        else {
                            self.sender.send(DebuggerCommand::RunModeChanged(RunMode::BreakMode(self.break_points.clone()))).unwrap();
                            self.step_by_step_mode = false;
                            info!("Breakpoints set, switching to Break Mode");
                        }
                    },
                    "rw" => {
                        let args = command_iter.collect::<Vec<&str>>();
                        if args.len() != 2 {
                            error!("Wrong number of arguments for 'rw' command: expected <hex address> <length>");
                        }
                        else {
                            let address = u32::from_str_radix(args[0], 16).unwrap();
                            self.sender.send(DebuggerCommand::ReadMemory(address,self.adjust_mem_len(args[1].parse().unwrap()),32)).unwrap();
                            self.handle_response(self.receiver.recv().unwrap(),cmd);
                        }
                    },
                    "rh" => {
                        let args = command_iter.collect::<Vec<&str>>();
                        if args.len() != 2 {
                            error!("Wrong number of arguments for 'rh' command: expected <hex address> <length>");
                        }
                        else {
                            let address = u32::from_str_radix(args[0], 16).unwrap();
                            self.sender.send(DebuggerCommand::ReadMemory(address,self.adjust_mem_len(args[1].parse().unwrap()),16)).unwrap();
                            self.handle_response(self.receiver.recv().unwrap(),cmd);
                        }
                    },
                    "rb" => {
                        let args = command_iter.collect::<Vec<&str>>();
                        if args.len() != 2 {
                            error!("Wrong number of arguments for 'rb' command: expected <hex address> <length>");
                        }
                        else {
                            let address = u32::from_str_radix(args[0], 16).unwrap();
                            self.sender.send(DebuggerCommand::ReadMemory(address,self.adjust_mem_len(args[1].parse().unwrap()),8)).unwrap();
                            self.handle_response(self.receiver.recv().unwrap(),cmd);
                        }
                    },
                    "break"|"b" => {
                        let args = command_iter.collect::<Vec<&str>>();
                        if args.len() == 0 {
                            info!("Break on execute:");
                            for (i,addr) in self.break_points.execute.iter().enumerate() {
                                info!("{:02}:{:08X}",i,addr);
                            }
                            info!("Break on read:");
                            for (i,addr) in self.break_points.read.iter().enumerate() {
                                info!("{:02}:{:08X}",i,addr);
                            }
                            info!("Break on write:");
                            for (i,addr) in self.break_points.write.iter().enumerate() {
                                info!("{:02}:{:08X}",i,addr);
                            }
                            info!("Break on opcode:");
                            if let Some(opcode) = self.break_points.opcode {
                                info!("{:08X}",opcode);
                            }
                        }
                        else if args.len() != 3 {
                            error!("Wrong number of arguments for 'break' command: expected <add/remove> <o|x|r|w> <hex address|opcode>");
                        }
                        else {
                            let bp_type = args[1];
                            let address = u32::from_str_radix(args[2], 16).unwrap();
                            let add = match args[0] {
                                "add"|"a" => true,
                                "remove"|"r" => false,
                                _ => {
                                    error!("Unrecognized breakpoint action: {}",args[0]);
                                    continue;
                                }
                            };
                            match bp_type {
                                "o" => {
                                    if add {
                                        self.break_points.opcode = Some(address)
                                    }
                                    else {
                                        self.break_points.opcode = None
                                    }
                                },
                                "x" => {
                                    if add {
                                        self.break_points.execute.insert(address);
                                    }
                                    else {
                                        self.break_points.execute.remove(&address);
                                    }
                                    info!("{} execute breakpoint at {:08X}",if add {"Add"} else {"Remove"},address);
                                },
                                "r" => {
                                    if add {
                                        self.break_points.read.insert(address);
                                    }
                                    else {
                                        self.break_points.read.remove(&address);
                                    }
                                    info!("{} read breakpoint at {:08X}",if add {"Add"} else {"Remove"},address);
                                },
                                "w" => {
                                    if add {
                                        self.break_points.write.insert(address);
                                    }
                                    else {
                                        self.break_points.write.remove(&address);
                                    }
                                    info!("{} write breakpoint at {:08X}",if add {"Add"} else {"Remove"},address);
                                },
                                _ => {
                                    error!("Unrecognized breakpoint type: {}",bp_type);
                                }
                            }
                        }
                    }
                    cmd => {
                        error!("Unrecognized command {cmd}")
                    }
                }
            //}
        }
    }

    fn adjust_mem_len(&self,n:usize) -> usize {
        let rem = n % DUMP_MEMORY_COLUMNS;
        if rem == 0 {
            n
        }
        else {
            n + DUMP_MEMORY_COLUMNS - rem
        }
    }

    fn handle_response(&mut self,resp:DebuggerResponse,cmd:&str) {
        match resp {
            DebuggerResponse::BreakAt(address) => {
                info!("Break at {:08X}",address);
                self.step_by_step_mode = true;
                self.wait_resp(true);
            },
            DebuggerResponse::CpuRegs(dis,regs,cycles) => {
                if cmd == "r" || cmd == "regs" {
                    info!("CPU Registers [{}]:\n{}",cycles,regs.dump());
                }
                if cmd != "regs" {
                    info!("{}", dis.formatted);
                }
            },
            DebuggerResponse::Cop0Regs(regs) => {
                info!("Cop0 Registers:\n{}",regs.dump());
            },
            DebuggerResponse::Memory(address,mem) => {
                let mut buffer = String::new();
                let mut ascii = String::new();
                let mut address = address;
                let mut base_address = address;
                for (i,v) in mem.iter().enumerate() {
                    let (elem,step) = match cmd {
                        "rw" => (format!("{:08X} ",v),4),
                        "rh" => (format!("{:04X} ",*v as u16),2),
                        "rb" => {
                            let byte = *v as u8;
                            let ascii_char = if byte.is_ascii_graphic() || byte == b' ' {
                                byte as char
                            }
                            else {
                                '.'
                            };
                            ascii.push(ascii_char);
                            (format!("{:02X} ",byte),1)
                        },
                        _ => (String::from(""),1),
                    };
                    buffer.push_str(&elem);
                    address += step;
                    if i != 0 && ((i + 1) % DUMP_MEMORY_COLUMNS) == 0 {
                        info!("{:08X} {} {}",base_address,buffer,ascii);
                        buffer.clear();
                        ascii.clear();
                        base_address = address;
                    }
                }
                if !buffer.is_empty() {
                    info!("{:08X} {} {}",base_address,buffer,ascii);
                }
            },
        }
    }
}