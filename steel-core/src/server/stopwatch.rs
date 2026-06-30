//! Server stopwatch state used by `/stopwatch` and `execute if stopwatch`.

use std::{
    collections::HashMap,
    io::Cursor,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use steel_utils::Identifier;
use tokio::{fs, io};
use wincode::{SchemaRead, SchemaWrite};

const STOPWATCH_MAGIC: [u8; 4] = *b"STSW";
const STOPWATCH_STORAGE_VERSION: u16 = 1;
const STOPWATCH_DATA_VERSION: i32 = 1;

/// One running stopwatch.
#[derive(Clone, Copy, Debug)]
pub struct Stopwatch {
    creation_time: Instant,
    accumulated_elapsed: Duration,
}

impl Stopwatch {
    /// Creates a stopwatch starting at the current monotonic time.
    #[must_use]
    pub fn started_now() -> Self {
        Self::started_at(Instant::now())
    }

    /// Creates a stopwatch starting at `creation_time`.
    #[must_use]
    pub const fn started_at(creation_time: Instant) -> Self {
        Self {
            creation_time,
            accumulated_elapsed: Duration::ZERO,
        }
    }

    const fn from_accumulated_at(creation_time: Instant, accumulated_elapsed: Duration) -> Self {
        Self {
            creation_time,
            accumulated_elapsed,
        }
    }

    /// Returns the elapsed duration at the current monotonic time.
    #[must_use]
    pub fn elapsed(self) -> Duration {
        self.elapsed_at(Instant::now())
    }

    /// Returns the elapsed duration at `current_time`.
    #[must_use]
    pub fn elapsed_at(self, current_time: Instant) -> Duration {
        self.accumulated_elapsed + current_time.saturating_duration_since(self.creation_time)
    }

    /// Returns the elapsed seconds at the current monotonic time.
    #[must_use]
    pub fn elapsed_seconds(self) -> f64 {
        self.elapsed().as_secs_f64()
    }

    fn elapsed_millis_at(self, current_time: Instant) -> u64 {
        u64::try_from(self.elapsed_at(current_time).as_millis()).unwrap_or(u64::MAX)
    }
}

/// Server stopwatch collection.
#[derive(Debug)]
pub struct Stopwatches {
    path: PathBuf,
    stopwatches: HashMap<Identifier, Stopwatch>,
    dirty: bool,
    generation: u64,
}

impl Stopwatches {
    /// Loads stopwatches from the server save root.
    ///
    /// # Errors
    ///
    /// Returns an I/O or decode error when an existing stopwatch file is malformed.
    pub async fn load(save_root: impl AsRef<Path>) -> io::Result<Self> {
        let path = stopwatch_file(save_root.as_ref());
        if !path.exists() {
            return Ok(Self::empty_at_path(path));
        }

        let bytes = fs::read(&path).await?;
        let file = decode_stopwatches_file(&bytes)?;
        let now = Instant::now();
        let stopwatches = file
            .entries
            .into_iter()
            .map(|entry| {
                (
                    entry.id,
                    Stopwatch::from_accumulated_at(
                        now,
                        Duration::from_millis(entry.accumulated_elapsed_millis),
                    ),
                )
            })
            .collect();

        Ok(Self {
            path,
            stopwatches,
            dirty: false,
            generation: 0,
        })
    }

    /// Creates an empty stopwatch collection for `save_root`.
    #[must_use]
    pub fn empty(save_root: impl AsRef<Path>) -> Self {
        Self::empty_at_path(stopwatch_file(save_root.as_ref()))
    }

    fn empty_at_path(path: PathBuf) -> Self {
        Self {
            path,
            stopwatches: HashMap::new(),
            dirty: false,
            generation: 0,
        }
    }

    /// Returns one stopwatch by id.
    #[must_use]
    pub fn get(&self, id: &Identifier) -> Option<Stopwatch> {
        self.stopwatches.get(id).copied()
    }

    /// Adds a stopwatch.
    pub fn add(&mut self, id: Identifier, stopwatch: Stopwatch) -> bool {
        if self.stopwatches.contains_key(&id) {
            return false;
        }

        self.stopwatches.insert(id, stopwatch);
        self.mark_dirty();
        true
    }

    /// Creates a stopwatch starting now.
    pub fn create(&mut self, id: Identifier) -> bool {
        self.add(id, Stopwatch::started_now())
    }

    /// Restarts an existing stopwatch.
    pub fn restart(&mut self, id: &Identifier) -> bool {
        let Some(stopwatch) = self.stopwatches.get_mut(id) else {
            return false;
        };

        *stopwatch = Stopwatch::started_now();
        self.mark_dirty();
        true
    }

    /// Removes an existing stopwatch.
    pub fn remove(&mut self, id: &Identifier) -> bool {
        let removed = self.stopwatches.remove(id).is_some();
        if removed {
            self.mark_dirty();
        }
        removed
    }

    /// Returns known stopwatch ids in deterministic order.
    #[must_use]
    pub fn ids(&self) -> Vec<Identifier> {
        let mut ids = self.stopwatches.keys().cloned().collect::<Vec<_>>();
        ids.sort_by_key(ToString::to_string);
        ids
    }

    /// Builds a save snapshot if vanilla would consider this saved data dirty.
    #[must_use]
    pub fn prepare_save(&self) -> Option<StopwatchesSave> {
        if !self.dirty && self.stopwatches.is_empty() {
            return None;
        }

        let now = Instant::now();
        let mut entries = self
            .stopwatches
            .iter()
            .map(|(id, stopwatch)| StopwatchFileEntry {
                id: id.clone(),
                accumulated_elapsed_millis: stopwatch.elapsed_millis_at(now),
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.id.to_string());

        Some(StopwatchesSave {
            generation: self.generation,
            path: self.path.clone(),
            file: StopwatchesFile {
                data_version: STOPWATCH_DATA_VERSION,
                entries,
            },
        })
    }

    /// Marks a previously prepared save as written if no newer mutation happened.
    pub fn mark_saved(&mut self, generation: u64) {
        if self.generation == generation {
            self.dirty = false;
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
        self.generation = self.generation.wrapping_add(1);
    }
}

/// Prepared stopwatch save snapshot.
#[derive(Debug)]
pub struct StopwatchesSave {
    generation: u64,
    path: PathBuf,
    file: StopwatchesFile,
}

impl StopwatchesSave {
    /// Generation captured when the save was prepared.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Writes this snapshot to disk.
    ///
    /// # Errors
    ///
    /// Returns an I/O or encode error when the snapshot cannot be persisted.
    pub async fn write(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let bytes = encode_stopwatches_file(&self.file)?;
        fs::write(&self.path, bytes).await
    }
}

#[derive(Clone, Debug, SchemaWrite, SchemaRead)]
struct StopwatchesFile {
    data_version: i32,
    entries: Vec<StopwatchFileEntry>,
}

#[derive(Clone, Debug, SchemaWrite, SchemaRead)]
struct StopwatchFileEntry {
    id: Identifier,
    accumulated_elapsed_millis: u64,
}

fn stopwatch_file(save_root: &Path) -> PathBuf {
    save_root
        .join("data")
        .join("minecraft")
        .join("stopwatches.dat")
}

fn encode_stopwatches_file(file: &StopwatchesFile) -> io::Result<Vec<u8>> {
    encode_file(
        STOPWATCH_MAGIC,
        STOPWATCH_STORAGE_VERSION,
        wincode::serialize(file),
    )
}

fn decode_stopwatches_file(bytes: &[u8]) -> io::Result<StopwatchesFile> {
    let payload = decode_file(STOPWATCH_MAGIC, STOPWATCH_STORAGE_VERSION, bytes)?;
    let file: StopwatchesFile = wincode::deserialize(&payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    if file.data_version != STOPWATCH_DATA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported stopwatch data version {}", file.data_version),
        ));
    }
    Ok(file)
}

fn encode_file(
    magic: [u8; 4],
    version: u16,
    serialized: wincode::WriteResult<Vec<u8>>,
) -> io::Result<Vec<u8>> {
    let payload =
        serialized.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    let compressed = zstd::encode_all(&payload[..], 3)?;
    let mut bytes = Vec::with_capacity(6 + compressed.len());
    bytes.extend_from_slice(&magic);
    bytes.extend_from_slice(&version.to_le_bytes());
    bytes.extend_from_slice(&compressed);
    Ok(bytes)
}

fn decode_file(
    expected_magic: [u8; 4],
    expected_version: u16,
    bytes: &[u8],
) -> io::Result<Vec<u8>> {
    if bytes.len() < 6 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "stopwatch file is too short",
        ));
    }
    if bytes[0..4] != expected_magic {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid stopwatch file magic",
        ));
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != expected_version {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported stopwatch storage version {version}"),
        ));
    }
    zstd::decode_all(Cursor::new(&bytes[6..]))
}

#[cfg(test)]
mod tests {
    use std::{
        fs as std_fs,
        path::PathBuf,
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn temp_save_root(test_name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "steel-stopwatches-{test_name}-{}-{unique}",
            process::id()
        ));
        std_fs::create_dir_all(&path).expect("temp save root should be created");
        path
    }

    #[test]
    fn stopwatch_elapsed_includes_accumulated_time() {
        let now = Instant::now();
        let started = now
            .checked_sub(Duration::from_millis(250))
            .expect("checked subtraction should stay in range");
        let stopwatch = Stopwatch::from_accumulated_at(started, Duration::from_millis(750));

        assert_eq!(stopwatch.elapsed_at(now), Duration::from_millis(1000));
    }

    #[test]
    fn collection_mutations_update_ids_and_dirty_generation() {
        let root = temp_save_root("mutations");
        let mut stopwatches = Stopwatches::empty(&root);
        let id = Identifier::new_static("minecraft", "test");

        assert!(stopwatches.add(id.clone(), Stopwatch::started_now()));
        assert!(!stopwatches.add(id.clone(), Stopwatch::started_now()));
        assert_eq!(stopwatches.ids(), vec![id.clone()]);

        let save = stopwatches
            .prepare_save()
            .expect("dirty collection should prepare save");
        stopwatches.mark_saved(save.generation());
        assert!(!stopwatches.dirty);

        assert!(stopwatches.restart(&id));
        assert!(stopwatches.dirty);
        assert!(stopwatches.remove(&id));
        assert!(stopwatches.ids().is_empty());

        let _ = std_fs::remove_dir_all(root);
    }

    #[test]
    fn non_empty_clean_collection_still_prepares_save() {
        let root = temp_save_root("clean-non-empty");
        let mut stopwatches = Stopwatches::empty(&root);
        let id = Identifier::new_static("minecraft", "test");

        assert!(stopwatches.add(id, Stopwatch::started_now()));
        let save = stopwatches
            .prepare_save()
            .expect("dirty collection should prepare save");
        stopwatches.mark_saved(save.generation());

        assert!(
            stopwatches.prepare_save().is_some(),
            "vanilla keeps non-empty stopwatches dirty enough to refresh elapsed time on save"
        );

        let _ = std_fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn load_missing_file_returns_empty_collection() {
        let root = temp_save_root("missing");

        let stopwatches = Stopwatches::load(&root)
            .await
            .expect("missing stopwatch file should load");

        assert!(stopwatches.ids().is_empty());
        assert!(stopwatches.prepare_save().is_none());

        let _ = std_fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn save_and_load_preserves_elapsed_time() {
        let root = temp_save_root("roundtrip");
        let mut stopwatches = Stopwatches::empty(&root);
        let id = Identifier::new_static("minecraft", "test");
        let now = Instant::now();
        let started = now
            .checked_sub(Duration::from_millis(1250))
            .expect("checked subtraction should stay in range");

        assert!(stopwatches.add(id.clone(), Stopwatch::started_at(started)));
        let save = stopwatches
            .prepare_save()
            .expect("dirty collection should prepare save");
        save.write()
            .await
            .expect("stopwatch snapshot should write to disk");
        stopwatches.mark_saved(save.generation());

        let loaded = Stopwatches::load(&root)
            .await
            .expect("saved stopwatch file should load");
        let loaded_elapsed = loaded
            .get(&id)
            .expect("saved stopwatch should exist")
            .elapsed_at(Instant::now());

        assert!(loaded_elapsed >= Duration::from_millis(1250));

        let _ = std_fs::remove_dir_all(root);
    }
}
