//! Command node requirements.

use std::sync::Arc;

use glam::DVec3;

use crate::{
    command::{
        context::{CommandContext, EntityAnchor},
        sender::CommandSender,
    },
    entity::SharedEntity,
    player::Player,
    server::Server,
    world::World,
};

pub use crate::permission::{
    PermissionCatalog, PermissionContextCatalog, PermissionExpr, PermissionKey, PermissionKeyError,
    PermissionMetadataCatalog,
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

    /// Returns the active command source entity when available.
    fn entity(&self) -> Option<&SharedEntity> {
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

    /// Returns the permission catalog available for command suggestions.
    fn permission_catalog(&self) -> Option<&PermissionCatalog> {
        None
    }

    /// Returns the permission metadata catalog available for command suggestions.
    fn permission_metadata_catalog(&self) -> Option<&PermissionMetadataCatalog> {
        None
    }

    /// Returns the permission context catalog available for command suggestions.
    fn permission_context_catalog(&self) -> Option<&PermissionContextCatalog> {
        None
    }
}

/// A non-permission or permission-backed command requirement.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Requirement {
    /// The node is always usable.
    #[default]
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

impl RequirementContext for CommandContext {
    fn source_kind(&self) -> CommandSourceKind {
        sender_source_kind(&self.sender)
    }

    fn has_permission(&self, permission: &PermissionExpr) -> bool {
        sender_has_permission(&self.sender, permission, self)
    }
}

fn sender_source_kind(sender: &CommandSender) -> CommandSourceKind {
    match sender {
        CommandSender::Player(_) => CommandSourceKind::Player,
        CommandSender::Console => CommandSourceKind::Console,
        CommandSender::Rcon => CommandSourceKind::Rcon,
        CommandSender::SuppressedOutput(sender) => sender_source_kind(sender),
    }
}

fn sender_has_permission(
    sender: &CommandSender,
    permission: &PermissionExpr,
    context: &CommandContext,
) -> bool {
    match sender {
        CommandSender::Player(player) => {
            player.has_permission_in(permission, context.permission_check_context())
        }
        CommandSender::Console | CommandSender::Rcon => true,
        CommandSender::SuppressedOutput(sender) => {
            sender_has_permission(sender, permission, context)
        }
    }
}

impl RequirementContext for Player {
    fn source_kind(&self) -> CommandSourceKind {
        CommandSourceKind::Player
    }

    fn has_permission(&self, permission: &PermissionExpr) -> bool {
        Player::has_permission(self, permission)
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

    fn entity(&self) -> Option<&SharedEntity> {
        self.entity.as_ref()
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

    fn permission_catalog(&self) -> Option<&PermissionCatalog> {
        self.permission_catalog()
    }

    fn permission_metadata_catalog(&self) -> Option<&PermissionMetadataCatalog> {
        self.permission_metadata_catalog()
    }

    fn permission_context_catalog(&self) -> Option<&PermissionContextCatalog> {
        self.permission_context_catalog()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CommandInputContext, CommandSourceKind, PermissionExpr, PermissionKey, Requirement,
        RequirementContext,
    };
    use crate::permission::{PermissionEntry, PermissionSet};

    struct StaticContext {
        source_kind: CommandSourceKind,
        permissions: PermissionSet,
    }

    impl RequirementContext for StaticContext {
        fn source_kind(&self) -> CommandSourceKind {
            self.source_kind
        }

        fn has_permission(&self, permission: &PermissionExpr) -> bool {
            self.permissions.allows(permission)
        }
    }

    impl CommandInputContext for StaticContext {}

    #[test]
    fn requirements_can_combine_source_and_permission() {
        let context = StaticContext {
            source_kind: CommandSourceKind::Player,
            permissions: PermissionSet::from_entries([PermissionEntry::allow(
                PermissionKey::parse("minecraft.command.give").expect("key parses"),
            )]),
        };
        let requirement = Requirement::Player.and(Requirement::Permission(PermissionExpr::key(
            PermissionKey::parse("minecraft.command.give").expect("key parses"),
        )));

        assert!(requirement.allows(&context));
    }
}
