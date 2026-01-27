use tracing::{debug, info, warn};
use crate::core::cpu::CpuException;
use crate::core::cpu::{Coprocessor, CopResult};

pub static COP0_REGISTER_ALIASES: [ &str; 32 ] = [
    "$cop0_r0", "$cop0_r1", "$cop0_r2", "$cop0_bpc", "$cop0_r4", "$cop0_bda", "$cop0_jumpdest", "$cop0_dcic",
    "$cop0_badvaddr", "$cop0_bdam", "$cop0_r10", "$cop0_bpcm", "$cop0_sr", "$cop0_cause", "$cop0_epc", "$cop0_prid",
    "$cop0_r16", "$cop0_r17", "$cop0_r18", "$cop0_r19", "$cop0_r20", "$cop0_r21", "$cop0_r22", "$cop0_r23",
    "$cop0_r24", "$cop0_r25", "$cop0_r26", "$cop0_r27", "$cop0_r28", "$cop0_r29", "$cop0_r30", "$cop0_r31",
];

const PRID_VALUE : u32 = 1;

/*
COP0 Register Summary
  cop0r0-r2   - N/A
  cop0r3      - BPC - Breakpoint on execute (R/W)
  cop0r4      - N/A
  cop0r5      - BDA - Breakpoint on data access (R/W)
  cop0r6      - JUMPDEST - Randomly memorized jump address (R)
  cop0r7      - DCIC - Breakpoint control (R/W)
  cop0r8      - BadVaddr - Bad Virtual Address (R)
  cop0r9      - BDAM - Data Access breakpoint mask (R/W)
  cop0r10     - N/A
  cop0r11     - BPCM - Execute breakpoint mask (R/W)
  cop0r12     - SR - System status register (R/W)
  cop0r13     - CAUSE - (R)  Describes the most recently recognised exception
  cop0r14     - EPC - Return Address from Trap (R)
  cop0r15     - PRID - Processor ID (R)
  cop0r16-r31 - Garbage
  cop0r32-r63 - N/A - None such (Control regs)

cop0r13 - CAUSE - (Read-only, except, Bit8-9 are R/W)
Describes the most recently recognised exception
  0-1   -      Not used (zero)
  2-6   Excode Describes what kind of exception occured:
                 00h INT     Interrupt
                 01h MOD     Tlb modification (none such in PSX)
                 02h TLBL    Tlb load         (none such in PSX)
                 03h TLBS    Tlb store        (none such in PSX)
                 04h AdEL    Address error, Data load or Instruction fetch
                 05h AdES    Address error, Data store
                             The address errors occur when attempting to read
                             outside of KUseg in user mode and when the address
                             is misaligned. (See also: BadVaddr register)
                 06h IBE     Bus error on Instruction fetch
                 07h DBE     Bus error on Data load/store
                 08h Syscall Generated unconditionally by syscall instruction
                 09h BP      Breakpoint - break instruction
                 0Ah RI      Reserved instruction
                 0Bh CpU     Coprocessor unusable
                 0Ch Ov      Arithmetic overflow
                 0Dh-1Fh     Not used
  7     -      Not used (zero)
  8-15  Ip     Interrupt pending field. Bit 8 and 9 are R/W, and
               contain the last value written to them. As long
               as any of the bits are set they will cause an
               interrupt if the corresponding bit is set in IM.
  16-27 -      Not used (zero)
  28-29 CE     Opcode Bit26-27 (aka coprocessor number in case of COP opcodes)
  30    -      Not used (zero) / Undoc: When BD=1, Branch condition (0=False)
  31    BD     Branch Delay (set when last exception points to the branch
               instruction instead of the instruction in the branch delay
               slot, where the exception occurred)

cop0r12 - SR - System status register (R/W)
  0     IEc Current Interrupt Enable  (0=Disable, 1=Enable) ;rfe pops IUp here
  1     KUc Current Kernal/User Mode  (0=Kernel, 1=User)    ;rfe pops KUp here
  2     IEp Previous Interrupt Disable                      ;rfe pops IUo here
  3     KUp Previous Kernal/User Mode                       ;rfe pops KUo here
  4     IEo Old Interrupt Disable                       ;left unchanged by rfe
  5     KUo Old Kernal/User Mode                        ;left unchanged by rfe
  6-7   -   Not used (zero)
  8-15  Im  8 bit interrupt mask fields. When set the corresponding
            interrupts are allowed to cause an exception.
  16    Isc Isolate Cache (0=No, 1=Isolate)
              When isolated, all load and store operations are targetted
              to the Data cache, and never the main memory.
              (Used by PSX Kernel, in combination with Port FFFE0130h)
  17    Swc Swapped cache mode (0=Normal, 1=Swapped)
              Instruction cache will act as Data cache and vice versa.
              Use only with Isc to access & invalidate Instr. cache entries.
              (Not used by PSX Kernel)
  18    PZ  When set cache parity bits are written as 0.
  19    CM  Shows the result of the last load operation with the D-cache
            isolated. It gets set if the cache really contained data
            for the addressed memory location.
  20    PE  Cache parity error (Does not cause exception)
  21    TS  TLB shutdown. Gets set if a programm address simultaneously
            matches 2 TLB entries.
            (initial value on reset allows to detect extended CPU version?)
  22    BEV Boot exception vectors in RAM/ROM (0=RAM/KSEG0, 1=ROM/KSEG1)
  23-24 -   Not used (zero)
  25    RE  Reverse endianness   (0=Normal endianness, 1=Reverse endianness)
              Reverses the byte order in which data is stored in
              memory. (lo-hi -> hi-lo)
              (Has affect only to User mode, not to Kernal mode) (?)
              (The bit doesn't exist in PSX ?)
  26-27 -   Not used (zero)
  28    CU0 COP0 Enable (0=Enable only in Kernal Mode, 1=Kernal and User Mode)
  29    CU1 COP1 Enable (0=Disable, 1=Enable) (none such in PSX)
  30    CU2 COP2 Enable (0=Disable, 1=Enable) (GTE in PSX)
  31    CU3 COP3 Enable (0=Disable, 1=Enable) (none such in PSX)
When writing to SR: Changing SR.bit0 from 0-to-1 won't trigger any IRQ until after executing the next opcode.
On the other hand, changing SR.bit8-15 from 0-to-1 can immediately trigger IRQs (if SR.bit0 was already set).
Another special case is the RFE opcode, which will also immediately trigger IRQs when changing SR.bit0 from 0-to-1.
 */
pub enum Cop0Reg {
    BPC = 3,
    BDA = 5,
    DCIC = 7,
    BadVAddr = 8,
    BDAM = 9,
    BPCM = 11,
    SR = 12,
    CAUSE = 13,
    EPC = 14,
}

pub struct Cop0 {
    regs: [u32;32],
    pending_int_on_next_opcode: bool,
}

impl Cop0 {
    pub fn new() -> Self {
        let mut cop0 = Cop0 {
            regs: [0;32],
            pending_int_on_next_opcode: false,
        };

        cop0.reset();

        cop0
    }
    
    pub fn get_regs(&self) -> &[u32;32] {
        &self.regs
    }

    pub fn reset(&mut self) {
        // At reset, the SWc, KUc, and IEc bits are set to zero; BEV is set to one; and the value of
        // the TS bit is set to 0 (TS = 0) The rest of the bit fields are undefined after reset.
        self.regs[Cop0Reg::SR as usize] = 1 << 22; // BEV = 1
        self.regs[15] = PRID_VALUE;
        self.pending_int_on_next_opcode = false;
        // TODO : set other registers to default values
    }

    pub fn is_in_kernel_mode(&self) -> bool {
        (self.regs[Cop0Reg::SR as usize] & 2) == 0 // KUc = 0
    }

    pub fn is_cache_isolated(&self) -> bool {
        (self.regs[Cop0Reg::SR as usize] & (1 << 16)) != 0
    }

    /*
    DCIC Register.
    0	        DB	            Debug	Automatically set upon Any break	R/W
    1	        PC	            Program Counter	Automatically set upon BPC Program Counter break	R/W
    2	        DA	            Data Address	Automatically set upon BDA Data Address break	R/W
    3	        R	            Read Reference	Automatically set upon BDA Data Read break	R/W
    4	        W	            Write Reference	Automatically set upon BDA Data Write break	R/W
    5	        T	            Trace	Automatically set upon Trace break	R/W
    6-11		Not used	    Always zero	R
    12-13		Jump Redirection	0=Disable, 1..3=Enable (see note)	R/W
    14-15		Unknown?		R/W
    16-22		Not used	    Always zero	R
    23	        DE	            Debug Enable	0=Disabled, 1=Enable bits 24-31	R/W
    24	        PCE	            Program Counter Breakpoint Enable	0=Disabled, 1=Enabled (see BPC, BPCM)	R/W
    25	        DAE	            Data Address Breakpoint Enable	0=Disabled, 1=Enabled (see BDA, BDAM)	R/W
    26	        DR	            Data Read Enable	0=No, 1=Break/when Bit25=1	R/W
    27	        DW	            Data Write Enable	0=No, 1=Break/when Bit25=1	R/W
    28	        TE	            Trace Enable	0=No, 1=Break on branch/jump/call/etc.	R/W
    29	        KD	            Kernel Debug Enable	0=Disabled, 1=Break in kernel mode	R/W
    30	        UD	            User Debug Enable	0=Disabled, 1=Break in user mode	R/W
    31	        TR	            Trap Enable	0=Only set status bits, 1=Jump to debug vector
     */
    pub fn is_at_pc_breakpoint(&self,pc:u32) -> bool {
        let dcic = self.regs[Cop0Reg::DCIC as usize];
        let bpc = self.regs[Cop0Reg::BPC as usize];
        let bpcm = self.regs[Cop0Reg::BPCM as usize];

        // bit 23 & 24 on and ((PC XOR BPC) AND BPCM)=0
        (dcic & (3 << 23)) != 0 && ((pc ^ bpc) & bpcm) == 0
    }

    pub fn is_at_rw_breakpoint(&self,address:u32,is_read:bool) -> bool {
        let dcic = self.regs[Cop0Reg::DCIC as usize];
        let dba = self.regs[Cop0Reg::BDA as usize];
        let bdam = self.regs[Cop0Reg::BDAM as usize];
        let rw_cond = (is_read && (dcic & (1 << 26)) != 0) || (!is_read && (dcic & (1 << 27)) != 0);

        // bit 23 & 24 on and ((addr XOR BDA) AND BDAM)=0
        (dcic & (3 << 23)) != 0 && ((address ^ dba) & bdam) == 0 && rw_cond
    }

    pub fn do_exception(&mut self,pc:u32,exception:CpuException,is_branch_delay_slot:bool,branch_taken:bool,branch_target_address:u32,opcode:u32) -> u32 {
        debug!("Do exception {:?} at PC={:04X} [branch_delay={}]",exception,pc,is_branch_delay_slot);

        // "push" 00 (kernel mode + interrupt disabled) to bits 0/1 of ST
        let stack = self.regs[Cop0Reg::SR as usize] & 0x3F; // bit 5-0
        self.regs[Cop0Reg::SR as usize] &= !0x3F; // clear old stack bits
        self.regs[Cop0Reg::SR as usize] |= (stack << 2) & 0x3F; // push 00 (shift left by 2)

        let pc_to_save = if is_branch_delay_slot {
            pc - 4
        }
        else {
            pc
        };

        // update EPC
        self.regs[Cop0Reg::EPC as usize] = pc_to_save;

        // update CAUSE
        self.regs[Cop0Reg::CAUSE as usize] &= 0xFF00;

        // ExcCode (bit 2-6)
        self.regs[Cop0Reg::CAUSE as usize] |= (exception.id() as u32) << 2;

        // BD (bit 31) + bit 30
        if is_branch_delay_slot {
            self.regs[Cop0Reg::CAUSE as usize] |= (branch_taken as u32) << 30;
            self.regs[Cop0Reg::CAUSE as usize] |= 1 << 31;
            self.regs[6] = if branch_taken {
                branch_target_address
            }
            else {
                pc + 4
            };
        }
        // 28-29 CE     Opcode Bit26-27 (aka coprocessor number in case of COP opcodes)
        let cop_num = (opcode >> 26) & 3;
        self.regs[Cop0Reg::CAUSE as usize] |= cop_num << 28;

        let mut handler_address = // check BEV
            if (self.regs[Cop0Reg::SR as usize] & (1 << 22)) == 0 {
                0x80000080
            }
            else {
                0xBFC00180
            };

        // check if the exception is an address error: must be stored in BadVAddr
        // check if the exception is a Breakpoint(0xFFFFFFFF): in this case the return address is 0x80000040
        match exception {
            CpuException::AddressErrorLoad(address) | CpuException::AddressErrorStore(address) => {
                self.regs[Cop0Reg::BadVAddr as usize] = address;
            },
            CpuException::BreakPoint(0xFFFFFFFF) => {
                handler_address = 0x80000040
            },
            _ => {}
        }

        handler_address
    }

    /*
        COP0 has six hardware interrupt bits, of which, the PSX uses only cop0r13.bit10 (the other ones, cop0r13.bit11-15 are always zero). 
        cop0r13.bit10 is NOT a latch, ie. it gets automatically cleared as soon as "(I_STAT AND I_MASK)=zero", so there's no need to do an acknowledgment at the cop0 side. 
        COP0 additionally has two software interrupt bits, cop0r13.bit8-9, which do exist in the PSX, too, 
        these bits are read/write-able latches which can be set/cleared manually to request/acknowledge exceptions by software.
     */
    pub fn set_hw_interrupt(&mut self) {
        self.regs[Cop0Reg::CAUSE as usize] |= 1 << 10
    }

    pub fn clear_hw_interrupt(&mut self) {
        self.regs[Cop0Reg::CAUSE as usize] &= !(1 << 10)
    }

    /*
      Must be called by CPU only.
      Takes care about the flag pending_int_on_next_opcode.
     */
    pub fn check_interrupt_pending(&mut self) -> bool {
        let sr = self.regs[Cop0Reg::SR as usize];
        let im = (sr >> 8) as u8;
        let ip = (self.regs[Cop0Reg::CAUSE as usize] >> 8) as u8;
        // global interrupts enabled + pending & mask != 0
        let pending_int = !self.pending_int_on_next_opcode && (sr & 1) == 1 && (im & ip) != 0;
        self.pending_int_on_next_opcode = false;
        pending_int
    }

    fn cmd_rfe(&mut self) {
        // pop the stack
        let stack = self.regs[Cop0Reg::SR as usize] & 0x3F; // bit 5-0
        self.regs[Cop0Reg::SR as usize] &= !0xF; // clear old stack bits
        self.regs[Cop0Reg::SR as usize] |= (stack >> 2) & 0xF;
    }

    pub fn is_accessible<const COP_INDEX: usize>(&self) -> bool {
        const { assert!(COP_INDEX == 0 || COP_INDEX == 1 || COP_INDEX == 2 || COP_INDEX == 3)}
        let sr = self.regs[Cop0Reg::SR as usize];
        if COP_INDEX == 0 {
            (sr & 2) == 0 || (sr & (1 << (28 + COP_INDEX)) != 0) // accessible if in kernel mode or CU0 = 1
        }
        else {
            sr & (1 << (28 + COP_INDEX)) != 0
        }
    }

    pub fn is_cop_enabled(&self,cop_index: usize) -> bool {
        let sr = self.regs[Cop0Reg::SR as usize];
        sr & (1 << (28 + cop_index)) != 0
    }
}

impl Coprocessor for Cop0 {
    fn get_name(&self) -> &'static str {
        "COP0"
    }

    fn read_data_register(&self, reg_index: usize) -> CopResult {
        CopResult(self.regs[reg_index],0)
    }

    fn write_data_register(&mut self, reg_index: usize, value: u32) -> CopResult {
        // check for read-only registers
        match reg_index {
            6 | 8 | 14 | 15 => {
                warn!("Writing to a read-only Cop0 register #{reg_index} = {value}")
                /*do nothing*/
            },
            12 => {
                debug!("Cop0 writing to SR: {:08X}",value);
                let sr = self.regs[Cop0Reg::SR as usize];
                // Changing SR.bit0 from 0-to-1 won't trigger any IRQ until after executing the next opcode
                if (sr & 1) == 0 && (value & 1) == 1 {
                    self.pending_int_on_next_opcode = true;
                }
                self.regs[Cop0Reg::SR as usize] = value & !0xD8000C0; // bits 6-7, 23-24, 26-27 must be 0
            },
            13 => { // Read-only, except, Bit8-9 are R/W
                debug!("Cop0 writing to CAUSE: {:08X}",value);
                self.regs[reg_index] = (self.regs[reg_index] & 0xFFFFFCFF) | (value & 0x300);
            },
            _ => {
                debug!("Cop0 writing to #{}: {:08X}",reg_index,value);
                self.regs[reg_index] = value;
            }
        }
        CopResult(0,0)
    }

    fn read_control_register(&self, reg_index: usize) -> CopResult {
        warn!("CP0 read_control_register {} called",reg_index);
        CopResult(0,0)
    }

    fn write_control_register(&mut self, reg_index: usize, value: u32) -> CopResult {
        warn!("CP0 write_control_register {} called, value={}",reg_index,value);
        CopResult(0,0)
    }

    fn execute_command(&mut self,cmd: u32) -> CopResult {
        match cmd & 0x3F {
            0x10 => self.cmd_rfe(),
            _ => {
                warn!("Unknown command {:04X}",cmd);
            }
        }
        CopResult(0,0)
    }
}