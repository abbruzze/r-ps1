use tracing::error;

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
}