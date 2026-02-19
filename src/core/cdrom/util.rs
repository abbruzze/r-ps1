use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Result};
use regex::Regex;
use crate::core::cdrom::Region;

const SECTOR_SIZE: u64 = 2352;
const USER_DATA_OFFSET: u64 = 24;
const USER_DATA_SIZE: usize = 2048;

fn read_sector(file: &mut File, lba: u32, buffer: &mut [u8; USER_DATA_SIZE]) -> Result<()> {
    let offset = (lba as u64) * SECTOR_SIZE + USER_DATA_OFFSET;
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(buffer)?;
    Ok(())
}

/*
SxPx - Japan (NTSC)
SxEx - Europe (PAL)
SxUx - USA (NTSC)
 */
pub fn get_cd_region(disc_path: &str) -> Option<Region> {
    match read_system_cnf(disc_path) {
        Ok(cnf) => {
            let boot_re = Regex::new(r"\s*BOOT\s*=\s*cdrom:\\(.*);.*").unwrap();
            match boot_re.captures(cnf.as_str()) {
                Some(file) => {
                    let upper = file[1].to_uppercase();
                    let file = upper.as_bytes();
                    if file.len() < 4 || file[0] != b'S' {
                        return None;
                    }
                    match file[2] {
                        b'P' => Some(Region::Japan),
                        b'U' => Some(Region::USA),
                        b'E' => Some(Region::Europe),
                        _ => None
                    }
                }
                None => None
            }
        }
        Err(_) => None,
    }
}

pub fn read_system_cnf(path: &str) -> Result<String> {
    let mut file = File::open(path)?;

    // Step 1: Read Primary Volume Descriptor (sector 16)
    let mut sector = [0u8; USER_DATA_SIZE];
    read_sector(&mut file, 16, &mut sector)?;

    // Root directory record starts at offset 156
    let root_dir_lba = u32::from_le_bytes([
        sector[158], sector[159], sector[160], sector[161]
    ]);

    let root_dir_size = u32::from_le_bytes([
        sector[166], sector[167], sector[168], sector[169]
    ]);

    // Step 2: Read root directory
    let num_sectors = (root_dir_size as usize + USER_DATA_SIZE - 1) / USER_DATA_SIZE;

    let mut dir_data = vec![0u8; num_sectors * USER_DATA_SIZE];

    for i in 0..num_sectors {
        let mut buf = [0u8; USER_DATA_SIZE];
        read_sector(&mut file, root_dir_lba + i as u32, &mut buf)?;
        dir_data[i * USER_DATA_SIZE..(i + 1) * USER_DATA_SIZE].copy_from_slice(&buf);
    }

    // Step 3: Find SYSTEM.CNF;1
    let mut offset = 0;

    while offset < root_dir_size as usize {
        let length = dir_data[offset] as usize;

        if length == 0 {
            offset += 1;
            continue;
        }

        let lba = u32::from_le_bytes([
            dir_data[offset + 2],
            dir_data[offset + 3],
            dir_data[offset + 4],
            dir_data[offset + 5],
        ]);

        let size = u32::from_le_bytes([
            dir_data[offset + 10],
            dir_data[offset + 11],
            dir_data[offset + 12],
            dir_data[offset + 13],
        ]);

        let name_len = dir_data[offset + 32] as usize;

        let name = &dir_data[offset + 33..offset + 33 + name_len];

        if name == b"SYSTEM.CNF;1" {
            // Found it
            let num_sectors = (size as usize + USER_DATA_SIZE - 1) / USER_DATA_SIZE;
            let mut file_data = vec![0u8; num_sectors * USER_DATA_SIZE];

            for i in 0..num_sectors {
                let mut buf = [0u8; USER_DATA_SIZE];
                read_sector(&mut file, lba + i as u32, &mut buf)?;
                file_data[i * USER_DATA_SIZE..(i + 1) * USER_DATA_SIZE]
                    .copy_from_slice(&buf);
            }

            file_data.truncate(size as usize);

            return Ok(String::from_utf8_lossy(&file_data).to_string());
        }

        offset += length;
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "SYSTEM.CNF not found",
    ))
}
