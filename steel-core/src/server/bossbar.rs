//! Server custom boss bar state used by `/bossbar` and `execute store ... bossbar`.

use std::{
    collections::{HashMap, HashSet},
    io::Cursor,
    path::{Path, PathBuf},
};

use simdnbt::{ToNbtTag, borrow::read_compound as read_borrowed_compound, owned::NbtCompound};
use steel_protocol::packets::game::{BossBarColor, BossBarOverlay, BossBarProperties, CBossEvent};
use steel_utils::Identifier;
use text_components::TextComponent;
use tokio::{fs, io};
use uuid::Uuid;
use wincode::{SchemaRead, SchemaWrite};

const BOSS_BAR_MAGIC: [u8; 4] = *b"STBB";
const BOSS_BAR_STORAGE_VERSION: u16 = 1;
const BOSS_BAR_DATA_VERSION: i32 = 1;

/// Packet fanout produced by one boss bar mutation.
#[derive(Clone, Debug)]
pub struct BossBarBroadcast {
    /// Player UUIDs that should receive this packet if currently online.
    pub recipients: Vec<Uuid>,
    /// Packet to send.
    pub packet: CBossEvent,
}

/// Public snapshot of one custom boss bar.
#[derive(Clone, Debug, PartialEq)]
pub struct BossBarSnapshot {
    /// Persistent boss bar id.
    pub id: Identifier,
    /// Runtime id used by the client protocol.
    pub runtime_id: Uuid,
    /// Display name.
    pub name: TextComponent,
    /// Whether this bar is visible to members.
    pub visible: bool,
    /// Current value.
    pub value: i32,
    /// Maximum value.
    pub max: i32,
    /// Bar color.
    pub color: BossBarColor,
    /// Bar overlay.
    pub overlay: BossBarOverlay,
    /// Screen effect properties.
    pub properties: BossBarProperties,
    /// Persisted player membership.
    pub players: Vec<Uuid>,
}

/// Result of a boss bar mutation.
#[derive(Clone, Debug)]
pub struct BossBarMutation {
    /// Snapshot after the attempted mutation.
    pub snapshot: BossBarSnapshot,
    /// Whether persisted state changed.
    pub changed: bool,
    /// Packets to send after the lock is released.
    pub broadcasts: Vec<BossBarBroadcast>,
}

/// Boss bar lookup or creation error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BossBarError {
    /// A bar already exists.
    AlreadyExists(Identifier),
    /// A bar does not exist.
    Missing(Identifier),
}

#[derive(Clone, Debug)]
struct BossBar {
    runtime_id: Uuid,
    id: Identifier,
    name: TextComponent,
    visible: bool,
    value: i32,
    max: i32,
    color: BossBarColor,
    overlay: BossBarOverlay,
    properties: BossBarProperties,
    players: HashSet<Uuid>,
}

impl BossBar {
    fn new(id: Identifier, name: TextComponent) -> Self {
        Self {
            runtime_id: Uuid::new_v4(),
            id,
            name,
            visible: true,
            value: 0,
            max: 100,
            color: BossBarColor::White,
            overlay: BossBarOverlay::Progress,
            properties: BossBarProperties::default(),
            players: HashSet::new(),
        }
    }

    fn snapshot(&self) -> BossBarSnapshot {
        let mut players = self.players.iter().copied().collect::<Vec<_>>();
        players.sort_by_key(|uuid| *uuid.as_bytes());
        BossBarSnapshot {
            id: self.id.clone(),
            runtime_id: self.runtime_id,
            name: self.name.clone(),
            visible: self.visible,
            value: self.value,
            max: self.max,
            color: self.color,
            overlay: self.overlay,
            properties: self.properties,
            players,
        }
    }

    fn progress(&self) -> f32 {
        clamp_progress(self.value, self.max)
    }

    fn add_packet(&self) -> CBossEvent {
        CBossEvent::add(
            self.runtime_id,
            self.name.clone(),
            self.progress(),
            self.color,
            self.overlay,
            self.properties,
        )
    }

    fn visible_broadcast(&self, packet: CBossEvent) -> Vec<BossBarBroadcast> {
        if !self.visible || self.players.is_empty() {
            return Vec::new();
        }

        vec![BossBarBroadcast {
            recipients: sorted_uuids(&self.players),
            packet,
        }]
    }
}

/// Server custom boss bar collection.
#[derive(Debug)]
pub struct BossBars {
    path: PathBuf,
    bars: HashMap<Identifier, BossBar>,
    dirty: bool,
    generation: u64,
}

impl BossBars {
    /// Loads custom boss bars from the server save root.
    ///
    /// # Errors
    ///
    /// Returns an I/O or decode error when an existing boss bar file is malformed.
    pub async fn load(save_root: impl AsRef<Path>) -> io::Result<Self> {
        let path = boss_bar_file(save_root.as_ref());
        if !path.exists() {
            return Ok(Self::empty_at_path(path));
        }

        let bytes = fs::read(&path).await?;
        let file = decode_boss_bars_file(&bytes)?;
        let mut bars = HashMap::new();
        for entry in file.entries {
            let bar = entry.into_boss_bar()?;
            if bars.insert(bar.id.clone(), bar).is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate boss bar id in saved data",
                ));
            }
        }

        Ok(Self {
            path,
            bars,
            dirty: false,
            generation: 0,
        })
    }

    /// Creates an empty boss bar collection for `save_root`.
    #[must_use]
    pub fn empty(save_root: impl AsRef<Path>) -> Self {
        Self::empty_at_path(boss_bar_file(save_root.as_ref()))
    }

    fn empty_at_path(path: PathBuf) -> Self {
        Self {
            path,
            bars: HashMap::new(),
            dirty: false,
            generation: 0,
        }
    }

    /// Returns the number of custom boss bars.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bars.len()
    }

    /// Returns true if there are no custom boss bars.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bars.is_empty()
    }

    /// Returns custom boss bar ids in deterministic order.
    #[must_use]
    pub fn ids(&self) -> Vec<Identifier> {
        let mut ids = self.bars.keys().cloned().collect::<Vec<_>>();
        ids.sort_by_key(ToString::to_string);
        ids
    }

    /// Returns snapshots of all bars in deterministic id order.
    #[must_use]
    pub fn snapshots(&self) -> Vec<BossBarSnapshot> {
        self.ids()
            .into_iter()
            .filter_map(|id| self.bars.get(&id).map(BossBar::snapshot))
            .collect()
    }

    /// Returns one boss bar snapshot.
    #[must_use]
    pub fn get(&self, id: &Identifier) -> Option<BossBarSnapshot> {
        self.bars.get(id).map(BossBar::snapshot)
    }

    /// Creates a custom boss bar.
    pub fn create(
        &mut self,
        id: Identifier,
        name: TextComponent,
    ) -> Result<BossBarSnapshot, BossBarError> {
        if self.bars.contains_key(&id) {
            return Err(BossBarError::AlreadyExists(id));
        }

        let bar = BossBar::new(id.clone(), name);
        let snapshot = bar.snapshot();
        self.bars.insert(id, bar);
        self.mark_dirty();
        Ok(snapshot)
    }

    /// Removes a custom boss bar.
    pub fn remove(
        &mut self,
        id: &Identifier,
    ) -> Result<(BossBarSnapshot, Vec<BossBarBroadcast>), BossBarError> {
        let Some(bar) = self.bars.remove(id) else {
            return Err(BossBarError::Missing(id.clone()));
        };

        let snapshot = bar.snapshot();
        let broadcasts = bar.visible_broadcast(CBossEvent::remove(bar.runtime_id));
        self.mark_dirty();
        Ok((snapshot, broadcasts))
    }

    /// Sets the current value.
    pub fn set_value(
        &mut self,
        id: &Identifier,
        value: i32,
    ) -> Result<BossBarMutation, BossBarError> {
        let (mutation, changed) = {
            let Some(bar) = self.bars.get_mut(id) else {
                return Err(BossBarError::Missing(id.clone()));
            };
            let old_progress = bar.progress();
            let changed = bar.value != value;
            if changed {
                bar.value = value;
            }
            let broadcasts = if changed && old_progress != bar.progress() {
                bar.visible_broadcast(CBossEvent::update_progress(bar.runtime_id, bar.progress()))
            } else {
                Vec::new()
            };
            (
                BossBarMutation {
                    snapshot: bar.snapshot(),
                    changed,
                    broadcasts,
                },
                changed,
            )
        };
        if changed {
            self.mark_dirty();
        }
        Ok(mutation)
    }

    /// Sets the maximum value.
    pub fn set_max(&mut self, id: &Identifier, max: i32) -> Result<BossBarMutation, BossBarError> {
        let (mutation, changed) = {
            let Some(bar) = self.bars.get_mut(id) else {
                return Err(BossBarError::Missing(id.clone()));
            };
            let old_progress = bar.progress();
            let changed = bar.max != max;
            if changed {
                bar.max = max;
            }
            let broadcasts = if changed && old_progress != bar.progress() {
                bar.visible_broadcast(CBossEvent::update_progress(bar.runtime_id, bar.progress()))
            } else {
                Vec::new()
            };
            (
                BossBarMutation {
                    snapshot: bar.snapshot(),
                    changed,
                    broadcasts,
                },
                changed,
            )
        };
        if changed {
            self.mark_dirty();
        }
        Ok(mutation)
    }

    /// Sets the display name.
    pub fn set_name(
        &mut self,
        id: &Identifier,
        name: TextComponent,
    ) -> Result<BossBarMutation, BossBarError> {
        let (mutation, changed) = {
            let Some(bar) = self.bars.get_mut(id) else {
                return Err(BossBarError::Missing(id.clone()));
            };
            let changed = bar.name != name;
            if changed {
                bar.name = name;
            }
            let broadcasts = if changed {
                bar.visible_broadcast(CBossEvent::update_name(bar.runtime_id, bar.name.clone()))
            } else {
                Vec::new()
            };
            (
                BossBarMutation {
                    snapshot: bar.snapshot(),
                    changed,
                    broadcasts,
                },
                changed,
            )
        };
        if changed {
            self.mark_dirty();
        }
        Ok(mutation)
    }

    /// Sets the bar color.
    pub fn set_color(
        &mut self,
        id: &Identifier,
        color: BossBarColor,
    ) -> Result<BossBarMutation, BossBarError> {
        let (mutation, changed) = {
            let Some(bar) = self.bars.get_mut(id) else {
                return Err(BossBarError::Missing(id.clone()));
            };
            let changed = bar.color != color;
            if changed {
                bar.color = color;
            }
            let broadcasts = if changed {
                bar.visible_broadcast(CBossEvent::update_style(
                    bar.runtime_id,
                    bar.color,
                    bar.overlay,
                ))
            } else {
                Vec::new()
            };
            (
                BossBarMutation {
                    snapshot: bar.snapshot(),
                    changed,
                    broadcasts,
                },
                changed,
            )
        };
        if changed {
            self.mark_dirty();
        }
        Ok(mutation)
    }

    /// Sets the bar overlay.
    pub fn set_overlay(
        &mut self,
        id: &Identifier,
        overlay: BossBarOverlay,
    ) -> Result<BossBarMutation, BossBarError> {
        let (mutation, changed) = {
            let Some(bar) = self.bars.get_mut(id) else {
                return Err(BossBarError::Missing(id.clone()));
            };
            let changed = bar.overlay != overlay;
            if changed {
                bar.overlay = overlay;
            }
            let broadcasts = if changed {
                bar.visible_broadcast(CBossEvent::update_style(
                    bar.runtime_id,
                    bar.color,
                    bar.overlay,
                ))
            } else {
                Vec::new()
            };
            (
                BossBarMutation {
                    snapshot: bar.snapshot(),
                    changed,
                    broadcasts,
                },
                changed,
            )
        };
        if changed {
            self.mark_dirty();
        }
        Ok(mutation)
    }

    /// Sets visibility.
    pub fn set_visible(
        &mut self,
        id: &Identifier,
        visible: bool,
    ) -> Result<BossBarMutation, BossBarError> {
        let (mutation, changed) = {
            let Some(bar) = self.bars.get_mut(id) else {
                return Err(BossBarError::Missing(id.clone()));
            };
            let changed = bar.visible != visible;
            let broadcasts = if changed {
                bar.visible = visible;
                let packet = if visible {
                    bar.add_packet()
                } else {
                    CBossEvent::remove(bar.runtime_id)
                };
                vec![BossBarBroadcast {
                    recipients: sorted_uuids(&bar.players),
                    packet,
                }]
            } else {
                Vec::new()
            };
            (
                BossBarMutation {
                    snapshot: bar.snapshot(),
                    changed,
                    broadcasts,
                },
                changed,
            )
        };
        if changed {
            self.mark_dirty();
        }
        Ok(mutation)
    }

    /// Replaces persisted player membership.
    pub fn set_players(
        &mut self,
        id: &Identifier,
        players: impl IntoIterator<Item = Uuid>,
    ) -> Result<BossBarMutation, BossBarError> {
        let (mutation, changed) = {
            let Some(bar) = self.bars.get_mut(id) else {
                return Err(BossBarError::Missing(id.clone()));
            };
            let players = players.into_iter().collect::<HashSet<_>>();
            let changed = bar.players != players;
            let broadcasts = if changed {
                let mut broadcasts = Vec::new();
                if bar.visible {
                    let removed = bar
                        .players
                        .difference(&players)
                        .copied()
                        .collect::<HashSet<_>>();
                    if !removed.is_empty() {
                        broadcasts.push(BossBarBroadcast {
                            recipients: sorted_uuids(&removed),
                            packet: CBossEvent::remove(bar.runtime_id),
                        });
                    }

                    let added = players
                        .difference(&bar.players)
                        .copied()
                        .collect::<HashSet<_>>();
                    if !added.is_empty() {
                        broadcasts.push(BossBarBroadcast {
                            recipients: sorted_uuids(&added),
                            packet: bar.add_packet(),
                        });
                    }
                }
                bar.players = players;
                broadcasts
            } else {
                Vec::new()
            };
            (
                BossBarMutation {
                    snapshot: bar.snapshot(),
                    changed,
                    broadcasts,
                },
                changed,
            )
        };
        if changed {
            self.mark_dirty();
        }
        Ok(mutation)
    }

    /// Returns packets to send when a player joins.
    #[must_use]
    pub fn player_connected_packets(&self, uuid: Uuid) -> Vec<CBossEvent> {
        self.ids()
            .into_iter()
            .filter_map(|id| self.bars.get(&id))
            .filter(|bar| bar.visible && bar.players.contains(&uuid))
            .map(BossBar::add_packet)
            .collect()
    }

    /// Builds a save snapshot if the boss bar data is dirty.
    #[must_use]
    pub fn prepare_save(&self) -> Option<BossBarsSave> {
        if !self.dirty {
            return None;
        }

        let mut entries = self
            .bars
            .values()
            .map(BossBarFileEntry::from_boss_bar)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.id.to_string());

        Some(BossBarsSave {
            generation: self.generation,
            path: self.path.clone(),
            file: BossBarsFile {
                data_version: BOSS_BAR_DATA_VERSION,
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

/// Prepared boss bar save snapshot.
#[derive(Debug)]
pub struct BossBarsSave {
    generation: u64,
    path: PathBuf,
    file: BossBarsFile,
}

impl BossBarsSave {
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

        let bytes = encode_boss_bars_file(&self.file)?;
        fs::write(&self.path, bytes).await
    }
}

#[derive(Clone, Debug, SchemaWrite, SchemaRead)]
struct BossBarsFile {
    data_version: i32,
    entries: Vec<BossBarFileEntry>,
}

#[derive(Clone, Debug, SchemaWrite, SchemaRead)]
struct BossBarFileEntry {
    id: Identifier,
    name_nbt: Vec<u8>,
    visible: bool,
    value: i32,
    max: i32,
    color: String,
    overlay: String,
    darken_screen: bool,
    play_music: bool,
    create_world_fog: bool,
    players: Vec<[u8; 16]>,
}

impl BossBarFileEntry {
    fn from_boss_bar(bar: &BossBar) -> Self {
        let mut players = bar
            .players
            .iter()
            .map(|uuid| *uuid.as_bytes())
            .collect::<Vec<_>>();
        players.sort();

        Self {
            id: bar.id.clone(),
            name_nbt: text_component_to_persistent("Name", &bar.name),
            visible: bar.visible,
            value: bar.value,
            max: bar.max,
            color: bar.color.as_str().to_owned(),
            overlay: bar.overlay.as_str().to_owned(),
            darken_screen: bar.properties.darken_screen,
            play_music: bar.properties.play_music,
            create_world_fog: bar.properties.create_world_fog,
            players,
        }
    }

    fn into_boss_bar(self) -> io::Result<BossBar> {
        let color = BossBarColor::from_name(&self.color).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid boss bar color '{}'", self.color),
            )
        })?;
        let overlay = BossBarOverlay::from_name(&self.overlay).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid boss bar overlay '{}'", self.overlay),
            )
        })?;

        Ok(BossBar {
            runtime_id: Uuid::new_v4(),
            id: self.id.clone(),
            name: text_component_from_persistent("Name", &self.name_nbt, &self.id)?,
            visible: self.visible,
            value: self.value,
            max: self.max,
            color,
            overlay,
            properties: BossBarProperties {
                darken_screen: self.darken_screen,
                play_music: self.play_music,
                create_world_fog: self.create_world_fog,
            },
            players: self.players.into_iter().map(Uuid::from_bytes).collect(),
        })
    }
}

fn boss_bar_file(save_root: &Path) -> PathBuf {
    save_root
        .join("data")
        .join("minecraft")
        .join("custom_boss_events.dat")
}

fn sorted_uuids(uuids: &HashSet<Uuid>) -> Vec<Uuid> {
    let mut uuids = uuids.iter().copied().collect::<Vec<_>>();
    uuids.sort_by_key(|uuid| *uuid.as_bytes());
    uuids
}

fn clamp_progress(value: i32, max: i32) -> f32 {
    let progress = value as f32 / max as f32;
    if progress < 0.0 {
        0.0
    } else if progress > 1.0 {
        1.0
    } else {
        progress
    }
}

fn text_component_to_persistent(name: &'static str, component: &TextComponent) -> Vec<u8> {
    let mut root = NbtCompound::new();
    root.insert(name, component.to_nbt_tag());
    let mut bytes = Vec::new();
    root.write(&mut bytes);
    bytes
}

fn text_component_from_persistent(
    name: &'static str,
    bytes: &[u8],
    id: &Identifier,
) -> io::Result<TextComponent> {
    let root = read_borrowed_compound(&mut Cursor::new(bytes)).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to parse boss bar name NBT for {id}"),
        )
    })?;
    let root = simdnbt::borrow::NbtCompound::from(&root);
    let tag = root.get(name).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("missing boss bar name NBT field for {id}"),
        )
    })?;
    TextComponent::from_nbt(&tag.to_owned()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to decode boss bar name for {id}"),
        )
    })
}

fn encode_boss_bars_file(file: &BossBarsFile) -> io::Result<Vec<u8>> {
    encode_file(
        BOSS_BAR_MAGIC,
        BOSS_BAR_STORAGE_VERSION,
        wincode::serialize(file),
    )
}

fn decode_boss_bars_file(bytes: &[u8]) -> io::Result<BossBarsFile> {
    let payload = decode_file(BOSS_BAR_MAGIC, BOSS_BAR_STORAGE_VERSION, bytes)?;
    let file: BossBarsFile = wincode::deserialize(&payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    if file.data_version != BOSS_BAR_DATA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported boss bar data version {}", file.data_version),
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
            "boss bar file is too short",
        ));
    }
    if bytes[0..4] != expected_magic {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid boss bar file magic",
        ));
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != expected_version {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported boss bar storage version {version}"),
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
            "steel-bossbars-{test_name}-{}-{unique}",
            process::id()
        ));
        std_fs::create_dir_all(&path).expect("temp save root should be created");
        path
    }

    #[test]
    fn created_bar_uses_vanilla_custom_boss_event_defaults() {
        let root = temp_save_root("defaults");
        let mut bars = BossBars::empty(&root);
        let id = Identifier::new_static("minecraft", "test");

        let bar = bars
            .create(id.clone(), TextComponent::plain("Test"))
            .expect("bar should be created");

        assert_eq!(bar.id, id);
        assert!(bar.visible);
        assert_eq!(bar.value, 0);
        assert_eq!(bar.max, 100);
        assert_eq!(bar.color, BossBarColor::White);
        assert_eq!(bar.overlay, BossBarOverlay::Progress);
        assert_eq!(bar.players, Vec::<Uuid>::new());
        assert!(bars.prepare_save().is_some());

        let _ = std_fs::remove_dir_all(root);
    }

    #[test]
    fn player_membership_broadcasts_adds_and_removes() {
        let root = temp_save_root("players");
        let mut bars = BossBars::empty(&root);
        let id = Identifier::new_static("minecraft", "test");
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);

        bars.create(id.clone(), TextComponent::plain("Test"))
            .expect("bar should be created");

        let added = bars
            .set_players(&id, [first])
            .expect("players should update");
        assert!(added.changed);
        assert_eq!(added.broadcasts.len(), 1);
        assert_eq!(added.broadcasts[0].recipients, vec![first]);

        let changed = bars
            .set_players(&id, [second])
            .expect("players should update");
        assert!(changed.changed);
        assert_eq!(changed.broadcasts.len(), 2);
        assert_eq!(changed.broadcasts[0].recipients, vec![first]);
        assert_eq!(changed.broadcasts[1].recipients, vec![second]);

        let _ = std_fs::remove_dir_all(root);
    }

    #[test]
    fn hidden_bars_do_not_broadcast_member_changes() {
        let root = temp_save_root("hidden");
        let mut bars = BossBars::empty(&root);
        let id = Identifier::new_static("minecraft", "test");

        bars.create(id.clone(), TextComponent::plain("Test"))
            .expect("bar should be created");
        bars.set_visible(&id, false)
            .expect("visibility should update");

        let changed = bars
            .set_players(&id, [Uuid::from_u128(1)])
            .expect("players should update");

        assert!(changed.changed);
        assert!(changed.broadcasts.is_empty());

        let _ = std_fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn save_and_load_preserves_persistent_state_but_regenerates_runtime_id() {
        let root = temp_save_root("roundtrip");
        let mut bars = BossBars::empty(&root);
        let id = Identifier::new_static("minecraft", "test");
        let player = Uuid::from_u128(42);

        let created = bars
            .create(id.clone(), TextComponent::plain("Test"))
            .expect("bar should be created");
        bars.set_color(&id, BossBarColor::Red)
            .expect("color should update");
        bars.set_players(&id, [player])
            .expect("players should update");
        let save = bars.prepare_save().expect("dirty bars should save");
        save.write().await.expect("boss bars should write to disk");
        bars.mark_saved(save.generation());

        let loaded = BossBars::load(&root)
            .await
            .expect("boss bars should load from disk");
        let loaded_bar = loaded.get(&id).expect("loaded bar should exist");

        assert_ne!(loaded_bar.runtime_id, created.runtime_id);
        assert_eq!(loaded_bar.name, TextComponent::plain("Test"));
        assert_eq!(loaded_bar.color, BossBarColor::Red);
        assert_eq!(loaded_bar.players, vec![player]);
        assert!(loaded.prepare_save().is_none());

        let _ = std_fs::remove_dir_all(root);
    }
}
