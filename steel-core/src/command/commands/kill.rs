//! Handler for the "kill" command.
//! Mirrors `net.minecraft.server.commands.KillCommand`.

use text_components::TextComponent;

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandNodeBuilder, CommandResult, ParsedArguments, argument, literal,
};
use crate::command::parsers::EntityParser;
use crate::command::CommandRegistrationSpec;
use crate::entity::SharedEntity;
use steel_utils::translations;

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Creates the `/kill` command handler.
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
    literal("kill")
        .executes(kill_self)
        .then(argument("targets", EntityParser::multiple()).executes(kill_targets))
}

fn kill_self(
    context: &mut CommandContext,
    _: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let entity = context
        .entity
        .as_ref()
        .ok_or(CommandError::InvalidRequirement)?;
    let entity_name = entity.plain_text_name();

    entity.kill();

    // TODO: use getDisplayName() (team formatting, hover event, UUID insertion)
    context.sender.send_message(
        &translations::COMMANDS_KILL_SUCCESS_SINGLE
            .message([TextComponent::plain(entity_name)])
            .into(),
    );

    Ok(CommandResult::success())
}

fn kill_targets(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = arguments
        .get::<Vec<SharedEntity>>("targets")
        .map_err(super::invalid_parsed_argument)?;

    if targets.is_empty() {
        return Err(CommandError::failure("No entity was found"));
    }

    let mut last_name = String::new();
    for target in &targets {
        last_name = target.plain_text_name();
        target.kill();
    }
    let victim_count = targets.len();

    // TODO: use getDisplayName() (team formatting, hover event, UUID insertion)
    if victim_count == 1 {
        context.sender.send_message(
            &translations::COMMANDS_KILL_SUCCESS_SINGLE
                .message([TextComponent::plain(last_name)])
                .into(),
        );
    } else {
        context.sender.send_message(
            &translations::COMMANDS_KILL_SUCCESS_MULTIPLE
                .message([TextComponent::plain(victim_count.to_string())])
                .into(),
        );
    }

    Ok(CommandResult {
        success_count: success_count(victim_count),
    })
}

fn success_count(count: usize) -> i32 {
    i32::try_from(count).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::Weak;

    use glam::DVec3;
    use steel_registry::{entity_type::EntityTypeRef, vanilla_entities};

    use crate::entity::{Entity, EntityBase, RemovalReason};

    struct KillTestEntity {
        base: EntityBase,
    }

    impl KillTestEntity {
        fn new() -> Self {
            Self {
                base: EntityBase::new(
                    1,
                    DVec3::ZERO,
                    vanilla_entities::ITEM.dimensions,
                    Weak::new(),
                ),
            }
        }
    }

    impl Entity for KillTestEntity {
        fn base(&self) -> &EntityBase {
            &self.base
        }

        fn entity_type(&self) -> EntityTypeRef {
            &vanilla_entities::ITEM
        }
    }

    #[test]
    fn kill_removes_non_living_entities_as_killed() {
        let entity = KillTestEntity::new();

        entity.kill();

        assert_eq!(entity.removal_reason(), Some(RemovalReason::Killed));
    }
}
