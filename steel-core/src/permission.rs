//! Permission keys, expressions, and effective permission evaluation.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    ops::{BitAnd, BitOr},
    sync::Arc,
};

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use steel_utils::Identifier;
use steel_utils::locks::{AsyncMutex, SyncRwLock};
use uuid::Uuid;

/// Built-in operator group name used by `/op`.
pub(crate) const OP_GROUP: &str = "op";

/// Rule-side context where a permission entry applies.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PermissionRuleContext {
    /// Permission applies in every context.
    Global,
    /// Permission applies while the subject is in a domain.
    Domain(String),
    /// Permission applies while the subject is in one loaded world.
    World(Identifier),
    /// Permission applies when a plugin or subsystem provides the same custom context.
    Custom {
        /// Context key, such as `region`.
        key: PermissionSegment,
        /// Context value owned by the provider.
        value: String,
    },
}

impl PermissionRuleContext {
    /// Returns the global permission rule context.
    #[must_use]
    pub const fn global() -> Self {
        Self::Global
    }

    /// Creates a domain permission rule context.
    #[must_use]
    pub fn domain(domain: impl Into<String>) -> Self {
        Self::Domain(domain.into())
    }

    /// Creates a world permission rule context.
    #[must_use]
    pub const fn world(world: Identifier) -> Self {
        Self::World(world)
    }

    /// Creates a custom permission rule context.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is empty.
    pub fn custom(
        key: PermissionSegment,
        value: impl Into<String>,
    ) -> Result<Self, PermissionRuleContextError> {
        let value = value.into();
        if value.is_empty() {
            return Err(PermissionRuleContextError::EmptyValue);
        }
        Ok(Self::Custom { key, value })
    }

    /// Returns true if this entry applies in every context.
    #[must_use]
    pub const fn is_global(&self) -> bool {
        matches!(self, Self::Global)
    }

    fn matches_context(&self, context: &PermissionContext) -> bool {
        match self {
            Self::Global => true,
            Self::Domain(domain) => context.domain.as_ref() == Some(domain),
            Self::World(world) => context.world.as_ref() == Some(world),
            Self::Custom { .. } => context.custom_contexts.contains(self),
        }
    }

    fn specificity(&self) -> usize {
        match self {
            Self::Global => 0,
            Self::Domain(_) | Self::Custom { .. } => 1,
            Self::World(_) => 2,
        }
    }
}

impl fmt::Display for PermissionRuleContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Global => write!(f, "global"),
            Self::Domain(domain) => write!(f, "domain {domain}"),
            Self::World(world) => write!(f, "world {world}"),
            Self::Custom { key, value } => write!(f, "{} {value}", key.as_str()),
        }
    }
}

/// Invalid permission rule context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionRuleContextError {
    /// Context value is empty.
    EmptyValue,
}

impl fmt::Display for PermissionRuleContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyValue => write!(f, "permission context value is empty"),
        }
    }
}

impl Error for PermissionRuleContextError {}

/// Context used when evaluating a permission expression.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PermissionContext {
    domain: Option<String>,
    world: Option<Identifier>,
    custom_contexts: Vec<PermissionRuleContext>,
}

impl PermissionContext {
    /// Creates a context with no active rule contexts.
    #[must_use]
    pub fn global() -> Self {
        Self::default()
    }

    /// Creates a context for one loaded world and its domain.
    #[must_use]
    pub fn for_world(domain: impl Into<String>, world: Identifier) -> Self {
        Self {
            domain: Some(domain.into()),
            world: Some(world),
            custom_contexts: Vec::new(),
        }
    }

    /// Adds a custom active rule context.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is empty.
    pub fn with_custom_context(
        mut self,
        key: PermissionSegment,
        value: impl Into<String>,
    ) -> Result<Self, PermissionRuleContextError> {
        let context = PermissionRuleContext::custom(key, value)?;
        if !self
            .custom_contexts
            .iter()
            .any(|existing| existing == &context)
        {
            self.custom_contexts.push(context);
        }
        Ok(self)
    }
}

/// One dotted permission key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

    fn scopes(&self, key: &Self) -> bool {
        if self == key {
            return true;
        }

        key.0
            .strip_prefix(self.as_str())
            .is_some_and(|remaining| remaining.starts_with('.'))
    }
}

/// One non-wildcard segment in a permission key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    if segment.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
    }) {
        Ok(())
    } else {
        Err(PermissionKeyError::InvalidSegment)
    }
}

/// Source that registered a permission key for discovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PermissionCatalogSource {
    /// Permission owned by a registered command.
    Command,
    /// Permission already present in resolved group configuration.
    Config,
}

/// One discoverable permission key and the sources that registered it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionCatalogEntry {
    key: PermissionKey,
    sources: BTreeSet<PermissionCatalogSource>,
}

impl PermissionCatalogEntry {
    fn new(key: PermissionKey, source: PermissionCatalogSource) -> Self {
        let mut sources = BTreeSet::new();
        sources.insert(source);
        Self { key, sources }
    }

    /// Returns the permission key.
    #[must_use]
    pub const fn key(&self) -> &PermissionKey {
        &self.key
    }

    /// Returns the sources that registered this key.
    #[must_use]
    pub const fn sources(&self) -> &BTreeSet<PermissionCatalogSource> {
        &self.sources
    }
}

/// Registry of permission keys available for discovery and autocomplete.
///
/// The catalog is not an enforcement boundary. Permission checks still accept
/// any syntactically valid key so plugins and config can introduce nodes before
/// Steel has metadata for them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PermissionCatalog {
    entries: BTreeMap<PermissionKey, PermissionCatalogEntry>,
}

impl PermissionCatalog {
    /// Creates an empty permission catalog.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Registers one permission key for discovery.
    pub fn insert(&mut self, key: PermissionKey, source: PermissionCatalogSource) {
        self.entries
            .entry(key.clone())
            .and_modify(|entry| {
                entry.sources.insert(source);
            })
            .or_insert_with(|| PermissionCatalogEntry::new(key, source));
    }

    /// Merges another catalog into this one.
    pub fn extend(&mut self, other: &Self) {
        for entry in other.entries.values() {
            for source in &entry.sources {
                self.insert(entry.key.clone(), *source);
            }
        }
    }

    /// Returns all catalog entries sorted by permission key.
    pub fn entries(&self) -> impl Iterator<Item = &PermissionCatalogEntry> {
        self.entries.values()
    }

    /// Returns suggestion text for keys matching `prefix`.
    #[must_use]
    pub fn suggestions(&self, prefix: &str) -> Vec<String> {
        self.entries
            .keys()
            .filter_map(|key| {
                let key = key.as_str();
                key.starts_with(prefix).then(|| key.to_owned())
            })
            .collect()
    }
}

/// Persisted permission state for one player.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PermissionSubjectState {
    groups: Vec<String>,
    overrides: PermissionSet,
}

impl PermissionSubjectState {
    /// Creates a persisted permission-state snapshot.
    #[must_use]
    pub const fn new(groups: Vec<String>, overrides: PermissionSet) -> Self {
        Self { groups, overrides }
    }

    /// Returns assigned permission groups.
    #[must_use]
    pub fn groups(&self) -> &[String] {
        &self.groups
    }

    /// Returns direct permission overrides.
    #[must_use]
    pub const fn overrides(&self) -> &PermissionSet {
        &self.overrides
    }

    /// Splits this snapshot into owned groups and overrides.
    #[must_use]
    pub fn into_parts(self) -> (Vec<String>, PermissionSet) {
        (self.groups, self.overrides)
    }
}

/// In-memory index of persisted player permission state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PermissionSubjectIndex {
    states: BTreeMap<Uuid, PermissionSubjectState>,
}

impl PermissionSubjectIndex {
    /// Creates an empty index.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            states: BTreeMap::new(),
        }
    }

    /// Returns one player's cached permission state.
    #[must_use]
    pub fn get(&self, uuid: Uuid) -> Option<&PermissionSubjectState> {
        self.states.get(&uuid)
    }

    /// Inserts or replaces one player's cached permission state.
    pub fn set(&mut self, uuid: Uuid, state: PermissionSubjectState) {
        self.states.insert(uuid, state);
    }

    /// Returns all cached entries sorted by UUID.
    pub fn entries(&self) -> impl Iterator<Item = (Uuid, &PermissionSubjectState)> {
        self.states.iter().map(|(uuid, state)| (*uuid, state))
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
                    "permission segment must contain only lowercase letters, numbers, '_' or '-'"
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
    /// A key that may also be granted by a parent key.
    ///
    /// The parent is treated as a broad grant for this key, but entries that
    /// target this key or one of its wildcard ancestors are more specific and
    /// can override the parent. This is used for command paths such as
    /// `minecraft.command.gamemode` granting `minecraft.command.gamemode.creative`.
    ScopedKey {
        /// Broad parent permission.
        parent: PermissionKey,
        /// Requested child permission.
        key: PermissionKey,
    },
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

    /// Creates a key expression that may also be granted by `parent`.
    #[must_use]
    pub const fn scoped_key(parent: PermissionKey, key: PermissionKey) -> Self {
        Self::ScopedKey { parent, key }
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
    context: PermissionRuleContext,
    state: PermissionState,
}

impl PermissionEntry {
    /// Creates one global permission entry.
    #[must_use]
    pub const fn new(key: PermissionKey, state: PermissionState) -> Self {
        Self {
            key,
            context: PermissionRuleContext::Global,
            state,
        }
    }

    /// Creates one contextual permission entry.
    #[must_use]
    pub const fn new_with_context(
        key: PermissionKey,
        context: PermissionRuleContext,
        state: PermissionState,
    ) -> Self {
        Self {
            key,
            context,
            state,
        }
    }

    /// Creates a global allow entry.
    #[must_use]
    pub const fn allow(key: PermissionKey) -> Self {
        Self::new(key, PermissionState::Allow)
    }

    /// Creates a contextual allow entry.
    #[must_use]
    pub const fn allow_with_context(key: PermissionKey, context: PermissionRuleContext) -> Self {
        Self::new_with_context(key, context, PermissionState::Allow)
    }

    /// Creates a global deny entry.
    #[must_use]
    pub const fn deny(key: PermissionKey) -> Self {
        Self::new(key, PermissionState::Deny)
    }

    /// Creates a contextual deny entry.
    #[must_use]
    pub const fn deny_with_context(key: PermissionKey, context: PermissionRuleContext) -> Self {
        Self::new_with_context(key, context, PermissionState::Deny)
    }

    /// Returns the permission key pattern.
    #[must_use]
    pub const fn key(&self) -> &PermissionKey {
        &self.key
    }

    /// Returns this rule's context.
    #[must_use]
    pub const fn context(&self) -> &PermissionRuleContext {
        &self.context
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

    /// Adds one contextual allow entry.
    pub fn allow_in(&mut self, key: PermissionKey, context: PermissionRuleContext) {
        self.push(PermissionEntry::allow_with_context(key, context));
    }

    /// Adds one deny entry.
    pub fn deny(&mut self, key: PermissionKey) {
        self.push(PermissionEntry::deny(key));
    }

    /// Adds one contextual deny entry.
    pub fn deny_in(&mut self, key: PermissionKey, context: PermissionRuleContext) {
        self.push(PermissionEntry::deny_with_context(key, context));
    }

    /// Sets one exact global permission entry, replacing any previous exact entry.
    pub fn set(&mut self, key: PermissionKey, state: PermissionState) {
        self.set_in(key, PermissionRuleContext::Global, state);
    }

    /// Sets one exact contextual permission entry, replacing any previous exact entry in that context.
    pub fn set_in(
        &mut self,
        key: PermissionKey,
        context: PermissionRuleContext,
        state: PermissionState,
    ) {
        self.entries
            .retain(|entry| entry.key != key || entry.context != context);
        self.entries
            .push(PermissionEntry::new_with_context(key, context, state));
    }

    /// Removes one exact global permission entry.
    ///
    /// Returns true when an entry was removed.
    pub fn unset(&mut self, key: &PermissionKey) -> bool {
        self.unset_in(key, &PermissionRuleContext::Global)
    }

    /// Removes one exact contextual permission entry.
    ///
    /// Returns true when an entry was removed.
    pub fn unset_in(&mut self, key: &PermissionKey, context: &PermissionRuleContext) -> bool {
        let old_len = self.entries.len();
        self.entries
            .retain(|entry| entry.key() != key || entry.context() != context);
        self.entries.len() != old_len
    }

    /// Resolves one key in the global context. Unset permissions return `None`.
    #[must_use]
    pub fn resolve_key(&self, key: &PermissionKey) -> Option<PermissionState> {
        self.resolve_key_in(key, &PermissionContext::global())
    }

    /// Resolves one key in a permission context. Unset permissions return `None`.
    #[must_use]
    pub fn resolve_key_in(
        &self,
        key: &PermissionKey,
        context: &PermissionContext,
    ) -> Option<PermissionState> {
        let mut best = None;

        for entry in &self.entries {
            if !entry.context.matches_context(context) {
                continue;
            }
            if !entry.key.matches(key) {
                continue;
            }

            push_permission_candidate(
                &mut best,
                entry.key.specificity(),
                entry.context.specificity(),
                entry.state,
            );
        }

        best.map(|(_, _, state)| state)
    }

    /// Resolves a child key in the global context while treating `parent` as a broad grant.
    ///
    /// Unset permissions return `None`. If both parent and child-side entries
    /// match, the more specific entry wins; tied specificity resolves to deny.
    #[must_use]
    pub fn resolve_scoped_key(
        &self,
        parent: &PermissionKey,
        key: &PermissionKey,
    ) -> Option<PermissionState> {
        self.resolve_scoped_key_in(parent, key, &PermissionContext::global())
    }

    /// Resolves a child key in a permission context while treating `parent` as a broad grant.
    ///
    /// Unset permissions return `None`. If both parent and child-side entries
    /// match, the more specific entry wins; tied specificity resolves to deny.
    #[must_use]
    pub fn resolve_scoped_key_in(
        &self,
        parent: &PermissionKey,
        key: &PermissionKey,
        context: &PermissionContext,
    ) -> Option<PermissionState> {
        let mut best = None;
        let parent_scopes_key = parent.scopes(key);

        for entry in &self.entries {
            if !entry.context.matches_context(context) {
                continue;
            }
            let matches_parent = parent_scopes_key && entry.key.matches(parent);
            let matches_key = entry.key.matches(key);
            if !matches_parent && !matches_key {
                continue;
            }

            let mut specificity = entry.key.specificity();
            if matches_key && !matches_parent {
                specificity += 1;
            }
            push_permission_candidate(
                &mut best,
                specificity,
                entry.context.specificity(),
                entry.state,
            );
        }

        best.map(|(_, _, state)| state)
    }

    /// Returns whether a key is allowed in the global context. Unset defaults to deny.
    #[must_use]
    pub fn allows_key(&self, key: &PermissionKey) -> bool {
        self.resolve_key(key) == Some(PermissionState::Allow)
    }

    /// Returns whether a key is allowed in a permission context. Unset defaults to deny.
    #[must_use]
    pub fn allows_key_in(&self, key: &PermissionKey, context: &PermissionContext) -> bool {
        self.resolve_key_in(key, context) == Some(PermissionState::Allow)
    }

    /// Returns whether a child key is allowed through itself or a broad parent in the global context.
    #[must_use]
    pub fn allows_scoped_key(&self, parent: &PermissionKey, key: &PermissionKey) -> bool {
        self.resolve_scoped_key(parent, key) == Some(PermissionState::Allow)
    }

    /// Returns whether a child key is allowed through itself or a broad parent in a permission context.
    #[must_use]
    pub fn allows_scoped_key_in(
        &self,
        parent: &PermissionKey,
        key: &PermissionKey,
        context: &PermissionContext,
    ) -> bool {
        self.resolve_scoped_key_in(parent, key, context) == Some(PermissionState::Allow)
    }

    /// Returns whether this set allows a permission expression in the global context.
    #[must_use]
    pub fn allows(&self, permission: &PermissionExpr) -> bool {
        self.allows_in(permission, &PermissionContext::global())
    }

    /// Returns whether this set allows a permission expression in a permission context.
    #[must_use]
    pub fn allows_in(&self, permission: &PermissionExpr, context: &PermissionContext) -> bool {
        match permission {
            PermissionExpr::Key(key) => self.allows_key_in(key, context),
            PermissionExpr::ScopedKey { parent, key } => {
                self.allows_scoped_key_in(parent, key, context)
            }
            PermissionExpr::All(children) => {
                children.iter().all(|child| self.allows_in(child, context))
            }
            PermissionExpr::Any(children) => {
                children.iter().any(|child| self.allows_in(child, context))
            }
        }
    }
}

fn push_permission_candidate(
    best: &mut Option<(usize, usize, PermissionState)>,
    key_specificity: usize,
    context_specificity: usize,
    state: PermissionState,
) {
    match best {
        None => *best = Some((key_specificity, context_specificity, state)),
        Some((best_key_specificity, best_context_specificity, _))
            if (key_specificity, context_specificity)
                > (*best_key_specificity, *best_context_specificity) =>
        {
            *best = Some((key_specificity, context_specificity, state));
        }
        Some((best_key_specificity, best_context_specificity, PermissionState::Allow))
            if (key_specificity, context_specificity)
                == (*best_key_specificity, *best_context_specificity)
                && state == PermissionState::Deny =>
        {
            *best = Some((key_specificity, context_specificity, PermissionState::Deny));
        }
        _ => {}
    }
}

/// Parsed `groups.toml` permissions configuration.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
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
            OP_GROUP.to_owned(),
            PermissionGroupConfig {
                allow: vec!["*".to_owned()],
                deny: Vec::new(),
                rules: Vec::new(),
            },
        );

        Self {
            default_groups: vec!["default".to_owned()],
            groups,
        }
    }
}

/// One configured permission group.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionGroupConfig {
    /// Permission keys explicitly allowed by this group.
    pub allow: Vec<String>,
    /// Permission keys explicitly denied by this group.
    pub deny: Vec<String>,
    /// Structured permission rules with optional contexts.
    pub rules: Vec<PermissionRuleConfig>,
}

/// One structured `groups.toml` permission rule.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PermissionRuleConfig {
    /// Permission key pattern affected by this rule.
    pub key: String,
    /// Whether this rule allows or denies the key.
    pub state: PermissionRuleStateConfig,
    /// Context where this rule applies. Omitted means global.
    pub context: Option<PermissionRuleContextConfig>,
}

/// Configured permission rule state.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PermissionRuleStateConfig {
    /// Explicitly allow the matching permission.
    Allow,
    /// Explicitly deny the matching permission.
    Deny,
}

impl PermissionRuleStateConfig {
    const fn permission_state(self) -> PermissionState {
        match self {
            Self::Allow => PermissionState::Allow,
            Self::Deny => PermissionState::Deny,
        }
    }
}

/// Configured rule-side context for one permission rule.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionRuleContextConfig {
    /// Domain where the rule applies.
    pub domain: Option<String>,
    /// Loaded world where the rule applies. Must be a namespaced world id.
    pub world: Option<String>,
}

impl PermissionRuleContextConfig {
    fn into_rule_context(self) -> Result<PermissionRuleContext, PermissionRuleContextConfigError> {
        match (self.domain, self.world) {
            (None, None) => Err(PermissionRuleContextConfigError::EmptyContext),
            (Some(domain), None) => {
                if domain.is_empty() || !Identifier::validate_namespace(&domain) {
                    return Err(PermissionRuleContextConfigError::InvalidDomain(domain));
                }
                Ok(PermissionRuleContext::domain(domain))
            }
            (None, Some(world)) => {
                parse_loaded_world_context(world).map(PermissionRuleContext::world)
            }
            (Some(_), Some(_)) => Err(PermissionRuleContextConfigError::MultipleContexts),
        }
    }
}

fn parse_loaded_world_context(
    world: String,
) -> Result<Identifier, PermissionRuleContextConfigError> {
    let Some((domain, name)) = world.split_once(':') else {
        return Err(PermissionRuleContextConfigError::InvalidWorld(world));
    };
    if domain.is_empty()
        || name.is_empty()
        || name.contains(':')
        || name.contains('/')
        || !Identifier::validate_namespace(domain)
        || !Identifier::validate_path(name)
    {
        return Err(PermissionRuleContextConfigError::InvalidWorld(world));
    }

    Ok(Identifier::new(domain.to_owned(), name.to_owned()))
}

/// Persists permission group configuration owned outside `steel-core`.
pub trait PermissionGroupStore: Send + Sync {
    /// Saves the complete permission group config.
    fn save_groups(
        &self,
        config: PermissionGroupsConfig,
    ) -> BoxFuture<'static, Result<(), PermissionGroupStoreError>>;
}

/// Permission group store failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionGroupStoreError {
    message: String,
}

impl PermissionGroupStoreError {
    /// Creates a store error with a displayable message.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for PermissionGroupStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for PermissionGroupStoreError {}

/// Permission group manager update failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionGroupManagerError {
    /// Updated config does not resolve to valid permission groups.
    Config(PermissionConfigError),
    /// Updated config could not be persisted.
    Store(PermissionGroupStoreError),
}

impl fmt::Display for PermissionGroupManagerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => write!(f, "invalid permission groups config: {error}"),
            Self::Store(error) => write!(f, "failed to store permission groups config: {error}"),
        }
    }
}

impl Error for PermissionGroupManagerError {}

impl From<PermissionConfigError> for PermissionGroupManagerError {
    fn from(value: PermissionConfigError) -> Self {
        Self::Config(value)
    }
}

impl From<PermissionGroupStoreError> for PermissionGroupManagerError {
    fn from(value: PermissionGroupStoreError) -> Self {
        Self::Store(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PermissionGroupManagerState {
    config: PermissionGroupsConfig,
    groups: PermissionGroups,
}

/// Runtime permission group state with optional config persistence.
pub struct PermissionGroupManager {
    updates: AsyncMutex<()>,
    state: SyncRwLock<PermissionGroupManagerState>,
    store: Option<Arc<dyn PermissionGroupStore>>,
}

impl fmt::Debug for PermissionGroupManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PermissionGroupManager")
            .field("state", &self.state)
            .field("has_store", &self.store.is_some())
            .finish()
    }
}

impl PermissionGroupManager {
    /// Builds a manager from typed config and an optional persistence store.
    ///
    /// # Errors
    ///
    /// Returns an error when the provided config does not resolve.
    pub fn new(
        config: PermissionGroupsConfig,
        store: Option<Arc<dyn PermissionGroupStore>>,
    ) -> Result<Self, PermissionConfigError> {
        let groups = PermissionGroups::from_config(config.clone())?;
        Ok(Self {
            updates: AsyncMutex::new(()),
            state: SyncRwLock::new(PermissionGroupManagerState { config, groups }),
            store,
        })
    }

    /// Builds a manager without persistence.
    ///
    /// # Errors
    ///
    /// Returns an error when the provided config does not resolve.
    pub fn transient(config: PermissionGroupsConfig) -> Result<Self, PermissionConfigError> {
        Self::new(config, None)
    }

    /// Returns a snapshot of the current typed config.
    #[must_use]
    pub fn config_snapshot(&self) -> PermissionGroupsConfig {
        self.state.read().config.clone()
    }

    /// Returns whether a group exists.
    #[must_use]
    pub fn contains_group(&self, group: &str) -> bool {
        self.state.read().groups.contains_group(group)
    }

    /// Returns configured group names sorted by group name.
    #[must_use]
    pub fn group_names(&self) -> Vec<String> {
        self.state.read().groups.groups().keys().cloned().collect()
    }

    /// Adds configured group permission keys to a discovery catalog.
    pub fn register_catalog_entries(&self, catalog: &mut PermissionCatalog) {
        self.state.read().groups.register_catalog_entries(catalog);
    }

    /// Builds an effective permission set from defaults, assigned groups, and player overrides.
    #[must_use]
    pub fn effective_permissions(
        &self,
        assigned_groups: &[String],
        player_permissions: &PermissionSet,
    ) -> PermissionSet {
        self.state
            .read()
            .groups
            .effective_permissions(assigned_groups, player_permissions)
    }

    /// Replaces the complete group config after validation and optional persistence.
    ///
    /// # Errors
    ///
    /// Returns an error if the config is invalid or if the configured store rejects the save.
    pub async fn replace_config(
        &self,
        config: PermissionGroupsConfig,
    ) -> Result<(), PermissionGroupManagerError> {
        let _guard = self.updates.lock().await;
        let groups = PermissionGroups::from_config(config.clone())?;
        if let Some(store) = &self.store {
            store.save_groups(config.clone()).await?;
        }

        *self.state.write() = PermissionGroupManagerState { config, groups };
        Ok(())
    }
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
            OP_GROUP.to_owned(),
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
            validate_group_name(group)?;
            if !config.groups.contains_key(group) {
                return Err(PermissionConfigError::MissingDefaultGroup(group.clone()));
            }
        }
        if !config.groups.contains_key(OP_GROUP) {
            return Err(PermissionConfigError::MissingRequiredGroup(
                OP_GROUP.to_owned(),
            ));
        }

        let mut groups = BTreeMap::new();
        for (name, group) in config.groups {
            validate_group_name(&name)?;
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
            for rule in group.rules {
                let key = PermissionKey::parse(rule.key).map_err(|source| {
                    PermissionConfigError::InvalidPermissionKey {
                        group: name.clone(),
                        source,
                    }
                })?;
                let context = rule
                    .context
                    .map_or(Ok(PermissionRuleContext::Global), |context| {
                        context.into_rule_context()
                    })
                    .map_err(|source| PermissionConfigError::InvalidRuleContext {
                        group: name.clone(),
                        source,
                    })?;
                permissions.push(PermissionEntry::new_with_context(
                    key,
                    context,
                    rule.state.permission_state(),
                ));
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

    /// Adds configured group permission keys to a discovery catalog.
    pub fn register_catalog_entries(&self, catalog: &mut PermissionCatalog) {
        for group in self.groups.values() {
            for entry in group.permissions.entries() {
                catalog.insert(entry.key.clone(), PermissionCatalogSource::Config);
            }
        }
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
            effective.set_in(entry.key().clone(), entry.context().clone(), entry.state());
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

fn validate_group_name(group: &str) -> Result<(), PermissionConfigError> {
    PermissionSegment::parse(group).map_or_else(
        |source| {
            Err(PermissionConfigError::InvalidGroupName {
                group: group.to_owned(),
                source,
            })
        },
        |_| Ok(()),
    )
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
    /// A built-in required group name does not exist in the group map.
    MissingRequiredGroup(String),
    /// A group contains an invalid permission key.
    InvalidPermissionKey {
        /// Group containing the bad key.
        group: String,
        /// Parse error.
        source: PermissionKeyError,
    },
    /// A group name is not command-usable.
    InvalidGroupName {
        /// Invalid group name.
        group: String,
        /// Parse error.
        source: PermissionKeyError,
    },
    /// A structured rule context is invalid.
    InvalidRuleContext {
        /// Group containing the bad rule.
        group: String,
        /// Parse error.
        source: PermissionRuleContextConfigError,
    },
}

/// Invalid configured permission rule context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionRuleContextConfigError {
    /// A present context table must declare one selector.
    EmptyContext,
    /// A rule can only declare one context selector for now.
    MultipleContexts,
    /// Domain name is not valid.
    InvalidDomain(String),
    /// World identifier is not valid.
    InvalidWorld(String),
}

impl fmt::Display for PermissionConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingDefaultGroup(group) => {
                write!(f, "default permission group '{group}' is not configured")
            }
            Self::MissingRequiredGroup(group) => {
                write!(f, "required permission group '{group}' is not configured")
            }
            Self::InvalidPermissionKey { group, source } => {
                write!(
                    f,
                    "permission group '{group}' contains invalid key: {source}"
                )
            }
            Self::InvalidGroupName { group, source } => {
                write!(f, "permission group name '{group}' is invalid: {source}")
            }
            Self::InvalidRuleContext { group, source } => {
                write!(
                    f,
                    "permission group '{group}' contains invalid rule context: {source}"
                )
            }
        }
    }
}

impl Error for PermissionConfigError {}

impl fmt::Display for PermissionRuleContextConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyContext => write!(f, "rule context must contain one selector"),
            Self::MultipleContexts => write!(f, "rule context must contain only one selector"),
            Self::InvalidDomain(domain) => write!(f, "invalid domain context '{domain}'"),
            Self::InvalidWorld(world) => write!(f, "invalid world context '{world}'"),
        }
    }
}

impl Error for PermissionRuleContextConfigError {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        PermissionCatalog, PermissionCatalogSource, PermissionEntry, PermissionExpr,
        PermissionGroupManager, PermissionGroupManagerError, PermissionGroups,
        PermissionGroupsConfig, PermissionKey, PermissionKeyError, PermissionRuleContext,
        PermissionSegment, PermissionSet, PermissionState,
    };
    use steel_utils::Identifier;
    use steel_utils::locks::SyncMutex;

    #[derive(Clone, Debug)]
    struct CapturingGroupStore {
        saved: Arc<SyncMutex<Vec<PermissionGroupsConfig>>>,
    }

    impl super::PermissionGroupStore for CapturingGroupStore {
        fn save_groups(
            &self,
            config: PermissionGroupsConfig,
        ) -> futures::future::BoxFuture<'static, Result<(), super::PermissionGroupStoreError>>
        {
            let saved = Arc::clone(&self.saved);
            Box::pin(async move {
                saved.lock().push(config);
                Ok(())
            })
        }
    }

    #[derive(Clone, Debug)]
    struct FailingGroupStore;

    impl super::PermissionGroupStore for FailingGroupStore {
        fn save_groups(
            &self,
            _config: PermissionGroupsConfig,
        ) -> futures::future::BoxFuture<'static, Result<(), super::PermissionGroupStoreError>>
        {
            Box::pin(async {
                Err(super::PermissionGroupStoreError::new(
                    "configured store failure",
                ))
            })
        }
    }

    fn key(value: &str) -> PermissionKey {
        PermissionKey::parse(value).expect("key parses")
    }

    fn world_context(domain: &str, world: &str) -> super::PermissionContext {
        super::PermissionContext::for_world(
            domain.to_owned(),
            Identifier::new(domain.to_owned(), world.to_owned()),
        )
    }

    fn config_with_builder_group() -> PermissionGroupsConfig {
        let mut config = PermissionGroupsConfig::default();
        config.groups.insert(
            "builder".to_owned(),
            super::PermissionGroupConfig {
                allow: vec!["steel.build".to_owned()],
                deny: Vec::new(),
                rules: Vec::new(),
            },
        );
        config
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
            PermissionKey::parse("Minecraft.command.give").err(),
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
    fn contextual_permissions_only_apply_in_matching_context() {
        let fly = key("steel.fly");
        let permissions = PermissionSet::from_entries([PermissionEntry::allow_with_context(
            fly.clone(),
            PermissionRuleContext::domain("lobby"),
        )]);

        assert!(permissions.allows_key_in(&fly, &world_context("lobby", "spawn")));
        assert!(!permissions.allows_key_in(&fly, &world_context("survival", "overworld")));
        assert!(!permissions.allows_key(&fly));
    }

    #[test]
    fn contextual_exact_entry_overrides_global_exact_entry() {
        let fly = key("steel.fly");
        let permissions = PermissionSet::from_entries([
            PermissionEntry::deny(fly.clone()),
            PermissionEntry::allow_with_context(
                fly.clone(),
                PermissionRuleContext::domain("lobby"),
            ),
        ]);

        assert!(permissions.allows_key_in(&fly, &world_context("lobby", "spawn")));
        assert!(!permissions.allows_key_in(&fly, &world_context("survival", "overworld")));
    }

    #[test]
    fn more_specific_key_overrides_broader_contextual_entry() {
        let creative = key("minecraft.command.gamemode.creative");
        let permissions = PermissionSet::from_entries([
            PermissionEntry::allow_with_context(
                key("minecraft.command.*"),
                PermissionRuleContext::domain("lobby"),
            ),
            PermissionEntry::deny(creative.clone()),
        ]);

        assert!(!permissions.allows_key_in(&creative, &world_context("lobby", "spawn")));
    }

    #[test]
    fn scoped_key_specific_entry_overrides_parent_grant() {
        let permissions = PermissionSet::from_entries([
            PermissionEntry::allow(key("minecraft.command.gamemode")),
            PermissionEntry::deny(key("minecraft.command.gamemode.creative")),
        ]);
        let parent = key("minecraft.command.gamemode");
        let creative = key("minecraft.command.gamemode.creative");
        let survival = key("minecraft.command.gamemode.survival");

        assert_eq!(
            permissions.resolve_scoped_key(&parent, &creative),
            Some(PermissionState::Deny)
        );
        assert!(!permissions.allows(&PermissionExpr::scoped_key(parent.clone(), creative)));
        assert!(permissions.allows(&PermissionExpr::scoped_key(parent, survival)));
    }

    #[test]
    fn scoped_child_entry_overrides_parent_grant_in_matching_context() {
        let permissions = PermissionSet::from_entries([
            PermissionEntry::allow(key("minecraft.command.gamemode")),
            PermissionEntry::deny_with_context(
                key("minecraft.command.gamemode.creative"),
                PermissionRuleContext::domain("lobby"),
            ),
        ]);
        let expression = PermissionExpr::scoped_key(
            key("minecraft.command.gamemode"),
            key("minecraft.command.gamemode.creative"),
        );

        assert!(!permissions.allows_in(&expression, &world_context("lobby", "spawn")));
        assert!(permissions.allows_in(&expression, &world_context("survival", "overworld")));
    }

    #[test]
    fn scoped_key_specific_grant_overrides_parent_deny() {
        let permissions = PermissionSet::from_entries([
            PermissionEntry::deny(key("minecraft.command.gamemode")),
            PermissionEntry::allow(key("minecraft.command.gamemode.creative")),
        ]);

        assert!(permissions.allows(&PermissionExpr::scoped_key(
            key("minecraft.command.gamemode"),
            key("minecraft.command.gamemode.creative")
        )));
    }

    #[test]
    fn scoped_key_descendant_wildcard_overrides_parent() {
        let permissions = PermissionSet::from_entries([
            PermissionEntry::deny(key("minecraft.command.gamemode")),
            PermissionEntry::allow(key("minecraft.command.gamemode.*")),
        ]);

        assert!(permissions.allows(&PermissionExpr::scoped_key(
            key("minecraft.command.gamemode"),
            key("minecraft.command.gamemode.creative")
        )));
    }

    #[test]
    fn scoped_key_parent_does_not_grant_unrelated_key() {
        let permissions = PermissionSet::from_entries([PermissionEntry::allow(key(
            "minecraft.command.gamemode",
        ))]);

        assert!(!permissions.allows(&PermissionExpr::scoped_key(
            key("minecraft.command.gamemode"),
            key("steel.command.fly")
        )));
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
    fn set_in_replaces_only_matching_context() {
        let fly = key("steel.fly");
        let mut permissions = PermissionSet::from_entries([
            PermissionEntry::allow(fly.clone()),
            PermissionEntry::deny_with_context(fly.clone(), PermissionRuleContext::domain("lobby")),
        ]);

        permissions.set_in(
            fly.clone(),
            PermissionRuleContext::domain("lobby"),
            PermissionState::Allow,
        );

        assert!(permissions.allows_key(&fly));
        assert!(permissions.allows_key_in(&fly, &world_context("lobby", "spawn")));
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
    fn unset_in_removes_only_matching_context() {
        let fly = key("steel.fly");
        let mut permissions = PermissionSet::from_entries([
            PermissionEntry::allow(fly.clone()),
            PermissionEntry::deny_with_context(fly.clone(), PermissionRuleContext::domain("lobby")),
        ]);

        assert!(permissions.unset_in(&fly, &PermissionRuleContext::domain("lobby")));
        assert!(!permissions.unset_in(&fly, &PermissionRuleContext::domain("lobby")));

        assert!(permissions.allows_key(&fly));
        assert!(permissions.allows_key_in(&fly, &world_context("lobby", "spawn")));
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
    fn permission_catalog_merges_sources_and_suggests_sorted_keys() {
        let mut catalog = PermissionCatalog::new();
        catalog.insert(
            key("steel.command.steelperms"),
            PermissionCatalogSource::Command,
        );
        catalog.insert(
            key("steel.command.steelperms"),
            PermissionCatalogSource::Config,
        );
        catalog.insert(
            key("minecraft.command.gamemode"),
            PermissionCatalogSource::Command,
        );

        assert_eq!(
            catalog.suggestions("steel.command"),
            vec!["steel.command.steelperms"]
        );
        let entry = catalog
            .entries()
            .find(|entry| entry.key().as_str() == "steel.command.steelperms")
            .expect("catalog entry exists");
        assert!(entry.sources().contains(&PermissionCatalogSource::Command));
        assert!(entry.sources().contains(&PermissionCatalogSource::Config));
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
    fn group_names_must_be_command_usable_segments() {
        let mut config = PermissionGroupsConfig::default();
        config.groups.insert(
            "Admin Group".to_owned(),
            super::PermissionGroupConfig::default(),
        );

        assert!(matches!(
            PermissionGroups::from_config(config),
            Err(super::PermissionConfigError::InvalidGroupName { group, .. })
                if group == "Admin Group"
        ));
    }

    #[test]
    fn op_group_is_required_for_builtin_op_command() {
        let mut config = PermissionGroupsConfig::default();
        config.groups.remove(super::OP_GROUP);

        assert!(matches!(
            PermissionGroups::from_config(config),
            Err(super::PermissionConfigError::MissingRequiredGroup(group))
                if group == super::OP_GROUP
        ));
    }

    #[test]
    fn groups_register_config_permissions_in_catalog() {
        let groups = PermissionGroups::from_config(PermissionGroupsConfig::default())
            .expect("default groups config is valid");
        let mut catalog = PermissionCatalog::new();

        groups.register_catalog_entries(&mut catalog);

        assert_eq!(catalog.suggestions("*"), vec!["*"]);
        assert!(
            catalog
                .entries()
                .all(|entry| entry.sources().contains(&PermissionCatalogSource::Config))
        );
    }

    #[tokio::test]
    async fn permission_group_manager_persists_replacements_before_swapping() {
        let saved = Arc::new(SyncMutex::new(Vec::new()));
        let manager = PermissionGroupManager::new(
            PermissionGroupsConfig::default(),
            Some(Arc::new(CapturingGroupStore {
                saved: Arc::clone(&saved),
            })),
        )
        .expect("default groups config resolves");
        let config = config_with_builder_group();

        manager
            .replace_config(config.clone())
            .await
            .expect("replacement config stores and swaps");

        assert!(manager.contains_group("builder"));
        assert_eq!(&*saved.lock(), &vec![config]);
    }

    #[tokio::test]
    async fn permission_group_manager_keeps_state_when_store_fails() {
        let manager = PermissionGroupManager::new(
            PermissionGroupsConfig::default(),
            Some(Arc::new(FailingGroupStore)),
        )
        .expect("default groups config resolves");

        let error = manager
            .replace_config(config_with_builder_group())
            .await
            .expect_err("store failure should reject replacement");

        assert!(matches!(error, PermissionGroupManagerError::Store(_)));
        assert!(!manager.contains_group("builder"));
    }

    #[test]
    fn groups_support_contextual_rules() {
        let mut config = PermissionGroupsConfig::default();
        let default_group = config
            .groups
            .get_mut("default")
            .expect("default group exists");
        default_group.rules.push(super::PermissionRuleConfig {
            key: "steel.chat".to_owned(),
            state: super::PermissionRuleStateConfig::Allow,
            context: None,
        });
        default_group.rules.push(super::PermissionRuleConfig {
            key: "steel.fly".to_owned(),
            state: super::PermissionRuleStateConfig::Allow,
            context: Some(super::PermissionRuleContextConfig {
                domain: Some("lobby".to_owned()),
                world: None,
            }),
        });
        let groups = PermissionGroups::from_config(config).expect("groups config is valid");
        let effective = groups.effective_permissions(&[], &PermissionSet::new());

        assert!(effective.allows_key(&key("steel.chat")));
        assert!(effective.allows_key_in(&key("steel.fly"), &world_context("lobby", "spawn")));
        assert!(
            !effective.allows_key_in(&key("steel.fly"), &world_context("survival", "overworld"))
        );
        assert!(!effective.allows_key(&key("steel.fly")));
    }

    #[test]
    fn group_rules_reject_ambiguous_contexts() {
        let mut config = PermissionGroupsConfig::default();
        let default_group = config
            .groups
            .get_mut("default")
            .expect("default group exists");
        default_group.rules.push(super::PermissionRuleConfig {
            key: "steel.fly".to_owned(),
            state: super::PermissionRuleStateConfig::Allow,
            context: Some(super::PermissionRuleContextConfig {
                domain: Some("lobby".to_owned()),
                world: Some("lobby:spawn".to_owned()),
            }),
        });

        assert!(matches!(
            PermissionGroups::from_config(config),
            Err(super::PermissionConfigError::InvalidRuleContext {
                group,
                source: super::PermissionRuleContextConfigError::MultipleContexts,
            }) if group == "default"
        ));
    }

    #[test]
    fn group_rules_reject_empty_contexts() {
        let mut config = PermissionGroupsConfig::default();
        let default_group = config
            .groups
            .get_mut("default")
            .expect("default group exists");
        default_group.rules.push(super::PermissionRuleConfig {
            key: "steel.fly".to_owned(),
            state: super::PermissionRuleStateConfig::Allow,
            context: Some(super::PermissionRuleContextConfig::default()),
        });

        assert!(matches!(
            PermissionGroups::from_config(config),
            Err(super::PermissionConfigError::InvalidRuleContext {
                group,
                source: super::PermissionRuleContextConfigError::EmptyContext,
            }) if group == "default"
        ));
    }

    #[test]
    fn group_rules_support_loaded_world_contexts() {
        let mut config = PermissionGroupsConfig::default();
        let default_group = config
            .groups
            .get_mut("default")
            .expect("default group exists");
        default_group.rules.push(super::PermissionRuleConfig {
            key: "steel.fly".to_owned(),
            state: super::PermissionRuleStateConfig::Allow,
            context: Some(super::PermissionRuleContextConfig {
                domain: None,
                world: Some("lobby:spawn".to_owned()),
            }),
        });
        let groups = PermissionGroups::from_config(config).expect("groups config is valid");
        let effective = groups.effective_permissions(&[], &PermissionSet::new());

        assert!(effective.allows_key_in(&key("steel.fly"), &world_context("lobby", "spawn")));
        assert!(!effective.allows_key_in(&key("steel.fly"), &world_context("lobby", "creative")));
    }

    #[test]
    fn group_rules_reject_invalid_loaded_world_contexts() {
        let mut config = PermissionGroupsConfig::default();
        let default_group = config
            .groups
            .get_mut("default")
            .expect("default group exists");
        default_group.rules.push(super::PermissionRuleConfig {
            key: "steel.fly".to_owned(),
            state: super::PermissionRuleStateConfig::Allow,
            context: Some(super::PermissionRuleContextConfig {
                domain: None,
                world: Some("lobby:spawn/extra".to_owned()),
            }),
        });

        assert!(matches!(
            PermissionGroups::from_config(config),
            Err(super::PermissionConfigError::InvalidRuleContext {
                group,
                source: super::PermissionRuleContextConfigError::InvalidWorld(world),
            }) if group == "default" && world == "lobby:spawn/extra"
        ));
    }

    #[test]
    fn player_permissions_override_exact_group_entries() {
        let mut config = PermissionGroupsConfig::default();
        let default_group = config
            .groups
            .get_mut("default")
            .expect("default group exists");
        default_group.deny.push("steel.admin".to_owned());
        default_group.allow.push("steel.fly".to_owned());
        let groups = PermissionGroups::from_config(config).expect("groups config is valid");

        let player_permissions = PermissionSet::from_entries([
            PermissionEntry::allow(key("steel.admin")),
            PermissionEntry::deny(key("steel.fly")),
        ]);
        let effective = groups.effective_permissions(&[], &player_permissions);

        assert!(effective.allows_key(&key("steel.admin")));
        assert!(!effective.allows_key(&key("steel.fly")));
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
