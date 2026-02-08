use std::fs::File;
use std::io;
use std::io::{ErrorKind, Read};
use tracing::warn;
use crate::core::controllers::MemoryCardCommand;

#[derive(Debug)]
pub struct MemoryCard {
    memory: Vec<u8>,
    present: bool,
    file_name: Option<String>,
    memory_card_command: MemoryCardCommand,
    flag: u8,
    sector_number: u16,
    checksum: u8,
    bytes_count: usize,
}

impl MemoryCard {
    const MEM_SIZE : usize = 128 * 1024;

    pub(super) fn new() -> Self {
        let card = MemoryCard {
            memory: vec![],
            present: false,
            file_name: None,
            memory_card_command: MemoryCardCommand::Read,
            flag: 0x8,
            sector_number: 0,
            checksum: 0,
            bytes_count: 0,
        };

        card
    }

    pub(super) fn reset(&mut self) {
        self.flag = 0x8;
        self.bytes_count = 0;
        self.checksum = 0;
    }

    pub(super) fn get_sector_number(&self) -> u16 {
        self.sector_number
    }

    pub(super) fn set_sector_number(&mut self, sector_number: u16) {
        self.sector_number = sector_number;
        self.checksum = ((sector_number >> 8) as u8) ^ (sector_number as u8); // MSB xor LSB
        self.bytes_count = 0;
    }

    /*
    Total Memory 128KB = 131072 bytes = 20000h bytes
      1 Block 8KB = 8192 bytes = 2000h bytes
      1 Frame 128 bytes = 80h bytes
     */
    pub(super) fn read_sector_data(&mut self) -> (u8,bool) {
        if self.bytes_count > 127 {
            return (0xFF,true)
        }
        let byte = self.memory[(self.sector_number << 7) as usize + self.bytes_count]; // 1 sector = 128 bytes
        self.bytes_count += 1;
        self.checksum ^= byte;
        (byte,self.bytes_count == 128)
    }

    pub(super) fn write_sector_data(&mut self,byte:u8) -> bool {
        if self.bytes_count > 127 {
            return true;
        }
        // FLAG byte
        // Bit3=1 is indicating that the directory wasn't read yet (allowing to sense memory card changes).
        // For some strange reason, bit3 is NOT reset when reading from the card, but rather when writing to it.
        self.flag &= !(1 << 3);
        self.memory[(self.sector_number << 7) as usize + self.bytes_count] = byte;
        self.bytes_count += 1;
        self.checksum ^= byte;
        self.bytes_count == 128
    }

    pub(super) fn get_checksum(&self) -> u8 {
        self.checksum
    }

    pub(super) const fn get_id(&self) -> u16 {
        0x5A5D
    }

    pub(super) const fn get_command_ack(&self) -> u16 {
        0x5C5D
    }

    pub(super) fn get_flag(&self) -> u8 {
        self.flag
    }

    pub(super) fn set_command(&mut self, command: MemoryCardCommand) {
        self.memory_card_command = command;
    }

    pub(super) fn get_command(&self) -> &MemoryCardCommand {
        &self.memory_card_command
    }

    pub fn set_present(&mut self,present:bool) {
        let size = if present { Self::MEM_SIZE } else { 0 };
        self.memory.resize(size,0);
        self.present = present;
    }

    pub fn is_present(&self) -> bool {
        self.present
    }

    pub fn set_file_name(&mut self,file_name:String) -> io::Result<()> {
        let mut file = File::open(&file_name)?;
        self.memory.clear();
        let n = file.read_to_end(&mut self.memory)?;
        if n != Self::MEM_SIZE {
            warn!("Invalid memory card snapshot {}: expected {} bytes, found {}",file_name,Self::MEM_SIZE,n);
            Err::<(),io::Error>(io::Error::new(ErrorKind::InvalidData,"Invalid memory card snapshot size"))
        }
        else {
            self.present = true;
            self.file_name = Some(file_name);
            Ok(())
        }
    }
}