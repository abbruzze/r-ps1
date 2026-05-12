use crate::core::cdrom::commands::{INT1, INT4};
use crate::core::cdrom::disc::{Disc, SectorReadResult, BCD};
use crate::core::cdrom::{CDRom, Command, DriveState};
use crate::core::interrupt::IrqHandler;
use std::process::exit;
use tracing::{debug, error, info, warn};

impl CDRom {
    pub(super) fn read_data_sector(&mut self,irq_handler:&mut IrqHandler) -> bool {
        let sector_size = self.get_sector_size();
        let mut send_int1 = false;
        let mut end_of_track = false;
        if let Some(disc) = self.disc.as_mut() {
            let read_result = disc.read_sector();
            match read_result {
                SectorReadResult::SectorReadOk(sector) => {
                    // 4-byte sector header, plus 4-byte subheader of the current sector
                    self.last_sector_header.clear();
                    self.last_sector_header.extend(&sector.sector[12..20]);

                    let adpcm_enabled = (self.mode & 0x40) != 0;
                    if sector.is_audio_adpcm() &&
                        adpcm_enabled &&
                        (!self.adpcm.filter_enabled || sector.matches_file_and_channel(self.adpcm.file,self.adpcm.channel)) {
                        // Audio ADPCM
                        self.adpcm.decode_sector(&sector.sector);
                        debug!("CDROM Audio ADPCM sector at {},decoding ...",disc.get_head_position());
                    }
                    else if self.adpcm.filter_enabled && sector.is_audio_adpcm() {
                        // The controller does not send sectors to the data FIFO if ADPCM filtering is enabled
                        // and this is a real-time audio sector
                    }
                    else {
                        if self.data_buffer.len() > 0 {
                            debug!("Reading next sector at {} with old bytes not read: {}",disc.get_head_position(),self.data_buffer.len());
                        }
                        send_int1 = true;
                        let data = sector.get_mode2_user_data(&sector_size);
                        self.data_buffer.clear();
                        self.data_buffer.extend(data);
                    }
                }
                SectorReadResult::EndOfTrack => { // end of track
                    end_of_track = true;
                }
                SectorReadResult::SectorReadError(e) => {
                    error!("CDROM read_data_sector at loc {:?} failed due to {:?}",disc.get_head_position(),e);
                }
                SectorReadResult::TrackNotFound => {
                    error!("CDROM read_data_sector at loc {:?} failed: track not found",disc.get_head_position());
                }
            }
            // go to next sector
            if disc.set_next_sector_head_position() {
                end_of_track = true;
            }
        }
        if end_of_track {
            self.handle_end_of_track(irq_handler);
        }
        send_int1
    }

    fn handle_end_of_track(&mut self,irq_handler:&mut IrqHandler) {
        if let Some(disc) = &self.disc && let Some(current_track) = disc.get_current_track() { // end of track
            let last_track_number = disc.get_tracks().last().map(|track| track.track_number()).unwrap_or(0);
            if current_track.track_number() == last_track_number {
                info!("Reached end of disc, stopping...");
                self.activate_motor(false);
                self.change_drive_state(DriveState::Idle);
                self.apply_irq_and_result(Command::Play,INT4,vec![],irq_handler);
            }
        }
    }

    pub(super) fn read_audio_sector(&mut self,send_report_flag:bool,report_absolute:bool,irq_handler:&mut IrqHandler) {
        let stat = self.get_stat(false,false,false);
        let mut report = [0u8; 8];
        let mut send_report = false;
        let mut end_of_track = false;

        if let Some(disc) = self.disc.as_mut() {
            let read_result = disc.read_sector();
            match read_result {
                SectorReadResult::SectorReadOk(sector) => {
                    debug!("Playing audio sector at {:?}",disc.get_head_position());
                    self.last_audio_sector = sector.get_audio_data();
                }
                SectorReadResult::EndOfTrack => { // end of track
                    end_of_track = true;
                }
                SectorReadResult::TrackNotFound => {
                    error!("CDROM read_data_sector at loc {:?} failed: track not found",disc.get_head_position());
                }
                SectorReadResult::SectorReadError(e) => {
                    error!("CDROM read_audio_sector at loc {:?} failed due to {:?}",disc.get_head_position(),e);
                }
            }
            // check reporting
            // Report --> INT1(stat,track,index,mm/amm,ss+80h/ass,sect/asect,peaklo,peakhi)
            // amm/ass/asect are returned on asect=00h,20h,40h,60h   ;-absolute time
            // mm/ss+80h/sect are returned on asect=10h,30h,50h,70h  ;-within current track
            // (or, in case of read errors, report may be returned on other asect's)
            if send_report_flag && (self.mode & 0x04) != 0 {
                send_report = true;

                if let Some(track) = disc.get_current_track() {
                    report[0] = stat;
                    report[1] = track.track_number();
                    report[2] = (disc.get_head_position() >= track.effective_start_time()) as u8;

                    let time = if report_absolute { disc.get_head_position() } else { disc.get_head_position().sub(&track.effective_start_time()) };
                    report[3] = time.m();
                    report[4] = if report_absolute { time.s() } else { time.s() + 0x80 };
                    report[5] = time.f();
                    // TODO peak values
                    debug!("CDROM Sending play report: {:?} is_absolute={report_absolute} time={:?} track={:?}",report,time,track);
                }

                for e in report.iter_mut().skip(1) {
                    *e = BCD::encode(*e);
                }
            }

            // go to next sector
            if disc.set_next_sector_head_position() {
                if (self.mode & 0x02) != 0 { // auto-pause on for end of track
                    debug!("End of track with auto-pause set...");
                    self.change_drive_state(DriveState::Idle);
                    self.apply_irq_and_result(Command::Play,INT4,vec![],irq_handler);
                }
                else {
                    end_of_track = true;
                }
            }
        }
        if end_of_track {
            self.handle_end_of_track(irq_handler);
        }
        if send_report {
            self.apply_irq_and_result(Command::Play,INT1,report.to_vec(),irq_handler);
        }
    }
}