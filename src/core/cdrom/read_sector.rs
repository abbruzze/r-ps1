use std::process::exit;
use tracing::{info, warn};
use crate::core::cdrom::{CDRom, Command};
use crate::core::cdrom::commands::INT1;
use crate::core::cdrom::disc::BCD;
use crate::core::interrupt::IrqHandler;

impl CDRom {
    pub(super) fn read_data_sector(&mut self) -> bool {
        let sector_size = self.get_sector_size();
        let mut send_int1 = false;
        if let Some(disc) = self.disc.as_mut() {
            match disc.read_sector() {
                Some(sector) => {
                    let adpcm_enabled = (self.mode & 0x40) != 0;
                    if sector.is_audio_adpcm() &&
                        adpcm_enabled &&
                        (!self.adpcm.filter_enabled || sector.matches_file_and_channel(self.adpcm.file,self.adpcm.channel)) {
                        // Audio ADPCM
                        // TODO ...
                        info!("CDROM Audio ADPCM sector at {:?}, discarding for now ...",disc.get_head_position());
                    }
                    else if self.adpcm.filter_enabled && sector.is_audio_adpcm() {
                        // The controller does not send sectors to the data FIFO if ADPCM filtering is enabled
                        // and this is a real-time audio sector
                    }
                    else {
                        let data = sector.get_mode2_user_data(&sector_size);
                        if self.data_buffer.len() > 0 {
                            warn!("Reading next sector with old bytes not read: {}",self.data_buffer.len());
                        }
                        send_int1 = true;
                        self.last_sector.clear();
                        self.last_sector.extend(data);
                        self.data_buffer.clear();
                        self.data_buffer.extend(data);
                    }
                }
                None => {
                    warn!("CDROM read_data_sector at loc {:?} failed",disc.get_head_position());
                    exit(1);
                }
            }
            // go to next sector
            disc.set_next_sector_head_position();
        }
        send_int1
    }

    pub(super) fn read_audio_sector(&mut self,irq_handler:&mut IrqHandler) {
        let stat = self.get_stat(false,false,false);
        let mut report = [0u8; 8];
        let mut send_report = false;

        if let Some(disc) = self.disc.as_mut() {
            match disc.read_sector() {
                Some(sector) => {
                    info!("Playing audio sector at {:?}",disc.get_head_position());
                    self.last_audio_sector = sector.get_audio_data();
                }
                None => {
                    warn!("CDROM read_audio_sector at loc {:?} failed",disc.get_head_position());
                    exit(1);
                }
            }
            // check reporting
            // Report --> INT1(stat,track,index,mm/amm,ss+80h/ass,sect/asect,peaklo,peakhi)
            // amm/ass/asect are returned on asect=00h,20h,40h,60h   ;-absolute time
            // mm/ss+80h/sect are returned on asect=10h,30h,50h,70h  ;-within current track
            // (or, in case of read errors, report may be returned on other asect's)
            let sect = disc.get_head_position().f();
            if (self.mode & 0x04) != 0 && (sect & 0xF) == 0 {
                send_report = true;
                let is_absolute_time = ((sect >> 4) & 1) == 0;

                if let Some(track) = disc.get_current_track() {
                    report[0] = stat;
                    report[1] = track.track_number();
                    report[2] = 0x01;

                    let time = if is_absolute_time { disc.get_head_position() } else { disc.get_head_position().sub(track.start_time()) };
                    report[3] = time.m();
                    report[4] = if is_absolute_time { time.s() } else { time.s() + 0x80 };
                    report[5] = time.f();
                    // TODO peak values
                }
                for e in report.iter_mut() {
                    *e = BCD::encode(*e);
                }
            }

            // go to next sector
            let end_of_track = disc.set_next_sector_head_position();
            if end_of_track && (self.mode & 0x02) != 0 { // auto-pause on for end of track
                // TODO
                info!("End of track ...");
            }
        }
        if send_report {
            info!("CDROM Sending play report: {:?}",report);
            self.apply_irq_and_result(INT1,report.to_vec(),irq_handler);
        }
    }
}