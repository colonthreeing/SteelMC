//! Command node requirements and permission expressions.

use std::{
    ops::{BitAnd, BitOr},
    sync::Arc,
};

use glam::DVec3;

use crate::{
    command::{
        context::{CommandContext, EntityAnchor},
        sender::CommandSender,
    },
    player::Player,
    server::Server,
    world::World,
};

/// The kind of source attempting to use a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandSourceKind {
    /// A connected player.
    Player,
    /// The server console.
    Console,
    /// A remote console connection.
    Rcon,
}

/// Context used when evaluating command requirements.
pub trait RequirementContext {
    /// Returns the source kind.
    fn source_kind(&self) -> CommandSourceKind;

    /// Returns whether this source satisfies `permission`.
    fn has_permission(&self, permission: &PermissionExpr) -> bool;
}

/// Runtime context available to command argument parsers and suggestion providers.
pub trait CommandInputContext: RequirementContext {
    /// Returns the server when this input is being parsed against a live command context.
    fn server(&self) -> Option<&Arc<Server>> {
        None
    }

    /// Returns the active world when available.
    fn world(&self) -> Option<&Arc<World>> {
        None
    }

    /// Returns the player source when available.
    fn player(&self) -> Option<&Arc<Player>> {
        None
    }

    /// Returns the command source position when available.
    fn position(&self) -> Option<DVec3> {
        None
    }

    /// Returns the command source rotation when available.
    fn rotation(&self) -> Option<(f32, f32)> {
        None
    }

    /// Returns the entity anchor for local coordinates.
    fn anchor(&self) -> EntityAnchor {
        EntityAnchor::Feet
    }
}

/// One permission key.
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

    /// Returns true when this key is a trailing wildcard pattern matching `other`.
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

/// A non-permission or permission-backed command requirement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// The node is always usable.
    Always,
    /// The source must be a player.
    Player,
    /// The source must be the console.
    Console,
    /// The source must pass the permission expression.
    Permission(PermissionExpr),
    /// All child requirements must pass.
    All(Vec<Requirement>),
    /// At least one child requirement must pass.
    Any(Vec<Requirement>),
}

impl Requirement {
    /// Returns true when the requirement allows `context`.
    #[must_use]
    pub fn allows(&self, context: &dyn RequirementContext) -> bool {
        match self {
            Self::Always => true,
            Self::Player => context.source_kind() == CommandSourceKind::Player,
            Self::Console => context.source_kind() == CommandSourceKind::Console,
            Self::Permission(permission) => context.has_permission(permission),
            Self::All(requirements) => requirements
                .iter()
                .all(|requirement| requirement.allows(context)),
            Self::Any(requirements) => requirements
                .iter()
                .any(|requirement| requirement.allows(context)),
        }
    }

    /// Combines two requirements with logical AND.
    #[must_use]
    pub fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Always, other) | (other, Self::Always) => other,
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

impl Default for Requirement {
    fn default() -> Self {
        Self::Always
    }
}

impl RequirementContext for CommandContext {
    fn source_kind(&self) -> CommandSourceKind {
        match self.sender {
            CommandSender::Player(_) => CommandSourceKind::Player,
            CommandSender::Console => CommandSourceKind::Console,
            CommandSender::Rcon => CommandSourceKind::Rcon,
        }
    }

    fn has_permission(&self, _permission: &PermissionExpr) -> bool {
        false
    }
}

impl CommandInputContext for CommandContext {
    fn server(&self) -> Option<&Arc<Server>> {
        Some(&self.server)
    }

    fn world(&self) -> Option<&Arc<World>> {
        Some(&self.world)
    }

    fn player(&self) -> Option<&Arc<Player>> {
        self.player.as_ref()
    }

    fn position(&self) -> Option<DVec3> {
        Some(self.position)
    }

    fn rotation(&self) -> Option<(f32, f32)> {
        self.rotation
    }

    fn anchor(&self) -> EntityAnchor {
        self.anchor
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CommandInputContext, CommandSourceKind, PermissionExpr, PermissionKey, PermissionKeyError,
        Requirement, RequirementContext,
    };

    struct StaticContext {
        source_kind: CommandSourceKind,
        allowed: Vec<PermissionKey>,
    }

    impl RequirementContext for StaticContext {
        fn source_kind(&self) -> CommandSourceKind {
            self.source_kind
        }

        fn has_permission(&self, permission: &PermissionExpr) -> bool {
            match permission {
                PermissionExpr::Key(key) => self.allowed.iter().any(|allowed| allowed.matches(key)),
                PermissionExpr::All(children) => {
                    children.iter().all(|child| self.has_permission(child))
                }
                PermissionExpr::Any(children) => {
                    children.iter().any(|child| self.has_permission(child))
                }
            }
        }
    }

    impl CommandInputContext for StaticContext {}

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
        let wildcard = PermissionKey::parse("minecraft.command.*").expect("key parses");
        let give = PermissionKey::parse("minecraft.command.give").expect("key parses");
        let sibling = PermissionKey::parse("minecraft.other.give").expect("key parses");

        assert!(wildcard.matches(&give));
        assert!(!wildcard.matches(&sibling));
    }

    #[test]
    fn star_wildcard_matches_every_permission() {
        let wildcard = PermissionKey::parse("*").expect("key parses");
        let give = PermissionKey::parse("minecraft.command.give").expect("key parses");
        let plugin = PermissionKey::parse("some.plugin.dangerous").expect("key parses");

        assert!(wildcard.matches(&give));
        assert!(wildcard.matches(&plugin));
    }

    #[test]
    fn requirements_can_combine_source_and_permission() {
        let context = StaticContext {
            source_kind: CommandSourceKind::Player,
            allowed: vec![PermissionKey::parse("minecraft.command.give").expect("key parses")],
        };
        let requirement = Requirement::Player.and(Requirement::Permission(PermissionExpr::key(
            PermissionKey::parse("minecraft.command.give").expect("key parses"),
        )));

        assert!(requirement.allows(&context));
    }
}
