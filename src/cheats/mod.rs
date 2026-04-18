use tracing::error;
use crate::core::memory::{Memory, ReadMemoryAccess};

/*

  30aaaaaa 00dd   ;-8bit Write  [aaaaaa]=dd
  80aaaaaa dddd   ;-16bit Write [aaaaaa]=dddd

  D0aaaaaa dddd   ;-16bit/Equal     If dddd=[aaaaaa] then (exec next code)
  D1aaaaaa dddd   ;-16bit/NotEqual  If dddd<>[aaaaaa] then (exec next code)
  D2aaaaaa dddd   ;-16bit/Less      If dddd<[aaaaaa] then (exec next code)
  D3aaaaaa dddd   ;-16bit/Greater   If dddd>[aaaaaa] then (exec next code)
  E0aaaaaa 00dd   ;-8bit/Equal      If dd=[aaaaaa] then (exec next code)
  E1aaaaaa 00dd   ;-8bit/NotEqual   If dd<>[aaaaaa] then (exec next code)
  E2aaaaaa 00dd   ;-8bit/Less       If dd<[aaaaaa] then (exec next code)
  E3aaaaaa 00dd   ;-8bit/Greater    If dd>[aaaaaa] then (exec next code)
  10aaaaaa dddd   ;-16bit Increment [aaaaaa]=[aaaaaa]+dddd
  11aaaaaa dddd   ;-16bit Decrement [aaaaaa]=[aaaaaa]-dddd
  20aaaaaa 00dd   ;-8bit Increment  [aaaaaa]=[aaaaaa]+dd
  21aaaaaa 00dd   ;-8bit Decrement  [aaaaaa]=[aaaaaa]-dd

  5000nnbb dddd   ;\Slide Code aka Patch Code aka Serial Repeater
  aaaaaaaa ??ee   ;/for i=0 to nn-1, [aaaaaaaa+(i*bb)]=dddd+(i*??ee), next i
  00000000 0000   ;-Dummy (do nothing?) needed between slides (CD version only)
 */

#[derive(Debug)]
pub enum CheatIfOp {
    Equal,
    NotEqual,
    GreaterThan,
    LessThan,
}
#[derive(Debug)]
pub enum Cheat {
    WriteMemory {
        address: u32,
        value: u32,
        is_byte: bool,
    },
    If {
        address: u32,
        value: u32,
        op: CheatIfOp,
        is_byte: bool,
    },
    IncDec {
        address: u32,
        value: u32,
        is_byte: bool,
        is_inc: bool,
    },
    Patch {
        nn: u8,
        bb: u8,
        dddd: u32,
        op: Option<Box<Cheat>>,
    }
}
#[derive(Debug)]
pub struct Cheats {
    pub cheats: Vec<Cheat>,
}

impl Cheats {
    pub fn new() -> Self {
        Self { cheats: vec![] }
    }

    pub fn parse(codes:&Vec<String>) -> Self {
        let mut cheats = Cheats::new();
        let mut patch : Option<Cheat> = None;

        for code in codes {
            let upper_code = code.to_uppercase();
            let code_value = upper_code.split_whitespace().collect::<Vec<&str>>();
            if code_value.len() != 2 || code_value[0].len() < 2 {
                error!("Invalid cheat code: {}", code);
                continue;
            }

            let cmd = &code_value[0][0..2];
            match cmd {
                "30"|"80" => {
                    let address = u32::from_str_radix(&code_value[0][2..], 16).ok();
                    let value = u32::from_str_radix(&code_value[1], 16).ok();
                    match (address, value) {
                        (Some(address), Some(value)) => {
                            let cheat = Cheat::WriteMemory { address, value, is_byte: cmd == "30" };

                            if let Some(Cheat::Patch { nn,bb, dddd, .. }) = patch.take() {
                                cheats.add(Cheat::Patch { nn, bb, dddd, op: Some(Box::new(cheat)) })
                            }
                            else {
                                cheats.add(cheat);
                            }
                        },
                        _ => {
                            error!("Syntax error on cheat code: {}", code);
                        }
                    }
                }
                "D0"|"D1"|"D2"|"D3"|"E0"|"E1"|"E2"|"E3" => {
                    let address = u32::from_str_radix(&code_value[0][2..], 16).ok();
                    let value = u32::from_str_radix(&code_value[1], 16).ok();
                    match (address, value) {
                        (Some(address), Some(value)) => {
                            let if_op = match &cmd[1..2] {
                                "0" => CheatIfOp::Equal,
                                "1" => CheatIfOp::NotEqual,
                                "2" => CheatIfOp::LessThan,
                                "3" => CheatIfOp::GreaterThan,
                                _ => unreachable!(),
                            };
                            cheats.add(Cheat::If { address, value, op: if_op, is_byte: &cmd[0..1] == "E" });
                        },
                        _ => {
                            error!("Syntax error on cheat code: {}", code);
                        }
                    }
                }
                "10"|"11"|"20"|"21" => {
                    let address = u32::from_str_radix(&code_value[0][2..], 16).ok();
                    let value = u32::from_str_radix(&code_value[1], 16).ok();
                    match (address, value) {
                        (Some(address), Some(value)) => {
                            let cheat = Cheat::IncDec { address, value, is_byte: &cmd[0..1] == "2", is_inc: &cmd[1..2] == "0" };
                            if let Some(Cheat::Patch { nn,bb, dddd, .. }) = patch.take() {
                                cheats.add(Cheat::Patch { nn, bb, dddd, op: Some(Box::new(cheat)) })
                            }
                            else {
                                cheats.add(cheat);
                            }
                        },
                        _ => {
                            error!("Syntax error on cheat code: {}", code);
                        }
                    }
                }
                "50" => {
                    let nn = u8::from_str_radix(&code_value[0][4..6], 16).ok();
                    let bb = u8::from_str_radix(&code_value[0][6..], 16).ok();
                    let dddd = u32::from_str_radix(&code_value[1], 16).ok();
                    match (nn, bb, dddd) {
                        (Some(nn), Some(bb), Some(dddd)) => {
                            patch = Some(Cheat::Patch { nn, bb,dddd, op: None   });
                        },
                        _ => {
                            error!("Syntax error on cheat code: {}", code);
                        }
                    }
                }
                _ => {
                    error!("Unsupported cheat code's command: {}", code);
                }
            }
        }
        cheats
    }

    pub fn add(&mut self, cheat: Cheat) {
        self.cheats.push(cheat);
    }

    pub fn apply<M : Memory>(&self,mem:&mut M) {
        let mut index = 0;
        while index < self.cheats.len() {
            let delta_index = self.apply_cheat(mem,self.cheats.get(index).unwrap());
            index += delta_index;
        }
    }

    fn apply_cheat<M : Memory>(&self,mem:&mut M,cheat: &Cheat) -> usize {
        //println!("Apply cheat: {:?}",cheat);
        match cheat {
            Cheat::WriteMemory { address, value, is_byte } => {
                //println!("Writing {:08X} to {:08X} [{is_byte}]",Self::adjust_value(*value,*is_byte),Self::get_address(*address));
                Self::write_memory(mem,*address,Self::adjust_value(*value,*is_byte),*is_byte);
                1
            }
            Cheat::If { address, value, op, is_byte } => {
                if Self::compare(Self::read_memory(mem,*address,*is_byte),op,Self::adjust_value(*value,*is_byte)) {
                    1
                }
                else {
                    2
                }
            }
            Cheat::IncDec { address, value, is_byte, is_inc } => {
                let read_value = Self::read_memory(mem,*address,*is_byte);
                let write_value = if *is_inc {
                    Self::adjust_value(read_value + Self::adjust_value(*value,*is_byte),*is_byte)
                }
                else {
                    Self::adjust_value(read_value.saturating_sub(Self::adjust_value(*value,*is_byte)),*is_byte)
                };
                Self::write_memory(mem,*address,write_value,*is_byte);
                1
            }
            Cheat::Patch { nn, bb, dddd, op } => {
                let cheat = op.as_ref().unwrap();
                for i in 0..*nn {
                    let delta_address = i as u32 * (*bb as u32);
                    match cheat.as_ref() {
                        Cheat::WriteMemory { address, value, is_byte } => {
                            let delta_value =  i as u32 * *value;
                            self.apply_cheat(mem,&Cheat::WriteMemory { address: *address + delta_address, value: *dddd + delta_value, is_byte: *is_byte });
                        }
                        Cheat::IncDec { address, value, is_byte, is_inc } => {
                            let delta_value =  i as u32 * *value;
                            self.apply_cheat(mem,&Cheat::IncDec { address: *address + delta_address, value: *dddd + delta_value, is_byte: *is_byte, is_inc: *is_inc });
                        }
                        _ => {/*ignored*/}
                    }
                }
                1
            }
        }
    }

    #[inline(always)]
    fn adjust_value(value:u32,is_byte:bool) -> u32 {
        if is_byte {
            value & 0xFF
        }
        else {
            value & 0xFFFF
        }
    }

    #[inline(always)]
    fn write_memory<M : Memory>(mem:&mut M,address:u32,value:u32,is_byte:bool) {
        let address = Self::get_address(address);
        if is_byte {
            mem.write::<8>(address,value);
        }
        else {
            mem.write::<16>(address,value);
        }
    }

    #[inline(always)]
    fn read_memory<M : Memory>(mem:&mut M,address:u32,is_byte:bool) -> u32 {
        let address = Self::get_address(address);
        let read = if is_byte {
            mem.read::<8>(address,false)
        }
        else {
            mem.read::<16>(address,false)
        };

        match read  {
            ReadMemoryAccess::Read(value, _) => value,
            ReadMemoryAccess::BusError => 0,
            ReadMemoryAccess::MemoryError => 0,
            ReadMemoryAccess::Wait => 0,
        }
    }

    #[inline(always)]
    fn compare(read_value:u32, op:&CheatIfOp, compare:u32) -> bool {
        match op {
            CheatIfOp::Equal => read_value == compare,
            CheatIfOp::NotEqual => read_value != compare,
            CheatIfOp::GreaterThan => compare > read_value,
            CheatIfOp::LessThan => compare < read_value,
        }
    }

    #[inline(always)]
    fn get_address(a:u32) -> u32 {
        a & 0xFFFFFF | 0x80000000
    }
}