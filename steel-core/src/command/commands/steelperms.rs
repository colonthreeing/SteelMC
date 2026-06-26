//! Steel permission management commands.

use std::sync::Arc;

use text_components::TextComponent;

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandNodeBuilder, CommandResult, ParsedArguments, PermissionTarget, argument, literal,
};
use crate::command::parsers::{PermissionGroupParser, PermissionKeyParser, PermissionTargetParser};
use crate::command::sender::CommandSender;
use crate::command::{CommandRegistration, CommandRegistrationError};
use crate::permission::{PermissionEntry, PermissionKey, PermissionState};
use crate::server::Server;

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
    let mut reported = 0;
    let mut offline_targets = Vec::new();

    for target in targets {
        if let Some((_, state)) = permission_targets::online_state(&context.server, &target) {
            send_user_info(&context.sender, &target, &state);
            reported += 1;
        } else {
            offline_targets.push(target);
        }
    }

    let scheduled = offline_targets.len();
    if scheduled != 0 {
        spawn_user_info(
            Arc::clone(&context.server),
            context.sender.clone(),
            offline_targets,
        );
    }

    Ok(command_result(reported + scheduled))
}

fn add_group(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let group = group(arguments)?;
    let mut changed = 0;
    let mut offline_targets = Vec::new();

    for target in targets {
        if let Some((player, mut state)) =
            permission_targets::online_state(&context.server, &target)
        {
            if state.groups.iter().any(|assigned| assigned == &group) {
                continue;
            }

            state.groups.push(group.clone());
            permission_targets::save_online_state(&context.server, &player, state)?;
            changed += 1;
        } else {
            offline_targets.push(target);
        }
    }

    let scheduled = offline_targets.len();
    if changed != 0 || scheduled == 0 {
        context.sender.send_message(&TextComponent::plain(format!(
            "Added group '{group}' to {}",
            target_count_text(changed)
        )));
    }
    if scheduled != 0 {
        spawn_add_group(
            Arc::clone(&context.server),
            context.sender.clone(),
            offline_targets,
            group,
        );
    }

    Ok(command_result(changed + scheduled))
}

fn remove_group(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let group = group(arguments)?;
    let mut changed = 0;
    let mut offline_targets = Vec::new();

    for target in targets {
        if let Some((player, mut state)) =
            permission_targets::online_state(&context.server, &target)
        {
            let old_len = state.groups.len();
            state.groups.retain(|assigned| assigned != &group);
            if state.groups.len() == old_len {
                continue;
            }

            permission_targets::save_online_state(&context.server, &player, state)?;
            changed += 1;
        } else {
            offline_targets.push(target);
        }
    }

    let scheduled = offline_targets.len();
    if changed != 0 || scheduled == 0 {
        context.sender.send_message(&TextComponent::plain(format!(
            "Removed group '{group}' from {}",
            target_count_text(changed)
        )));
    }
    if scheduled != 0 {
        spawn_remove_group(
            Arc::clone(&context.server),
            context.sender.clone(),
            offline_targets,
            group,
        );
    }

    Ok(command_result(changed + scheduled))
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
    let mut changed = 0;
    let mut offline_targets = Vec::new();

    for target in targets {
        if let Some((player, mut target_state)) =
            permission_targets::online_state(&context.server, &target)
        {
            target_state.overrides.set(permission.clone(), state);
            permission_targets::save_online_state(&context.server, &player, target_state)?;
            changed += 1;
        } else {
            offline_targets.push(target);
        }
    }

    let scheduled = offline_targets.len();
    if changed != 0 || scheduled == 0 {
        send_set_permission_summary(&context.sender, state, &permission, changed);
    }
    if scheduled != 0 {
        spawn_set_permission(
            Arc::clone(&context.server),
            context.sender.clone(),
            offline_targets,
            permission,
            state,
        );
    }

    Ok(command_result(changed + scheduled))
}

fn unset_permission(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let permission = permission(arguments)?;
    let mut changed = 0;
    let mut offline_targets = Vec::new();

    for target in targets {
        if let Some((player, mut target_state)) =
            permission_targets::online_state(&context.server, &target)
        {
            if !target_state.overrides.unset(&permission) {
                continue;
            }

            permission_targets::save_online_state(&context.server, &player, target_state)?;
            changed += 1;
        } else {
            offline_targets.push(target);
        }
    }

    let scheduled = offline_targets.len();
    if changed != 0 || scheduled == 0 {
        send_unset_permission_summary(&context.sender, &permission, changed);
    }
    if scheduled != 0 {
        spawn_unset_permission(
            Arc::clone(&context.server),
            context.sender.clone(),
            offline_targets,
            permission,
        );
    }

    Ok(command_result(changed + scheduled))
}

fn spawn_user_info(server: Arc<Server>, sender: CommandSender, targets: Vec<PermissionTarget>) {
    tokio::spawn(async move {
        for target in &targets {
            match permission_targets::load_offline_state(&server, target).await {
                Ok(state) => send_user_info(&sender, target, &state),
                Err(error) => send_background_error(&sender, "steelperms user info", error),
            }
        }
    });
}

fn spawn_add_group(
    server: Arc<Server>,
    sender: CommandSender,
    targets: Vec<PermissionTarget>,
    group: String,
) {
    tokio::spawn(async move {
        let mut changed = 0;
        for target in &targets {
            let Ok(mut state) = load_offline_or_report(&server, &sender, target).await else {
                continue;
            };
            if state.groups.iter().any(|assigned| assigned == &group) {
                continue;
            }

            state.groups.push(group.clone());
            if save_offline_or_report(&server, &sender, target, state).await {
                changed += 1;
            }
        }

        sender.send_message(&TextComponent::plain(format!(
            "Added group '{group}' to {}",
            target_count_text(changed)
        )));
    });
}

fn spawn_remove_group(
    server: Arc<Server>,
    sender: CommandSender,
    targets: Vec<PermissionTarget>,
    group: String,
) {
    tokio::spawn(async move {
        let mut changed = 0;
        for target in &targets {
            let Ok(mut state) = load_offline_or_report(&server, &sender, target).await else {
                continue;
            };
            let old_len = state.groups.len();
            state.groups.retain(|assigned| assigned != &group);
            if state.groups.len() == old_len {
                continue;
            }

            if save_offline_or_report(&server, &sender, target, state).await {
                changed += 1;
            }
        }

        sender.send_message(&TextComponent::plain(format!(
            "Removed group '{group}' from {}",
            target_count_text(changed)
        )));
    });
}

fn spawn_set_permission(
    server: Arc<Server>,
    sender: CommandSender,
    targets: Vec<PermissionTarget>,
    permission: PermissionKey,
    state: PermissionState,
) {
    tokio::spawn(async move {
        let mut changed = 0;
        for target in &targets {
            let Ok(mut target_state) = load_offline_or_report(&server, &sender, target).await
            else {
                continue;
            };
            target_state.overrides.set(permission.clone(), state);

            if save_offline_or_report(&server, &sender, target, target_state).await {
                changed += 1;
            }
        }

        send_set_permission_summary(&sender, state, &permission, changed);
    });
}

fn spawn_unset_permission(
    server: Arc<Server>,
    sender: CommandSender,
    targets: Vec<PermissionTarget>,
    permission: PermissionKey,
) {
    tokio::spawn(async move {
        let mut changed = 0;
        for target in &targets {
            let Ok(mut target_state) = load_offline_or_report(&server, &sender, target).await
            else {
                continue;
            };
            if !target_state.overrides.unset(&permission) {
                continue;
            }

            if save_offline_or_report(&server, &sender, target, target_state).await {
                changed += 1;
            }
        }

        send_unset_permission_summary(&sender, &permission, changed);
    });
}

async fn load_offline_or_report(
    server: &Arc<Server>,
    sender: &CommandSender,
    target: &PermissionTarget,
) -> Result<permission_targets::PermissionTargetState, ()> {
    permission_targets::load_offline_state(server, target)
        .await
        .map_err(|error| {
            send_background_error(sender, "steelperms", error);
        })
}

async fn save_offline_or_report(
    server: &Arc<Server>,
    sender: &CommandSender,
    target: &PermissionTarget,
    state: permission_targets::PermissionTargetState,
) -> bool {
    permission_targets::save_offline_state(server, target, state)
        .await
        .map_or_else(
            |error| {
                send_background_error(sender, "steelperms", error);
                false
            },
            |_| true,
        )
}

fn send_user_info(
    sender: &CommandSender,
    target: &PermissionTarget,
    state: &permission_targets::PermissionTargetState,
) {
    let groups = group_list_text(&state.groups);
    let overrides = permission_entries_text(state.overrides.entries());

    sender.send_message(&TextComponent::plain(format!(
        "{}: groups [{}], direct permissions [{}]",
        target.name(),
        groups,
        overrides
    )));
}

fn send_set_permission_summary(
    sender: &CommandSender,
    state: PermissionState,
    permission: &PermissionKey,
    count: usize,
) {
    sender.send_message(&TextComponent::plain(format!(
        "{} permission '{}' for {}",
        permission_action_text(state),
        permission.as_str(),
        target_count_text(count)
    )));
}

fn send_unset_permission_summary(sender: &CommandSender, permission: &PermissionKey, count: usize) {
    sender.send_message(&TextComponent::plain(format!(
        "Unset direct permission '{}' for {}",
        permission.as_str(),
        target_count_text(count)
    )));
}

fn send_background_error(sender: &CommandSender, command: &str, error: CommandError) {
    sender.send_failure_feedback(error.into_feedback(command));
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
