use tokio::io;
use wincode::{SchemaRead, SchemaWrite};

use super::codec::{decode_file, encode_file};

const GLOBAL_MAGIC: [u8; 4] = *b"STLG";
pub(super) const GLOBAL_STORAGE_VERSION: u16 = 7;
pub(super) const GLOBAL_PLAYER_DATA_VERSION: i32 = 7;

/// Server-wide player data.
#[derive(Debug, Clone)]
pub struct GlobalPlayerData {
    /// Last active domain for reconnects.
    pub last_active_domain: String,
}

#[derive(SchemaWrite, SchemaRead)]
pub(super) struct GlobalPlayerDataFile {
    pub(super) data_version: i32,
    pub(super) last_active_domain: String,
}

impl GlobalPlayerDataFile {
    pub(super) fn from_global_data(data: &GlobalPlayerData) -> Self {
        Self {
            data_version: GLOBAL_PLAYER_DATA_VERSION,
            last_active_domain: data.last_active_domain.clone(),
        }
    }

    pub(super) fn into_global_data(self) -> io::Result<GlobalPlayerData> {
        if self.data_version != GLOBAL_PLAYER_DATA_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported global player data payload version {}",
                    self.data_version
                ),
            ));
        }

        Ok(GlobalPlayerData {
            last_active_domain: self.last_active_domain,
        })
    }
}

pub(super) fn encode_global_file(file: &GlobalPlayerDataFile) -> io::Result<Vec<u8>> {
    encode_file(
        GLOBAL_MAGIC,
        GLOBAL_STORAGE_VERSION,
        wincode::serialize(file),
    )
}

pub(super) fn decode_global_file(bytes: &[u8]) -> io::Result<GlobalPlayerDataFile> {
    let payload = decode_file(GLOBAL_MAGIC, GLOBAL_STORAGE_VERSION, bytes)?;
    wincode::deserialize(&payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
}
