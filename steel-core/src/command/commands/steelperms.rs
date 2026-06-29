//! Steel permission management commands.

use std::{collections::BTreeSet, sync::Arc};

use steel_protocol::packets::game::SuggestionEntry;
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
use crate::command::requirement::RequirementContext;
use crate::command::sender::CommandSender;
use crate::command::CommandRegistrationSpec;
use crate::permission::{
    PermissionContext, PermissionContextKey, PermissionExpr, PermissionGroupConfig, PermissionKey,
    PermissionKeyError, PermissionMetadataCatalog, PermissionRuleConfig, PermissionRuleContext,
    PermissionRuleExpression, PermissionSegment, PermissionSet, PermissionState, PermissionValue,
    PermissionValueSet, parse_permission_value_key,
};
use crate::server::Server;
use crate::world::World;
use steel_utils::Identifier;

use super::permission_targets;
#[path = "steelperms/config.rs"]
mod config;
#[path = "steelperms/messages.rs"]
mod messages;
#[path = "steelperms/parsers.rs"]
mod parsers;

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

fn group_priority(arguments: &ParsedArguments) -> Result<i32, CommandError> {
    arguments
        .get::<i32>("priority")
        .map_err(super::invalid_parsed_argument)
}

fn permission(arguments: &ParsedArguments) -> Result<PermissionKey, CommandError> {
    if let Ok(expression) = arguments.get::<PermissionRuleExpression>("permission") {
        return Ok(expression.key().clone());
    }

    arguments
        .get::<PermissionKey>("permission")
        .map_err(super::invalid_parsed_argument)
}

fn metadata_key(arguments: &ParsedArguments) -> Result<Identifier, CommandError> {
    arguments
        .get::<Identifier>("metadata_key")
        .map_err(super::invalid_parsed_argument)
}

fn metadata_value(arguments: &ParsedArguments) -> Result<PermissionValue, CommandError> {
    if let Ok(value) = arguments.get::<i64>("metadata_int_value") {
        return Ok(PermissionValue::Integer(value));
    }
    if let Ok(value) = arguments.get::<bool>("metadata_bool_value") {
        return Ok(PermissionValue::Bool(value));
    }
    if let Ok(value) = arguments.get::<String>("metadata_string_value") {
        return Ok(PermissionValue::String(value));
    }

    Err(CommandError::failure("Missing metadata value"))
}

fn permission_rule_context(
    arguments: &ParsedArguments,
) -> Result<PermissionRuleContext, CommandError> {
    let mut contexts = Vec::new();
    if let Ok(expression) = arguments.get::<PermissionRuleExpression>("permission") {
        contexts.push(expression.context().clone());
    }
    if let Ok(domain) = arguments.get::<String>("context_domain") {
        contexts.push(PermissionRuleContext::domain(domain));
    }
    if let Ok(world) = arguments.get::<Arc<World>>("context_world") {
        contexts.push(PermissionRuleContext::world(world.key.clone()));
    }
    if let Some(custom) = custom_permission_rule_context(arguments)? {
        contexts.push(custom);
    }

    PermissionRuleContext::all(contexts).map_err(|error| CommandError::failure(error.to_string()))
}

fn permission_context(arguments: &ParsedArguments) -> Result<PermissionContext, CommandError> {
    let rule_context = permission_rule_context(arguments)?;
    permission_context_from_rule_context(&rule_context)
}

fn permission_context_from_rule_context(
    rule_context: &PermissionRuleContext,
) -> Result<PermissionContext, CommandError> {
    let mut context = PermissionContext::global();
    append_permission_context_from_rule_context(&mut context, rule_context)?;
    Ok(context)
}

fn append_permission_context_from_rule_context(
    context: &mut PermissionContext,
    rule_context: &PermissionRuleContext,
) -> Result<(), CommandError> {
    match rule_context {
        PermissionRuleContext::Global => {}
        PermissionRuleContext::Domain(domain) => {
            *context = PermissionContext::for_domain(domain.clone());
        }
        PermissionRuleContext::World(world) => {
            *context =
                PermissionContext::for_world(world.namespace.as_ref().to_owned(), world.clone());
        }
        PermissionRuleContext::Custom { key, value } => {
            context
                .add_custom_context(key.clone(), value.clone())
                .map_err(|error| CommandError::failure(error.to_string()))?;
        }
        PermissionRuleContext::All(contexts) => {
            for rule_context in contexts.iter() {
                append_permission_context_from_rule_context(context, rule_context)?;
            }
        }
    }
    Ok(())
}

fn custom_permission_rule_context(
    arguments: &ParsedArguments,
) -> Result<Option<PermissionRuleContext>, CommandError> {
    let Some((key, value)) = custom_permission_context(arguments)? else {
        return Ok(None);
    };

    PermissionRuleContext::custom(key, value)
        .map(Some)
        .map_err(|error| CommandError::failure(error.to_string()))
}

fn custom_permission_context(
    arguments: &ParsedArguments,
) -> Result<Option<(PermissionContextKey, String)>, CommandError> {
    let Ok(key) = arguments.get::<String>("context_custom_key") else {
        return Ok(None);
    };
    let value = arguments
        .get::<String>("context_custom_value")
        .map_err(super::invalid_parsed_argument)?;
    let key = PermissionContextKey::parse(key)
        .map_err(|error| CommandError::failure(format!("Invalid context key: {error}")))?;
    Ok(Some((key, value)))
}

fn direct_permission_override_suggestions(
    prefix: &str,
    overrides: impl IntoIterator<Item = PermissionSet>,
    context: &dyn RequirementContext,
) -> Vec<SuggestionEntry> {
    let mut permissions = BTreeSet::new();
    for overrides in overrides {
        for entry in overrides.entries() {
            let expression =
                PermissionRuleExpression::new(entry.key().clone(), entry.context().clone())
                    .to_string();
            if expression.starts_with(prefix) && can_manage_permission(context, entry.key()) {
                permissions.insert(expression);
            }
        }
    }

    permissions.into_iter().map(SuggestionEntry::new).collect()
}

fn direct_metadata_override_suggestions(
    prefix: &str,
    values: impl IntoIterator<Item = PermissionValueSet>,
    context: &dyn RequirementContext,
) -> Vec<SuggestionEntry> {
    let mut keys = BTreeSet::new();
    for values in values {
        for entry in values.entries() {
            let key = entry.key().to_string();
            if key.starts_with(prefix) && can_manage_metadata(context, entry.key()) {
                keys.insert(key);
            }
        }
    }

    keys.into_iter().map(SuggestionEntry::new).collect()
}

fn group_permission_suggestions(
    prefix: &str,
    group_config: &PermissionGroupConfig,
    context: &dyn RequirementContext,
) -> Vec<SuggestionEntry> {
    let mut permissions = BTreeSet::new();
    for permission in &group_config.allow {
        push_managed_group_permission_suggestion(&mut permissions, prefix, permission, context);
    }
    for permission in &group_config.deny {
        push_managed_group_permission_suggestion(&mut permissions, prefix, permission, context);
    }
    for rule in &group_config.rules {
        push_managed_group_permission_rule_suggestion(&mut permissions, prefix, rule, context);
    }

    permissions.into_iter().map(SuggestionEntry::new).collect()
}

fn group_metadata_suggestions(
    prefix: &str,
    group_config: &PermissionGroupConfig,
    context: &dyn RequirementContext,
) -> Vec<SuggestionEntry> {
    let mut keys = BTreeSet::new();
    for value in &group_config.values {
        if !value.key.starts_with(prefix) {
            continue;
        }
        let Ok(key) = parse_permission_value_key(value.key.clone()) else {
            continue;
        };
        if can_manage_metadata(context, &key) {
            keys.insert(key.to_string());
        }
    }

    keys.into_iter().map(SuggestionEntry::new).collect()
}

fn metadata_catalog_suggestions(
    prefix: &str,
    catalog: &PermissionMetadataCatalog,
    context: &dyn RequirementContext,
) -> Vec<SuggestionEntry> {
    catalog
        .suggestions(prefix)
        .into_iter()
        .filter_map(|key| {
            let Ok(parsed_key) = parse_permission_value_key(key.clone()) else {
                return None;
            };
            can_manage_metadata(context, &parsed_key).then(|| SuggestionEntry::new(key))
        })
        .collect()
}

fn push_managed_group_permission_suggestion(
    permissions: &mut BTreeSet<String>,
    prefix: &str,
    permission: &str,
    context: &dyn RequirementContext,
) {
    if !permission.starts_with(prefix) {
        return;
    }
    let Ok(permission) = PermissionKey::parse(permission) else {
        return;
    };
    if can_manage_permission(context, &permission) {
        permissions.insert(permission.as_str().to_owned());
    }
}

fn push_managed_group_permission_rule_suggestion(
    permissions: &mut BTreeSet<String>,
    prefix: &str,
    rule: &PermissionRuleConfig,
    context: &dyn RequirementContext,
) {
    let Ok(permission) = PermissionKey::parse(rule.key.clone()) else {
        return;
    };
    if !can_manage_permission(context, &permission) {
        return;
    }

    let rule_context = rule
        .context
        .clone()
        .map_or(Ok(PermissionRuleContext::Global), |context| {
            context.into_rule_context()
        });
    let Ok(rule_context) = rule_context else {
        return;
    };
    let expression = PermissionRuleExpression::new(permission, rule_context).to_string();
    if expression.starts_with(prefix) {
        permissions.insert(expression);
    }
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

fn require_metadata_management(
    context: &dyn RequirementContext,
    key: &Identifier,
) -> Result<(), CommandError> {
    if can_manage_metadata(context, key) {
        Ok(())
    } else {
        Err(CommandError::PermissionDenied)
    }
}

fn can_manage_metadata(context: &dyn RequirementContext, key: &Identifier) -> bool {
    let Ok(management_permission) = metadata_management_key(key) else {
        return false;
    };
    context.has_permission(&PermissionExpr::key(management_permission))
}

fn metadata_management_key(key: &Identifier) -> Result<PermissionKey, PermissionKeyError> {
    let mut segments = vec![
        PermissionSegment::parse("steel")?,
        PermissionSegment::parse("permission")?,
        PermissionSegment::parse("metadata")?,
    ];
    push_metadata_permission_segments(&mut segments, key.namespace.as_ref())?;
    push_metadata_permission_segments(&mut segments, key.path.as_ref())?;
    PermissionKey::from_segments(segments)
}

fn push_metadata_permission_segments(
    segments: &mut Vec<PermissionSegment>,
    value: &str,
) -> Result<(), PermissionKeyError> {
    for segment in value.split(['.', '/']) {
        segments.push(PermissionSegment::parse(segment)?);
    }
    Ok(())
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

#[cfg(test)]
#[path = "steelperms/tests.rs"]
mod tests;
