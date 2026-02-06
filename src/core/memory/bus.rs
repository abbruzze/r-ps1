use crate::core::cpu::cop0::Cop0;
use crate::core::cpu::Cpu;
use crate::core::dma::DMAController;
use crate::core::gpu::GPU;
use crate::core::interrupt::{InterruptController, InterruptType, IrqHandler};
use crate::core::memory::get_memory_map;
use crate::core::memory::{ArrayMemory, MemoryMap, MemorySection, MemorySegment};
use crate::core::memory::{Memory, ReadMemoryAccess, WriteMemoryAccess};
use crate::core::sio::SIO0;
use crate::core::timer::Timer;
use std::cell::RefCell;
use std::process::exit;
use std::rc::Rc;
use tracing::{debug, info, warn};
use crate::core::clock::{Clock, ClockConfig};

const DEBUG_MEM : bool = false;

const PHYSICAL_MEMORY_SIZE : usize = 2 * 1024 * 1024; // 2M

// Timings
const SCRATCHPAD_READ_CYCLES: usize = 1;
const SCRATCHPAD_WRITE_CYCLES: usize = 1;
const RAM_ACCESS_CYCLES: usize = 5;
const BIOS8_READ_CYCLES: usize = 8;
const BIOS16_READ_CYCLES: usize = 12;
const BIOS32_READ_CYCLES: usize = 24;
const IO_REG_ACCESS_CYCLES: usize = RAM_ACCESS_CYCLES;
const EXP1_8_ACCESS_CYCLES : usize = 7;
const EXP1_16_ACCESS_CYCLES : usize = 13;
const EXP1_32_ACCESS_CYCLES : usize = 25;
const EXP2_8_ACCESS_CYCLES : usize = 11;
const EXP2_16_ACCESS_CYCLES : usize = 26;
const EXP2_32_ACCESS_CYCLES : usize = 56;
const EXP3_8_ACCESS_CYCLES : usize = 7;
const EXP3_16_ACCESS_CYCLES : usize = 6;
const EXP3_32_ACCESS_CYCLES : usize = 10;

// Interrupts
const I_STAT : u32 = 0x1F801070;
const I_MASK : u32 = 0x1F801074;
/*
Memory Control
  0-3   Write Delay        (00h..0Fh=01h..10h Cycles)
  4-7   Read Delay         (00h..0Fh=01h..10h Cycles)
  8     Recovery Period    (0=No, 1=Yes, uses COM0 timings)
  9     Hold Period        (0=No, 1=Yes, uses COM1 timings)
  10    Floating Period    (0=No, 1=Yes, uses COM2 timings)
  11    Pre-strobe Period  (0=No, 1=Yes, uses COM3 timings)
  12    Data Bus-width     (0=8bits, 1=16bits)
  13    Auto Increment     (0=No, 1=Yes)
  14-15 Unknown (R/W)
  16-20 Number of address bits (memory window size = 1 << N bytes)
  21-23 Unknown (always zero)
  24-27 DMA timing override
  28    Address error flag. Write 1 to it to clear it.
  29    DMA timing select  (0=use normal timings, 1=use bits 24-27)
  30    Wide DMA           (0=use bit 12, 1=override to full 32 bits)
  31    Wait               (1=wait on external device before being ready)
 */
const IO_BASE_ADDRESS: u32 = 0x1F801000;
const EXP1_BASE_ADDRESS : u32 = IO_BASE_ADDRESS; // Expansion 1 Base Address (usually 1F000000h)
const EXP2_BASE_ADDRESS : u32 = 0x1F801004; // Expansion 2 Base Address (usually 1F802000h)
const EXP1_DELAY_SIZE : u32 = 0x1F801008; // Expansion 1 Delay/Size (usually 0013243Fh) (512Kbytes, 8bit bus) (573: 24173F47h)
const EXP3_DELAY_SIZE : u32 = 0x1F80100C; // Expansion 3 Delay/Size (usually 00003022h) (1 byte)
const BIOS_ROM_DELAY_SIZE : u32 = 0x1F801010; // BIOS ROM Delay/Size (usually 0013243Fh) (512Kbytes, 8bit bus)
const SPU_DELAY_SIZE : u32 = 0x1F801014; // SPU Delay/Size (200931E1h) (use 220931E1h for SPU-RAM reads)
const CDROM_DELAY_SIZE : u32 = 0x1F801018; // CDROM Delay/Size (00020843h or 00020943h)
const EXP2_DELAY_SIZE : u32 = 0x1F80101C; // Expansion 2 Delay/Size (usually 00070777h) (128 bytes, 8bit bus)
const COM_DELAY : u32 = 0x1F801020; // COM_DELAY / COMMON_DELAY (00031125h or 0000132Ch or 00001325h)
/*
RAM size.
  0-2   Unknown (no effect)
  3     Crashes when zero (except PU-7 and EARLY-PU-8, which <do> set bit3=0)
  4-6   Unknown (no effect)
  7     Delay on simultaneous CODE+DATA fetch from RAM (0=None, 1=One Cycle)
  8     Unknown (no effect) (should be set for 8MB, cleared for 2MB)
  9     RAM chip size 1 (0=1MB or 2MB, 1=4MB or 8MB)
  10    Enable /RAS1 bank (0=disable/bus fault on access, 1=enable)
  11    RAM chip size 2 (0=1MB or 4MB, 1=2MB or 8MB)
  12-15 Unknown (no effect)
  16-31 Unknown (Garbage)

Possible values for bits 9-11 are:
  000 = 1MB bank on /RAS0 + 15MB unmapped
  001 = 4MB bank on /RAS0 + 12MB unmapped
  010 = 1MB bank on /RAS0 + 1MB bank on /RAS1 (?) + 14MB unmapped
  011 = 4MB bank on /RAS0 + 4MB bank on /RAS1 (?) + 8MB unmapped
  100 = 2MB bank on /RAS0 + 14MB unmapped
  101 = 8MB bank on /RAS0 + 8MB unmapped
  110 = 2MB bank on /RAS0 + 2MB bank on /RAS1 (?) + 12MB unmapped
  111 = 8MB bank on /RAS0 + 8MB bank on /RAS1 (?)

Additional mirrors within these 512MB regions are:
  2MB RAM can be mirrored to the first 8MB (strangely, enabled by default)
  512K BIOS ROM can be mirrored to the last 4MB (disabled by default)
  Expansion hardware (if any) may be mirrored within expansion region
  The seven DMA Control Registers at 1F8010x8h are mirrored to 1F8010xCh
 */
const RAM_SIZE : u32 = 0x1F801060; // RAM_SIZE (R/W) (usually 00000B88h) (or 00000888h)

// Cache configuration
const CACHE_CONF_REG : u32 = 0xFFFE0130; // BCC, BIU/Cache Configuration Register (R/W)

const IO_PORTS_LEN : usize = ((RAM_SIZE + 4 - IO_BASE_ADDRESS) >> 2) as usize;

// Timers
const TIMER0_COUNTER : u32 = 0x1F801100;
const TIMER0_COUNTER_MODE : u32 = 0x1F801104;
const TIMER0_COUNTER_TARGET : u32 = 0x1F801108;
const TIMER1_COUNTER : u32 = 0x1F801110;
const TIMER1_COUNTER_MODE : u32 = 0x1F801114;
const TIMER1_COUNTER_TARGET : u32 = 0x1F801118;
const TIMER2_COUNTER : u32 = 0x1F801120;
const TIMER2_COUNTER_MODE : u32 = 0x1F801124;
const TIMER2_COUNTER_TARGET : u32 = 0x1F801128;

struct MemoryBridge<const N: usize> {
    read: [fn(&mut Bus,u32,usize) -> ReadMemoryAccess; N],
    write: [fn(&mut Bus,u32,u32,usize) -> WriteMemoryAccess; N],
    peek: [fn(&Bus,u32) -> Option<u32>; N],
}

impl<const N: usize> MemoryBridge<N> {
    fn new() -> Self {
        Self {
            read: [Bus::read_unmapped;N],
            write: [Bus::write_unmapped;N],
            peek: [Bus::peek_unmapped;N],
        }
    }
}

struct Interrupt {
    pending: u16,
    mask: u16,
}

impl Interrupt {
    fn new() -> Self {
        Self {
            pending: 0,
            mask : 0,
        }
    }

    fn reset(&mut self) {
        self.pending = 0;
        self.mask = 0;
    }

    fn is_interrupt_pending(&self) -> bool {
        (self.pending & self.mask) != 0
    }
}

pub struct Bus {
    clock: Clock,
    bios: ArrayMemory,
    main_ram: Vec<u8>,
    cop0: Cop0,
    timer0: Timer<0>,
    timer1: Timer<1>,
    timer2: Timer<2>,
    dma: Rc<RefCell<DMAController>>,
    gpu: Rc<RefCell<GPU>>,
    sio0: SIO0,
    io_ports: [u32;IO_PORTS_LEN],
    scratchpad: Vec<u8>,
    cache_control_reg: u32,
    interrupt: Interrupt,
    io_mem_bridge: MemoryBridge<0x1000>,
    spu_dummy_regs: ArrayMemory,
}

impl InterruptController for Bus {
    fn raise_hw_interrupts(&mut self,irqs:u16) {
        debug!("Raising interrupts {:04X} I_STAT={:04X} I_MASK=${:04X}",irqs,self.interrupt.pending,self.interrupt.mask);
        self.interrupt.pending |= irqs;
        self.check_interrupt();
    }
}

impl Bus {
    pub fn new(clock_config:ClockConfig,
               bios: ArrayMemory,
               dma: &Rc<RefCell<DMAController>>,
               gpu: &Rc<RefCell<GPU>>) -> Self {
        let main_ram = vec![0; PHYSICAL_MEMORY_SIZE]; // 2MB of main RAM
        let mut bus = Bus {
            clock: Clock::new(clock_config),
            bios,
            main_ram,
            cop0: Cop0::new(),
            timer0: Timer::<0>::new(),
            timer1: Timer::<1>::new(),
            timer2: Timer::<2>::new(),
            dma: dma.clone(),
            gpu: gpu.clone(),
            sio0: SIO0::new(true,true),
            io_ports: [0;IO_PORTS_LEN],
            scratchpad: vec![0; 0x400],
            cache_control_reg: 0,
            interrupt: Interrupt::new(),
            io_mem_bridge: MemoryBridge::<0x1000>::new(),
            spu_dummy_regs: ArrayMemory::new(&[0;640],false,0,0),
        };
        bus.init_io_bridge();
        bus.timer0.initial_scheduling(&mut bus.clock);
        bus.timer1.initial_scheduling(&mut bus.clock);
        bus.timer2.initial_scheduling(&mut bus.clock);
        bus
    }
    
    pub fn get_timer0_and_clock_mut(&mut self) -> (&mut Timer<0>,&mut Clock) {
        (&mut self.timer0, &mut self.clock)
    }
    pub fn get_timer1_mut(&mut self) -> &mut Timer<1> {
        &mut self.timer1
    }
    pub fn get_timer1_and_clock_mut(&mut self) -> (&mut Timer<1>,&mut Clock) {
        (&mut self.timer1, &mut self.clock)
    }
    pub fn get_timer2_and_clock_mut(&mut self) -> (&mut Timer<2>,&mut Clock) {
        (&mut self.timer2, &mut self.clock)
    }

    pub fn get_clock_mut(&mut self) -> &mut Clock {
        &mut self.clock
    }

    pub fn get_clock(&self) -> &Clock {
        &self.clock
    }

    pub fn get_sio0_and_clock_mut(&mut self) -> (&mut SIO0,&mut Clock) {
        (&mut self.sio0, &mut self.clock)
    }
    
    pub fn get_sio0_mut(&mut self) -> &mut SIO0 {
        &mut self.sio0
    }

    pub fn get_cop0_mut(&mut self) -> &mut Cop0 {
        &mut self.cop0
    }

    pub fn get_cop0(&mut self) -> &Cop0 {
        &self.cop0
    }

    pub fn get_main_ram(&self) -> &Vec<u8> {
        &self.main_ram
    }

    pub fn get_scratchpad(&self) -> &Vec<u8> {
        &self.scratchpad
    }

    pub fn get_bios(&self) -> &Vec<u8> {
        &self.bios.memory
    }

    fn io_port_read(&self,address:u32) -> u32 {
        self.io_ports[((address - IO_BASE_ADDRESS) >> 2) as usize]
    }

    fn io_port_write(&mut self,address:u32,value:u32) {
        self.io_ports[((address - IO_BASE_ADDRESS) >> 2) as usize] = value;
    }

    fn check_interrupt(&mut self) {
        if self.interrupt.is_interrupt_pending() {
            self.cop0.set_hw_interrupt();
        }
        else {
            self.cop0.clear_hw_interrupt();
        }
    }

    fn read_unmapped(&mut self,address:u32,_size:usize) -> ReadMemoryAccess {
        warn!("Reading from an unmapped I/O address: {:08X}",address);
        exit(1);
        //ReadMemoryAccess::BusError
        //ReadMemoryAccess::Read(0,0)
    }

    fn write_unmapped(&mut self,address:u32,value:u32,_size:usize) -> WriteMemoryAccess {
        warn!("Writing to an unmapped I/O address: {:08X} = {:08X}",address,value);
        exit(1);
        //WriteMemoryAccess::BusError
        //WriteMemoryAccess::Write(0)
    }

    fn peek_unmapped(&self,_address:u32) -> Option<u32> {
        None
    }

    fn init_io_bridge(&mut self) {
        for address in IO_BASE_ADDRESS..IO_BASE_ADDRESS + 0x1000 {
            let fun_offset = (address - IO_BASE_ADDRESS) as usize;
            match address {
                EXP1_BASE_ADDRESS |
                EXP2_BASE_ADDRESS |
                EXP1_DELAY_SIZE |
                EXP3_DELAY_SIZE |
                BIOS_ROM_DELAY_SIZE |
                SPU_DELAY_SIZE |
                CDROM_DELAY_SIZE |
                EXP2_DELAY_SIZE |
                COM_DELAY |
                RAM_SIZE => {
                    self.io_mem_bridge.read[fun_offset] = |bus,address,size| {
                        if size == 8 {
                            warn!("Reading from I/O register with size {size}");
                        }
                        ReadMemoryAccess::Read(bus.io_port_read(address),IO_REG_ACCESS_CYCLES)
                    };
                    self.io_mem_bridge.peek[fun_offset] = |bus,address| { Some(bus.io_port_read(address)) };
                    self.io_mem_bridge.write[fun_offset] = |bus,address,value,size| {
                      debug!("Writing to I/O register {:08X} = {:08X}",address,value);
                      if size == 8 {
                          warn!("Writing to I/O register with size {size}");
                      }
                      bus.io_port_write(address,value);
                      WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    };
                },
                TIMER0_COUNTER => {
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,size| {
                        if size == 8 {
                            warn!("Reading from Timer0 counter with size {size}");
                        }
                        ReadMemoryAccess::Read(bus.timer0.read_counter(&bus.clock),IO_REG_ACCESS_CYCLES)
                    };
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| { Some(bus.timer0.read_counter(&bus.clock)) };
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,value,size| {
                        if size == 8 {
                            warn!("Writing to Timer0 counter with size {size}");
                        }
                        bus.timer0.write_counter(value,&mut bus.clock);
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    }
                },
                TIMER1_COUNTER => {
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,size| {
                        if size == 8 {
                            warn!("Reading from Timer1 counter with size {size}");
                        }
                        ReadMemoryAccess::Read(bus.timer1.read_counter(&bus.clock),IO_REG_ACCESS_CYCLES)
                    };
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| { Some(bus.timer1.read_counter(&bus.clock)) };
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,value,size| {
                        if size == 8 {
                            warn!("Writing to Timer1 counter with size {size}");
                        }
                        bus.timer1.write_counter(value,&mut bus.clock);
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    }
                },
                TIMER2_COUNTER => {
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,size| {
                        if size == 8 {
                            warn!("Reading from Timer2 counter with size {size}");
                        }
                        ReadMemoryAccess::Read(bus.timer2.read_counter(&bus.clock),IO_REG_ACCESS_CYCLES)
                    };
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| { Some(bus.timer2.read_counter(&bus.clock)) };
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,value,size| {
                        if size == 8 {
                            warn!("Writing to Timer2 counter with size {size}");
                        }
                        bus.timer2.write_counter(value,&mut bus.clock);
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    }
                },
                TIMER0_COUNTER_MODE => {
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,size| {
                        if size == 8 {
                            warn!("Reading from Timer0 counter mode with size {size}");
                        }
                        ReadMemoryAccess::Read(bus.timer0.read_counter_mode(),IO_REG_ACCESS_CYCLES)
                    };
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| { Some(bus.timer0.peek_counter_mode()) };
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,value,size| {
                        if size == 8 {
                            warn!("Writing to Timer0 counter mode with size {size}");
                        }
                        bus.timer0.write_counter_mode(value,&mut bus.clock);
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    }
                },
                TIMER1_COUNTER_MODE => {
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,size| {
                        if size == 8 {
                            warn!("Reading from Timer1 counter mode with size {size}");
                        }
                        ReadMemoryAccess::Read(bus.timer1.read_counter_mode(),IO_REG_ACCESS_CYCLES)
                    };
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| { Some(bus.timer1.peek_counter_mode()) };
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,value,size| {
                        if size == 8 {
                            warn!("Writing to Timer1 counter mode with size {size}");
                        }
                        bus.timer1.write_counter_mode(value,&mut bus.clock);
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    }
                },
                TIMER2_COUNTER_MODE => {
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,size| {
                        if size == 8 {
                            warn!("Reading from Timer2 counter mode with size {size}");
                        }
                        ReadMemoryAccess::Read(bus.timer2.read_counter_mode(),IO_REG_ACCESS_CYCLES)
                    };
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| { Some(bus.timer2.peek_counter_mode()) };
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,value,size| {
                        if size == 8 {
                            warn!("Writing to Timer2 counter mode with size {size}");
                        }
                        bus.timer2.write_counter_mode(value,&mut bus.clock);
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    }
                },
                TIMER0_COUNTER_TARGET => {
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,size| {
                        if size == 8 {
                            warn!("Reading from Timer0 target mode with size {size}");
                        }
                        ReadMemoryAccess::Read(bus.timer0.read_counter_target(),IO_REG_ACCESS_CYCLES)
                    };
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| { Some(bus.timer0.read_counter_target()) };
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,value,size| {
                        if size == 8 {
                            warn!("Writing to Timer0 counter target with size {size}");
                        }
                        bus.timer0.write_counter_target(value,&mut bus.clock);
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    }
                },
                TIMER1_COUNTER_TARGET => {
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,size| {
                        if size == 8 {
                            warn!("Reading from Timer1 target mode with size {size}");
                        }
                        ReadMemoryAccess::Read(bus.timer1.read_counter_target(),IO_REG_ACCESS_CYCLES)
                    };
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| { Some(bus.timer1.read_counter_target()) };
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,value,size| {
                        if size == 8 {
                            warn!("Writing to Timer1 counter target with size {size}");
                        }
                        bus.timer1.write_counter_target(value,&mut bus.clock);
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    }
                },
                TIMER2_COUNTER_TARGET => {
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,size| {
                        if size == 8 {
                            warn!("Reading from Timer2 target mode with size {size}");
                        }
                        ReadMemoryAccess::Read(bus.timer2.read_counter_target(),IO_REG_ACCESS_CYCLES)
                    };
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| { Some(bus.timer2.read_counter_target()) };
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,value,size| {
                        if size == 8 {
                            warn!("Writing to Timer2 counter target with size {size}");
                        }
                        bus.timer2.write_counter_target(value,&mut bus.clock);
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    }
                },
                I_STAT => {
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,_size| {
                        ReadMemoryAccess::Read(bus.interrupt.pending as u32,IO_REG_ACCESS_CYCLES)
                    };
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| { Some(bus.interrupt.pending as u32) };
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,value,size| {
                        debug!("Writing I_STAT {:08X}",value);
                        // if size != 16 {
                        //     warn!("Writing to I_STAT port with size {size}");
                        // }
                        bus.interrupt.pending &= value as u16;
                        bus.check_interrupt();
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    };
                },
                I_MASK => {
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,size| {
                        ReadMemoryAccess::Read(bus.interrupt.mask as u32,IO_REG_ACCESS_CYCLES)
                    };
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| { Some(bus.interrupt.mask as u32) };
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,value,size| {
                        debug!("Writing I_MASK {:08X}",value);
                        // if size != 16 {
                        //     warn!("Writing to I_MASK port with size {size}");
                        // }
                        bus.interrupt.mask = value as u16;
                        bus.check_interrupt();
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    };
                },
                // SPU
                0x1F801C00..=0x1F801E80 => {
                    self.io_mem_bridge.read[fun_offset] = Bus::spu_read;
                    self.io_mem_bridge.peek[fun_offset] = Bus::spu_peek;
                    self.io_mem_bridge.write[fun_offset] = Bus::spu_write;
                },
                // DMA
                0x1F801080|0x1F801090|0x1F8010A0|0x1F8010B0|0x1F8010C0|0x1F8010D0|0x1F8010E0 => {
                    self.io_mem_bridge.read[fun_offset] = Bus::dma_read_madr;
                    self.io_mem_bridge.peek[fun_offset] = Bus::dma_peek_madr;
                    self.io_mem_bridge.write[fun_offset] = Bus::dma_write_madr;
                }
                0x1F801084|0x1F801094|0x1F8010A4|0x1F8010B4|0x1F8010C4|0x1F8010D4|0x1F8010E4 => {
                    self.io_mem_bridge.read[fun_offset] = Bus::dma_read_bcr;
                    self.io_mem_bridge.peek[fun_offset] = Bus::dma_peek_bcr;
                    self.io_mem_bridge.write[fun_offset] = Bus::dma_write_bcr;
                }
                0x1F801088|0x1F801098|0x1F8010A8|0x1F8010B8|0x1F8010C8|0x1F8010D8|0x1F8010E8 => {
                    self.io_mem_bridge.read[fun_offset] = Bus::dma_read_chcr;
                    self.io_mem_bridge.peek[fun_offset] = Bus::dma_peek_chcr;
                    self.io_mem_bridge.write[fun_offset] = Bus::dma_write_chcr;
                }
                0x1F8010F0 => {
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,_size| ReadMemoryAccess::Read(bus.dma.borrow().read_dpcr(),IO_REG_ACCESS_CYCLES);
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| Some(bus.dma.borrow().read_dpcr());
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,value,_size| {
                        bus.dma.borrow_mut().write_dpcr(value);
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    }
                }
                0x1F8010F4 => {
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,_size| ReadMemoryAccess::Read(bus.dma.borrow().read_dicr(),IO_REG_ACCESS_CYCLES);
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| Some(bus.dma.borrow().read_dicr());
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,value,_size| {
                        bus.dma.borrow_mut().write_dicr(value);
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    }
                }
                0x1F8010F8|0x1F8010FC => {
                    self.io_mem_bridge.read[fun_offset] = |bus,address,_size| {
                        if address == 0x1F8010F8 {
                            ReadMemoryAccess::Read(bus.dma.borrow().read_reg_f8(), IO_REG_ACCESS_CYCLES)
                        }
                        else {
                            ReadMemoryAccess::Read(bus.dma.borrow().read_reg_fc(), IO_REG_ACCESS_CYCLES)
                        }
                    };
                    self.io_mem_bridge.peek[fun_offset] = |bus,address| {
                        if address == 0x1F8010F8 {
                            Some(bus.dma.borrow().read_reg_f8())
                        }
                        else {
                            Some(bus.dma.borrow().read_reg_fc())
                        }
                    };
                    self.io_mem_bridge.write[fun_offset] = |bus,address,value,_size| {
                        if address == 0x1F8010F8 {
                            bus.dma.borrow_mut().write_reg_f8(value);
                        }
                        else {
                            bus.dma.borrow_mut().write_reg_fc(value);
                        }
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    }
                }
                // GPU
                0x1F801810 => {
                    // 1F801810h-Write GP0     Send GP0 Commands/Packets (Rendering and VRAM Access)
                    // 1F801810h-Read  GPUREAD Receive responses to GP0(C0h) and GP1(10h) commands
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,_size| ReadMemoryAccess::Read(bus.gpu.borrow_mut().gpu_read_read(), IO_REG_ACCESS_CYCLES);
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| Some(bus.gpu.borrow().gpu_read_peek());
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,value,_size| {
                        let mut irq_handler = IrqHandler::new();
                        bus.gpu.borrow_mut().gp0_cmd(value,&mut bus.clock,&mut irq_handler);
                        irq_handler.forward_to_controller(bus);
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    }
                }
                0x1F801814 => {
                    // 1F801814h-Write GP1     Send GP1 Commands (Display Control) (and DMA Control)
                    // 1F801814h-Read  GPUSTAT Receive GPU Status Register
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,_size| ReadMemoryAccess::Read(bus.gpu.borrow().gpu_stat_read(), IO_REG_ACCESS_CYCLES);
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| Some(bus.gpu.borrow().gpu_stat_read());
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,value,_size| {
                        bus.gpu.borrow_mut().gp1_cmd(value);
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    }
                }
                // JOY DATA
                0x1F801040 => {
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,_size| ReadMemoryAccess::Read(bus.sio0.read_rx_data() as u32, IO_REG_ACCESS_CYCLES);
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| Some(bus.sio0.peek_rx_data() as u32);
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,value,_size| {
                        bus.sio0.write_tx_data(value as u8,&mut bus.clock);
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    }
                }
                // JOY STAT
                0x1F801044 => {
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,_size| ReadMemoryAccess::Read(bus.sio0.read_status(&bus.clock), IO_REG_ACCESS_CYCLES);
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| Some(bus.sio0.read_status(&bus.clock));
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,_value,_size| WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES); // read-only
                }
                // JOY MODE
                0x1F801048 => {
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,_size| ReadMemoryAccess::Read(bus.sio0.read_mode() as u32, IO_REG_ACCESS_CYCLES);
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| Some(bus.sio0.read_mode() as u32);
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,value,_size| {
                        bus.sio0.write_mode(value as u16);
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    }
                }
                // JOY CTRL
                0x1F80104A => {
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,_size| ReadMemoryAccess::Read(bus.sio0.read_ctrl() as u32, IO_REG_ACCESS_CYCLES);
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| Some(bus.sio0.read_ctrl() as u32);
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,value,_size| {
                        bus.sio0.write_ctrl(value as u16);
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    }
                }
                // JOY BAUD
                0x1F80104E => {
                    self.io_mem_bridge.read[fun_offset] = |bus,_address,_size| ReadMemoryAccess::Read(bus.sio0.read_baud() as u32, IO_REG_ACCESS_CYCLES);
                    self.io_mem_bridge.peek[fun_offset] = |bus,_address| Some(bus.sio0.read_baud() as u32);
                    self.io_mem_bridge.write[fun_offset] = |bus,_address,value,_size| {
                        bus.sio0.write_baud(value as u16);
                        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
                    }
                }
                _ => {}
            }
        }
    }
    
    // SPU handling
    fn spu_read(&mut self,address:u32,size:usize) -> ReadMemoryAccess {
        // TODO
        debug!("Reading SPU register at {:08X} [{size}]",address);
        let offset = address - 0x1F801C00;
        match size {
            8 => self.spu_dummy_regs.read::<8>(offset,false),
            16 => self.spu_dummy_regs.read::<16>(offset,false),
            32 => self.spu_dummy_regs.read::<32>(offset,false),
            _ => panic!("Invalid SPU register read size: {}",size),
        }
        //ReadMemoryAccess::Read(0,IO_REG_ACCESS_CYCLES)
    }
    fn spu_peek(&self,_address:u32) -> Option<u32> {
        // TODO
        Some(0)
    }
    fn spu_write(&mut self,address:u32,value:u32,size:usize) -> WriteMemoryAccess {
        // TODO
        debug!("Writing SPU register at {:08X} = {:08X} [{size}]",address,value);
        let offset = address - 0x1F801C00;
        match size {
            8 => self.spu_dummy_regs.write::<8>(offset,value),
            16 => self.spu_dummy_regs.write::<16>(offset,value),
            32 => self.spu_dummy_regs.write::<32>(offset,value),
            _ => panic!("Invalid SPU register read size: {}",size),
        }
        //WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
    }
    // DMA handling
    // MADR
    fn dma_read_madr(&mut self,address:u32,_size:usize) -> ReadMemoryAccess {
        let channel = ((address >> 4) & 0xF) - 8;
        ReadMemoryAccess::Read(self.dma.borrow().read_madr(channel as usize),IO_REG_ACCESS_CYCLES)

    }
    fn dma_peek_madr(&self,address:u32) -> Option<u32> {
        let channel = ((address >> 4) & 0xF) - 8;
        Some(self.dma.borrow().read_madr(channel as usize))

    }
    fn dma_write_madr(&mut self,address:u32,value:u32,_size:usize) -> WriteMemoryAccess {
        let channel = ((address >> 4) & 0xF) - 8;
        self.dma.borrow_mut().write_madr(channel as usize,value);
        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
    }
    // BCR
    fn dma_read_bcr(&mut self,address:u32,_size:usize) -> ReadMemoryAccess {
        let channel = ((address >> 4) & 0xF) - 8;
        ReadMemoryAccess::Read(self.dma.borrow().read_bcr(channel as usize),IO_REG_ACCESS_CYCLES)

    }
    fn dma_peek_bcr(&self,address:u32) -> Option<u32> {
        let channel = ((address >> 4) & 0xF) - 8;
        Some(self.dma.borrow().read_bcr(channel as usize))

    }
    fn dma_write_bcr(&mut self,address:u32,value:u32,_size:usize) -> WriteMemoryAccess {
        let channel = ((address >> 4) & 0xF) - 8;
        self.dma.borrow_mut().write_bcr(channel as usize,value);
        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
    }
    // CHCR
    fn dma_read_chcr(&mut self,address:u32,_size:usize) -> ReadMemoryAccess {
        let channel = ((address >> 4) & 0xF) - 8;
        ReadMemoryAccess::Read(self.dma.borrow().read_chcr(channel as usize),IO_REG_ACCESS_CYCLES)

    }
    fn dma_peek_chcr(&self,address:u32) -> Option<u32> {
        let channel = ((address >> 4) & 0xF) - 8;
        Some(self.dma.borrow().read_chcr(channel as usize))

    }
    fn dma_write_chcr(&mut self,address:u32,value:u32,_size:usize) -> WriteMemoryAccess {
        let channel = ((address >> 4) & 0xF) - 8;
        self.dma.borrow_mut().write_chcr(channel as usize,value);
        WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
    }

    pub fn load_exe(&mut self, exe: Vec<u8>, cpu:&mut Cpu) {
        // Parse EXE header
        let initial_pc = u32::from_le_bytes(exe[0x10..0x14].try_into().unwrap());
        let initial_r28 = u32::from_le_bytes(exe[0x14..0x18].try_into().unwrap());
        let exe_ram_addr = u32::from_le_bytes(exe[0x18..0x1C].try_into().unwrap()) & 0x1FFFFF;
        let exe_size_2kb = u32::from_le_bytes(exe[0x1C..0x20].try_into().unwrap());
        let initial_sp = u32::from_le_bytes(exe[0x30..0x34].try_into().unwrap());

        // Copy EXE code/data into PS1 RAM
        let mut exe_size = if (exe_size_2kb * 2048) as usize > exe.len() {
            exe_size_2kb
        }
        else {
            exe_size_2kb * 2048
        };
        if exe_size + 2048 > exe.len() as u32 {
            exe_size = (exe.len() - 2048) as u32;
            warn!("Invalid EXE file size: forced to {}",exe_size);
        }
        self.main_ram[exe_ram_addr as usize..(exe_ram_addr + exe_size) as usize].copy_from_slice(&exe[2048..2048 + exe_size as usize]);
        cpu.get_registers_mut()[28] = initial_r28;
        if initial_sp != 0 {
            cpu.get_registers_mut()[29] = initial_sp;
            cpu.get_registers_mut()[30] = initial_sp;
        }
        cpu.set_pc(initial_pc);
        info!("EXE loaded at {:08X} size is {} bytes PC={:08X}",exe_ram_addr,exe_size,initial_pc);
    }

    pub fn load_pre_exe(&mut self,bin:Vec<u8>,address:u32) {
        let address = address & 0x1FFFFF;
        self.main_ram[address as usize..(address as usize + bin.len() )].copy_from_slice(&bin);
    }
}
/*
Memory Exceptions
  Memory Error ------> Misalignments
               (and probably also KSEG access in User mode)
  Bus Error    ------> Unused Memory Regions (including Gaps in I/O Region)
               (unless RAM/BIOS/Expansion mirrors are mapped to "unused" area)
 */
impl Memory for Bus {
    // address alignment is done by the caller
    fn read<const SIZE: usize>(&mut self, address: u32,is_fetching:bool) -> ReadMemoryAccess {
        const { assert!(SIZE == 8 || SIZE == 16 || SIZE == 32) }
        // get address's info
        let MemoryMap(segment,section,address,offset) = get_memory_map(address);
        if DEBUG_MEM {
            debug!("Memory read<{SIZE}>({:?}/{:?}): address={:08X} is_fetching={is_fetching}",segment,section,address);
        }
        // check segment access privilege: user can access to KUSEG only
        if segment != MemorySegment::KUSEG && !self.cop0.is_in_kernel_mode() {
            return ReadMemoryAccess::MemoryError
        }
        match section {
            // RAM =======================================================================================
            MemorySection::MainRAM => {
                if offset < 0x80_0000 { // first 8M
                    let ram_address = offset & 0x1F_FFFF; // // 2MB RAM can be mirrored to the first 8MB (strangely, enabled by default)
                    let read = <Self as Memory>::mem_read::<SIZE>(ram_address,&self.main_ram);
                    ReadMemoryAccess::Read(read, RAM_ACCESS_CYCLES)
                }
                else {
                    warn!("Reading from an unmapped RAM address: {:08X}",address);
                    ReadMemoryAccess::BusError
                }
            },
            // Scratchpad ================================================================================
            MemorySection::ScratchPad => {
                // Note that the scratchpad is NOT executable. Attempts to jump to this region will cause a bus error on the first instruction fetch
                if is_fetching {
                    warn!("Fetching from scratchpad");
                    ReadMemoryAccess::BusError
                }
                else if offset < 0x400 {
                    let read = <Self as Memory>::mem_read::<SIZE>(offset,&self.scratchpad);
                    ReadMemoryAccess::Read(read,SCRATCHPAD_READ_CYCLES)
                }
                else {
                    warn!("Reading from an unmapped scratchpad address: {:08X}",address);
                    ReadMemoryAccess::BusError
                }
            },
            // BIOS ======================================================================================
            MemorySection::BIOSRom => {
                // BIOS is 512K
                if offset < 0x80_000 {
                    let read = <Self as Memory>::mem_read::<SIZE>(offset,&self.bios.memory);
                    let penalty = match SIZE {
                        8 => BIOS8_READ_CYCLES,
                        16 => BIOS16_READ_CYCLES,
                        32 => BIOS32_READ_CYCLES,
                        _ => unreachable!()
                    };
                    ReadMemoryAccess::Read(read,penalty)
                }
                else {
                    warn!("Reading from an unmapped BIOS address: {:08X}",address);
                    ReadMemoryAccess::BusError
                }
            },
            // I/O =======================================================================================
            MemorySection::IOPorts => {
                self.io_mem_bridge.read[offset as usize](self,address,SIZE)
            }
            // Expansion 1 ===============================================================================
            MemorySection::ExpansionRegion1 => {
                let (value,penalty) = match SIZE {
                    8 => (0xFF,EXP1_8_ACCESS_CYCLES),
                    16 => (0xFFFF,EXP1_16_ACCESS_CYCLES),
                    32 => (0xFFFFFFFF,EXP1_32_ACCESS_CYCLES),
                    _ => (0,0),
                };
                ReadMemoryAccess::Read(value,penalty)
            },
            // Expansion 2 ===============================================================================
            MemorySection::ExpansionRegion2 => {
                let (value,penalty) = match SIZE {
                    8 => (0xFF,EXP1_8_ACCESS_CYCLES),
                    16 => (0xFFFF,EXP1_16_ACCESS_CYCLES),
                    32 => (0xFFFFFFFF,EXP1_32_ACCESS_CYCLES),
                    _ => (0,0),
                };
                ReadMemoryAccess::Read(value,penalty)
            },
            // Expansion 3 ===============================================================================
            MemorySection::ExpansionRegion3 => {
                let (value,penalty) = match SIZE {
                    8 => (0xFF,EXP1_8_ACCESS_CYCLES),
                    16 => (0xFFFF,EXP1_16_ACCESS_CYCLES),
                    32 => (0xFFFFFFFF,EXP1_32_ACCESS_CYCLES),
                    _ => (0,0),
                };
                ReadMemoryAccess::Read(value,penalty)
            },
            // Cache Control =============================================================================
            MemorySection::CacheControl => {
                ReadMemoryAccess::Read(self.cache_control_reg,IO_REG_ACCESS_CYCLES)
            },
            MemorySection::Unmapped => {
                warn!("Reading from an unmapped address {:08X}",address);
                ReadMemoryAccess::BusError
            }
        }
    }

    fn write<const SIZE: usize>(&mut self, address: u32, value: u32) -> WriteMemoryAccess {
        const { assert!(SIZE == 8 || SIZE == 16 || SIZE == 32) }
        // check if cache is isolated
        let cache_isolated = self.cop0.is_cache_isolated();
        if cache_isolated {
            // If cache is isolated, send writes directly to instruction cache
            return if (self.cache_control_reg & 0x04) != 0 { // TAG    Enable cache tag test mode (when COP0_SR.IsC=1, used to flush i-cache)
                WriteMemoryAccess::InvalidateICacheTag
            } else {
                WriteMemoryAccess::InvalidateICacheOpcode
            }
        };

        // get address's info
        let MemoryMap(segment,section,address,offset) = get_memory_map(address);
        if DEBUG_MEM {
            debug!("Memory write<{SIZE}>({:?}/{:?}): address={:08X} value={:08X}",segment,section,address,value);
        }
        // check KSEG access in user mode
        if segment == MemorySegment::KUSEG && !self.cop0.is_in_kernel_mode() {
            return WriteMemoryAccess::MemoryError
        }

        match section {
            // RAM =======================================================================================
            MemorySection::MainRAM => {
                if offset < 0x80_0000 { // first 8M
                    let ram_address = offset & 0x1F_FFFF; // // 2MB RAM can be mirrored to the first 8MB (strangely, enabled by default)
                    <Self as Memory>::mem_write::<SIZE>(ram_address,value,&mut self.main_ram);
                    WriteMemoryAccess::Write(RAM_ACCESS_CYCLES)
                }
                else {
                    warn!("Writing to an unmapped RAM address: {:08X}",offset);
                    WriteMemoryAccess::BusError
                }
            },
            // Scratchpad ================================================================================
            MemorySection::ScratchPad => {
                if offset < 0x400 {
                    <Self as Memory>::mem_write::<SIZE>(offset, value, &mut self.scratchpad);
                    WriteMemoryAccess::Write(SCRATCHPAD_WRITE_CYCLES)
                }
                else {
                    warn!("Writing to an unmapped scratchpad address: {:08X}",offset);
                    WriteMemoryAccess::BusError
                }
            },
            // BIOS ======================================================================================
            MemorySection::BIOSRom => {
                // BIOS is 512K
                if offset < 0x80_000 {
                    WriteMemoryAccess::WriteErrorReadOnly(BIOS8_READ_CYCLES) // just to put a value for penalty
                }
                else {
                    warn!("Writing to an unmapped BIOS address: {:08X}",offset);
                    WriteMemoryAccess::BusError
                }
            },
            // I/O =======================================================================================
            MemorySection::IOPorts => {
                self.io_mem_bridge.write[offset as usize](self,address,value,SIZE)
            },
            // Expansion 1 ===============================================================================
            MemorySection::ExpansionRegion1 => {
                debug!("Writing Expansion 1 {:08X} = {:08X}",address,value);
                // TODO
                let penalty = match SIZE {
                    8 => EXP1_8_ACCESS_CYCLES,
                    16 => EXP1_16_ACCESS_CYCLES,
                    32 => EXP1_32_ACCESS_CYCLES,
                    _ => 0,
                };
                WriteMemoryAccess::Write(penalty)
            },
            // Expansion 2 ===============================================================================
            MemorySection::ExpansionRegion2 => {
                debug!("Writing Expansion 2 {:08X} = {:08X}",address,value);
                // TODO
                let penalty = match SIZE {
                    8 => EXP2_8_ACCESS_CYCLES,
                    16 => EXP2_16_ACCESS_CYCLES,
                    32 => EXP2_32_ACCESS_CYCLES,
                    _ => 0,
                };
                WriteMemoryAccess::Write(penalty)
            },
            // Expansion 3 ===============================================================================
            MemorySection::ExpansionRegion3 => {
                debug!("Writing Expansion 3 {:08X} = {:08X}",address,value);
                // TODO
                let penalty = match SIZE {
                    8 => EXP3_8_ACCESS_CYCLES,
                    16 => EXP3_16_ACCESS_CYCLES,
                    32 => EXP3_32_ACCESS_CYCLES,
                    _ => 0,
                };
                WriteMemoryAccess::Write(penalty)
            },
            // Cache Control =============================================================================
            MemorySection::CacheControl => {
                debug!("Writing CacheControl {:08X}",value);
                self.cache_control_reg = value;
                WriteMemoryAccess::Write(IO_REG_ACCESS_CYCLES)
            },
            MemorySection::Unmapped => {
                debug!("Writing to an unmapped address: {:08X} = {:08X}",address,value);
                WriteMemoryAccess::BusError
            }
        }
    }

    fn peek<const SIZE: usize>(&self, address: u32) -> Option<u32> {
        const { assert!(SIZE == 8 || SIZE == 16 || SIZE == 32) }
        // get address's info
        let MemoryMap(_segment,section,_base_address,offset) = get_memory_map(address);

        match section {
            // RAM =======================================================================================
            MemorySection::MainRAM => {
                if offset < 0x80_0000 { // first 8M
                    let ram_address = offset & 0x1F_FFFF; // // 2MB RAM can be mirrored to the first 8MB (strangely, enabled by default)
                    Some(<Self as Memory>::mem_read::<SIZE>(ram_address, &self.main_ram))
                } else {
                    None
                }
            },
            // Scratchpad ================================================================================
            MemorySection::ScratchPad => {
                if offset < 0x400 {
                    Some(<Self as Memory>::mem_read::<SIZE>(offset, &self.scratchpad))
                }
                else {
                    None
                }
            },
            // BIOS ======================================================================================
            MemorySection::BIOSRom => {
                // BIOS is 512K
                if offset < 0x80_000 {
                    Some(<Self as Memory>::mem_read::<SIZE>(offset, &self.bios.memory))
                } else {
                    None
                }
            },
            // I/O =======================================================================================
            MemorySection::IOPorts => {
                self.io_mem_bridge.peek[offset as usize](self,address)
            },
            // Expansion 1 ===============================================================================
            MemorySection::ExpansionRegion1 => {
                let value = match SIZE {
                    8 => 0xFF,
                    16 => 0xFFFF,
                    32 => 0xFFFFFFFF,
                    _ => 0,
                };
                Some(value)
            },
            // Expansion 2 ===============================================================================
            MemorySection::ExpansionRegion2 => {
                let value = match SIZE {
                    8 => 0xFF,
                    16 => 0xFFFF,
                    32 => 0xFFFFFFFF,
                    _ => 0,
                };
                Some(value)
            },
            // Expansion 3 ===============================================================================
            MemorySection::ExpansionRegion3 => {
                let value = match SIZE {
                    8 => 0xFF,
                    16 => 0xFFFF,
                    32 => 0xFFFFFFFF,
                    _ => 0,
                };
                Some(value)
            },
            // Cache Control =============================================================================
            MemorySection::CacheControl => {
                Some(self.cache_control_reg)
            },
            MemorySection::Unmapped => {
                None
            }
        }
    }
}