use tracing::{info, warn};

#[derive(Copy, Clone, Debug)]
pub enum ControllerButton {
    Select,
    L3,
    R3,
    Start,
    Up,
    Right,
    Down,
    Left,
    L2,
    R2,
    L1,
    R1,
    Triangle,
    Circle,
    Cross,
    Square,
}

#[derive(Debug,Default)]
enum ControllerMode {
    #[default]
    Digital,
    Analog
}
impl ControllerMode {
    fn id(&self) -> u16 {
        match self {
            ControllerMode::Digital => 0x5A41,
            ControllerMode::Analog => 0x5A53,
        }
    }
    fn is_digital(&self) -> bool {
        matches!(self, ControllerMode::Digital)
    }
}
#[derive(Debug,Default)]
enum ControllerState {
    #[default]
    Init,
    IdLo,
    IdHi,
    SwLo,
    SwHi,
    Analog0,
    Analog1,
    Analog2,
    Analog3,
}

#[derive(Debug)]
pub struct Controller {
    id: u8,
    mode: ControllerMode,
    digital_switches: u16,
    analog_switches: u32,
    state: ControllerState,
    connected: bool,
}

impl Controller {
    pub fn new(id:u8,connected: bool) -> Controller {
        Controller {
            id,
            mode: ControllerMode::Digital,
            digital_switches: 0xFFFF,
            analog_switches: 0,
            state: ControllerState::Init,
            connected
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected
    }

    pub fn set_connected(&mut self,connected:bool) {
        self.connected = connected;
    }

    pub fn on_key(&mut self,key:ControllerButton,pressed:bool) {
        if self.connected {
            if pressed {
                self.digital_switches &= !(1 << (key as u16));
            } else {
                self.digital_switches |= 1 << (key as u16);
            }
        }
    }

    pub fn reset(&mut self) {
        self.state = ControllerState::Init;
    }

    pub fn ack(&self) -> bool {
        !matches!(self.state,ControllerState::Init)
    }

    pub fn read_byte_after_command(&mut self,cmd:u8) -> u8 {
        if !self.connected {
            return 0xFF;
        }

        match self.state {
            ControllerState::Init => {
                if cmd == 0x01 {
                    self.state = ControllerState::IdLo;
                }
                0xFF
            }
            ControllerState::IdLo => {
                if cmd == 0x42 {
                    self.state = ControllerState::IdHi;
                    self.mode.id() as u8
                }
                else {
                    warn!("Unexpected controller command on state {:?}: {:02X}",self.state,cmd);
                    self.state = ControllerState::Init;
                    0xFF
                }
            }
            ControllerState::IdHi => {
                self.state = ControllerState::SwLo;
                (self.mode.id() >> 8) as u8
            }
            ControllerState::SwLo => {
                self.state = ControllerState::SwHi;
                self.digital_switches as u8
            }
            ControllerState::SwHi => {
                self.state = if self.mode.is_digital() {
                    ControllerState::Init
                }
                else {
                    ControllerState::Analog0
                };
                // if self.digital_switches != 0xFFFF {
                //     println!("Sending switches: {:04X}",self.digital_switches);
                // }
                (self.digital_switches >> 8) as u8
            }
            ControllerState::Analog0 => {
                if cmd == 0x00 {
                    self.state = ControllerState::Analog1;
                    (self.analog_switches & 0xFF) as u8
                }
                else {
                    warn!("Unexpected controller command on state {:?}: {:02X}",self.state,cmd);
                    self.state = ControllerState::Init;
                    0xFF
                }
            }
            ControllerState::Analog1 => {
                if cmd == 0x00 {
                    self.state = ControllerState::Analog2;
                    ((self.analog_switches >> 8) & 0xFF) as u8
                }
                else {
                    warn!("Unexpected controller command on state {:?}: {:02X}",self.state,cmd);
                    self.state = ControllerState::Init;
                    0xFF
                }
            }
            ControllerState::Analog2 => {
                if cmd == 0x00 {
                    self.state = ControllerState::Analog3;
                    ((self.analog_switches >> 16) & 0xFF) as u8
                }
                else {
                    warn!("Unexpected controller command on state {:?}: {:02X}",self.state,cmd);
                    self.state = ControllerState::Init;
                    0xFF
                }
            }
            ControllerState::Analog3 => {
                if cmd == 0x00 {
                    self.state = ControllerState::Init;
                    ((self.analog_switches >> 24) & 0xFF) as u8
                }
                else {
                    warn!("Unexpected controller command on state {:?}: {:02X}",self.state,cmd);
                    self.state = ControllerState::Init;
                    0xFF
                }
            }
        }
    }
}