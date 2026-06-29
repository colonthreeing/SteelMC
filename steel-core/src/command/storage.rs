//! Server-level command storage.

use std::collections::HashMap;

use simdnbt::owned::NbtCompound;
use steel_utils::{Identifier, locks::SyncRwLock};

/// Server-level storage for NBT compounds keyed by identifiers.
///
/// Mirrors vanilla `CommandStorage` runtime semantics: missing keys read as an
/// empty compound, and setting an empty compound removes the key.
#[derive(Default)]
pub struct CommandStorage {
    entries: SyncRwLock<HashMap<Identifier, NbtCompound>>,
}

impl CommandStorage {
    /// Creates an empty command storage.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the compound stored at `id`, or an empty compound if the key is absent.
    #[must_use]
    pub fn get(&self, id: &Identifier) -> NbtCompound {
        self.entries
            .read()
            .get(id)
            .cloned()
            .unwrap_or_else(NbtCompound::new)
    }

    /// Stores `contents` at `id`.
    ///
    /// Empty compounds remove the key, matching vanilla command storage.
    pub fn set(&self, id: Identifier, contents: NbtCompound) {
        let mut entries = self.entries.write();
        if contents.is_empty() {
            entries.remove(&id);
        } else {
            entries.insert(id, contents);
        }
    }

    /// Returns a snapshot of stored keys.
    #[must_use]
    pub fn keys(&self) -> Vec<Identifier> {
        let mut keys = self.entries.read().keys().cloned().collect::<Vec<_>>();
        keys.sort_by(|left, right| left.to_string().cmp(&right.to_string()));
        keys
    }
}

#[cfg(test)]
mod tests {
    use simdnbt::owned::NbtCompound;
    use steel_utils::Identifier;

    use super::CommandStorage;

    #[test]
    fn missing_keys_read_as_empty_compounds() {
        let storage = CommandStorage::new();

        assert!(
            storage
                .get(&Identifier::new_static("minecraft", "missing"))
                .is_empty()
        );
    }

    #[test]
    fn empty_compounds_remove_keys() {
        let storage = CommandStorage::new();
        let key = Identifier::new_static("steel", "data");
        let mut value = NbtCompound::new();
        value.insert("Count", 3);

        storage.set(key.clone(), value);
        assert_eq!(storage.keys(), vec![key.clone()]);

        storage.set(key, NbtCompound::new());
        assert!(storage.keys().is_empty());
    }
}
