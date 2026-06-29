//! Permission keys, expressions, and effective permission evaluation.

use std::{
    cmp::Ordering,
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
        /// Context key, such as `plugin:region`.
        key: PermissionContextKey,
        /// Context value owned by the provider.
        value: String,
    },
    /// Permission applies when every nested context applies.
    All(PermissionRuleContexts),
}

/// Validated AND-chain of permission rule contexts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PermissionRuleContexts {
    contexts: Vec<PermissionRuleContext>,
}

impl PermissionRuleContexts {
    fn new(contexts: Vec<PermissionRuleContext>) -> Self {
        Self { contexts }
    }

    fn into_vec(self) -> Vec<PermissionRuleContext> {
        self.contexts
    }

    /// Returns chained contexts in canonical order.
    pub fn iter(&self) -> impl Iterator<Item = &PermissionRuleContext> {
        self.contexts.iter()
    }
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
        key: PermissionContextKey,
        value: impl Into<String>,
    ) -> Result<Self, PermissionRuleContextError> {
        let value = value.into();
        if value.is_empty() {
            return Err(PermissionRuleContextError::EmptyValue);
        }
        Ok(Self::Custom { key, value })
    }

    /// Creates an AND-chain of rule contexts.
    ///
    /// # Errors
    ///
    /// Returns an error when two custom contexts use the same key with different values.
    pub fn all(
        contexts: impl IntoIterator<Item = Self>,
    ) -> Result<Self, PermissionRuleContextError> {
        let mut flattened = Vec::new();
        for context in contexts {
            match context {
                Self::Global => {}
                Self::All(contexts) => {
                    for context in contexts.into_vec() {
                        push_unique_context(&mut flattened, context)?;
                    }
                }
                context => push_unique_context(&mut flattened, context)?,
            }
        }
        flattened.sort_by(compare_rule_contexts);

        Ok(match flattened.len() {
            0 => Self::Global,
            1 => flattened.pop().unwrap_or(Self::Global),
            _ => Self::All(PermissionRuleContexts::new(flattened)),
        })
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
            Self::All(contexts) => contexts
                .iter()
                .all(|constraint| constraint.matches_context(context)),
        }
    }

    fn specificity(&self) -> usize {
        match self {
            Self::Global => 0,
            Self::Domain(_) | Self::Custom { .. } => 1,
            Self::World(_) => 2,
            Self::All(contexts) => contexts.iter().map(Self::specificity).sum(),
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
            Self::All(contexts) => {
                for (index, context) in contexts.iter().enumerate() {
                    if index != 0 {
                        write!(f, " + ")?;
                    }
                    write!(f, "{context}")?;
                }
                Ok(())
            }
        }
    }
}

fn push_unique_context(
    contexts: &mut Vec<PermissionRuleContext>,
    context: PermissionRuleContext,
) -> Result<(), PermissionRuleContextError> {
    if let PermissionRuleContext::Custom { key, value } = &context {
        for existing in contexts.iter() {
            let PermissionRuleContext::Custom {
                key: existing_key,
                value: existing_value,
            } = existing
            else {
                continue;
            };
            if existing_key != key {
                continue;
            }
            if existing_value == value {
                return Ok(());
            }
            return Err(PermissionRuleContextError::DuplicateCustomKey(key.clone()));
        }
    }
    if contexts.iter().any(|existing| existing == &context) {
        return Ok(());
    }
    contexts.push(context);
    Ok(())
}

fn compare_rule_contexts(left: &PermissionRuleContext, right: &PermissionRuleContext) -> Ordering {
    rule_context_rank(left)
        .cmp(&rule_context_rank(right))
        .then_with(|| match (left, right) {
            (PermissionRuleContext::Domain(left), PermissionRuleContext::Domain(right)) => {
                left.cmp(right)
            }
            (PermissionRuleContext::World(left), PermissionRuleContext::World(right)) => left
                .namespace
                .cmp(&right.namespace)
                .then_with(|| left.path.cmp(&right.path)),
            (
                PermissionRuleContext::Custom {
                    key: left_key,
                    value: left_value,
                },
                PermissionRuleContext::Custom {
                    key: right_key,
                    value: right_value,
                },
            ) => left_key
                .as_str()
                .cmp(right_key.as_str())
                .then_with(|| left_value.cmp(right_value)),
            _ => Ordering::Equal,
        })
}

const fn rule_context_rank(context: &PermissionRuleContext) -> u8 {
    match context {
        PermissionRuleContext::Domain(_) => 0,
        PermissionRuleContext::World(_) => 1,
        PermissionRuleContext::Custom { .. } => 2,
        PermissionRuleContext::Global => 3,
        PermissionRuleContext::All(_) => 4,
    }
}

/// Invalid permission rule context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionRuleContextError {
    /// Context value is empty.
    EmptyValue,
    /// One chained context tried to bind the same custom key to multiple values.
    DuplicateCustomKey(PermissionContextKey),
}

impl fmt::Display for PermissionRuleContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyValue => write!(f, "permission context value is empty"),
            Self::DuplicateCustomKey(key) => write!(
                f,
                "custom permission context key '{}' cannot have multiple values",
                key.as_str()
            ),
        }
    }
}

impl Error for PermissionRuleContextError {}

/// A permission key plus the rule context embedded in command/config syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionRuleExpression {
    key: PermissionKey,
    context: PermissionRuleContext,
}

impl PermissionRuleExpression {
    /// Creates a permission rule expression.
    #[must_use]
    pub const fn new(key: PermissionKey, context: PermissionRuleContext) -> Self {
        Self { key, context }
    }

    /// Parses `permission` or `permission{context=value,...}` syntax.
    ///
    /// # Errors
    ///
    /// Returns an error when the permission key or context selector is invalid.
    pub fn parse(value: impl Into<String>) -> Result<Self, PermissionRuleExpressionError> {
        let value = value.into();
        let Some(context_start) = value.find('{') else {
            let key = PermissionKey::parse(value.clone()).map_err(|source| {
                PermissionRuleExpressionError::InvalidPermissionKey { value, source }
            })?;
            return Ok(Self::new(key, PermissionRuleContext::Global));
        };

        if !value.ends_with('}') {
            return Err(PermissionRuleExpressionError::UnclosedContext);
        }

        let key_value = &value[..context_start];
        let key = PermissionKey::parse(key_value.to_owned()).map_err(|source| {
            PermissionRuleExpressionError::InvalidPermissionKey {
                value: key_value.to_owned(),
                source,
            }
        })?;

        let context_value = &value[context_start + 1..value.len() - 1];
        let context = parse_permission_rule_expression_context(context_value)?;
        Ok(Self::new(key, context))
    }

    /// Returns the permission key.
    #[must_use]
    pub const fn key(&self) -> &PermissionKey {
        &self.key
    }

    /// Returns the rule context.
    #[must_use]
    pub const fn context(&self) -> &PermissionRuleContext {
        &self.context
    }

    /// Splits this expression into its key and context.
    #[must_use]
    pub fn into_parts(self) -> (PermissionKey, PermissionRuleContext) {
        (self.key, self.context)
    }
}

impl fmt::Display for PermissionRuleExpression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.key.as_str())?;
        write_permission_expression_context(f, &self.context)
    }
}

fn parse_permission_rule_expression_context(
    value: &str,
) -> Result<PermissionRuleContext, PermissionRuleExpressionError> {
    if value.is_empty() {
        return Err(PermissionRuleExpressionError::EmptyContext);
    }

    let mut contexts = Vec::new();
    let mut seen_keys = BTreeSet::new();
    for entry in value.split(',') {
        let Some((key, context_value)) = entry.split_once('=') else {
            return Err(PermissionRuleExpressionError::InvalidContextEntry(
                entry.to_owned(),
            ));
        };
        if key.is_empty() || context_value.is_empty() {
            return Err(PermissionRuleExpressionError::InvalidContextEntry(
                entry.to_owned(),
            ));
        }
        if context_value
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '{' | '}' | ',' | '='))
        {
            return Err(PermissionRuleExpressionError::InvalidContextValue {
                key: key.to_owned(),
                value: context_value.to_owned(),
            });
        }
        if !seen_keys.insert(key.to_owned()) {
            return Err(PermissionRuleExpressionError::DuplicateContextKey(
                key.to_owned(),
            ));
        }

        match key {
            "domain" => contexts.push(parse_rule_expression_domain(context_value)?),
            "world" => contexts.push(parse_rule_expression_world(context_value)?),
            custom_key => contexts.push(parse_rule_expression_custom(custom_key, context_value)?),
        }
    }

    PermissionRuleContext::all(contexts).map_err(PermissionRuleExpressionError::InvalidRuleContext)
}

fn parse_rule_expression_domain(
    value: &str,
) -> Result<PermissionRuleContext, PermissionRuleExpressionError> {
    if value.is_empty() || !Identifier::validate_namespace(value) {
        return Err(PermissionRuleExpressionError::InvalidDomain(
            value.to_owned(),
        ));
    }
    Ok(PermissionRuleContext::domain(value.to_owned()))
}

fn parse_rule_expression_world(
    value: &str,
) -> Result<PermissionRuleContext, PermissionRuleExpressionError> {
    let Some(world) = parse_loaded_world_identifier(value) else {
        return Err(PermissionRuleExpressionError::InvalidWorld(
            value.to_owned(),
        ));
    };
    Ok(PermissionRuleContext::world(world))
}

fn parse_rule_expression_custom(
    key: &str,
    value: &str,
) -> Result<PermissionRuleContext, PermissionRuleExpressionError> {
    let key = PermissionContextKey::parse(key.to_owned()).map_err(|source| {
        PermissionRuleExpressionError::InvalidContextKey {
            key: key.to_owned(),
            source,
        }
    })?;
    PermissionRuleContext::custom(key, value.to_owned())
        .map_err(PermissionRuleExpressionError::InvalidRuleContext)
}

fn write_permission_expression_context(
    f: &mut fmt::Formatter<'_>,
    context: &PermissionRuleContext,
) -> fmt::Result {
    if context.is_global() {
        return Ok(());
    }

    write!(f, "{{")?;
    write_permission_expression_context_entries(f, context, &mut true)?;
    write!(f, "}}")
}

fn write_permission_expression_context_entries(
    f: &mut fmt::Formatter<'_>,
    context: &PermissionRuleContext,
    first: &mut bool,
) -> fmt::Result {
    match context {
        PermissionRuleContext::Global => {}
        PermissionRuleContext::Domain(domain) => {
            write_permission_expression_context_entry(f, first, "domain", domain)?;
        }
        PermissionRuleContext::World(world) => {
            write_permission_expression_context_entry(f, first, "world", &world.to_string())?;
        }
        PermissionRuleContext::Custom { key, value } => {
            write_permission_expression_context_entry(f, first, key.as_str(), value)?;
        }
        PermissionRuleContext::All(contexts) => {
            for context in contexts.iter() {
                write_permission_expression_context_entries(f, context, first)?;
            }
        }
    }
    Ok(())
}

fn write_permission_expression_context_entry(
    f: &mut fmt::Formatter<'_>,
    first: &mut bool,
    key: &str,
    value: &str,
) -> fmt::Result {
    if *first {
        *first = false;
    } else {
        write!(f, ",")?;
    }
    write!(f, "{key}={value}")
}

/// Invalid permission rule expression syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionRuleExpressionError {
    /// The permission key is invalid.
    InvalidPermissionKey {
        /// Invalid permission key text.
        value: String,
        /// Parse error.
        source: PermissionKeyError,
    },
    /// A context selector started with `{` but did not end with `}`.
    UnclosedContext,
    /// The context selector was empty.
    EmptyContext,
    /// A context entry was not `key=value`.
    InvalidContextEntry(String),
    /// A context entry used an invalid value.
    InvalidContextValue {
        /// Context key.
        key: String,
        /// Invalid context value.
        value: String,
    },
    /// The same context key appeared more than once.
    DuplicateContextKey(String),
    /// The domain context value is invalid.
    InvalidDomain(String),
    /// The world context value is invalid.
    InvalidWorld(String),
    /// A custom context key is invalid.
    InvalidContextKey {
        /// Invalid context key text.
        key: String,
        /// Parse error.
        source: PermissionContextKeyError,
    },
    /// The resulting rule context is invalid.
    InvalidRuleContext(PermissionRuleContextError),
}

impl fmt::Display for PermissionRuleExpressionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPermissionKey { value, source } => {
                write!(f, "invalid permission key '{value}': {source}")
            }
            Self::UnclosedContext => write!(f, "permission context selector is not closed"),
            Self::EmptyContext => write!(f, "permission context selector is empty"),
            Self::InvalidContextEntry(entry) => {
                write!(f, "invalid permission context entry '{entry}'")
            }
            Self::InvalidContextValue { key, value } => {
                write!(f, "invalid permission context value '{value}' for '{key}'")
            }
            Self::DuplicateContextKey(key) => {
                write!(f, "permission context key '{key}' appears more than once")
            }
            Self::InvalidDomain(domain) => write!(f, "invalid domain context '{domain}'"),
            Self::InvalidWorld(world) => write!(f, "invalid world context '{world}'"),
            Self::InvalidContextKey { key, source } => {
                write!(f, "invalid permission context key '{key}': {source}")
            }
            Self::InvalidRuleContext(source) => write!(f, "{source}"),
        }
    }
}

impl Error for PermissionRuleExpressionError {}

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

    /// Creates a context for one domain without a specific loaded world.
    #[must_use]
    pub fn for_domain(domain: impl Into<String>) -> Self {
        Self {
            domain: Some(domain.into()),
            world: None,
            custom_contexts: Vec::new(),
        }
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
    /// Returns an error when the value is empty, or when this context already
    /// has a different value for the same custom key.
    pub fn with_custom_context(
        mut self,
        key: PermissionContextKey,
        value: impl Into<String>,
    ) -> Result<Self, PermissionRuleContextError> {
        self.add_custom_context(key, value)?;
        Ok(self)
    }

    /// Adds a custom active rule context in place.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is empty, or when this context already
    /// has a different value for the same custom key.
    pub fn add_custom_context(
        &mut self,
        key: PermissionContextKey,
        value: impl Into<String>,
    ) -> Result<(), PermissionRuleContextError> {
        let context = PermissionRuleContext::custom(key, value)?;
        if let PermissionRuleContext::Custom { key, value } = &context {
            for existing in &self.custom_contexts {
                let PermissionRuleContext::Custom {
                    key: existing_key,
                    value: existing_value,
                } = existing
                else {
                    continue;
                };
                if existing_key != key {
                    continue;
                }
                if existing_value == value {
                    return Ok(());
                }
                return Err(PermissionRuleContextError::DuplicateCustomKey(key.clone()));
            }
        }
        self.custom_contexts.push(context);
        Ok(())
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

/// One custom permission context key.
///
/// Namespaced keys like `plugin:region` are preferred for plugin-owned
/// contexts. Unqualified keys remain valid for local/server-owned contexts and
/// existing configuration.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PermissionContextKey(String);

impl PermissionContextKey {
    /// Parses a custom permission context key.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is empty or contains invalid characters.
    pub fn parse(value: impl Into<String>) -> Result<Self, PermissionContextKeyError> {
        let value = value.into();
        if value.contains(':') {
            validate_namespaced_context_key(&value)?;
            return Ok(Self(value));
        }

        PermissionSegment::parse(value.clone())
            .map_err(PermissionContextKeyError::InvalidUnqualified)?;
        Ok(Self(value))
    }

    /// Returns this key as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_namespaced_context_key(value: &str) -> Result<(), PermissionContextKeyError> {
    let Some((namespace, path)) = value.split_once(':') else {
        return Err(PermissionContextKeyError::InvalidFormat);
    };
    if namespace.is_empty() {
        return Err(PermissionContextKeyError::EmptyNamespace);
    }
    if path.is_empty() {
        return Err(PermissionContextKeyError::EmptyPath);
    }
    if path.contains(':') {
        return Err(PermissionContextKeyError::InvalidFormat);
    }
    if namespace.split('.').any(str::is_empty) {
        return Err(PermissionContextKeyError::InvalidNamespace);
    }
    if path.split(['.', '/']).any(str::is_empty) {
        return Err(PermissionContextKeyError::InvalidPath);
    }
    if !Identifier::validate_namespace(namespace) {
        return Err(PermissionContextKeyError::InvalidNamespace);
    }
    if !Identifier::validate_path(path) {
        return Err(PermissionContextKeyError::InvalidPath);
    }
    Ok(())
}

/// Invalid custom permission context key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionContextKeyError {
    /// A namespaced key did not use `namespace:path` syntax.
    InvalidFormat,
    /// The namespace was empty.
    EmptyNamespace,
    /// The path was empty.
    EmptyPath,
    /// The namespace contains invalid characters.
    InvalidNamespace,
    /// The path contains invalid characters.
    InvalidPath,
    /// An unqualified key was not a valid permission segment.
    InvalidUnqualified(PermissionKeyError),
}

impl fmt::Display for PermissionContextKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat => write!(f, "context key must be a name or namespaced id"),
            Self::EmptyNamespace => write!(f, "context key namespace is empty"),
            Self::EmptyPath => write!(f, "context key path is empty"),
            Self::InvalidNamespace => {
                write!(f, "context key namespace contains invalid characters")
            }
            Self::InvalidPath => write!(f, "context key path contains invalid characters"),
            Self::InvalidUnqualified(source) => write!(f, "{source}"),
        }
    }
}

impl Error for PermissionContextKeyError {}

fn validate_permission_segment(segment: &str) -> Result<(), PermissionKeyError> {
    if segment.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
    }) {
        Ok(())
    } else {
        Err(PermissionKeyError::InvalidSegment)
    }
}

/// Parses a namespaced permission metadata key.
///
/// # Errors
///
/// Returns an error when the key is not a non-empty `namespace:path` identifier.
pub fn parse_permission_value_key(
    value: impl Into<String>,
) -> Result<Identifier, PermissionValueKeyError> {
    let value = value.into();
    let Some((namespace, path)) = value.split_once(':') else {
        return Err(PermissionValueKeyError::InvalidFormat);
    };
    if namespace.is_empty() {
        return Err(PermissionValueKeyError::EmptyNamespace);
    }
    if path.is_empty() {
        return Err(PermissionValueKeyError::EmptyPath);
    }
    if path.contains(':') {
        return Err(PermissionValueKeyError::InvalidFormat);
    }
    if namespace.split('.').any(str::is_empty) {
        return Err(PermissionValueKeyError::InvalidNamespace);
    }
    if path.split(['.', '/']).any(str::is_empty) {
        return Err(PermissionValueKeyError::InvalidPath);
    }
    if !Identifier::validate_namespace(namespace) {
        return Err(PermissionValueKeyError::InvalidNamespace);
    }
    if !Identifier::validate_path(path) {
        return Err(PermissionValueKeyError::InvalidPath);
    }

    Ok(Identifier::new(namespace.to_owned(), path.to_owned()))
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

/// Source that registered a permission metadata key for discovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PermissionMetadataCatalogSource {
    /// Metadata key already present in resolved group configuration.
    Config,
}

/// One discoverable permission metadata key and the sources that registered it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionMetadataCatalogEntry {
    key: Identifier,
    sources: BTreeSet<PermissionMetadataCatalogSource>,
}

impl PermissionMetadataCatalogEntry {
    fn new(key: Identifier, source: PermissionMetadataCatalogSource) -> Self {
        let mut sources = BTreeSet::new();
        sources.insert(source);
        Self { key, sources }
    }

    /// Returns the permission metadata key.
    #[must_use]
    pub const fn key(&self) -> &Identifier {
        &self.key
    }

    /// Returns the sources that registered this key.
    #[must_use]
    pub const fn sources(&self) -> &BTreeSet<PermissionMetadataCatalogSource> {
        &self.sources
    }
}

/// Registry of permission metadata keys available for discovery and autocomplete.
///
/// The catalog is not an enforcement boundary. Metadata checks and edits still
/// accept any syntactically valid key so plugins and config can introduce keys
/// before Steel has metadata for them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PermissionMetadataCatalog {
    entries: BTreeMap<String, PermissionMetadataCatalogEntry>,
}

impl PermissionMetadataCatalog {
    /// Creates an empty permission metadata catalog.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Registers one permission metadata key for discovery.
    pub fn insert(&mut self, key: Identifier, source: PermissionMetadataCatalogSource) {
        self.entries
            .entry(key.to_string())
            .and_modify(|entry| {
                entry.sources.insert(source);
            })
            .or_insert_with(|| PermissionMetadataCatalogEntry::new(key, source));
    }

    /// Merges another catalog into this one.
    pub fn extend(&mut self, other: &Self) {
        for entry in other.entries.values() {
            for source in &entry.sources {
                self.insert(entry.key.clone(), *source);
            }
        }
    }

    /// Returns all catalog entries sorted by metadata key.
    pub fn entries(&self) -> impl Iterator<Item = &PermissionMetadataCatalogEntry> {
        self.entries.values()
    }

    /// Returns suggestion text for keys matching `prefix`.
    #[must_use]
    pub fn suggestions(&self, prefix: &str) -> Vec<String> {
        self.entries
            .keys()
            .filter(|key| key.starts_with(prefix))
            .cloned()
            .collect()
    }
}

/// Source that registered a custom permission context for discovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PermissionContextCatalogSource {
    /// Custom context already present in resolved group configuration.
    Config,
}

/// One discoverable custom permission context key and its known values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionContextCatalogEntry {
    key: PermissionContextKey,
    sources: BTreeSet<PermissionContextCatalogSource>,
    values: BTreeMap<String, BTreeSet<PermissionContextCatalogSource>>,
}

impl PermissionContextCatalogEntry {
    fn new(key: PermissionContextKey, source: PermissionContextCatalogSource) -> Self {
        let mut sources = BTreeSet::new();
        sources.insert(source);
        Self {
            key,
            sources,
            values: BTreeMap::new(),
        }
    }

    /// Returns the custom context key.
    #[must_use]
    pub const fn key(&self) -> &PermissionContextKey {
        &self.key
    }

    /// Returns the sources that registered this key.
    #[must_use]
    pub const fn sources(&self) -> &BTreeSet<PermissionContextCatalogSource> {
        &self.sources
    }

    /// Returns known values for this key sorted by value.
    pub fn values(
        &self,
    ) -> impl Iterator<Item = (&str, &BTreeSet<PermissionContextCatalogSource>)> {
        self.values
            .iter()
            .map(|(value, sources)| (value.as_str(), sources))
    }
}

/// Registry of custom permission contexts available for discovery and autocomplete.
///
/// The catalog is not an enforcement boundary. Permission checks and edits still
/// accept any syntactically valid custom context so plugins and config can
/// introduce contexts before Steel has metadata for them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PermissionContextCatalog {
    entries: BTreeMap<String, PermissionContextCatalogEntry>,
}

impl PermissionContextCatalog {
    /// Creates an empty permission context catalog.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Registers one custom context key for discovery.
    pub fn insert_key(
        &mut self,
        key: PermissionContextKey,
        source: PermissionContextCatalogSource,
    ) {
        self.entries
            .entry(key.as_str().to_owned())
            .and_modify(|entry| {
                entry.sources.insert(source);
            })
            .or_insert_with(|| PermissionContextCatalogEntry::new(key, source));
    }

    /// Registers one custom context value for discovery.
    pub fn insert_value(
        &mut self,
        key: PermissionContextKey,
        value: impl Into<String>,
        source: PermissionContextCatalogSource,
    ) {
        let value = value.into();
        self.insert_key(key.clone(), source);
        if let Some(entry) = self.entries.get_mut(key.as_str()) {
            entry
                .values
                .entry(value)
                .or_insert_with(BTreeSet::new)
                .insert(source);
        }
    }

    /// Merges another catalog into this one.
    pub fn extend(&mut self, other: &Self) {
        for entry in other.entries.values() {
            for source in &entry.sources {
                self.insert_key(entry.key.clone(), *source);
            }
            for (value, sources) in entry.values() {
                for source in sources {
                    self.insert_value(entry.key.clone(), value, *source);
                }
            }
        }
    }

    /// Returns all catalog entries sorted by custom context key.
    pub fn entries(&self) -> impl Iterator<Item = &PermissionContextCatalogEntry> {
        self.entries.values()
    }

    /// Returns suggestion text for keys matching `prefix`.
    #[must_use]
    pub fn key_suggestions(&self, prefix: &str) -> Vec<String> {
        self.entries
            .keys()
            .filter(|key| key.starts_with(prefix))
            .cloned()
            .collect()
    }

    /// Returns suggestion text for values of `key` matching `prefix`.
    #[must_use]
    pub fn value_suggestions(&self, key: &PermissionContextKey, prefix: &str) -> Vec<String> {
        self.entries
            .get(key.as_str())
            .map(|entry| {
                entry
                    .values
                    .keys()
                    .filter(|value| value.starts_with(prefix))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Persisted permission state for one player.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PermissionSubjectState {
    groups: Vec<String>,
    overrides: PermissionSet,
    value_overrides: PermissionValueSet,
}

impl PermissionSubjectState {
    /// Creates a persisted permission-state snapshot.
    #[must_use]
    pub const fn new(groups: Vec<String>, overrides: PermissionSet) -> Self {
        Self {
            groups,
            overrides,
            value_overrides: PermissionValueSet::new(),
        }
    }

    /// Creates a persisted permission-state snapshot with value overrides.
    #[must_use]
    pub const fn new_with_values(
        groups: Vec<String>,
        overrides: PermissionSet,
        value_overrides: PermissionValueSet,
    ) -> Self {
        Self {
            groups,
            overrides,
            value_overrides,
        }
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

    /// Returns direct permission value overrides.
    #[must_use]
    pub const fn value_overrides(&self) -> &PermissionValueSet {
        &self.value_overrides
    }

    /// Splits this snapshot into owned groups and overrides.
    #[must_use]
    pub fn into_parts(self) -> (Vec<String>, PermissionSet, PermissionValueSet) {
        (self.groups, self.overrides, self.value_overrides)
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

/// Invalid permission metadata identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionValueKeyError {
    /// The value did not use `namespace:path` form.
    InvalidFormat,
    /// The namespace is empty.
    EmptyNamespace,
    /// The path is empty.
    EmptyPath,
    /// The namespace contains invalid characters.
    InvalidNamespace,
    /// The path contains invalid characters.
    InvalidPath,
}

impl fmt::Display for PermissionValueKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat => write!(f, "permission metadata key must be a namespaced id"),
            Self::EmptyNamespace => write!(f, "permission metadata key namespace is empty"),
            Self::EmptyPath => write!(f, "permission metadata key path is empty"),
            Self::InvalidNamespace => {
                write!(
                    f,
                    "permission metadata key namespace contains invalid characters"
                )
            }
            Self::InvalidPath => {
                write!(
                    f,
                    "permission metadata key path contains invalid characters"
                )
            }
        }
    }
}

impl Error for PermissionValueKeyError {}

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

/// A configured permission value.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum PermissionValue {
    /// Boolean metadata.
    Bool(bool),
    /// Integer metadata.
    Integer(i64),
    /// String metadata.
    String(String),
}

impl PermissionValue {
    /// Returns this value as a boolean when it has boolean type.
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            Self::Integer(_) | Self::String(_) => None,
        }
    }

    /// Returns this value as an integer when it has integer type.
    #[must_use]
    pub const fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            Self::Bool(_) | Self::String(_) => None,
        }
    }

    /// Returns this value as a string when it has string type.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            Self::Bool(_) | Self::Integer(_) => None,
        }
    }
}

impl fmt::Display for PermissionValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(value) => write!(f, "{value}"),
            Self::Integer(value) => write!(f, "{value}"),
            Self::String(value) => f.write_str(value),
        }
    }
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

/// One configured permission value rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionValueEntry {
    key: Identifier,
    context: PermissionRuleContext,
    value: PermissionValue,
}

impl PermissionValueEntry {
    /// Creates one global permission value entry.
    #[must_use]
    pub const fn new(key: Identifier, value: PermissionValue) -> Self {
        Self {
            key,
            context: PermissionRuleContext::Global,
            value,
        }
    }

    /// Creates one contextual permission value entry.
    #[must_use]
    pub const fn new_with_context(
        key: Identifier,
        context: PermissionRuleContext,
        value: PermissionValue,
    ) -> Self {
        Self {
            key,
            context,
            value,
        }
    }

    /// Returns the permission metadata key.
    #[must_use]
    pub const fn key(&self) -> &Identifier {
        &self.key
    }

    /// Returns this rule's context.
    #[must_use]
    pub const fn context(&self) -> &PermissionRuleContext {
        &self.context
    }

    /// Returns the configured value.
    #[must_use]
    pub const fn value(&self) -> &PermissionValue {
        &self.value
    }
}

/// A flat effective permission set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PermissionSet {
    entries: Vec<PermissionEntry>,
    sources: Vec<PermissionResolutionSource>,
}

impl PermissionSet {
    /// Creates an empty permission set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            sources: Vec::new(),
        }
    }

    /// Creates a permission set from entries.
    #[must_use]
    pub fn from_entries(entries: impl IntoIterator<Item = PermissionEntry>) -> Self {
        let entries = entries.into_iter().collect::<Vec<_>>();
        let sources = vec![PermissionResolutionSource::Subject; entries.len()];
        Self { entries, sources }
    }

    /// Returns all entries in insertion order.
    #[must_use]
    pub fn entries(&self) -> &[PermissionEntry] {
        &self.entries
    }

    /// Adds one permission entry.
    pub fn push(&mut self, entry: PermissionEntry) {
        self.push_with_source(entry, PermissionResolutionSource::Subject);
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
        self.retain_entries(|entry| entry.key != key || entry.context != context);
        self.push(PermissionEntry::new_with_context(key, context, state));
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
        self.retain_entries(|entry| entry.key() != key || entry.context() != context);
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
        self.best_key_candidate(key, context)
            .map(|candidate| candidate.state)
    }

    /// Resolves one key in the global context and returns the winning rule.
    ///
    /// Unset permissions return `None`.
    #[must_use]
    pub fn resolve_key_detailed(&self, key: &PermissionKey) -> Option<PermissionResolution> {
        self.resolve_key_in_detailed(key, &PermissionContext::global())
    }

    /// Resolves one key in a permission context and returns the winning rule.
    ///
    /// Unset permissions return `None`.
    #[must_use]
    pub fn resolve_key_in_detailed(
        &self,
        key: &PermissionKey,
        context: &PermissionContext,
    ) -> Option<PermissionResolution> {
        self.best_key_candidate(key, context)
            .map(|candidate| self.permission_resolution(candidate))
    }

    /// Resolves a child key in the global context while treating `parent` as a broad grant.
    ///
    /// Unset permissions return `None`. If both parent and child-side entries
    /// match, the more specific entry wins; tied specificity resolves by
    /// source/priority, then deny.
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
    /// match, the more specific entry wins; tied specificity resolves by
    /// source/priority, then deny.
    #[must_use]
    pub fn resolve_scoped_key_in(
        &self,
        parent: &PermissionKey,
        key: &PermissionKey,
        context: &PermissionContext,
    ) -> Option<PermissionState> {
        self.best_scoped_key_candidate(parent, key, context)
            .map(|candidate| candidate.state)
    }

    /// Resolves a child key in the global context and returns the winning rule.
    ///
    /// This treats `parent` as a broad grant. Unset permissions return `None`.
    #[must_use]
    pub fn resolve_scoped_key_detailed(
        &self,
        parent: &PermissionKey,
        key: &PermissionKey,
    ) -> Option<PermissionResolution> {
        self.resolve_scoped_key_in_detailed(parent, key, &PermissionContext::global())
    }

    /// Resolves a child key in a permission context and returns the winning rule.
    ///
    /// This treats `parent` as a broad grant. Unset permissions return `None`.
    #[must_use]
    pub fn resolve_scoped_key_in_detailed(
        &self,
        parent: &PermissionKey,
        key: &PermissionKey,
        context: &PermissionContext,
    ) -> Option<PermissionResolution> {
        self.best_scoped_key_candidate(parent, key, context)
            .map(|candidate| self.permission_resolution(candidate))
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

    fn push_group(&mut self, entry: PermissionEntry, group: &str, group_priority: i32) {
        self.push_with_source(
            entry,
            PermissionResolutionSource::Group {
                name: group.to_owned(),
                priority: group_priority,
            },
        );
    }

    fn push_with_source(&mut self, entry: PermissionEntry, source: PermissionResolutionSource) {
        self.entries.push(entry);
        self.sources.push(source);
    }

    fn retain_entries(&mut self, mut keep: impl FnMut(&PermissionEntry) -> bool) {
        let entries = std::mem::take(&mut self.entries);
        let sources = std::mem::take(&mut self.sources);
        for (entry, source) in entries.into_iter().zip(sources) {
            if keep(&entry) {
                self.entries.push(entry);
                self.sources.push(source);
            }
        }
    }

    fn best_key_candidate(
        &self,
        key: &PermissionKey,
        context: &PermissionContext,
    ) -> Option<PermissionCandidate> {
        let mut best = None;

        for (index, (entry, source)) in self.entries.iter().zip(&self.sources).enumerate() {
            if !entry.context.matches_context(context) || !entry.key.matches(key) {
                continue;
            }

            push_permission_candidate(
                &mut best,
                index,
                entry.key.specificity(),
                entry.context.specificity(),
                source,
                entry.state,
            );
        }

        best
    }

    fn best_scoped_key_candidate(
        &self,
        parent: &PermissionKey,
        key: &PermissionKey,
        context: &PermissionContext,
    ) -> Option<PermissionCandidate> {
        let mut best = None;
        let parent_scopes_key = parent.scopes(key);

        for (index, (entry, source)) in self.entries.iter().zip(&self.sources).enumerate() {
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
                index,
                specificity,
                entry.context.specificity(),
                source,
                entry.state,
            );
        }

        best
    }

    fn permission_resolution(&self, candidate: PermissionCandidate) -> PermissionResolution {
        PermissionResolution {
            entry: self.entries[candidate.entry_index].clone(),
            source: candidate.source,
            key_specificity: candidate.key_specificity,
            context_specificity: candidate.context_specificity,
        }
    }
}

/// A flat effective permission value set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PermissionValueSet {
    entries: Vec<PermissionValueEntry>,
    sources: Vec<PermissionResolutionSource>,
}

impl PermissionValueSet {
    /// Creates an empty permission value set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            sources: Vec::new(),
        }
    }

    /// Creates a permission value set from entries.
    #[must_use]
    pub fn from_entries(entries: impl IntoIterator<Item = PermissionValueEntry>) -> Self {
        let entries = entries.into_iter().collect::<Vec<_>>();
        let sources = vec![PermissionResolutionSource::Subject; entries.len()];
        Self { entries, sources }
    }

    /// Returns all entries in insertion order.
    #[must_use]
    pub fn entries(&self) -> &[PermissionValueEntry] {
        &self.entries
    }

    /// Adds one permission value entry.
    pub fn push(&mut self, entry: PermissionValueEntry) {
        self.push_with_source(entry, PermissionResolutionSource::Subject);
    }

    /// Sets one exact global permission value, replacing any previous exact value.
    pub fn set(&mut self, key: Identifier, value: PermissionValue) {
        self.set_in(key, PermissionRuleContext::Global, value);
    }

    /// Sets one exact contextual permission value, replacing any previous exact value in that context.
    pub fn set_in(
        &mut self,
        key: Identifier,
        context: PermissionRuleContext,
        value: PermissionValue,
    ) {
        self.retain_entries(|entry| entry.key != key || entry.context != context);
        self.push(PermissionValueEntry::new_with_context(key, context, value));
    }

    /// Removes one exact global permission value.
    ///
    /// Returns true when an entry was removed.
    pub fn unset(&mut self, key: &Identifier) -> bool {
        self.unset_in(key, &PermissionRuleContext::Global)
    }

    /// Removes one exact contextual permission value.
    ///
    /// Returns true when an entry was removed.
    pub fn unset_in(&mut self, key: &Identifier, context: &PermissionRuleContext) -> bool {
        let old_len = self.entries.len();
        self.retain_entries(|entry| entry.key() != key || entry.context() != context);
        self.entries.len() != old_len
    }

    /// Resolves one value in the global context.
    #[must_use]
    pub fn resolve(&self, key: &Identifier) -> Option<&PermissionValue> {
        self.resolve_in(key, &PermissionContext::global())
    }

    /// Resolves one value in a permission context.
    ///
    /// More specific contexts win. Tied contexts prefer player-level values
    /// over group values, then group priority, then final insertion order as a
    /// deterministic fallback.
    #[must_use]
    pub fn resolve_in(
        &self,
        key: &Identifier,
        context: &PermissionContext,
    ) -> Option<&PermissionValue> {
        self.best_value_candidate(key, context)
            .map(|candidate| self.entries[candidate.entry_index].value())
    }

    /// Resolves one value in the global context and returns the winning rule.
    #[must_use]
    pub fn resolve_detailed(&self, key: &Identifier) -> Option<PermissionValueResolution> {
        self.resolve_in_detailed(key, &PermissionContext::global())
    }

    /// Resolves one value in a permission context and returns the winning rule.
    #[must_use]
    pub fn resolve_in_detailed(
        &self,
        key: &Identifier,
        context: &PermissionContext,
    ) -> Option<PermissionValueResolution> {
        self.best_value_candidate(key, context)
            .map(|candidate| self.value_resolution(candidate))
    }

    fn push_group(&mut self, entry: PermissionValueEntry, group: &str, group_priority: i32) {
        self.push_with_source(
            entry,
            PermissionResolutionSource::Group {
                name: group.to_owned(),
                priority: group_priority,
            },
        );
    }

    fn push_with_source(
        &mut self,
        entry: PermissionValueEntry,
        source: PermissionResolutionSource,
    ) {
        self.entries.push(entry);
        self.sources.push(source);
    }

    fn retain_entries(&mut self, mut keep: impl FnMut(&PermissionValueEntry) -> bool) {
        let entries = std::mem::take(&mut self.entries);
        let sources = std::mem::take(&mut self.sources);
        for (entry, source) in entries.into_iter().zip(sources) {
            if keep(&entry) {
                self.entries.push(entry);
                self.sources.push(source);
            }
        }
    }

    fn best_value_candidate(
        &self,
        key: &Identifier,
        context: &PermissionContext,
    ) -> Option<PermissionValueCandidate> {
        let mut best = None;

        for (index, (entry, source)) in self.entries.iter().zip(&self.sources).enumerate() {
            if entry.key() != key || !entry.context.matches_context(context) {
                continue;
            }
            let specificity = entry.context.specificity();
            let rank = source.rank();
            let priority = source.tie_priority();
            let candidate = PermissionValueCandidate {
                entry_index: index,
                context_specificity: specificity,
                source: source.clone(),
            };
            match best {
                None => best = Some(candidate),
                Some(current)
                    if (specificity, rank, priority, index)
                        > (
                            current.context_specificity,
                            current.source.rank(),
                            current.source.tie_priority(),
                            current.entry_index,
                        ) =>
                {
                    best = Some(candidate);
                }
                _ => {}
            }
        }

        best
    }

    fn value_resolution(&self, candidate: PermissionValueCandidate) -> PermissionValueResolution {
        PermissionValueResolution {
            entry: self.entries[candidate.entry_index].clone(),
            source: candidate.source,
            context_specificity: candidate.context_specificity,
            insertion_index: candidate.entry_index,
        }
    }
}

/// Source that contributed the winning permission rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionResolutionSource {
    /// A named permission group contributed the rule.
    Group {
        /// Group name.
        name: String,
        /// Group conflict priority.
        priority: i32,
    },
    /// A direct player/subject override contributed the rule.
    Subject,
}

impl PermissionResolutionSource {
    /// Returns the contributing group name, if this source is a group.
    #[must_use]
    pub fn group_name(&self) -> Option<&str> {
        match self {
            Self::Group { name, .. } => Some(name),
            Self::Subject => None,
        }
    }

    /// Returns the contributing group priority, if this source is a group.
    #[must_use]
    pub const fn group_priority(&self) -> Option<i32> {
        match self {
            Self::Group { priority, .. } => Some(*priority),
            Self::Subject => None,
        }
    }

    const fn rank(&self) -> usize {
        match self {
            Self::Group { .. } => 0,
            Self::Subject => 1,
        }
    }

    const fn tie_priority(&self) -> i32 {
        match self {
            Self::Group { priority, .. } => *priority,
            Self::Subject => 0,
        }
    }
}

/// Detailed winning permission rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionResolution {
    entry: PermissionEntry,
    source: PermissionResolutionSource,
    key_specificity: usize,
    context_specificity: usize,
}

impl PermissionResolution {
    /// Returns the winning permission entry.
    #[must_use]
    pub const fn entry(&self) -> &PermissionEntry {
        &self.entry
    }

    /// Returns the source that contributed the winning rule.
    #[must_use]
    pub const fn source(&self) -> &PermissionResolutionSource {
        &self.source
    }

    /// Returns the winning state.
    #[must_use]
    pub const fn state(&self) -> PermissionState {
        self.entry.state()
    }

    /// Returns the winning key pattern.
    #[must_use]
    pub const fn key(&self) -> &PermissionKey {
        self.entry.key()
    }

    /// Returns the winning rule context.
    #[must_use]
    pub const fn context(&self) -> &PermissionRuleContext {
        self.entry.context()
    }

    /// Returns the key specificity used by resolution.
    #[must_use]
    pub const fn key_specificity(&self) -> usize {
        self.key_specificity
    }

    /// Returns the context specificity used by resolution.
    #[must_use]
    pub const fn context_specificity(&self) -> usize {
        self.context_specificity
    }
}

/// Detailed winning permission value rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionValueResolution {
    entry: PermissionValueEntry,
    source: PermissionResolutionSource,
    context_specificity: usize,
    insertion_index: usize,
}

impl PermissionValueResolution {
    /// Returns the winning permission value entry.
    #[must_use]
    pub const fn entry(&self) -> &PermissionValueEntry {
        &self.entry
    }

    /// Returns the source that contributed the winning value.
    #[must_use]
    pub const fn source(&self) -> &PermissionResolutionSource {
        &self.source
    }

    /// Returns the configured value.
    #[must_use]
    pub const fn value(&self) -> &PermissionValue {
        self.entry.value()
    }

    /// Returns the winning metadata key.
    #[must_use]
    pub const fn key(&self) -> &Identifier {
        self.entry.key()
    }

    /// Returns the winning value context.
    #[must_use]
    pub const fn context(&self) -> &PermissionRuleContext {
        self.entry.context()
    }

    /// Returns the context specificity used by resolution.
    #[must_use]
    pub const fn context_specificity(&self) -> usize {
        self.context_specificity
    }

    /// Returns the source insertion index used as final metadata tie-breaker.
    #[must_use]
    pub const fn insertion_index(&self) -> usize {
        self.insertion_index
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PermissionCandidate {
    entry_index: usize,
    key_specificity: usize,
    context_specificity: usize,
    source: PermissionResolutionSource,
    state: PermissionState,
}

impl PermissionCandidate {
    fn order(&self) -> (usize, usize, usize, i32) {
        (
            self.key_specificity,
            self.context_specificity,
            self.source.rank(),
            self.source.tie_priority(),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PermissionValueCandidate {
    entry_index: usize,
    context_specificity: usize,
    source: PermissionResolutionSource,
}

fn push_permission_candidate(
    best: &mut Option<PermissionCandidate>,
    entry_index: usize,
    key_specificity: usize,
    context_specificity: usize,
    source: &PermissionResolutionSource,
    state: PermissionState,
) {
    let candidate = PermissionCandidate {
        entry_index,
        key_specificity,
        context_specificity,
        source: source.clone(),
        state,
    };
    match best {
        None => *best = Some(candidate),
        Some(current) if candidate.order() > current.order() => *best = Some(candidate),
        Some(current)
            if candidate.order() == current.order()
                && current.state == PermissionState::Allow
                && state == PermissionState::Deny =>
        {
            *best = Some(candidate);
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
                priority: 0,
                allow: vec!["*".to_owned()],
                deny: Vec::new(),
                rules: Vec::new(),
                values: Vec::new(),
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
    /// Priority used to resolve conflicts between equally specific group rules.
    /// Higher priority wins.
    pub priority: i32,
    /// Permission keys explicitly allowed by this group.
    pub allow: Vec<String>,
    /// Permission keys explicitly denied by this group.
    pub deny: Vec<String>,
    /// Structured permission rules with optional contexts.
    pub rules: Vec<PermissionRuleConfig>,
    /// Structured permission values with optional contexts.
    pub values: Vec<PermissionValueRuleConfig>,
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

/// One structured `groups.toml` permission value rule.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PermissionValueRuleConfig {
    /// Permission metadata key affected by this rule.
    pub key: String,
    /// Configured value.
    pub value: PermissionValue,
    /// Context where this value applies. Omitted means global.
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
    /// Plugin or subsystem-defined custom context.
    #[serde(
        default,
        deserialize_with = "deserialize_custom_contexts",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub custom: Vec<PermissionRuleCustomContextConfig>,
}

impl PermissionRuleContextConfig {
    pub(crate) fn into_rule_context(
        self,
    ) -> Result<PermissionRuleContext, PermissionRuleContextConfigError> {
        let mut contexts = Vec::new();
        if let Some(domain) = self.domain {
            if domain.is_empty() || !Identifier::validate_namespace(&domain) {
                return Err(PermissionRuleContextConfigError::InvalidDomain(domain));
            }
            contexts.push(PermissionRuleContext::domain(domain));
        }
        if let Some(world) = self.world {
            contexts.push(PermissionRuleContext::world(parse_loaded_world_context(
                world,
            )?));
        }
        for custom in self.custom {
            contexts.push(custom.into_rule_context()?);
        }

        if contexts.is_empty() {
            return Err(PermissionRuleContextConfigError::EmptyContext);
        }

        PermissionRuleContext::all(contexts).map_err(permission_rule_context_config_error)
    }
}

/// Configured custom rule-side context.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PermissionRuleCustomContextConfig {
    /// Context key, such as `region`.
    pub key: String,
    /// Context value owned by the provider.
    pub value: String,
}

impl PermissionRuleCustomContextConfig {
    fn into_rule_context(self) -> Result<PermissionRuleContext, PermissionRuleContextConfigError> {
        let key = PermissionContextKey::parse(self.key.clone()).map_err(|source| {
            PermissionRuleContextConfigError::InvalidCustomKey {
                key: self.key,
                source,
            }
        })?;
        PermissionRuleContext::custom(key, self.value)
            .map_err(|_| PermissionRuleContextConfigError::InvalidCustomValue)
    }
}

fn permission_rule_context_config_error(
    error: PermissionRuleContextError,
) -> PermissionRuleContextConfigError {
    match error {
        PermissionRuleContextError::EmptyValue => {
            PermissionRuleContextConfigError::InvalidCustomValue
        }
        PermissionRuleContextError::DuplicateCustomKey(key) => {
            PermissionRuleContextConfigError::DuplicateCustomKey(key.as_str().to_owned())
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PermissionRuleCustomContextsConfig {
    One(PermissionRuleCustomContextConfig),
    Many(Vec<PermissionRuleCustomContextConfig>),
}

fn deserialize_custom_contexts<'de, D>(
    deserializer: D,
) -> Result<Vec<PermissionRuleCustomContextConfig>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match Option::<PermissionRuleCustomContextsConfig>::deserialize(deserializer)? {
        None => Ok(Vec::new()),
        Some(PermissionRuleCustomContextsConfig::One(context)) => Ok(vec![context]),
        Some(PermissionRuleCustomContextsConfig::Many(contexts)) => Ok(contexts),
    }
}

fn parse_loaded_world_context(
    world: String,
) -> Result<Identifier, PermissionRuleContextConfigError> {
    parse_loaded_world_identifier(&world)
        .ok_or(PermissionRuleContextConfigError::InvalidWorld(world))
}

fn parse_loaded_world_identifier(value: &str) -> Option<Identifier> {
    let (domain, name) = value.split_once(':')?;
    if domain.is_empty()
        || name.is_empty()
        || name.contains(':')
        || name.contains('/')
        || !Identifier::validate_namespace(domain)
        || !Identifier::validate_path(name)
    {
        return None;
    }

    Some(Identifier::new(domain.to_owned(), name.to_owned()))
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

/// Permission group manager update failure with a caller-provided edit rejection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionGroupUpdateError<E> {
    /// The caller rejected the edit before persistence.
    Edit(E),
    /// The updated config failed validation or persistence.
    Manager(PermissionGroupManagerError),
}

impl<E: fmt::Display> fmt::Display for PermissionGroupUpdateError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Edit(error) => write!(f, "{error}"),
            Self::Manager(error) => write!(f, "{error}"),
        }
    }
}

impl<E> Error for PermissionGroupUpdateError<E> where E: Error + 'static {}

impl<E> From<PermissionGroupManagerError> for PermissionGroupUpdateError<E> {
    fn from(value: PermissionGroupManagerError) -> Self {
        Self::Manager(value)
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

    /// Adds configured group metadata keys to a discovery catalog.
    pub fn register_metadata_catalog_entries(&self, catalog: &mut PermissionMetadataCatalog) {
        self.state
            .read()
            .groups
            .register_metadata_catalog_entries(catalog);
    }

    /// Adds configured custom permission contexts to a discovery catalog.
    pub fn register_context_catalog_entries(&self, catalog: &mut PermissionContextCatalog) {
        self.state
            .read()
            .groups
            .register_context_catalog_entries(catalog);
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

    /// Builds an effective permission value set from defaults, assigned groups, and player overrides.
    #[must_use]
    pub fn effective_values(
        &self,
        assigned_groups: &[String],
        player_values: &PermissionValueSet,
    ) -> PermissionValueSet {
        self.state
            .read()
            .groups
            .effective_values(assigned_groups, player_values)
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
        self.replace_config_locked(config).await
    }

    /// Updates the current group config under the manager update lock.
    ///
    /// # Errors
    ///
    /// Returns an error if the updated config is invalid or if the configured
    /// store rejects the save.
    pub async fn update_config(
        &self,
        update: impl FnOnce(&mut PermissionGroupsConfig) + Send,
    ) -> Result<(), PermissionGroupManagerError> {
        let _guard = self.updates.lock().await;
        let mut config = self.state.read().config.clone();
        update(&mut config);
        self.replace_config_locked(config).await
    }

    /// Updates the current group config under the manager update lock with a fallible edit.
    ///
    /// # Errors
    ///
    /// Returns an edit error from the caller, or an error if the updated config
    /// is invalid or cannot be persisted.
    pub async fn try_update_config<T, E>(
        &self,
        update: impl FnOnce(&mut PermissionGroupsConfig) -> Result<T, E> + Send,
    ) -> Result<T, PermissionGroupUpdateError<E>>
    where
        T: Send,
        E: Send,
    {
        let _guard = self.updates.lock().await;
        let current = self.state.read().config.clone();
        let mut config = current.clone();
        let result = update(&mut config).map_err(PermissionGroupUpdateError::Edit)?;
        if config == current {
            return Ok(result);
        }

        self.replace_config_locked(config)
            .await
            .map_err(PermissionGroupUpdateError::Manager)?;
        Ok(result)
    }

    async fn replace_config_locked(
        &self,
        config: PermissionGroupsConfig,
    ) -> Result<(), PermissionGroupManagerError> {
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
                priority: 0,
                permissions: PermissionSet::new(),
                values: PermissionValueSet::new(),
            },
        );
        groups.insert(
            OP_GROUP.to_owned(),
            PermissionGroup {
                priority: 0,
                permissions: op_permissions,
                values: PermissionValueSet::new(),
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
            let mut values = PermissionValueSet::new();
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
            for value in group.values {
                let key = parse_permission_value_key(value.key).map_err(|source| {
                    PermissionConfigError::InvalidValueKey {
                        group: name.clone(),
                        source,
                    }
                })?;
                let context = value
                    .context
                    .map_or(Ok(PermissionRuleContext::Global), |context| {
                        context.into_rule_context()
                    })
                    .map_err(|source| PermissionConfigError::InvalidValueContext {
                        group: name.clone(),
                        source,
                    })?;
                values.push(PermissionValueEntry::new_with_context(
                    key,
                    context,
                    value.value,
                ));
            }
            groups.insert(
                name,
                PermissionGroup {
                    priority: group.priority,
                    permissions,
                    values,
                },
            );
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

    /// Adds configured group metadata keys to a discovery catalog.
    pub fn register_metadata_catalog_entries(&self, catalog: &mut PermissionMetadataCatalog) {
        for group in self.groups.values() {
            for entry in group.values.entries() {
                catalog.insert(entry.key.clone(), PermissionMetadataCatalogSource::Config);
            }
        }
    }

    /// Adds configured custom permission contexts to a discovery catalog.
    pub fn register_context_catalog_entries(&self, catalog: &mut PermissionContextCatalog) {
        for group in self.groups.values() {
            for entry in group.permissions.entries() {
                register_context_catalog_entry(entry.context(), catalog);
            }
            for entry in group.values.entries() {
                register_context_catalog_entry(entry.context(), catalog);
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
            effective.push(entry.clone());
        }

        effective
    }

    /// Builds an effective permission value set from default groups, assigned groups,
    /// and player-level value overrides.
    ///
    /// Assigned groups that are not configured have no effect.
    #[must_use]
    pub fn effective_values(
        &self,
        assigned_groups: &[String],
        player_values: &PermissionValueSet,
    ) -> PermissionValueSet {
        let mut effective = PermissionValueSet::new();

        for group in &self.default_groups {
            self.append_group_values(group, &mut effective);
        }
        for group in assigned_groups {
            self.append_group_values(group, &mut effective);
        }
        for entry in player_values.entries() {
            effective.push(entry.clone());
        }

        effective
    }

    fn append_group_permissions(&self, group_name: &str, effective: &mut PermissionSet) {
        let Some(group) = self.groups.get(group_name) else {
            return;
        };

        for entry in group.permissions.entries() {
            effective.push_group(entry.clone(), group_name, group.priority);
        }
    }

    fn append_group_values(&self, group_name: &str, effective: &mut PermissionValueSet) {
        let Some(group) = self.groups.get(group_name) else {
            return;
        };

        for entry in group.values.entries() {
            effective.push_group(entry.clone(), group_name, group.priority);
        }
    }
}

fn register_context_catalog_entry(
    context: &PermissionRuleContext,
    catalog: &mut PermissionContextCatalog,
) {
    match context {
        PermissionRuleContext::Custom { key, value } => {
            catalog.insert_value(
                key.clone(),
                value.clone(),
                PermissionContextCatalogSource::Config,
            );
        }
        PermissionRuleContext::All(contexts) => {
            for context in contexts.iter() {
                register_context_catalog_entry(context, catalog);
            }
        }
        PermissionRuleContext::Global
        | PermissionRuleContext::Domain(_)
        | PermissionRuleContext::World(_) => {}
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
    priority: i32,
    permissions: PermissionSet,
    values: PermissionValueSet,
}

impl PermissionGroup {
    /// Returns this group's conflict priority.
    #[must_use]
    pub const fn priority(&self) -> i32 {
        self.priority
    }

    /// Returns this group's permission entries.
    #[must_use]
    pub const fn permissions(&self) -> &PermissionSet {
        &self.permissions
    }

    /// Returns this group's permission value entries.
    #[must_use]
    pub const fn values(&self) -> &PermissionValueSet {
        &self.values
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
    /// A group contains an invalid permission metadata key.
    InvalidValueKey {
        /// Group containing the bad metadata key.
        group: String,
        /// Parse error.
        source: PermissionValueKeyError,
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
    /// A structured value context is invalid.
    InvalidValueContext {
        /// Group containing the bad value.
        group: String,
        /// Parse error.
        source: PermissionRuleContextConfigError,
    },
}

/// Invalid configured permission rule context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionRuleContextConfigError {
    /// A present context table must declare at least one constraint.
    EmptyContext,
    /// Domain name is not valid.
    InvalidDomain(String),
    /// World identifier is not valid.
    InvalidWorld(String),
    /// Custom context key is not valid.
    InvalidCustomKey {
        /// Invalid custom context key.
        key: String,
        /// Parse error.
        source: PermissionContextKeyError,
    },
    /// Custom context value is empty.
    InvalidCustomValue,
    /// Custom context key appears with multiple values in the same rule context.
    DuplicateCustomKey(String),
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
            Self::InvalidValueKey { group, source } => {
                write!(
                    f,
                    "permission group '{group}' contains invalid metadata key: {source}"
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
            Self::InvalidValueContext { group, source } => {
                write!(
                    f,
                    "permission group '{group}' contains invalid value context: {source}"
                )
            }
        }
    }
}

impl Error for PermissionConfigError {}

impl fmt::Display for PermissionRuleContextConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyContext => write!(f, "rule context must contain at least one constraint"),
            Self::InvalidDomain(domain) => write!(f, "invalid domain context '{domain}'"),
            Self::InvalidWorld(world) => write!(f, "invalid world context '{world}'"),
            Self::InvalidCustomKey { key, source } => {
                write!(f, "invalid custom context key '{key}': {source}")
            }
            Self::InvalidCustomValue => write!(f, "custom context value is empty"),
            Self::DuplicateCustomKey(key) => {
                write!(f, "custom context key '{key}' cannot have multiple values")
            }
        }
    }
}

impl Error for PermissionRuleContextConfigError {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        PermissionCatalog, PermissionCatalogSource, PermissionContextCatalog,
        PermissionContextCatalogSource, PermissionContextKey, PermissionEntry, PermissionExpr,
        PermissionGroupManager, PermissionGroupManagerError, PermissionGroups,
        PermissionGroupsConfig, PermissionKey, PermissionKeyError, PermissionMetadataCatalog,
        PermissionMetadataCatalogSource, PermissionResolutionSource, PermissionRuleContext,
        PermissionRuleExpression, PermissionRuleExpressionError, PermissionSegment, PermissionSet,
        PermissionState, PermissionValue, PermissionValueEntry, PermissionValueKeyError,
        PermissionValueSet, parse_permission_value_key,
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

    fn value_key(value: &str) -> Identifier {
        parse_permission_value_key(value).expect("metadata key parses")
    }

    fn context_key(value: &str) -> PermissionContextKey {
        PermissionContextKey::parse(value).expect("context key parses")
    }

    fn world_context(domain: &str, world: &str) -> super::PermissionContext {
        super::PermissionContext::for_world(
            domain.to_owned(),
            Identifier::new(domain.to_owned(), world.to_owned()),
        )
    }

    fn rule_custom_context(key: &str, value: &str) -> PermissionRuleContext {
        PermissionRuleContext::custom(context_key(key), value).expect("custom context parses")
    }

    fn config_with_builder_group() -> PermissionGroupsConfig {
        let mut config = PermissionGroupsConfig::default();
        config.groups.insert(
            "builder".to_owned(),
            super::PermissionGroupConfig {
                priority: 0,
                allow: vec!["steel.build".to_owned()],
                deny: Vec::new(),
                rules: Vec::new(),
                values: Vec::new(),
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
    fn permission_value_keys_reject_invalid_identifiers() {
        assert_eq!(
            parse_permission_value_key("steel:homes*").err(),
            Some(PermissionValueKeyError::InvalidPath)
        );
        assert_eq!(
            parse_permission_value_key("*").err(),
            Some(PermissionValueKeyError::InvalidFormat)
        );
        assert_eq!(
            parse_permission_value_key(":homes").err(),
            Some(PermissionValueKeyError::EmptyNamespace)
        );
        assert_eq!(
            parse_permission_value_key("steel:").err(),
            Some(PermissionValueKeyError::EmptyPath)
        );
        assert_eq!(
            parse_permission_value_key("steel:homes:limit").err(),
            Some(PermissionValueKeyError::InvalidFormat)
        );
        assert_eq!(
            parse_permission_value_key("steel.:homes").err(),
            Some(PermissionValueKeyError::InvalidNamespace)
        );
        assert_eq!(
            parse_permission_value_key("steel:homes//limit").err(),
            Some(PermissionValueKeyError::InvalidPath)
        );
    }

    #[test]
    fn permission_values_resolve_by_context_then_order() {
        let limit = value_key("steel:homes");
        let values = PermissionValueSet::from_entries([
            PermissionValueEntry::new(limit.clone(), PermissionValue::Integer(5)),
            PermissionValueEntry::new(limit.clone(), PermissionValue::Integer(10)),
            PermissionValueEntry::new_with_context(
                limit.clone(),
                PermissionRuleContext::domain("lobby"),
                PermissionValue::Integer(3),
            ),
            PermissionValueEntry::new_with_context(
                limit.clone(),
                PermissionRuleContext::world(Identifier::new("lobby", "spawn")),
                PermissionValue::Integer(2),
            ),
        ]);

        assert_eq!(
            values.resolve(&limit).and_then(PermissionValue::as_i64),
            Some(10)
        );
        assert_eq!(
            values
                .resolve_in(&limit, &world_context("lobby", "creative"))
                .and_then(PermissionValue::as_i64),
            Some(3)
        );
        assert_eq!(
            values
                .resolve_in(&limit, &world_context("lobby", "spawn"))
                .and_then(PermissionValue::as_i64),
            Some(2)
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
    fn permission_rule_expression_parses_plain_permission_keys() {
        let expression =
            PermissionRuleExpression::parse("minecraft.command.gamemode").expect("key parses");

        assert_eq!(expression.key(), &key("minecraft.command.gamemode"));
        assert_eq!(expression.context(), &PermissionRuleContext::Global);
        assert_eq!(expression.to_string(), "minecraft.command.gamemode");
    }

    #[test]
    fn permission_rule_expression_parses_chained_contexts() {
        let expression = PermissionRuleExpression::parse(
            "minecraft.command.gamemode{plugin:region=spawn,world=lobby:spawn,domain=lobby}",
        )
        .expect("expression parses");
        let expected_context = PermissionRuleContext::all([
            PermissionRuleContext::domain("lobby"),
            PermissionRuleContext::world(Identifier::new("lobby", "spawn")),
            rule_custom_context("plugin:region", "spawn"),
        ])
        .expect("context chain is valid");

        assert_eq!(expression.key(), &key("minecraft.command.gamemode"));
        assert_eq!(expression.context(), &expected_context);
        assert_eq!(
            expression.to_string(),
            "minecraft.command.gamemode{domain=lobby,world=lobby:spawn,plugin:region=spawn}"
        );
    }

    #[test]
    fn permission_rule_expression_rejects_duplicate_context_keys() {
        let error =
            PermissionRuleExpression::parse("steel.fly{plugin:region=spawn,plugin:region=market}")
                .expect_err("duplicate context key is rejected");

        assert!(matches!(
            error,
            PermissionRuleExpressionError::DuplicateContextKey(key) if key == "plugin:region"
        ));
    }

    #[test]
    fn permission_rule_expression_rejects_invalid_context_values() {
        let error = PermissionRuleExpression::parse("steel.fly{plugin:region=spawn=bad}")
            .expect_err("separator in context value is rejected");

        assert!(matches!(
            error,
            PermissionRuleExpressionError::InvalidContextValue { key, value }
                if key == "plugin:region" && value == "spawn=bad"
        ));
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
    fn chained_context_specificity_beats_single_context() {
        let fly = key("steel.fly");
        let world = PermissionRuleContext::world(Identifier::new("lobby", "spawn"));
        let region = rule_custom_context("region", "spawn");
        let permissions = PermissionSet::from_entries([
            PermissionEntry::allow_with_context(fly.clone(), world.clone()),
            PermissionEntry::deny_with_context(
                fly.clone(),
                PermissionRuleContext::all([world, region]).expect("context chain is valid"),
            ),
        ]);
        let matching_context = world_context("lobby", "spawn")
            .with_custom_context(context_key("region"), "spawn")
            .expect("custom context is valid");

        assert!(!permissions.allows_key_in(&fly, &matching_context));
        assert!(permissions.allows_key_in(&fly, &world_context("lobby", "spawn")));
    }

    #[test]
    fn chained_rule_contexts_are_canonicalized() {
        let domain = PermissionRuleContext::domain("lobby");
        let world = PermissionRuleContext::world(Identifier::new("lobby", "spawn"));
        let owner = rule_custom_context("owner", "builders");
        let region = rule_custom_context("region", "spawn");

        let first = PermissionRuleContext::all([
            region.clone(),
            world.clone(),
            domain.clone(),
            owner.clone(),
        ])
        .expect("context chain is valid");
        let second = PermissionRuleContext::all([owner, domain, region, world])
            .expect("context chain is valid");

        assert_eq!(first, second);
        assert_eq!(
            first.to_string(),
            "domain lobby + world lobby:spawn + owner builders + region spawn"
        );
    }

    #[test]
    fn chained_rule_contexts_reject_conflicting_custom_keys() {
        let error = PermissionRuleContext::all([
            rule_custom_context("region", "spawn"),
            rule_custom_context("region", "market"),
        ])
        .expect_err("custom context key cannot have multiple values");

        assert!(matches!(
            error,
            super::PermissionRuleContextError::DuplicateCustomKey(key) if key.as_str() == "region"
        ));
    }

    #[test]
    fn active_permission_contexts_reject_conflicting_custom_keys() {
        let context = super::PermissionContext::global()
            .with_custom_context(context_key("region"), "spawn")
            .expect("custom context is valid")
            .with_custom_context(context_key("region"), "spawn")
            .expect("same custom context is idempotent");

        let error = context
            .with_custom_context(context_key("region"), "market")
            .expect_err("custom context key cannot have multiple values");

        assert!(matches!(
            error,
            super::PermissionRuleContextError::DuplicateCustomKey(key) if key.as_str() == "region"
        ));
    }

    #[test]
    fn configured_chained_rule_contexts_are_canonicalized() {
        let config = super::PermissionRuleContextConfig {
            domain: Some("lobby".to_owned()),
            world: Some("lobby:spawn".to_owned()),
            custom: vec![
                super::PermissionRuleCustomContextConfig {
                    key: "region".to_owned(),
                    value: "spawn".to_owned(),
                },
                super::PermissionRuleCustomContextConfig {
                    key: "owner".to_owned(),
                    value: "builders".to_owned(),
                },
            ],
        };
        let expected = PermissionRuleContext::all([
            rule_custom_context("owner", "builders"),
            PermissionRuleContext::world(Identifier::new("lobby", "spawn")),
            rule_custom_context("region", "spawn"),
            PermissionRuleContext::domain("lobby"),
        ])
        .expect("context chain is valid");

        assert_eq!(
            config
                .into_rule_context()
                .expect("configured context is valid"),
            expected
        );
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
        let resolution = permissions
            .resolve_scoped_key_detailed(&parent, &creative)
            .expect("scoped permission resolves");
        assert_eq!(resolution.state(), PermissionState::Deny);
        assert_eq!(resolution.key(), &creative);
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
    fn permission_metadata_catalog_deduplicates_and_suggests_sorted_keys() {
        let mut catalog = PermissionMetadataCatalog::new();
        catalog.insert(
            value_key("plugin:homes"),
            PermissionMetadataCatalogSource::Config,
        );
        catalog.insert(
            value_key("plugin:homes"),
            PermissionMetadataCatalogSource::Config,
        );
        catalog.insert(
            value_key("other:homes"),
            PermissionMetadataCatalogSource::Config,
        );

        assert_eq!(catalog.suggestions("plugin:"), vec!["plugin:homes"]);
        let entry = catalog
            .entries()
            .find(|entry| entry.key().to_string() == "plugin:homes")
            .expect("catalog entry exists");
        assert!(
            entry
                .sources()
                .contains(&PermissionMetadataCatalogSource::Config)
        );
    }

    #[test]
    fn permission_context_catalog_suggests_keys_and_values() {
        let mut catalog = PermissionContextCatalog::new();
        catalog.insert_value(
            context_key("region"),
            "spawn",
            PermissionContextCatalogSource::Config,
        );
        catalog.insert_value(
            context_key("region"),
            "market",
            PermissionContextCatalogSource::Config,
        );
        catalog.insert_value(
            context_key("arena"),
            "duel",
            PermissionContextCatalogSource::Config,
        );

        assert_eq!(catalog.key_suggestions("r"), vec!["region"]);
        assert_eq!(
            catalog.value_suggestions(&context_key("region"), ""),
            vec!["market", "spawn"]
        );
        let entry = catalog
            .entries()
            .find(|entry| entry.key().as_str() == "region")
            .expect("catalog entry exists");
        assert!(
            entry
                .sources()
                .contains(&PermissionContextCatalogSource::Config)
        );
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

    #[test]
    fn groups_register_config_metadata_in_catalog() {
        let mut config = PermissionGroupsConfig::default();
        let default_group = config
            .groups
            .get_mut("default")
            .expect("default group exists");
        default_group.values.push(super::PermissionValueRuleConfig {
            key: "plugin:homes".to_owned(),
            value: PermissionValue::Integer(10),
            context: None,
        });
        let groups = PermissionGroups::from_config(config).expect("groups config is valid");
        let mut catalog = PermissionMetadataCatalog::new();

        groups.register_metadata_catalog_entries(&mut catalog);

        assert_eq!(catalog.suggestions("plugin:"), vec!["plugin:homes"]);
        assert!(catalog.entries().all(|entry| {
            entry
                .sources()
                .contains(&PermissionMetadataCatalogSource::Config)
        }));
    }

    #[test]
    fn groups_register_config_contexts_in_catalog() {
        let mut config = PermissionGroupsConfig::default();
        let default_group = config
            .groups
            .get_mut("default")
            .expect("default group exists");
        default_group.rules.push(super::PermissionRuleConfig {
            key: "plugin.region.build".to_owned(),
            state: super::PermissionRuleStateConfig::Allow,
            context: Some(super::PermissionRuleContextConfig {
                domain: None,
                world: None,
                custom: vec![super::PermissionRuleCustomContextConfig {
                    key: "region".to_owned(),
                    value: "spawn".to_owned(),
                }],
            }),
        });
        default_group.values.push(super::PermissionValueRuleConfig {
            key: "plugin:homes".to_owned(),
            value: PermissionValue::Integer(5),
            context: Some(super::PermissionRuleContextConfig {
                domain: None,
                world: None,
                custom: vec![super::PermissionRuleCustomContextConfig {
                    key: "region".to_owned(),
                    value: "market".to_owned(),
                }],
            }),
        });
        let groups = PermissionGroups::from_config(config).expect("groups config is valid");
        let mut catalog = PermissionContextCatalog::new();

        groups.register_context_catalog_entries(&mut catalog);

        assert_eq!(catalog.key_suggestions("r"), vec!["region"]);
        assert_eq!(
            catalog.value_suggestions(&context_key("region"), ""),
            vec!["market", "spawn"]
        );
        assert!(catalog.entries().all(|entry| {
            entry
                .sources()
                .contains(&PermissionContextCatalogSource::Config)
        }));
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
    async fn permission_group_manager_updates_latest_config_under_lock() {
        let saved = Arc::new(SyncMutex::new(Vec::new()));
        let manager = PermissionGroupManager::new(
            PermissionGroupsConfig::default(),
            Some(Arc::new(CapturingGroupStore {
                saved: Arc::clone(&saved),
            })),
        )
        .expect("default groups config resolves");

        manager
            .update_config(|config| {
                config.groups.insert(
                    "builder".to_owned(),
                    super::PermissionGroupConfig {
                        priority: 0,
                        allow: vec!["steel.build".to_owned()],
                        deny: Vec::new(),
                        rules: Vec::new(),
                        values: Vec::new(),
                    },
                );
            })
            .await
            .expect("config update stores and swaps");

        assert!(manager.contains_group("builder"));
        assert!(saved.lock()[0].groups.contains_key("builder"));
    }

    #[tokio::test]
    async fn permission_group_manager_try_update_returns_edit_result_after_store() {
        let saved = Arc::new(SyncMutex::new(Vec::new()));
        let manager = PermissionGroupManager::new(
            PermissionGroupsConfig::default(),
            Some(Arc::new(CapturingGroupStore {
                saved: Arc::clone(&saved),
            })),
        )
        .expect("default groups config resolves");

        let result = manager
            .try_update_config(|config| {
                config.groups.insert(
                    "builder".to_owned(),
                    super::PermissionGroupConfig {
                        priority: 0,
                        allow: vec!["steel.build".to_owned()],
                        deny: Vec::new(),
                        rules: Vec::new(),
                        values: Vec::new(),
                    },
                );
                Ok::<_, &'static str>("builder")
            })
            .await
            .expect("config update stores and swaps");

        assert_eq!(result, "builder");
        assert!(manager.contains_group("builder"));
        assert!(saved.lock()[0].groups.contains_key("builder"));
    }

    #[tokio::test]
    async fn permission_group_manager_try_update_skips_store_when_unchanged() {
        let saved = Arc::new(SyncMutex::new(Vec::new()));
        let manager = PermissionGroupManager::new(
            PermissionGroupsConfig::default(),
            Some(Arc::new(CapturingGroupStore {
                saved: Arc::clone(&saved),
            })),
        )
        .expect("default groups config resolves");

        let result = manager
            .try_update_config(|_config| Ok::<_, &'static str>(false))
            .await
            .expect("unchanged update returns result");

        assert!(!result);
        assert!(saved.lock().is_empty());
    }

    #[tokio::test]
    async fn permission_group_manager_try_update_keeps_state_on_edit_error() {
        let saved = Arc::new(SyncMutex::new(Vec::new()));
        let manager = PermissionGroupManager::new(
            PermissionGroupsConfig::default(),
            Some(Arc::new(CapturingGroupStore {
                saved: Arc::clone(&saved),
            })),
        )
        .expect("default groups config resolves");

        let error = manager
            .try_update_config(|config| {
                config.groups.insert(
                    "builder".to_owned(),
                    super::PermissionGroupConfig::default(),
                );
                Err::<(), _>("rejected")
            })
            .await
            .expect_err("edit error rejects update");

        assert_eq!(error, super::PermissionGroupUpdateError::Edit("rejected"));
        assert!(!manager.contains_group("builder"));
        assert!(saved.lock().is_empty());
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
                custom: Vec::new(),
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
    fn groups_support_contextual_values() {
        let mut config = PermissionGroupsConfig::default();
        let default_group = config
            .groups
            .get_mut("default")
            .expect("default group exists");
        default_group.values.push(super::PermissionValueRuleConfig {
            key: "steel:homes".to_owned(),
            value: PermissionValue::Integer(5),
            context: None,
        });
        default_group.values.push(super::PermissionValueRuleConfig {
            key: "steel:homes".to_owned(),
            value: PermissionValue::Integer(3),
            context: Some(super::PermissionRuleContextConfig {
                domain: Some("lobby".to_owned()),
                world: None,
                custom: Vec::new(),
            }),
        });
        let groups = PermissionGroups::from_config(config).expect("groups config is valid");
        let player_values = PermissionValueSet::from_entries([PermissionValueEntry::new(
            value_key("steel:homes"),
            PermissionValue::Integer(10),
        )]);
        let effective = groups.effective_values(&[], &player_values);
        let limit = value_key("steel:homes");

        assert_eq!(
            effective.resolve(&limit).and_then(PermissionValue::as_i64),
            Some(10)
        );
        assert_eq!(
            effective
                .resolve_in(&limit, &world_context("lobby", "spawn"))
                .and_then(PermissionValue::as_i64),
            Some(3)
        );

        let player_values = PermissionValueSet::from_entries([
            PermissionValueEntry::new(limit.clone(), PermissionValue::Integer(10)),
            PermissionValueEntry::new_with_context(
                limit.clone(),
                PermissionRuleContext::domain("lobby"),
                PermissionValue::Integer(20),
            ),
        ]);
        let effective = groups.effective_values(&[], &player_values);
        assert_eq!(
            effective
                .resolve_in(&limit, &world_context("lobby", "spawn"))
                .and_then(PermissionValue::as_i64),
            Some(20)
        );
    }

    #[test]
    fn group_priority_breaks_equally_specific_permission_ties() {
        let mut config = PermissionGroupsConfig::default();
        config.groups.insert(
            "low".to_owned(),
            super::PermissionGroupConfig {
                priority: 0,
                allow: Vec::new(),
                deny: vec!["steel.fly".to_owned()],
                rules: Vec::new(),
                values: Vec::new(),
            },
        );
        config.groups.insert(
            "high".to_owned(),
            super::PermissionGroupConfig {
                priority: 50,
                allow: vec!["steel.fly".to_owned()],
                deny: Vec::new(),
                rules: Vec::new(),
                values: Vec::new(),
            },
        );
        let groups = PermissionGroups::from_config(config).expect("groups config is valid");
        let effective = groups.effective_permissions(
            &["high".to_owned(), "low".to_owned()],
            &PermissionSet::new(),
        );

        assert!(effective.allows_key(&key("steel.fly")));
        let resolution = effective
            .resolve_key_detailed(&key("steel.fly"))
            .expect("permission resolves");
        assert_eq!(resolution.state(), PermissionState::Allow);
        assert_eq!(resolution.source().group_name(), Some("high"));
        assert_eq!(resolution.source().group_priority(), Some(50));
    }

    #[test]
    fn deny_wins_equally_specific_same_priority_group_ties() {
        let mut config = PermissionGroupsConfig::default();
        config.groups.insert(
            "allow".to_owned(),
            super::PermissionGroupConfig {
                priority: 10,
                allow: vec!["steel.fly".to_owned()],
                deny: Vec::new(),
                rules: Vec::new(),
                values: Vec::new(),
            },
        );
        config.groups.insert(
            "deny".to_owned(),
            super::PermissionGroupConfig {
                priority: 10,
                allow: Vec::new(),
                deny: vec!["steel.fly".to_owned()],
                rules: Vec::new(),
                values: Vec::new(),
            },
        );
        let groups = PermissionGroups::from_config(config).expect("groups config is valid");
        let effective = groups.effective_permissions(
            &["deny".to_owned(), "allow".to_owned()],
            &PermissionSet::new(),
        );

        assert!(!effective.allows_key(&key("steel.fly")));
    }

    #[test]
    fn permission_specificity_beats_group_priority() {
        let mut config = PermissionGroupsConfig::default();
        config.groups.insert(
            "broad".to_owned(),
            super::PermissionGroupConfig {
                priority: 100,
                allow: vec!["steel.*".to_owned()],
                deny: Vec::new(),
                rules: Vec::new(),
                values: Vec::new(),
            },
        );
        config.groups.insert(
            "specific".to_owned(),
            super::PermissionGroupConfig {
                priority: 0,
                allow: Vec::new(),
                deny: vec!["steel.fly".to_owned()],
                rules: Vec::new(),
                values: Vec::new(),
            },
        );
        let groups = PermissionGroups::from_config(config).expect("groups config is valid");
        let effective = groups.effective_permissions(
            &["specific".to_owned(), "broad".to_owned()],
            &PermissionSet::new(),
        );

        assert!(!effective.allows_key(&key("steel.fly")));
    }

    #[test]
    fn context_specificity_beats_player_permission_source() {
        let mut config = PermissionGroupsConfig::default();
        let default_group = config
            .groups
            .get_mut("default")
            .expect("default group exists");
        default_group.priority = 100;
        default_group.rules.push(super::PermissionRuleConfig {
            key: "steel.fly".to_owned(),
            state: super::PermissionRuleStateConfig::Deny,
            context: Some(super::PermissionRuleContextConfig {
                domain: Some("lobby".to_owned()),
                world: None,
                custom: Vec::new(),
            }),
        });
        let groups = PermissionGroups::from_config(config).expect("groups config is valid");
        let player_permissions =
            PermissionSet::from_entries([PermissionEntry::allow(key("steel.fly"))]);
        let effective = groups.effective_permissions(&[], &player_permissions);

        assert!(!effective.allows_key_in(&key("steel.fly"), &world_context("lobby", "spawn")));
        let resolution = effective
            .resolve_key_in_detailed(&key("steel.fly"), &world_context("lobby", "spawn"))
            .expect("permission resolves");
        assert_eq!(resolution.state(), PermissionState::Deny);
        assert_eq!(resolution.source().group_name(), Some("default"));
        assert_eq!(
            resolution.context(),
            &PermissionRuleContext::domain("lobby")
        );
    }

    #[test]
    fn group_priority_breaks_equally_specific_metadata_ties() {
        let mut config = PermissionGroupsConfig::default();
        config.groups.insert(
            "low".to_owned(),
            super::PermissionGroupConfig {
                priority: 0,
                allow: Vec::new(),
                deny: Vec::new(),
                rules: Vec::new(),
                values: vec![super::PermissionValueRuleConfig {
                    key: "steel:homes".to_owned(),
                    value: PermissionValue::Integer(5),
                    context: None,
                }],
            },
        );
        config.groups.insert(
            "high".to_owned(),
            super::PermissionGroupConfig {
                priority: 50,
                allow: Vec::new(),
                deny: Vec::new(),
                rules: Vec::new(),
                values: vec![super::PermissionValueRuleConfig {
                    key: "steel:homes".to_owned(),
                    value: PermissionValue::Integer(10),
                    context: None,
                }],
            },
        );
        let groups = PermissionGroups::from_config(config).expect("groups config is valid");
        let effective = groups.effective_values(
            &["high".to_owned(), "low".to_owned()],
            &PermissionValueSet::new(),
        );
        let limit = value_key("steel:homes");

        assert_eq!(
            effective.resolve(&limit).and_then(PermissionValue::as_i64),
            Some(10)
        );
        let resolution = effective
            .resolve_detailed(&limit)
            .expect("metadata value resolves");
        assert_eq!(resolution.value().as_i64(), Some(10));
        assert_eq!(resolution.source().group_name(), Some("high"));
        assert_eq!(resolution.source().group_priority(), Some(50));
    }

    #[test]
    fn group_values_reject_wildcard_keys() {
        let mut config = PermissionGroupsConfig::default();
        let default_group = config
            .groups
            .get_mut("default")
            .expect("default group exists");
        default_group.values.push(super::PermissionValueRuleConfig {
            key: "steel:homes*".to_owned(),
            value: PermissionValue::Integer(5),
            context: None,
        });

        assert!(matches!(
            PermissionGroups::from_config(config),
            Err(super::PermissionConfigError::InvalidValueKey {
                group,
                source: PermissionValueKeyError::InvalidPath,
            }) if group == "default"
        ));
    }

    #[test]
    fn group_rules_support_chained_contexts() {
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
                custom: vec![super::PermissionRuleCustomContextConfig {
                    key: "region".to_owned(),
                    value: "spawn".to_owned(),
                }],
            }),
        });
        let groups = PermissionGroups::from_config(config).expect("groups config is valid");
        let effective = groups.effective_permissions(&[], &PermissionSet::new());
        let matching_context = world_context("lobby", "spawn")
            .with_custom_context(context_key("region"), "spawn")
            .expect("custom context is valid");
        let wrong_region = world_context("lobby", "spawn")
            .with_custom_context(context_key("region"), "market")
            .expect("custom context is valid");

        assert!(effective.allows_key_in(&key("steel.fly"), &matching_context));
        assert!(!effective.allows_key_in(&key("steel.fly"), &wrong_region));
        assert!(!effective.allows_key_in(&key("steel.fly"), &world_context("lobby", "creative")));
    }

    #[test]
    fn group_values_support_chained_contexts() {
        let mut config = PermissionGroupsConfig::default();
        let default_group = config
            .groups
            .get_mut("default")
            .expect("default group exists");
        default_group.values.push(super::PermissionValueRuleConfig {
            key: "steel:homes".to_owned(),
            value: PermissionValue::Integer(5),
            context: Some(super::PermissionRuleContextConfig {
                domain: Some("lobby".to_owned()),
                world: Some("lobby:spawn".to_owned()),
                custom: vec![super::PermissionRuleCustomContextConfig {
                    key: "region".to_owned(),
                    value: "spawn".to_owned(),
                }],
            }),
        });
        let groups = PermissionGroups::from_config(config).expect("groups config is valid");
        let effective = groups.effective_values(&[], &PermissionValueSet::new());
        let homes = value_key("steel:homes");
        let matching_context = world_context("lobby", "spawn")
            .with_custom_context(context_key("region"), "spawn")
            .expect("custom context is valid");

        assert_eq!(
            effective
                .resolve_in(&homes, &matching_context)
                .and_then(PermissionValue::as_i64),
            Some(5)
        );
        assert_eq!(
            effective
                .resolve_in(&homes, &world_context("lobby", "spawn"))
                .and_then(PermissionValue::as_i64),
            None
        );
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
    fn group_rules_reject_conflicting_custom_context_keys() {
        let mut config = PermissionGroupsConfig::default();
        let default_group = config
            .groups
            .get_mut("default")
            .expect("default group exists");
        default_group.rules.push(super::PermissionRuleConfig {
            key: "steel.region.build".to_owned(),
            state: super::PermissionRuleStateConfig::Allow,
            context: Some(super::PermissionRuleContextConfig {
                domain: None,
                world: None,
                custom: vec![
                    super::PermissionRuleCustomContextConfig {
                        key: "region".to_owned(),
                        value: "spawn".to_owned(),
                    },
                    super::PermissionRuleCustomContextConfig {
                        key: "region".to_owned(),
                        value: "market".to_owned(),
                    },
                ],
            }),
        });

        assert!(matches!(
            PermissionGroups::from_config(config),
            Err(super::PermissionConfigError::InvalidRuleContext {
                group,
                source: super::PermissionRuleContextConfigError::DuplicateCustomKey(key),
            }) if group == "default" && key == "region"
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
                custom: Vec::new(),
            }),
        });
        let groups = PermissionGroups::from_config(config).expect("groups config is valid");
        let effective = groups.effective_permissions(&[], &PermissionSet::new());

        assert!(effective.allows_key_in(&key("steel.fly"), &world_context("lobby", "spawn")));
        assert!(!effective.allows_key_in(&key("steel.fly"), &world_context("lobby", "creative")));
    }

    #[test]
    fn group_rules_support_custom_contexts() {
        let mut config = PermissionGroupsConfig::default();
        let default_group = config
            .groups
            .get_mut("default")
            .expect("default group exists");
        default_group.rules.push(super::PermissionRuleConfig {
            key: "steel.region.build".to_owned(),
            state: super::PermissionRuleStateConfig::Allow,
            context: Some(super::PermissionRuleContextConfig {
                domain: None,
                world: None,
                custom: vec![super::PermissionRuleCustomContextConfig {
                    key: "region".to_owned(),
                    value: "spawn".to_owned(),
                }],
            }),
        });
        let groups = PermissionGroups::from_config(config).expect("groups config is valid");
        let effective = groups.effective_permissions(&[], &PermissionSet::new());
        let context = super::PermissionContext::global()
            .with_custom_context(context_key("region"), "spawn")
            .expect("custom context is valid");

        assert!(effective.allows_key_in(&key("steel.region.build"), &context));
        assert!(!effective.allows_key(&key("steel.region.build")));
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
                custom: Vec::new(),
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
    fn player_permissions_break_same_specificity_group_ties() {
        let mut config = PermissionGroupsConfig::default();
        let default_group = config
            .groups
            .get_mut("default")
            .expect("default group exists");
        default_group.priority = 100;
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
        let resolution = effective
            .resolve_key_detailed(&key("steel.fly"))
            .expect("permission resolves");
        assert_eq!(resolution.state(), PermissionState::Deny);
        assert_eq!(resolution.source(), &PermissionResolutionSource::Subject);
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
