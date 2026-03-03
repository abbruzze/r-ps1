use std::process::exit;
use tracing::{info, warn};
use crate::core::cdrom::CDRom;

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
}