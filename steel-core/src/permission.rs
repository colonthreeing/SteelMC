//! Permission keys, expressions, and effective permission evaluation.

use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    ops::{BitAnd, BitOr},
};

use serde::Deserialize;

/// One dotted permission key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PermissionKey(String);

impl PermissionKey {
    /// Parses a dotted permission key.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is empty, has empty segments, or places a
    /// wildcard anywhere except the final segment.
    pub fn parse(value: impl Into<String>) -> Result<Self, PermissionKeyError> {
        let value = value.into();
        if value.is_empty() {
            return Err(PermissionKeyError::Empty);
        }

        let segments = value.split('.').collect::<Vec<_>>();
        for (index, segment) in segments.iter().enumerate() {
            if segment.is_empty() {
                return Err(PermissionKeyError::EmptySegment);
            }
            if *segment == "*" {
                if index + 1 != segments.len() {
                    return Err(PermissionKeyError::WildcardNotFinal);
                }
                continue;
            }
            if segment.contains('*') {
                return Err(PermissionKeyError::InvalidWildcardSegment);
            }
            validate_permission_segment(segment)?;
        }

        Ok(Self(value))
    }

    /// Builds a permission key from already-validated segments.
    ///
    /// # Errors
    ///
    /// Returns an error when no segments are supplied.
    pub fn from_segments(
        segments: impl IntoIterator<Item = PermissionSegment>,
    ) -> Result<Self, PermissionKeyError> {
        let mut value = String::new();
        for segment in segments {
            if !value.is_empty() {
                value.push('.');
            }
            value.push_str(segment.as_str());
        }
        if value.is_empty() {
            return Err(PermissionKeyError::Empty);
        }
        Ok(Self(value))
    }

    /// Returns the key as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Builds a child permission key by appending one non-wildcard segment.
    ///
    /// # Errors
    ///
    /// Returns an error when appending to this key would create an invalid
    /// permission key, such as appending below a wildcard.
    pub fn child(&self, segment: &PermissionSegment) -> Result<Self, PermissionKeyError> {
        Self::parse(format!("{}.{}", self.0, segment.as_str()))
    }

    /// Returns true when this key pattern matches `other`.
    #[must_use]
    pub fn matches(&self, other: &Self) -> bool {
        if self.0 == "*" {
            return true;
        }

        let Some(prefix) = self.0.strip_suffix(".*") else {
            return self == other;
        };

        other
            .0
            .strip_prefix(prefix)
            .is_some_and(|remaining| remaining.starts_with('.'))
    }

    fn specificity(&self) -> usize {
        if self.0 == "*" {
            return 0;
        }

        self.0
            .strip_suffix(".*")
            .map_or(self.0.as_str(), |prefix| prefix)
            .split('.')
            .count()
    }
}

/// One non-wildcard segment in a permission key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PermissionSegment(String);

impl PermissionSegment {
    /// Parses one non-wildcard permission segment.
    ///
    /// # Errors
    ///
    /// Returns an error when the segment is empty or contains invalid characters.
    pub fn parse(value: impl Into<String>) -> Result<Self, PermissionKeyError> {
        let value = value.into();
        if value.is_empty() {
            return Err(PermissionKeyError::EmptySegment);
        }
        if value.contains('*') {
            return Err(PermissionKeyError::InvalidWildcardSegment);
        }
        validate_permission_segment(&value)?;
        Ok(Self(value))
    }

    /// Returns this segment as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_permission_segment(segment: &str) -> Result<(), PermissionKeyError> {
    if segment
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        Ok(())
    } else {
        Err(PermissionKeyError::InvalidSegment)
    }
}

/// Invalid permission key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionKeyError {
    /// The key is empty.
    Empty,
    /// The key contains an empty segment.
    EmptySegment,
    /// A wildcard was used before the final segment.
    WildcardNotFinal,
    /// A wildcard was embedded inside a segment.
    InvalidWildcardSegment,
    /// A segment contains a character outside `[A-Za-z0-9_-]`.
    InvalidSegment,
}

impl fmt::Display for PermissionKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "permission key is empty"),
            Self::EmptySegment => write!(f, "permission key contains an empty segment"),
            Self::WildcardNotFinal => {
                write!(f, "permission wildcard must be the final segment")
            }
            Self::InvalidWildcardSegment => {
                write!(f, "permission wildcard must occupy the full segment")
            }
            Self::InvalidSegment => {
                write!(
                    f,
                    "permission segment must contain only letters, numbers, '_' or '-'"
                )
            }
        }
    }
}

impl Error for PermissionKeyError {}

/// A boolean permission expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionExpr {
    /// A single permission key.
    Key(PermissionKey),
    /// All child expressions must be allowed.
    All(Vec<PermissionExpr>),
    /// At least one child expression must be allowed.
    Any(Vec<PermissionExpr>),
}

impl PermissionExpr {
    /// Creates a leaf expression from a parsed permission key.
    #[must_use]
    pub const fn key(key: PermissionKey) -> Self {
        Self::Key(key)
    }
}

impl BitAnd for PermissionExpr {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::All(mut left), Self::All(mut right)) => {
                left.append(&mut right);
                Self::All(left)
            }
            (Self::All(mut left), right) => {
                left.push(right);
                Self::All(left)
            }
            (left, Self::All(mut right)) => {
                right.insert(0, left);
                Self::All(right)
            }
            (left, right) => Self::All(vec![left, right]),
        }
    }
}

impl BitOr for PermissionExpr {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::Any(mut left), Self::Any(mut right)) => {
                left.append(&mut right);
                Self::Any(left)
            }
            (Self::Any(mut left), right) => {
                left.push(right);
                Self::Any(left)
            }
            (left, Self::Any(mut right)) => {
                right.insert(0, left);
                Self::Any(right)
            }
            (left, right) => Self::Any(vec![left, right]),
        }
    }
}

/// Resolved permission state for one entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionState {
    /// Explicitly allow the matching permission.
    Allow,
    /// Explicitly deny the matching permission.
    Deny,
}

/// One permission rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionEntry {
    key: PermissionKey,
    state: PermissionState,
}

impl PermissionEntry {
    /// Creates one permission entry.
    #[must_use]
    pub const fn new(key: PermissionKey, state: PermissionState) -> Self {
        Self { key, state }
    }

    /// Creates an allow entry.
    #[must_use]
    pub const fn allow(key: PermissionKey) -> Self {
        Self::new(key, PermissionState::Allow)
    }

    /// Creates a deny entry.
    #[must_use]
    pub const fn deny(key: PermissionKey) -> Self {
        Self::new(key, PermissionState::Deny)
    }

    /// Returns the permission key pattern.
    #[must_use]
    pub const fn key(&self) -> &PermissionKey {
        &self.key
    }

    /// Returns the entry state.
    #[must_use]
    pub const fn state(&self) -> PermissionState {
        self.state
    }
}

/// A flat effective permission set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PermissionSet {
    entries: Vec<PermissionEntry>,
}

impl PermissionSet {
    /// Creates an empty permission set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Creates a permission set from entries.
    #[must_use]
    pub fn from_entries(entries: impl IntoIterator<Item = PermissionEntry>) -> Self {
        Self {
            entries: entries.into_iter().collect(),
        }
    }

    /// Returns all entries in insertion order.
    #[must_use]
    pub fn entries(&self) -> &[PermissionEntry] {
        &self.entries
    }

    /// Adds one permission entry.
    pub fn push(&mut self, entry: PermissionEntry) {
        self.entries.push(entry);
    }

    /// Adds one allow entry.
    pub fn allow(&mut self, key: PermissionKey) {
        self.push(PermissionEntry::allow(key));
    }

    /// Adds one deny entry.
    pub fn deny(&mut self, key: PermissionKey) {
        self.push(PermissionEntry::deny(key));
    }

    /// Sets one exact permission entry, replacing any previous exact entry.
    pub fn set(&mut self, key: PermissionKey, state: PermissionState) {
        self.entries.retain(|entry| entry.key != key);
        self.entries.push(PermissionEntry::new(key, state));
    }

    /// Removes one exact permission entry.
    ///
    /// Returns true when an entry was removed.
    pub fn unset(&mut self, key: &PermissionKey) -> bool {
        let old_len = self.entries.len();
        self.entries.retain(|entry| entry.key() != key);
        self.entries.len() != old_len
    }

    /// Resolves one key. Unset permissions return `None`.
    #[must_use]
    pub fn resolve_key(&self, key: &PermissionKey) -> Option<PermissionState> {
        let mut best = None;

        for entry in &self.entries {
            if !entry.key.matches(key) {
                continue;
            }

            let specificity = entry.key.specificity();
            match best {
                None => best = Some((specificity, entry.state)),
                Some((best_specificity, _)) if specificity > best_specificity => {
                    best = Some((specificity, entry.state));
                }
                Some((best_specificity, PermissionState::Allow))
                    if specificity == best_specificity && entry.state == PermissionState::Deny =>
                {
                    best = Some((specificity, PermissionState::Deny));
                }
                _ => {}
            }
        }

        best.map(|(_, state)| state)
    }

    /// Returns whether a key is allowed. Unset defaults to deny.
    #[must_use]
    pub fn allows_key(&self, key: &PermissionKey) -> bool {
        self.resolve_key(key) == Some(PermissionState::Allow)
    }

    /// Returns whether this set allows a permission expression.
    #[must_use]
    pub fn allows(&self, permission: &PermissionExpr) -> bool {
        match permission {
            PermissionExpr::Key(key) => self.allows_key(key),
            PermissionExpr::All(children) => children.iter().all(|child| self.allows(child)),
            PermissionExpr::Any(children) => children.iter().any(|child| self.allows(child)),
        }
    }
}

/// Parsed `groups.toml` permissions configuration.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionGroupsConfig {
    /// Groups every player receives.
    pub default_groups: Vec<String>,
    /// Named groups available for assignment.
    pub groups: BTreeMap<String, PermissionGroupConfig>,
}

impl Default for PermissionGroupsConfig {
    fn default() -> Self {
        let mut groups = BTreeMap::new();
        groups.insert("default".to_owned(), PermissionGroupConfig::default());
        groups.insert(
            "op".to_owned(),
            PermissionGroupConfig {
                allow: vec!["*".to_owned()],
                deny: Vec::new(),
            },
        );

        Self {
            default_groups: vec!["default".to_owned()],
            groups,
        }
    }
}

/// One configured permission group.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionGroupConfig {
    /// Permission keys explicitly allowed by this group.
    pub allow: Vec<String>,
    /// Permission keys explicitly denied by this group.
    pub deny: Vec<String>,
}

/// Resolved permission groups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionGroups {
    default_groups: Vec<String>,
    groups: BTreeMap<String, PermissionGroup>,
}

impl Default for PermissionGroups {
    fn default() -> Self {
        let mut op_permissions = PermissionSet::new();
        op_permissions.allow(PermissionKey("*".to_owned()));

        let mut groups = BTreeMap::new();
        groups.insert(
            "default".to_owned(),
            PermissionGroup {
                permissions: PermissionSet::new(),
            },
        );
        groups.insert(
            "op".to_owned(),
            PermissionGroup {
                permissions: op_permissions,
            },
        );

        Self {
            default_groups: vec!["default".to_owned()],
            groups,
        }
    }
}

impl PermissionGroups {
    /// Builds resolved permission groups from config.
    ///
    /// # Errors
    ///
    /// Returns an error when a default group is missing or a permission key is invalid.
    pub fn from_config(config: PermissionGroupsConfig) -> Result<Self, PermissionConfigError> {
        for group in &config.default_groups {
            if !config.groups.contains_key(group) {
                return Err(PermissionConfigError::MissingDefaultGroup(group.clone()));
            }
        }

        let mut groups = BTreeMap::new();
        for (name, group) in config.groups {
            let mut permissions = PermissionSet::new();
            for permission in group.allow {
                permissions.allow(PermissionKey::parse(permission).map_err(|source| {
                    PermissionConfigError::InvalidPermissionKey {
                        group: name.clone(),
                        source,
                    }
                })?);
            }
            for permission in group.deny {
                permissions.deny(PermissionKey::parse(permission).map_err(|source| {
                    PermissionConfigError::InvalidPermissionKey {
                        group: name.clone(),
                        source,
                    }
                })?);
            }
            groups.insert(name, PermissionGroup { permissions });
        }

        Ok(Self {
            default_groups: config.default_groups,
            groups,
        })
    }

    /// Returns the configured default group names.
    #[must_use]
    pub fn default_groups(&self) -> &[String] {
        &self.default_groups
    }

    /// Returns configured groups keyed by name.
    #[must_use]
    pub const fn groups(&self) -> &BTreeMap<String, PermissionGroup> {
        &self.groups
    }

    /// Returns whether a group exists.
    #[must_use]
    pub fn contains_group(&self, group: &str) -> bool {
        self.groups.contains_key(group)
    }

    /// Builds an effective permission set from default groups, assigned groups,
    /// and player-level overrides.
    ///
    /// Assigned groups that are not configured have no effect.
    #[must_use]
    pub fn effective_permissions(
        &self,
        assigned_groups: &[String],
        player_permissions: &PermissionSet,
    ) -> PermissionSet {
        let mut effective = PermissionSet::new();

        for group in &self.default_groups {
            self.append_group_permissions(group, &mut effective);
        }
        for group in assigned_groups {
            self.append_group_permissions(group, &mut effective);
        }
        for entry in player_permissions.entries() {
            effective.push(entry.clone());
        }

        effective
    }

    fn append_group_permissions(&self, group: &str, effective: &mut PermissionSet) {
        let Some(group) = self.groups.get(group) else {
            return;
        };

        for entry in group.permissions.entries() {
            effective.push(entry.clone());
        }
    }
}

/// One resolved permission group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionGroup {
    permissions: PermissionSet,
}

impl PermissionGroup {
    /// Returns this group's permission entries.
    #[must_use]
    pub const fn permissions(&self) -> &PermissionSet {
        &self.permissions
    }
}

/// Invalid permission group configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionConfigError {
    /// A default group name does not exist in the group map.
    MissingDefaultGroup(String),
    /// A group contains an invalid permission key.
    InvalidPermissionKey {
        /// Group containing the bad key.
        group: String,
        /// Parse error.
        source: PermissionKeyError,
    },
}

impl fmt::Display for PermissionConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingDefaultGroup(group) => {
                write!(f, "default permission group '{group}' is not configured")
            }
            Self::InvalidPermissionKey { group, source } => {
                write!(
                    f,
                    "permission group '{group}' contains invalid key: {source}"
                )
            }
        }
    }
}

impl Error for PermissionConfigError {}

#[cfg(test)]
mod tests {
    use super::{
        PermissionEntry, PermissionExpr, PermissionGroups, PermissionGroupsConfig, PermissionKey,
        PermissionKeyError, PermissionSegment, PermissionSet, PermissionState,
    };

    fn key(value: &str) -> PermissionKey {
        PermissionKey::parse(value).expect("key parses")
    }

    #[test]
    fn permission_keys_reject_mid_pattern_wildcards() {
        assert_eq!(
            PermissionKey::parse("minecraft.*.give").err(),
            Some(PermissionKeyError::WildcardNotFinal)
        );
        assert_eq!(
            PermissionKey::parse("minecraft.command.g*").err(),
            Some(PermissionKeyError::InvalidWildcardSegment)
        );
    }

    #[test]
    fn permission_keys_reject_invalid_segments() {
        assert_eq!(
            PermissionKey::parse("minecraft.command.give item").err(),
            Some(PermissionKeyError::InvalidSegment)
        );
        assert_eq!(
            PermissionSegment::parse("give.item").err(),
            Some(PermissionKeyError::InvalidSegment)
        );
    }

    #[test]
    fn permission_key_can_be_built_from_typed_segments() {
        let key = PermissionKey::from_segments([
            PermissionSegment::parse("minecraft").expect("segment parses"),
            PermissionSegment::parse("command").expect("segment parses"),
            PermissionSegment::parse("give").expect("segment parses"),
        ])
        .expect("segments build key");

        assert_eq!(key.as_str(), "minecraft.command.give");
    }

    #[test]
    fn permission_key_can_append_child_segments() {
        let parent = key("minecraft.command.tick");
        let child = parent
            .child(&PermissionSegment::parse("freeze").expect("segment parses"))
            .expect("child key parses");

        assert_eq!(child.as_str(), "minecraft.command.tick.freeze");
        assert_eq!(
            key("minecraft.command.*")
                .child(&PermissionSegment::parse("freeze").expect("segment parses"))
                .err(),
            Some(PermissionKeyError::WildcardNotFinal)
        );
    }

    #[test]
    fn trailing_wildcard_matches_descendants() {
        let wildcard = key("minecraft.command.*");
        let give = key("minecraft.command.give");
        let sibling = key("minecraft.other.give");

        assert!(wildcard.matches(&give));
        assert!(!wildcard.matches(&sibling));
    }

    #[test]
    fn star_wildcard_matches_every_permission() {
        let wildcard = key("*");
        let give = key("minecraft.command.give");
        let plugin = key("some.plugin.dangerous");

        assert!(wildcard.matches(&give));
        assert!(wildcard.matches(&plugin));
    }

    #[test]
    fn unset_permissions_default_to_deny() {
        let permissions = PermissionSet::new();

        assert_eq!(
            permissions.resolve_key(&key("minecraft.command.give")),
            None
        );
        assert!(!permissions.allows_key(&key("minecraft.command.give")));
    }

    #[test]
    fn more_specific_entry_wins() {
        let permissions = PermissionSet::from_entries([
            PermissionEntry::deny(key("minecraft.command.*")),
            PermissionEntry::allow(key("minecraft.command.give")),
        ]);

        assert!(permissions.allows_key(&key("minecraft.command.give")));
        assert!(!permissions.allows_key(&key("minecraft.command.kill")));
    }

    #[test]
    fn deny_wins_tied_specificity() {
        let permissions = PermissionSet::from_entries([
            PermissionEntry::allow(key("minecraft.command.give")),
            PermissionEntry::deny(key("minecraft.command.give")),
        ]);

        assert_eq!(
            permissions.resolve_key(&key("minecraft.command.give")),
            Some(PermissionState::Deny)
        );
    }

    #[test]
    fn set_replaces_exact_permission_entry() {
        let mut permissions = PermissionSet::from_entries([
            PermissionEntry::deny(key("minecraft.command.*")),
            PermissionEntry::allow(key("minecraft.command.give")),
        ]);

        permissions.set(key("minecraft.command.give"), PermissionState::Deny);

        assert_eq!(
            permissions.resolve_key(&key("minecraft.command.give")),
            Some(PermissionState::Deny)
        );
        assert_eq!(permissions.entries().len(), 2);
    }

    #[test]
    fn unset_removes_exact_permission_entry() {
        let mut permissions = PermissionSet::from_entries([
            PermissionEntry::deny(key("minecraft.command.*")),
            PermissionEntry::allow(key("minecraft.command.give")),
        ]);

        assert!(permissions.unset(&key("minecraft.command.give")));
        assert!(!permissions.unset(&key("minecraft.command.give")));

        assert_eq!(
            permissions.resolve_key(&key("minecraft.command.give")),
            Some(PermissionState::Deny)
        );
        assert_eq!(permissions.entries().len(), 1);
    }

    #[test]
    fn permission_expressions_support_all_and_any() {
        let permissions = PermissionSet::from_entries([
            PermissionEntry::allow(key("minecraft.command.gamemode")),
            PermissionEntry::deny(key("minecraft.command.gamemode.creative")),
            PermissionEntry::allow(key("minecraft.command.gamemode.survival")),
        ]);
        let base = PermissionExpr::key(key("minecraft.command.gamemode"));
        let creative = PermissionExpr::key(key("minecraft.command.gamemode.creative"));
        let survival = PermissionExpr::key(key("minecraft.command.gamemode.survival"));

        assert!(!permissions.allows(&(base.clone() & creative)));
        assert!(permissions.allows(&(base | survival)));
    }

    #[test]
    fn default_group_config_contains_editable_op_group() {
        let groups = PermissionGroups::from_config(PermissionGroupsConfig::default())
            .expect("default groups config is valid");

        assert!(groups.groups().contains_key("default"));
        assert!(groups.groups().contains_key("op"));
        assert_eq!(groups.default_groups(), ["default"]);
        assert!(
            groups
                .groups()
                .get("op")
                .is_some_and(|group| group.permissions().allows_key(&key("steel.admin")))
        );
    }

    #[test]
    fn groups_combine_with_player_permissions() {
        let config = PermissionGroupsConfig::default();
        let groups = PermissionGroups::from_config(config).expect("groups config is valid");
        let player_permissions =
            PermissionSet::from_entries([PermissionEntry::deny(key("steel.stop"))]);

        let effective = groups.effective_permissions(&["op".to_owned()], &player_permissions);

        assert!(effective.allows_key(&key("steel.admin")));
        assert!(!effective.allows_key(&key("steel.stop")));
    }
}
