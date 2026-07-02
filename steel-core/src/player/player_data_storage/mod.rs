//! Player data storage for global and domain-specific player state.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use rustc_hash::FxHashMap;
use tokio::{fs, io};
use uuid::Uuid;

mod codec;
mod domain;
mod global;
mod known;
mod permissions;

#[cfg(test)]
mod tests;

use self::domain::{PlayerDataFile, decode_player_file, encode_player_file};
pub use self::global::GlobalPlayerData;
use self::global::{GlobalPlayerDataFile, decode_global_file, encode_global_file};
use self::known::{KnownPlayersFile, decode_known_players_file, encode_known_players_file};
pub use self::permissions::PlayerPermissionData;
use self::permissions::{
    PlayerPermissionsFile, serialize_player_permissions_file, set_player_permission_entry,
};
use crate::config::StorageSelection;
use crate::permission::{PermissionSubjectIndex, PermissionSubjectState};
use crate::player::Player;
use crate::player::known_players::KnownPlayers;
use crate::player::player_data::PersistentPlayerData;
use steel_utils::Identifier;
use steel_utils::locks::{AsyncMutex, SyncMutex};

/// Manages player data persistence.
pub struct PlayerDataStorage {
    backend: PlayerDataStorageBackend,
}

enum PlayerDataStorageBackend {
    File(FilePlayerDataStorage),
}

struct FilePlayerDataStorage {
    save_root: PathBuf,
    file_locks: SyncMutex<FxHashMap<PathBuf, Arc<AsyncMutex<()>>>>,
}

impl PlayerDataStorage {
    /// Creates player data storage from config.
    pub async fn new(save_root: PathBuf, selection: StorageSelection) -> io::Result<Self> {
        if selection.kind != Identifier::new("steel", "file") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown player storage {}", selection.kind),
            ));
        }
        let backend = PlayerDataStorageBackend::File(FilePlayerDataStorage::new(save_root).await?);
        Ok(Self { backend })
    }

    /// Saves a player's current domain data and global last-active-domain.
    pub async fn save(&self, player: &Player) -> io::Result<()> {
        let domain = player.get_world().domain().to_owned();
        self.save_domain(&domain, player).await?;
        self.update_global(player.gameprofile.id, move |global| {
            let mut global = global.unwrap_or(GlobalPlayerData {
                last_active_domain: domain.clone(),
            });
            global.last_active_domain = domain;
            Ok::<_, io::Error>(global)
        })
        .await
        .map(|_| ())
    }

    /// Saves a player's data for a specific domain.
    pub async fn save_domain(&self, domain: &str, player: &Player) -> io::Result<()> {
        match &self.backend {
            PlayerDataStorageBackend::File(storage) => storage.save_domain(domain, player).await,
        }
    }

    /// Saves an already captured player data snapshot for a specific domain.
    pub async fn save_domain_data(
        &self,
        domain: &str,
        uuid: Uuid,
        data: &PersistentPlayerData,
    ) -> io::Result<()> {
        match &self.backend {
            PlayerDataStorageBackend::File(storage) => {
                storage.save_domain_data(domain, uuid, data).await
            }
        }
    }

    /// Loads a player's data for a specific domain.
    pub async fn load_domain(
        &self,
        domain: &str,
        uuid: Uuid,
    ) -> io::Result<Option<PersistentPlayerData>> {
        match &self.backend {
            PlayerDataStorageBackend::File(storage) => storage.load_domain(domain, uuid).await,
        }
    }

    /// Loads server-wide player data.
    pub async fn load_global(&self, uuid: Uuid) -> io::Result<Option<GlobalPlayerData>> {
        match &self.backend {
            PlayerDataStorageBackend::File(storage) => storage.load_global(uuid).await,
        }
    }

    /// Loads cached permission state for all persisted player permission data.
    pub async fn load_player_permission_states(&self) -> io::Result<PermissionSubjectIndex> {
        match &self.backend {
            PlayerDataStorageBackend::File(storage) => {
                storage.load_player_permission_states().await
            }
        }
    }

    /// Loads one player's persisted permission state.
    pub async fn load_player_permissions(
        &self,
        uuid: Uuid,
    ) -> io::Result<Option<PlayerPermissionData>> {
        match &self.backend {
            PlayerDataStorageBackend::File(storage) => storage.load_player_permissions(uuid).await,
        }
    }

    /// Saves server-wide player data.
    pub async fn save_global(&self, uuid: Uuid, data: &GlobalPlayerData) -> io::Result<()> {
        match &self.backend {
            PlayerDataStorageBackend::File(storage) => storage.save_global(uuid, data).await,
        }
    }

    /// Saves server-wide player data if `is_current` still returns true while
    /// holding the destination file lock.
    pub async fn save_global_if_current(
        &self,
        uuid: Uuid,
        data: &GlobalPlayerData,
        is_current: impl FnOnce() -> bool + Send,
    ) -> io::Result<bool> {
        match &self.backend {
            PlayerDataStorageBackend::File(storage) => {
                storage.save_global_if_current(uuid, data, is_current).await
            }
        }
    }

    /// Atomically loads, updates, and saves server-wide player data while holding
    /// the destination player's global file lock.
    pub async fn update_global<F, E>(&self, uuid: Uuid, update: F) -> Result<GlobalPlayerData, E>
    where
        F: FnOnce(Option<GlobalPlayerData>) -> Result<GlobalPlayerData, E> + Send,
        E: From<io::Error>,
    {
        match &self.backend {
            PlayerDataStorageBackend::File(storage) => storage.update_global(uuid, update).await,
        }
    }

    /// Saves one player's permission data if `is_current` still returns true
    /// while holding the shared permission file lock.
    pub async fn save_player_permissions_if_current(
        &self,
        uuid: Uuid,
        data: &PlayerPermissionData,
        is_current: impl FnOnce() -> bool + Send,
    ) -> io::Result<bool> {
        match &self.backend {
            PlayerDataStorageBackend::File(storage) => {
                storage
                    .save_player_permissions_if_current(uuid, data, is_current)
                    .await
            }
        }
    }

    /// Atomically loads, updates, and saves one player's permission data while
    /// holding the shared permission file lock.
    pub async fn update_player_permissions<F, E>(
        &self,
        uuid: Uuid,
        update: F,
    ) -> Result<PlayerPermissionData, E>
    where
        F: FnOnce(Option<PlayerPermissionData>) -> Result<PlayerPermissionData, E> + Send,
        E: From<io::Error>,
    {
        match &self.backend {
            PlayerDataStorageBackend::File(storage) => {
                storage.update_player_permissions(uuid, update).await
            }
        }
    }

    /// Loads the known player index.
    pub async fn load_known_players(&self) -> io::Result<KnownPlayers> {
        match &self.backend {
            PlayerDataStorageBackend::File(storage) => storage.load_known_players().await,
        }
    }

    /// Saves the known player index.
    pub async fn save_known_players(&self, players: &KnownPlayers) -> io::Result<()> {
        match &self.backend {
            PlayerDataStorageBackend::File(storage) => storage.save_known_players(players).await,
        }
    }

    /// Saves the known player index if `is_current` still returns true while
    /// holding the destination file lock.
    pub async fn save_known_players_if_current(
        &self,
        players: &KnownPlayers,
        is_current: impl FnOnce() -> bool + Send,
    ) -> io::Result<bool> {
        match &self.backend {
            PlayerDataStorageBackend::File(storage) => {
                storage
                    .save_known_players_if_current(players, is_current)
                    .await
            }
        }
    }

    /// Saves multiple players' data.
    pub async fn save_all(&self, players: &[Arc<Player>]) -> io::Result<usize> {
        let mut saved = 0;
        for player in players {
            match self.save(player).await {
                Ok(()) => saved += 1,
                Err(e) => {
                    log::error!("Failed to save player {}: {e}", player.gameprofile.id);
                }
            }
        }
        Ok(saved)
    }
}

impl FilePlayerDataStorage {
    async fn new(save_root: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(save_root.join("global").join("players")).await?;
        Ok(Self {
            save_root,
            file_locks: SyncMutex::new(FxHashMap::default()),
        })
    }

    async fn save_domain(&self, domain: &str, player: &Player) -> io::Result<()> {
        let uuid = player.gameprofile.id;
        let data = PersistentPlayerData::from_player(player);
        self.save_domain_data(domain, uuid, &data).await
    }

    async fn save_domain_data(
        &self,
        domain: &str,
        uuid: Uuid,
        data: &PersistentPlayerData,
    ) -> io::Result<()> {
        let file = PlayerDataFile::from_persistent(data)?;
        let bytes = encode_player_file(&file)?;
        self.write_atomic(&self.domain_players_dir(domain), uuid, bytes)
            .await?;
        log::debug!("Saved player data for {uuid} in domain {domain}");
        Ok(())
    }

    async fn load_domain(
        &self,
        domain: &str,
        uuid: Uuid,
    ) -> io::Result<Option<PersistentPlayerData>> {
        let path = Self::player_file(&self.domain_players_dir(domain), uuid);
        let lock = self.file_lock(&path);
        let _guard = lock.lock().await;
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path).await?;
        let file = decode_player_file(&bytes)?;
        let data = file.into_persistent()?;
        log::debug!("Loaded player data for {uuid} in domain {domain}");
        Ok(Some(data))
    }

    async fn load_global(&self, uuid: Uuid) -> io::Result<Option<GlobalPlayerData>> {
        let path = Self::player_file(&self.global_players_dir(), uuid);
        let lock = self.file_lock(&path);
        let _guard = lock.lock().await;
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path).await?;
        let file = decode_global_file(&bytes)?;
        file.into_global_data().map(Some)
    }

    async fn load_player_permission_states(&self) -> io::Result<PermissionSubjectIndex> {
        let mut states = PermissionSubjectIndex::new();
        let file = self.load_player_permissions_file().await?;
        for (uuid, data) in file.into_player_permission_data()? {
            states.set(
                uuid,
                PermissionSubjectState::new_with_metadata(
                    data.groups,
                    data.permissions,
                    data.metadata,
                ),
            );
        }
        Ok(states)
    }

    async fn load_player_permissions(
        &self,
        uuid: Uuid,
    ) -> io::Result<Option<PlayerPermissionData>> {
        let file = self.load_player_permissions_file().await?;
        let Some(entry) = file.players.get(&uuid.to_string()) else {
            return Ok(None);
        };
        entry.clone().into_player_permission_data(uuid).map(Some)
    }

    async fn save_global(&self, uuid: Uuid, data: &GlobalPlayerData) -> io::Result<()> {
        let file = GlobalPlayerDataFile::from_global_data(data);
        let bytes = encode_global_file(&file)?;
        self.write_atomic(&self.global_players_dir(), uuid, bytes)
            .await
    }

    async fn save_global_if_current(
        &self,
        uuid: Uuid,
        data: &GlobalPlayerData,
        is_current: impl FnOnce() -> bool + Send,
    ) -> io::Result<bool> {
        let file = GlobalPlayerDataFile::from_global_data(data);
        let bytes = encode_global_file(&file)?;
        self.write_atomic_if_current(&self.global_players_dir(), uuid, bytes, is_current)
            .await
    }

    async fn update_global<F, E>(&self, uuid: Uuid, update: F) -> Result<GlobalPlayerData, E>
    where
        F: FnOnce(Option<GlobalPlayerData>) -> Result<GlobalPlayerData, E> + Send,
        E: From<io::Error>,
    {
        let players_dir = self.global_players_dir();
        fs::create_dir_all(&players_dir).await.map_err(E::from)?;
        let final_path = Self::player_file(&players_dir, uuid);
        let lock = self.file_lock(&final_path);
        let _guard = lock.lock().await;

        let current = if final_path.exists() {
            let bytes = fs::read(&final_path).await.map_err(E::from)?;
            Some(
                decode_global_file(&bytes)
                    .map_err(E::from)?
                    .into_global_data()
                    .map_err(E::from)?,
            )
        } else {
            None
        };

        let updated = update(current)?;
        let file = GlobalPlayerDataFile::from_global_data(&updated);
        let bytes = encode_global_file(&file).map_err(E::from)?;
        Self::write_atomic_locked(&players_dir, uuid, bytes)
            .await
            .map_err(E::from)?;
        Ok(updated)
    }

    async fn save_player_permissions_if_current(
        &self,
        uuid: Uuid,
        data: &PlayerPermissionData,
        is_current: impl FnOnce() -> bool + Send,
    ) -> io::Result<bool> {
        let path = self.player_permissions_file();
        let lock = self.file_lock(&path);
        let _guard = lock.lock().await;

        if !is_current() {
            return Ok(false);
        }

        let mut file = self.read_player_permissions_file_locked(&path).await?;
        set_player_permission_entry(&mut file, uuid, data);
        self.write_player_permissions_file_locked(&path, &file)
            .await?;
        Ok(true)
    }

    async fn update_player_permissions<F, E>(
        &self,
        uuid: Uuid,
        update: F,
    ) -> Result<PlayerPermissionData, E>
    where
        F: FnOnce(Option<PlayerPermissionData>) -> Result<PlayerPermissionData, E> + Send,
        E: From<io::Error>,
    {
        let path = self.player_permissions_file();
        let lock = self.file_lock(&path);
        let _guard = lock.lock().await;

        let mut file = self
            .read_player_permissions_file_locked(&path)
            .await
            .map_err(E::from)?;
        let current = match file.players.get(&uuid.to_string()) {
            Some(entry) => Some(
                entry
                    .clone()
                    .into_player_permission_data(uuid)
                    .map_err(E::from)?,
            ),
            None => None,
        };

        let updated = update(current)?;
        set_player_permission_entry(&mut file, uuid, &updated);
        self.write_player_permissions_file_locked(&path, &file)
            .await
            .map_err(E::from)?;
        Ok(updated)
    }

    async fn load_known_players(&self) -> io::Result<KnownPlayers> {
        let path = self.known_players_file();
        let lock = self.file_lock(&path);
        let _guard = lock.lock().await;
        if !path.exists() {
            return Ok(KnownPlayers::new());
        }

        let bytes = fs::read(&path).await?;
        let file = decode_known_players_file(&bytes)?;
        file.into_known_players()
    }

    async fn save_known_players(&self, players: &KnownPlayers) -> io::Result<()> {
        let file = KnownPlayersFile::from_known_players(players);
        let bytes = encode_known_players_file(&file)?;
        self.write_atomic_path(&self.known_players_file(), bytes)
            .await
    }

    async fn save_known_players_if_current(
        &self,
        players: &KnownPlayers,
        is_current: impl FnOnce() -> bool + Send,
    ) -> io::Result<bool> {
        let file = KnownPlayersFile::from_known_players(players);
        let bytes = encode_known_players_file(&file)?;
        self.write_atomic_path_if_current(&self.known_players_file(), bytes, is_current)
            .await
    }

    async fn load_player_permissions_file(&self) -> io::Result<PlayerPermissionsFile> {
        let path = self.player_permissions_file();
        let lock = self.file_lock(&path);
        let _guard = lock.lock().await;
        self.read_player_permissions_file_locked(&path).await
    }

    async fn read_player_permissions_file_locked(
        &self,
        path: &Path,
    ) -> io::Result<PlayerPermissionsFile> {
        if !path.exists() {
            return Ok(PlayerPermissionsFile::default());
        }

        let contents = fs::read_to_string(path).await?;
        let file = toml::from_str::<PlayerPermissionsFile>(&contents).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "invalid player permissions TOML in {}: {error}",
                    path.display()
                ),
            )
        })?;
        file.validate()?;
        Ok(file)
    }

    async fn write_player_permissions_file_locked(
        &self,
        path: &Path,
        file: &PlayerPermissionsFile,
    ) -> io::Result<()> {
        let contents = serialize_player_permissions_file(file).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to serialize player permissions TOML: {error}"),
            )
        })?;
        Self::write_atomic_path_locked(path, contents.into_bytes()).await
    }

    fn global_dir(&self) -> PathBuf {
        self.save_root.join("global")
    }

    fn global_players_dir(&self) -> PathBuf {
        self.global_dir().join("players")
    }

    fn known_players_file(&self) -> PathBuf {
        self.global_dir().join("known_players.dat")
    }

    fn player_permissions_file(&self) -> PathBuf {
        self.global_dir().join("player_permissions.toml")
    }

    fn domain_players_dir(&self, domain: &str) -> PathBuf {
        self.save_root.join(domain).join("players")
    }

    fn player_file(players_dir: &Path, uuid: Uuid) -> PathBuf {
        players_dir.join(format!("{uuid}.dat"))
    }

    fn temp_file(players_dir: &Path, uuid: Uuid) -> PathBuf {
        players_dir.join(format!("{uuid}.dat.tmp"))
    }

    fn backup_file(players_dir: &Path, uuid: Uuid) -> PathBuf {
        players_dir.join(format!("{uuid}.dat_old"))
    }

    fn file_lock(&self, path: &Path) -> Arc<AsyncMutex<()>> {
        let mut locks = self.file_locks.lock();
        locks
            .entry(path.to_path_buf())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    async fn write_atomic(&self, players_dir: &Path, uuid: Uuid, bytes: Vec<u8>) -> io::Result<()> {
        self.write_atomic_if_current(players_dir, uuid, bytes, || true)
            .await
            .map(|_| ())
    }

    async fn write_atomic_if_current(
        &self,
        players_dir: &Path,
        uuid: Uuid,
        bytes: Vec<u8>,
        is_current: impl FnOnce() -> bool + Send,
    ) -> io::Result<bool> {
        fs::create_dir_all(players_dir).await?;
        let final_path = Self::player_file(players_dir, uuid);
        let lock = self.file_lock(&final_path);
        let _guard = lock.lock().await;

        if !is_current() {
            return Ok(false);
        }

        Self::write_atomic_locked(players_dir, uuid, bytes).await?;
        Ok(true)
    }

    async fn write_atomic_locked(players_dir: &Path, uuid: Uuid, bytes: Vec<u8>) -> io::Result<()> {
        let temp_path = Self::temp_file(players_dir, uuid);
        let final_path = Self::player_file(players_dir, uuid);
        let backup_path = Self::backup_file(players_dir, uuid);

        fs::write(&temp_path, bytes).await?;
        if final_path.exists() {
            if backup_path.exists() {
                let _ = fs::remove_file(&backup_path).await;
            }
            fs::rename(&final_path, &backup_path).await?;
        }
        fs::rename(&temp_path, &final_path).await
    }

    async fn write_atomic_path(&self, final_path: &Path, bytes: Vec<u8>) -> io::Result<()> {
        self.write_atomic_path_if_current(final_path, bytes, || true)
            .await
            .map(|_| ())
    }

    async fn write_atomic_path_if_current(
        &self,
        final_path: &Path,
        bytes: Vec<u8>,
        is_current: impl FnOnce() -> bool + Send,
    ) -> io::Result<bool> {
        let lock = self.file_lock(final_path);
        let _guard = lock.lock().await;

        if !is_current() {
            return Ok(false);
        }

        Self::write_atomic_path_locked(final_path, bytes).await?;
        Ok(true)
    }

    async fn write_atomic_path_locked(final_path: &Path, bytes: Vec<u8>) -> io::Result<()> {
        let Some(parent) = final_path.parent() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "atomic write path has no parent",
            ));
        };
        fs::create_dir_all(parent).await?;

        let extension = final_path
            .extension()
            .and_then(|extension| extension.to_str());
        let temp_path = final_path.with_extension(match extension {
            Some(extension) => format!("{extension}.tmp"),
            None => "tmp".to_owned(),
        });
        let backup_path = final_path.with_extension(match extension {
            Some(extension) => format!("{extension}_old"),
            None => "old".to_owned(),
        });

        fs::write(&temp_path, bytes).await?;
        if final_path.exists() {
            if backup_path.exists() {
                let _ = fs::remove_file(&backup_path).await;
            }
            fs::rename(final_path, &backup_path).await?;
        }
        fs::rename(&temp_path, final_path).await?;
        Ok(())
    }
}
