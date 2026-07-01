//! Handler for the "deop" command.
//! Mirrors `net.minecraft.server.commands.DeOpCommands`, backed by Steel permission groups.

use std::sync::Arc;

use steel_protocol::packets::game::SuggestionEntry;
use steel_utils::translations;
use text_components::TextComponent;

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandArgumentClientParser, CommandArgumentParser, CommandNodeBuilder, CommandParseError,
    CommandResult, ParsedArgument, ParsedArguments, PermissionTarget, argument, literal,
};
use crate::command::parsers::{PermissionTargetParser, resolve_required_permission_targets};
use crate::command::reader::CommandReader;
use crate::command::requirement::CommandInputContext;
use crate::command::sender::CommandSender;
use crate::command::CommandRegistrationSpec;
use crate::permission::OP_GROUP;
use crate::server::Server;

use super::permission_targets;

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Creates the `/deop` command handler.
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
    literal("deop").then(argument("targets", DeOpTargetsParser).executes(deop_targets))
}

fn deop_targets(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = resolve_required_permission_targets(arguments, "targets", context)?;

    let mut changed_count = 0;
    let mut offline_targets = Vec::new();
    for target in targets {
        if let Some((player, mut state)) =
            permission_targets::online_state(&context.server, &target)
        {
            if !remove_op_group(&mut state.groups) {
                continue;
            }

            permission_targets::save_online_state(&context.server, &player, state)?;
            changed_count += 1;
            send_deop_success(&context.sender, &target);
        } else {
            offline_targets.push(target);
        }
    }

    let scheduled = offline_targets.len();
    if scheduled != 0 {
        spawn_offline_deop(
            Arc::clone(&context.server),
            context.sender.clone(),
            offline_targets,
        );
    }

    if changed_count == 0 && scheduled == 0 {
        return Err(CommandError::failure(
            translations::COMMANDS_DEOP_FAILED.msg(),
        ));
    }

    Ok(CommandResult::from_usize_success_count(changed_count))
}

fn spawn_offline_deop(server: Arc<Server>, sender: CommandSender, targets: Vec<PermissionTarget>) {
    tokio::spawn(async move {
        let mut changed_count = 0;
        for target in targets {
            let loaded = permission_targets::load_state(&server, target).await;
            let Ok(mut loaded) = loaded.map_err(|error| {
                send_background_error(&sender, error);
            }) else {
                continue;
            };
            let state = loaded.state_mut();
            if !remove_op_group(&mut state.groups) {
                continue;
            }

            let target = loaded.target().clone();
            match permission_targets::save_state(&server, loaded).await {
                Ok(()) => {
                    changed_count += 1;
                    send_deop_success(&sender, &target);
                }
                Err(error) => send_background_error(&sender, error),
            }
        }

        if changed_count == 0 {
            sender.send_failure(translations::COMMANDS_DEOP_FAILED.msg());
        }
    });
}

fn remove_op_group(groups: &mut Vec<String>) -> bool {
    let old_len = groups.len();
    groups.retain(|group| group != OP_GROUP);
    groups.len() != old_len
}

fn send_deop_success(sender: &CommandSender, target: &PermissionTarget) {
    sender.send_message(
        &translations::COMMANDS_DEOP_SUCCESS
            .message([TextComponent::plain(target.name().to_owned())])
            .into(),
    );
}

fn send_background_error(sender: &CommandSender, error: CommandError) {
    sender.send_failure_feedback(error.into_feedback("deop"));
}

#[derive(Clone, Copy, Debug, Default)]
struct DeOpTargetsParser;

impl CommandArgumentParser for DeOpTargetsParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        PermissionTargetParser.parse(reader, context)
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        PermissionTargetParser.client_parser()
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
        for player in server.get_players() {
            if player
                .permission_groups()
                .iter()
                .any(|group| group == OP_GROUP)
            {
                suggestions.push(SuggestionEntry::new(player.gameprofile.name.clone()));
            }
        }
        for known in server.known_players().entries() {
            if suggestions.iter().any(|suggestion| {
                suggestion
                    .text
                    .eq_ignore_ascii_case(known.last_known_name())
            }) {
                continue;
            }
            if server
                .global_permission_state(known.uuid())
                .is_some_and(|state| state.groups().iter().any(|group| group == OP_GROUP))
            {
                suggestions.push(SuggestionEntry::new(known.last_known_name().to_owned()));
            }
        }

        suggestions.retain(|suggestion| suggestion.text.starts_with(prefix));
        suggestions
    }
}

#[cfg(test)]
mod tests {
    use super::remove_op_group;

    #[test]
    fn remove_op_group_removes_only_op_assignments() {
        let mut groups = vec!["default".to_owned(), "op".to_owned(), "admin".to_owned()];

        assert!(remove_op_group(&mut groups));
        assert_eq!(groups, vec!["default", "admin"]);
        assert!(!remove_op_group(&mut groups));
    }
}
