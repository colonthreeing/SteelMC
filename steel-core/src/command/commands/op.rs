//! Handler for the "op" command.
//! Mirrors `net.minecraft.server.commands.OpCommand`, backed by Steel permission groups.

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use steel_utils::translations;
use text_components::TextComponent;

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandArgumentParser, CommandFuture, CommandNodeBuilder, CommandParseError, CommandResult,
    ParsedArgument, ParsedArguments, PermissionTarget, argument, literal,
};
use crate::command::parsers::PermissionTargetParser;
use crate::command::reader::CommandReader;
use crate::command::requirement::CommandInputContext;
use crate::command::{CommandRegistration, CommandRegistrationError};

use super::permission_targets;

const OP_GROUP: &str = "op";

pub(crate) fn registration() -> Result<CommandRegistration, CommandRegistrationError> {
    CommandRegistration::minecraft(command())
}

/// Creates the `/op` command handler.
#[must_use]
pub fn command() -> CommandNodeBuilder {
    literal("op").then(argument("targets", OpTargetsParser).executes_async(op_targets))
}

fn op_targets<'a>(
    context: &'a mut CommandContext,
    arguments: &'a ParsedArguments,
) -> CommandFuture<'a> {
    Box::pin(async move {
        let targets = arguments
            .get::<Vec<PermissionTarget>>("targets")
            .map_err(super::invalid_parsed_argument)?;
        if targets.is_empty() {
            return Err(CommandError::failure("No player was found"));
        }

        let mut changed_count = 0;
        for target in &targets {
            let mut state = permission_targets::load_state(context, target).await?;
            if state.groups.iter().any(|group| group == OP_GROUP) {
                continue;
            }

            state.groups.push(OP_GROUP.to_owned());
            permission_targets::save_state(context, target, state).await?;
            changed_count += 1;

            context.sender.send_message(
                &translations::COMMANDS_OP_SUCCESS
                    .message([TextComponent::plain(target.name().to_owned())])
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
        PermissionTargetParser.parse(reader, context)
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        PermissionTargetParser.usage()
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

        let mut suggestions = Vec::new();
        let mut hidden_online_names = Vec::new();
        for player in server.get_players() {
            if player
                .permission_groups()
                .iter()
                .any(|group| group == OP_GROUP)
            {
                hidden_online_names.push(player.gameprofile.name.clone());
                continue;
            }
            suggestions.push(SuggestionEntry::new(player.gameprofile.name.clone()));
        }
        for known in server.known_players().entries() {
            if hidden_online_names
                .iter()
                .any(|name| name.eq_ignore_ascii_case(known.last_known_name()))
            {
                continue;
            }
            if suggestions.iter().any(|suggestion| {
                suggestion
                    .text
                    .eq_ignore_ascii_case(known.last_known_name())
            }) {
                continue;
            }
            suggestions.push(SuggestionEntry::new(known.last_known_name().to_owned()));
        }

        suggestions.retain(|suggestion| suggestion.text.starts_with(prefix));
        suggestions
    }
}
