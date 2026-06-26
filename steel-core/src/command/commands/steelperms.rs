//! Steel permission management commands.

use std::sync::Arc;

use text_components::TextComponent;

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandNodeBuilder, CommandResult, ParsedArguments, argument, literal,
};
use crate::command::parsers::{PermissionGroupParser, PermissionKeyParser, PlayerParser};
use crate::command::{CommandRegistration, CommandRegistrationError};
use crate::permission::{PermissionEntry, PermissionKey, PermissionState};
use crate::player::Player;

pub(crate) fn registration() -> Result<CommandRegistration, CommandRegistrationError> {
    CommandRegistration::steel(command())?.alias("sp")
}

/// Handler for the "steelperms" command group.
#[must_use]
pub fn command() -> CommandNodeBuilder {
    literal("steelperms").then(
        literal("user").then(
            argument("targets", PlayerParser::multiple())
                .then(
                    literal("info")
                        .requires_subcommand_permission()
                        .executes(user_info),
                )
                .then(
                    literal("allow").requires_subcommand_permission().then(
                        argument("permission", PermissionKeyParser).executes(allow_permission),
                    ),
                )
                .then(
                    literal("deny").requires_subcommand_permission().then(
                        argument("permission", PermissionKeyParser).executes(deny_permission),
                    ),
                )
                .then(
                    literal("unset").requires_subcommand_permission().then(
                        argument("permission", PermissionKeyParser).executes(unset_permission),
                    ),
                )
                .then(
                    literal("group").then(
                        literal("add")
                            .requires_subcommand_permission()
                            .then(argument("group", PermissionGroupParser).executes(add_group)),
                    ),
                )
                .then(
                    literal("group").then(
                        literal("remove")
                            .requires_subcommand_permission()
                            .then(argument("group", PermissionGroupParser).executes(remove_group)),
                    ),
                ),
        ),
    )
}

fn user_info(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    for target in &targets {
        let groups = group_list_text(&target.permission_groups());
        let overrides = permission_entries_text(target.permission_overrides().entries());

        context.sender.send_message(&TextComponent::plain(format!(
            "{}: groups [{}], direct permissions [{}]",
            target.gameprofile.name, groups, overrides
        )));
    }

    Ok(command_result(targets.len()))
}

fn add_group(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let group = group(arguments)?;
    let mut changed = 0;

    for target in &targets {
        let mut groups = target.permission_groups();
        if groups.iter().any(|assigned| assigned == &group) {
            continue;
        }
        groups.push(group.clone());
        context
            .server
            .update_player_global_permissions(target, groups, target.permission_overrides())
            .map_err(|error| CommandError::failure(error.to_string()))?;
        changed += 1;
    }

    context.sender.send_message(&TextComponent::plain(format!(
        "Added group '{group}' to {}",
        target_count_text(changed)
    )));
    Ok(command_result(changed))
}

fn remove_group(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let group = group(arguments)?;
    let mut changed = 0;

    for target in &targets {
        let mut groups = target.permission_groups();
        let old_len = groups.len();
        groups.retain(|assigned| assigned != &group);
        if groups.len() == old_len {
            continue;
        }
        context
            .server
            .update_player_global_permissions(target, groups, target.permission_overrides())
            .map_err(|error| CommandError::failure(error.to_string()))?;
        changed += 1;
    }

    context.sender.send_message(&TextComponent::plain(format!(
        "Removed group '{group}' from {}",
        target_count_text(changed)
    )));
    Ok(command_result(changed))
}

fn allow_permission(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    set_permission(context, arguments, PermissionState::Allow)
}

fn deny_permission(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    set_permission(context, arguments, PermissionState::Deny)
}

fn set_permission(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    state: PermissionState,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let permission = permission(arguments)?;

    for target in &targets {
        let mut overrides = target.permission_overrides();
        overrides.set(permission.clone(), state);
        context
            .server
            .update_player_global_permissions(target, target.permission_groups(), overrides)
            .map_err(|error| CommandError::failure(error.to_string()))?;
    }

    context.sender.send_message(&TextComponent::plain(format!(
        "{} permission '{}' for {}",
        permission_action_text(state),
        permission.as_str(),
        target_count_text(targets.len())
    )));
    Ok(command_result(targets.len()))
}

fn unset_permission(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let permission = permission(arguments)?;
    let mut changed = 0;

    for target in &targets {
        let mut overrides = target.permission_overrides();
        if !overrides.unset(&permission) {
            continue;
        }

        context
            .server
            .update_player_global_permissions(target, target.permission_groups(), overrides)
            .map_err(|error| CommandError::failure(error.to_string()))?;
        changed += 1;
    }

    context.sender.send_message(&TextComponent::plain(format!(
        "Unset direct permission '{}' for {}",
        permission.as_str(),
        target_count_text(changed)
    )));
    Ok(command_result(changed))
}

fn targets(arguments: &ParsedArguments) -> Result<Vec<Arc<Player>>, CommandError> {
    let targets = arguments
        .get::<Vec<Arc<Player>>>("targets")
        .map_err(super::invalid_parsed_argument)?;
    if targets.is_empty() {
        return Err(CommandError::failure("No players matched"));
    }

    Ok(targets)
}

fn group(arguments: &ParsedArguments) -> Result<String, CommandError> {
    arguments
        .get::<String>("group")
        .map_err(super::invalid_parsed_argument)
}

fn permission(arguments: &ParsedArguments) -> Result<PermissionKey, CommandError> {
    arguments
        .get::<PermissionKey>("permission")
        .map_err(super::invalid_parsed_argument)
}

fn group_list_text(groups: &[String]) -> String {
    if groups.is_empty() {
        return "none".to_owned();
    }

    groups.join(", ")
}

fn permission_entries_text(entries: &[PermissionEntry]) -> String {
    if entries.is_empty() {
        return "none".to_owned();
    }

    entries
        .iter()
        .map(|entry| {
            format!(
                "{} {}",
                permission_state_text(entry.state()),
                entry.key().as_str()
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn permission_state_text(state: PermissionState) -> &'static str {
    match state {
        PermissionState::Allow => "allow",
        PermissionState::Deny => "deny",
    }
}

fn permission_action_text(state: PermissionState) -> &'static str {
    match state {
        PermissionState::Allow => "Allowed",
        PermissionState::Deny => "Denied",
    }
}

fn target_count_text(count: usize) -> String {
    match count {
        1 => "1 player".to_owned(),
        _ => format!("{count} players"),
    }
}

fn command_result(count: usize) -> CommandResult {
    CommandResult {
        success_count: i32::try_from(count).map_or(i32::MAX, |count| count),
    }
}
