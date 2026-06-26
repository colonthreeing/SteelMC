//! Steel permission management commands.

use std::{collections::BTreeSet, sync::Arc};

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use text_components::TextComponent;

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandArgumentParser, CommandNodeBuilder, CommandParseError, CommandParseErrorKind,
    CommandResult, ParsedArgument, ParsedArguments, PermissionTarget, argument, literal,
};
use crate::command::parsers::{PermissionGroupParser, PermissionKeyParser, PermissionTargetParser};
use crate::command::reader::{CommandReader, StringMode};
use crate::command::requirement::{CommandInputContext, RequirementContext};
use crate::command::sender::CommandSender;
use crate::command::{CommandRegistration, CommandRegistrationError};
use crate::permission::{
    PermissionEntry, PermissionExpr, PermissionKey, PermissionKeyError, PermissionSegment,
    PermissionSet, PermissionState,
};
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
                    literal("allow")
                        .requires_additional_subcommand_permission()
                        .then(
                            argument("permission", PermissionKeyParser).executes(allow_permission),
                        ),
                )
                .then(
                    literal("deny")
                        .requires_additional_subcommand_permission()
                        .then(
                            argument("permission", PermissionKeyParser).executes(deny_permission),
                        ),
                )
                .then(
                    literal("unset")
                        .requires_additional_subcommand_permission()
                        .then(
                            argument("permission", PermissionOverrideParser::new("targets"))
                                .executes(unset_permission),
                        ),
                )
                .then(
                    literal("group").then(
                        literal("add")
                            .requires_additional_subcommand_permission()
                            .then(argument("group", PermissionGroupParser).executes(add_group)),
                    ),
                )
                .then(
                    literal("group").then(
                        literal("remove")
                            .requires_additional_subcommand_permission()
                            .then(
                                argument("group", PermissionAssignedGroupParser::new("targets"))
                                    .executes(remove_group),
                            ),
                    ),
                ),
        ),
    )
}

#[derive(Clone, Copy, Debug)]
struct PermissionOverrideParser {
    targets_argument: &'static str,
}

impl PermissionOverrideParser {
    const fn new(targets_argument: &'static str) -> Self {
        Self { targets_argument }
    }
}

impl CommandArgumentParser for PermissionOverrideParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        PermissionKeyParser.parse(reader, context)
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        PermissionKeyParser.usage()
    }

    fn parsed_type(&self) -> &'static str {
        PermissionKeyParser.parsed_type()
    }

    fn suggest(
        &self,
        prefix: &str,
        arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let Ok(targets) = arguments.get::<Vec<PermissionTarget>>(self.targets_argument) else {
            return Vec::new();
        };
        let Some(server) = context.server() else {
            return Vec::new();
        };

        let overrides = targets
            .into_iter()
            .filter_map(|target| permission_targets::cached_state(server, &target))
            .map(|state| state.overrides);
        direct_permission_override_suggestions(prefix, overrides, context)
    }
}

#[derive(Clone, Copy, Debug)]
struct PermissionAssignedGroupParser {
    targets_argument: &'static str,
}

impl PermissionAssignedGroupParser {
    const fn new(targets_argument: &'static str) -> Self {
        Self { targets_argument }
    }
}

impl CommandArgumentParser for PermissionAssignedGroupParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let value = reader.read_string(StringMode::SingleWord)?;
        PermissionSegment::parse(value.clone()).map_err(|_| {
            CommandParseError::new(
                CommandParseErrorKind::InvalidPermissionGroup(value.clone()),
                cursor,
            )
        })?;

        Ok(ParsedArgument::String(value))
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        PermissionGroupParser.usage()
    }

    fn parsed_type(&self) -> &'static str {
        PermissionGroupParser.parsed_type()
    }

    fn suggest(
        &self,
        prefix: &str,
        arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let Ok(targets) = arguments.get::<Vec<PermissionTarget>>(self.targets_argument) else {
            return Vec::new();
        };
        let Some(server) = context.server() else {
            return Vec::new();
        };

        let groups = targets
            .into_iter()
            .filter_map(|target| permission_targets::cached_state(server, &target))
            .map(|state| state.groups);
        assigned_group_suggestions(prefix, groups)
    }
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

    Ok(command_result(reported))
}

fn add_group(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let group = group(arguments)?;
    require_group_management(context, &group)?;
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

    Ok(command_result(changed))
}

fn remove_group(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let group = group(arguments)?;
    require_group_management(context, &group)?;
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
    require_permission_management(context, &permission)?;
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

    Ok(command_result(changed))
}

fn unset_permission(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let permission = permission(arguments)?;
    require_permission_management(context, &permission)?;
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

    Ok(command_result(changed))
}

fn spawn_user_info(server: Arc<Server>, sender: CommandSender, targets: Vec<PermissionTarget>) {
    tokio::spawn(async move {
        for target in targets {
            match permission_targets::load_state(&server, target).await {
                Ok(loaded) => send_user_info(&sender, loaded.target(), loaded.state()),
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
        for target in targets {
            let Ok(mut loaded) = load_or_report(&server, &sender, target).await else {
                continue;
            };
            let state = loaded.state_mut();
            if state.groups.iter().any(|assigned| assigned == &group) {
                continue;
            }

            state.groups.push(group.clone());
            if save_or_report(&server, &sender, loaded).await {
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
        for target in targets {
            let Ok(mut loaded) = load_or_report(&server, &sender, target).await else {
                continue;
            };
            let state = loaded.state_mut();
            let old_len = state.groups.len();
            state.groups.retain(|assigned| assigned != &group);
            if state.groups.len() == old_len {
                continue;
            }

            if save_or_report(&server, &sender, loaded).await {
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
        for target in targets {
            let Ok(mut loaded) = load_or_report(&server, &sender, target).await else {
                continue;
            };
            let target_state = loaded.state_mut();
            target_state.overrides.set(permission.clone(), state);

            if save_or_report(&server, &sender, loaded).await {
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
        for target in targets {
            let Ok(mut loaded) = load_or_report(&server, &sender, target).await else {
                continue;
            };
            let target_state = loaded.state_mut();
            if !target_state.overrides.unset(&permission) {
                continue;
            }

            if save_or_report(&server, &sender, loaded).await {
                changed += 1;
            }
        }

        send_unset_permission_summary(&sender, &permission, changed);
    });
}

async fn load_or_report(
    server: &Arc<Server>,
    sender: &CommandSender,
    target: PermissionTarget,
) -> Result<permission_targets::LoadedPermissionTargetState, ()> {
    permission_targets::load_state(server, target)
        .await
        .map_err(|error| {
            send_background_error(sender, "steelperms", error);
        })
}

async fn save_or_report(
    server: &Arc<Server>,
    sender: &CommandSender,
    loaded: permission_targets::LoadedPermissionTargetState,
) -> bool {
    permission_targets::save_state(server, loaded)
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

fn direct_permission_override_suggestions(
    prefix: &str,
    overrides: impl IntoIterator<Item = PermissionSet>,
    context: &dyn RequirementContext,
) -> Vec<SuggestionEntry> {
    let mut permissions = BTreeSet::new();
    for overrides in overrides {
        for entry in overrides.entries() {
            let key = entry.key().as_str();
            if key.starts_with(prefix) && can_manage_permission(context, entry.key()) {
                permissions.insert(key.to_owned());
            }
        }
    }

    permissions.into_iter().map(SuggestionEntry::new).collect()
}

fn require_permission_management(
    context: &dyn RequirementContext,
    permission: &PermissionKey,
) -> Result<(), CommandError> {
    if can_manage_permission(context, permission) {
        Ok(())
    } else {
        Err(CommandError::PermissionDenied)
    }
}

fn can_manage_permission(context: &dyn RequirementContext, permission: &PermissionKey) -> bool {
    let Ok(management_permission) = permission_management_key(permission) else {
        return false;
    };
    context.has_permission(&PermissionExpr::key(management_permission))
}

fn permission_management_key(
    permission: &PermissionKey,
) -> Result<PermissionKey, PermissionKeyError> {
    PermissionKey::parse(format!("steel.permission.manage.{}", permission.as_str()))
}

fn require_group_management(
    context: &dyn RequirementContext,
    group: &str,
) -> Result<(), CommandError> {
    if can_manage_group(context, group) {
        Ok(())
    } else {
        Err(CommandError::PermissionDenied)
    }
}

fn can_manage_group(context: &dyn RequirementContext, group: &str) -> bool {
    let Ok(group) = PermissionSegment::parse(group) else {
        return false;
    };
    let Ok(permission) = PermissionKey::parse(format!("steel.permission.group.{}", group.as_str()))
    else {
        return false;
    };
    context.has_permission(&PermissionExpr::key(permission))
}

fn assigned_group_suggestions(
    prefix: &str,
    groups: impl IntoIterator<Item = Vec<String>>,
) -> Vec<SuggestionEntry> {
    let mut assigned_groups = BTreeSet::new();
    for groups in groups {
        for group in groups {
            if group.starts_with(prefix) {
                assigned_groups.insert(group);
            }
        }
    }

    assigned_groups
        .into_iter()
        .map(SuggestionEntry::new)
        .collect()
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

#[cfg(test)]
mod tests {
    use super::{
        PermissionAssignedGroupParser, assigned_group_suggestions, can_manage_group,
        can_manage_permission, direct_permission_override_suggestions,
    };
    use crate::command::graph::{CommandArgumentParser, CommandParseErrorKind, ParsedArgument};
    use crate::command::reader::CommandReader;
    use crate::command::requirement::{
        CommandInputContext, CommandSourceKind, PermissionExpr, RequirementContext,
    };
    use crate::permission::{PermissionEntry, PermissionKey, PermissionSet};

    struct TestContext {
        permissions: PermissionSet,
    }

    impl TestContext {
        fn empty() -> Self {
            Self {
                permissions: PermissionSet::new(),
            }
        }

        fn with_permissions<const N: usize>(permissions: [&str; N]) -> Self {
            Self {
                permissions: PermissionSet::from_entries(
                    permissions.map(|permission| PermissionEntry::allow(key(permission))),
                ),
            }
        }
    }

    impl RequirementContext for TestContext {
        fn source_kind(&self) -> CommandSourceKind {
            CommandSourceKind::Player
        }

        fn has_permission(&self, permission: &PermissionExpr) -> bool {
            self.permissions.allows(permission)
        }
    }

    impl CommandInputContext for TestContext {}

    fn key(value: &str) -> PermissionKey {
        PermissionKey::parse(value).expect("permission key parses")
    }

    fn suggestion_texts(
        suggestions: Vec<steel_protocol::packets::game::SuggestionEntry>,
    ) -> Vec<String> {
        suggestions
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect()
    }

    #[test]
    fn unset_permission_suggestions_only_include_direct_overrides() {
        let first = PermissionSet::from_entries([
            PermissionEntry::allow(key("minecraft.command.gamemode.creative")),
            PermissionEntry::deny(key("steel.command.steelperms.user.allow")),
        ]);
        let second = PermissionSet::from_entries([
            PermissionEntry::allow(key("minecraft.command.gamemode.survival")),
            PermissionEntry::deny(key("steel.command.steelperms.user.allow")),
        ]);

        assert_eq!(
            suggestion_texts(direct_permission_override_suggestions(
                "minecraft.command.gamemode.",
                [first, second],
                &TestContext::with_permissions([
                    "steel.permission.manage.minecraft.command.gamemode.*",
                ]),
            )),
            vec![
                "minecraft.command.gamemode.creative",
                "minecraft.command.gamemode.survival",
            ]
        );
    }

    #[test]
    fn remove_group_parser_accepts_unknown_group_names() {
        let mut reader = CommandReader::new("legacy");
        let parsed = PermissionAssignedGroupParser::new("targets")
            .parse(&mut reader, &TestContext::empty())
            .expect("group parses");

        assert!(matches!(parsed, ParsedArgument::String(group) if group == "legacy"));
    }

    #[test]
    fn remove_group_parser_rejects_invalid_group_names() {
        let mut reader = CommandReader::new("Legacy");
        let error = PermissionAssignedGroupParser::new("targets")
            .parse(&mut reader, &TestContext::empty())
            .expect_err("uppercase group should be invalid");

        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::InvalidPermissionGroup(group) if group == "Legacy"
        ));
    }

    #[test]
    fn remove_group_suggestions_only_include_assigned_groups() {
        assert_eq!(
            suggestion_texts(assigned_group_suggestions(
                "v",
                [
                    vec!["vip".to_owned(), "legacy".to_owned()],
                    vec!["vip".to_owned(), "veteran".to_owned()],
                ],
            )),
            vec!["veteran".to_owned(), "vip".to_owned()]
        );
    }

    #[test]
    fn manage_permission_requires_targeted_management_permission() {
        let context = TestContext::with_permissions([
            "steel.permission.manage.minecraft.command.gamemode.creative",
        ]);

        assert!(can_manage_permission(
            &context,
            &key("minecraft.command.gamemode.creative")
        ));
        assert!(!can_manage_permission(
            &context,
            &key("minecraft.command.gamemode.survival")
        ));
    }

    #[test]
    fn manage_group_requires_targeted_group_permission() {
        let context = TestContext::with_permissions(["steel.permission.group.op"]);

        assert!(can_manage_group(&context, "op"));
        assert!(!can_manage_group(&context, "admin"));
    }
}
