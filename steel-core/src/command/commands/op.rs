//! Handler for the "op" command.
//! Mirrors `net.minecraft.server.commands.OpCommand` for online player targets.

use std::sync::Arc;

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use steel_utils::translations;
use text_components::TextComponent;

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandArgumentParser, CommandNodeBuilder, CommandParseError, CommandResult, ParsedArgument,
    ParsedArguments, argument, literal,
};
use crate::command::parsers::PlayerParser;
use crate::command::reader::CommandReader;
use crate::command::requirement::CommandInputContext;
use crate::command::{CommandRegistration, CommandRegistrationError};
use crate::player::Player;

const OP_GROUP: &str = "op";

pub(crate) fn registration() -> Result<CommandRegistration, CommandRegistrationError> {
    CommandRegistration::minecraft(command())
}

/// Creates the `/op` command handler.
#[must_use]
pub fn command() -> CommandNodeBuilder {
    literal("op").then(argument("targets", OpTargetsParser).executes(op_targets))
}

fn op_targets(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = arguments
        .get::<Vec<Arc<Player>>>("targets")
        .map_err(super::invalid_parsed_argument)?;
    if targets.is_empty() {
        return Err(CommandError::failure("No player was found"));
    }

    let mut changed_count = 0;
    for target in &targets {
        let mut groups = target.permission_groups();
        if groups.iter().any(|group| group == OP_GROUP) {
            continue;
        }

        groups.push(OP_GROUP.to_owned());
        context
            .server
            .update_player_global_permissions(target, groups, target.permission_overrides())
            .map_err(|error| CommandError::failure(error.to_string()))?;
        changed_count += 1;

        context.sender.send_message(
            &translations::COMMANDS_OP_SUCCESS
                .message([TextComponent::plain(target.gameprofile.name.clone())])
                .into(),
        );
    }

    if changed_count == 0 {
        return Err(CommandError::failure(
            translations::COMMANDS_OP_FAILED.msg(),
        ));
    }

    Ok(CommandResult {
        success_count: i32::try_from(changed_count).map_or(i32::MAX, |count| count),
    })
}

#[derive(Clone, Copy, Debug, Default)]
struct OpTargetsParser;

impl CommandArgumentParser for OpTargetsParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        PlayerParser::multiple().parse(reader, context)
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        PlayerParser::multiple().usage()
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let Some(server) = context.server() else {
            return Vec::new();
        };

        server
            .get_players()
            .iter()
            .filter(|player| {
                !player
                    .permission_groups()
                    .iter()
                    .any(|group| group == OP_GROUP)
            })
            .map(|player| player.gameprofile.name.clone())
            .filter(|name| name.starts_with(prefix))
            .map(SuggestionEntry::new)
            .collect()
    }
}
