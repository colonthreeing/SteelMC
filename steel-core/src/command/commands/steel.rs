//! Steel server commands: /steel tp <targets> <world>

use std::sync::Arc;

use text_components::TextComponent;

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandNodeBuilder, CommandResult, ParsedArgumentError, ParsedArguments, argument, literal,
};
use crate::command::parsers::{PlayerParser, WorldParser};
use crate::command::{CommandRegistration, CommandRegistrationError};
use crate::entity::SharedEntity;
use crate::player::Player;
use crate::portal::WorldChangeRequest;
use crate::world::World;

pub(crate) fn registration() -> Result<CommandRegistration, CommandRegistrationError> {
    CommandRegistration::steel(command())
}

/// Handler for the "steel" command group.
#[must_use]
pub fn command() -> CommandNodeBuilder {
    literal("steel").then(
        literal("tp").then(
            argument("targets", PlayerParser::multiple())
                .then(argument("world", WorldParser).executes(teleport_to_world)),
        ),
    )
}

fn teleport_to_world(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = arguments
        .get::<Vec<Arc<Player>>>("targets")
        .map_err(invalid_parsed_argument)?;
    let world = arguments
        .get::<Arc<World>>("world")
        .map_err(invalid_parsed_argument)?;
    let dim_name = &world.key;
    let count = targets.len();

    for target in &targets {
        if target.is_domain_switching() {
            return Err(CommandError::failure(format!(
                "{} is already switching domains",
                target.gameprofile.name
            )));
        }
    }

    for target in &targets {
        let current_world = target.get_world();
        if current_world.domain() == world.domain() {
            context.server.queue_world_change(
                target.clone() as SharedEntity,
                WorldChangeRequest::WorldSpawn {
                    target_world: world.clone(),
                },
            );
        } else {
            context
                .server
                .queue_domain_switch_to_world(target.clone(), world.clone())
                .map_err(CommandError::failure)?;
        }
    }

    let msg = if count == 1 {
        format!(
            "Teleporting {} to {}",
            targets[0].gameprofile.name, dim_name
        )
    } else {
        format!("Teleporting {count} players to {dim_name}")
    };
    context.sender.send_message(&TextComponent::from(msg));

    Ok(CommandResult::success())
}

fn invalid_parsed_argument(error: ParsedArgumentError) -> CommandError {
    CommandError::InvalidConsumption(Some(format!("{error:?}")))
}
