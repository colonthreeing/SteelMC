//! Steel permission management commands.

use text_components::TextComponent;

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandFuture, CommandNodeBuilder, CommandResult, ParsedArguments, PermissionTarget, argument,
    literal,
};
use crate::command::parsers::{PermissionGroupParser, PermissionKeyParser, PermissionTargetParser};
use crate::command::{CommandRegistration, CommandRegistrationError};
use crate::permission::{PermissionEntry, PermissionKey, PermissionState};

use super::permission_targets;

pub(crate) fn registration() -> Result<CommandRegistration, CommandRegistrationError> {
    CommandRegistration::steel(command())?.alias("sp")
}

/// Handler for the "steelperms" command group.
#[must_use]
pub fn command() -> CommandNodeBuilder {
    literal("steelperms").then(
        literal("user").then(
            argument("targets", PermissionTargetParser)
                .then(
                    literal("info")
                        .requires_subcommand_permission()
                        .executes_async(user_info),
                )
                .then(literal("allow").requires_subcommand_permission().then(
                    argument("permission", PermissionKeyParser).executes_async(allow_permission),
                ))
                .then(literal("deny").requires_subcommand_permission().then(
                    argument("permission", PermissionKeyParser).executes_async(deny_permission),
                ))
                .then(literal("unset").requires_subcommand_permission().then(
                    argument("permission", PermissionKeyParser).executes_async(unset_permission),
                ))
                .then(
                    literal("group").then(
                        literal("add").requires_subcommand_permission().then(
                            argument("group", PermissionGroupParser).executes_async(add_group),
                        ),
                    ),
                )
                .then(literal("group").then(
                    literal("remove").requires_subcommand_permission().then(
                        argument("group", PermissionGroupParser).executes_async(remove_group),
                    ),
                )),
        ),
    )
}

fn user_info<'a>(
    context: &'a mut CommandContext,
    arguments: &'a ParsedArguments,
) -> CommandFuture<'a> {
    Box::pin(async move {
        let targets = targets(arguments)?;
        for target in &targets {
            let state = permission_targets::load_state(context, target).await?;
            let groups = group_list_text(&state.groups);
            let overrides = permission_entries_text(state.overrides.entries());

            context.sender.send_message(&TextComponent::plain(format!(
                "{}: groups [{}], direct permissions [{}]",
                target.name(),
                groups,
                overrides
            )));
        }

        Ok(command_result(targets.len()))
    })
}

fn add_group<'a>(
    context: &'a mut CommandContext,
    arguments: &'a ParsedArguments,
) -> CommandFuture<'a> {
    Box::pin(async move {
        let targets = targets(arguments)?;
        let group = group(arguments)?;
        let mut changed = 0;

        for target in &targets {
            let mut state = permission_targets::load_state(context, target).await?;
            if state.groups.iter().any(|assigned| assigned == &group) {
                continue;
            }
            state.groups.push(group.clone());
            permission_targets::save_state(context, target, state).await?;
            changed += 1;
        }

        context.sender.send_message(&TextComponent::plain(format!(
            "Added group '{group}' to {}",
            target_count_text(changed)
        )));
        Ok(command_result(changed))
    })
}

fn remove_group<'a>(
    context: &'a mut CommandContext,
    arguments: &'a ParsedArguments,
) -> CommandFuture<'a> {
    Box::pin(async move {
        let targets = targets(arguments)?;
        let group = group(arguments)?;
        let mut changed = 0;

        for target in &targets {
            let mut state = permission_targets::load_state(context, target).await?;
            let old_len = state.groups.len();
            state.groups.retain(|assigned| assigned != &group);
            if state.groups.len() == old_len {
                continue;
            }
            permission_targets::save_state(context, target, state).await?;
            changed += 1;
        }

        context.sender.send_message(&TextComponent::plain(format!(
            "Removed group '{group}' from {}",
            target_count_text(changed)
        )));
        Ok(command_result(changed))
    })
}

fn allow_permission<'a>(
    context: &'a mut CommandContext,
    arguments: &'a ParsedArguments,
) -> CommandFuture<'a> {
    set_permission(context, arguments, PermissionState::Allow)
}

fn deny_permission<'a>(
    context: &'a mut CommandContext,
    arguments: &'a ParsedArguments,
) -> CommandFuture<'a> {
    set_permission(context, arguments, PermissionState::Deny)
}

fn set_permission<'a>(
    context: &'a mut CommandContext,
    arguments: &'a ParsedArguments,
    state: PermissionState,
) -> CommandFuture<'a> {
    Box::pin(async move {
        let targets = targets(arguments)?;
        let permission = permission(arguments)?;

        for target in &targets {
            let mut target_state = permission_targets::load_state(context, target).await?;
            target_state.overrides.set(permission.clone(), state);
            permission_targets::save_state(context, target, target_state).await?;
        }

        context.sender.send_message(&TextComponent::plain(format!(
            "{} permission '{}' for {}",
            permission_action_text(state),
            permission.as_str(),
            target_count_text(targets.len())
        )));
        Ok(command_result(targets.len()))
    })
}

fn unset_permission<'a>(
    context: &'a mut CommandContext,
    arguments: &'a ParsedArguments,
) -> CommandFuture<'a> {
    Box::pin(async move {
        let targets = targets(arguments)?;
        let permission = permission(arguments)?;
        let mut changed = 0;

        for target in &targets {
            let mut target_state = permission_targets::load_state(context, target).await?;
            if !target_state.overrides.unset(&permission) {
                continue;
            }

            permission_targets::save_state(context, target, target_state).await?;
            changed += 1;
        }

        context.sender.send_message(&TextComponent::plain(format!(
            "Unset direct permission '{}' for {}",
            permission.as_str(),
            target_count_text(changed)
        )));
        Ok(command_result(changed))
    })
}

fn targets(arguments: &ParsedArguments) -> Result<Vec<PermissionTarget>, CommandError> {
    let targets = arguments
        .get::<Vec<PermissionTarget>>("targets")
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
