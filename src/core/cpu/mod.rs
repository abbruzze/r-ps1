use crate::core::cpu::cop2::Cop2;
use crate::core::cpu::instruction::{Instruction, Opcode};
use crate::core::memory;
use crate::core::memory::{Memory, MemorySection, ReadMemoryAccess, WriteMemoryAccess};
use std::mem;
use tracing::{debug, error, info, warn};
use crate::core::memory::bus::Bus;

pub mod instruction;
pub mod disassembler;
pub mod cop0;
mod cop2;

pub const RESET_ADDRESS : u32 = 0xBFC00000;
const MUL_AVERAGE_CYCLES : usize = 9;
const DIV_CYCLES : usize = 36;
const ICACHE_LINE_CYCLES : usize = 8;
const QUEUE_WRITE_MEM_CYCLES: usize = 4;

const HW_BREAKPOINT_VALUE : u32 = 0xFFFFFFFF;

#[derive(Debug)]
pub enum CpuException {
    Interrupt,
    AddressErrorLoad(u32),      // Address Error Exception (Load or instruction fetch)
    AddressErrorStore(u32),     // Address Error Exception (Store)
    BusErrorFetch(u32),         // Bus Error Exception (for Instruction Fetch)
    BusErrorData(u32),          // Bus Error Exception (for data Load or Store)
    SysCall(u32),               // SYSCALL Exception
    BreakPoint(u32),            // Breakpoint Exception
    ReservedInstruction(u32),        // Reserved Instruction Exception
    CoprocessorUnusable(usize), // Co-Processor Unusable Exception
    ArithmeticOverflow,         // Arithmetic Overflow Exception
    Reset,
    // internal only
    ReadWriteWait,
}

impl CpuException {
    pub fn id(&self) -> usize {
        use CpuException::*;
        match self {
            Interrupt => 0,
            AddressErrorLoad(_) => 4,
            AddressErrorStore(_) => 5,
            BusErrorFetch(_) => 6,
            BusErrorData(_) => 7,
            SysCall(_) => 8,
            BreakPoint(_) => 9,
            ReservedInstruction(_) => 10,
            CoprocessorUnusable(_) => 11,
            ArithmeticOverflow => 12,
            Reset => 32,
            _ => unreachable!()
        }
    }
}

type OperationException = Result<(),CpuException>;

/*******************************************************************
                        COPROCESSOR
*******************************************************************/
pub struct CopResult(u32,usize); // value read or 0 in case of write/execute, cycles penalty

pub trait Coprocessor {
    fn get_name(&self) -> &'static str;
    fn read_data_register(&self, reg_index:usize) -> CopResult;
    fn write_data_register(&mut self, reg_index:usize, value: u32) -> CopResult;
    fn read_control_register(&self, reg_index:usize) -> CopResult;
    fn write_control_register(&mut self, reg_index:usize, value: u32) -> CopResult;
    fn execute_command(&mut self,cmd:u32) -> CopResult;
}

/*******************************************************************
                        I-CACHE
*******************************************************************/
#[derive(Copy,Clone)]
struct CacheLine {
    valid: bool,
    tag: u32, // [31:12]
    line: [u32;4],
}

impl CacheLine {
    fn new() -> Self {
        CacheLine {
            valid : false,
            tag : 0,
            line : [0;4]
        }
    }

    fn reset(&mut self) {
        self.valid = false;
    }
}

struct ICache {
    lines: [CacheLine;256],
    cache_miss: usize,
    requests: usize,
}

struct ICacheResult(u32,usize,bool); // value read, cycle penalty

impl ICache {
    fn new() -> Self {
        // let mut lines = Vec::with_capacity(256);
        // for _ in 0..256 {
        //     lines.push(CacheLine::new());
        // }
        ICache { lines: [CacheLine::new();256], cache_miss: 0, requests: 0 }
    }
    /*
        Address:
            31              12           4      0
            -------------------------------------
            |      tag       |    index  | ofs  |
            -------------------------------------
            tag = 31:12
            index = 11:4
            offset = 3:0
     */
    fn read<M : Memory>(&mut self, address: u32,memory:&mut M) -> ICacheResult {
        self.requests += 1;
        let index = (address >> 4) & 0xFF; // 256 lines
        let tag = address >> 12; // [31:12]
        let line = &mut self.lines[index as usize];
        let offset = ((address & 0x0F) >> 2) as usize; // 16 bytes, 4 words
        let mut penalty = 0usize;

        let mut cache_hit = true;
        if !line.valid || line.tag != tag {
            // Cache miss
            cache_hit = false;
            self.cache_miss += 1;
            line.valid = true;
            line.tag = tag;
            penalty += ICACHE_LINE_CYCLES;
            let mut base_address = address & !0xF;
            // Loads data into the cache line
            for i in 0..4 {
                line.line[i] = match memory.read::<32>(base_address,true) {
                    ReadMemoryAccess::Read(data,_) => {
                        data
                    },
                    ReadMemoryAccess::BusError | ReadMemoryAccess::MemoryError => {
                        error!("BusError while reading memory for ICache");
                        0
                    }
                    ReadMemoryAccess::Wait => {
                        error!("Wait while reading memory for ICache");
                        0
                    }
                };
                base_address += 4; // next word
            }
        }
        ICacheResult(line.line[offset],penalty,cache_hit)
    }

    fn invalidate_tag(&mut self,address:u32) {
        let index = (address >> 4) & 0xFF; // 256 lines
        let line = &mut self.lines[index as usize];
        debug!("Invalidating tag i-cache at {:04X} index = {}",address,index);
        line.valid = false;
    }

    fn write_opcode(&mut self,address:u32,opcode: u32) {
        let index = (address >> 4) & 0xFF; // 256 lines
        let line = &mut self.lines[index as usize];
        let offset = ((address & 0x0F) >> 2) as usize; // 16 bytes, 4 words
        debug!("Writing opcode to i-cache at {:04X} with {:04X} index = {} offset = {}",address,opcode,index,offset);
        line.line[offset] = opcode;
    }

    fn reset(&mut self) {
        self.requests = 0;
        self.cache_miss = 0;
        for line in self.lines.iter_mut() {
            line.reset();
        }
    }

    fn cache_miss_perc(&self) -> f32 {
        self.cache_miss as f32 / self.requests as f32 * 100.0
    }
}

/*******************************************************************
                        WRITE-QUEUE

Writes to memory addresses in kuseg and kseg0 are performed asynchronously so that software can continue to execute
while the hardware persists the write to memory.
The R3000 has a 4-word queue for writes to cached memory addresses,
and the CPU will only stall on writes if the write queue is full.

Reading from a kuseg/kseg0 address that has a pending write in the write queue will stall the CPU until the write is applied,
to guarantee that the CPU does not read stale data.
Additionally, reading from any uncached address (i.e. kseg1) will stall the CPU until the entire write queue is flushed.
*******************************************************************/
struct WriteQueue {
    queue: [(u32, u32, usize); 4],  // Array fisso
    head: usize,
    tail: usize,
    len: usize,
}

impl WriteQueue {
    fn new() -> Self {
        WriteQueue {
            queue: [(0, 0, 0); 4],
            head: 0,
            tail: 0,
            len: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn is_full(&self) -> bool {
        self.len == 4
    }

    fn enqueue(&mut self, address: u32, value: u32, byte_size: usize) {
        if self.len < 4 {
            self.queue[self.tail] = (address, value, byte_size);
            self.tail = (self.tail + 1) & 3;  // Modulo 4 con AND
            self.len += 1;
        }
    }

    fn peek_address(&self) -> Option<u32> {
        if self.len == 0 {
            return None;
        }
        Some(self.queue[self.head].0)
    }

    fn peek(&self) -> Option<(u32,u32,usize)> {
        if self.len == 0 {
            return None;
        }
        let item = self.queue[self.head];
        Some(item)
    }

    fn dequeue(&mut self) -> Option<(u32, u32, usize)> {
        if self.len == 0 {
            return None;
        }
        let item = self.queue[self.head];
        self.head = (self.head + 1) & 3;
        self.len -= 1;
        Some(item)
    }

    fn exists_address(&self, address_to_find: u32) -> bool {
        if self.len == 0 {
            return false;
        }

        // Unrolled loop - controlla fino a 4 elementi
        let check_entry = |idx: usize| -> bool {
            let (address, _, byte_size) = self.queue[idx];
            address_to_find >= address && address_to_find < address + byte_size as u32
        };

        match self.len {
            1 => check_entry(self.head),
            2 => {
                check_entry(self.head) ||
                    check_entry((self.head + 1) & 3)
            },
            3 => {
                check_entry(self.head) ||
                    check_entry((self.head + 1) & 3) ||
                    check_entry((self.head + 2) & 3)
            },
            4 => {
                check_entry(0) || check_entry(1) || check_entry(2) || check_entry(3)
            },
            _ => unreachable!(),
        }
    }

    fn reset(&mut self) {
        self.head = 0;
        self.tail = 0;
        self.len = 0;
    }
}

pub struct Cpu {
    op_functions: [fn(&mut Cpu,&mut Bus,&Instruction,bool) -> OperationException; 80],
    cop2: Cop2,
    i_cache: ICache,
    write_queue: WriteQueue,
    pc: u32,
    regs: [u32; 32],
    hi: u32,
    lo: u32,
    delayed_load: (usize,u32),
    delayed_load_next : (usize,u32),
    branch_address:u32,
    branch_taken: bool,
    in_branch_delay_slot: bool,
    mul_div_pending_cycles: usize,
    op_cycles: usize,
    write_queue_elapsed: usize,
    bios_tty_capture_enabled: bool,
    bios_tty_buffer: String,
    last_mem_read_address : Option<u32>,
    last_mem_write_address : Option<u32>,
    last_mem_rw_value: u32,
    last_opcode: u32,
    cop2_remaining_cycles: usize,
}

impl Cpu {
    pub fn new() -> Self {
        let mut cpu = Cpu {
            op_functions: [Cpu::op_nop;80],
            cop2: Cop2::new(),
            i_cache: ICache::new(),
            write_queue: WriteQueue::new(),
            pc : RESET_ADDRESS,
            regs: [0; 32],
            hi: 0,
            lo: 0,
            delayed_load: (0,0),
            delayed_load_next: (0,0),
            branch_address: 0,
            branch_taken: false,
            in_branch_delay_slot: false,
            mul_div_pending_cycles: 0,
            op_cycles: 0,
            write_queue_elapsed: 0,
            bios_tty_capture_enabled: false,
            bios_tty_buffer: String::new(),
            last_mem_read_address: None,
            last_mem_write_address: None,
            last_mem_rw_value: 0,
            last_opcode: 0,
            cop2_remaining_cycles: 0,
        };

        cpu.init_op_functions();

        cpu
    }

    fn init_op_functions(&mut self) {
        use Opcode::*;
        // lui
        self.op_functions[LUI as usize] = Cpu::op_lui;
        // shift immediate
        self.op_functions[SLL as usize] = Cpu::op_shift::<true,false,true>;
        self.op_functions[SRL as usize] = Cpu::op_shift::<false,false,true>;
        self.op_functions[SRA as usize] = Cpu::op_shift::<false,true,true>;
        // shift register
        self.op_functions[SLLV as usize] = Cpu::op_shift::<true,false,false>;
        self.op_functions[SRLV as usize] = Cpu::op_shift::<false,false,false>;
        self.op_functions[SRAV as usize] = Cpu::op_shift::<false,true,false>;
        // jumps
        self.op_functions[JR as usize] = Cpu::op_jr;
        self.op_functions[JALR as usize] = Cpu::op_jalr;
        // syscall / break
        self.op_functions[SYSCALL as usize] = Cpu::op_syscall;
        self.op_functions[BREAK as usize] = Cpu::op_break;
        // mfhi / mflo
        self.op_functions[MFHI as usize] = Cpu::op_mf_lohi::<false>;
        self.op_functions[MFLO as usize] = Cpu::op_mf_lohi::<true>;
        // mthi / mtlo
        self.op_functions[MTHI as usize] = Cpu::op_mt_lohi::<false>;
        self.op_functions[MTLO as usize] = Cpu::op_mt_lohi::<true>;
        // mult / multu
        self.op_functions[MULT as usize] = Cpu::op_mult::<true>;
        self.op_functions[MULTU as usize] = Cpu::op_mult::<false>;
        // div / divu
        self.op_functions[DIV as usize] = Cpu::op_div::<true>;
        self.op_functions[DIVU as usize] = Cpu::op_div::<false>;
        // add / addu
        self.op_functions[ADD as usize] = Cpu::op_arithmetic::<true,true,false>;
        self.op_functions[ADDU as usize] = Cpu::op_arithmetic::<true,false,false>;
        // sub / subu
        self.op_functions[SUB as usize] = Cpu::op_arithmetic::<false,true,false>;
        self.op_functions[SUBU as usize] = Cpu::op_arithmetic::<false,false,false>;
        // addi / addiu
        self.op_functions[ADDI as usize] = Cpu::op_arithmetic::<true,true,true>;
        self.op_functions[ADDIU as usize] = Cpu::op_arithmetic::<true,false,true>;
        // logical
        self.op_functions[AND as usize] = Cpu::op_and;
        self.op_functions[OR as usize] = Cpu::op_or;
        self.op_functions[XOR as usize] = Cpu::op_xor;
        self.op_functions[NOR as usize] = Cpu::op_nor;
        self.op_functions[ANDI as usize] = Cpu::op_andi;
        self.op_functions[ORI as usize] = Cpu::op_ori;
        self.op_functions[XORI as usize] = Cpu::op_xori;
        // compare
        self.op_functions[SLTU as usize] = Cpu::op_compare::<false,false>;
        self.op_functions[SLT as usize] = Cpu::op_compare::<true,false>;
        self.op_functions[SLTI as usize] = Cpu::op_compare::<true,true>;
        self.op_functions[SLTIU as usize] = Cpu::op_compare::<false,true>;
        // branches
        self.op_functions[BLTZ as usize] = Cpu::op_bltz;
        self.op_functions[BGEZ as usize] = Cpu::op_bgez;
        self.op_functions[BLTZAL as usize] = Cpu::op_bltzal;
        self.op_functions[BGEZAL as usize] = Cpu::op_bgezal;
        self.op_functions[BEQ as usize] = Cpu::op_beq;
        self.op_functions[BNE as usize] = Cpu::op_bne;
        self.op_functions[BLEZ as usize] = Cpu::op_blez;
        self.op_functions[BGTZ as usize] = Cpu::op_bgtz;
        self.op_functions[J as usize] = Cpu::op_j::<false>;
        self.op_functions[JAL as usize] = Cpu::op_j::<true>;
        // mfc / cfc
        self.op_functions[MFCn as usize] = Cpu::op_mc_fc_n::<false>;
        self.op_functions[CFCn as usize] = Cpu::op_mc_fc_n::<true>;
        // mtc / ctc
        self.op_functions[MTCn as usize] = Cpu::op_mc_tc_n::<false>;
        self.op_functions[CTCn as usize] = Cpu::op_mc_tc_n::<true>;
        // bcnf / bctf
        self.op_functions[BCnF as usize] = Cpu::op_bcn_ft::<true>;
        self.op_functions[BCnT as usize] = Cpu::op_bcn_ft::<false>;
        // copn
        self.op_functions[TLBR as usize] = Cpu::op_copn;
        self.op_functions[TLBWI as usize] = Cpu::op_copn;
        self.op_functions[TLBWR as usize] = Cpu::op_copn;
        self.op_functions[TLBP as usize] = Cpu::op_copn;
        self.op_functions[RFE as usize] = Cpu::op_copn;
        self.op_functions[COPn as usize] = Cpu::op_copn;
        // lb / lh / lw / lbu / lhu
        self.op_functions[LB as usize] = Cpu::op_lb::<true>;
        self.op_functions[LBU as usize] = Cpu::op_lb::<false>;
        self.op_functions[LH as usize] = Cpu::op_lh::<true>;
        self.op_functions[LHU as usize] = Cpu::op_lh::<false>;
        self.op_functions[LW as usize] = Cpu::op_lw;
        // lwr / lwl
        self.op_functions[LWR as usize] = Cpu::op_lwr;
        self.op_functions[LWL as usize] = Cpu::op_lwl;
        // sb / sh / sw
        self.op_functions[SB as usize] = Cpu::op_sb;
        self.op_functions[SH as usize] = Cpu::op_sh;
        self.op_functions[SW as usize] = Cpu::op_sw;
        // swl / swr
        self.op_functions[SWL as usize] = Cpu::op_swl;
        self.op_functions[SWR as usize] = Cpu::op_swr;
        // lwc
        self.op_functions[LWC0 as usize] = Cpu::op_lwc::<0>;
        self.op_functions[LWC1 as usize] = Cpu::op_lwc::<1>;
        self.op_functions[LWC2 as usize] = Cpu::op_lwc::<2>;
        self.op_functions[LWC3 as usize] = Cpu::op_lwc::<3>;
        // swc
        self.op_functions[SWC0 as usize] = Cpu::op_swc::<0>;
        self.op_functions[SWC1 as usize] = Cpu::op_swc::<1>;
        self.op_functions[SWC2 as usize] = Cpu::op_swc::<2>;
        self.op_functions[SWC3 as usize] = Cpu::op_swc::<3>;

    }

    pub fn set_bios_tty_capture_enabled(&mut self, bios_tty_capture_enabled: bool) {
        info!("Cpu, BIOS tty enabled: {bios_tty_capture_enabled}");
        self.bios_tty_capture_enabled = bios_tty_capture_enabled;
    }

    pub fn get_last_elapsed_cycles(&self) -> usize {
        self.op_cycles
    }

    pub fn get_pc(&self) -> u32 {
        self.pc
    }

    pub fn get_last_mem_read_address(&self) -> Option<u32> {
        self.last_mem_read_address
    }

    pub fn get_last_mem_write_address(&self) -> Option<u32> {
        self.last_mem_write_address
    }

    pub fn get_last_mem_rw_value(&self) -> u32 {
        self.last_mem_rw_value
    }
    
    pub fn set_pc(&mut self, pc:u32) {
        self.pc = pc;
    }

    pub fn get_lo(&self) -> u32 {
        self.lo
    }

    pub fn get_hi(&self) -> u32 {
        self.hi
    }

    pub fn get_last_opcode(&self) -> u32 {
        self.last_opcode
    }

    pub fn reset(&mut self) {
        self.pc = RESET_ADDRESS;
        //self.cop0.borrow_mut().reset();
        self.i_cache.reset();
        self.write_queue.reset();
        self.write_queue_elapsed = 0;
        self.mul_div_pending_cycles = 0;
        self.hi = 0;
        self.lo = 0;
        self.regs.fill(0);
        self.delayed_load = (0,0);
        self.delayed_load_next = (0,0);
    }
    
    pub fn get_registers(&self) -> &[u32;32] {
        &self.regs
    }

    pub fn get_registers_mut(&mut self) -> &mut[u32;32] {
        &mut self.regs
    }

    fn cache_invalidate_tag(&mut self,address:u32) {
        self.i_cache.invalidate_tag(address);
    }

    fn cache_write_opcode(&mut self,address:u32,opcode: u32) {
        self.i_cache.write_opcode(address,opcode);
    }

    pub fn execute_next_instruction(&mut self,memory: &mut Bus,dma_in_progress:bool) -> usize {
        // check pending operations during last op_cycles
        self.update_pending_operations(memory,dma_in_progress);
        // reset cycle count
        self.op_cycles = 1;
        self.last_mem_read_address = None;
        self.last_mem_write_address = None;
        // check PC address alignment
        if (self.pc & 3) != 0 {
            self.handle_exception(memory,CpuException::AddressErrorLoad(self.pc),self.in_branch_delay_slot,self.branch_address,self.last_opcode);
            self.apply_delayed_load();
            return self.op_cycles
        }
        // Interrupt handling ===================================================================================
        if memory.get_cop0_mut().check_interrupt_pending() {
            // TODO check if the operation is GTE related ...
            self.handle_exception(memory,CpuException::Interrupt,self.in_branch_delay_slot,self.branch_address,self.last_opcode);
            self.apply_delayed_load();
            return self.op_cycles;
        }
        // opcode fetching ======================================================================================
        /*
            Address   Name   i-Cache     Write-Queue
          00000000h KUSEG       Yes         Yes
          80000000h KSEG0       Yes         Yes
          A0000000h KSEG1       No          No
          C0000000h KSEG2       (No code)   No
         */
        let pc_k_segment = memory::get_memory_seg(self.pc);
        self.last_opcode = if pc_k_segment.is_cached() {
            let ICacheResult(op,cycles,hit) = self.i_cache.read(self.pc,memory);
            if dma_in_progress && !hit {
                return self.op_cycles;
            }
            self.op_cycles += cycles;
            op
        }
        else { // opcode is fetched directly from non-cached memory
            if dma_in_progress {
                return self.op_cycles;
            }
            let mem_read = memory.read::<32>(self.pc,true);
            match mem_read {
                ReadMemoryAccess::Read(op,penalty_cycles) => {
                    self.op_cycles += penalty_cycles;
                    op
                }
                ReadMemoryAccess::BusError | ReadMemoryAccess::MemoryError => {
                    self.handle_exception(memory,CpuException::BusErrorFetch(self.pc),self.in_branch_delay_slot,self.branch_address,self.last_opcode);
                    self.apply_delayed_load();
                    return self.op_cycles;
                }
                ReadMemoryAccess::Wait => {
                    error!("Wait while reading memory for opcode fetch at address {:08X}",self.pc);
                    0
                }
            }
        };

        // instruction decoding =================================================================================
        let i = Instruction(self.last_opcode);
        let mut use_write_cache = false;

        // exit conditions
        let opcode = Opcode::from_instruction(&i);
        // 1) unknown instruction
        if opcode == Opcode::UNKNOWN {
            error!("Unknown opcode during fetching: {:08X} at address: {:08X}",self.last_opcode,self.pc);
            self.handle_exception(memory,CpuException::ReservedInstruction(i.0),self.in_branch_delay_slot,self.branch_address,i.0);
            self.apply_delayed_load();
            return self.op_cycles;
        }

        // 2)check hi/low access during in progress div/mul operation
        if opcode.is_accessing_hi_low() && self.mul_div_pending_cycles > 0 {
            self.apply_delayed_load();
            return self.op_cycles;
        }
        // 3) writing to full write-queue
        if opcode.is_write_memory() {
            let target_address = self.get_read_write_memory_address(&i);
            use_write_cache = memory::get_memory_seg(target_address).is_cached();
            if !use_write_cache && dma_in_progress && !matches!(memory::get_memory_section(target_address),MemorySection::ScratchPad) {
                return self.op_cycles;
            }
            if use_write_cache && self.write_queue.is_full() { // writing to a cached memory address with queue full
                self.apply_delayed_load();
                return self.op_cycles;
            }
        }
        // 4) reading from an uncached address or reading from cached address that is pending in the write-queue
        else if opcode.is_read_memory() {
            let target_address = self.get_read_write_memory_address(&i);
            if memory::get_memory_seg(target_address).is_cached() {
                if self.write_queue.exists_address(target_address) {
                    self.apply_delayed_load();
                    return self.op_cycles;
                }
            }
            if dma_in_progress {
                // During DMA, any read access from RAM or I/O registers or filling more than 4 entries into the write queue will stall the CPU until the DMA is finished.
                match memory::get_memory_section(target_address) {
                    MemorySection::IOPorts | MemorySection::MainRAM => {
                        return self.op_cycles;
                    }
                    _ => {}
                }
            }
            // reads from uncached memory always wait for the write queue to empty
            else if !self.write_queue.is_empty() {
                self.apply_delayed_load();
                return self.op_cycles;
            }
        }

        // check BIOS TTY
        if self.bios_tty_capture_enabled {
            let pc = self.pc & 0x1F_FFFFFF;
            if (pc == 0x000000A0 && self.regs[9] == 0x3C) || (pc == 0x000000B0 && self.regs[9] == 0x3D) {
                if self.regs[4] == 10  {
                    info!("[TTY] {}",self.bios_tty_buffer);
                    self.bios_tty_buffer.clear();
                }
                else {
                    if let Some(ch) = char::from_u32(self.regs[4]) {
                        self.bios_tty_buffer.push(ch);
                    }
                }
            }
        };

        // check hw breakpoint
        if memory.get_cop0_mut().is_at_pc_breakpoint(self.pc) {
            self.handle_exception(memory,CpuException::BreakPoint(HW_BREAKPOINT_VALUE),self.in_branch_delay_slot,self.branch_address,i.0);
            self.apply_delayed_load();
            return self.op_cycles;
        }

        // Determine next PC *before* executing the instruction in case the instruction is another branch/jump.
        // A small number of games do sadistically put branch instructions in branch delay slots!
        let was_in_branch_delay_slot = self.in_branch_delay_slot;
        self.in_branch_delay_slot = false;
        let next_pc= if was_in_branch_delay_slot && self.branch_taken {
            self.branch_address
        }
        else {
            self.pc.wrapping_add(4)
        };

        // execute instruction
        if let Err(ex) = self.op_functions[opcode as usize](self,memory,&i,use_write_cache) {
            if matches!(ex,CpuException::ReadWriteWait) {
                self.in_branch_delay_slot = was_in_branch_delay_slot;
                // stay on the same instruction and wait for the memory operation to complete, we will re-attempt to execute the same instruction on the next call to execute_next_instruction
            }
            self.handle_exception(memory, ex, was_in_branch_delay_slot, self.branch_address,self.last_opcode)
        }
        else {
            self.pc = next_pc;
            self.apply_delayed_load();
        }

        // return elapsed cycles
        self.op_cycles
    }

    // ==========================================================================

    fn update_pending_operations(&mut self,memory:&mut Bus,dma_in_progress:bool) {
        // mul/div operation
        if self.mul_div_pending_cycles > 0 {
            self.mul_div_pending_cycles = self.mul_div_pending_cycles.saturating_sub(self.op_cycles);
        }
        // gte remaining cycles
        if self.cop2_remaining_cycles > 0 {
            self.cop2_remaining_cycles = self.cop2_remaining_cycles.saturating_sub(self.op_cycles);
        }
        // write-queue
        if !self.write_queue.is_empty() {
            if dma_in_progress {
                if let Some(address) = self.write_queue.peek_address() {
                    if !matches!(memory::get_memory_section(address), MemorySection::ScratchPad) {
                        return;
                    }
                }
            }
            self.write_queue_elapsed += self.op_cycles;
            while self.write_queue_elapsed > QUEUE_WRITE_MEM_CYCLES {
                self.write_queue_elapsed -= QUEUE_WRITE_MEM_CYCLES;
                // perform a write to memory, we consider every write to memory a fixed 4 cycles penalty
                if let Some((address,value,byte_size)) = self.write_queue.peek() {
                    debug!("Cpu flushing queue ({byte_size}) {:08X} = {:08X}",address,value);
                    let res = match byte_size {
                        1 => self.write_data_memory::<8>(memory,address,value,false),
                        2 => self.write_data_memory::<16>(memory,address,value,false),
                        4 => self.write_data_memory::<32>(memory,address,value,false),
                        _ => {
                            unreachable!()
                        }
                    };
                    if let Err(CpuException::ReadWriteWait) = res {
                        break;
                    }
                    else {
                        self.write_queue.dequeue();
                    }
                }
            }
        }
    }
    #[inline(always)]
    fn write_reg(&mut self, register: usize, value: u32) {
        self.regs[register] = value;
        let (delayed_reg, _) = self.delayed_load;
        // if the register is a delayed one, cancel delayed load
        if delayed_reg == register {
            self.delayed_load = (0,0)
        }
        // reg[0] is always 0
        self.regs[0] = 0;
    }
    #[inline(always)]
    fn write_delayed_reg(&mut self, register: usize, value: u32) {
        self.delayed_load_next = (register,value);
        // If two consecutive load instructions write to the same register, cancel the first load
        let (delayed_reg, _) = self.delayed_load;
        if delayed_reg == register {
            self.delayed_load = (0,0)
        }
    }
    #[inline(always)]
    // Called after executing every instruction
    fn apply_delayed_load(&mut self) {
        let (delayed_reg, delayed_value) = self.delayed_load;
        self.regs[delayed_reg] = delayed_value;
        self.regs[0] = 0;
        self.delayed_load = mem::take(&mut self.delayed_load_next) // delayed_load <- delayed_load_next + delayed_load_next = (0,0)
    }

    // ================================================

    fn op_nop(&mut self,_memory:&mut Bus,_instruction: &Instruction,_use_write_cache:bool) -> OperationException {
        Ok(())
    }

    /*
    logical instructions
      and  rd,rs,rt    and  rd,rs,rt         rd = rs AND rt
      or   rd,rs,rt    or   rd,rs,rt         rd = rs OR  rt
      xor  rd,rs,rt    xor  rd,rs,rt         rd = rs XOR rt
      nor  rd,rs,rt    nor  rd,rs,rt         rd = FFFFFFFFh XOR (rs OR rt)
      and  rt,rs,imm   andi rt,rs,imm        rt = rs AND (0000h..FFFFh)
      or   rt,rs,imm   ori  rt,rs,imm        rt = rs OR  (0000h..FFFFh)
      xor  rt,rs,imm   xori rt,rs,imm        rt = rs XOR (0000h..FFFFh)
     */
    #[inline(always)]
    fn op_logical<F,const IMMEDIATE:bool>(&mut self,instr:&Instruction,log:F)
    where
        F: Fn(u32, u32) -> u32,
    {
        let rs = self.regs[instr.rs()];
        let rt = if IMMEDIATE {
            instr.unsigned_immediate16()
        } else {
            self.regs[instr.rt()]
        };
        if IMMEDIATE {
            self.write_reg(instr.rt(), log(rs, rt));
        }
        else {
            self.write_reg(instr.rd(), log(rs, rt));
        }
    }

    fn op_and(&mut self,_memory:&mut Bus,instruction: &Instruction,_use_write_cache:bool) -> OperationException {
        self.op_logical::<_,false>(instruction,|a,b| a & b);
        Ok(())
    }
    fn op_or(&mut self,_memory:&mut Bus,instruction: &Instruction,_use_write_cache:bool) -> OperationException {
        self.op_logical::<_,false>(instruction,|a,b| a | b);
        Ok(())
    }
    fn op_xor(&mut self,_memory:&mut Bus,instruction: &Instruction,_use_write_cache:bool) -> OperationException {
        self.op_logical::<_,false>(instruction,|a,b| a ^ b);
        Ok(())
    }
    fn op_nor(&mut self,_memory:&mut Bus,instruction: &Instruction,_use_write_cache:bool) -> OperationException {
        self.op_logical::<_,false>(instruction,|a,b| !(a | b));
        Ok(())
    }
    fn op_andi(&mut self,_memory:&mut Bus,instruction: &Instruction,_use_write_cache:bool) -> OperationException {
        self.op_logical::<_,true>(instruction,|a,b| a & b);
        Ok(())
    }
    fn op_ori(&mut self,_memory:&mut Bus,instruction: &Instruction,_use_write_cache:bool) -> OperationException {
        self.op_logical::<_,true>(instruction,|a,b| a | b);
        Ok(())
    }
    fn op_xori(&mut self,_memory:&mut Bus,instruction: &Instruction,_use_write_cache:bool) -> OperationException {
        self.op_logical::<_,true>(instruction,|a,b| a ^ b);
        Ok(())
    }

    // lui  rt,imm
    fn op_lui(&mut self,_memory:&mut Bus,instr: &Instruction,_use_write_cache:bool) -> OperationException {
        let imm = instr.unsigned_immediate16();
        self.write_reg(instr.rt(),imm << 16);
        Ok(())
    }

    /*
    shifting instructions
      shl  rd,rt,rs    sllv rd,rt,rs          rd = rt SHL (rs AND 1Fh)
      shr  rd,rt,rs    srlv rd,rt,rs          rd = rt SHR (rs AND 1Fh)
      sar  rd,rt,rs    srav rd,rt,rs          rd = rt SAR (rs AND 1Fh)
      shl  rd,rt,imm   sll  rd,rt,imm         rd = rt SHL (00h..1Fh)
      shr  rd,rt,imm   srl  rd,rt,imm         rd = rt SHR (00h..1Fh)
      sar  rd,rt,imm   sra  rd,rt,imm         rd = rt SAR (00h..1Fh)
     */
    fn op_shift<const SHIFT_LEFT:bool,const ARITHMETIC:bool,const IMMEDIATE:bool>(&mut self,_memory: &mut Bus, instr:&Instruction,_use_write_cache:bool) -> OperationException {
        let rs = if IMMEDIATE {
            instr.shift_amount()
        } else {
            self.regs[instr.rs()] & 0x1F
        };

        let rt = self.regs[instr.rt()];

        if SHIFT_LEFT {
            self.write_reg(instr.rd(),rt << rs);
        } else {
            if ARITHMETIC {
                self.write_reg(instr.rd(),((rt as i32) >> rs) as u32);
            } else {
                self.write_reg(instr.rd(),rt >> rs);
            }
        }

        Ok(())
    }

    /*
    comparison instructions
      setlt slt   rd,rs,rt  if rs<rt then rd=1 else rd=0 (signed)
      setb  sltu  rd,rs,rt  if rs<rt then rd=1 else rd=0 (unsigned)
      setlt slti  rt,rs,imm if rs<(-8000h..+7FFFh)  then rt=1 else rt=0 (signed)
      setb  sltiu rt,rs,imm if rs<(FFFF8000h..7FFFh) then rt=1 else rt=0(unsigned)
     */
    fn op_compare<const SIGNED:bool,const IMMEDIATE:bool>(&mut self,_memory:&mut Bus,instr: &Instruction,_use_write_cache:bool) -> OperationException {
        let mut result = 0;
        if SIGNED {
            let rs = self.regs[instr.rs()] as i32;
            let rt = if IMMEDIATE {
                instr.signed_immediate16() as i32
            }
            else {
                self.regs[instr.rt()] as i32
            };
            if rs < rt {
                result = 1;
            }
        }
        else {
            let rs = self.regs[instr.rs()];
            let rt = if IMMEDIATE {
                instr.signed_immediate16()
            }
            else {
                self.regs[instr.rt()]
            };
            if rs < rt {
                result = 1;
            }
        }
        if IMMEDIATE {
            self.write_reg(instr.rt(),result);
        } else {
            self.write_reg(instr.rd(),result);
        }

        Ok(())
    }

    /*
    arithmetic instructions
      addt rd,rs,rt    add   rd,rs,rt         rd=rs+rt (with overflow trap)
      add  rd,rs,rt    addu  rd,rs,rt         rd=rs+rt
      subt rd,rs,rt    sub   rd,rs,rt         rd=rs-rt (with overflow trap)
      sub  rd,rs,rt    subu  rd,rs,rt         rd=rs-rt
      addt rt,rs,imm   addi  rt,rs,imm        rt=rs+(-8000h..+7FFFh) (with ov.trap)
      add  rt,rs,imm   addiu rt,rs,imm        rt=rs+(-8000h..+7FFFh)
      The opcodes "with overflow trap" do trigger an exception (and leave rd unchanged) in case of overflows.
     */
    fn op_arithmetic<const ADD:bool,const WITH_OVERFLOW:bool,const IMMEDIATE:bool>(&mut self,_memory: &mut Bus, instr:&Instruction,_use_write_cache:bool) -> OperationException {
        let result = if WITH_OVERFLOW {
            let rs = self.regs[instr.rs()] as i32;
            let rt = if IMMEDIATE {
                instr.signed_immediate16() as i32
            }
            else {
                self.regs[instr.rt()] as i32
            };
            if ADD {
                if let Some(sum) = rs.checked_add(rt) {
                    sum as u32
                }
                else {
                    return Err(CpuException::ArithmeticOverflow)
                }
            }
            else {
                if let Some(diff) = rs.checked_sub(rt) {
                    diff as u32
                }
                else {
                    return Err(CpuException::ArithmeticOverflow)
                }
            }
        }
        else {
            let rs = self.regs[instr.rs()];
            let rt = if IMMEDIATE {
                instr.signed_immediate16()
            }
            else {
                self.regs[instr.rt()]
            };
            if ADD {
                rs.wrapping_add(rt)
            }
            else {
                rs.wrapping_sub(rt)
            }
        };

        if IMMEDIATE {
            self.write_reg(instr.rt(),result)
        }
        else {
            self.write_reg(instr.rd(),result)
        }

        Ok(())
    }

    fn op_mult<const SIGNED:bool>(&mut self,_memory: &mut Bus, instr:&Instruction,_use_write_cache:bool) -> OperationException {
        if SIGNED {
            let rs = (self.regs[instr.rs()] as i32) as i64;
            let rt = (self.regs[instr.rt()] as i32) as i64;
            let result = (rs * rt) as u64;
            self.lo = result as u32;
            self.hi = (result >> 32) as u32;

        }
        else {
            let rs = self.regs[instr.rs()] as u64;
            let rt = self.regs[instr.rt()] as u64;
            let result : u64 = rs * rt;
            self.lo = result as u32;
            self.hi = (result >> 32) as u32;
        }

        self.mul_div_pending_cycles = MUL_AVERAGE_CYCLES;
        Ok(())
    }

    fn op_div<const SIGNED:bool>(&mut self,_memory: &mut Bus, instr:&Instruction,_use_write_cache:bool) -> OperationException {
        if SIGNED {
            let rs = self.regs[instr.rs()] as i32;
            let rt = self.regs[instr.rt()] as i32;
            if rt == 0 {
                self.hi = rs as u32;
                if rs >= 0 {
                    self.lo = 0xFFFFFFFF;
                }
                else {
                    self.lo = 1;
                }
            }
            else if rs as u32 == 0x80000000 && rt == -1 {
                self.hi = 0;
                self.lo = 0x80000000;
            }
            else {
                self.lo = (rs / rt) as u32;
                self.hi = (rs % rt) as u32;
            }
        }
        else {
            let rs = self.regs[instr.rs()];
            let rt = self.regs[instr.rt()];
            if rt == 0 {
                self.hi = rs;
                self.lo = 0xFFFFFFFF;
            }
            else {
                self.lo = rs / rt;
                self.hi = rs % rt;
            }
        }

        self.mul_div_pending_cycles = DIV_CYCLES;
        Ok(())
    }

    // j      dest        pc=(pc and F0000000h)+(imm26bit*4)
    // jal    dest        pc=(pc and F0000000h)+(imm26bit*4),ra=$+8
    fn op_j<const JAL:bool>(&mut self,_memory:&mut Bus,instruction: &Instruction,_use_write_cache:bool) -> OperationException {
        let target = (self.pc & 0xF000_0000) | (instruction.imm26() << 2);
        self.branch_address = target;
        self.in_branch_delay_slot = true;
        self.branch_taken = true;
        if JAL {
            self.write_reg(31,self.pc.wrapping_add(8))
        }
        Ok(())
    }

    fn op_jr(&mut self,_memory: &mut Bus, instr:&Instruction,_use_write_cache:bool) -> OperationException {
        let target = self.regs[instr.rs()];
        self.branch_address = target;
        self.in_branch_delay_slot = true;
        self.branch_taken = true;
        Ok(())
    }

    fn op_jalr(&mut self,_memory: &mut Bus, instr:&Instruction,_use_write_cache:bool) -> OperationException {
        let target = self.regs[instr.rs()];
        self.branch_address = target;
        self.in_branch_delay_slot = true;
        self.branch_taken = true;
        self.write_reg(instr.rd(),self.pc.wrapping_add(8));
        Ok(())
    }

    /*
      je   rs,rt,dest  beq    rs,rt,dest  if rs=rt  then pc=$+4+(-8000h..+7FFFh)*4
      jne  rs,rt,dest  bne    rs,rt,dest  if rs<>rt then pc=$+4+(-8000h..+7FFFh)*4
      js   rs,dest     bltz   rs,dest     if rs<0   then pc=$+4+(-8000h..+7FFFh)*4
      jns  rs,dest     bgez   rs,dest     if rs>=0  then pc=$+4+(-8000h..+7FFFh)*4
      jgtz rs,dest     bgtz   rs,dest     if rs>0   then pc=$+4+(-8000h..+7FFFh)*4
      jlez rs,dest     blez   rs,dest     if rs<=0  then pc=$+4+(-8000h..+7FFFh)*4
      calls  rs,dest   bltzal rs,dest     ra=$+8, if rs<0  then pc=$+4+(..)*4
      callns rs,dest   bgezal rs,dest     ra=$+8, if rs>=0 then pc=$+4+(..)*4
     */
    #[inline(always)]
    fn op_branch<F,const CALL:bool>(&mut self,instr:&Instruction,cond:F)
    where
        F: Fn(u32, u32) -> bool,
    {
        let rs = self.regs[instr.rs()];
        let rt = self.regs[instr.rt()];
        let offset = instr.signed_immediate16() << 2;
        let base = self.pc.wrapping_add(4);
        let target = base.wrapping_add(offset);
        self.branch_address = target;
        if cond(rs,rt) {
            self.branch_taken = true;
        }
        else {
            self.branch_taken = false;
        }
        if CALL {
            self.write_reg(31,self.pc.wrapping_add(8))
        }
        self.in_branch_delay_slot = true;
    }

    fn op_bltz(&mut self,_memory:&mut Bus,instruction: &Instruction,_use_write_cache:bool) -> OperationException {
        self.op_branch::<_,false>(instruction,|a,_| (a as i32) < 0);
        Ok(())
    }
    fn op_bgez(&mut self,_memory:&mut Bus,instruction: &Instruction,_use_write_cache:bool) -> OperationException {
        self.op_branch::<_,false>(instruction,|a,_| (a as i32) >= 0);
        Ok(())
    }
    fn op_bltzal(&mut self,_memory:&mut Bus,instruction: &Instruction,_use_write_cache:bool) -> OperationException {
        self.op_branch::<_,true>(instruction,|a,_| (a as i32) < 0);
        Ok(())
    }
    fn op_bgezal(&mut self,_memory:&mut Bus,instruction: &Instruction,_use_write_cache:bool) -> OperationException {
        self.op_branch::<_,true>(instruction,|a,_| (a as i32) >= 0);
        Ok(())
    }
    fn op_beq(&mut self,_memory:&mut Bus,instruction: &Instruction,_use_write_cache:bool) -> OperationException {
        self.op_branch::<_,false>(instruction,|a,b| a == b);
        Ok(())
    }
    fn op_bne(&mut self,_memory:&mut Bus,instruction: &Instruction,_use_write_cache:bool) -> OperationException {
        self.op_branch::<_,false>(instruction,|a,b| a != b);
        Ok(())
    }
    fn op_blez(&mut self,_memory:&mut Bus,instruction: &Instruction,_use_write_cache:bool) -> OperationException {
        self.op_branch::<_,false>(instruction,|a,_| (a as i32) <= 0);
        Ok(())
    }
    fn op_bgtz(&mut self,_memory:&mut Bus,instruction: &Instruction,_use_write_cache:bool) -> OperationException {
        self.op_branch::<_,false>(instruction,|a,_| (a as i32) > 0);
        Ok(())
    }

    #[inline(always)]
    fn get_read_write_memory_address(&mut self,instr:&Instruction) -> u32 {
        let rs = self.regs[instr.rs()];
        rs.wrapping_add(instr.signed_immediate16())
    }
    #[inline(always)]
    fn read_data_memory<const N:usize>(&mut self,memory:&mut Bus,address:u32) -> Result<u32,CpuException> {
        const { assert!(N == 8 || N == 16 || N == 32) }
        // check HW breakpoint
        if memory.get_cop0_mut().is_at_rw_breakpoint(address,true) {
            return Err(CpuException::BreakPoint(HW_BREAKPOINT_VALUE))
        }
        self.last_mem_read_address = Some(address);
        match memory.read::<N>(address,false) {
            ReadMemoryAccess::Read(data,penalty_cycles) => {
                self.op_cycles += penalty_cycles;
                self.last_mem_rw_value = data;
                Ok(data)
            },
            ReadMemoryAccess::BusError => Err(CpuException::BusErrorData(address)),
            ReadMemoryAccess::MemoryError => Err(CpuException::AddressErrorLoad(address)),
            ReadMemoryAccess::Wait => Err(CpuException::ReadWriteWait)
        }
    }
    #[inline(always)]
    fn write_data_memory<const N:usize>(&mut self,memory:&mut Bus,address:u32,value:u32,use_write_cache:bool) -> OperationException {
        const { assert!(N == 8 || N == 16 || N == 32) }
        // check HW breakpoint
        if memory.get_cop0_mut().is_at_rw_breakpoint(address,false) {
            return Err(CpuException::BreakPoint(HW_BREAKPOINT_VALUE))
        }
        self.last_mem_write_address = Some(address);
        self.last_mem_rw_value = value;
        if use_write_cache {
            let byte_size = N >> 3;
            self.write_queue.enqueue(address, value,byte_size);
            Ok(())
        }
        else {
            match memory.write::<N>(address,value) {
                WriteMemoryAccess::Write(penalty_cycles) => {
                    self.op_cycles += penalty_cycles;
                    Ok(())
                },
                WriteMemoryAccess::BusError => Err(CpuException::BusErrorData(address)),
                WriteMemoryAccess::WriteErrorReadOnly(penalty_cycles) => {
                    self.op_cycles += penalty_cycles;
                    Err(CpuException::BusErrorData(address))
                },
                WriteMemoryAccess::MemoryError => Err(CpuException::AddressErrorStore(address)),
                WriteMemoryAccess::InvalidateICacheTag => {
                    self.cache_invalidate_tag(address);
                    Ok(())
                },
                WriteMemoryAccess::InvalidateICacheOpcode => {
                    self.cache_write_opcode(address, value);
                    Ok(())
                }
                WriteMemoryAccess::Wait => Err(CpuException::ReadWriteWait),
            }
        }
    }

    fn op_lb<const SIGNED:bool>(&mut self,memory:&mut Bus,instr: &Instruction,_use_write_cache:bool) -> OperationException {
        let target = self.get_read_write_memory_address(instr);
        let tmp = self.read_data_memory::<8>(memory,target)?;
        let read = if SIGNED {
            ((tmp as i8) as i32) as u32 // sign-extended
        }
        else {
            tmp
        };
        self.write_delayed_reg(instr.rt(),read);

        Ok(())
    }

    fn op_lh<const SIGNED:bool>(&mut self,memory:&mut Bus,instr: &Instruction,_use_write_cache:bool) -> OperationException {
        let target = self.get_read_write_memory_address(instr);
        // half-word alignment check
        if (target & 1) == 1 {
            return Err(CpuException::AddressErrorLoad(target))
        }
        let tmp = self.read_data_memory::<16>(memory,target)?;
        let read = if SIGNED {
            ((tmp as i16) as i32) as u32 // sign-extended
        }
        else {
            tmp
        };
        self.write_delayed_reg(instr.rt(),read);
        Ok(())
    }

    fn op_lw(&mut self,memory:&mut Bus,instr: &Instruction,_use_write_cache:bool) -> OperationException {
        let target = self.get_read_write_memory_address(instr);
        // word alignment check
        if (target & 3) != 0 {
            return Err(CpuException::AddressErrorLoad(target))
        }
        let read = self.read_data_memory::<32>(memory,target)?;
        self.write_delayed_reg(instr.rt(),read);
        Ok(())
    }

    fn op_lwl(&mut self,memory:&mut Bus,instr: &Instruction,_use_write_cache:bool) -> OperationException {
        let addr = self.get_read_write_memory_address(instr);

        // This instruction bypasses the load delay restriction: this
        // instruction will merge the new contents with the value
        // currently being loaded if need be.
        let (pending_reg, pending_value) = self.delayed_load;
        let cur_v = if pending_reg == instr.rt() {
            pending_value
        }
        else {
            self.regs[instr.rt()]
        };

        // Next we load the *aligned* word containing the first
        // addressed byte
        let aligned_addr = addr & !3;
        let aligned_word = self.read_data_memory::<32>(memory,aligned_addr)?;
        // Depending on the address alignment we fetch the 1, 2, 3 or
        // 4 *most* significant bytes and put them in the target
        // register.
        let v = match addr & 3 {
            0 => (cur_v & 0x00ffffff) | (aligned_word << 24),
            1 => (cur_v & 0x0000ffff) | (aligned_word << 16),
            2 => (cur_v & 0x000000ff) | (aligned_word << 8),
            3 => (cur_v & 0x00000000) | (aligned_word << 0),
            _ => unreachable!(),
        };
        self.write_delayed_reg(instr.rt(),v);

        Ok(())
    }

    fn op_lwr(&mut self,memory:&mut Bus,instr: &Instruction,_use_write_cache:bool) -> OperationException {
        let addr = self.get_read_write_memory_address(instr);

        // This instruction bypasses the load delay restriction: this
        // instruction will merge the new contents with the value
        // currently being loaded if need be.
        let (pending_reg, pending_value) = self.delayed_load;
        let cur_v = if pending_reg == instr.rt() {
            pending_value
        }
        else {
            self.regs[instr.rt()]
        };

        // Next we load the *aligned* word containing the first
        // addressed byte
        let aligned_addr = addr & !3;
        let aligned_word = self.read_data_memory::<32>(memory,aligned_addr)?;
        // Depending on the address alignment we fetch the 1, 2, 3 or
        // 4 *least* significant bytes and put them in the target
        // register.
        let v = match addr & 3 {
            0 => (cur_v & 0x00000000) | (aligned_word >> 0),
            1 => (cur_v & 0xff000000) | (aligned_word >> 8),
            2 => (cur_v & 0xffff0000) | (aligned_word >> 16),
            3 => (cur_v & 0xffffff00) | (aligned_word >> 24),
            _ => unreachable!(),
        };
        self.write_delayed_reg(instr.rt(),v);

        Ok(())
    }

    /*
     Doesn't check if the write queue is full. Must be checked by the caller.
     */
    fn op_sb(&mut self,memory:&mut Bus,instr:&Instruction, use_write_cache:bool) -> OperationException {
        let target = self.get_read_write_memory_address(instr);
        let rt_b = self.regs[instr.rt()] & 0xFF;
        self.write_data_memory::<8>(memory,target,rt_b,use_write_cache)
    }

    /*
     Doesn't check if the write queue is full. Must be checked by the caller.
     */
    fn op_sh(&mut self,memory:&mut Bus,instr:&Instruction, use_write_cache:bool) -> OperationException {
        let target = self.get_read_write_memory_address(instr);
        // half-word alignment check
        if (target & 1) == 1 {
            return Err(CpuException::AddressErrorStore(target))
        }
        let rt_h = self.regs[instr.rt()] & 0xFFFF;
        self.write_data_memory::<16>(memory,target,rt_h,use_write_cache)
    }

    /*
     Doesn't check if the write queue is full. Must be checked by the caller.
     */
    fn op_sw(&mut self,memory:&mut Bus,instr:&Instruction, use_write_cache:bool) -> OperationException {
        let target = self.get_read_write_memory_address(instr);
        if (target & 3) != 0 {
            return Err(CpuException::AddressErrorStore(target))
        }
        let rt_w = self.regs[instr.rt()];
        self.write_data_memory::<32>(memory,target,rt_w,use_write_cache)
    }

    fn op_swl(&mut self,memory:&mut Bus,instr:&Instruction, use_write_cache:bool) -> OperationException {
        let addr = self.get_read_write_memory_address(instr);
        let v = self.regs[instr.rt()];

        let aligned_addr = addr & !3;
        // Load the current value for the aligned word at the target
        // address
        let cur_mem = self.read_data_memory::<32>(memory,aligned_addr)?;
        let mem = match addr & 3 {
                0 => (cur_mem & 0xffffff00) | (v >> 24),
                1 => (cur_mem & 0xffff0000) | (v >> 16),
                2 => (cur_mem & 0xff000000) | (v >> 8),
                3 => (cur_mem & 0x00000000) | (v >> 0),
                _ => unreachable!(),
        };
        self.write_data_memory::<32>(memory,aligned_addr,mem,use_write_cache)
    }

    fn op_swr(&mut self,memory:&mut Bus,instr:&Instruction, use_write_cache:bool) -> OperationException {
        let addr = self.get_read_write_memory_address(instr);
        let v = self.regs[instr.rt()];

        let aligned_addr = addr & !3;
        // Load the current value for the aligned word at the target
        // address
        let cur_mem = self.read_data_memory::<32>(memory,aligned_addr)?;
        let mem = match addr & 3 {
                0 => (cur_mem & 0x00000000) | (v << 0),
                1 => (cur_mem & 0x000000ff) | (v << 8),
                2 => (cur_mem & 0x0000ffff) | (v << 16),
                3 => (cur_mem & 0x00ffffff) | (v << 24),
                _ => unreachable!(),
            };
        self.write_data_memory::<32>(memory,aligned_addr,mem,use_write_cache)
    }

    fn op_syscall(&mut self,_memory: &mut Bus, instr:&Instruction,_use_write_cache:bool) -> OperationException {
        Err(CpuException::SysCall(instr.imm20()))
    }

    fn op_break(&mut self,_memory: &mut Bus, instr:&Instruction,_use_write_cache:bool) -> OperationException {
        Err(CpuException::BreakPoint(instr.imm20()))
    }

    // Does not check if the LO/HI register is ready. Must be checked by the caller.
    // mfhi   rd
    // mflo   rd
    fn op_mf_lohi<const LO:bool>(&mut self,_memory: &mut Bus, instr:&Instruction,_use_write_cache:bool) -> OperationException {
        let target_reg = if LO {
            self.lo
        }
        else {
            self.hi
        };
        self.write_reg(instr.rd(),target_reg);

        Ok(())
    }

    // Does not check if the LO/HI register is ready. Must be checked by the caller.
    // mthi   rs
    // mtlo   rs
    fn op_mt_lohi<const LO:bool>(&mut self,_memory: &mut Bus, instr:&Instruction,_use_write_cache:bool) -> OperationException {
        if LO {
            self.lo = self.regs[instr.rs()];
        }
        else {
            self.hi = self.regs[instr.rs()]
        }

        Ok(())
    }

    fn op_mc_fc_n<const CONTROL_REG:bool>(&mut self,memory:&mut Bus,instr: &Instruction,_use_write_cache:bool) -> OperationException {
        let n = instr.op() & 0xF;
        match n {
            0 => {
                if !memory.get_cop0().is_accessible::<0>() {
                    return Err(CpuException::CoprocessorUnusable(n as usize))
                }
                if CONTROL_REG {
                    let CopResult(read,penalty) = memory.get_cop0().read_control_register(instr.rd());
                    self.write_delayed_reg(instr.rt(),read);
                    self.op_cycles += penalty;
                }
                else {
                    if instr.rd() < 3 {
                        return Err(CpuException::ReservedInstruction(instr.0))
                    }
                    let CopResult(read,penalty) = memory.get_cop0().read_data_register(instr.rd());
                    self.write_delayed_reg(instr.rt(),read);

                    self.op_cycles += penalty;
                }

            },
            2 => {
                if !memory.get_cop0().is_accessible::<2>() {
                    return Err(CpuException::CoprocessorUnusable(n as usize))
                }
                if CONTROL_REG {
                    let CopResult(read,penalty) = self.cop2.read_control_register(instr.rd());
                    self.write_delayed_reg(instr.rt(),read);
                    self.op_cycles += penalty;
                }
                else {
                    // If an instruction that reads a GTE register or a GTE command is executed before the current GTE command is finished,
                    // the CPU will hold until the instruction has finished.
                    let CopResult(read,penalty) = self.cop2.read_data_register(instr.rd());
                    self.write_delayed_reg(instr.rt(),read);
                    self.op_cycles += penalty + self.cop2_remaining_cycles;
                    self.cop2_remaining_cycles = 0;
                }
            }
            _ => {
                error!("Unsupported MFC operation on #{}", n);
                if !memory.get_cop0().is_cop_enabled(n as usize) {
                    return Err(CpuException::CoprocessorUnusable(n as usize))
                }
                else {
                    return Ok(())
                }
            }
        }
        Ok(())
    }

    fn op_mc_tc_n<const CONTROL_REG:bool>(&mut self,memory:&mut Bus,instr: &Instruction,_use_write_cache:bool) -> OperationException {
        let n = instr.op() & 0xF;
        match n {
            0 => {
                if !memory.get_cop0().is_accessible::<0>() {
                    return Err(CpuException::CoprocessorUnusable(n as usize))
                }
                if CONTROL_REG {
                    let CopResult(_,penalty) = memory.get_cop0_mut().write_control_register(instr.rd(), self.regs[instr.rt()]);
                    self.op_cycles += penalty;
                }
                else {
                    let CopResult(_,penalty) = memory.get_cop0_mut().write_data_register(instr.rd(), self.regs[instr.rt()]);
                    self.op_cycles += penalty;
                }
                Ok(())
            },
            2 => {
                if !memory.get_cop0().is_accessible::<2>() {
                    return Err(CpuException::CoprocessorUnusable(n as usize))
                }
                if CONTROL_REG {
                    let CopResult(_,penalty) = self.cop2.write_control_register(instr.rd(), self.regs[instr.rt()]);
                    self.op_cycles += penalty;
                }
                else {
                    let CopResult(_,penalty) = self.cop2.write_data_register(instr.rd(), self.regs[instr.rt()]);
                    self.op_cycles += penalty;
                }
                Ok(())
            }
            _ => {
                error!("Unsupported MTC operation on #{}", n);
                Err(CpuException::CoprocessorUnusable(n as usize))
            }
        }
    }

    fn op_copn(&mut self,memory:&mut Bus,instr: &Instruction,_use_write_cache:bool) -> OperationException {
        let n = instr.op() & 0xF;
        match n {
            0 => {
                if !memory.get_cop0().is_accessible::<0>() {
                    return Err(CpuException::CoprocessorUnusable(n as usize))
                }
                memory.get_cop0_mut().execute_command(instr.0);
                Ok(())
            },
            2 => {
                if !memory.get_cop0().is_accessible::<2>() {
                    return Err(CpuException::CoprocessorUnusable(n as usize))
                }
                // If an instruction that reads a GTE register or a GTE command is executed before the current GTE command is finished,
                // the CPU will hold until the instruction has finished.
                let gte_rem_cycles = self.cop2_remaining_cycles;
                let CopResult(_,penalty) = self.cop2.execute_command(instr.0);
                self.cop2_remaining_cycles = penalty;
                self.op_cycles += gte_rem_cycles; // here, we eventually consume the remaining gte cycles because the CPU will hold waiting the completion
                Ok(())
            },
            _ => {
                error!("Unsupported COP operation on #{}", n);
                Err(CpuException::CoprocessorUnusable(n as usize))
            }
        }
    }

    fn op_bcn_ft<const FALSE:bool>(&mut self,_memory:&mut Bus,instr: &Instruction,_use_write_cache:bool) -> OperationException {
        todo!{"Implement BCnF/BCnT instructions"}
    }

    fn op_lwc<const N:usize>(&mut self,memory:&mut Bus,instr: &Instruction,_use_write_cache:bool) -> OperationException {
        let target = self.get_read_write_memory_address(instr);
        // word alignment check
        if (target & 3) != 0 {
            return Err(CpuException::AddressErrorLoad(target))
        }
        let read = self.read_data_memory::<32>(memory,target)?;

        if N != 2 {
            error!("Unsupported LWC{} operation",N);
            if !memory.get_cop0().is_cop_enabled(N) {
                return Err(CpuException::CoprocessorUnusable(N))
            }
            return Ok(())
        }
        if !memory.get_cop0().is_cop_enabled(N) {
            return Err(CpuException::CoprocessorUnusable(N))
        }

        let CopResult(_,penalty) = self.cop2.write_data_register(instr.rt(), read);
        self.op_cycles += penalty;
        Ok(())
    }

    fn op_swc<const N:usize>(&mut self,memory:&mut Bus,instr:&Instruction,use_write_cache:bool) -> OperationException {
        let target = self.get_read_write_memory_address(instr);
        // word alignment check
        if (target & 3) != 0 {
            return Err(CpuException::AddressErrorStore(target))
        }

        if N != 2 {
            error!("Unsupported SWC{} operation",N);
            if !memory.get_cop0().is_cop_enabled(N) {
                return Err(CpuException::CoprocessorUnusable(N))
            }
            return Ok(())
        }
        if !memory.get_cop0().is_cop_enabled(N) {
            return Err(CpuException::CoprocessorUnusable(N))
        }

        let CopResult(data,penalty) = self.cop2.read_data_register(instr.rt());
        self.op_cycles += penalty;
        self.write_data_memory::<32>(memory,target,data,use_write_cache)

    }

    // ===============================================================
    fn handle_exception(&mut self,memory:&mut Bus,cpu_exception: CpuException,is_branch_delay_slot:bool,branch_target_address:u32,opcode:u32) {
        self.apply_delayed_load();
        self.in_branch_delay_slot = false;
        debug!("Handling CPU exception {:?} at PC={:04X} [branch_delay={}]",cpu_exception,self.pc,is_branch_delay_slot);
        self.pc = memory.get_cop0_mut().do_exception(self.pc,cpu_exception,is_branch_delay_slot,self.branch_taken,branch_target_address,opcode);
    }
}