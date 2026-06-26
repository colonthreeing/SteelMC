//! Known player profile index.

use uuid::Uuid;

/// One player profile known to this server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownPlayer {
    uuid: Uuid,
    last_known_name: String,
}

impl KnownPlayer {
    /// Creates a known player entry.
    #[must_use]
    pub fn new(uuid: Uuid, last_known_name: impl Into<String>) -> Self {
        Self {
            uuid,
            last_known_name: last_known_name.into(),
        }
    }

    /// Returns the player's UUID.
    #[must_use]
    pub const fn uuid(&self) -> Uuid {
        self.uuid
    }

    /// Returns the player's last known name.
    #[must_use]
    pub fn last_known_name(&self) -> &str {
        &self.last_known_name
    }
}

/// In-memory known player index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KnownPlayers {
    entries: Vec<KnownPlayer>,
}

impl KnownPlayers {
    /// Creates an empty known player index.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Creates an index from entries, normalizing duplicate UUIDs and names.
    #[must_use]
    pub fn from_entries(entries: impl IntoIterator<Item = KnownPlayer>) -> Self {
        let mut players = Self::new();
        for entry in entries {
            players.record(entry.uuid, entry.last_known_name);
        }
        players
    }

    /// Returns the known player entries.
    #[must_use]
    pub fn entries(&self) -> &[KnownPlayer] {
        &self.entries
    }

    /// Records a profile as known.
    ///
    /// If another entry has the same UUID, its last known name is updated. If
    /// another entry has the same case-insensitive name, the new UUID owns that
    /// name.
    ///
    /// Returns true when the index changed.
    pub fn record(&mut self, uuid: Uuid, last_known_name: impl Into<String>) -> bool {
        let last_known_name = last_known_name.into();
        let mut changed = false;

        self.entries.retain(|entry| {
            let duplicate_name =
                entry.uuid != uuid && entry.last_known_name.eq_ignore_ascii_case(&last_known_name);
            changed |= duplicate_name;
            !duplicate_name
        });

        let Some(entry) = self.entries.iter_mut().find(|entry| entry.uuid == uuid) else {
            self.entries.push(KnownPlayer::new(uuid, last_known_name));
            return true;
        };

        if entry.last_known_name == last_known_name {
            return changed;
        }

        entry.last_known_name = last_known_name;
        true
    }

    /// Looks up a known player by UUID.
    #[must_use]
    pub fn by_uuid(&self, uuid: Uuid) -> Option<&KnownPlayer> {
        self.entries.iter().find(|entry| entry.uuid == uuid)
    }

    /// Looks up a known player by case-insensitive name.
    #[must_use]
    pub fn by_name(&self, name: &str) -> Option<&KnownPlayer> {
        self.entries
            .iter()
            .find(|entry| entry.last_known_name.eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::{KnownPlayer, KnownPlayers};
    use uuid::Uuid;

    #[test]
    fn record_updates_existing_uuid_name() {
        let uuid = Uuid::from_u128(1);
        let mut players = KnownPlayers::new();

        assert!(players.record(uuid, "Steve"));
        assert!(players.record(uuid, "Alex"));
        assert!(!players.record(uuid, "Alex"));

        assert_eq!(players.entries().len(), 1);
        assert_eq!(
            players.by_uuid(uuid).map(KnownPlayer::last_known_name),
            Some("Alex")
        );
        assert!(players.by_name("alex").is_some());
    }

    #[test]
    fn record_reassigns_duplicate_name_to_latest_uuid() {
        let old_uuid = Uuid::from_u128(1);
        let new_uuid = Uuid::from_u128(2);
        let mut players = KnownPlayers::from_entries([
            KnownPlayer::new(old_uuid, "Steve"),
            KnownPlayer::new(Uuid::from_u128(3), "Alex"),
        ]);

        assert!(players.record(new_uuid, "steve"));

        assert!(players.by_uuid(old_uuid).is_none());
        assert_eq!(
            players.by_name("STEVE").map(KnownPlayer::uuid),
            Some(new_uuid)
        );
        assert_eq!(players.entries().len(), 2);
    }
}
