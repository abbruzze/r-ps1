pub mod disc;
pub mod util;
mod cue;
mod commands;
mod read_sector;
mod xaadpcm;

use std::collections::VecDeque;
use std::ops::RangeInclusive;
use tracing::{info, warn};
use crate::core::cdrom::disc::{AudioLeftRight, Disc, DiscTime, TrackSectorDataSize};
use crate::core::cdrom::xaadpcm::XaAdpcmState;
use crate::core::clock::Clock;
use crate::core::dma::DmaDevice;
use crate::core::interrupt::{InterruptType, IrqHandler};

/*
19h,20h --> INT3(yy,mm,dd,ver)
Indicates the date (Year-month-day, in BCD format) and version of the HC05 CDROM controller BIOS. Known/existing values are:
    (unknown)        ;DTL-H2000 (with SPC700 instead HC05)
      94h,09h,19h,C0h  ;PSX (PU-7)               19 Sep 1994, version vC0 (a)
      94h,11h,18h,C0h  ;PSX (PU-7)               18 Nov 1994, version vC0 (b)
      94h,11h,28h,01h  ;PSX (DTL-H2000)          28 Nov 1994, version v01 (debug)
      95h,05h,16h,C1h  ;PSX (LATE-PU-8)          16 May 1995, version vC1 (a)
      95h,07h,24h,C1h  ;PSX (LATE-PU-8)          24 Jul 1995, version vC1 (b)
      95h,07h,24h,D1h  ;PSX (LATE-PU-8,debug ver)24 Jul 1995, version vD1 (debug)
      96h,08h,15h,C2h  ;PSX (PU-16, Video CD)    15 Aug 1996, version vC2 (VCD)
      96h,08h,18h,C1h  ;PSX (LATE-PU-8,yaroze)   18 Aug 1996, version vC1 (yaroze)
      96h,09h,12h,C2h  ;PSX (PU-18) (japan)      12 Sep 1996, version vC2 (a.jap)
      97h,01h,10h,C2h  ;PSX (PU-18) (us/eur)     10 Jan 1997, version vC2 (a)
      97h,08h,14h,C2h  ;PSX (PU-20)              14 Aug 1997, version vC2 (b)
      98h,06h,10h,C3h  ;PSX (PU-22)              10 Jun 1998, version vC3 (a)
      99h,02h,01h,C3h  ;PSX/PSone (PU-23, PM-41) 01 Feb 1999, version vC3 (b)
      A1h,03h,06h,C3h  ;PSone/late (PM-41(2))    06 Jun 2001, version vC3 (c)
      (unknown)        ;PS2,   xx xxx xxxx, late PS2 models...?
 */
const CDROM_VER : [u8;4] = [0x95, 0x07, 0x24, 0xC1]; // 95h,07h,24h,C1h  ;PSX (LATE-PU-8)          24 Jul 1995, version vC1 (b)

const PARAMETER_FIFO_LEN : usize = 16;

#[derive(Debug,Clone,Copy)]
enum Speed {
    Normal = 0x00,
    DoubleSpeed = 0x01,
}

impl Speed {
    #[inline(always)]
    fn get_read_sector_ms(&self) -> f32 {
        match self {
            Speed::Normal => 1000.0 / 75.0,
            Speed::DoubleSpeed => 1000.0 / 150.0,
        }
    }
}

#[derive(Debug,Copy,Clone,PartialEq)]
pub enum Region {
    Japan,
    USA,
    Europe,
}

impl Region {
    pub fn to_scee_letter(&self) -> char {
        match self {
            Region::Japan => 'I',
            Region::USA => 'A',
            Region::Europe => 'E',
        }
    }
    pub fn get_crt_total_lines(&self) -> usize {
        match self {
            Region::Japan | Region::USA => 263,
            Region::Europe => 312,
        }
    }
}
#[derive(Debug,Copy,Clone)]
enum Command {
    Nop,
    Setloc,
    Play,
    Read,
    MotorOn,
    Stop,
    Pause,
    Init,
    Mute,
    Demute,
    SetMode,
    SetFilter,
    GetLocL,
    GetLocP,
    GetTN,
    GetTD,
    SeekL,
    SeekP,
    Test,
    GetID,
    ReadTOC,
}

impl Command {
    fn from_u8(value:u8) -> Option<Self> {
        match value {
            0x01 => Some(Command::Nop),
            0x02 => Some(Command::Setloc),
            0x03 => Some(Command::Play),
            0x06|0x1B => Some(Command::Read),
            0x07 => Some(Command::MotorOn),
            0x08 => Some(Command::Stop),
            0x09 => Some(Command::Pause),
            0x0A => Some(Command::Init),
            0x0B => Some(Command::Mute),
            0x0C => Some(Command::Demute),
            0x0D => Some(Command::SetFilter),
            0x0E => Some(Command::SetMode),
            0x10 => Some(Command::GetLocL),
            0x11 => Some(Command::GetLocP),
            0x13 => Some(Command::GetTN),
            0x14 => Some(Command::GetTD),
            0x15 => Some(Command::SeekL),
            0x16 => Some(Command::SeekP),
            0x19 => Some(Command::Test),
            0x1A => Some(Command::GetID),
            0x1E => Some(Command::ReadTOC),
            _ => None
        }
    }

    fn parameters_number(&self) -> RangeInclusive<usize> {
        match self {
            Command::Nop => 0..=0,
            Command::Setloc => 3..=3,
            Command::Play => 0..=1,
            Command::SetFilter => 2..=2,
            Command::SetMode => 1..=1,
            Command::GetTD => 1..=1,
            Command::Test => 1..=1,
            _ => 0..=0,
        }
    }
}

#[derive(Debug,Clone)]
enum CommandState {
    Idle,
    WaitingIrqAck(u8),
    ToProcess(u8),
    Response { cmd: Command, irq: u8, delay_cycles: usize, response: Vec<u8>, next_state: Box<CommandState> },
    Delay { cmd: Command, delay_cycles: usize, next_state: Box<CommandState> },
    Response2 { cmd: Command },
}
#[derive(Debug,Clone)]
enum DriveState {
    Idle,
    Playing { sample_index:isize, report_counter:usize, report_absolute: bool },
    Seeking,
    Reading { next_sector_cycles: usize },
}

impl DriveState {
    fn to_u8(&self) -> u8 {
        match self {
            DriveState::Playing{ .. } => 0x80,
            DriveState::Seeking => 0x40,
            DriveState::Reading{ .. } => 0x20,
            _ => 0x00,
        }
    }
}

#[derive(Debug, Clone)]
pub enum CDOperation {
    Reading(DiscTime),
    Playing(DiscTime),
    Idle,
}

pub struct CDRom {
    drive_state: DriveState,
    disc: Option<Disc>,
    bank_address: usize,
    hintmsk_reg: u8,
    hintsts_reg: u8,
    hchpctl: u8,
    parameter_fifo: VecDeque<u8>,
    result_fifo: VecDeque<u8>,
    data_buffer: VecDeque<u8>,
    last_sector: Vec<u8>,
    last_audio_sector: Vec<AudioLeftRight>,
    cd_to_spu_volume: [[u8; 2]; 2],
    pending_cd_to_spu_volume: [[u8; 2]; 2],
    audio_mute: bool,
    audio_sample: AudioLeftRight,
    command_state: CommandState,
    shell_once_opened: bool,
    motor_on: bool,
    pending_setloc: Option<DiscTime>,
    mode: u8,
    adpcm: XaAdpcmState,
}

impl DmaDevice for CDRom {
    fn is_dma_ready(&self) -> bool {
        !self.data_buffer.is_empty()
    }
    fn dma_request(&self) -> bool {
        true
    }
    fn dma_write(&mut self, _word: u32, _clock: &mut Clock, _irq_handler: &mut IrqHandler) {
        todo!()
    }
    fn dma_read(&mut self) -> u32 {
        //info!("CDROM dma read");
        self.read_2::<32>()
    }
    fn dma_cycles_per_word(&self) -> usize {
        1
    }
}

impl CDRom {
    pub fn new() -> Self {
        Self {
            drive_state: DriveState::Idle,
            disc: None,
            bank_address: 0,
            hintmsk_reg: 0,
            hintsts_reg: 0,
            hchpctl: 0,
            parameter_fifo: Default::default(),
            result_fifo: Default::default(),
            data_buffer: Default::default(),
            last_sector: Vec::with_capacity(disc::SECTOR_SIZE as usize),
            last_audio_sector: Vec::with_capacity(disc::SECTOR_SIZE as usize),
            cd_to_spu_volume: [[0; 2]; 2],
            pending_cd_to_spu_volume: [[0; 2]; 2],
            audio_mute: false,
            audio_sample: AudioLeftRight(0,0),
            command_state: CommandState::Idle,
            shell_once_opened: false,
            motor_on: false,
            pending_setloc: None,
            mode: 0,
            adpcm: XaAdpcmState::new(),
        }
    }

    pub fn insert_disk(&mut self,disc:Disc) {
        self.disc = Some(disc);
        info!("CDROM inserted disk '{}'",self.disc.as_ref().unwrap().get_cue_file_name());
    }

    pub fn clock_44100hz(&mut self,irq_handler: &mut IrqHandler) -> CDOperation {
        self.audio_sample = AudioLeftRight(0,0);
        self.check_drive_state(irq_handler);
        self.check_command_state(irq_handler);
        
        match self.drive_state {
            DriveState::Reading { .. } => {
                let time = if let Some(disc) = &self.disc {
                    disc.get_head_position()
                }
                else {
                    DiscTime::ZERO_TIME
                };
                CDOperation::Reading(time)
            }
            DriveState::Playing { .. } => {
                let time = if let Some(disc) = &self.disc {
                    disc.get_head_position()
                }
                else {
                    DiscTime::ZERO_TIME
                };
                CDOperation::Playing(time)
            }
            _ => {
                CDOperation::Idle
            }
        } 
    }

    pub fn spu_volume_matrix(&self) -> [[u8; 2]; 2] {
        self.cd_to_spu_volume
    }

    pub fn get_audio_sample(&self) -> (i16,i16) {
        if self.audio_mute { (0,0) } else { (self.audio_sample.0,self.audio_sample.1) }
    }

    #[inline]
    fn get_speed(&self) -> Speed {
        if (self.mode & 0x80) != 0 {
            Speed::DoubleSpeed
        }
        else {
            Speed::Normal
        }
    }

    #[inline]
    fn get_sector_size(&self) -> TrackSectorDataSize {
        if (self.mode & 0x20) != 0 {
            TrackSectorDataSize::WholeSectorExceptSyncBytes
        }
        else {
            TrackSectorDataSize::DataOnly
        }
    }

    /*
       Status code (stat)
       The 8bit status code is returned by Nop command (and many other commands), the meaning of the separate stat bits is:

         7  Play          Playing CD-DA         ;\only ONE of these bits can be set
         6  Seek          Seeking               ; at a time (ie. Read/Play won't get
         5  Read          Reading data sectors  ;/set until after Seek completion)
         4  ShellOpen     Once shell open (0=Closed, 1=Is/was Open)
         3  IdError       (0=Okay, 1=GetID denied) (also set when Setmode.Bit4=1)
         2  SeekError     (0=Okay, 1=Seek error)     (followed by Error Byte)
         1  Spindle Motor (0=Motor off, or in spin-up phase, 1=Motor on)
         0  Error         Invalid Command/parameters (followed by Error Byte)
       If the shell is closed, then bit4 is automatically reset to zero after reading stat with the Nop command
       (most or all other commands do not reset that bit after reading). If stat bit0 or bit2 is set, then the normal respons(es) and interrupt(s) are not send,
       and, instead, INT5 occurs, and an error-byte is send as second response byte.
    */
    fn get_stat(&self,id_error:bool,seek_error:bool,error:bool) -> u8 {
        let mut stat = 0u8;
        stat |= self.drive_state.to_u8();
        if self.shell_once_opened || self.is_shell_opened() {
            stat |= 1 << 4;
        }
        if id_error {
            stat |= 1 << 3;
        }
        if seek_error {
            stat |= 1 << 2;
        }
        if self.motor_on {
            stat |= 1 << 1;
        }
        if error {
            stat |= 1 << 0;
        }
        stat
    }

    #[inline(always)]
    fn is_shell_opened(&self) -> bool {
        // TODO
        false
    }
    #[inline(always)]
    fn is_disk_inserted(&self) -> bool {
        self.disc.is_some()
    }

    fn activate_motor(&mut self,enabled:bool) {
        self.motor_on = true;
        info!("CDROM motor activated: {enabled}");
        // TODO
    }

    /*
    0x1f801800 (read, all banks): HSTS
    0x1f801800 (write, all banks): ADDRESS
      0-1 RA       Current register bank (R/W)
      2   ADPBUSY  ADPCM busy            (R, 1=playing XA-ADPCM)
      3   PRMEMPT  Parameter empty       (R, 1=parameter FIFO empty)
      4   PRMWRDY  Parameter write ready (R, 1=parameter FIFO not full)
      5   RSLRRDY  Result read ready     (R, 1=result FIFO not empty)
      6   DRQSTS   Data request          (R, 1=one or more RDDATA reads or WRDATA writes pending)
      7   BUSYSTS  Busy status           (R, 1=HC05 busy acknowledging command)
    Writing a value to the low 2 bits of this address changes the bank to said value. Likewise, the low 2 bits of this address can be read to get the current bank.
     */
    pub fn read_0(&self) -> u8 {
        let mut hsts = self.bank_address as u8;
        // TODO bit 2
        if self.parameter_fifo.is_empty() {
            hsts |= 1 << 3;
        }
        if self.parameter_fifo.len() < PARAMETER_FIFO_LEN {
            hsts |= 1 << 4;
        }
        if !self.result_fifo.is_empty() {
            hsts |= 1 << 5;
        }
        if !self.data_buffer.is_empty() {
            hsts |= 1 << 6;
        }
        let busy_status = matches!(self.command_state,CommandState::WaitingIrqAck(_) | CommandState::ToProcess(_));
        if busy_status {
            hsts |= 1 << 7;
        }
        //info!("CDROM reading status: {:02X}",hsts);
        hsts
    }
    /*
    0x1f801801 (read, all banks): RESULT
      0-7  Response Byte(s) received after sending a Command
    The result FIFO can hold up to 16 bytes (most or all responses are less than 16 bytes).
    The decoder clears RSLRRDY after the last byte of the HC05's response is read from this register.
    When reading further bytes: The buffer is padded with 00h's to the end of the 16-bytes, and does then restart at the first response byte
    (that, without receiving a new response, so it'll always return the same 16 bytes, until a new command/response has been sent/received).
     */
    pub fn read_1(&mut self) -> u8 {
         self.result_fifo.pop_front().unwrap_or(0)
    }
    pub fn peek_1(&self) -> u8 {
        match self.result_fifo.front() {
            Some(v) => *v,
            None => 0
        }
    }
    /*
    0x1f801802 (read, all banks): RDDATA
    After ReadS/ReadN commands have generated INT1, software must set the BFRD flag, then wait until DRQSTS is set,
    the datablock (disk sector) can be then read from this register.

      0-7  Data 8bit  (one byte), or alternately,
      0-15 Data 16bit (LSB=First byte, MSB=Second byte)
    The PSX hardware allows to read 800h-byte or 924h-byte sectors, indexed as [000h..7FFh] or [000h..923h], when trying to read further bytes,
    then the PSX will repeat the byte at index [800h-8] or [924h-4] as padding value.
    RDDATA can be accessed with 8bit or 16bit reads (ie. to read a 2048-byte sector, one can use 2048 load-byte opcodes, or 1024 load halfword opcodes,
    or, more conventionally, a 512 word DMA transfer; the actual CDROM databus is only 8bits wide, so the CPU's bus interface handles splitting the reads).
     */
    pub fn read_2<const SIZE : usize>(&mut self) -> u32 {
        const { assert!(SIZE == 8 || SIZE == 16 || SIZE == 32) }

        if (self.hchpctl & 0x80) == 0 {
            warn!("CDROM read_2 called with HCHPCTL.Bit7=0");
        }
        let read = match SIZE {
            8 => self.data_buffer.pop_front().unwrap_or(0) as u32,
            16 => self.data_buffer.pop_front().unwrap_or(0) as u32 | (self.data_buffer.pop_front().unwrap_or(0) as u32) << 8,
            32 => u32::from_le_bytes([
                self.data_buffer.pop_front().unwrap_or(0),
                self.data_buffer.pop_front().unwrap_or(0),
                self.data_buffer.pop_front().unwrap_or(0),
                self.data_buffer.pop_front().unwrap_or(0)
            ]),
            _ => unreachable!()
        };

        //info!("CDROM read_2 read {:08X} from buffer [remain bytes={}]",read,self.data_buffer.len());

        read
    }
    pub fn peek_2(&self) -> u8 {
        0
    }
    /*
    0x1f801803 (read, banks 1 and 3): HINTSTS
      0-2 INTSTS Interrupt "flags" from HC05
      3   BFEMPT Sound map XA-ADPCM buffer empty       (1=decoder ran out of sectors to play)
      4   BFWRDY Sound map XA-ADPCM buffer write ready (1=decoder is ready for next sector)
      5-7 -      Reserved                              (always 1)
    Bits 0-2 are supposed to be used as three separate IRQ flags, however the HC05 misuses them as a single 3-bit "interrupt type" value, which always assumes one of the following values:
      INT0 NoIntr      No interrupt pending
      INT1 DataReady   New sector (ReadN/ReadS) or report packet (Play) available
      INT2 Complete    Command finished processing (some commands, after INT3 is fired)
      INT3 Acknowledge Command received and acknowledged (all commands)
      INT4 DataEnd     Reached end of disc (or end of track if auto-pause enabled)
      INT5 DiskError   Command error, read error, license string error or lid opened
      INT6 -
      INT7 -
    The response interrupts are queued, for example, if the 1st response is INT3, and the second INT5, then INT3 is delivered first,
    and INT5 is not delivered until INT3 is acknowledged (ie. the response interrupts are NOT ORed together to produce INT7 or so).
    BFEMPT and BFWRDY however can be ORed with the lower bits (i.e. BFWRDY + INT3 would give 13h).
    All interrupts are always fired in response to a command with the exception of INT5, which may also be triggered at any time by opening the lid.
     */
    pub fn read_3(&self) -> u8 {
        let value = match self.bank_address {
            0|2 => self.hintmsk_reg | 0xE0,
            1|3 => self.hintsts_reg | 0xE0,
            _ => unreachable!()
        };

        //info!("CDROM read 3({}) = {:02X}",self.bank_address,value);
        value
    }

    pub fn write_0(&mut self, value: u8) {
        self.bank_address = (value & 3) as usize;
        //info!("CDROM selected bank is {}",self.bank_address);
    }
    pub fn write_1(&mut self,value:u8) {
        match self.bank_address {
            0 => {
                // handling command
                if !matches!(self.command_state, CommandState::Idle) {
                    warn!("CDROM requesting new command while command state is not idle. Command state: {:?}, drive state: {:?}",self.command_state,self.drive_state);
                }
                self.command_state = if self.is_irq_pending() {
                    CommandState::WaitingIrqAck(value)
                }
                else {
                    CommandState::ToProcess(value)
                };
                info!("CDROM pending command: {:02X} while drive state={:?}",value,self.drive_state);
            },
            1 => self.write_data(value),
            2 => self.write_ci(value),
            3 => self.write_atv2(value),
            _ => unreachable!()
        }
    }
    pub fn write_2(&mut self,value:u8) {
        match self.bank_address {
            0 => self.write_parameter(value),
            1 => self.write_hintmsk(value),
            2 => self.write_atv0(value),
            3 => self.write_atv3(value),
            _ => unreachable!()
        }
    }
    pub fn write_3(&mut self,value:u8) {
        match self.bank_address {
            0 => self.write_hchpctl(value),
            1 => self.write_hclrctl(value),
            2 => self.write_atv1(value),
            3 => self.write_adpctl(value),
            _ => unreachable!()
        }
    }

    /*
    0x1f801802 (write, bank 1): HINTMSK
      0-2 ENINT    Enable IRQ on respective INTSTS bits
      3   ENBFEMPT Enable IRQ on BFEMPT
      4   ENBFWRDY Enable IRQ on BFWRDY
      5-7 -        Reserved (should be 0 when written, always 1 when read)
    The CD-ROM drive fires an interrupt whenever (HINTMSK & HINTSTS) is non-zero.
    This register is typically set to 1Fh, allowing any of the flags to trigger an IRQ (even though BFEMPT and BFWRDY are never used).
     */
    fn write_hintmsk(&mut self, value: u8) {
        //info!("CDROM set irq mask {:02X}",value);
        self.hintmsk_reg = value;
    }
    fn write_atv0(&mut self, value: u8) {
        info!("L CD to L SPU volume: {value:02X}");
        self.pending_cd_to_spu_volume[0][0] = value;
    }
    fn write_atv1(&mut self,value:u8) {
        info!("L CD to R SPU volume: {value:02X}");
        self.pending_cd_to_spu_volume[1][0] = value;
    }
    fn write_atv2(&mut self, value: u8) {
        info!("R CD to R SPU volume: {value:02X}");
        self.pending_cd_to_spu_volume[1][1] = value;
    }
    fn write_atv3(&mut self, value: u8) {
        info!("R CD to L SPU volume: {value:02X}");
        self.pending_cd_to_spu_volume[0][1] = value;
    }

    fn write_data(&mut self, value: u8) {
        info!("CDROM write data");
    }
    fn write_ci(&mut self, value: u8) {
        info!("CDROM write ci");
    }

    #[inline]
    fn ack_irqs(&mut self,ints:u8) {
        // TODO check all irq flags (0x1F)
        self.hintsts_reg = (self.hintsts_reg & !7) | (self.hintsts_reg & 7 & !ints);
    }

    /*
    The PSX can deliver one INT after another. Instead of using a real queue, it's merely using some flags that do indicate which INT(s) need to be delivered.
    Basically, there seem to be two flags: One for Second Response (INT2), and one for Data/Report Response (INT1).
    There is no flag for First Response (INT3); because that INT is generated immediately after executing a command.
    The flag mechanism means that the SUB-CPU cannot hold more than one undelivered INT1.
     */
    #[inline]
    fn set_irq(&mut self,irq:u8) {
        // TODO check all irq flags (0x1F)
        self.hintsts_reg = (self.hintsts_reg & !7) | irq;
    }

    #[inline]
    fn check_irq(&mut self,irq_handler:&mut IrqHandler) {
        if (self.hintmsk_reg & self.hintsts_reg) != 0 {
            irq_handler.set_irq(InterruptType::CDROM)
        }
    }
    #[inline]
    fn is_irq_pending(&self) -> bool {
        (self.hintsts_reg & 7) != 0
    }

    /*
    0x1f801803 (write, bank 0): HCHPCTL
      0-4 -    Reserved                                    (should be 0)
      5   SMEN Sound map (manual XA-ADPCM playback) enable
      6   BFWR Request sector buffer write                 (1=prepare for writes to WRDATA)
      7   BFRD Request sector buffer read                  (1=prepare for reads from RDDATA)
     */
    fn write_hchpctl(&mut self, value: u8) {
        self.hchpctl = value;
    }

    /*
    0x1f801803 (write, bank 1): HCLRCTL
      0-2 CLRINT     Acknowledge HC05 interrupt "flags" (0=no change, 1=clear)
      3   CLRBFEMPT  Acknowledge BFEMPT                 (0=no change, 1=clear)
      4   CLRBFWRDY  Acknowledge BFBFWRDY               (0=no change, 1=clear)
      5   SMADPCLR   Clear sound map XA-ADPCM buffer    (0=no change, 1=clear/stop playback)
      6   CLRPRM     Clear parameter FIFO               (0=no change, 1=clear)
      7   CHPRST     Reset decoder chip                 (0=no change, 1=reset)
    Setting bits 0-4 resets the corresponding flags in HINTSTS; normally one should write 07h to reset the HC05 interrupt flags, or 1Fh to acknowledge all IRQs. Acknowledging individual HC05 flags (e.g. writing 01h to change INT3 to INT2) is possible, if completely useless. After acknowledge, the result FIFO is drained and if there's been a pending command, then that command gets send to the controller.
    Setting CHPRST will result in a complete reset of the decoder. Unclear if this also reboots the HC05 and CD-ROM DSP (the decoder has an "external reset" pin which is pulled low when setting CHPRST).
     */
    fn write_hclrctl(&mut self, value: u8) {
        //info!("CDROM write hclrctl {:02X}",value);
        self.ack_irqs(value);
        if (value & 0x40) != 0 {
            //info!("Resetting parameter FIFO..");
            self.parameter_fifo.clear();
        }
    }

    /*
    0x1f801803 (write, bank 3): ADPCTL
      0    ADPMUTE Mute XA-ADPCM           (1=mute)
      1-4  -       Reserved                (should be 0)
      5    CHNGATV Apply ATV0-ATV3 changes (0=no change, 1=apply)
      6-7  -       Reserved                (should be 0)
     */
    fn write_adpctl(&mut self,value:u8) {
        info!("CDROM write adpctl {value:02X}");
        self.adpcm.muted = (value & 0x01) != 0;
        if (value & 0x20) != 0 {
            self.cd_to_spu_volume = self.pending_cd_to_spu_volume;
        }
    }

    /*
   0x1f801802 (write, bank 0): PARAMETER
     0-7  Parameter Byte(s) to be used for next Command
   Before sending a command, write any parameter byte(s) to this address. The FIFO can hold up to 16 bytes; once full, the decoder will clear the PRMWRDY flag.
    */
    fn write_parameter(&mut self,value:u8) {
        //info!("CDROM write cmd parameter {:02X}",value);
        if self.parameter_fifo.len() < PARAMETER_FIFO_LEN {
            self.parameter_fifo.push_back(value);
        } else {
            warn!("CDROM parameter FIFO is full, ignoring write");
        }
    }
}