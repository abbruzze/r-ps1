use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CueSheet {
    pub files: Vec<CueFile>,
}

#[derive(Debug, Clone)]
pub struct CueFile {
    pub path: PathBuf,
    pub file_type: CueFileType,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone)]
pub enum CueFileType {
    Binary,
    Wave,
    Mp3,
    Unknown(String),
}

#[derive(Debug, Clone)]
pub struct Track {
    pub number: u8,
    pub track_type: TrackType,
    pub indices: Vec<Index>,
}

#[derive(Debug, Clone)]
pub enum TrackType {
    Mode1_2352,
    Mode2_2352,
    Audio,
    Unknown(String),
}

#[derive(Debug, Clone)]
pub struct Index {
    pub number: u8,
    pub time: Msf,
}

#[derive(Debug, Clone, Copy)]
pub struct Msf {
    pub minute: u8,
    pub second: u8,
    pub frame: u8,
}

pub fn parse_cue<P: AsRef<Path>>(path: P) -> std::io::Result<CueSheet> {
    let file = File::open(&path)?;
    let reader = BufReader::new(file);

    let base_dir = path.as_ref().parent().unwrap_or(Path::new(""));

    let mut cue = CueSheet { files: Vec::new() };

    let mut current_file: Option<CueFile> = None;
    let mut current_track: Option<Track> = None;

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        let parts = split_cue_line(line);

        if parts.is_empty() {
            continue;
        }

        match parts[0].as_str() {
            "FILE" => {
                if let Some(track) = current_track.take() {
                    if let Some(file) = current_file.as_mut() {
                        file.tracks.push(track);
                    }
                }

                if let Some(file) = current_file.take() {
                    cue.files.push(file);
                }

                let filename = parts[1].clone();
                let file_type = parse_file_type(&parts[2]);

                current_file = Some(CueFile {
                    path: base_dir.join(filename),
                    file_type,
                    tracks: Vec::new(),
                });
            }

            "TRACK" => {
                if let Some(track) = current_track.take() {
                    if let Some(file) = current_file.as_mut() {
                        file.tracks.push(track);
                    }
                }

                let number = parts[1].parse().unwrap_or(0);
                let track_type = parse_track_type(&parts[2]);

                current_track = Some(Track {
                    number,
                    track_type,
                    indices: Vec::new(),
                });
            }

            "INDEX" => {
                if let Some(track) = current_track.as_mut() {
                    let number = parts[1].parse().unwrap_or(0);
                    let time = parse_msf(&parts[2]);

                    track.indices.push(Index {
                        number,
                        time,
                    });
                }
            }

            _ => {}
        }
    }

    if let Some(track) = current_track {
        if let Some(file) = current_file.as_mut() {
            file.tracks.push(track);
        }
    }

    if let Some(file) = current_file {
        cue.files.push(file);
    }

    Ok(cue)
}

fn split_cue_line(line: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for c in line.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
            }
            ' ' if !in_quotes => {
                if !current.is_empty() {
                    result.push(current.clone());
                    current.clear();
                }
            }
            _ => current.push(c),
        }
    }

    if !current.is_empty() {
        result.push(current);
    }

    result
}

fn parse_file_type(s: &str) -> CueFileType {
    match s {
        "BINARY" => CueFileType::Binary,
        "WAVE" => CueFileType::Wave,
        "MP3" => CueFileType::Mp3,
        _ => CueFileType::Unknown(s.to_string()),
    }
}

fn parse_track_type(s: &str) -> TrackType {
    match s {
        "MODE1/2352" => TrackType::Mode1_2352,
        "MODE2/2352" => TrackType::Mode2_2352,
        "AUDIO" => TrackType::Audio,
        _ => TrackType::Unknown(s.to_string()),
    }
}

fn parse_msf(s: &str) -> Msf {
    let parts: Vec<&str> = s.split(':').collect();

    Msf {
        minute: parts.get(0).and_then(|x| x.parse().ok()).unwrap_or(0),
        second: parts.get(1).and_then(|x| x.parse().ok()).unwrap_or(0),
        frame: parts.get(2).and_then(|x| x.parse().ok()).unwrap_or(0),
    }
}
