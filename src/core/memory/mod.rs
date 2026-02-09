use std::fs::File;
use std::io;
use std::io::Read;
use md5;
use crate::core::memory::MemorySection::{BIOSRom, ExpansionRegion1, ExpansionRegion2, ExpansionRegion3, IOPorts, MainRAM, ScratchPad, Unmapped};

pub mod bus;

pub const BIOS_LEN : usize = 512 * 1024;
// memory access time
pub const MEM_READ_MAIN_CYCLES : usize = 5;
pub const MEM_READ_BIOS_CYCLES : usize = 24;

/*
Memory Access time (from ps1-tests)
------------------------------------------------------------------------------
SEGMENT    (   ADDRESS)    8bit    16bit    32bit    (cpu cycles per bitsize)
RAM        (0x80000000)    5.21     5.3      5.14
BIOS       (0xbfc00000)    7.6     12.94    24.94
SCRATCHPAD (0x1f800000)    1.5      1.1      0.94
EXPANSION1 (0x1f000000)    6.94    13.7     25.7
EXPANSION2 (0x1f802000)   10.99    25.99    55.98
EXPANSION3 (0x1fa00000)    6.7      6.1      9.95
DMAC_CTRL  (0x1f8010f0)    3.8      3.8      3.1
JOY_STAT   (0x1f801044)    3.1      3.1      3.2
SIO_STAT   (0x1f801054)    3.2      3.2      2.92
RAM_SIZE   (0x1f801060)    3.4      3.4      3.18
I_STAT     (0x1f801070)    3.8      3.8      3.1
TIMER0_VAL (0x1f801100)    3.1      3.1      3.2
CDROM_STAT (0x1f801800)    8.0     14.0     25.93
GPUSTAT    (0x1f801814)    2.93     2.92     3.8
MDECSTAT   (0x1f801824)    3.8      3.8      3.1
SPUCNT     (0x1f801daa)   17.99    17.99    38.94
CACHECTRL  (0xfffe0130)    0.95     1.9      1.9

Memory Access Timing
------------------------------------------------------------------
Contents                    Number      Penalty Required Number
                            of Words    Cycles  of Cycles
Reads
I-Cache → CPU               1           0       0
ScratchPad → CPU            1           0       0
Main Memory → I-Cache       1           5       4
                            2           6       5
                            3           7       6
                            4           8       7
Main Memory → CPU           1           5       4
Writes
CPU → W Buffer              1           0       0
CPU → ScratchPad            1           0       0
W-Buffer → Main Memory      1           0       4
*/
#[derive(PartialEq,Debug)]
pub enum MemorySection {
    MainRAM,            // 2048K Main RAM (first 64K reserved for BIOS)
    ExpansionRegion1,   // 8192K Expansion Region 1 (ROM/RAM)
    ScratchPad,         // 1K Scratchpad (D-Cache used as Fast RAM)
    IOPorts,            // 8K I/O Ports
    ExpansionRegion2,   // 8K Expansion Region 2 (I/O Ports)
    ExpansionRegion3,   // 2048K Expansion Region 3
    BIOSRom,            // 512K BIOS ROM (Kernel) (4096K max)
    CacheControl,       // 0.5K I/O Ports (Cache Control)
    Unmapped,
}

#[derive(PartialEq,Debug)]
pub enum MemorySegment {
    KUSEG,
    KUSEG0,
    KUSEG1,
    KUSEG2,
}

impl MemorySegment {
    pub fn is_kernel(&self) -> bool {
        use MemorySegment::*;
        match self {
            KUSEG => false,
            _ => true,
        }
    }

    pub fn is_user(&self) -> bool {
        use MemorySegment::*;
        match self {
            KUSEG => true,
            _ => false,
        }
    }

    pub fn is_cached(&self) -> bool {
        use MemorySegment::*;
        match self {
            KUSEG | KUSEG0 => true,
            _ => false,
        }
    }
}

pub struct MemoryMap(MemorySegment, MemorySection,u32,u32);

/*
  KUSEG     KSEG0     KSEG1
  ----------------------------------------------------------------------------
  00000000h 80000000h A0000000h  2048K  Main RAM (first 64K reserved for BIOS)
  1F000000h 9F000000h BF000000h  8192K  Expansion Region 1 (ROM/RAM)
  1F800000h 9F800000h    --      1K     Scratchpad (D-Cache used as Fast RAM)
  1F801000h 9F801000h BF801000h  8K     I/O Ports
  1F802000h 9F802000h BF802000h  8K     Expansion Region 2 (I/O Ports)
  1FA00000h 9FA00000h BFA00000h  2048K  Expansion Region 3 (whatever purpose)
  1FC00000h 9FC00000h BFC00000h  512K   BIOS ROM (Kernel) (4096K max)
        FFFE0000h (KSEG2)        0.5K   I/O Ports (Cache Control)

  Address   Name   Size   Privilege    Code-Cache  Data-Cache
  ----------------------------------------------------------------------------
  00000000h KUSEG  2048M  Kernel/User  Yes         (Scratchpad)
  80000000h KSEG0  512M   Kernel       Yes         (Scratchpad)
  A0000000h KSEG1  512M   Kernel       No          No
  C0000000h KSEG2  1024M  Kernel       (No code)   No
 */
#[inline(always)]
pub fn get_memory_seg(address:u32) -> MemorySegment {
    let msb = address >> 29;
    match msb {
        0b000 => MemorySegment::KUSEG,
        0b100 => MemorySegment::KUSEG0,
        0b101 => MemorySegment::KUSEG1,
        _ => MemorySegment::KUSEG2,
    }
}
#[inline(always)]
pub fn get_memory_section(address:u32) -> MemorySection {
    // clear msb
    let address = address & 0x1FFFFFFF;

    if address < 0x1F000000 {
        MainRAM
    }
    else if address < 0x1F800000 {
        ExpansionRegion1
    }
    else if address < 0x1F801000 {
        ScratchPad
    }
    else if address < 0x1F802000 {
        IOPorts
    }
    else if address < 0x1FA00000 {
        ExpansionRegion2
    }
    else if address < 0x1FC00000 {
        ExpansionRegion3
    }
    else if address < 0x20000000 {
        BIOSRom
    }
    else {
        Unmapped
    }
}

#[inline(always)]
pub fn get_memory_map(address:u32) -> MemoryMap {
    use MemorySection::*;
    use MemorySegment::*;
    let mut address = address;
    let region;
    let section;
    let offset;

    let msb = address >> 29;
    match msb {
        0b000 => region = KUSEG,
        0b100 => region = KUSEG0,
        0b101 => region = KUSEG1,
        _ => {
            return if address == 0xFFFE0130 {
                MemoryMap(KUSEG2, CacheControl, address,address)
            } else {
                MemoryMap(KUSEG2, Unmapped, address,address)
            }
        }
    }

    // clear msb
    address &= 0x1FFFFFFF;

    if address < 0x1F000000 {
        section = MainRAM;
        offset = address;
    }
    else if address < 0x1F800000 {
        section = ExpansionRegion1;
        offset = address - 0x1F000000;
    }
    else if address < 0x1F801000 {
        section = ScratchPad;
        offset = address - 0x1F800000;
    }
    else if address < 0x1F802000 {
        section = IOPorts;
        offset = address - 0x1F801000;
    }
    else if address < 0x1FA00000 {
        section = ExpansionRegion2;
        offset = address - 0x1F802000;
    }
    else if address < 0x1FC00000 {
        section = ExpansionRegion3;
        offset = address - 0x1FA00000;
    }
    else if address < 0x20000000 {
        section = BIOSRom;
        offset = address - 0x1FC00000;
    }
    else {
        section = Unmapped;
        offset = address - 0x20000000;
    }

    MemoryMap(region,section,address,offset)
}

#[derive(Debug)]
pub enum ReadMemoryAccess {
    Read(u32,usize),
    BusError,
    MemoryError,
    Wait,
}
#[derive(Debug)]
pub enum WriteMemoryAccess {
    Write(usize),
    WriteErrorReadOnly(usize),
    BusError,
    MemoryError,
    InvalidateICacheTag,
    InvalidateICacheOpcode,
    Wait,
}

// All the memory access use little-endian
pub trait Memory {
    fn read<const SIZE: usize>(&mut self, address:u32,is_fetching:bool) -> ReadMemoryAccess;
    fn write<const SIZE: usize>(&mut self, address:u32, value:u32) -> WriteMemoryAccess;
    fn peek<const SIZE: usize>(&self, address:u32) -> Option<u32>;

    fn mem_read<const SIZE: usize>(address:u32,memory:&Vec<u8>) -> u32 {
        const { assert!(SIZE == 8 || SIZE == 16 || SIZE == 32) }
        let mut mem_address = address as usize; //(address % memory.len() as u32) as usize;
        /*
        let address = address as usize;
        match SIZE {
            8 => {
                memory[address] as u32
            },
            16 => {
                memory[address] as u32 | (memory[address + 1] as u32) << 8
            }
            32 => {
                memory[address] as u32 | (memory[address + 1] as u32) << 8 | (memory[address + 2] as u32) << 16 | (memory[address + 3] as u32) << 24
            }
            _ => 0
        }

         */

        let bytes = SIZE >> 3;
        let mut result : u32 = 0;
        for i in 0..bytes {
            result |= (memory[mem_address] as u32) << (i << 3);
            mem_address += 1; //= (mem_address + 1) % memory.len();
        }
        result
    }

    fn mem_write<const SIZE: usize>(address:u32,value:u32,memory:&mut Vec<u8>) {
        const { assert!(SIZE == 8 || SIZE == 16 || SIZE == 32) }
        /*
        let address = address as usize;
        match SIZE {
            8 => {
                memory[address] = value as u8;
            },
            16 => {
                memory[address] = value as u8;
                memory[address + 1] = (value >> 8) as u8;
            }
            32 => {
                memory[address] = value as u8;
                memory[address + 1] = (value >> 8) as u8;
                memory[address + 2] = (value >> 16) as u8;
                memory[address + 3] = (value >> 24) as u8;
            }
            _ => {}
        }

         */
        let mut mem_address = address as usize; //(address % memory.len() as u32) as usize;
        let bytes = SIZE >> 3;
        for i in 0..bytes {
            memory[mem_address] = (value >> (i << 3)) as u8;
            mem_address += 1; //= (mem_address + 1) % memory.len();
        }
    }
}

pub struct ArrayMemory {
    pub memory: Vec<u8>,
    pub md5: String,
    pub read_only: bool,
    read_penalty: usize,
    write_penalty: usize,
}

impl ArrayMemory {
    pub fn new(bytes:&[u8],read_only:bool,read_penalty: usize,write_penalty: usize) -> Self {
        let digest = md5::compute(&bytes);
        ArrayMemory {
            memory: bytes.to_vec(),
            md5: format!("{:X}", digest),
            read_only,
            read_penalty,
            write_penalty,
        }
    }
    pub fn load_from_file(path: &str,expected_len:usize,read_only:bool,read_penalty: usize,write_penalty: usize) -> io::Result<Self> {
        let mut file = File::open(path)?;
        let mut memory = Vec::new();
        let n = file.read_to_end(&mut memory)?;
        if n != expected_len {
            Err(io::Error::new(io::ErrorKind::UnexpectedEof, format!("BIOS file is not the correct size: found {}, expected {}",n,expected_len)))
        }
        else {
            let digest = md5::compute(&memory);
            Ok(ArrayMemory {
                memory,
                md5: format!("{:X}", digest),
                read_only,
                read_penalty,
                write_penalty,
            })
        }
    }

    pub fn update_m5(&mut self) {
        self.md5 = format!("{:X}",md5::compute(&self.memory))
    }
}

impl Memory for ArrayMemory {
    fn read<const SIZE: usize>(&mut self, address:u32,_is_fetching:bool) -> ReadMemoryAccess {
        ReadMemoryAccess::Read(<Self as Memory>::mem_read::<SIZE>(address, &self.memory), self.read_penalty)
    }
    fn write<const SIZE: usize>(&mut self, address:u32, value:u32) -> WriteMemoryAccess {
        if !self.read_only {
            <Self as Memory>::mem_write::<SIZE>(address, value, &mut self.memory);
            WriteMemoryAccess::Write(self.read_penalty)
        }
        else {
            WriteMemoryAccess::WriteErrorReadOnly(self.write_penalty)
        }
    }
    fn peek<const SIZE: usize>(&self, address:u32) -> Option<u32> {
        Some(<Self as Memory>::mem_read::<SIZE>(address, &self.memory))
    }
}

