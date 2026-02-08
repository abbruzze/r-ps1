mod memory_card;

use tracing::{info, warn};
use crate::core::controllers::memory_card::MemoryCard;

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
    // memory card
    MemCommand,
    MemId1,
    MemId2,
    MemLSB,
    MemMSB,
    MemAck1,
    MemAck2,
    MemConfirmedLSB,
    MemConfirmedMSB,
    MemReadDataSector,
    MemSendDataSector,
    MemChecksum,
    MemEndByteRead,
    MemEndByteWrite,
    MemGetIdEpilogue(usize),
}
#[derive(Debug)]
enum MemoryCardCommand {
    Read,
    Write,
    GetId,
}

#[derive(Debug)]
pub struct Controller {
    id: u8,
    mode: ControllerMode,
    digital_switches: u16,
    analog_switches: u32,
    state: ControllerState,
    connected: bool,
    memory_card: MemoryCard,
    memory_card_selected: bool,
    memory_card_sector: u16,
    last_cmd: u8,
    write_cheksum: u8,
}

impl Controller {
    pub fn new(id:u8,connected: bool) -> Controller {
        Controller {
            id,
            mode: ControllerMode::Digital,
            digital_switches: 0xFFFF,
            analog_switches: 0,
            state: ControllerState::Init,
            connected,
            memory_card: MemoryCard::new(),
            memory_card_selected: false,
            memory_card_sector: 0,
            last_cmd: 0,
            write_cheksum: 0,
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
        self.memory_card.reset();
        self.memory_card_selected = false;
    }

    pub fn ack(&self) -> bool {
        !matches!(self.state,ControllerState::Init)
    }

    /*
    Reading Data from Memory Card
      Send Reply Comment
      81h  N/A   Memory card address
      52h  FLAG  Send Read Command (ASCII "R"), Receive FLAG Byte
      00h  5Ah   Receive Memory Card ID1
      00h  5Dh   Receive Memory Card ID2
      MSB  (00h) Send Address MSB  ;\sector number (0..3FFh)
      LSB  (pre) Send Address LSB  ;/
      00h  5Ch   Receive Command Acknowledge 1  ;<-- late /ACK after this byte-pair
      00h  5Dh   Receive Command Acknowledge 2
      00h  MSB   Receive Confirmed Address MSB
      00h  LSB   Receive Confirmed Address LSB
      00h  ...   Receive Data Sector (128 bytes)
      00h  CHK   Receive Checksum (MSB xor LSB xor Data bytes)
      00h  47h   Receive Memory End Byte (should be always 47h="G"=Good for Read)

    Writing Data to Memory Card
      Send Reply Comment
      81h  N/A   Memory card address
      57h  FLAG  Send Write Command (ASCII "W"), Receive FLAG Byte
      00h  5Ah   Receive Memory Card ID1
      00h  5Dh   Receive Memory Card ID2
      MSB  (00h) Send Address MSB  ;\sector number (0..3FFh)
      LSB  (pre) Send Address LSB  ;/
      ...  (pre) Send Data Sector (128 bytes)
      CHK  (pre) Send Checksum (MSB xor LSB xor Data bytes)
      00h  5Ch   Receive Command Acknowledge 1
      00h  5Dh   Receive Command Acknowledge 2
      00h  4xh   Receive Memory End Byte (47h=Good, 4Eh=BadChecksum, FFh=BadSector)

   Get Memory Card ID Command
      Send Reply Comment
      81h  N/A   Memory card address
      53h  FLAG  Send Get ID Command (ASCII "S"), Receive FLAG Byte
      00h  5Ah   Receive Memory Card ID1
      00h  5Dh   Receive Memory Card ID2
      00h  5Ch   Receive Command Acknowledge 1
      00h  5Dh   Receive Command Acknowledge 2
      00h  04h   Receive 04h
      00h  00h   Receive 00h
      00h  00h   Receive 00h
      00h  80h   Receive 80h

   Invalid Commands
      Send Reply Comment
      81h  N/A   Memory card address
      xxh  FLAG  Send Invalid Command (anything else than "R", "W", or "S")
     Transfer aborts immediately after the faulty command byte, or, occasionally after one more byte (with response FFh to that extra byte).

   FLAG Byte
    The initial value of the FLAG byte on power-up (and when re-inserting the memory card) is 08h.
    Bit3=1 is indicating that the directory wasn't read yet (allowing to sense memory card changes).
    For some strange reason, bit3 is NOT reset when reading from the card, but rather when writing to it.
    To reset the flag, games are usually issuing a dummy write to sector number 003Fh, more or less unneccessarily stressing the lifetime of that sector.
    Bit2=1 seems to be intended to indicate write errors, however, the write command seems to be always finishing without setting that bit, instead, the error flag may get set on the NEXT command.

      When sending an invalid sector number, original Sony memory cards respond with FFFFh as Confirmed Address
      (and do then abort the transfer without sending any data, checksum, or end flag)
     */
    fn read_mem_card_byte_after_command(&mut self,cmd:u8) -> u8 {
        let response = match self.state {
            ControllerState::MemCommand => {
                match cmd {
                    b'R' => {
                        self.memory_card.set_command(MemoryCardCommand::Read);
                        self.state = ControllerState::MemId1;
                        self.memory_card.get_flag()
                    }
                    b'W' => {
                        self.memory_card.set_command(MemoryCardCommand::Write);
                        self.state = ControllerState::MemId1;
                        self.memory_card.get_flag()
                    }
                    b'S' => {
                        self.memory_card.set_command(MemoryCardCommand::GetId);
                        self.state = ControllerState::MemId1;
                        self.memory_card.get_flag()
                    }
                    _ => {
                        warn!("Unknown command in memory card {:02X}",cmd);
                        self.state = ControllerState::Init;
                        cmd
                    }
                }
            }
            ControllerState::MemId1 => {
                self.state = ControllerState::MemId2;
                (self.memory_card.get_id() >> 8) as u8
            }
            ControllerState::MemId2 => {
                if matches!(self.memory_card.get_command(),MemoryCardCommand::GetId) {
                    self.state = ControllerState::MemAck1;
                }
                else {
                    self.state = ControllerState::MemMSB;
                }
                self.memory_card.get_id() as u8
            }
            ControllerState::MemMSB => {
                self.state = ControllerState::MemLSB;
                self.memory_card_sector = (cmd as u16) << 8;
                0x00
            }
            ControllerState::MemLSB => {
                self.memory_card_sector |= cmd as u16;
                self.memory_card.set_sector_number(self.memory_card_sector);
                if matches!(self.memory_card.get_command(),MemoryCardCommand::Read) {
                    self.state = ControllerState::MemAck1;
                }
                else {
                    self.state = ControllerState::MemSendDataSector;
                }
                self.last_cmd
            }
            ControllerState::MemAck1 => {
                self.state = ControllerState::MemAck2;
                (self.memory_card.get_command_ack() >> 8) as u8
            }
            ControllerState::MemAck2 => {
                self.state = match self.memory_card.get_command() {
                    MemoryCardCommand::Read => ControllerState::MemConfirmedMSB,
                    MemoryCardCommand::GetId => ControllerState::MemGetIdEpilogue(1),
                    MemoryCardCommand::Write => ControllerState::MemEndByteWrite,
                };

                self.memory_card.get_command_ack() as u8
            }
            ControllerState::MemGetIdEpilogue(index) => {
                if index == 4 {
                    self.state = ControllerState::Init;
                    self.memory_card_selected = false;
                }
                else {
                    self.state = ControllerState::MemGetIdEpilogue(index + 1);
                }
                match index {
                    1 => 0x04,
                    2|3 => 0x00,
                    4 => 0x80,
                    _ => unreachable!()
                }
            }
            ControllerState::MemConfirmedMSB => {
                self.state = ControllerState::MemConfirmedLSB;
                let sector = self.memory_card.get_sector_number();
                if sector > 0x3FF {
                    0xFF
                }
                else {
                    (sector >> 8) as u8
                }
            }
            ControllerState::MemConfirmedLSB => {
                let sector = self.memory_card.get_sector_number();
                if sector > 0x3FF {
                    self.state = ControllerState::Init;
                    self.memory_card_selected = false;
                    0xFF
                }
                else {
                    self.state = ControllerState::MemReadDataSector;
                    sector as u8
                }
            }
            ControllerState::MemReadDataSector => {
                let (byte,last) = self.memory_card.read_sector_data();
                if last {
                    self.state = ControllerState::MemChecksum;
                }
                byte
            }
            ControllerState::MemChecksum => {
                if matches!(self.memory_card.get_command(),MemoryCardCommand::Read) {
                    self.state = ControllerState::MemEndByteRead;
                    self.memory_card.get_checksum()
                }
                else {
                    self.write_cheksum = cmd;
                    self.state = ControllerState::MemAck1;
                    self.last_cmd
                }
            }
            ControllerState::MemEndByteRead => {
                self.state = ControllerState::Init;
                self.memory_card_selected = false;
                0x47
            }
            ControllerState::MemSendDataSector => {
                if self.memory_card.write_sector_data(cmd) {
                    self.state = ControllerState::MemChecksum;
                }
                self.last_cmd
            }
            ControllerState::MemEndByteWrite => {
                self.state = ControllerState::Init;
                self.memory_card_selected = false;
                if self.write_cheksum == self.memory_card.get_checksum() {
                    0x47
                }
                else {
                    0x4E
                }
            }
            _ => unreachable!()
        };

        self.last_cmd = cmd;
        response
    }

    pub fn read_byte_after_command(&mut self,cmd:u8) -> u8 {
        if self.memory_card_selected {
            return self.read_mem_card_byte_after_command(cmd);
        }

        match self.state {
            ControllerState::Init => {
                if cmd == 0x01 && self.connected {
                    self.memory_card_selected = false;
                    self.state = ControllerState::IdLo;
                }
                else if cmd == 0x81 && self.memory_card.is_present() {
                    self.memory_card_selected = true;
                    self.state = ControllerState::MemCommand;
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
            _ => unreachable!()
        }
    }
}