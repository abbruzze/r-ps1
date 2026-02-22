use std::cmp::Ordering;
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use tracing::{info, warn};
use crate::core::cdrom::{cue, util, Region};

pub(super) const SECTOR_SIZE : u16 = 2352;

#[derive(Copy,Clone,Debug)]
pub struct DiscTime {
    minutes:u8, // 00 - 99
    seconds:u8, // 00 - 59
    frames:u8 // 00 - 74
}

impl DiscTime {
    const ZERO_TIME : DiscTime = DiscTime { minutes: 0, seconds: 0, frames: 0 };
    const FRAME_TIME : DiscTime = DiscTime { minutes: 0, seconds: 0, frames: 1 };
    const _2_SEC_TIME : DiscTime = DiscTime { minutes: 0, seconds: 2, frames: 0 };

    pub fn new(minutes:u8,seconds:u8,frames:u8) -> Self {
        Self { minutes, seconds, frames }
    }

    pub fn new_checked(minutes:u8,seconds:u8,frames:u8) -> Option<Self> {
        if minutes < 80 && seconds < 60 && frames < 75 {
            Some(Self::new(minutes,seconds,frames))
        } else {
            None
        }
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

pub struct BCD {}
impl BCD {
    pub fn decode(value:u8) -> u8 {
        ((value >> 4) * 10) + (value & 0x0F)
    }
    pub fn encode(value:u8) -> u8 {
        ((value / 10) << 4) + (value % 10)
    }
    pub fn is_valid(value:u8) -> bool {
        value <= 0x99
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
    pre_gap: DiscTime,
    post_gap: DiscTime,
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
    pub fn duration(&self) -> DiscTime {
        self.end_time.sub(&self.start_time).add(&self.post_gap).add(&self.pre_gap)
    }

    fn new(file_id:u8,number:u8,track_type:TrackType,start_time:DiscTime,end_time:DiscTime) -> Self {
        let (pre_gap,post_gap) = match track_type {
            TrackType::Audio => (DiscTime::ZERO_TIME,DiscTime::ZERO_TIME),
            TrackType::Data(_,_) => (DiscTime::_2_SEC_TIME,DiscTime::_2_SEC_TIME)
        };
        Self { file_id, number, track_type, start_time: start_time.add(&pre_gap), end_time: end_time.add(&pre_gap),pre_gap, post_gap }
    }

    fn contains_msf(&self,msf:DiscTime) -> bool {
        msf >= self.start_time && msf < self.end_time
    }

    fn read_sector_into(&mut self,file:&mut File, msf:DiscTime, buffer: &mut [u8]) -> std::io::Result<bool> {
        if msf < self.start_time || msf > self.end_time {
            // TODO: read fake sector
            todo!("read fake sector");
            return Ok(false);
        }

        let offset : u64 = (msf.to_lba() - self.start_time.to_lba()) as u64 * SECTOR_SIZE as u64;
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(buffer)?;

        Ok(true)
    }
}

#[derive(Debug)]
pub struct Disc {
    cue_file_name:String,
    tracks:Vec<Track>,
    files:Vec<(File,PathBuf)>,
    region: Option<Region>,
    head_position: DiscTime,
}

impl Disc {
    pub fn new(cue_file_name:&String) -> Result<Self,String> {
        let cue = match cue::parse_cue(cue_file_name) {
            Ok(cue) => cue,
            Err(e) => return Err(format!("Failed to parse cue sheet '{}': {}",cue_file_name,e))
        };
        println!("Cue {:?}",cue);

        let file_path_dir = Path::new(cue_file_name).parent().unwrap();

        let mut disc = Disc {
            cue_file_name: cue_file_name.clone(),
            tracks: Vec::new(),
            files: Vec::new(),
            region: None,
            head_position: DiscTime::new(0,0,0),
        };

        let mut last_time = DiscTime::new(0,0,0);
        for track_file in cue.files.iter() {
            let path = file_path_dir.join(&track_file.path);
            if !path.exists() {
                return Err(format!("File '{:?}' referenced in cue sheet '{}' does not exist",track_file.path,cue_file_name));
            }
            match File::open(path.clone()) {
                Ok(file) => disc.files.push((file,path)),
                Err(e) => return Err(format!("Failed to open file '{}': {}",path.display(),e))
            }
            let file_time = DiscTime::from_file_length(disc.files.last().unwrap().0.metadata().unwrap().len() as u32);
            let file_id = disc.files.len() as u8 - 1;

            for cue_track in track_file.tracks.iter() {
                let track_type = match cue_track.track_type {
                    cue::TrackType::Audio => TrackType::Audio,
                    cue::TrackType::Mode1_2352  => TrackType::Data(1,SECTOR_SIZE),
                    cue::TrackType::Mode2_2352  => TrackType::Data(2,SECTOR_SIZE),
                    _ => return Err(format!("Unsupported track type {:?}",cue_track.track_type))
                };

                let track_start_time = cue_track.indices.iter().find(|i| i.number == 1).map(|i| DiscTime::new(i.time.minute,i.time.second,i.time.frame)).unwrap_or(DiscTime::new(0,0,0));
                let duration = file_time; // suppose one track per file only
                let track_end_time = track_start_time.add(&duration);

                let track = Track::new(file_id,cue_track.number,track_type,last_time.add(&track_start_time),last_time.add(&track_end_time));
                last_time = last_time.add(&track.duration());
                disc.tracks.push(track);
            }
        }

        // try to extract region from track 1
        if let Some(track) = disc.tracks.get(0) && matches!(track.track_type,TrackType::Data(_,_) ) {
            if let Some((_,file_path)) = disc.files.get(0) {
                disc.region = util::get_cd_region(file_path.as_os_str().to_str().unwrap());
            }
        }

        info!("Disc '{}' info:",cue_file_name);
        disc.tracks.iter().for_each(|t| info!("  Track [{:?}] {}: {} - {}",t.track_type(),t.track_number(),t.start_time(),t.end_time()));

        Ok(disc)
    }

    fn find_track(&mut self,msf:DiscTime) -> Option<(&mut Track,&mut File,PathBuf)> {
        self.tracks.iter_mut().find(|t| t.contains_msf(msf)).map(|t| {
            let file_id = t.get_file_id() as usize;
            let file_path = self.files[file_id].1.clone();
            (t,&mut self.files[file_id].0,file_path)
        })
    }

    pub fn get_cue_file_name(&self) -> &String {
        &self.cue_file_name
    }

    pub fn get_region(&self) -> Option<Region> {
        self.region
    }

    pub fn is_audio_cd(&self) -> bool {
        if let Some(track_1) = self.tracks.get(0) {
            matches!(track_1.track_type,TrackType::Audio)
        } else {
            false
        }
    }

    pub fn read_sector(&mut self) -> Option<DataSector> {
        let msf = self.head_position;

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

    pub fn seek_sector(&mut self,msf:DiscTime) {
        self.head_position = msf;
    }

    pub fn get_head_position(&self) -> DiscTime {
        self.head_position
    }

    pub fn set_next_sector_head_position(&mut self) {
        self.head_position = self.head_position.add(&DiscTime::FRAME_TIME);
    }

    pub fn get_tracks(&self) -> &[Track] {
        &self.tracks
    }

    // 0 means last track
    pub fn get_track_by_number(&self,number:u8) -> Option<&Track> {
        if number == 0 {
            return self.tracks.last();
        }
        self.tracks.iter().find(|t| t.track_number() == number)
    }
}