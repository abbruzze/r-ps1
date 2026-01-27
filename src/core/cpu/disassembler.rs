use crate::core::cpu::cop0::COP0_REGISTER_ALIASES;
use crate::core::cpu::cop2::{COP2_CONTROL_REGISTER_ALIASES, COP2_DATA_REGISTER_ALIASES};
use crate::core::cpu::instruction::Instruction;
use crate::core::cpu::instruction::Opcode;
use crate::core::cpu::instruction::Opcode::SRL;

#[derive(Debug)]
pub struct Disassembled {
    pub address: u32,
    pub opcode: Opcode,
    pub parameters: String,
    pub formatted: String,
}

static REGISTER_ALIASES: [ &str; 32 ] = [
    "$zero", "$at", "$v0", "$v1", "$a0", "$a1", "$a2", "$a3",
    "$t0", "$t1", "$t2", "$t3", "$t4", "$t5", "$t6", "$t7",
    "$s0", "$s1", "$s2", "$s3", "$s4", "$s5", "$s6", "$s7",
    "$t8", "$t9", "$k0", "$k1", "$gp", "$sp", "$fp", "$ra",
];

pub static USE_REGISTER_ALIASES: bool = true;

pub fn register_alias(reg_index: usize) -> String {
    if USE_REGISTER_ALIASES {
        String::from(REGISTER_ALIASES[reg_index])
    }
    else {
        format!("${}", reg_index)
    }
}

pub fn cop_register_alias(cop_index:usize,reg_index:usize,data:bool) -> String {
    if USE_REGISTER_ALIASES {
        match cop_index {
            0 => return String::from(COP0_REGISTER_ALIASES[reg_index]),
            2 => {
                if data {
                    return String::from(COP2_DATA_REGISTER_ALIASES[reg_index])
                }
                else {
                    return String::from(COP2_CONTROL_REGISTER_ALIASES[reg_index])
                }
            }
            _ => {}
        }
    }
    format!("$cop_r{}",reg_index)
}

/*
Disassembles a single MIPS instruction into a human-readable string.

Primary opcode field (Bit 26..31)
  00h=SPECIAL 08h=ADDI  10h=COP0 18h=N/A   20h=LB   28h=SB   30h=LWC0 38h=SWC0
  01h=BcondZ  09h=ADDIU 11h=COP1 19h=N/A   21h=LH   29h=SH   31h=LWC1 39h=SWC1
  02h=J       0Ah=SLTI  12h=COP2 1Ah=N/A   22h=LWL  2Ah=SWL  32h=LWC2 3Ah=SWC2
  03h=JAL     0Bh=SLTIU 13h=COP3 1Bh=N/A   23h=LW   2Bh=SW   33h=LWC3 3Bh=SWC3
  04h=BEQ     0Ch=ANDI  14h=N/A  1Ch=N/A   24h=LBU  2Ch=N/A  34h=N/A  3Ch=N/A
  05h=BNE     0Dh=ORI   15h=N/A  1Dh=N/A   25h=LHU  2Dh=N/A  35h=N/A  3Dh=N/A
  06h=BLEZ    0Eh=XORI  16h=N/A  1Eh=N/A   26h=LWR  2Eh=SWR  36h=N/A  3Eh=N/A
  07h=BGTZ    0Fh=LUI   17h=N/A  1Fh=N/A   27h=N/A  2Fh=N/A  37h=N/A  3Fh=N/A

Secondary opcode field (Bit 0..5) (when Primary opcode = 00h)
  00h=SLL   08h=JR      10h=MFHI 18h=MULT  20h=ADD  28h=N/A  30h=N/A  38h=N/A
  01h=N/A   09h=JALR    11h=MTHI 19h=MULTU 21h=ADDU 29h=N/A  31h=N/A  39h=N/A
  02h=SRL   0Ah=N/A     12h=MFLO 1Ah=DIV   22h=SUB  2Ah=SLT  32h=N/A  3Ah=N/A
  03h=SRA   0Bh=N/A     13h=MTLO 1Bh=DIVU  23h=SUBU 2Bh=SLTU 33h=N/A  3Bh=N/A
  04h=SLLV  0Ch=SYSCALL 14h=N/A  1Ch=N/A   24h=AND  2Ch=N/A  34h=N/A  3Ch=N/A
  05h=N/A   0Dh=BREAK   15h=N/A  1Dh=N/A   25h=OR   2Dh=N/A  35h=N/A  3Dh=N/A
  06h=SRLV  0Eh=N/A     16h=N/A  1Eh=N/A   26h=XOR  2Eh=N/A  36h=N/A  3Eh=N/A
  07h=SRAV  0Fh=N/A     17h=N/A  1Fh=N/A   27h=NOR  2Fh=N/A  37h=N/A  3Fh=N/A

 31..26 |25..21|20..16|15..11|10..6 |  5..0  |
   6bit  | 5bit | 5bit | 5bit | 5bit |  6bit  |
  -------+------+------+------+------+--------+------------
  000000 | N/A  | rt   | rd   | imm5 | 0000xx | shift-imm
  000000 | rs   | rt   | rd   | N/A  | 0001xx | shift-reg
  000000 | rs   | N/A  | N/A  | N/A  | 001000 | jr
  000000 | rs   | N/A  | rd   | N/A  | 001001 | jalr
  000000 | <-----comment20bit------> | 00110x | sys/brk
  000000 | N/A  | N/A  | rd   | N/A  | 0100x0 | mfhi/mflo
  000000 | rs   | N/A  | N/A  | N/A  | 0100x1 | mthi/mtlo
  000000 | rs   | rt   | N/A  | N/A  | 0110xx | mul/div
  000000 | rs   | rt   | rd   | N/A  | 10xxxx | alu-reg
  000001 | rs   | 00000| <--immediate16bit--> | bltz
  000001 | rs   | 00001| <--immediate16bit--> | bgez
  000001 | rs   | 10000| <--immediate16bit--> | bltzal
  000001 | rs   | 10001| <--immediate16bit--> | bgezal
  000001 | rs   | xxxx0| <--immediate16bit--> | bltz  ;\undocumented dupes
  000001 | rs   | xxxx1| <--immediate16bit--> | bgez  ;/(when bit17-19=nonzero)
  00001x | <---------immediate26bit---------> | j/jal
  00010x | rs   | rt   | <--immediate16bit--> | beq/bne
  00011x | rs   | N/A  | <--immediate16bit--> | blez/bgtz
  001xxx | rs   | rt   | <--immediate16bit--> | alu-imm
  001111 | N/A  | rt   | <--immediate16bit--> | lui-imm
  100xxx | rs   | rt   | <--immediate16bit--> | load rt,[rs+imm]
  101xxx | rs   | rt   | <--immediate16bit--> | store rt,[rs+imm]
  x1xxxx | <------coprocessor specific------> | coprocessor (see below)

Coprocessor Opcode/Parameter Encoding
  31..26 |25..21|20..16|15..11|10..6 |  5..0  |
   6bit  | 5bit | 5bit | 5bit | 5bit |  6bit  |
  -------+------+------+------+------+--------+------------
  0100nn |0|0000| rt   | rd   | N/A  | 000000 | MFCn rt,rd_dat  ;rt = dat
  0100nn |0|0010| rt   | rd   | N/A  | 000000 | CFCn rt,rd_cnt  ;rt = cnt
  0100nn |0|0100| rt   | rd   | N/A  | 000000 | MTCn rt,rd_dat  ;dat = rt
  0100nn |0|0110| rt   | rd   | N/A  | 000000 | CTCn rt,rd_cnt  ;cnt = rt
  0100nn |0|1000|00000 | <--immediate16bit--> | BCnF target ;jump if false
  0100nn |0|1000|00001 | <--immediate16bit--> | BCnT target ;jump if true
  0100nn |1| <--------immediate25bit--------> | COPn imm25
  010000 |1|0000| N/A  | N/A  | N/A  | 000001 | COP0 01h  ;=TLBR   ;\if any
  010000 |1|0000| N/A  | N/A  | N/A  | 000010 | COP0 02h  ;=TLBWI  ; (not on
  010000 |1|0000| N/A  | N/A  | N/A  | 000110 | COP0 06h  ;=TLBWR  ; psx)
  010000 |1|0000| N/A  | N/A  | N/A  | 001000 | COP0 08h  ;=TLBP   ;/
  010000 |1|0000| N/A  | N/A  | N/A  | 010000 | COP0 10h  ;=RFE
  1100nn | rs   | rt   | <--immediate16bit--> | LWCn rt_dat,[rs+imm]
  1110nn | rs   | rt   | <--immediate16bit--> | SWCn rt_dat,[rs+imm]
 */
pub fn disassemble(pc:u32,instruction: u32) -> Disassembled {
    let instr = Instruction(instruction);
    let opcode = Opcode::from_instruction(&instr);
    let mut alias_opcode : Option<String> = None;
    use Opcode::*;
    let op = instr.op();
    let rs = instr.rs();
    let rd = instr.rd();
    let rt = instr.rt();
    let sa = instr.shift_amount();

    let parameters = match opcode {
        NOP => String::from(""),
        SLL => format!("{}, {}, {:X}", register_alias(rd), register_alias(rt), sa),
        SRL => format!("{}, {}, 0x{:X}", register_alias(rd), register_alias(rt), sa),
        SRA => format!("{}, {}, 0x{:X}", register_alias(rd), register_alias(rt), sa),
        SLLV => format!("{}, {}, {}", register_alias(rd), register_alias(rt), register_alias(rs)),
        SRLV => format!("{}, {}, {}", register_alias(rd), register_alias(rt), register_alias(rs)),
        SRAV => format!("{}, {}, {}", register_alias(rd), register_alias(rt), register_alias(rs)),
        JR => format!("{}", register_alias(rs)),
        JALR => {
            if rd == 31 {
                format!("{}", register_alias(rs))
            }
            else {
                format!("{}, {}", register_alias(rd), register_alias(instr.rs()))
            }
        },
        SYSCALL => format!("0x{:X}",instr.imm20()),
        BREAK => format!("0x{:X}",instr.imm20()),
        MFHI => format!("{}", register_alias(rd)),
        MTHI => format!("{}", register_alias(rs)),
        MFLO => format!("{}", register_alias(rd)),
        MTLO => format!("{}", register_alias(rs)),
        MULT => format!("{}, {}", register_alias(rs), register_alias(rt)),
        MULTU => format!("{}, {}", register_alias(rs), register_alias(rt)),
        DIV => format!("{}, {}", register_alias(rs), register_alias(rt)),
        DIVU => format!("{}, {}", register_alias(rs), register_alias(rt)),
        ADD => format!("{}, {}, {}", register_alias(rd), register_alias(rs), register_alias(rt)),
        ADDU => {
            if rt == 0 {
                alias_opcode = Some(String::from("move"));
                format!("{}, {}", register_alias(rd), register_alias(rs))
            }
            else {
                format!("{}, {}, {}", register_alias(rd), register_alias(rs), register_alias(rt))
            }
        }
        SUB => format!("{}, {}, {}", register_alias(rd), register_alias(rs), register_alias(rt)),
        SUBU => format!("{}, {}, {}", register_alias(rd), register_alias(rs), register_alias(rt)),
        AND => format!("{}, {}, {}", register_alias(rd), register_alias(rs), register_alias(rt)),
        OR => {
            if rt == 0 {
                alias_opcode = Some(String::from("move"));
                format!("{}, {}", register_alias(rd), register_alias(rs))
            }
            else {
                format!("{}, {}, {}", register_alias(rd), register_alias(rs), register_alias(rt))
            }
        },
        XOR => format!("{}, {}, {}", register_alias(rd), register_alias(rs), register_alias(rt)),
        NOR => format!("{}, {}, {}", register_alias(rd), register_alias(rs), register_alias(rt)),
        SLT => format!("{}, {}, {}", register_alias(rd), register_alias(rs), register_alias(rt)),
        SLTU => format!("{}, {}, {}", register_alias(rd), register_alias(rs), register_alias(rt)),

        BLTZ => {
            let offset = instr.signed_immediate16() << 2;
            let base = pc.wrapping_add(4);
            let target = base.wrapping_add(offset);
            format!("{}, 0x{:08X}", register_alias(rs), target)
        },
        BGEZ => {
            let offset = instr.signed_immediate16() << 2;
            let base = pc.wrapping_add(4);
            let target = base.wrapping_add(offset);
            format!("{}, 0x{:08X}", register_alias(rs), target)
        },
        BLTZAL => {
            let offset = instr.signed_immediate16() << 2;
            let base = pc.wrapping_add(4);
            let target = base.wrapping_add(offset);
            format!("{}, 0x{:08X}", register_alias(rs), target)
        },
        BGEZAL => {
            let offset = instr.signed_immediate16() << 2;
            let base = pc.wrapping_add(4);
            let target = base.wrapping_add(offset);
            format!("{}, 0x{:08X}", register_alias(rs), target)
        },
        J => {
            let base = pc;
            let target = (base & 0xF000_0000) | (instr.imm26() << 2);
            format!("0x{:08X}", target)
        },
        JAL => {
            let base = pc;
            let target = (base & 0xF000_0000) | (instr.imm26() << 2);
            format!("0x{:08X}", target)
        },
        BEQ => {
            let offset = instr.signed_immediate16() << 2;
            let base = pc.wrapping_add(4);
            let target = base.wrapping_add(offset);
            if rt == 0 {
                alias_opcode = Some(String::from("beqz"));
                format!("{}, {:08X}",register_alias(rs),target)
            }
            else {
                format!("{}, {}, {:08X}",register_alias(rs),register_alias(rt),target)
            }

        },
        BNE => {
            let offset = instr.signed_immediate16() << 2;
            let base = pc.wrapping_add(4);
            let target = base.wrapping_add(offset);
            if rt == 0 {
                alias_opcode = Some(String::from("bnez"));
                format!("{}, {:08X}",register_alias(rs),target)
            }
            else {
                format!("{}, {}, {:08X}",register_alias(rs),register_alias(rt),target)
            }
        },
        BLEZ => {
            let offset = instr.signed_immediate16() << 2;
            let base = pc.wrapping_add(4);
            let target = base.wrapping_add(offset);
            format!("{}, {:08X}",register_alias(rs),target)
        },
        BGTZ => {
            let offset = instr.signed_immediate16() << 2;
            let base = pc.wrapping_add(4);
            let target = base.wrapping_add(offset);
            format!("{}, {:08X}",register_alias(rs),target)
        },
        ADDI => format!("{}, {}, {}",register_alias(rt),register_alias(rs),hex16_signed(instr.signed_immediate16())),
        ADDIU => format!("{}, {}, {}",register_alias(rt),register_alias(rs),hex16_signed(instr.signed_immediate16())),
        SLTI => format!("{}, {}, {}",register_alias(rt),register_alias(rs),hex16_signed(instr.signed_immediate16())),
        SLTIU => format!("{}, {}, {}",register_alias(rt),register_alias(rs),hex16_signed(instr.signed_immediate16())),
        ANDI => format!("{}, {}, {:04X}",register_alias(rt),register_alias(rs),instr.unsigned_immediate16()),
        ORI => format!("{}, {}, {:04X}",register_alias(rt),register_alias(rs),instr.unsigned_immediate16()),
        XORI => format!("{}, {}, {:04X}",register_alias(rt),register_alias(rs),instr.unsigned_immediate16()),
        LUI => format!("{}, {:04X}",register_alias(rt),instr.unsigned_immediate16()),

        MFCn => {
            alias_opcode = Some(format!("mfc{}",op & 0xF));
            format!("{}, {}", register_alias(rt), cop_register_alias(op as usize & 0xF,rd,true))
        },
        CFCn => {
            alias_opcode = Some(format!("cfc{}",op & 0xF));
            format!("{}, {}", register_alias(rt), cop_register_alias(op as usize & 0xF,rd,false))
        },
        MTCn => {
            alias_opcode = Some(format!("mtc{}",op & 0xF));
            format!("{}, {}", register_alias(rt), cop_register_alias(op as usize & 0xF,rd,true))
        },
        CTCn => {
            alias_opcode = Some(format!("ctc{}",op & 0xF));
            format!("{}, {}", register_alias(rt), cop_register_alias(op as usize & 0xF,rd,false))
        },
        COPn => {
            alias_opcode = Some(format!("cop{}",op & 0xF));
            format!("{:03X}", instr.imm25())
        },
        BCnF => format!("{:04X}",instr.unsigned_immediate16()),
        BCnT => format!("{:04X}",instr.unsigned_immediate16()),
        TLBR => String::from(""),
        TLBWI => String::from(""),
        TLBWR => String::from(""),
        TLBP => String::from(""),
        RFE => String::from(""),
        LB|LH|LWL|LW|LBU|LHU|LWR => format!("{},{}({})",register_alias(rt),hex16_signed(instr.signed_immediate16()),register_alias(rs)),
        SB|SH|SWL|SW|SWR => format!("{},{}({})",register_alias(rt),hex16_signed(instr.signed_immediate16()),register_alias(rs)),
        LWC0|LWC1|LWC2|LWC3 => format!("{},{}({})",cop_register_alias(op as usize & 0xF,rt,false),hex16_signed(instr.signed_immediate16()),register_alias(rs)),
        SWC0|SWC1|SWC2|SWC3 => format!("{},{}({})",cop_register_alias(op as usize & 0xF,rt,false),hex16_signed(instr.signed_immediate16()),register_alias(rs)),
        UNKNOWN => {
            alias_opcode = Some(String::from("???"));
            String::from("")
        }
    };

    let opcode_string = if let Some(opcode) = alias_opcode {
        format!("{}", opcode.to_lowercase())
    }
    else {
        format!("{:?}", opcode).to_lowercase()
    };
    let formatted = format!("{:<7}{}", opcode_string, parameters.trim());

    let dump = format!("{:08X} {:08X} {}", pc, instruction, formatted);

    Disassembled { address: pc, opcode, parameters, formatted: dump }
}

fn hex16_signed(v: u32) -> String {
    let iv = v as i16;
    if iv < 0 {
        format!("-{:04X}", (-iv) as u16)
    } else {
        format!("{:04X}", iv as u16)
    }
}