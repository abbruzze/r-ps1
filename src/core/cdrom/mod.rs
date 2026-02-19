pub mod disc;
pub mod util;

use std::collections::VecDeque;
use std::process::exit;
use tracing::{debug, info, warn};
use crate::core::cdrom::disc::{Disc, DiscTime, TrackSectorDataSize, TrackType, BCD};
use crate::core::clock::{CDROMEventType, Clock, EventType};
use crate::core::dma::DmaDevice;
use crate::core::interrupt::{InterruptType, IrqHandler};

const FIRST_RESPONSE_IRQ_DELAY : u64 = 0x20; // almost immediately
const GET_ID_SECOND_RESPONSE_IRQ_DELAY : u64 = 0x4A00;
const INIT_SECOND_RESPONSE_IRQ_DELAY : u64 = 0x13CCE;
const STD_SECOND_RESPONSE_IRQ_DELAY : u64 = 0x4A73;

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
const CDROM_VER : [u8;4] = [0x94, 0x11, 0x18, 0xC1]; // 94h,11h,18h,C0h  ;PSX (PU-7)               18 Nov 1994, version vC0 (b)

#[derive(Debug,Copy,Clone)]
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
}

#[derive(Debug,Clone,Copy)]
enum CdromIRQ {
    NoINT = 0,
    INT1 = 1, // data ready
    INT2 = 2, // command completed
    INT3 = 3, // Command received and acknowledged
    INT4 = 4, // DataEnd, reached end of disc (or end of track if auto-pause enabled)
    INT5 = 5, // Command error, read error, license string error or lid opened
}

impl CdromIRQ {
    fn from_u8(value: u8) -> Option<CdromIRQ> {
        let int = match value {
            0 => Self::NoINT,
            1 => Self::INT1,
            2 => Self::INT2,
            3 => Self::INT3,
            4 => Self::INT4,
            5 => Self::INT5,
            _ => return None,
        };
        Some(int)
    }
}

#[derive(Debug,Clone,Copy)]
enum State {
    Idle = 0x00,
    Play = 0x80,
    Seek = 0x40,
    Read = 0x20,
}

#[derive(Debug,Clone,Copy)]
enum Speed {
    Normal = 0x00,
    DoubleSpeed = 0x01,
}

impl Speed {
    #[inline(always)]
    fn get_read_sector_ms(&self) -> u64 {
        match self {
            Speed::Normal => 1000 / 75,
            Speed::DoubleSpeed => 1000 / 150,
        }
    }
}

/*
___These values appear in the FIRST response; with stat.bit0 set___
  10h - Invalid Sub_function (for command 19h), or invalid parameter value
  20h - Wrong number of parameters
  40h - Invalid command
  80h - Cannot respond yet (eg. required info was not yet read from disk yet)
           (namely, TOC not-yet-read or so)
           (also appears if no disk inserted at all)
  ___These values appear in the SECOND response; with stat.bit2 set___
  04h - Seek failed (when trying to use SeekL on Audio CDs)
  ___These values appear even if no command was sent; with stat.bit2 set___
  08h - Drive door became opened
 */
#[derive(Debug,Clone,Copy)]
enum INT5Cause {
    InvalidSubFunction = 0x10,
    WrongNumberOfParameters = 0x20,
    InvalidCommand = 0x40,
    CannotRespondYet = 0x80,
    SeekFailed = 0x04,
    DriveDoorBecameOpened = 0x08,
}

const PARAMETER_FIFO_LEN : usize = 16;

pub struct CDRom {
    bank_address: usize,
    parameter_fifo: VecDeque<u8>,
    result_fifo: VecDeque<u8>,
    hintmsk_reg: u8,
    hintsts_reg: u8,
    int_1_pending_flag: bool,
    int_2_pending_flag: bool,
    state: State,
    motor_on: bool,
    shell_once_opened: bool,
    busy_status: bool,
    mode:u8,
    disc: Option<Disc>,
    pending_setloc: Option<DiscTime>,
    read_buffer: VecDeque<u8>,
    hchpctl: u8,
}

/*
Read:
Bank	0x1f801800	0x1f801801	0x1f801802	0x1f801803
0, 2	HSTS	    RESULT	    RDDATA	    HINTMSK
1, 3	HSTS	    RESULT	    RDDATA	    HINTSTS
Write:
Bank	0x1f801800	0x1f801801	0x1f801802	0x1f801803
0	    ADDRESS	    COMMAND	    PARAMETER	HCHPCTL
1	    ADDRESS	    WRDATA	    HINTMSK	    HCLRCTL
2	    ADDRESS	    CI	        ATV0	    ATV1
3	    ADDRESS	    ATV2	    ATV3	    ADPCTL
 */
impl CDRom {
    pub fn new() -> Self {
        Self {
            bank_address: 0,
            parameter_fifo: Default::default(),
            result_fifo: Default::default(),
            hintmsk_reg: 0,
            hintsts_reg: 0,
            int_1_pending_flag: false,
            int_2_pending_flag: false,
            state: State::Idle,
            motor_on: false,
            shell_once_opened: false,
            busy_status: false,
            mode: 0,
            disc: None,
            pending_setloc: None,
            read_buffer: Default::default(),
            hchpctl: 0,
        }
    }

    pub fn insert_disk(&mut self,disc:Disc) {
        self.disc = Some(disc);
        info!("CDROM inserted disk '{}'",self.disc.as_ref().unwrap().get_cue_file_name());
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
        if !self.read_buffer.is_empty() {
            hsts |= 1 << 6;
        }
        if self.busy_status {
            hsts |= 1 << 7;
        }
        //info!("CDROM reading status: {:02X}",hsts);
        hsts
    }

    pub fn write_0(&mut self, value: u8) {
        self.bank_address = (value & 3) as usize;
        //info!("CDROM selected bank is {}",self.bank_address);
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
        let empty = self.result_fifo.is_empty();
        let byte = self.result_fifo.pop_front().unwrap_or(0);
        //info!("CDROM read result byte {:02X}[empty={empty}]",byte);
        byte
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
            8 => self.read_buffer.pop_front().unwrap_or(0) as u32,
            16 => self.read_buffer.pop_front().unwrap_or(0) as u32 | (self.read_buffer.pop_front().unwrap_or(0) as u32) << 8,
            32 => u32::from_le_bytes([
                self.read_buffer.pop_front().unwrap_or(0),
                self.read_buffer.pop_front().unwrap_or(0),
                self.read_buffer.pop_front().unwrap_or(0),
                self.read_buffer.pop_front().unwrap_or(0)
            ]),
            _ => unreachable!()
        };

        //info!("CDROM read_2 read {:08X} from buffer [remain bytes={}]",read,self.read_buffer.len());

        read
    }

    pub fn peek_2(&self) -> u8 {
        0
    }

    pub fn write_1(&mut self,value:u8,clock:&mut Clock,irq_handler:&mut IrqHandler) {
        match self.bank_address {
            0 => self.write_cmd(value,clock,false),
            1 => self.write_data(value),
            2 => self.write_ci(value),
            3 => self.write_atv2(value),
            _ => unreachable!()
        }
    }
    /*
    Writing to this address sends the command byte to the HC05, which will proceed to drain the parameter FIFO, process the command, push any return values into the result FIFO and fire INT3 (or INT5 if an error occurs).
    Command/Parameter processing is indicated by BUSYSTS.
    When that bit gets zero, the response can be read immediately (immediately for MOST commands, but not ALL commands; so better wait for the IRQ).
    Alternately, you can wait for an IRQ (which seems to take place MUCH later), and then read the response.
     */
    fn write_cmd(&mut self, value: u8,clock:&mut Clock,second_response:bool) {
        if second_response {
            info!("CDROM completing second response of command {:02X}",value);
        }
        else {
            info!("CDROM sending command {:02X}",value);
            self.result_fifo.clear();
        }

        match value {
            0x01 => self.command_nop(clock),
            0x02 => self.command_setloc(clock),
            0x06 => self.command_readn(clock,second_response),
            0x09 => self.command_pause(clock,second_response),
            0x0A => self.command_init(clock,second_response),
            0x0C => self.command_demute(clock),
            0x0E => self.command_set_mode(clock),
            0x13 => self.command_get_tn(clock),
            0x15 => self.command_seekl(clock,second_response),
            0x19 => self.command_test(clock),
            0x1A => self.command_get_id(clock,second_response),
            _ => {
                warn!("CDROM send unknown command {:02X}",value);
                exit(1);
                self.schedule_irq_no_2nd_response(
                    CdromIRQ::INT5,
                    Some(&[self.get_stat(false,false,true),INT5Cause::InvalidCommand as u8]),
                    clock,
                    FIRST_RESPONSE_IRQ_DELAY,
                    true
                );
            }
        }
    }

    /*
    The PSX can deliver one INT after another. Instead of using a real queue, it's merely using some flags that do indicate which INT(s) need to be delivered.
    Basically, there seem to be two flags: One for Second Response (INT2), and one for Data/Report Response (INT1).
    There is no flag for First Response (INT3); because that INT is generated immediately after executing a command.
    The flag mechanism means that the SUB-CPU cannot hold more than one undelivered INT1.
     */
    #[inline]
    fn set_irq(&mut self,int:CdromIRQ) {
        // TODO check all irq flags (0x1F)
        let current_irq = self.hintsts_reg & 7;
        // check int1,int2 for pending flags
        if matches!(int,CdromIRQ::INT1) && current_irq == 1 { // INT1 not acknowledged
            self.int_1_pending_flag = true;
        }
        else if matches!(int,CdromIRQ::INT2) && current_irq == 2 { // INT2 not acknowledged
            self.int_2_pending_flag = true;
        }
        self.hintsts_reg = (self.hintsts_reg & !7) | (int as u8);
    }
    #[inline]
    fn ack_irqs(&mut self,ints:u8) {
        // TODO check all irq flags (0x1F)
        self.hintsts_reg = (self.hintsts_reg & !7) | (self.hintsts_reg & 7 & !ints);

        // check int1,int2 for pending flags
        if self.int_1_pending_flag {
            self.int_1_pending_flag = false;
            self.hintsts_reg |= CdromIRQ::INT1 as u8;
        }
        else if self.int_2_pending_flag {
            self.int_2_pending_flag = false;
            self.hintsts_reg |= CdromIRQ::INT2 as u8;
        }
    }

    fn schedule_irq_no_2nd_response(&mut self,int:CdromIRQ,response_bytes:Option<&[u8]>,clock:&mut Clock,irq_delay:u64,completed:bool) {
        self.busy_status = true;
        clock.schedule(EventType::CDROM(CDROMEventType::CdRomRaiseIrq { irq : int as u8 , completed }),irq_delay);
        if let Some(bytes) = response_bytes {
            for b in bytes {
                self.result_fifo.push_back(*b);
            }
        }
    }
    fn schedule_irq_with_2nd_response(&mut self,int:CdromIRQ,response_bytes:Option<&[u8]>,clock:&mut Clock,irq_delay:u64,cmd_to_complete:u8,second_delay:Option<u64>) {
        self.busy_status = true;
        clock.schedule(EventType::CDROM(CDROMEventType::CdRomRaiseIrqFor2ndResponse { irq : int as u8,cmd_to_complete,delay: second_delay }),irq_delay);
        if let Some(bytes) = response_bytes {
            for b in bytes {
                self.result_fifo.push_back(*b);
            }
        }
    }

    #[inline]
    fn check_irq(&mut self,irq_handler:&mut IrqHandler) {
        if (self.hintmsk_reg & self.hintsts_reg) != 0 {
            irq_handler.set_irq(InterruptType::CDROM)
        }
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
        stat |= self.state as u8;
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

    fn activate_motor(&mut self) {
        self.motor_on = true;
        info!("CDROM motor activated");
        // TODO
    }

    fn get_approx_seek_cycles(&self,from:&DiscTime,target:&DiscTime,clock:&Clock) -> u64 {
        let distance = (from.to_lba() as i32 - target.to_lba() as i32).abs() as u64;
        let seek_time_ms = 600 * distance / (75 * 60 * 80); // 600ms per minute, 75 frames per second, 80 sectors per frame
        clock.get_cycles_per_ms(seek_time_ms).max(1000)
    }

    fn read_data_sector(&mut self) {
        let sector_size = self.get_sector_size();
        if let Some(disc) = self.disc.as_mut() {
            match disc.read_sector() {
                Some(sector) => {
                    let data = sector.get_mode2_user_data(&sector_size);
                    self.read_buffer.extend(data);
                }
                None => {
                    warn!("CDROM read_data_sector at loc {:?} failed",disc.get_head_position());
                    exit(1);
                }
            }
            // go to next sector
            disc.set_next_sector_head_position();
        }
    }

    // ================ Commands ====================================
    // Demute - Command 0Ch --> INT3(stat)
    fn command_demute(&mut self,clock: &mut Clock) {
        if self.parameter_fifo.len() != 0 {
            self.raise_wrong_number_parameters_error(clock);
            return;
        }
        info!("CDROM demute");
        self.return_1st_response_stat(clock);
    }

    // Pause - Command 09h --> INT3(stat) --> INT2(stat)
    fn command_pause(&mut self,clock:&mut Clock,second_response:bool) {
        if second_response {
            clock.cancel_where(|event| matches!(event,EventType::CDROM(_)));
            info!("CDROM pause completed");
            self.command_completed();
            self.return_2nd_response_stat(clock);
        }
        else {
            if self.parameter_fifo.len() != 0 {
                self.raise_wrong_number_parameters_error(clock);
                return;
            }
            let stat = self.get_stat(false,false,false);
            self.schedule_irq_with_2nd_response(
                CdromIRQ::INT3,
                Some(&[stat]),
                clock,
                FIRST_RESPONSE_IRQ_DELAY,
                0x09,
                Some(STD_SECOND_RESPONSE_IRQ_DELAY),
            );
        }
    }
    // ReadN - Command 06h --> INT3(stat) --> INT1(stat) --> datablock
    fn command_readn(&mut self,clock:&mut Clock,second_response:bool) {
        if second_response || matches!(self.state,State::Read) {
            if let Some(disc) = self.disc.as_mut() {
                if let Some(loc) = self.pending_setloc.take() {
                    disc.seek_sector(loc);
                }
                info!("CDROM readn reading loc {:?} previous data in queue: {}",disc.get_head_position(),self.read_buffer.len());
                self.read_data_sector();
                // send INT1
                self.return_data_ready_response_stat(clock,0x06);
            }
        }
        else {
            if self.is_disk_inserted() {
                self.state = State::Read;
                let stat = self.get_stat(false,false,false);
                let read_sector_cycles = clock.get_cycles_per_ms(self.get_speed().get_read_sector_ms() * 1);
                self.schedule_irq_with_2nd_response(
                    CdromIRQ::INT3,
                    Some(&[stat]),
                    clock,
                    FIRST_RESPONSE_IRQ_DELAY,
                    0x06,
                    Some(read_sector_cycles),
                );
            }
            else {
                let stat = self.get_stat(false,false,true);
                self.schedule_irq_no_2nd_response(
                    CdromIRQ::INT5,
                    Some(&[stat,INT5Cause::CannotRespondYet as u8]),
                    clock,
                    FIRST_RESPONSE_IRQ_DELAY,
                    true
                );
            }
        }
    }
    // SeekL - Command 15h --> INT3(stat) --> INT2(stat)
    fn command_seekl(&mut self,clock:&mut Clock,second_response:bool) {
        if second_response {
            info!("CDROM seeking loc {:?} completed",self.pending_setloc);
            if let Some(disc) = self.disc.as_mut() {
                if let Some(loc) = self.pending_setloc.take() {
                    disc.seek_sector(loc);
                }
            }
            let stat = self.get_stat(false,false,false);
            self.schedule_irq_no_2nd_response(
                CdromIRQ::INT2,
                Some(&[stat]),
                clock,
                FIRST_RESPONSE_IRQ_DELAY,
                true
            );
            self.state = State::Idle;
        }
        else {
            if self.parameter_fifo.len() > 0 {
                self.raise_wrong_number_parameters_error(clock);
                return;
            }
            self.state = State::Seek;
            let seek_cycles = match (self.disc.as_ref(),self.pending_setloc.as_ref()) {
                (Some(disc),Some(loc)) => self.get_approx_seek_cycles(&disc.get_head_position(),&loc,clock),
                _ => STD_SECOND_RESPONSE_IRQ_DELAY
            };
            let stat = self.get_stat(false,false,false);
            info!("CDROM seeking loc {:?} with approx. {} cycles",self.pending_setloc,seek_cycles);
            self.schedule_irq_with_2nd_response(
                CdromIRQ::INT3,
                Some(&[stat]),
                clock,
                FIRST_RESPONSE_IRQ_DELAY,
                0x15,
                Some(seek_cycles),
            );
        }
    }
    // Setloc - Command 02h,amm,ass,asect --> INT3(stat)
    fn command_setloc(&mut self,clock:&mut Clock) {
        if self.parameter_fifo.len() != 3 {
            self.raise_wrong_number_parameters_error(clock);
            return;
        }
        let min = BCD::decode(self.parameter_fifo.pop_front().unwrap());
        let sec = BCD::decode(self.parameter_fifo.pop_front().unwrap());
        let frame = BCD::decode(self.parameter_fifo.pop_front().unwrap());

        if let Some(loc) = DiscTime::new_checked(min,sec,frame) {
            self.pending_setloc = Some(loc);
            info!("CDROM setloc to {:?}",loc);
            self.return_1st_response_stat(clock);
        }
        else {
            self.raise_invalid_parameters_error(clock);
        }
    }
    // GetTN - Command 13h --> INT3(stat,first,last) ;BCD
    fn command_get_tn(&mut self,clock:&mut Clock) {
        if self.parameter_fifo.len() > 0 {
            self.raise_wrong_number_parameters_error(clock);
            return;
        }
        if let Some(disc) = self.disc.as_ref() {
            let tracks = disc.get_tracks();
            let first_track_n = tracks[0].track_number();
            let last_track_n = tracks[tracks.len()-1].track_number();
            let stat = self.get_stat(false,false,false);
            self.schedule_irq_no_2nd_response(
                CdromIRQ::INT3,
                Some(&[stat,BCD::encode(first_track_n),BCD::encode(last_track_n)]),
                clock,
                FIRST_RESPONSE_IRQ_DELAY,
                true
            );
        }
        else {
            let stat = self.get_stat(false,false,true);
            self.schedule_irq_no_2nd_response(
                CdromIRQ::INT5,
                Some(&[stat,INT5Cause::CannotRespondYet as u8]),
                clock,
                FIRST_RESPONSE_IRQ_DELAY,
                true
            );
            // let stat = self.get_stat(false,false,false);
            // info!("CDROM init stat={:02X}",stat);
            // self.schedule_irq_no_2nd_response(
            //     CdromIRQ::INT3,
            //     Some(&[stat,0x01,0x02]),
            //     clock,
            //     FIRST_RESPONSE_IRQ_DELAY
            // );
        }
    }
    /*
    Init - Command 0Ah --> INT3(stat) --> INT2(stat)
    Multiple effects at once. Sets mode=20h, activates drive motor, Standby, abort all commands.
     */
    fn command_init(&mut self,clock:&mut Clock,second_response:bool) {
        if second_response {
            // apply command here
            self.state = State::Idle;
            self.pending_setloc = None;
            self.mode = 0x20;
            self.activate_motor();
            clock.cancel_where(|event| matches!(event,EventType::CDROM(_)));
            info!("CDROM init command executed");
            self.return_2nd_response_stat(clock);
        }
        else {
            if self.parameter_fifo.len() > 0 {
                self.raise_wrong_number_parameters_error(clock);
                return;
            }
            let stat = self.get_stat(false,false,false);
            info!("CDROM init stat={:02X}",stat);
            self.schedule_irq_with_2nd_response(
                CdromIRQ::INT3,
                Some(&[stat]),
                clock,
                FIRST_RESPONSE_IRQ_DELAY,
                0x0A,
                Some(INIT_SECOND_RESPONSE_IRQ_DELAY),
            );
        }
    }
    /*
    Setmode - Command 0Eh,mode --> INT3(stat)
      7   Speed       (0=Normal speed, 1=Double speed)
      6   XA-ADPCM    (0=Off, 1=Send XA-ADPCM sectors to SPU Audio Input)
      5   Sector Size (0=800h=DataOnly, 1=924h=WholeSectorExceptSyncBytes)
      4   Ignore Bit  (0=Normal, 1=Ignore Sector Size and Setloc position)
      3   XA-Filter   (0=Off, 1=Process only XA-ADPCM sectors that match Setfilter)
      2   Report      (0=Off, 1=Enable Report-Interrupts for Audio Play)
      1   AutoPause   (0=Off, 1=Auto Pause upon End of Track) ;for Audio Play
      0   CDDA        (0=Off, 1=Allow to Read CD-DA Sectors; ignore missing EDC)
     */
    fn command_set_mode(&mut self,clock:&mut Clock) {
        if self.parameter_fifo.len() != 1 {
            self.raise_wrong_number_parameters_error(clock);
            return;
        }
        self.mode = self.parameter_fifo.pop_front().unwrap();
        info!("CDROM set mode to {:02X}, speed={:?} sector size={:?}",self.mode,self.get_speed(),self.get_sector_size());
        self.return_1st_response_stat(clock);
    }

    fn command_test(&mut self,clock:&mut Clock) {
        if self.parameter_fifo.len() != 1 {
            self.raise_wrong_number_parameters_error(clock);
            return;
        }
        let sub_function = self.parameter_fifo.pop_front().unwrap();
        match sub_function {
            0x20 => {
                info!("CDROM test sub function 0x20: sending {:?}",CDROM_VER);
                self.schedule_irq_no_2nd_response(
                    CdromIRQ::INT3,
                    Some(CDROM_VER.as_slice()),
                    clock,
                    FIRST_RESPONSE_IRQ_DELAY,
                    true
                );
            }
            _ => {
                warn!("Unsupported test command sub function {}",sub_function);
                self.schedule_irq_no_2nd_response(
                    CdromIRQ::INT5,
                    Some(&[self.get_stat(false,false,true),INT5Cause::InvalidSubFunction as u8]),
                    clock,
                    FIRST_RESPONSE_IRQ_DELAY,
                    true
                );
            }
        }
    }

    fn command_nop(&mut self,clock:&mut Clock) {
        if self.parameter_fifo.len() > 0 {
            self.raise_wrong_number_parameters_error(clock);
            return;
        }
        let stat = self.get_stat(false,false,false);
        //info!("CDROM get stat (nop): sending {:02X}",stat);
        self.return_1st_response_stat(clock);
        if !self.is_shell_opened() {
            self.shell_once_opened = false;
        }
    }
    /*
        GetID - Command 1Ah --> INT3(stat) --> INT2/5 (stat,flags,type,atip,"SCEx")
        Drive Status           1st Response   2nd Response
          Door Open              INT5(11h,80h)  N/A
          Spin-up                INT5(01h,80h)  N/A
          Detect busy            INT5(03h,80h)  N/A
          No Disk                INT3(stat)     INT5(08h,40h, 00h,00h, 00h,00h,00h,00h)
          Audio Disk             INT3(stat)     INT5(0Ah,90h, 00h,00h, 00h,00h,00h,00h)
          Unlicensed:Mode1       INT3(stat)     INT5(0Ah,80h, 00h,00h, 00h,00h,00h,00h)
          Unlicensed:Mode2       INT3(stat)     INT5(0Ah,80h, 20h,00h, 00h,00h,00h,00h)
          Unlicensed:Mode2+Audio INT3(stat)     INT5(0Ah,90h, 20h,00h, 00h,00h,00h,00h)
          Debug/Yaroze:Mode2     INT3(stat)     INT2(02h,00h, 20h,00h, 20h,20h,20h,20h)
          Licensed:Mode2         INT3(stat)     INT2(02h,00h, 20h,00h, 53h,43h,45h,4xh)
          Modchip:Audio/Mode1    INT3(stat)     INT2(02h,00h, 00h,00h, 53h,43h,45h,4xh)

          1st byte: stat  (as usually, but with bit3 same as bit7 in 2nd byte)
          2nd byte: flags (bit7=denied, bit4=audio... or reportedly import, uh?)
            bit7: Licensed (0=Licensed Data CD, 1=Denied Data CD or Audio CD)
            bit6: Missing  (0=Disk Present, 1=Disk Missing)
            bit4: Audio CD (0=Data CD, 1=Audio CD) (always 0 when Modchip installed)
          3rd byte: Disk type (from TOC Point=A0h) (eg. 00h=Audio or Mode1, 20h=Mode2)
          4th byte: Usually 00h (or 8bit ATIP from Point=C0h, if session info exists)
            that 8bit ATIP value is taken form the middle 8bit of the 24bit ATIP value
          5th-8th byte: SCEx region (eg. ASCII "SCEE" = Europe) (0,0,0,0 = Unlicensed)
     */
    fn command_get_id(&mut self,clock:&mut Clock,second_response:bool) {
        if second_response {
            if self.is_shell_opened() || self.motor_on {
                let stat = self.get_stat(false,false,true);
                self.schedule_irq_no_2nd_response(
                    CdromIRQ::INT5,
                    Some(&[stat,INT5Cause::CannotRespondYet as u8]),
                    clock,
                    FIRST_RESPONSE_IRQ_DELAY,
                    true
                );
            }
            else if let Some(disc) = self.disc.as_ref() {
                // check for audio disc
                if disc.is_audio_cd() {
                    let stat = self.get_stat(true,false,false);
                    self.schedule_irq_no_2nd_response(
                        CdromIRQ::INT3,
                        Some(&[stat,INT5Cause::CannotRespondYet as u8 | INT5Cause::InvalidSubFunction as u8]),
                        clock,
                        FIRST_RESPONSE_IRQ_DELAY,
                        true
                    );
                }
                else { // Licensed:Mode2         INT3(stat)     INT2(02h,00h, 20h,00h, 53h,43h,45h,4xh)
                    let stat = self.get_stat(false,false,false);
                    let mode = if matches!(disc.get_tracks()[0].track_type(),TrackType::Data(2,_)) {
                        0x20
                    }
                    else {
                        0x00
                    };
                    let region = disc.get_region().unwrap_or(Region::USA).to_scee_letter() as u8;
                    self.schedule_irq_no_2nd_response(
                        CdromIRQ::INT2,
                        Some(&[stat,0x00,mode,0x00,b'S',b'C',b'E',b'A']),
                        clock,
                        FIRST_RESPONSE_IRQ_DELAY,
                        true
                    );
                }
            }
            else { // No Disk  INT3(stat)     INT5(08h,40h)
                let stat = self.get_stat(true,false,false);
                self.schedule_irq_no_2nd_response(
                    CdromIRQ::INT5,
                    Some(&[stat,INT5Cause::InvalidCommand as u8]),
                    clock,
                    FIRST_RESPONSE_IRQ_DELAY,
                    true
                );
            }
        }
        else {
            if self.parameter_fifo.len() > 0 {
                self.raise_wrong_number_parameters_error(clock);
                return;
            }
            if self.is_shell_opened() || self.motor_on || self.busy_status {
                self.schedule_irq_no_2nd_response(
                    CdromIRQ::INT5,
                    Some(&[self.get_stat(false,false,true), INT5Cause::CannotRespondYet as u8]),
                    clock,
                    FIRST_RESPONSE_IRQ_DELAY,
                    true
                );
                return;
            }
            let stat = self.get_stat(false,false,false);
            info!("CDROM get id stat={:02X}",stat);
            self.schedule_irq_with_2nd_response(
                CdromIRQ::INT3,
                Some(&[stat]),
                clock,
                FIRST_RESPONSE_IRQ_DELAY,
                0x1A,
                Some(GET_ID_SECOND_RESPONSE_IRQ_DELAY),
            );
        }
    }

    fn return_data_ready_response_stat(&mut self, clock:&mut Clock,cmd:u8) {
        let stat = self.get_stat(false,false,false);
        self.schedule_irq_no_2nd_response(
            CdromIRQ::INT1,
            Some(&[stat]),
            clock,
            FIRST_RESPONSE_IRQ_DELAY,
            false
        );
        // schedule next sector event
        //let cycles = clock.get_cycles_per_ms(1000 / 75);
        let cycles = clock.get_cycles_per_ms(self.get_speed().get_read_sector_ms());
        clock.schedule(EventType::CDROM(CDROMEventType::ReadNextSector(cmd)),cycles);
    }

    fn return_1st_response_stat(&mut self, clock:&mut Clock) {
        let stat = self.get_stat(false,false,false);
        self.schedule_irq_no_2nd_response(
            CdromIRQ::INT3,
            Some(&[stat]),
            clock,
            FIRST_RESPONSE_IRQ_DELAY,
            true
        );
    }

    fn return_2nd_response_stat(&mut self, clock:&mut Clock) {
        let stat = self.get_stat(false,false,false);
        self.schedule_irq_no_2nd_response(
            CdromIRQ::INT2,
            Some(&[stat]),
            clock,
            FIRST_RESPONSE_IRQ_DELAY,
            true
        );
    }

    fn raise_invalid_parameters_error(&mut self, clock:&mut Clock) {
        self.schedule_irq_no_2nd_response(
            CdromIRQ::INT5,
            Some(&[self.get_stat(false,false,true),INT5Cause::InvalidCommand as u8]),
            clock,
            FIRST_RESPONSE_IRQ_DELAY,
            true
        );
    }

    fn raise_wrong_number_parameters_error(&mut self, clock:&mut Clock) {
        self.schedule_irq_no_2nd_response(
            CdromIRQ::INT5,
            Some(&[self.get_stat(false,false,true),INT5Cause::WrongNumberOfParameters as u8]),
            clock,
            FIRST_RESPONSE_IRQ_DELAY,
            true
        );
    }
    // ==============================================================

    fn write_data(&mut self, value: u8) {
        info!("CDROM write data");
    }
    fn write_ci(&mut self, value: u8) {
        info!("CDROM write ci");
    }
    fn write_atv2(&mut self, value: u8) {
        info!("CDROM write atv2");
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
        info!("CDROM set irq mask {:02X}",value);
        self.hintmsk_reg = value;
    }
    fn write_atv0(&mut self, value: u8) {
        info!("CDROM write atv0");
    }
    fn write_atv3(&mut self, value: u8) {
        info!("CDROM write atv3");
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
            info!("CDROM parameter FIFO is full, ignoring write");
        }
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

    pub fn write_3(&mut self,value:u8, irq_handler: &mut IrqHandler) {
        match self.bank_address {
            0 => self.write_hchpctl(value),
            1 => self.write_hclrctl(value,irq_handler),
            2 => self.write_atv1(value),
            3 => self.write_adpctl(value),
            _ => unreachable!()
        }
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
    fn write_hclrctl(&mut self, value: u8, irq_handler: &mut IrqHandler) {
        //info!("CDROM write hclrctl {:02X}",value);
        self.ack_irqs(value);
        self.check_irq(irq_handler);
        if (value & 0x40) != 0 {
            //info!("Resetting parameter FIFO..");
            self.parameter_fifo.clear();
        }
    }

    fn write_atv1(&mut self,value:u8) {
        info!("CDROM write atv1");
    }

    fn write_adpctl(&mut self,value:u8) {
        info!("CDROM write adpctl");
    }

    fn command_completed(&mut self) {
        info!("Last command completed");
        self.busy_status = false;
        self.state = State::Idle;
    }

    pub fn on_event(&mut self,event: CDROMEventType,clock:&mut Clock,irq_handler:&mut IrqHandler) {
        match event {
            CDROMEventType::CdRomRaiseIrq { irq, completed } => {
                if let Some(irq) = CdromIRQ::from_u8(irq) {
                    self.set_irq(irq);
                    self.check_irq(irq_handler);
                    info!("CDROM generating {:?}",irq);
                    if completed {
                        self.command_completed();
                    }
                }
            }
            CDROMEventType::CdRomRaiseIrqFor2ndResponse { irq, cmd_to_complete, delay } => {
                if let Some(int) = CdromIRQ::from_u8(irq) {
                    if let Some(delay) = delay {
                        self.set_irq(int);
                        self.check_irq(irq_handler);
                        info!("CDROM generating {:?}",irq);
                        clock.schedule(
                            EventType::CDROM(CDROMEventType::CdRomRaiseIrqFor2ndResponse { irq,cmd_to_complete,delay:None }),
                            delay
                        );
                    }
                    else {
                        self.write_cmd(cmd_to_complete, clock, true);
                    }
                }
            }
            CDROMEventType::ReadNextSector(cmd) => {
                info!("CDROM re-executing read sector..[{}]",clock.current_time());
                self.write_cmd(cmd, clock, false);
            }
        }
    }
}

impl DmaDevice for CDRom {
    fn is_dma_ready(&self) -> bool {
        true
    }

    fn dma_request(&self) -> bool {
        true
    }

    fn dma_write(&mut self, word: u32, clock: &mut Clock, irq_handler: &mut IrqHandler) {
        todo!()
    }

    fn dma_read(&mut self) -> u32 {
        //info!("CDROM dma read");
        self.read_2::<32>()
    }
}