use crate::core::clock::Clock;
use crate::core::interrupt::{InterruptController, IrqHandler};
use crate::core::memory::bus::Bus;
use crate::core::memory::{Memory, ReadMemoryAccess, WriteMemoryAccess};
use std::cell::RefCell;
use std::rc::Rc;
use tracing::{debug, info, warn};

pub trait DmaDevice {
    // true if device is ready for DMA transfer
    fn is_dma_ready(&self) -> bool;
    // true if device requests a word now
    fn dma_request(&self) -> bool;
    // RAM -> device
    fn dma_write(&mut self, word: u32,clock:&mut Clock,irq_handler:&mut IrqHandler);
    // device -> RAM
    fn dma_read(&mut self) -> u32;
}

pub struct DummyDMAChannel {}
impl DmaDevice for DummyDMAChannel {
    fn is_dma_ready(&self) -> bool { true }
    fn dma_request(&self) -> bool {
        true
    }
    fn dma_write(&mut self, _word: u32,_clock:&mut Clock,_irq_handler:&mut IrqHandler) {}
    fn dma_read(&mut self) -> u32 {
        0
    }
}

#[derive(Debug)]
enum SyncMode {
    Manual,
    Slice,
    LinkedList
}

impl SyncMode {
    fn from_chcr(chcr:u32) -> Self {
        match (chcr >> 9) & 3 {
            // Transfer starts when the CPU writes to the Trigger bit and transfers everything at once
            0 => SyncMode::Manual,
            // Sync blocks to DMA requests
            1 => SyncMode::Slice,
            // Used to transfer GPU command lists
            2 => SyncMode::LinkedList,
            _ => unreachable!()
        }
    }
}
#[derive(Debug,PartialEq)]
enum TransferDirection {
    DeviceToRAM,
    RAMToDevice,
}

impl TransferDirection {
    fn from_chcr(chcr: u32) -> Self {
        match chcr & 1 {
            0 => TransferDirection::DeviceToRAM,
            1 => TransferDirection::RAMToDevice,
            _ => unreachable!()
        }
    }
}

enum DMAResult {
    InProgress,
    Paused,
    Finished,
    BlockFinished,
}

/*
bcr:
    For SyncMode=0 (ie. for OTC and CDROM):
      0-15  BC    Number of words (0001h..FFFFh) (or 0=10000h words)
      16-31 0     Not used (usually 0 for OTC, or 1 ("one block") for CDROM)
    For SyncMode=1 (ie. for MDEC, SPU, and GPU-vram-data):
      0-15  BS    Blocksize (words) ;for GPU/SPU max 10h, for MDEC max 20h
      16-31 BA    Amount of blocks  ;ie. total length = BS*BA words
    For SyncMode=2 (ie. for GPU-command-lists):
      0-31  0     Not used (should be zero) (transfer ends at END-CODE in list)

    BC/BS/BA can be in range 0001h..FFFFh (or 0=10000h). For BS, take care not to set the blocksize larger than the buffer of the corresponding unit can hold. (GPU and SPU both have a 16-word buffer). A larger blocksize means faster transfer.
    SyncMode=1 decrements BA to zero, SyncMode=0 with chopping enabled decrements BC to zero (aside from that two cases, D#_BCR isn't changed during/after transfer).
chcr:
  0     Transfer direction (0=device to RAM, 1=RAM to device)
  1     MADR increment per step (0=+4, 1=-4)
  2-7   Unused
  8     When 1:
        -Burst mode: enable "chopping" (cycle stealing by CPU)
        -Slice mode: Causes DMA to hang
        -Linked-list mode: Transfer header before data?
  9-10  Transfer mode (SyncMode)
        0=Burst (transfer data all at once after DREQ is first asserted)
        1=Slice (split data into blocks, transfer next block whenever DREQ is asserted)
        2=Linked-list mode
        3=Reserved
  11-15 Unused
  16-18 Chopping DMA window size (1 << N words)
  19    Unused
  20-22 Chopping CPU window size (1 << N cycles)
  23    Unused
  24    Start transfer (0=stopped/completed, 1=start/busy)
  25-27 Unused
  28    Force transfer start without waiting for DREQ
  29    In forced-burst mode, pauses transfer while set.
        In other modes, stops bit 28 from being cleared after a slice is transferred.
        No effect when transfer was caused by a DREQ.
  30    Perform bus snooping (allows DMA to read from -nonexistent- cache?)
  31    Unused

  Bit 28 is automatically cleared upon BEGIN of the transfer, this bit needs to be set only in SyncMode=0 (setting it in other SyncModes would force the first block to be transferred instantly without DREQ, which isn't desired).
  Bit 24 is automatically cleared upon COMPLETION of the transfer, this bit must be always set for all SyncModes when starting a transfer.
  For DMA6/OTC there are some restrictions, D6_CHCR has only three read/write-able bits: 24,28,30. All other bits are read-only: bit 1 is always 1 (increment=-4), and the other bits are always 0.
 */
struct DMAChannel {
    id: usize,
    // 1F801080h+N*10h - D#_MADR - DMA base address (Channel 0..6) (R/W)
    // 0-23  Memory Address where the DMA will start reading from/writing to
    // 24-31 Not used (always zero)
    madr: u32,
    // In SyncMode=0, the hardware doesn't update the MADR registers
    // In SyncMode=1 and SyncMode=2, the hardware does update MADR (it will contain the start address of the currently transferred block; at transfer end, it'll hold the end-address in SyncMode=1, or the end marker in SyncMode=2)
    madr_read: u32,
    // 1F801084h+N*10h - D#_BCR - DMA Block Control (Channel 0..6) (R/W)
    bcr: u32,
    // 1F801088h+N*10h - D#_CHCR - DMA Channel Control (Channel 0..6) (R/W)
    chcr: u32,
    enabled: bool,
    bus_error: bool,
    device: Rc<RefCell<dyn DmaDevice>>,
    sync_mode: SyncMode,
    transfer_direction: TransferDirection,
    remaining_words: u16,
    remaining_blocks: u16,
    waiting_next_block: bool,
    chopping_window_words: usize,
    chopping_window_cycles: usize,
    linked_list_header: Option<(u32,u32)>,
}

impl DMAChannel {
    fn new(id:usize,device: &Rc<RefCell<dyn DmaDevice>>) -> DMAChannel {
        DMAChannel {
            id,
            madr: 0,
            madr_read: 0,
            bcr: 0,
            chcr: 0,
            enabled: false,
            bus_error: false,
            device: device.clone(),
            sync_mode: SyncMode::Slice,
            transfer_direction: TransferDirection::DeviceToRAM,
            remaining_words: 0,
            remaining_blocks: 0,
            waiting_next_block: false,
            chopping_window_words: 0,
            chopping_window_cycles: 0,
            linked_list_header: None,
        }
    }

    fn get_bus_error(&mut self) -> bool {
        let bus_error = self.bus_error;
        self.bus_error = false;
        bus_error
    }

    fn transfer_completed(&mut self) {
        debug!("Transfer of channel #{} completed", self.id);
        // chcr: Bit 24 is automatically cleared upon COMPLETION of the transfer, this bit must be always set for all SyncModes when starting a transfer.
        self.chcr &= !(1 << 24);
        self.linked_list_header = None;
    }

    fn do_dma(&mut self,bus: &mut Bus,irq_handler:&mut IrqHandler) -> DMAResult {
        // chcr: Bit 28 is automatically cleared upon BEGIN of the transfer, this bit needs to be set only in SyncMode=0 (setting it in other SyncModes would force the first block to be transferred instantly without DREQ, which isn't desired).
        if self.chcr & (1 << 28) != 0 {
            self.chcr &= !(1 << 28);
        }
        match self.sync_mode {
            SyncMode::Manual => {
                if self.id == 6 {
                    self.do_dma_channel6_ot(bus)
                }
                else {
                    self.do_dma_manual(bus,irq_handler)
                }
            }
            SyncMode::Slice => self.do_dma_slice(bus,irq_handler),
            SyncMode::LinkedList => self.do_dma_linked_list(bus,irq_handler),
        }
    }

    fn do_dma_channel6_ot(&mut self, bus: &mut Bus) -> DMAResult {
        let chopping = (self.chcr & 0x100) != 0;
        if chopping {
            if self.chopping_window_words == 0 {
                self.chopping_window_cycles -= 1;
                if self.chopping_window_cycles == 0 {
                    self.update_chopping_windows();
                }
                else {
                    return DMAResult::Paused
                }
            }
        }
        if self.remaining_words == 1 {
            bus.write::<32>(self.madr, 0xFF_FFFF);
            debug!("DMA OT last writing: {:08X}",self.madr);
        }
        else {
            let target = self.madr;
            self.madr = target.wrapping_sub(4) & 0xFF_FFFC;
            match bus.write::<32>(target, self.madr) {
                WriteMemoryAccess::Write(_) => debug!("DMA OT writing: {:08X} = {:08X} remaining words={:4X}",target,self.madr,self.remaining_words),
                _ => {
                    warn!("DMA Bus error while accessing address {:08X}",target);
                    self.bus_error = true
                }
            }

        }
        self.remaining_words = self.remaining_words.wrapping_sub(1);
        if self.remaining_words == 0 {
            self.transfer_completed();
            DMAResult::Finished
        }
        else {
            if chopping {
                self.chopping_window_words -= 1;
                if self.chopping_window_words == 0 {
                    DMAResult::Paused
                }
                else {
                    DMAResult::InProgress
                }
            }
            else {
                DMAResult::InProgress
            }
        }
    }
    fn do_dma_manual(&mut self, bus: &mut Bus,irq_handler:&mut IrqHandler) -> DMAResult {
        let chopping = (self.chcr & 0x100) != 0;
        if chopping {
            if self.chopping_window_words == 0 {
                self.chopping_window_cycles -= 1;
                if self.chopping_window_cycles == 0 {
                    self.update_chopping_windows();
                }
                else {
                    return DMAResult::Paused
                }
            }
        }
        if !self.dma_read_write_word(bus,irq_handler) {
            return DMAResult::Paused;
        }
        self.remaining_words = self.remaining_words.wrapping_sub(1);
        if self.remaining_words == 0 {
            self.transfer_completed();
            DMAResult::Finished
        }
        else {
            if chopping {
                self.chopping_window_words -= 1;
                if self.chopping_window_words == 0 {
                    DMAResult::Paused
                }
                else {
                    DMAResult::InProgress
                }
            }
            else {
                DMAResult::InProgress
            }
        }
    }
    fn dma_read_write_word(&mut self, bus: &mut Bus,irq_handler:&mut IrqHandler) -> bool  {
        if !self.device.borrow().is_dma_ready() {
            return false;
        }
        let target = self.madr;
        self.madr = if (self.chcr & 2) == 0 { self.madr.wrapping_add(4) } else { self.madr.wrapping_sub(4) };
        self.madr &= 0xFF_FFFC;
        match self.transfer_direction {
            TransferDirection::DeviceToRAM => {
                let device_read = self.device.borrow_mut().dma_read();
                match bus.write::<32>(target, device_read) {
                    WriteMemoryAccess::Write(_) => {}
                    _ => {
                        warn!("DMA Bus error while accessing address {:08X}",target);
                        self.bus_error = true
                    }
                }
            }
            TransferDirection::RAMToDevice => {
                match bus.read::<32>(target, false) {
                    ReadMemoryAccess::Read(mem_read,_) => self.device.borrow_mut().dma_write(mem_read,bus.get_clock_mut(),irq_handler),
                    _ => {
                        warn!("DMA Bus error while accessing address {:08X}",target);
                        self.bus_error = true
                    }
                }
            }
        }
        true
    }
    fn do_dma_slice(&mut self, bus: &mut Bus,irq_handler:&mut IrqHandler) -> DMAResult {
        if self.waiting_next_block {
            // check if device has another block available
            if self.device.borrow().dma_request() {
                self.waiting_next_block = false;
            }
            else {
                // keep waiting...
                return DMAResult::Paused;
            }
        }
        if !self.dma_read_write_word(bus,irq_handler) {
            return DMAResult::Paused;
        }
        self.remaining_words = self.remaining_words.wrapping_sub(1);
        if self.remaining_words == 0 {
            self.remaining_blocks = self.remaining_blocks.wrapping_sub(1);
            if self.remaining_blocks == 0 {
                self.transfer_completed();
                return DMAResult::Finished
            }
            self.update_remaining_blocks_words(true);
            self.waiting_next_block = true;
            return DMAResult::BlockFinished;
        };

        DMAResult::InProgress
    }
    fn do_dma_linked_list(&mut self, bus: &mut Bus,irq_handler:&mut IrqHandler) -> DMAResult {
        if self.transfer_direction != TransferDirection::RAMToDevice {
            panic!("DMA linked list set transfer to Device->RAM")
        }
        let target = self.madr;
        self.madr = if (self.chcr & 2) == 0 { self.madr.wrapping_add(4) } else { self.madr.wrapping_sub(4) };
        self.madr &= 0xFF_FFFC;

        let word = match bus.read::<32>(target, false) {
            ReadMemoryAccess::Read(mem_read,_) => mem_read,
            _ => {
                self.bus_error = true;
                warn!("DMA Bus error while accessing address {:08X}",target);
                0
            }
        };

        let (next_node_address,extra_words) = match self.linked_list_header {
            None => {
                let next_node_address = word & 0xFFFFFF;
                let words = word >> 24;
                if words > 0 {
                    self.linked_list_header = Some((next_node_address, words));
                }
                else {
                    self.linked_list_header = None;
                    if (next_node_address & 0x800000) != 0 { // TODO check
                        self.madr_read = next_node_address;
                        self.transfer_completed();
                        debug!("Linked List transfer completed");
                        return DMAResult::Finished;
                    }
                    self.madr = next_node_address;
                }
                if words > 0 {
                    debug!("DMA read linked list header: {:08X} [next_addr={:08X} words={}]",word,next_node_address,words);
                }

                return DMAResult::InProgress;
            }
            Some(h) => h,
        };

        // send word to device
        self.device.borrow_mut().dma_write(word,bus.get_clock_mut(),irq_handler);
        let extra_words = extra_words - 1;
        if extra_words == 0 {
            // The transfer is stopped once an end marker is reached. On some (earlier?) CPU revisions any address with bit 23 set will be interpreted as an end marker,
            // while on other revisions all bits must be set (i.e. the address must be FFFFFF)
            if (next_node_address & 0x800000) != 0 { // TODO check
                self.madr_read = next_node_address;
                self.transfer_completed();
                return DMAResult::Finished;
            }
            self.madr = next_node_address;
            self.linked_list_header = None;
        }
        else {
            self.linked_list_header = Some((next_node_address, extra_words));
        }

        DMAResult::InProgress
    }
    /*
    Bit 28 is automatically cleared upon BEGIN of the transfer, this bit needs to be set only in SyncMode=0 (setting it in other SyncModes would force the first block to be transferred instantly without DREQ, which isn't desired).
    Bit 24 is automatically cleared upon COMPLETION of the transfer, this bit must be always set for all SyncModes when starting a transfer.
     */
    fn is_ready(&self) -> bool {
        let active = (self.chcr & (1 << 24)) != 0;
        let trigger = (self.chcr & (1 << 28)) != 0;

        let ready = match self.sync_mode {
            SyncMode::Manual => trigger && active,
            SyncMode::Slice => active,
            SyncMode::LinkedList => active
        };

        self.enabled & ready
    }

    fn read_madr(&self) -> u32 {
        self.madr_read
    }
    fn write_madr(&mut self, value: u32) {
        self.madr = value & 0xFF_FFFC;
        self.madr_read = value;
        debug!("Channel[{}] write memory address: {:08X}",self.id,value);
    }
    // SyncMode=1 decrements BA to zero, SyncMode=0 with chopping enabled decrements BC to zero
    // (aside from that two cases, D#_BCR isn't changed during/after transfer).
    fn read_bcr(&self) -> u32 {
        match self.sync_mode {
            SyncMode::Manual => {
                if (self.chcr & 0x100) != 0 {
                    self.remaining_words as u32
                }
                else {
                    self.bcr
                }
            }
            SyncMode::Slice => {
                self.bcr & 0xFFFF | (self.remaining_blocks as u32) << 16
            }
            SyncMode::LinkedList => self.bcr,
        }
    }
    fn write_bcr(&mut self,value:u32) {
        self.bcr = value;
        self.update_remaining_blocks_words(false);
        debug!("Channel[{}] write block control register: {:08X}",self.id,value);
    }
    fn read_chcr(&self) -> u32 {
        self.chcr
    }
    fn write_chcr(&mut self, value:u32) {
        self.chcr = value;
        // For DMA6/OTC there are some restrictions, D6_CHCR has only three read/write-able bits: 24,28,30.
        // All other bits are read-only: bit 1 is always 1 (increment=-4), and the other bits are always 0.
        if self.id == 6 {
            self.chcr &= (1 << 24) | (1 << 28) | (1 << 30);
            self.chcr |= 2
        }

        self.sync_mode = SyncMode::from_chcr(self.chcr);
        self.transfer_direction = TransferDirection::from_chcr(self.chcr);

        self.update_remaining_blocks_words(false);
        self.update_chopping_windows();
        self.waiting_next_block = false;
        self.linked_list_header = None;
        debug!("Channel[{}] write control register: {:08X} direction={:?} syncMode={:?} active={} trigger={} remaining_words={:04X} remaining_blocks={:04X} madr={:08X}",self.id,value,self.transfer_direction,self.sync_mode,(value & (1 << 24)) != 0,(value & (1 << 28)) != 0,self.remaining_words,self.remaining_blocks,self.madr);
    }

    fn update_chopping_windows(&mut self) {
        let chopping_words = (self.chcr >> 16) & 7; // 16-18 Chopping DMA window size (1 << N words)
        let chopping_cycles = (self.chcr >> 20) & 7; // 20-22 Chopping CPU window size (1 << N cycles)
        self.chopping_window_words = 1 << chopping_words;
        self.chopping_window_cycles = 1 << chopping_cycles;
        debug!("DMA chopping words={} cycles={}",self.chopping_window_words,self.chopping_window_cycles);
    }

    fn update_remaining_blocks_words(&mut self,only_words:bool) {
        match self.sync_mode {
            SyncMode::Manual => {
                self.remaining_words = (self.bcr & 0xFFFF) as u16;
                self.remaining_blocks = 0;
            }
            SyncMode::Slice => {
                self.remaining_words = (self.bcr & 0xFFFF) as u16;
                if !only_words {
                    self.remaining_blocks = (self.bcr >> 16) as u16;
                }
            }
            SyncMode::LinkedList => {
                self.remaining_words = 0;
                self.remaining_blocks = 0;
            }
        }
    }
}
/*
1F8010F0h - DPCR - DMA Control Register (R/W)
  0-2   DMA0, MDECin  Priority      (0..7; 0=Highest, 7=Lowest)
  3     DMA0, MDECin  Master Enable (0=Disable, 1=Enable)
  4-6   DMA1, MDECout Priority      (0..7; 0=Highest, 7=Lowest)
  7     DMA1, MDECout Master Enable (0=Disable, 1=Enable)
  8-10  DMA2, GPU     Priority      (0..7; 0=Highest, 7=Lowest)
  11    DMA2, GPU     Master Enable (0=Disable, 1=Enable)
  12-14 DMA3, CDROM   Priority      (0..7; 0=Highest, 7=Lowest)
  15    DMA3, CDROM   Master Enable (0=Disable, 1=Enable)
  16-18 DMA4, SPU     Priority      (0..7; 0=Highest, 7=Lowest)
  19    DMA4, SPU     Master Enable (0=Disable, 1=Enable)
  20-22 DMA5, PIO     Priority      (0..7; 0=Highest, 7=Lowest)
  23    DMA5, PIO     Master Enable (0=Disable, 1=Enable)
  24-26 DMA6, OTC     Priority      (0..7; 0=Highest, 7=Lowest)
  27    DMA6, OTC     Master Enable (0=Disable, 1=Enable)
  28-30 CPU memory access priority  (0..7; 0=Highest, 7=Lowest)
  31    No effect, should be CPU memory access enable (R/W)

1F8010F4h - DICR - DMA Interrupt Register (R/W)
  0-6   Controls channel 0-6 completion interrupts in bits 24-30.
        When 0, an interrupt only occurs when the entire transfer completes.
        When 1, interrupts can occur for every slice and linked-list transfer.
        No effect if the interrupt is masked by bits 16-22.
  7-14  Unused
  15    Bus error flag. Raised when transferring to/from an address outside of RAM. Forces bit 31. (R/W)
  16-22 Channel 0-6 interrupt mask. If enabled, channels cause interrupts as per bits 0-6.
  23    Master channel interrupt enable.
  24-30 Channel 0-6 interrupt flags. (R, write 1 to reset)
  31    Master interrupt flag (R)
 */
pub struct DMAController {
    channels: [DMAChannel; 7],
    dpcr: u32,
    dpcr_changed: bool,
    dcir: u32,
    priorities: [(usize, usize); 8], // id,priority
    irq_flags: u8,
    reg_f8: u32,
    reg_fc: u32,
    dma_in_progress_on_channel: Option<usize>,
    dma_enabled: bool,
}

impl DMAController {
    pub fn new(devices:&[Rc<RefCell<dyn DmaDevice>>;7]) -> Self {
        Self {
            channels: std::array::from_fn(|i| DMAChannel::new(i,&devices[i].clone())),
            dpcr: 0x07654321,
            dpcr_changed: false,
            dcir: 0,
            priorities: std::array::from_fn(|i| (7 - i, 7 - i)),
            irq_flags: 0,
            reg_f8: 0,
            reg_fc: 0,
            dma_in_progress_on_channel: None,
            dma_enabled: false,
        }
    }

    pub fn read_dpcr(&self) -> u32 {
        self.dpcr
    }

    pub fn write_dpcr(&mut self,value:u32) {
        debug!("DMA write dpcr {:08X}",value);
        self.dpcr = value;
        self.dma_enabled = false; 
        let mut value = value;
        for ch in 0..8 {
            let pr = (value & 7) as usize;
            value >>= 3;
            let enabled = (value & 1) != 0;
            self.dma_enabled |= enabled;
            value >>= 1;
            if ch < 7 {
                self.channels[ch].enabled = enabled;
                debug!("DMA channel #{} enabled={} priority={}",ch,enabled,pr);
            }
            else {
                debug!("DMA CPU priority={}",pr);
            }
            self.priorities[ch] = (ch,pr);
        }
        self.priorities.sort_by_key(|&(ch, prio)| std::cmp::Reverse((prio << 3) | ch));
        self.dpcr_changed = true;
        debug!("DMA channels priorities: {:?}",self.priorities)
    }

    pub fn read_dicr(&self) -> u32 {
        let mut dcir = self.dcir & !(0x7F << 24); // clear 24-30 bits
        dcir |= (self.irq_flags as u32) << 16;
        // Bit 31 is a simple readonly flag that follows the following rules:
        //   IF b15=1 OR (b23=1 AND (b16-22 AND b24-30)>0) THEN b31=1 ELSE b31=0
        let irq_mask = ((dcir >> 16) & 0x7F) as u8;
        let b31_cond = (dcir & (1 << 15) != 0) || ((dcir & (1 << 23) != 0) && (irq_mask & self.irq_flags) != 0);
        dcir | (b31_cond as u32) << 31
    }

    pub fn write_dicr(&mut self,value:u32) {
        self.dcir = value & 0x7FFFFFFF; // 31    Master interrupt flag (R)
        self.irq_flags &= !((value >> 24) & 0x7F) as u8; // 24-30 Channel 0-6 interrupt flags. (R, write 1 to reset)
        debug!("DMA write interrupt register {:08X} irq_flags={:02X}",value,self.irq_flags);
    }

    pub fn read_madr(&self,channel:usize) -> u32 {
        self.channels[channel].read_madr()
    }
    pub fn write_madr(&mut self,channel:usize,value:u32) {
        self.channels[channel].write_madr(value);
    }
    pub fn read_bcr(&self,channel:usize) -> u32 {
        self.channels[channel].read_bcr()
    }
    pub fn write_bcr(&mut self,channel:usize,value:u32) {
        self.channels[channel].write_bcr(value);
    }
    pub fn read_chcr(&self,channel:usize) -> u32 {
        self.channels[channel].read_chcr()
    }
    pub fn write_chcr(&mut self,channel:usize,value:u32) {
        self.channels[channel].write_chcr(value);
    }
    pub fn read_reg_f8(&self) -> u32 {
        self.reg_f8
    }
    pub fn write_reg_f8(&mut self,value:u32) {
        self.reg_f8 = value;
    }
    pub fn read_reg_fc(&self) -> u32 {
        self.reg_fc
    }
    pub fn write_reg_fc(&mut self,value:u32) {
        self.reg_fc = value;
    }
    
    pub fn do_dma_for_cpu_cycles(&mut self,cpu_cycles:usize,bus:&mut Bus,irq_handler:&mut IrqHandler) -> bool {
        let mut dma_in_progress = false;
        for _ in 0..cpu_cycles {
            dma_in_progress = self.do_dma(bus,irq_handler);
        }

        dma_in_progress
    }
    #[inline]
    fn do_dma(&mut self,bus:&mut Bus,irq_handler:&mut IrqHandler) -> bool {
        let channel_in_progress = match self.dma_in_progress_on_channel {
            Some(channel_in_progress) if !self.dpcr_changed => channel_in_progress,
            _ => {
                self.dpcr_changed = false;
                if self.dma_enabled {
                    // check if some channel is ready to start DMA according to priorities
                    let mut channel_found: Option<usize> = None;
                    for &(channel, _) in &self.priorities {
                        if channel == 7 {
                            // CPU has priority, no DMA
                            //return false;
                            continue; // TODO
                        }
                        if self.channels[channel].is_ready() {
                            channel_found = Some(channel);
                            debug!("DMA found channel to activate: #{channel}");
                            break;
                        }
                    }
                    if let Some(channel) = channel_found {
                        self.dma_in_progress_on_channel = Some(channel);
                        channel
                    } else {
                        // can never happen
                        return false; // no DMA in progress
                    }
                }
                else {
                    return false; // no DMA in progress
                }
            }
        };

        // do_dma
        let dma_in_progress = match self.channels[channel_in_progress].do_dma(bus,irq_handler) {
            DMAResult::InProgress => true,
            DMAResult::Paused => false,
            DMAResult::Finished => {
                // TODO IRQ
                self.dma_in_progress_on_channel = None;
                false
            }
            DMAResult::BlockFinished => {
                // TODO IRQ
                false
            }
        };

        // TODO check bus error

        dma_in_progress
    }
}