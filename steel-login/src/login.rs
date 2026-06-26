//! Login helper functions.
//!
//! Contains utilities for player name validation and offline UUID generation.

use uuid::{Builder, Uuid, Variant, Version};

/// Checks if a player name is valid.
///
/// A valid player name:
/// - Is between 3 and 16 characters long
/// - Contains only ASCII alphanumeric characters or underscores
#[must_use]
pub fn is_valid_player_name(name: &str) -> bool {
    (3..=16).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Generates an offline mode UUID for a player.
///
/// This creates a deterministic UUID based on the username hash,
/// used when the server is in offline mode.
///
#[must_use]
pub fn offline_uuid(username: &str) -> Uuid {
    Builder::from_md5_bytes(md5::compute(format!("OfflinePlayer:{username}")).0)
        .with_version(Version::Md5)
        .with_variant(Variant::RFC4122)
        .into_uuid()
}

#[cfg(test)]
mod tests {
    use super::{is_valid_player_name, offline_uuid};

    #[test]
    fn validates_player_names() {
        assert!(is_valid_player_name("Steve"));
        assert!(is_valid_player_name("Alex_123"));
        assert!(!is_valid_player_name("ab"));
        assert!(!is_valid_player_name("name-with-dash"));
        assert!(!is_valid_player_name("way_too_long_player_name"));
    }

    #[test]
    fn offline_uuid_matches_vanilla_name_uuid() {
        assert_eq!(
            offline_uuid("Steve").to_string(),
            "5627dd98-e6be-3c21-b8a8-e92344183641"
        );
    }
}
