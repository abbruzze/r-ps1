use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;
use tracing::{error, info};

pub trait SnapshotAware {
    type State: Serialize + for<'de> Deserialize<'de>;

    fn snapshot(&self) -> Self::State;
    fn restore(&mut self, state: Self::State);
}

pub struct SnapshotManager {
    slot: u8,
    slot_path: PathBuf,
}

impl SnapshotManager {
    pub fn new(slot_path: PathBuf) -> Self {
        match std::fs::create_dir_all(slot_path.clone()) {
            Err(e) => error!("Cannot create snapshot directory: {}", e),
            Ok(_) => ()
        }
        Self {
            slot: 0,
            slot_path,
        }
    }

    pub fn set_slot(&mut self,slot:u8) {
        if slot > 9 {
            self.slot = slot;
            info!("Snapshot slot set to {}", slot);
        }
    }

    pub fn get_slot(&self) -> u8 {
        self.slot
    }

    pub fn save_state<T: SnapshotAware>(&self, state: &T) -> Result<(),String> {
        let start = Instant::now();
        let path = self.slot_path.join(format!("slot_{}", self.slot));
        let file = std::fs::File::create(path.clone()).map_err(|e| format!("Cannot create snapshot file: {}", e))?;

        let encoded = bincode::serialize(&state.snapshot()).map_err(|e| format!("Serialization error: {}", e))?;
        let mut encoder = GzEncoder::new(file, Compression::fast());
        encoder.write_all(&encoded).map_err(|e| format!("Write error: {}", e))?;
        encoder.finish().map_err(|e| format!("Compression error: {}", e))?;
        info!("State saved to {:?} ({} bytes compressed) in {} millis", path, encoded.len(), start.elapsed().as_millis());
        Ok(())
    }

    pub fn load_state<T: Serialize + for<'de> Deserialize<'de>>(&self) -> Result<T,String> {
        let start = Instant::now();
        let path = self.slot_path.join(format!("slot_{}", self.slot));
        let file = std::fs::File::open(path).map_err(|e| format!("Cannot open snapshot file: {}", e))?;
        let decoder = GzDecoder::new(file);
        let decoded: T = bincode::deserialize_from(decoder).map_err(|e| format!("Deserialization error: {}", e))?;

        info!("State loaded from {:?} in {} millis", self.slot_path, start.elapsed().as_millis());
        Ok(decoded)
    }
}