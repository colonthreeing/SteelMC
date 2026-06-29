//! Steel permission management commands.

use std::sync::Arc;

use text_components::TextComponent;

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    BoolParser, CommandArgumentParser, CommandNodeBuilder, CommandResult, IntegerParser,
    LongParser, ParsedArguments, PermissionTarget, StringParser, argument, literal,
};
use crate::command::parsers::{
    DomainParser, PermissionGroupParser, PermissionRuleExpressionParser, PermissionTargetParser,
    WorldParser,
};
use crate::command::reader::StringMode;
use crate::command::sender::CommandSender;
use crate::command::CommandRegistrationSpec;
use crate::permission::{
    PermissionContext, PermissionKey, PermissionRuleContext, PermissionState, PermissionValue,
};
use crate::server::Server;
use steel_utils::Identifier;

use super::permission_targets;
#[path = "steelperms/access.rs"]
mod access;
#[path = "steelperms/arguments.rs"]
mod arguments;
#[path = "steelperms/config.rs"]
mod config;
#[path = "steelperms/messages.rs"]
mod messages;
#[path = "steelperms/parsers.rs"]
mod parsers;

use self::access::{
    require_group_management, require_metadata_management, require_permission_management,
};
#[cfg(test)]
use self::access::{
    assigned_group_suggestions, can_manage_group, can_manage_metadata, can_manage_permission,
    direct_metadata_override_suggestions, direct_permission_override_suggestions,
    group_metadata_suggestions, group_permission_suggestions, metadata_catalog_suggestions,
    metadata_management_key,
};
use self::arguments::{
    group, group_priority, metadata_key, metadata_value, permission, permission_context,
    permission_rule_context, targets,
};
use self::messages::{
    command_result, group_list_text, group_metadata_list_text, group_rule_list_text,
    permission_key_list_text, send_add_default_group_summary, send_background_error,
    send_default_group_already_set_summary, send_default_group_not_set_summary,
    send_group_metadata_not_set_summary, send_group_metadata_unchanged_summary,
    send_group_permission_not_set_summary, send_group_permission_unchanged_summary,
    send_group_priority_unchanged_summary, send_group_update_error, send_metadata_check,
    send_permission_check, send_remove_default_group_summary, send_set_group_metadata_summary,
    send_set_group_permission_summary, send_set_group_priority_summary, send_set_metadata_summary,
    send_set_permission_summary, send_unset_group_metadata_summary,
    send_unset_group_permission_summary, send_unset_metadata_summary, send_unset_permission_summary,
    send_user_info, target_count_text,
};
#[cfg(test)]
use self::messages::{
    metadata_resolution_text, permission_check_result_text, permission_resolution_source_text,
    permission_rule_context_suffix,
};
use self::config::{
    add_default_group_config, create_group_config, delete_group_config, remove_default_group_config,
    set_group_config_metadata, set_group_config_permission, set_group_config_priority,
    unset_group_config_metadata, unset_group_config_permission,
};
#[cfg(test)]
use self::config::{
    PermissionGroupEditError, group_config_metadata_value, group_config_permission_states,
};
use self::parsers::{
    PermissionAssignedGroupParser, PermissionContextKeyParser, PermissionContextValueParser,
    PermissionGroupMetadataParser, PermissionGroupNameParser, PermissionGroupRuleParser,
    PermissionMetadataKeyParser, PermissionMetadataOverrideParser, PermissionOverrideParser,
};

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::steel().aliases(&["sp"]);

/// Handler for the "steelperms" command group.
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
    literal("steelperms").then_all([user_command(), group_command(), groups_command()])
}

fn user_command() -> CommandNodeBuilder {
    literal("user").then(
        argument("targets", PermissionTargetParser).then_all([
            literal("info")
                .requires_subcommand_permission()
                .executes(user_info),
            literal("allow")
                .requires_additional_subcommand_permission()
                .then(permission_key_argument(allow_permission)),
            literal("deny")
                .requires_additional_subcommand_permission()
                .then(permission_key_argument(deny_permission)),
            literal("unset")
                .requires_additional_subcommand_permission()
                .then(permission_override_argument(unset_permission)),
            literal("check")
                .requires_subcommand_permission()
                .then(permission_key_argument(check_permission)),
            user_metadata_arguments(),
            contextual_user_permission_arguments(),
            literal("group").then_all([
                literal("add")
                    .requires_additional_subcommand_permission()
                    .then(argument("group", PermissionGroupParser).executes(add_group)),
                literal("remove")
                    .requires_additional_subcommand_permission()
                    .then(
                        argument("group", PermissionAssignedGroupParser::new("targets"))
                            .executes(remove_group),
                    ),
            ]),
        ]),
    )
}

fn group_command() -> CommandNodeBuilder {
    literal("group").then(
        argument("group", PermissionGroupNameParser).then_all([
            literal("create")
                .requires_additional_subcommand_permission()
                .executes(create_group),
            literal("info")
                .requires_subcommand_permission()
                .executes(group_info),
            literal("delete")
                .requires_additional_subcommand_permission()
                .executes(delete_group),
            literal("allow")
                .requires_additional_subcommand_permission()
                .then(permission_key_argument(allow_group_permission)),
            literal("deny")
                .requires_additional_subcommand_permission()
                .then(permission_key_argument(deny_group_permission)),
            literal("unset")
                .requires_additional_subcommand_permission()
                .then(group_permission_argument(unset_group_permission)),
            literal("priority")
                .requires_additional_subcommand_permission()
                .then(argument("priority", IntegerParser::new()).executes(set_group_priority)),
            group_metadata_arguments(),
            contextual_group_permission_arguments(),
        ]),
    )
}

fn groups_command() -> CommandNodeBuilder {
    literal("groups").then_all([
        literal("list")
            .requires_subcommand_permission()
            .executes(group_list),
        literal("default").then_all([
            literal("add")
                .requires_additional_subcommand_permission()
                .then(argument("group", PermissionGroupParser).executes(add_default_group)),
            literal("remove")
                .requires_additional_subcommand_permission()
                .then(argument("group", PermissionGroupParser).executes(remove_default_group)),
        ]),
    ])
}

fn permission_key_argument(
    executor: fn(&mut CommandContext, &ParsedArguments) -> Result<CommandResult, CommandError>,
) -> CommandNodeBuilder {
    argument("permission", PermissionRuleExpressionParser).executes(executor)
}

fn permission_override_argument(
    executor: fn(&mut CommandContext, &ParsedArguments) -> Result<CommandResult, CommandError>,
) -> CommandNodeBuilder {
    argument("permission", PermissionOverrideParser::new("targets")).executes(executor)
}

fn group_permission_argument(
    executor: fn(&mut CommandContext, &ParsedArguments) -> Result<CommandResult, CommandError>,
) -> CommandNodeBuilder {
    argument("permission", PermissionGroupRuleParser::new("group")).executes(executor)
}

fn metadata_override_argument(
    executor: fn(&mut CommandContext, &ParsedArguments) -> Result<CommandResult, CommandError>,
) -> CommandNodeBuilder {
    argument(
        "metadata_key",
        PermissionMetadataOverrideParser::new("targets"),
    )
    .executes(executor)
}

fn group_metadata_argument(
    executor: fn(&mut CommandContext, &ParsedArguments) -> Result<CommandResult, CommandError>,
) -> CommandNodeBuilder {
    argument("metadata_key", PermissionGroupMetadataParser::new("group")).executes(executor)
}

fn user_metadata_arguments() -> CommandNodeBuilder {
    literal("metadata").then_all([
        metadata_set_arguments(set_metadata),
        literal("check").requires_subcommand_permission().then(
            argument("metadata_key", PermissionMetadataKeyParser).executes(check_metadata),
        ),
        literal("unset")
            .requires_additional_subcommand_permission()
            .then(metadata_override_argument(unset_metadata)),
    ])
}

fn group_metadata_arguments() -> CommandNodeBuilder {
    literal("metadata").then_all([
        metadata_set_arguments(set_group_metadata),
        literal("unset")
            .requires_additional_subcommand_permission()
            .then(group_metadata_argument(unset_group_metadata)),
    ])
}

fn metadata_set_arguments(
    executor: fn(&mut CommandContext, &ParsedArguments) -> Result<CommandResult, CommandError>,
) -> CommandNodeBuilder {
    literal("set")
        .requires_additional_subcommand_permission()
        .then_all([
            literal("int").then(
                argument("metadata_key", PermissionMetadataKeyParser)
                    .then(argument("metadata_int_value", LongParser::new()).executes(executor)),
            ),
            literal("bool").then(
                argument("metadata_key", PermissionMetadataKeyParser)
                    .then(argument("metadata_bool_value", BoolParser).executes(executor)),
            ),
            literal("string").then(
                argument("metadata_key", PermissionMetadataKeyParser).then(
                    argument(
                        "metadata_string_value",
                        StringParser::new(StringMode::QuotablePhrase),
                    )
                    .executes(executor),
                ),
            ),
        ])
}

fn contextual_user_permission_arguments() -> CommandNodeBuilder {
    literal("context").then_all([
        literal("domain").then(user_permission_context_argument(
            "context_domain",
            DomainParser,
        )),
        literal("world").then(user_permission_context_argument(
            "context_world",
            WorldParser,
        )),
        user_custom_context_argument(),
    ])
}

fn user_permission_context_argument(
    name: &'static str,
    parser: impl CommandArgumentParser + Clone + 'static,
) -> CommandNodeBuilder {
    user_context_actions(argument(name, parser)).then(user_custom_context_argument())
}

fn user_custom_context_argument() -> CommandNodeBuilder {
    literal("custom").then(
        argument("context_custom_key", PermissionContextKeyParser).then(user_context_actions(
            argument(
                "context_custom_value",
                PermissionContextValueParser::new("context_custom_key"),
            ),
        )),
    )
}

fn user_context_actions(node: CommandNodeBuilder) -> CommandNodeBuilder {
    node.then_all([
        literal("allow")
            .requires_additional_subcommand_permission()
            .then(permission_key_argument(allow_permission)),
        literal("deny")
            .requires_additional_subcommand_permission()
            .then(permission_key_argument(deny_permission)),
        literal("unset")
            .requires_additional_subcommand_permission()
            .then(permission_override_argument(unset_permission)),
        literal("check")
            .requires_subcommand_permission()
            .then(permission_key_argument(check_permission)),
        user_metadata_arguments(),
    ])
}

fn contextual_group_permission_arguments() -> CommandNodeBuilder {
    literal("context").then_all([
        literal("domain").then(group_permission_context_argument(
            "context_domain",
            DomainParser,
        )),
        literal("world").then(group_permission_context_argument(
            "context_world",
            WorldParser,
        )),
        group_custom_context_argument(),
    ])
}

fn group_permission_context_argument(
    name: &'static str,
    parser: impl CommandArgumentParser + Clone + 'static,
) -> CommandNodeBuilder {
    group_context_actions(argument(name, parser)).then(group_custom_context_argument())
}

fn group_custom_context_argument() -> CommandNodeBuilder {
    literal("custom").then(
        argument("context_custom_key", PermissionContextKeyParser).then(group_context_actions(
            argument(
                "context_custom_value",
                PermissionContextValueParser::new("context_custom_key"),
            ),
        )),
    )
}

fn group_context_actions(node: CommandNodeBuilder) -> CommandNodeBuilder {
    node.then_all([
        literal("allow")
            .requires_additional_subcommand_permission()
            .then(permission_key_argument(allow_group_permission)),
        literal("deny")
            .requires_additional_subcommand_permission()
            .then(permission_key_argument(deny_group_permission)),
        literal("unset")
            .requires_additional_subcommand_permission()
            .then(group_permission_argument(unset_group_permission)),
        group_metadata_arguments(),
    ])
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

fn check_permission(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let permission = permission(arguments)?;
    let rule_context = permission_rule_context(arguments)?;
    let check_context = permission_context(arguments)?;
    require_permission_management(context, &permission)?;
    let mut reported = 0;
    let mut offline_targets = Vec::new();

    for target in targets {
        if let Some((_, state)) = permission_targets::online_state(&context.server, &target) {
            let effective = context
                .server
                .permission_groups
                .effective_permissions(&state.groups, &state.overrides);
            let resolution = effective.resolve_key_in_detailed(&permission, &check_context);
            send_permission_check(
                &context.sender,
                &target,
                &permission,
                &rule_context,
                resolution.as_ref(),
            );
            reported += 1;
        } else {
            offline_targets.push(target);
        }
    }

    let scheduled = offline_targets.len();
    if scheduled != 0 {
        spawn_check_permission(
            Arc::clone(&context.server),
            context.sender.clone(),
            offline_targets,
            permission,
            rule_context,
            check_context,
        );
    }

    Ok(command_result(reported))
}

fn group_list(
    context: &mut CommandContext,
    _arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let config = context.server.permission_groups.config_snapshot();
    let groups = context.server.permission_groups.group_names();
    context.sender.send_message(&TextComponent::plain(format!(
        "Permission groups: defaults [{}], groups [{}]",
        group_list_text(&config.default_groups),
        groups.join(", ")
    )));

    Ok(command_result(groups.len()))
}

fn add_default_group(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let group = group(arguments)?;
    require_group_management(context, &group)?;
    spawn_add_default_group(Arc::clone(&context.server), context.sender.clone(), group);

    Ok(CommandResult::success())
}

fn remove_default_group(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let group = group(arguments)?;
    require_group_management(context, &group)?;
    spawn_remove_default_group(Arc::clone(&context.server), context.sender.clone(), group);

    Ok(CommandResult::success())
}

fn group_info(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let group = group(arguments)?;
    require_group_management(context, &group)?;
    let config = context.server.permission_groups.config_snapshot();
    let Some(group_config) = config.groups.get(&group) else {
        return Err(CommandError::failure(format!(
            "Unknown permission group '{group}'"
        )));
    };

    context.sender.send_message(&TextComponent::plain(format!(
        "Group '{group}': priority {}, allow [{}], deny [{}], contextual [{}], metadata [{}]",
        group_config.priority,
        permission_key_list_text(&group_config.allow),
        permission_key_list_text(&group_config.deny),
        group_rule_list_text(&group_config.rules),
        group_metadata_list_text(&group_config.values)
    )));

    Ok(CommandResult::success())
}

fn create_group(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let group = group(arguments)?;
    require_group_management(context, &group)?;
    spawn_create_group(Arc::clone(&context.server), context.sender.clone(), group);

    Ok(CommandResult::success())
}

fn delete_group(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let group = group(arguments)?;
    require_group_management(context, &group)?;
    spawn_delete_group(Arc::clone(&context.server), context.sender.clone(), group);

    Ok(CommandResult::success())
}

fn allow_group_permission(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    set_group_permission(context, arguments, PermissionState::Allow)
}

fn deny_group_permission(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    set_group_permission(context, arguments, PermissionState::Deny)
}

fn set_group_permission(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    state: PermissionState,
) -> Result<CommandResult, CommandError> {
    let group = group(arguments)?;
    let permission = permission(arguments)?;
    let rule_context = permission_rule_context(arguments)?;
    require_group_management(context, &group)?;
    require_permission_management(context, &permission)?;
    spawn_set_group_permission(
        Arc::clone(&context.server),
        context.sender.clone(),
        group,
        permission,
        rule_context,
        state,
    );

    Ok(CommandResult::success())
}

fn unset_group_permission(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let group = group(arguments)?;
    let permission = permission(arguments)?;
    let rule_context = permission_rule_context(arguments)?;
    require_group_management(context, &group)?;
    require_permission_management(context, &permission)?;
    spawn_unset_group_permission(
        Arc::clone(&context.server),
        context.sender.clone(),
        group,
        permission,
        rule_context,
    );

    Ok(CommandResult::success())
}

fn set_group_priority(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let group = group(arguments)?;
    let priority = group_priority(arguments)?;
    require_group_management(context, &group)?;
    spawn_set_group_priority(
        Arc::clone(&context.server),
        context.sender.clone(),
        group,
        priority,
    );

    Ok(CommandResult::success())
}

fn set_group_metadata(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let group = group(arguments)?;
    let key = metadata_key(arguments)?;
    let value = metadata_value(arguments)?;
    let rule_context = permission_rule_context(arguments)?;
    require_group_management(context, &group)?;
    require_metadata_management(context, &key)?;
    spawn_set_group_metadata(
        Arc::clone(&context.server),
        context.sender.clone(),
        group,
        key,
        value,
        rule_context,
    );

    Ok(CommandResult::success())
}

fn unset_group_metadata(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let group = group(arguments)?;
    let key = metadata_key(arguments)?;
    let rule_context = permission_rule_context(arguments)?;
    require_group_management(context, &group)?;
    require_metadata_management(context, &key)?;
    spawn_unset_group_metadata(
        Arc::clone(&context.server),
        context.sender.clone(),
        group,
        key,
        rule_context,
    );

    Ok(CommandResult::success())
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
    let rule_context = permission_rule_context(arguments)?;
    require_permission_management(context, &permission)?;
    let mut changed = 0;
    let mut offline_targets = Vec::new();

    for target in targets {
        if let Some((player, mut target_state)) =
            permission_targets::online_state(&context.server, &target)
        {
            target_state
                .overrides
                .set_in(permission.clone(), rule_context.clone(), state);
            permission_targets::save_online_state(&context.server, &player, target_state)?;
            changed += 1;
        } else {
            offline_targets.push(target);
        }
    }

    let scheduled = offline_targets.len();
    if changed != 0 || scheduled == 0 {
        send_set_permission_summary(&context.sender, state, &permission, &rule_context, changed);
    }
    if scheduled != 0 {
        spawn_set_permission(
            Arc::clone(&context.server),
            context.sender.clone(),
            offline_targets,
            permission,
            rule_context,
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
    let rule_context = permission_rule_context(arguments)?;
    require_permission_management(context, &permission)?;
    let mut changed = 0;
    let mut offline_targets = Vec::new();

    for target in targets {
        if let Some((player, mut target_state)) =
            permission_targets::online_state(&context.server, &target)
        {
            if !target_state.overrides.unset_in(&permission, &rule_context) {
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
        send_unset_permission_summary(&context.sender, &permission, &rule_context, changed);
    }
    if scheduled != 0 {
        spawn_unset_permission(
            Arc::clone(&context.server),
            context.sender.clone(),
            offline_targets,
            permission,
            rule_context,
        );
    }

    Ok(command_result(changed))
}

fn set_metadata(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let key = metadata_key(arguments)?;
    let value = metadata_value(arguments)?;
    let rule_context = permission_rule_context(arguments)?;
    require_metadata_management(context, &key)?;
    let mut changed = 0;
    let mut offline_targets = Vec::new();

    for target in targets {
        if let Some((player, mut target_state)) =
            permission_targets::online_state(&context.server, &target)
        {
            target_state
                .value_overrides
                .set_in(key.clone(), rule_context.clone(), value.clone());
            permission_targets::save_online_state(&context.server, &player, target_state)?;
            changed += 1;
        } else {
            offline_targets.push(target);
        }
    }

    let scheduled = offline_targets.len();
    if changed != 0 || scheduled == 0 {
        send_set_metadata_summary(&context.sender, &key, &value, &rule_context, changed);
    }
    if scheduled != 0 {
        spawn_set_metadata(
            Arc::clone(&context.server),
            context.sender.clone(),
            offline_targets,
            key,
            value,
            rule_context,
        );
    }

    Ok(command_result(changed))
}

fn unset_metadata(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let key = metadata_key(arguments)?;
    let rule_context = permission_rule_context(arguments)?;
    require_metadata_management(context, &key)?;
    let mut changed = 0;
    let mut offline_targets = Vec::new();

    for target in targets {
        if let Some((player, mut target_state)) =
            permission_targets::online_state(&context.server, &target)
        {
            if !target_state.value_overrides.unset_in(&key, &rule_context) {
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
        send_unset_metadata_summary(&context.sender, &key, &rule_context, changed);
    }
    if scheduled != 0 {
        spawn_unset_metadata(
            Arc::clone(&context.server),
            context.sender.clone(),
            offline_targets,
            key,
            rule_context,
        );
    }

    Ok(command_result(changed))
}

fn check_metadata(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let key = metadata_key(arguments)?;
    let rule_context = permission_rule_context(arguments)?;
    let check_context = permission_context(arguments)?;
    require_metadata_management(context, &key)?;
    let mut reported = 0;
    let mut offline_targets = Vec::new();

    for target in targets {
        if let Some((_, state)) = permission_targets::online_state(&context.server, &target) {
            let effective = context
                .server
                .permission_groups
                .effective_values(&state.groups, &state.value_overrides);
            let resolution = effective.resolve_in_detailed(&key, &check_context);
            send_metadata_check(
                &context.sender,
                &target,
                &key,
                &rule_context,
                resolution.as_ref(),
            );
            reported += 1;
        } else {
            offline_targets.push(target);
        }
    }

    let scheduled = offline_targets.len();
    if scheduled != 0 {
        spawn_check_metadata(
            Arc::clone(&context.server),
            context.sender.clone(),
            offline_targets,
            key,
            rule_context,
            check_context,
        );
    }

    Ok(command_result(reported))
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

fn spawn_check_permission(
    server: Arc<Server>,
    sender: CommandSender,
    targets: Vec<PermissionTarget>,
    permission: PermissionKey,
    rule_context: PermissionRuleContext,
    check_context: PermissionContext,
) {
    tokio::spawn(async move {
        for target in targets {
            let Ok(loaded) = load_or_report(&server, &sender, target).await else {
                continue;
            };
            let state = loaded.state();
            let effective = server
                .permission_groups
                .effective_permissions(&state.groups, &state.overrides);
            let resolution = effective.resolve_key_in_detailed(&permission, &check_context);
            send_permission_check(
                &sender,
                loaded.target(),
                &permission,
                &rule_context,
                resolution.as_ref(),
            );
        }
    });
}

fn spawn_check_metadata(
    server: Arc<Server>,
    sender: CommandSender,
    targets: Vec<PermissionTarget>,
    key: Identifier,
    rule_context: PermissionRuleContext,
    check_context: PermissionContext,
) {
    tokio::spawn(async move {
        for target in targets {
            let Ok(loaded) = load_or_report(&server, &sender, target).await else {
                continue;
            };
            let state = loaded.state();
            let effective = server
                .permission_groups
                .effective_values(&state.groups, &state.value_overrides);
            let resolution = effective.resolve_in_detailed(&key, &check_context);
            send_metadata_check(
                &sender,
                loaded.target(),
                &key,
                &rule_context,
                resolution.as_ref(),
            );
        }
    });
}

fn spawn_create_group(server: Arc<Server>, sender: CommandSender, group: String) {
    tokio::spawn(async move {
        let group_for_update = group.clone();
        match server
            .try_update_permission_groups(move |config| {
                create_group_config(config, &group_for_update)
            })
            .await
        {
            Ok(()) => sender.send_message(&TextComponent::plain(format!(
                "Created permission group '{group}'"
            ))),
            Err(error) => send_group_update_error(&sender, error),
        }
    });
}

fn spawn_delete_group(server: Arc<Server>, sender: CommandSender, group: String) {
    tokio::spawn(async move {
        let group_for_update = group.clone();
        match server
            .try_update_permission_groups(move |config| {
                delete_group_config(config, &group_for_update)
            })
            .await
        {
            Ok(()) => sender.send_message(&TextComponent::plain(format!(
                "Deleted permission group '{group}'"
            ))),
            Err(error) => send_group_update_error(&sender, error),
        }
    });
}

fn spawn_add_default_group(server: Arc<Server>, sender: CommandSender, group: String) {
    tokio::spawn(async move {
        let group_for_update = group.clone();
        match server
            .try_update_permission_groups(move |config| {
                add_default_group_config(config, &group_for_update)
            })
            .await
        {
            Ok(true) => send_add_default_group_summary(&sender, &group),
            Ok(false) => send_default_group_already_set_summary(&sender, &group),
            Err(error) => send_group_update_error(&sender, error),
        }
    });
}

fn spawn_remove_default_group(server: Arc<Server>, sender: CommandSender, group: String) {
    tokio::spawn(async move {
        let group_for_update = group.clone();
        match server
            .try_update_permission_groups(move |config| {
                remove_default_group_config(config, &group_for_update)
            })
            .await
        {
            Ok(true) => send_remove_default_group_summary(&sender, &group),
            Ok(false) => send_default_group_not_set_summary(&sender, &group),
            Err(error) => send_group_update_error(&sender, error),
        }
    });
}

fn spawn_set_group_permission(
    server: Arc<Server>,
    sender: CommandSender,
    group: String,
    permission: PermissionKey,
    rule_context: PermissionRuleContext,
    state: PermissionState,
) {
    tokio::spawn(async move {
        let group_for_update = group.clone();
        let permission_for_update = permission.clone();
        let context_for_update = rule_context.clone();
        match server
            .try_update_permission_groups(move |config| {
                set_group_config_permission(
                    config,
                    &group_for_update,
                    &permission_for_update,
                    &context_for_update,
                    state,
                )
            })
            .await
        {
            Ok(true) => send_set_group_permission_summary(
                &sender,
                state,
                &group,
                &permission,
                &rule_context,
            ),
            Ok(false) => send_group_permission_unchanged_summary(
                &sender,
                state,
                &group,
                &permission,
                &rule_context,
            ),
            Err(error) => send_group_update_error(&sender, error),
        }
    });
}

fn spawn_unset_group_permission(
    server: Arc<Server>,
    sender: CommandSender,
    group: String,
    permission: PermissionKey,
    rule_context: PermissionRuleContext,
) {
    tokio::spawn(async move {
        let group_for_update = group.clone();
        let permission_for_update = permission.clone();
        let context_for_update = rule_context.clone();
        match server
            .try_update_permission_groups(move |config| {
                unset_group_config_permission(
                    config,
                    &group_for_update,
                    &permission_for_update,
                    &context_for_update,
                )
            })
            .await
        {
            Ok(true) => {
                send_unset_group_permission_summary(&sender, &group, &permission, &rule_context)
            }
            Ok(false) => {
                send_group_permission_not_set_summary(&sender, &group, &permission, &rule_context)
            }
            Err(error) => send_group_update_error(&sender, error),
        }
    });
}

fn spawn_set_group_priority(
    server: Arc<Server>,
    sender: CommandSender,
    group: String,
    priority: i32,
) {
    tokio::spawn(async move {
        let group_for_update = group.clone();
        match server
            .try_update_permission_groups(move |config| {
                set_group_config_priority(config, &group_for_update, priority)
            })
            .await
        {
            Ok(true) => send_set_group_priority_summary(&sender, &group, priority),
            Ok(false) => send_group_priority_unchanged_summary(&sender, &group, priority),
            Err(error) => send_group_update_error(&sender, error),
        }
    });
}

fn spawn_set_group_metadata(
    server: Arc<Server>,
    sender: CommandSender,
    group: String,
    key: Identifier,
    value: PermissionValue,
    rule_context: PermissionRuleContext,
) {
    tokio::spawn(async move {
        let group_for_update = group.clone();
        let key_for_update = key.clone();
        let value_for_update = value.clone();
        let context_for_update = rule_context.clone();
        match server
            .try_update_permission_groups(move |config| {
                set_group_config_metadata(
                    config,
                    &group_for_update,
                    &key_for_update,
                    &value_for_update,
                    &context_for_update,
                )
            })
            .await
        {
            Ok(true) => {
                send_set_group_metadata_summary(&sender, &group, &key, &value, &rule_context)
            }
            Ok(false) => {
                send_group_metadata_unchanged_summary(&sender, &group, &key, &value, &rule_context)
            }
            Err(error) => send_group_update_error(&sender, error),
        }
    });
}

fn spawn_unset_group_metadata(
    server: Arc<Server>,
    sender: CommandSender,
    group: String,
    key: Identifier,
    rule_context: PermissionRuleContext,
) {
    tokio::spawn(async move {
        let group_for_update = group.clone();
        let key_for_update = key.clone();
        let context_for_update = rule_context.clone();
        match server
            .try_update_permission_groups(move |config| {
                unset_group_config_metadata(
                    config,
                    &group_for_update,
                    &key_for_update,
                    &context_for_update,
                )
            })
            .await
        {
            Ok(true) => send_unset_group_metadata_summary(&sender, &group, &key, &rule_context),
            Ok(false) => send_group_metadata_not_set_summary(&sender, &group, &key, &rule_context),
            Err(error) => send_group_update_error(&sender, error),
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
    rule_context: PermissionRuleContext,
    state: PermissionState,
) {
    tokio::spawn(async move {
        let mut changed = 0;
        for target in targets {
            let Ok(mut loaded) = load_or_report(&server, &sender, target).await else {
                continue;
            };
            let target_state = loaded.state_mut();
            target_state
                .overrides
                .set_in(permission.clone(), rule_context.clone(), state);

            if save_or_report(&server, &sender, loaded).await {
                changed += 1;
            }
        }

        send_set_permission_summary(&sender, state, &permission, &rule_context, changed);
    });
}

fn spawn_unset_permission(
    server: Arc<Server>,
    sender: CommandSender,
    targets: Vec<PermissionTarget>,
    permission: PermissionKey,
    rule_context: PermissionRuleContext,
) {
    tokio::spawn(async move {
        let mut changed = 0;
        for target in targets {
            let Ok(mut loaded) = load_or_report(&server, &sender, target).await else {
                continue;
            };
            let target_state = loaded.state_mut();
            if !target_state.overrides.unset_in(&permission, &rule_context) {
                continue;
            }

            if save_or_report(&server, &sender, loaded).await {
                changed += 1;
            }
        }

        send_unset_permission_summary(&sender, &permission, &rule_context, changed);
    });
}

fn spawn_set_metadata(
    server: Arc<Server>,
    sender: CommandSender,
    targets: Vec<PermissionTarget>,
    key: Identifier,
    value: PermissionValue,
    rule_context: PermissionRuleContext,
) {
    tokio::spawn(async move {
        let mut changed = 0;
        for target in targets {
            let Ok(mut loaded) = load_or_report(&server, &sender, target).await else {
                continue;
            };
            let target_state = loaded.state_mut();
            target_state
                .value_overrides
                .set_in(key.clone(), rule_context.clone(), value.clone());

            if save_or_report(&server, &sender, loaded).await {
                changed += 1;
            }
        }

        send_set_metadata_summary(&sender, &key, &value, &rule_context, changed);
    });
}

fn spawn_unset_metadata(
    server: Arc<Server>,
    sender: CommandSender,
    targets: Vec<PermissionTarget>,
    key: Identifier,
    rule_context: PermissionRuleContext,
) {
    tokio::spawn(async move {
        let mut changed = 0;
        for target in targets {
            let Ok(mut loaded) = load_or_report(&server, &sender, target).await else {
                continue;
            };
            let target_state = loaded.state_mut();
            if !target_state.value_overrides.unset_in(&key, &rule_context) {
                continue;
            }

            if save_or_report(&server, &sender, loaded).await {
                changed += 1;
            }
        }

        send_unset_metadata_summary(&sender, &key, &rule_context, changed);
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

#[cfg(test)]
#[path = "steelperms/tests.rs"]
mod tests;
