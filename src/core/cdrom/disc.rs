use std::cmp::Ordering;
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const SECTOR_SIZE : u16 = 2352;

#[derive(Copy,Clone,Debug)]
pub struct DiscTime {
    minutes:u8, // 00 - 99
    seconds:u8, // 00 - 59
    frames:u8 // 00 - 74
}

impl DiscTime {
    const FRAME_TIME : DiscTime = DiscTime { minutes: 0, seconds: 0, frames: 1 };

    pub fn new(minutes:u8,seconds:u8,frames:u8) -> Self {
        Self { minutes, seconds, frames }
    }

    pub fn m(&self) -> u8 {
        self.minutes
    }

    pub fn s(&self) -> u8 {
        self.seconds
    }

    pub fn f(&self) -> u8 {
        self.frames
    }

    pub fn to_lba(&self) -> u32 {
        (self.minutes as u32 * 60 + self.seconds as u32) * 75 + self.frames as u32
    }

    pub fn from_lba(total_frames:u32) -> Self {
        let frames = (total_frames % 75) as u8;
        let total_seconds = total_frames / 75;
        let seconds = (total_seconds % 60) as u8;
        let minutes = (total_seconds / 60) as u8;
        Self { minutes, seconds, frames }
    }

    pub fn from_file_length(file_length:u32) -> Self {
        Self::from_lba(file_length / SECTOR_SIZE as u32)
    }

    pub fn add(&self,other:&Self) -> Self {
        Self::from_lba(self.to_lba() + other.to_lba())
    }

    pub fn sub(&self,other:&Self) -> Self {
        Self::from_lba(self.to_lba().saturating_sub(other.to_lba()))
    }
}

impl Into<u32> for DiscTime {
    fn into(self) -> u32 {
        self.to_lba()
    }
}

impl PartialEq for DiscTime {
    fn eq(&self, other: &Self) -> bool {
        self.minutes == other.minutes && self.seconds == other.seconds && self.frames == other.frames
    }
}

impl Eq for DiscTime {}

impl PartialOrd<Self> for DiscTime {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.to_lba().cmp(&other.to_lba()))
    }
}

impl Ord for DiscTime {
    fn cmp(&self, other: &Self) -> Ordering {
        self.to_lba().cmp(&other.to_lba())
    }
}

impl fmt::Display for DiscTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DiscTime({:02}:{:02}:{:02})", self.minutes, self.seconds,self.frames)
    }
}

struct BCD {}
impl BCD {
    fn decode(value:u8) -> u8 {
        ((value >> 4) * 10) + (value & 0x0F)
    }
    fn encode(value:u8) -> u8 {
        ((value / 10) << 4) + (value % 10)
    }
}

#[derive(Debug)]
pub struct AudioLeftRight(u16,u16);

#[derive(Debug)]
pub enum TrackType {
    Audio,
    Data(u8,u16), // mode, length
}
#[derive(Debug)]
pub enum TrackSectorDataSize {
    DataOnly,
    WholeSectorExceptSyncBytes,
}
#[derive(Debug)]
pub struct DataSector {
    pub lba: u32,
    pub sector: [u8; SECTOR_SIZE as usize],
}

impl DataSector {
    fn empty(lba:u32) -> Self {
        Self { lba, sector: [0; SECTOR_SIZE as usize] }
    }

    /*
    User Data Offset

    Mode	    Offset	Size
    Mode1	    16	    2048
    Mode2 Form1	24	    2048
    Mode2 Form2	24	    2324

    Mode 2:
    Offset  Size  Nome
    0       12    Sync
    12      3     MSF (Minute, Second, Frame)
    15      1     Mode (sempre = 02)
    16      8     Subheader

    Subheader:
    Offset  Size  Nome
    16      1     File Number
    17      1     Channel Number
    18      1     Submode
    19      1     Coding Info
    20      1     File Number (copy)
    21      1     Channel Number (copy)
    22      1     Submode (copy)
    23      1     Coding Info (copy)
     */
    pub fn get_mode2_user_data(&self, data_size:&TrackSectorDataSize) -> &[u8] {
        match data_size {
            TrackSectorDataSize::DataOnly => &self.sector[24..24 + 2048],
            TrackSectorDataSize::WholeSectorExceptSyncBytes => &self.sector[12..]
        }
    }

    pub fn get_audio_data(&self) -> Vec<AudioLeftRight> {
        let mut result = Vec::new();
        for i in 0..(self.sector.len() / 4) {
            result.push(AudioLeftRight(u16::from_le_bytes([self.sector[i * 4], self.sector[i * 4 + 1]]),u16::from_le_bytes([self.sector[i * 4 + 2], self.sector[i * 4 + 3]])));
        }
        result
    }

    pub fn get_data_mode(&self) -> u8 {
        self.sector[15]
    }

    pub fn get_data_msf(&self) -> DiscTime {
        DiscTime::new(BCD::decode(self.sector[12]),BCD::decode(self.sector[13]),BCD::decode(self.sector[14]))
    }
}

#[derive(Debug)]
pub struct Track {
    file_id:u8,
    number:u8,
    track_type:TrackType,
    start_time:DiscTime,
    end_time:DiscTime,
    pre_gap: Option<DiscTime>,
    head_position: DiscTime,
}

impl Track {
    fn get_file_id(&self) -> u8 {
        self.file_id
    }

    pub fn track_number(&self) -> u8 {
        self.number
    }
    pub fn track_type(&self) -> &TrackType {
        &self.track_type
    }
    pub fn start_time(&self) -> &DiscTime {
        &self.start_time
    }
    pub fn end_time(&self) -> &DiscTime {
        &self.end_time
    }

    fn new(file_id:u8,number:u8,track_type:TrackType,start_time:DiscTime,end_time:DiscTime,pre_gap:Option<DiscTime>) -> Self {
        Self { file_id, number, track_type, start_time, end_time, pre_gap, head_position: DiscTime::new(0,0,0) }
    }

    fn contains_msf(&self,msf:DiscTime) -> bool {
        msf >= self.start_time && msf < self.end_time
    }

    fn read_sector_into(&mut self,file:&mut File, msf:DiscTime, buffer: &mut [u8]) -> std::io::Result<bool> {
        if msf < self.start_time || msf.add(&DiscTime::new(0,0,1)) > self.end_time {
            return Ok(false);
        }

        let msf = match self.pre_gap {
            Some(pre_gap) => msf.sub(&pre_gap),
            None => msf
        };
        let offset : u64 = msf.to_lba() as u64 * SECTOR_SIZE as u64;
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(buffer)?;

        Ok(true)
    }
    fn read_next_sector(&mut self,file:&mut File, buffer: &mut [u8]) -> std::io::Result<()> {
        self.head_position = self.head_position.add(&DiscTime::FRAME_TIME);

        let offset : u64 = self.head_position.to_lba() as u64 * SECTOR_SIZE as u64;
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(buffer)?;

        Ok(())
    }
}

#[derive(Debug)]
pub struct Disc {
    tracks:Vec<Track>,
    files:Vec<(File,PathBuf)>,
}

impl Disc {
    pub fn new(cue_file_name:&String) -> Result<Self,String> {
        let cue = match std::fs::read_to_string(cue_file_name) {
            Ok(cue) => cue,
            Err(e) => return Err(format!("Failed to read cue sheet '{}': {}",cue_file_name,e))
        };
        let track_list = match cue_sheet::tracklist::Tracklist::parse(cue.as_str()) {
            Ok(tl) => tl,
            Err(e) => return Err(format!("Failed to parse cue sheet '{}': {}",cue_file_name,e))
        };

        let file_path_dir = Path::new(cue_file_name).parent().unwrap();

        let mut disc = Disc {
            tracks: Vec::new(),
            files: Vec::new(),
        };

        for track_file in track_list.files.iter() {
            let path = file_path_dir.join(&track_file.name);
            if !path.exists() {
                return Err(format!("File '{}' referenced in cue sheet '{}' does not exist",track_file.name,cue_file_name));
            }
            match File::open(path.clone()) {
                Ok(file) => disc.files.push((file,path)),
                Err(e) => return Err(format!("Failed to open file '{}': {}",path.display(),e))
            }
            let file_time = DiscTime::from_file_length(disc.files.last().unwrap().0.metadata().unwrap().len() as u32);
            let file_id = disc.files.len() as u8 - 1;
            let mut last_time = DiscTime::new(0,0,0);

            for cue_track in track_file.tracks.iter() {
                let track_type = match cue_track.track_type {
                    cue_sheet::parser::TrackType::Audio => TrackType::Audio,
                    cue_sheet::parser::TrackType::Mode(_,SECTOR_SIZE) => TrackType::Data(2,SECTOR_SIZE),
                    _ => return Err(format!("Unsupported track type {:?}",cue_track.track_type))
                };
                let mut pre_gap = cue_track.index.iter().find(|i| i.0 == 0).map(|_i| DiscTime::new(0,2,0)); // fixed to 2 seconds
                if pre_gap.is_none() {
                    if matches!(track_type, TrackType::Data(_,_) ) {
                        pre_gap = Some(DiscTime::new(0, 2, 0));
                    }
                }
                let start_time = cue_track.index.iter().find(|i| i.0 == 1).map(|i| DiscTime::new(i.1.minutes() as u8,i.1.seconds() as u8,i.1.frames() as u8)).unwrap_or(DiscTime::new(0,0,0));
                let duration = cue_track.duration.as_ref().map(|d| DiscTime::new(d.minutes() as u8,d.seconds() as u8,d.frames() as u8)).unwrap_or(file_time);
                let end_time = start_time.add(&duration);

                let track = Track::new(file_id,cue_track.number as u8,track_type,last_time.add(&start_time),end_time,pre_gap);
                disc.tracks.push(track);
                last_time = last_time.add(&end_time);
            }
        }

        Ok(disc)
    }

    fn find_track(&mut self,msf:DiscTime) -> Option<(&mut Track,&mut File,PathBuf)> {
        self.tracks.iter_mut().find(|t| t.contains_msf(msf)).map(|t| {
            let file_id = t.get_file_id() as usize;
            let file_path = self.files[file_id].1.clone();
            (t,&mut self.files[file_id].0,file_path)
        })
    }

    pub fn read_sector(&mut self,msf:DiscTime) -> Option<DataSector> {
        match self.find_track(msf) {
            Some((track,file,file_path)) => {
                info!("Reading sector {} from track {} in '{}'",msf,track.track_number(),file_path.display());
                let mut sector = DataSector::empty(msf.to_lba());
                match track.read_sector_into(file, msf, &mut sector.sector) {
                    Ok(true) => Some(sector),
                    Ok(false) => {
                        warn!("Cannot read sector {} from track {} in '{}': sector out of range",msf,track.track_number(),file_path.display());
                        None
                    },
                    Err(e) => {
                        info!("Failed to read sector {} from track {} in '{}': {}",msf,track.track_number(),file_path.display(),e);
                        None
                    }
                }
            },
            None => None
        }
    }


}