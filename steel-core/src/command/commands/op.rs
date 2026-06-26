//! Handler for the "op" command.
//! Mirrors `net.minecraft.server.commands.OpCommand`, backed by Steel permission groups.

use std::sync::Arc;

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use steel_utils::translations;
use text_components::TextComponent;

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandArgumentParser, CommandNodeBuilder, CommandParseError, CommandResult, ParsedArgument,
    ParsedArguments, PermissionTarget, argument, literal,
};
use crate::command::parsers::PermissionTargetParser;
use crate::command::reader::CommandReader;
use crate::command::requirement::CommandInputContext;
use crate::command::sender::CommandSender;
use crate::command::{CommandRegistration, CommandRegistrationError};
use crate::permission::OP_GROUP;
use crate::server::Server;

use super::permission_targets;

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
        .get::<Vec<PermissionTarget>>("targets")
        .map_err(super::invalid_parsed_argument)?;
    if targets.is_empty() {
        return Err(CommandError::failure("No player was found"));
    }

    let mut changed_count = 0;
    let mut offline_targets = Vec::new();
    for target in targets {
        if let Some((player, mut state)) =
            permission_targets::online_state(&context.server, &target)
        {
            if state.groups.iter().any(|group| group == OP_GROUP) {
                continue;
            }

            state.groups.push(OP_GROUP.to_owned());
            permission_targets::save_online_state(&context.server, &player, state)?;
            changed_count += 1;
            send_op_success(&context.sender, &target);
        } else {
            offline_targets.push(target);
        }
    }

    let scheduled = offline_targets.len();
    if scheduled != 0 {
        spawn_offline_op(
            Arc::clone(&context.server),
            context.sender.clone(),
            offline_targets,
        );
    }

    if changed_count == 0 && scheduled == 0 {
        return Err(CommandError::failure(
            translations::COMMANDS_OP_FAILED.msg(),
        ));
    }

    Ok(CommandResult {
        success_count: i32::try_from(changed_count + scheduled).map_or(i32::MAX, |count| count),
    })
}

fn spawn_offline_op(server: Arc<Server>, sender: CommandSender, targets: Vec<PermissionTarget>) {
    tokio::spawn(async move {
        let mut changed_count = 0;
        for target in &targets {
            let state = permission_targets::load_offline_state(&server, target).await;
            let Ok(mut state) = state.map_err(|error| {
                send_background_error(&sender, error);
            }) else {
                continue;
            };
            if state.groups.iter().any(|group| group == OP_GROUP) {
                continue;
            }

            state.groups.push(OP_GROUP.to_owned());
            match permission_targets::save_offline_state(&server, target, state).await {
                Ok(()) => {
                    changed_count += 1;
                    send_op_success(&sender, target);
                }
                Err(error) => send_background_error(&sender, error),
            }
        }

        if changed_count == 0 {
            sender.send_failure(translations::COMMANDS_OP_FAILED.msg());
        }
    });
}

fn send_op_success(sender: &CommandSender, target: &PermissionTarget) {
    sender.send_message(
        &translations::COMMANDS_OP_SUCCESS
            .message([TextComponent::plain(target.name().to_owned())])
            .into(),
    );
}

fn send_background_error(sender: &CommandSender, error: CommandError) {
    sender.send_failure_feedback(error.into_feedback("op"));
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

    fn parsed_type(&self) -> &'static str {
        PermissionTargetParser.parsed_type()
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
