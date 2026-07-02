//! Message and display helpers for the `steelperms` command.

use steel_utils::Identifier;
use text_components::TextComponent;

use crate::command::error::CommandError;
use crate::command::graph::{CommandResult, PermissionTarget};
use crate::command::sender::CommandSender;
use crate::permission::{
    PermissionEntry, PermissionKey, PermissionResolution, PermissionResolutionSource,
    PermissionMetadataRuleConfig, PermissionRuleContext, PermissionState, PermissionMetadataValue,
    PermissionMetadataEntry, PermissionMetadataResolution,
};

use super::config::PermissionGroupEditError;
pub(super) fn send_user_info(
    sender: &CommandSender,
    target: &PermissionTarget,
    groups: &[String],
    overrides: &[PermissionEntry],
    metadata: &[PermissionMetadataEntry],
) {
    let groups = group_list_text(groups);
    let overrides = permission_entries_text(overrides);
    let metadata = metadata_entries_text(metadata);

    sender.send_message(&TextComponent::plain(format!(
        "{}: groups [{}], direct permissions [{}], metadata [{}]",
        target.name(),
        groups,
        overrides,
        metadata
    )));
}

pub(super) fn send_permission_check(
    sender: &CommandSender,
    target: &PermissionTarget,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
    resolution: Option<&PermissionResolution>,
) {
    let state = resolution.map_or(PermissionState::Deny, PermissionResolution::state);
    let detail = resolution.map_or_else(|| "unset".to_owned(), permission_resolution_text);
    sender.send_message(&TextComponent::plain(format!(
        "{}: permission '{}' in {} is {} ({detail})",
        target.name(),
        permission.as_str(),
        rule_context,
        permission_check_result_text(state)
    )));
}

pub(super) fn send_metadata_check(
    sender: &CommandSender,
    target: &PermissionTarget,
    key: &Identifier,
    rule_context: &PermissionRuleContext,
    resolution: Option<&PermissionMetadataResolution>,
) {
    let value = resolution
        .map(|resolution| permission_metadata_value_text(resolution.value()))
        .unwrap_or_else(|| "unset".to_owned());
    let detail = resolution.map_or_else(|| "unset".to_owned(), metadata_resolution_text);
    sender.send_message(&TextComponent::plain(format!(
        "{}: metadata '{key}' in {rule_context} is {value} ({detail})",
        target.name()
    )));
}

pub(super) fn send_set_permission_summary(
    sender: &CommandSender,
    state: PermissionState,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
    count: usize,
) {
    sender.send_message(&TextComponent::plain(format!(
        "{} permission '{}'{} for {}",
        permission_action_text(state),
        permission.as_str(),
        permission_rule_context_suffix(rule_context),
        target_count_text(count)
    )));
}

pub(super) fn send_unset_permission_summary(
    sender: &CommandSender,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
    count: usize,
) {
    sender.send_message(&TextComponent::plain(format!(
        "Unset direct permission '{}'{} for {}",
        permission.as_str(),
        permission_rule_context_suffix(rule_context),
        target_count_text(count)
    )));
}

pub(super) fn send_set_metadata_summary(
    sender: &CommandSender,
    key: &Identifier,
    value: &PermissionMetadataValue,
    rule_context: &PermissionRuleContext,
    count: usize,
) {
    sender.send_message(&TextComponent::plain(format!(
        "Set metadata '{key}'{} = {} for {}",
        permission_rule_context_suffix(rule_context),
        permission_metadata_value_text(value),
        target_count_text(count)
    )));
}

pub(super) fn send_unset_metadata_summary(
    sender: &CommandSender,
    key: &Identifier,
    rule_context: &PermissionRuleContext,
    count: usize,
) {
    sender.send_message(&TextComponent::plain(format!(
        "Unset metadata '{key}'{} for {}",
        permission_rule_context_suffix(rule_context),
        target_count_text(count)
    )));
}

pub(super) fn send_set_group_permission_summary(
    sender: &CommandSender,
    state: PermissionState,
    group: &str,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
) {
    sender.send_message(&TextComponent::plain(format!(
        "{} permission '{}'{} for group '{group}'",
        permission_action_text(state),
        permission.as_str(),
        permission_rule_context_suffix(rule_context)
    )));
}

pub(super) fn send_set_group_priority_summary(sender: &CommandSender, group: &str, priority: i32) {
    sender.send_message(&TextComponent::plain(format!(
        "Set priority {priority} for group '{group}'"
    )));
}

pub(super) fn send_group_priority_unchanged_summary(sender: &CommandSender, group: &str, priority: i32) {
    sender.send_message(&TextComponent::plain(format!(
        "Group '{group}' already has priority {priority}"
    )));
}

pub(super) fn send_add_default_group_summary(sender: &CommandSender, group: &str) {
    sender.send_message(&TextComponent::plain(format!(
        "Added group '{group}' to default groups"
    )));
}

pub(super) fn send_default_group_already_set_summary(sender: &CommandSender, group: &str) {
    sender.send_message(&TextComponent::plain(format!(
        "Group '{group}' is already a default group"
    )));
}

pub(super) fn send_remove_default_group_summary(sender: &CommandSender, group: &str) {
    sender.send_message(&TextComponent::plain(format!(
        "Removed group '{group}' from default groups"
    )));
}

pub(super) fn send_default_group_not_set_summary(sender: &CommandSender, group: &str) {
    sender.send_message(&TextComponent::plain(format!(
        "Group '{group}' is not a default group"
    )));
}

pub(super) fn send_set_group_metadata_summary(
    sender: &CommandSender,
    group: &str,
    key: &Identifier,
    value: &PermissionMetadataValue,
    rule_context: &PermissionRuleContext,
) {
    sender.send_message(&TextComponent::plain(format!(
        "Set metadata '{key}'{} = {} for group '{group}'",
        permission_rule_context_suffix(rule_context),
        permission_metadata_value_text(value)
    )));
}

pub(super) fn send_group_metadata_unchanged_summary(
    sender: &CommandSender,
    group: &str,
    key: &Identifier,
    value: &PermissionMetadataValue,
    rule_context: &PermissionRuleContext,
) {
    sender.send_message(&TextComponent::plain(format!(
        "Group '{group}' already sets metadata '{key}'{} = {}",
        permission_rule_context_suffix(rule_context),
        permission_metadata_value_text(value)
    )));
}

pub(super) fn send_group_permission_unchanged_summary(
    sender: &CommandSender,
    state: PermissionState,
    group: &str,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
) {
    sender.send_message(&TextComponent::plain(format!(
        "Group '{group}' already {} permission '{}'{}",
        permission_state_text(state),
        permission.as_str(),
        permission_rule_context_suffix(rule_context)
    )));
}

pub(super) fn send_unset_group_metadata_summary(
    sender: &CommandSender,
    group: &str,
    key: &Identifier,
    rule_context: &PermissionRuleContext,
) {
    sender.send_message(&TextComponent::plain(format!(
        "Unset metadata '{key}'{} for group '{group}'",
        permission_rule_context_suffix(rule_context)
    )));
}

pub(super) fn send_group_metadata_not_set_summary(
    sender: &CommandSender,
    group: &str,
    key: &Identifier,
    rule_context: &PermissionRuleContext,
) {
    sender.send_message(&TextComponent::plain(format!(
        "Group '{group}' does not set metadata '{key}'{}",
        permission_rule_context_suffix(rule_context)
    )));
}

pub(super) fn send_unset_group_permission_summary(
    sender: &CommandSender,
    group: &str,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
) {
    sender.send_message(&TextComponent::plain(format!(
        "Unset permission '{}'{} for group '{group}'",
        permission.as_str(),
        permission_rule_context_suffix(rule_context)
    )));
}

pub(super) fn send_group_permission_not_set_summary(
    sender: &CommandSender,
    group: &str,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
) {
    sender.send_message(&TextComponent::plain(format!(
        "Group '{group}' does not set permission '{}'{}",
        permission.as_str(),
        permission_rule_context_suffix(rule_context)
    )));
}

pub(super) fn send_background_error(sender: &CommandSender, command: &str, error: CommandError) {
    sender.send_failure_feedback(error.into_feedback(command));
}

pub(super) fn send_group_update_error(
    sender: &CommandSender,
    error: crate::permission::PermissionGroupUpdateError<PermissionGroupEditError>,
) {
    sender.send_failure(error.to_string());
}

pub(super) fn group_list_text(groups: &[String]) -> String {
    if groups.is_empty() {
        return "none".to_owned();
    }

    groups.join(", ")
}

pub(super) fn permission_key_list_text(permissions: &[String]) -> String {
    if permissions.is_empty() {
        return "none".to_owned();
    }

    permissions.join(", ")
}

pub(super) fn group_metadata_list_text(values: &[PermissionMetadataRuleConfig]) -> String {
    if values.is_empty() {
        return "none".to_owned();
    }

    values
        .iter()
        .map(|value| {
            format!(
                "{} = {}",
                value.key,
                permission_metadata_value_text(&value.value),
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn permission_entries_text(entries: &[PermissionEntry]) -> String {
    if entries.is_empty() {
        return "none".to_owned();
    }

    entries
        .iter()
        .map(|entry| {
            format!(
                "{} {}{}",
                permission_state_text(entry.state()),
                entry.key().as_str(),
                permission_rule_context_suffix(entry.context())
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn metadata_entries_text(entries: &[PermissionMetadataEntry]) -> String {
    if entries.is_empty() {
        return "none".to_owned();
    }

    entries
        .iter()
        .map(|entry| {
            format!(
                "{} = {}{}",
                entry.key(),
                permission_metadata_value_text(entry.value()),
                permission_rule_context_suffix(entry.context())
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn permission_metadata_value_text(value: &PermissionMetadataValue) -> String {
    match value {
        PermissionMetadataValue::Bool(value) => value.to_string(),
        PermissionMetadataValue::Integer(value) => value.to_string(),
        PermissionMetadataValue::String(value) => format!("{value:?}"),
    }
}

pub(super) fn permission_rule_context_suffix(rule_context: &PermissionRuleContext) -> String {
    if rule_context.is_global() {
        String::new()
    } else {
        format!(" ({rule_context})")
    }
}

fn permission_state_text(state: PermissionState) -> &'static str {
    match state {
        PermissionState::Allow => "allow",
        PermissionState::Deny => "deny",
    }
}

pub(super) fn permission_check_result_text(state: PermissionState) -> &'static str {
    match state {
        PermissionState::Allow => "allowed",
        PermissionState::Deny => "denied",
    }
}

fn permission_action_text(state: PermissionState) -> &'static str {
    match state {
        PermissionState::Allow => "Allowed",
        PermissionState::Deny => "Denied",
    }
}

fn permission_resolution_text(resolution: &PermissionResolution) -> String {
    format!(
        "{} by {}, rule {} {}{}, key specificity {}, context specificity {}",
        permission_check_result_text(resolution.state()),
        permission_resolution_source_text(resolution.source()),
        permission_state_text(resolution.state()),
        resolution.key().as_str(),
        permission_rule_context_suffix(resolution.context()),
        resolution.key_specificity(),
        resolution.context_specificity()
    )
}

pub(super) fn permission_resolution_source_text(source: &PermissionResolutionSource) -> String {
    match source {
        PermissionResolutionSource::Subject => "direct permission".to_owned(),
        PermissionResolutionSource::Group { name, priority } => {
            format!("group '{name}' priority {priority}")
        }
    }
}

pub(super) fn metadata_resolution_text(resolution: &PermissionMetadataResolution) -> String {
    format!(
        "set by {}, rule {} = {}{}, context specificity {}, insertion {}",
        permission_resolution_source_text(resolution.source()),
        resolution.key(),
        permission_metadata_value_text(resolution.value()),
        permission_rule_context_suffix(resolution.context()),
        resolution.context_specificity(),
        resolution.insertion_index()
    )
}

pub(super) fn target_count_text(count: usize) -> String {
    match count {
        1 => "1 player".to_owned(),
        _ => format!("{count} players"),
    }
}

pub(super) fn command_result(count: usize) -> CommandResult {
    CommandResult::from_usize_success_count(count)
}
