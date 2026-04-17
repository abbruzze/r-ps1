use std::process::exit;
use tracing::{debug, error, info, warn};
use crate::core::cdrom::{CDRom, Command, CommandState, DriveState, Region, CDROM_VER};
use crate::core::cdrom::disc::{AudioLeftRight, DiscTime, TrackType, BCD};
use crate::core::interrupt::IrqHandler;

pub(super) const INT1 : u8 = 0x01;
pub(super) const INT2 : u8 = 0x02;
pub(super) const INT3 : u8 = 0x03;
pub(super) const INT4 : u8 = 0x04;
pub(super) const INT5 : u8 = 0x05;

pub(super) const STAT_NO_ERR : Option<(bool,bool,bool)> = Some((false,false,false));
pub(super) const STAT_NO_DATA : &[u8] = &[];

pub(super) const FIRST_RESPONSE_IRQ_DELAY_44100 : usize = 1;
pub(super) const STD_SECOND_RESPONSE_IRQ_DELAY_44100 : usize = delay_cycles_44100(0x4A73);
pub(super) const INIT_SECOND_RESPONSE_IRQ_DELAY_44100 : usize = delay_cycles_44100(900_000);
pub(super) const GET_ID_SECOND_RESPONSE_IRQ_DELAY_44100: usize = STD_SECOND_RESPONSE_IRQ_DELAY_44100;
pub(super) const READ_TOC_SECOND_RESPONSE_IRQ_DELAY_44100: usize = STD_SECOND_RESPONSE_IRQ_DELAY_44100;

const fn delay_cycles_44100(cycles:usize) -> usize {
    cycles / (33_868_800 / 44_100)
}

impl CDRom {
    pub(super) fn change_drive_state(&mut self, new_state: DriveState) {
        //info!("CDROM drive state changed to {:?}",new_state);
        self.drive_state = new_state;
    }

    pub(super) fn check_drive_state(&mut self, irq_handler: &mut IrqHandler) {
        let new_state = match self.drive_state.clone() {
            DriveState::Idle => DriveState::Idle,
            DriveState::Playing { sample_index, report_counter, report_absolute } if sample_index < 0 => {
                DriveState::Playing { sample_index: sample_index + 1, report_counter, report_absolute }
            }
            DriveState::Playing { sample_index, report_counter, report_absolute } => {
                self.play_sample(sample_index,report_counter,report_absolute,irq_handler)
            },
            DriveState::Seeking => DriveState::Seeking,
            DriveState::Reading { next_sector_cycles } => {
                // update adpcm sample
                if let Some((sample_l, sample_r)) = self.adpcm.maybe_output_sample() {
                    self.audio_sample = AudioLeftRight(sample_l, sample_r);
                }
                if next_sector_cycles == 1 {
                    self.keep_reading_sector(irq_handler)
                }
                else {
                    DriveState::Reading { next_sector_cycles: next_sector_cycles - 1 }
                }
            }
        };

        self.change_drive_state(new_state);
    }

    pub(super) fn check_command_state(&mut self, irq_handler: &mut IrqHandler) {
        let state = std::mem::replace(&mut self.command_state, CommandState::Idle);

        let new_state = match state {
            CommandState::Idle => {
                CommandState::Idle
            }
            CommandState::WaitingIrqAck(cmd) => {
                if self.is_irq_pending() {
                    state
                }
                else {
                    CommandState::ToProcess(cmd)
                }
            }
            CommandState::ToProcess(cmd) => {
                self.process_command(cmd)
            }
            CommandState::Response { cmd, irq, delay_cycles, response, next_state } => {
                if delay_cycles == 1 {
                    // send INT
                    self.apply_irq_and_result(cmd,irq, response, irq_handler);
                    *next_state
                } else {
                    CommandState::Response { cmd, irq, delay_cycles: delay_cycles - 1, response, next_state }
                }
            }
            CommandState::Delay { cmd, delay_cycles, next_state } => {
                if delay_cycles == 1 {
                    *next_state
                } else {
                    CommandState::Delay { cmd, delay_cycles: delay_cycles - 1, next_state }
                }
            }
            CommandState::Response2 { cmd } => {
                self.process_2nd_response(cmd)
            }
        };
        self.command_state = new_state;
        // if matches!(self.command_state,CommandState::Response { irq:INT3, ..}) {
        //     self.check_command_state(irq_handler);
        // }
    }

    fn process_command(&mut self, cmd: u8) -> CommandState {
        info!("CDROM processing command {:02X}",cmd);
        match Command::from_u8(cmd) {
            Some(command) => {
                if !command.parameters_number().contains(&self.parameter_fifo.len()) {
                    error!("CDROM invalid parameters for command {:?}, found {} expected {:?}",command,self.parameter_fifo.len(),command.parameters_number());
                    self.make_bad_parameter_response(command)
                } else {
                    self.execute_command(command, false)
                }
            }
            None => {
                error!("CDROM unknown command {:02X}",cmd);
                // TODO
                exit(1);
            }
        }
    }

    fn process_2nd_response(&mut self, cmd: Command) -> CommandState {
        info!("CDROM processing 2nd response for command {:?}",cmd);
        self.execute_command(cmd, true)
    }

    pub(super) fn apply_irq_and_result(&mut self,cmd:Command, irq: u8, response: Vec<u8>, irq_handler: &mut IrqHandler) {
        debug!("CDROM applying irq {:02X} with response {:?} for command {cmd:?}",irq,response);
        self.set_irq(irq);
        self.check_irq(irq_handler);
        for b in response {
            self.result_fifo.push_back(b);
        }
    }

    fn execute_command(&mut self, cmd: Command, second_response: bool) -> CommandState {
        match cmd {
            Command::Nop => self.command_nop(),
            Command::Setloc => self.command_setloc(),
            Command::SeekL|Command::SeekP => self.command_seek(second_response),
            Command::Test => self.command_test(),
            Command::GetID => self.command_get_id(second_response),
            Command::ReadTOC => self.command_read_toc(second_response),
            Command::SetMode => self.command_setmode(),
            Command::Read => self.command_read(second_response),
            Command::Pause => self.command_pause(second_response),
            Command::Init => self.command_init(second_response),
            Command::Demute => self.command_demute(),
            Command::SetFilter => self.command_set_filter(),
            Command::Stop => self.command_stop(second_response),
            Command::GetLocL => self.command_get_locl(),
            Command::GetLocP => self.command_get_locp(),
            Command::GetTN => self.command_get_tn(),
            Command::GetTD => self.command_get_td(),
            Command::Play => self.command_play(),
            Command::MotorOn => self.command_motor_on(second_response),
            Command::Mute => self.command_mute(),
        }
    }

    fn make_bad_parameter_response(&mut self, cmd: Command) -> CommandState {
        self.make_response(
            cmd,
            INT5,
            FIRST_RESPONSE_IRQ_DELAY_44100,
            &[0x20], // WrongNumberOfParameters
            Some((false, false, true)),
            CommandState::Idle
        )
    }

    fn make_stat_response(&mut self, irq: u8, cmd: Command) -> CommandState {
        self.make_response(
            cmd,
            irq,
            FIRST_RESPONSE_IRQ_DELAY_44100,
            &[self.get_stat(false, false, false)],
            None,
            CommandState::Idle
        )
    }

    pub(super) fn make_response(&mut self,
                     cmd: Command,
                     irq: u8,
                     delay_cycles: usize,
                     data: &[u8],
                     errors: Option<(bool, bool, bool)>,
                     next_state: CommandState) -> CommandState {
        let mut response = vec![];
        if let Some((id_error, seek_error, error)) = errors {
            let state = self.get_stat(id_error, seek_error, error);
            response.push(state);
        }

        response.extend(data);
        CommandState::Response {
            cmd,
            irq,
            delay_cycles,
            response,
            next_state: Box::new(next_state),
        }
    }

    fn make_error_response(&mut self, cmd: Command, error: u8, stat_flags: (bool, bool, bool)) -> CommandState {
        self.make_response(
            cmd,
            INT5,
            FIRST_RESPONSE_IRQ_DELAY_44100,
            &[error],
            Some(stat_flags),
            CommandState::Idle
        )
    }

    fn get_approx_seek_cycles_44100(&self, from: &DiscTime, target: &DiscTime) -> usize {
        const MIN_SEEK_TIME_44100: usize = 24;
        let distance = (from.to_lba() as i32 - target.to_lba() as i32).abs() as u64;
        let seek_time_ms = 1000.0 * distance as f32 / (75.0 * 60.0 * 80.0); // 1000ms per minute, 75 frames per second, 80 sectors per frame
        self.get_cycles_per_ms_44100(seek_time_ms).max(MIN_SEEK_TIME_44100)

    }

    #[inline(always)]
    fn get_cycles_per_ms_44100(&self, ms: f32) -> usize {
        ((44100.0 / 1000.0 * ms) as usize).max(1)
    }

    // =============================================================================================
    // Nop - Command 01h --> INT3(stat)
    fn command_nop(&mut self) -> CommandState {
        if !self.is_shell_opened() {
            self.shell_once_opened = false;
        }
        self.make_stat_response(INT3, Command::Nop)
    }
    // Setloc - Command 02h,amm,ass,asect --> INT3(stat)
    fn command_setloc(&mut self) -> CommandState {
        let min = BCD::decode(self.parameter_fifo.pop_front().unwrap());
        let sec = BCD::decode(self.parameter_fifo.pop_front().unwrap());
        let frame = BCD::decode(self.parameter_fifo.pop_front().unwrap());

        if let Some(loc) = DiscTime::new_checked(min, sec, frame) {
            self.pending_setloc = Some(loc);
            info!("CDROM setloc to {:?}",loc);
            self.make_stat_response(INT3, Command::Setloc)
        } else {
            error!("CDROM invalid setloc {:02X}:{:02X}:{:02X}",min,sec,frame);
            self.make_response(Command::Setloc, INT5, FIRST_RESPONSE_IRQ_DELAY_44100, &[0x40], Some((false, false, true)), CommandState::Idle) // InvalidCommand
        }
    }
    // Play - Command 03h (,track) --> INT3(stat) --> optional INT1(report bytes)
    fn command_play(&mut self) -> CommandState {
        self.activate_motor(true);

        if let Some(disc) = self.disc.as_mut() {
            if !matches!(self.drive_state,DriveState::Playing { .. }) { // fixes motorhead
                self.drive_state = DriveState::Playing { sample_index: 0, report_counter: 1, report_absolute: true };
            }
            let mut track = BCD::decode(self.parameter_fifo.pop_front().unwrap_or(0));
            if track == 0 {
                if let Some(loc) = self.pending_setloc.take() {
                    info!("CDROM play seeking {:?}",loc);
                    disc.seek_sector(loc);
                }
                else {
                    debug!("CDROM play requested track 0 => playing from current track {:?}",disc.get_current_track().map(|t|t.track_number()));
                }
            }
            else {
                let t = disc.get_track_by_number(track).unwrap();
                track = t.track_number();
                let start_time = t.effective_start_time();
                disc.seek_sector(start_time);

            }
            info!("CDROM play track {track} at loc {:?}",disc.get_head_position());

            self.make_stat_response(INT3,Command::Play)
        }
        else {
            self.make_error_response(Command::Play, 0x80, (false, false, true)) // cannot response yet
        }
    }
    // SeekL - Command 15h --> INT3(stat) --> INT2(stat)
    fn command_seek(&mut self, second_response: bool) -> CommandState {
        if second_response {
            info!("CDROM seeking loc {:?} completed",self.pending_setloc);
            if let Some(disc) = self.disc.as_mut() {
                if let Some(loc) = self.pending_setloc.take() {
                    disc.seek_sector(loc);
                }
            }
            self.change_drive_state(DriveState::Idle);
            self.make_stat_response(INT2, Command::SeekL)
        } else {
            self.change_drive_state(DriveState::Seeking);
            self.activate_motor(true);
            let seek_cycles = match (self.disc.as_ref(), self.pending_setloc.as_ref()) {
                (Some(disc), Some(loc)) => {
                    let cycles = self.get_approx_seek_cycles_44100(&disc.get_head_position(), &loc);
                    info!("CDROM seekl from {:?} to {:?} approx cycles={}",disc.get_head_position(),loc,cycles);
                    cycles
                },
                _ => FIRST_RESPONSE_IRQ_DELAY_44100
            };

            self.make_response(
                Command::SeekL,
                INT3,
                FIRST_RESPONSE_IRQ_DELAY_44100,
                STAT_NO_DATA,
                STAT_NO_ERR,
                CommandState::Delay {
                    cmd: Command::SeekL,
                    delay_cycles: seek_cycles,
                    next_state: Box::new(
                        CommandState::Response2 { cmd: Command::SeekL }
                    )
                }
            )
        }
    }
    // 19h,20h --> INT3(yy,mm,dd,ver)
    fn command_test(&mut self) -> CommandState {
        let sub_function = self.parameter_fifo.pop_front().unwrap();
        match sub_function {
            0x04 => {
                info!("CDROM test sub function 0x04");
                self.activate_motor(true);
                self.make_stat_response(INT3,Command::Test)
            }
            0x05 => {
                info!("CDROM test sub function 0x05");
                self.make_response(
                    Command::Test,
                    INT3,
                    FIRST_RESPONSE_IRQ_DELAY_44100,
                    &[0x00,0x00],
                    STAT_NO_ERR,
                    CommandState::Idle
                )
            }
            0x20 => {
                info!("CDROM test sub function 0x20: sending {:?}",CDROM_VER);
                self.make_response(
                    Command::Test,
                    INT3,
                    FIRST_RESPONSE_IRQ_DELAY_44100,
                    CDROM_VER.as_slice(), // Invalid sub-function
                    None,
                    CommandState::Idle
                )
            }
            _ => {
                warn!("Unsupported test command sub function {}",sub_function);
                self.make_error_response(Command::Test, 0x10, (false, false, true)) // Invalid sub-function
            }
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
    fn command_get_id(&mut self, second_response: bool) -> CommandState {
        if second_response {
            if let Some(disc) = self.disc.as_ref() {
                // check for audio disc
                if disc.is_audio_cd() {
                    info!("CDROM get_id error: audio-cd");
                    self.make_error_response(Command::GetID, 0x80, (false, false, true)) // cannot response yet
                } else {
                    // Licensed:Mode2         INT3(stat)     INT2(02h,00h, 20h,00h, 53h,43h,45h,4xh)
                    let mode = if matches!(disc.get_tracks()[0].track_type(),TrackType::Data(2,_)) { 0x20 } else { 0x00 };
                    let region = disc.get_region().unwrap_or(Region::USA).to_scee_letter() as u8;
                    info!("CDROM executing get_id region={:?}:",region);
                    self.make_response(
                        Command::GetID,
                        INT2,
                        FIRST_RESPONSE_IRQ_DELAY_44100,
                        &[0x00, mode, 0x00, b'S', b'C', b'E', region],
                        STAT_NO_ERR,
                        CommandState::Idle
                    )
                }
            } else {
                self.make_error_response(Command::GetID, 0x40, (false, false, true)) // invalid command
            }
        } else {
            self.make_response(
                Command::GetID,
                INT3,
                FIRST_RESPONSE_IRQ_DELAY_44100,
                STAT_NO_DATA,
                STAT_NO_ERR,
                CommandState::Delay {
                    cmd: Command::GetID,
                    delay_cycles: GET_ID_SECOND_RESPONSE_IRQ_DELAY_44100,
                    next_state: Box::new(
                        CommandState::Response2 { cmd: Command::GetID }
                    )
                }
            )
        }
    }
    // ReadTOC - Command 1Eh --> INT3(stat) --> INT2(stat)
    fn command_read_toc(&mut self, second_response: bool) -> CommandState {
        if second_response {
            info!("CDROM read_toc completed");
            self.make_stat_response(INT2, Command::ReadTOC)
        } else {
            self.make_response(
                Command::ReadTOC,
                INT3,
                FIRST_RESPONSE_IRQ_DELAY_44100,
                STAT_NO_DATA,
                STAT_NO_ERR,
                CommandState::Delay {
                    cmd: Command::ReadTOC,
                    delay_cycles: READ_TOC_SECOND_RESPONSE_IRQ_DELAY_44100,
                    next_state: Box::new(
                        CommandState::Response2 { cmd: Command::ReadTOC }
                    )
                }
            )
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

      The "Ignore Bit" does reportedly force a sector size of 2328 bytes (918h), however, that doesn't seem to be true.
      Instead, Bit4 seems to cause the controller to ignore the sector size in Bit5 (instead, the size is kept from the most recent Setmode command which didn't have Bit4 set).
     */
    fn command_setmode(&mut self) -> CommandState {
        let prev_mode = self.mode;
        self.mode = self.parameter_fifo.pop_front().unwrap();
        self.adpcm.filter_enabled = (self.mode & (1 << 3)) != 0;
        let ignore_bit = (self.mode & (1 << 4)) != 0;
        if ignore_bit {
            // preserve last sector size bit
            self.mode = (self.mode & !(1 << 5)) | (prev_mode & (1 << 5));
        }
        info!("CDROM set mode to {:02X}, speed={:?} sector size={:?} ignore_bit={ignore_bit} cd-da:{} report={}",self.mode,self.get_speed(),self.get_sector_size(),(self.mode & 1) != 0,(self.mode & 4) != 0);
        self.make_stat_response(INT3, Command::SetMode)
    }
    // ReadN/S - Command 06h --> INT3(stat) --> INT1(stat) --> datablock
    fn command_read(&mut self,second_response:bool) -> CommandState {
        if second_response {
            let next_sector_cycles = self.get_cycles_per_ms_44100(self.get_speed().get_read_sector_ms());
            self.change_drive_state(DriveState::Reading { next_sector_cycles });
            CommandState::Idle
        }
        else if self.is_disk_inserted() {
            self.activate_motor(true);
            //self.data_buffer.clear(); // maybe ?
            //self.make_stat_response(INT3, Command::Read)
            self.make_response(
                Command::Read,
                INT3,
                FIRST_RESPONSE_IRQ_DELAY_44100,
                STAT_NO_DATA,
                STAT_NO_ERR,
                CommandState::Response2 { cmd : Command::Read }
            )
        } else {
            self.make_error_response(Command::Read, 0x80, (false, false, true)) // cannot response yet
        }
    }

    fn keep_reading_sector(&mut self, irq_handler: &mut IrqHandler) -> DriveState {
        if let Some(loc) = self.pending_setloc.take() {
            if let Some(disc) = self.disc.as_mut() {
                disc.seek_sector(loc);
                info!("CDROM start reading from loc {:?} previous data in queue: {}",disc.get_head_position(),self.data_buffer.len());
            }
        }

        let send_int1 = self.read_data_sector(irq_handler);
        let next_sector_cycles = self.get_cycles_per_ms_44100(self.get_speed().get_read_sector_ms());
        if send_int1 {
            debug!("CDROM reading sending INT1");
            self.apply_irq_and_result(Command::Read,INT1, vec![self.get_stat(false, false, false)], irq_handler);
        }

        debug!("CDROM reading next sector in {} cycles",next_sector_cycles);
        DriveState::Reading { next_sector_cycles }
    }
    // Pause - Command 09h --> INT3(stat) --> INT2(stat)
    fn command_pause(&mut self, second_response: bool) -> CommandState {
        if second_response {
            info!("CDROM executing pause from drive state: {:?}",self.drive_state);
            self.make_stat_response(INT2, Command::Pause)
        } else {
            let response = self.make_response(
                Command::Pause,
                INT3,
                FIRST_RESPONSE_IRQ_DELAY_44100,
                STAT_NO_DATA,
                STAT_NO_ERR,
                CommandState::Delay {
                    cmd: Command::Pause,
                    delay_cycles: 5 * self.get_cycles_per_ms_44100(self.get_speed().get_read_sector_ms()),
                    next_state: Box::new(
                        CommandState::Response2 { cmd: Command::Pause }
                    )
                }
            );
            self.change_drive_state(DriveState::Idle);
            response
        }
    }

    /*
       Init - Command 0Ah --> INT3(stat) --> INT2(stat)
       Multiple effects at once. Sets mode=20h, activates drive motor, Standby, abort all commands.
    */
    fn command_init(&mut self, second_response: bool) -> CommandState {
        if second_response {
            self.pending_setloc = None;
            self.mode = 0x20;
            self.data_buffer.clear();
            self.activate_motor(true);
            info!("CDROM init executed");
            self.change_drive_state(DriveState::Idle);
            self.make_stat_response(INT2, Command::Init)
        } else {
            self.make_response(
                Command::Init,
                INT3,
                FIRST_RESPONSE_IRQ_DELAY_44100,
                STAT_NO_DATA,
                STAT_NO_ERR,
                CommandState::Delay {
                    cmd: Command::Pause,
                    delay_cycles: INIT_SECOND_RESPONSE_IRQ_DELAY_44100,
                    next_state: Box::new(
                        CommandState::Response2 { cmd: Command::Init }
                    )
                }
            )
        }
    }
    // Demute - Command 0Ch --> INT3(stat)
    fn command_demute(&mut self) -> CommandState {
        info!("CDROM demute");
        self.audio_mute = false;
        self.make_stat_response(INT3, Command::Demute)
    }
    // Setfilter - Command 0Dh,file,channel --> INT3(stat)
    fn command_set_filter(&mut self) -> CommandState {
        self.adpcm.file = self.parameter_fifo.pop_front().unwrap();
        self.adpcm.channel = self.parameter_fifo.pop_front().unwrap();
        info!("CDROM set filter file {:02X} channel {:02X}",self.adpcm.file,self.adpcm.channel);
        self.make_stat_response(INT3, Command::SetFilter)
    }
    // Stop - Command 08h --> INT3(stat) --> INT2(stat)
    fn command_stop(&mut self, second_response: bool) -> CommandState {
        if second_response {
            self.activate_motor(false);
            if let Some(disc) = self.disc.as_mut() {
                // move to start of track 1
                let start_time = disc.get_tracks()[0].effective_start_time().clone();
                disc.seek_sector(start_time);
                info!("CDROM stop completed. Seeked to track1 start time {:?} ",start_time);
            }
            self.make_stat_response(INT2, Command::Stop)
        } else {
            self.drive_state = DriveState::Idle;
            self.make_response(
                Command::Stop,
                INT3,
                FIRST_RESPONSE_IRQ_DELAY_44100,
                STAT_NO_DATA,
                STAT_NO_ERR,
                CommandState::Delay {
                    cmd: Command::Stop,
                    delay_cycles: STD_SECOND_RESPONSE_IRQ_DELAY_44100,
                    next_state: Box::new(
                        CommandState::Response2 { cmd: Command::Stop }
                    )
                }
            )
        }
    }
    // GetlocL - Command 10h --> INT3(amm,ass,asect,mode,file,channel,sm,ci)
    fn command_get_locl(&mut self) -> CommandState {
        // extract 8 bytes (12..19) from current sector
        let mut locl = Vec::new();
        for b in self.last_sector[12..20].iter() {
            locl.push(*b);
        }
        info!("CDROM getlocl: {:?}",locl);
        self.make_response(
            Command::GetLocL,
            INT3,
            FIRST_RESPONSE_IRQ_DELAY_44100,
            &locl,
            None,
            CommandState::Idle
        )
    }
    // GetlocP - Command 11h - INT3(track,index,mm,ss,sect,amm,ass,asect)
    fn command_get_locp(&mut self) -> CommandState {
        let mut locp = [0u8; 8];
        if let Some(disc) = self.disc.as_ref() {
            if let Some(track) = disc.get_current_track() {
                locp[0] = track.track_number();
                let absolute_time = disc.get_head_position();
                locp[1] = (absolute_time >= track.effective_start_time()).into();
                let track_relative_time = disc.get_head_position().sub(&track.start_time());
                locp[2] = absolute_time.m();
                locp[3] = absolute_time.s();
                locp[4] = absolute_time.f();
                locp[5] = track_relative_time.m();
                locp[6] = track_relative_time.s();
                locp[7] = track_relative_time.f();
                info!("CDROM getlocp absolute={:?} relative={:?} index={}",absolute_time,track_relative_time,locp[1]);
            }
            for e in locp.iter_mut() {
                *e = BCD::encode(*e);
            }
        }
        self.make_response(
            Command::GetLocP,
            INT3,
            FIRST_RESPONSE_IRQ_DELAY_44100,
            &locp,
            None,
            CommandState::Idle
        )
    }
    // GetTN - Command 13h --> INT3(stat,first,last) ;BCD
    fn command_get_tn(&mut self) -> CommandState {
        if let Some(disc) = self.disc.as_ref() {
            let tracks = disc.get_tracks();
            let first_track_n = tracks[0].track_number();
            let last_track_n = tracks[tracks.len() - 1].track_number();
            info!("CDROM get_tn first track {} last track {}",first_track_n,last_track_n);
            self.make_response(
                Command::GetTN,
                INT3,
                FIRST_RESPONSE_IRQ_DELAY_44100,
                &[BCD::encode(first_track_n), BCD::encode(last_track_n)],
                STAT_NO_ERR,
                CommandState::Idle
            )
        } else {
            self.make_error_response(Command::GetTN, 0x80, (false, false, true)) // cannot response yet
        }
    }
    // GetTD - Command 14h,track --> INT3(stat,mm,ss) ;BCD
    fn command_get_td(&mut self) -> CommandState {
        if let Some(disc) = self.disc.as_ref() {
            let track_n = BCD::decode(self.parameter_fifo.pop_front().unwrap());
            match disc.get_track_by_number(track_n) {
                Some(track) => {
                    let start_time = track.effective_start_time();
                    info!("CDROM get_td track {}/{} start time {:?}",track_n,track.track_number(),start_time);

                    return self.make_response(
                        Command::GetTD,
                        INT3,
                        FIRST_RESPONSE_IRQ_DELAY_44100,
                        &[BCD::encode(start_time.m()),BCD::encode(start_time.s())],
                        STAT_NO_ERR,
                        CommandState::Idle
                    )
                }
                None => {
                    error!("CDROM get_td track {} not found",track_n);
                }
            }
        }
        self.make_error_response(Command::GetTD, 0x10, (false, false, true)) // Invalid sub-function
    }
    // MotorOn - Command 07h --> INT3(stat) --> INT2(stat)
    // Activates the drive motor, works ONLY if the motor was off (otherwise fails with INT5(stat,20h);
    // that error code would normally indicate "wrong number of parameters", but means "motor already on" in this case).
    fn command_motor_on(&mut self,second_response:bool) -> CommandState {
        if second_response {
            info!("CDROM motor_on completed");
            self.make_stat_response(INT2, Command::MotorOn)
        }
        else {
            if self.motor_on {
                self.make_bad_parameter_response(Command::MotorOn)
            }
            else {
                self.motor_on = true;
                self.make_response(
                    Command::MotorOn,
                    INT3,
                    FIRST_RESPONSE_IRQ_DELAY_44100,
                    STAT_NO_DATA,
                    STAT_NO_ERR,
                    CommandState::Delay {
                        cmd: Command::MotorOn,
                        delay_cycles: STD_SECOND_RESPONSE_IRQ_DELAY_44100,
                        next_state: Box::new(
                            CommandState::Response2 { cmd: Command::MotorOn }
                        )
                    }
                )
            }
        }
    }
    // Mute - Command 0Bh --> INT3(stat)
    fn command_mute(&mut self) -> CommandState {
        info!("CDROM mute on");
        self.audio_mute = true;
        self.make_stat_response(INT3, Command::MotorOn)
    }

    // =========================================

    fn play_sample(&mut self,mut sample_index:isize,mut report_counter:usize, mut report_absolute: bool,irq_handler:&mut IrqHandler) -> DriveState {
        if sample_index == 0 {
            self.read_audio_sector(report_counter == 1,report_absolute,irq_handler);
            if report_counter == 1 {
                report_counter = 16;
                report_absolute ^= true;
            }
            else {
                report_counter -= 1;
            }
        }
        let AudioLeftRight(left,right) = &self.last_audio_sector[sample_index as usize];
        self.audio_sample = AudioLeftRight(*left,*right);

        sample_index += 1;
        if sample_index as usize == self.last_audio_sector.len() {
            sample_index = 0;
        }

        DriveState::Playing { sample_index, report_counter, report_absolute }
    }
}