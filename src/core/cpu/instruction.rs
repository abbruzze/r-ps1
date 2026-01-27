pub struct Instruction(pub u32);

static OPCODE_CACHE: [fn(&Instruction) -> Opcode;64] = init_opcode_cache();
static OPCODE_CACHE_SPECIAL: [fn(&Instruction) -> Opcode;64] = init_opcode_special_cache();

#[derive(Debug,PartialEq)]
pub enum Opcode {
    // SPECIAL
    NOP,
    SLL,
    SRL,
    SRA,
    SLLV,
    SRLV,
    SRAV,
    JR,
    JALR,
    SYSCALL,
    BREAK,
    MFHI,
    MTHI,
    MFLO,
    MTLO,
    MULT,
    MULTU,
    DIV,
    DIVU,
    ADD,
    ADDU,
    SUB,
    SUBU,
    AND,
    OR,
    XOR,
    NOR,
    SLT,
    SLTU,
    // BCONDZ
    BLTZ,
    BGEZ,
    BLTZAL,
    BGEZAL,
    // NORMAL
    J,
    JAL,
    BEQ,
    BNE,
    BLEZ,
    BGTZ,
    ADDI,
    ADDIU,
    SLTI,
    SLTIU,
    ANDI,
    ORI,
    XORI,
    LUI,
    // COP
    MFCn,
    CFCn,
    MTCn,
    CTCn,
    BCnF,
    BCnT,
    TLBR,
    TLBWI,
    TLBWR,
    TLBP,
    RFE,
    COPn,
    LB,
    LH,
    LWL,
    LW,
    LBU,
    LHU,
    LWR,
    SB,
    SH,
    SWL,
    SW,
    SWR,
    LWC0,
    LWC1,
    LWC2,
    LWC3,
    SWC0,
    SWC1,
    SWC2,
    SWC3,
    UNKNOWN,
}

impl Instruction {
    #[inline(always)]
    pub fn op(&self) -> u32 {
        self.0 >> 26
    }
    #[inline(always)]
    pub fn rs(&self) -> usize {
        ((self.0 >> 21) & 0x1F) as usize
    }
    #[inline(always)]
    pub fn rt(&self) -> usize {
        ((self.0 >> 16) & 0x1F) as usize
    }
    #[inline(always)]
    pub fn rd(&self) -> usize {
        ((self.0 >> 11) & 0x1F) as usize
    }
    #[inline(always)]
    pub fn shift_amount(&self) -> u32 {
        (self.0 >> 6) & 0x1F
    }
    #[inline(always)]
    pub fn function(&self) -> u32 {
        self.0 & 0x3F
    }
    #[inline(always)]
    pub fn signed_immediate16(&self) -> u32 {
        let offset = (self.0 & 0xFFFF) as i16;
        offset as u32
    }
    #[inline(always)]
    pub fn unsigned_immediate16(&self) -> u32 {
        self.0 & 0xFFFF
    }
    #[inline(always)]
    pub fn imm20(&self) -> u32 {
        (self.0 >> 6) & 0xF_FFFF
    }
    #[inline(always)]
    pub fn imm26(&self) -> u32 {
        self.0 & 0x3FF_FFFF
    }
    #[inline(always)]
    pub fn imm25(&self) -> u32 {
        self.0 & 0x1FF_FFFF
    }
}

const fn init_opcode_special_cache() -> [fn(&Instruction) -> Opcode; 64] {
    let mut cache: [fn(&Instruction) -> Opcode; 64] = [|_| Opcode::UNKNOWN; 64];
    let mut fun: usize = 0;
    while fun < 64 {
        cache[fun] = match fun {
            0x00 => |i| {
                if i.rd() == 0 && i.rt() == 0 && i.shift_amount() == 0 {
                    Opcode::NOP
                } else {
                    Opcode::SLL
                }
            },
            0x02 => |_| Opcode::SRL,
            0x03 => |_| Opcode::SRA,
            0x04 => |_| Opcode::SLLV,
            0x06 => |_| Opcode::SRLV,
            0x07 => |_| Opcode::SRAV,
            0x08 => |_| Opcode::JR,
            0x09 => |_| Opcode::JALR,
            0x0C => |_| Opcode::SYSCALL,
            0x0D => |_| Opcode::BREAK,
            0x10 => |_| Opcode::MFHI,
            0x11 => |_| Opcode::MTHI,
            0x12 => |_| Opcode::MFLO,
            0x13 => |_| Opcode::MTLO,
            0x18 => |_| Opcode::MULT,
            0x19 => |_| Opcode::MULTU,
            0x1A => |_| Opcode::DIV,
            0x1B => |_| Opcode::DIVU,
            0x20 => |_| Opcode::ADD,
            0x21 => |_| Opcode::ADDU,
            0x22 => |_| Opcode::SUB,
            0x23 => |_| Opcode::SUBU,
            0x24 => |_| Opcode::AND,
            0x25 => |_| Opcode::OR,
            0x26 => |_| Opcode::XOR,
            0x27 => |_| Opcode::NOR,
            0x2A => |_| Opcode::SLT,
            0x2B => |_| Opcode::SLTU,
            // N/A
            _ => |_| Opcode::UNKNOWN,
        };

        fun += 1;
    }

    cache
}

const fn init_opcode_cache() -> [fn(&Instruction) -> Opcode; 64] {
    let mut cache : [fn(&Instruction) -> Opcode; 64] = [|_| Opcode::UNKNOWN;64];
    let mut op : usize = 0;
    while op < 64 {
        cache[op] = match op {
            0x01 => |i| {
                match i.rt() {
                    0x00 => Opcode::BLTZ,
                    0x01 => Opcode::BGEZ,
                    0x10 => Opcode::BLTZAL,
                    0x11 => Opcode::BGEZAL,
                    rt => {
                        if (rt & 1) == 0 {
                            Opcode::BLTZ
                        }
                        else if ((rt >> 1) & 7) != 0 {
                            Opcode::BGEZ
                        }
                        else {
                            Opcode::UNKNOWN                        }
                    }
                }
            },
            0x02 => |_| Opcode::J,
            0x03 => |_| Opcode::JAL,
            0x04 => |_| Opcode::BEQ,
            0x05 => |_| Opcode::BNE,
            0x06 => |_| Opcode::BLEZ,
            0x07 => |_| Opcode::BGTZ,
            0x08 => |_| Opcode::ADDI,
            0x09 => |_| Opcode::ADDIU,
            0x0A => |_| Opcode::SLTI,
            0x0B => |_| Opcode::SLTIU,
            0x0C => |_| Opcode::ANDI,
            0x0D => |_| Opcode::ORI,
            0x0E => |_| Opcode::XORI,
            0x0F => |_| Opcode::LUI,
            0x10..=0x13 => |i| { // COP
                match i.rs() {
                    0x00 => Opcode::MFCn,
                    0x02 => Opcode::CFCn,
                    0x04 => Opcode::MTCn,
                    0x06 => Opcode::CTCn,
                    0x08 => {
                        match i.rt() {
                            0x00 => Opcode::BCnF,
                            0x01 => Opcode::BCnT,
                            _ => Opcode::UNKNOWN
                        }
                    },
                    _ => {
                        let cop_n = (i.op() & 0xF) as usize;
                        if cop_n == 0 {
                            if i.rs() == 0x10 {
                                match i.function() {
                                    0x01 => Opcode::TLBR,
                                    0x02 => Opcode::TLBWI,
                                    0x06 => Opcode::TLBWR,
                                    0x08 => Opcode::TLBP,
                                    0x10 => Opcode::RFE,
                                    _ => Opcode::COPn // to handle unknown COP command
                                }
                            }
                            else {
                                Opcode::COPn // to handle unknown COP command
                            }
                        }
                        else {
                            Opcode::COPn
                        }
                    }
                }
            },
            0x20 => |_| Opcode::LB,
            0x21 => |_| Opcode::LH,
            0x22 => |_| Opcode::LWL,
            0x23 => |_| Opcode::LW,
            0x24 => |_| Opcode::LBU,
            0x25 => |_| Opcode::LHU,
            0x26 => |_| Opcode::LWR,
            0x28 => |_| Opcode::SB,
            0x29 => |_| Opcode::SH,
            0x2A => |_| Opcode::SWL,
            0x2B => |_| Opcode::SW,
            0x2E => |_| Opcode::SWR,
            0x30 => |_| Opcode::LWC0,
            0x31 => |_| Opcode::LWC1,
            0x32 => |_| Opcode::LWC2,
            0x33 => |_| Opcode::LWC3,
            0x38 => |_| Opcode::SWC0,
            0x39 => |_| Opcode::SWC1,
            0x3A => |_| Opcode::SWC2,
            0x3B => |_| Opcode::SWC3,
            _ => |_| Opcode::UNKNOWN
        };

        op += 1;
    }
    cache
}

impl Opcode {
    pub fn is_read_memory(&self) -> bool {
        use Opcode::*;
        match self {
           LB|LH|LWL|LW|LBU|LHU|LWR => true,
           _ => false
        }
    }

    pub fn is_write_memory(&self) -> bool {
        use Opcode::*;
        match self {
            SB|SH|SWL|SW|SWR => true,
            _ => false
        }
    }

    pub fn is_accessing_hi_low(&self) -> bool {
        use Opcode::*;
        match self {
            MFHI|MFLO|MTHI|MTLO => true,
            _ => false
        }
    }

    #[inline(always)]
    pub fn from_instruction(instruction: &Instruction) -> Opcode {
        let op = instruction.op() as usize;
        if op == 0x00 {
            OPCODE_CACHE_SPECIAL[instruction.function() as usize](instruction)
        }
        else {
            OPCODE_CACHE[op](instruction)
        }
    }

    /*
    pub fn from_instruction(instruction: u32) -> Option<Self> {
        let instr = Instruction(instruction);
        let op = instr.op();
        let function = instr.function();
        let rs = instr.rs() as usize;
        let rd = instr.rd() as usize;
        let rt = instr.rt() as usize;
        let sa = instr.shift_amount();

        let opcode = match op {
            0x00 => { // special
                match function {
                    0x00 => {
                        if rd == 0 && rt == 0 && sa == 0 {
                            Opcode::NOP
                        } else {
                            Opcode::SLL
                        }
                    },
                    0x02 => Opcode::SRL,
                    0x03 => Opcode::SRA,
                    0x04 => Opcode::SLLV,
                    0x06 => Opcode::SRLV,
                    0x07 => Opcode::SRAV,
                    0x08 => Opcode::JR,
                    0x09 => Opcode::JALR,
                    0x0C => Opcode::SYSCALL,
                    0x0D => Opcode::BREAK,
                    0x10 => {
                        if rs == 0 && rt == 0 && sa == 0 {
                            Opcode::MFHI
                        }
                        else {
                            return None
                        }
                    },
                    0x11 => {
                        if rt == 0 && rd == 0 && sa == 0 {
                            Opcode::MTHI
                        }
                        else {
                            return None
                        }
                    },
                    0x12 => {
                        if rs == 0 && rt == 0 && sa == 0 {
                            Opcode::MFLO
                        }
                        else {
                            return None
                        }
                    },
                    0x13 => {
                        if rt == 0 && rd == 0 && sa == 0 {
                            Opcode::MTLO
                        }
                        else {
                            return None
                        }
                    },
                    0x18 => Opcode::MULT,
                    0x19 => Opcode::MULTU,
                    0x1A => {
                        if rd == 0 && sa == 0 {
                            Opcode::DIV
                        }
                        else {
                            return None
                        }
                    },
                    0x1B => {
                        if rd == 0 && sa == 0 {
                            Opcode::DIVU
                        }
                        else {
                            return None
                        }
                    },
                    0x20 => {
                        if sa == 0 {
                            Opcode::ADD
                        }
                        else {
                            return None
                        }
                    },
                    0x21 => {
                        if sa == 0 {
                            Opcode::ADDU
                        }
                        else {
                            return None
                        }
                    },
                    0x22 => {
                        if sa == 0 {
                            Opcode::SUB
                        }
                        else {
                            return None
                        }
                    },
                    0x23 => {
                        if sa == 0 {
                            Opcode::SUBU
                        }
                        else {
                            return None
                        }
                    },
                    0x24 => {
                        if sa == 0 {
                            Opcode::AND
                        }
                        else {
                            return None
                        }
                    },
                    0x25 => {
                        if sa == 0 {
                            Opcode::OR
                        }
                        else {
                            return None
                        }
                    },
                    0x26 => {
                        if sa == 0 {
                            Opcode::XOR
                        }
                        else {
                            return None
                        }
                    },
                    0x27 => {
                        if sa == 0 {
                            Opcode::NOR
                        }
                        else {
                            return None
                        }
                    },
                    0x2A => {
                        if sa == 0 {
                            Opcode::SLT
                        }
                        else {
                            return None
                        }
                    },
                    0x2B => {
                        if sa == 0 {
                            Opcode::SLTU
                        }
                        else {
                            return None
                        }
                    },
                    // N/A
                    _ => return None
                }
            },
            0x01 => {
                match rt {
                    0x00 => Opcode::BLTZ,
                    0x01 => Opcode::BGEZ,
                    0x10 => Opcode::BLTZAL,
                    0x11 => Opcode::BGEZAL,
                    _ => {
                        if (rt & 1) == 0 {
                            Opcode::BLTZ
                        }
                        else if ((rt >> 1) & 7) != 0 {
                            Opcode::BGEZ
                        }
                        else {
                            return None
                        }
                    }
                }
            },
            0x02 => Opcode::J,
            0x03 => Opcode::JAL,
            0x04 => Opcode::BEQ,
            0x05 => Opcode::BNE,
            0x06 => Opcode::BLEZ,
            0x07 => Opcode::BGTZ,
            0x08 => Opcode::ADDI,
            0x09 => Opcode::ADDIU,
            0x0A => Opcode::SLTI,
            0x0B => Opcode::SLTIU,
            0x0C => Opcode::ANDI,
            0x0D => Opcode::ORI,
            0x0E => Opcode::XORI,
            0x0F => Opcode::LUI,
            0x10..=0x13 => { // COP
                match rs {
                    0x00 => Opcode::MFCn,
                    0x02 => Opcode::CFCn,
                    0x04 => Opcode::MTCn,
                    0x06 => Opcode::CTCn,
                    0x08 => {
                        match rt {
                            0x00 => Opcode::BCnF,
                            0x01 => Opcode::BCnT,
                            _ => return None
                        }
                    },
                    _ => {
                        let cop_n = (op & 0xF) as usize;
                        if cop_n == 0 {
                            if rs == 0x10 {
                                match function {
                                    0x01 => Opcode::TLBR,
                                    0x02 => Opcode::TLBWI,
                                    0x06 => Opcode::TLBWR,
                                    0x08 => Opcode::TLBP,
                                    0x10 => Opcode::RFE,
                                    _ => return None
                                }
                            }
                            else {
                                return None
                            }
                        }
                        else {
                            Opcode::COPn
                        }
                    }
                }
            },
            0x20 => Opcode::LB,
            0x21 => Opcode::LH,
            0x22 => Opcode::LWL,
            0x23 => Opcode::LW,
            0x24 => Opcode::LBU,
            0x25 => Opcode::LHU,
            0x26 => Opcode::LWR,
            0x28 => Opcode::SB,
            0x29 => Opcode::SH,
            0x2A => Opcode::SWL,
            0x2B => Opcode::SW,
            0x2E => Opcode::SWR,
            0x30 => Opcode::LWC0,
            0x31 => Opcode::LWC1,
            0x32 => Opcode::LWC2,
            0x33 => Opcode::LWC3,
            0x38 => Opcode::SWC0,
            0x39 => Opcode::SWC1,
            0x3A => Opcode::SWC2,
            0x3B => Opcode::SWC3,

            _ => return None
        };

        Some(opcode)
    }

     */
}