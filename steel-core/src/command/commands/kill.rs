//! Handler for the "kill" command.
//! Mirrors `net.minecraft.server.commands.KillCommand`.

use std::sync::Arc;

use text_components::TextComponent;

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandNodeBuilder, CommandResult, ParsedArguments, argument, literal,
};
use crate::command::parsers::EntityParser;
use crate::command::CommandRegistrationSpec;
use crate::entity::damage::DamageSource;
use crate::entity::{Entity, LivingEntity};
use crate::player::Player;
use steel_registry::vanilla_damage_types;
use steel_utils::translations;

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Creates the `/kill` command handler.
#[must_use]
pub fn command() -> CommandNodeBuilder {
    literal("kill")
        .executes(kill_self)
        .then(argument("targets", EntityParser::multiple()).executes(kill_targets))
}

/// `LivingEntity.kill()` — hurt with `genericKill` at `Float.MAX_VALUE`.
fn kill_player(player: &Player) {
    player.hurt(
        &DamageSource::environment(&vanilla_damage_types::GENERIC_KILL),
        f32::MAX,
    );
}

fn kill_self(
    context: &mut CommandContext,
    _: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let player = context
        .sender
        .get_player()
        .ok_or(CommandError::InvalidRequirement)?;

    kill_player(player);

    // TODO: use getDisplayName() (team formatting, hover event, UUID insertion)
    context.sender.send_message(
        &translations::COMMANDS_KILL_SUCCESS_SINGLE
            .message([TextComponent::plain(player.gameprofile.name.clone())])
            .into(),
    );

    Ok(CommandResult::success())
}

fn kill_targets(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = arguments
        .get::<Vec<Arc<dyn LivingEntity + Send + Sync>>>("targets")
        .map_err(super::invalid_parsed_argument)?;

    if targets.is_empty() {
        return Err(CommandError::failure("No entity was found"));
    }

    let players = context.server.get_players();

    let mut last_name = String::new();
    let mut victim_count = 0;
    for target in &targets {
        let target_uuid = target.uuid();
        if let Some(player) = players.iter().find(|p| p.uuid() == target_uuid) {
            kill_player(player);
            victim_count += 1;
            last_name.clone_from(&player.gameprofile.name);
        }
        // TODO: non-player entities via Entity::kill() (remove with RemovalReason::KILLED)
    }

    if victim_count == 0 {
        return Err(CommandError::failure("No entity was found"));
    }

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

    Ok(CommandResult::success())
}
