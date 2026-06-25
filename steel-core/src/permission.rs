//! Permission keys, expressions, and effective permission evaluation.

use std::ops::{BitAnd, BitOr};

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
        if segments.iter().any(|segment| segment.is_empty()) {
            return Err(PermissionKeyError::EmptySegment);
        }

        if let Some((index, _)) = segments
            .iter()
            .enumerate()
            .find(|(_, segment)| **segment == "*")
            && index + 1 != segments.len()
        {
            return Err(PermissionKeyError::WildcardNotFinal);
        }

        if segments
            .iter()
            .any(|segment| segment.contains('*') && *segment != "*")
        {
            return Err(PermissionKeyError::InvalidWildcardSegment);
        }

        Ok(Self(value))
    }

    /// Returns the key as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
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
}

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

#[cfg(test)]
mod tests {
    use super::{
        PermissionEntry, PermissionExpr, PermissionKey, PermissionKeyError, PermissionSet,
        PermissionState,
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
}
