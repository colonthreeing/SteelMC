//! Steel permission management commands.

use std::{collections::BTreeSet, fmt, sync::Arc};

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use text_components::TextComponent;

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    BoolParser, CommandArgumentClientParser, CommandArgumentParser, CommandNodeBuilder,
    CommandParseError, CommandParseErrorKind, CommandResult, IntegerParser, LongParser,
    ParsedArgument, ParsedArguments, PermissionTarget, StringParser, argument, literal,
};
use crate::command::parsers::{
    DomainParser, PermissionGroupParser, PermissionRuleExpressionParser, PermissionTargetParser,
    WorldParser,
};
use crate::command::reader::{CommandReader, StringMode};
use crate::command::requirement::{CommandInputContext, RequirementContext};
use crate::command::sender::CommandSender;
use crate::command::{CommandRegistration, CommandRegistrationError};
use crate::permission::{
    OP_GROUP, PermissionContext, PermissionContextKey, PermissionEntry, PermissionExpr,
    PermissionGroupConfig, PermissionGroupsConfig, PermissionKey, PermissionKeyError,
    PermissionMetadataCatalog, PermissionResolution, PermissionResolutionSource,
    PermissionRuleConfig, PermissionRuleContext, PermissionRuleContextConfig,
    PermissionRuleCustomContextConfig, PermissionRuleExpression, PermissionRuleStateConfig,
    PermissionSegment, PermissionSet, PermissionState, PermissionValue, PermissionValueEntry,
    PermissionValueResolution, PermissionValueRuleConfig, PermissionValueSet,
    parse_permission_value_key,
};
use crate::server::Server;
use crate::world::World;
use steel_utils::Identifier;

use super::permission_targets;

pub(crate) fn registration() -> Result<CommandRegistration, CommandRegistrationError> {
    CommandRegistration::steel(command())?.alias("sp")
}

/// Handler for the "steelperms" command group.
#[must_use]
pub fn command() -> CommandNodeBuilder {
    literal("steelperms")
        .then(user_command())
        .then(group_command())
        .then(groups_command())
}

fn user_command() -> CommandNodeBuilder {
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
                    .then(permission_key_argument(allow_permission)),
            )
            .then(
                literal("deny")
                    .requires_additional_subcommand_permission()
                    .then(permission_key_argument(deny_permission)),
            )
            .then(
                literal("unset")
                    .requires_additional_subcommand_permission()
                    .then(permission_override_argument(unset_permission)),
            )
            .then(
                literal("check")
                    .requires_subcommand_permission()
                    .then(permission_key_argument(check_permission)),
            )
            .then(user_metadata_arguments())
            .then(contextual_user_permission_arguments())
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
    )
}

fn group_command() -> CommandNodeBuilder {
    literal("group").then(
        argument("group", PermissionGroupNameParser)
            .then(
                literal("create")
                    .requires_additional_subcommand_permission()
                    .executes(create_group),
            )
            .then(
                literal("info")
                    .requires_subcommand_permission()
                    .executes(group_info),
            )
            .then(
                literal("delete")
                    .requires_additional_subcommand_permission()
                    .executes(delete_group),
            )
            .then(
                literal("allow")
                    .requires_additional_subcommand_permission()
                    .then(permission_key_argument(allow_group_permission)),
            )
            .then(
                literal("deny")
                    .requires_additional_subcommand_permission()
                    .then(permission_key_argument(deny_group_permission)),
            )
            .then(
                literal("unset")
                    .requires_additional_subcommand_permission()
                    .then(group_permission_argument(unset_group_permission)),
            )
            .then(
                literal("priority")
                    .requires_additional_subcommand_permission()
                    .then(argument("priority", IntegerParser::new()).executes(set_group_priority)),
            )
            .then(group_metadata_arguments())
            .then(contextual_group_permission_arguments()),
    )
}

fn groups_command() -> CommandNodeBuilder {
    literal("groups")
        .then(
            literal("list")
                .requires_subcommand_permission()
                .executes(group_list),
        )
        .then(
            literal("default")
                .then(
                    literal("add")
                        .requires_additional_subcommand_permission()
                        .then(argument("group", PermissionGroupParser).executes(add_default_group)),
                )
                .then(
                    literal("remove")
                        .requires_additional_subcommand_permission()
                        .then(
                            argument("group", PermissionGroupParser).executes(remove_default_group),
                        ),
                ),
        )
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
    literal("metadata")
        .then(metadata_set_arguments(set_metadata))
        .then(
            literal("check").requires_subcommand_permission().then(
                argument("metadata_key", PermissionMetadataKeyParser).executes(check_metadata),
            ),
        )
        .then(
            literal("unset")
                .requires_additional_subcommand_permission()
                .then(metadata_override_argument(unset_metadata)),
        )
}

fn group_metadata_arguments() -> CommandNodeBuilder {
    literal("metadata")
        .then(metadata_set_arguments(set_group_metadata))
        .then(
            literal("unset")
                .requires_additional_subcommand_permission()
                .then(group_metadata_argument(unset_group_metadata)),
        )
}

fn metadata_set_arguments(
    executor: fn(&mut CommandContext, &ParsedArguments) -> Result<CommandResult, CommandError>,
) -> CommandNodeBuilder {
    literal("set")
        .requires_additional_subcommand_permission()
        .then(
            literal("int").then(
                argument("metadata_key", PermissionMetadataKeyParser)
                    .then(argument("metadata_int_value", LongParser::new()).executes(executor)),
            ),
        )
        .then(
            literal("bool").then(
                argument("metadata_key", PermissionMetadataKeyParser)
                    .then(argument("metadata_bool_value", BoolParser).executes(executor)),
            ),
        )
        .then(
            literal("string").then(
                argument("metadata_key", PermissionMetadataKeyParser).then(
                    argument(
                        "metadata_string_value",
                        StringParser::new(StringMode::QuotablePhrase),
                    )
                    .executes(executor),
                ),
            ),
        )
}

fn contextual_user_permission_arguments() -> CommandNodeBuilder {
    literal("context")
        .then(literal("domain").then(user_permission_context_argument(
            "context_domain",
            DomainParser,
        )))
        .then(literal("world").then(user_permission_context_argument(
            "context_world",
            WorldParser,
        )))
        .then(user_custom_context_argument())
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
    node.then(
        literal("allow")
            .requires_additional_subcommand_permission()
            .then(permission_key_argument(allow_permission)),
    )
    .then(
        literal("deny")
            .requires_additional_subcommand_permission()
            .then(permission_key_argument(deny_permission)),
    )
    .then(
        literal("unset")
            .requires_additional_subcommand_permission()
            .then(permission_override_argument(unset_permission)),
    )
    .then(
        literal("check")
            .requires_subcommand_permission()
            .then(permission_key_argument(check_permission)),
    )
    .then(user_metadata_arguments())
}

fn contextual_group_permission_arguments() -> CommandNodeBuilder {
    literal("context")
        .then(literal("domain").then(group_permission_context_argument(
            "context_domain",
            DomainParser,
        )))
        .then(literal("world").then(group_permission_context_argument(
            "context_world",
            WorldParser,
        )))
        .then(group_custom_context_argument())
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
    node.then(
        literal("allow")
            .requires_additional_subcommand_permission()
            .then(permission_key_argument(allow_group_permission)),
    )
    .then(
        literal("deny")
            .requires_additional_subcommand_permission()
            .then(permission_key_argument(deny_group_permission)),
    )
    .then(
        literal("unset")
            .requires_additional_subcommand_permission()
            .then(group_permission_argument(unset_group_permission)),
    )
    .then(group_metadata_arguments())
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
        PermissionRuleExpressionParser.parse(reader, context)
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        PermissionRuleExpressionParser.client_parser()
    }

    fn parsed_type(&self) -> &'static str {
        PermissionRuleExpressionParser.parsed_type()
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

    fn client_parser(&self) -> CommandArgumentClientParser {
        PermissionGroupParser.client_parser()
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

#[derive(Clone, Copy, Debug, Default)]
struct PermissionGroupNameParser;

impl CommandArgumentParser for PermissionGroupNameParser {
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

    fn client_parser(&self) -> CommandArgumentClientParser {
        PermissionGroupParser.client_parser()
    }

    fn parsed_type(&self) -> &'static str {
        PermissionGroupParser.parsed_type()
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

        server
            .permission_groups
            .group_names()
            .into_iter()
            .filter(|group| group.starts_with(prefix))
            .map(SuggestionEntry::new)
            .collect()
    }
}

#[derive(Clone, Copy, Debug)]
struct PermissionGroupRuleParser {
    group_argument: &'static str,
}

impl PermissionGroupRuleParser {
    const fn new(group_argument: &'static str) -> Self {
        Self { group_argument }
    }
}

impl CommandArgumentParser for PermissionGroupRuleParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        PermissionRuleExpressionParser.parse(reader, context)
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        PermissionRuleExpressionParser.client_parser()
    }

    fn parsed_type(&self) -> &'static str {
        PermissionRuleExpressionParser.parsed_type()
    }

    fn suggest(
        &self,
        prefix: &str,
        arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let Ok(group) = arguments.get::<String>(self.group_argument) else {
            return Vec::new();
        };
        let Some(server) = context.server() else {
            return Vec::new();
        };
        let config = server.permission_groups.config_snapshot();
        let Some(group_config) = config.groups.get(&group) else {
            return Vec::new();
        };

        group_permission_suggestions(prefix, group_config, context)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct PermissionContextKeyParser;

impl CommandArgumentParser for PermissionContextKeyParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let value = reader.read_token()?;
        PermissionContextKey::parse(value.as_str()).map_err(|_| {
            CommandParseError::new(
                CommandParseErrorKind::InvalidPermissionKey(value.clone()),
                cursor,
            )
        })?;

        Ok(ParsedArgument::String(value))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::Identifier, Some(SuggestionType::AskServer))
    }

    fn parsed_type(&self) -> &'static str {
        "string"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let Some(catalog) = context.permission_context_catalog() else {
            return Vec::new();
        };

        catalog
            .key_suggestions(prefix)
            .into_iter()
            .map(SuggestionEntry::new)
            .collect()
    }
}

#[derive(Clone, Copy, Debug)]
struct PermissionContextValueParser {
    key_argument: &'static str,
}

impl PermissionContextValueParser {
    const fn new(key_argument: &'static str) -> Self {
        Self { key_argument }
    }
}

impl CommandArgumentParser for PermissionContextValueParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        reader
            .read_string(StringMode::SingleWord)
            .map(ParsedArgument::String)
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(
            ArgumentType::String {
                behavior: steel_protocol::packets::game::ArgumentStringTypeBehavior::SingleWord,
            },
            Some(SuggestionType::AskServer),
        )
    }

    fn parsed_type(&self) -> &'static str {
        "string"
    }

    fn suggest(
        &self,
        prefix: &str,
        arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let Ok(key) = arguments.get::<String>(self.key_argument) else {
            return Vec::new();
        };
        let Ok(key) = PermissionContextKey::parse(key) else {
            return Vec::new();
        };
        let Some(catalog) = context.permission_context_catalog() else {
            return Vec::new();
        };

        catalog
            .value_suggestions(&key, prefix)
            .into_iter()
            .map(SuggestionEntry::new)
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct PermissionMetadataKeyParser;

impl CommandArgumentParser for PermissionMetadataKeyParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let value = reader.read_token()?;
        let key = parse_permission_value_key(value.clone()).map_err(|_| {
            CommandParseError::new(
                CommandParseErrorKind::InvalidPermissionMetadataKey(value),
                cursor,
            )
        })?;

        Ok(ParsedArgument::Identifier(key))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::Identifier, Some(SuggestionType::AskServer))
    }

    fn parsed_type(&self) -> &'static str {
        "identifier"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let Some(catalog) = context.permission_metadata_catalog() else {
            return Vec::new();
        };

        metadata_catalog_suggestions(prefix, catalog, context)
    }
}

#[derive(Clone, Copy, Debug)]
struct PermissionMetadataOverrideParser {
    targets_argument: &'static str,
}

impl PermissionMetadataOverrideParser {
    const fn new(targets_argument: &'static str) -> Self {
        Self { targets_argument }
    }
}

impl CommandArgumentParser for PermissionMetadataOverrideParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        PermissionMetadataKeyParser.parse(reader, context)
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::Identifier, Some(SuggestionType::AskServer))
    }

    fn parsed_type(&self) -> &'static str {
        "identifier"
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

        let values = targets
            .into_iter()
            .filter_map(|target| permission_targets::cached_state(server, &target))
            .map(|state| state.value_overrides);
        direct_metadata_override_suggestions(prefix, values, context)
    }
}

#[derive(Clone, Copy, Debug)]
struct PermissionGroupMetadataParser {
    group_argument: &'static str,
}

impl PermissionGroupMetadataParser {
    const fn new(group_argument: &'static str) -> Self {
        Self { group_argument }
    }
}

impl CommandArgumentParser for PermissionGroupMetadataParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        PermissionMetadataKeyParser.parse(reader, context)
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::Identifier, Some(SuggestionType::AskServer))
    }

    fn parsed_type(&self) -> &'static str {
        "identifier"
    }

    fn suggest(
        &self,
        prefix: &str,
        arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let Ok(group) = arguments.get::<String>(self.group_argument) else {
            return Vec::new();
        };
        let Some(server) = context.server() else {
            return Vec::new();
        };
        let config = server.permission_groups.config_snapshot();
        let Some(group_config) = config.groups.get(&group) else {
            return Vec::new();
        };

        group_metadata_suggestions(prefix, group_config, context)
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

#[derive(Clone, Debug, PartialEq, Eq)]
enum PermissionGroupEditError {
    AlreadyExists(String),
    Missing(String),
    Required(String),
    Default(String),
    UnsupportedContext(PermissionRuleContext),
}

impl fmt::Display for PermissionGroupEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyExists(group) => write!(f, "permission group '{group}' already exists"),
            Self::Missing(group) => write!(f, "unknown permission group '{group}'"),
            Self::Required(group) => write!(f, "permission group '{group}' is required"),
            Self::Default(group) => {
                write!(f, "permission group '{group}' is still a default group")
            }
            Self::UnsupportedContext(context) => {
                write!(
                    f,
                    "permission group config cannot store context '{context}'"
                )
            }
        }
    }
}

impl std::error::Error for PermissionGroupEditError {}

fn create_group_config(
    config: &mut PermissionGroupsConfig,
    group: &str,
) -> Result<(), PermissionGroupEditError> {
    if config.groups.contains_key(group) {
        return Err(PermissionGroupEditError::AlreadyExists(group.to_owned()));
    }

    config
        .groups
        .insert(group.to_owned(), PermissionGroupConfig::default());
    Ok(())
}

fn delete_group_config(
    config: &mut PermissionGroupsConfig,
    group: &str,
) -> Result<(), PermissionGroupEditError> {
    if group == OP_GROUP {
        return Err(PermissionGroupEditError::Required(group.to_owned()));
    }
    if !config.groups.contains_key(group) {
        return Err(PermissionGroupEditError::Missing(group.to_owned()));
    }
    if config.default_groups.iter().any(|default| default == group) {
        return Err(PermissionGroupEditError::Default(group.to_owned()));
    }

    config.groups.remove(group);
    Ok(())
}

fn add_default_group_config(
    config: &mut PermissionGroupsConfig,
    group: &str,
) -> Result<bool, PermissionGroupEditError> {
    if !config.groups.contains_key(group) {
        return Err(PermissionGroupEditError::Missing(group.to_owned()));
    }
    if config.default_groups.iter().any(|default| default == group) {
        return Ok(false);
    }

    config.default_groups.push(group.to_owned());
    Ok(true)
}

fn remove_default_group_config(
    config: &mut PermissionGroupsConfig,
    group: &str,
) -> Result<bool, PermissionGroupEditError> {
    if !config.groups.contains_key(group) {
        return Err(PermissionGroupEditError::Missing(group.to_owned()));
    }
    let old_len = config.default_groups.len();
    config.default_groups.retain(|default| default != group);
    Ok(config.default_groups.len() != old_len)
}

fn set_group_config_permission(
    config: &mut PermissionGroupsConfig,
    group: &str,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
    state: PermissionState,
) -> Result<bool, PermissionGroupEditError> {
    let Some(group_config) = config.groups.get_mut(group) else {
        return Err(PermissionGroupEditError::Missing(group.to_owned()));
    };
    let states = group_config_permission_states(group_config, permission, rule_context);
    if states.len() == 1 && states[0] == state {
        return Ok(false);
    }

    remove_group_config_permission(group_config, permission, rule_context);
    push_group_config_permission(group_config, permission, rule_context, state)?;
    Ok(true)
}

fn unset_group_config_permission(
    config: &mut PermissionGroupsConfig,
    group: &str,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
) -> Result<bool, PermissionGroupEditError> {
    let Some(group_config) = config.groups.get_mut(group) else {
        return Err(PermissionGroupEditError::Missing(group.to_owned()));
    };

    Ok(remove_group_config_permission(
        group_config,
        permission,
        rule_context,
    ))
}

fn set_group_config_metadata(
    config: &mut PermissionGroupsConfig,
    group: &str,
    key: &Identifier,
    value: &PermissionValue,
    rule_context: &PermissionRuleContext,
) -> Result<bool, PermissionGroupEditError> {
    let Some(group_config) = config.groups.get_mut(group) else {
        return Err(PermissionGroupEditError::Missing(group.to_owned()));
    };
    if group_config_metadata_value(group_config, key, rule_context) == Some(value) {
        return Ok(false);
    }

    remove_group_config_metadata(group_config, key, rule_context);
    push_group_config_metadata(group_config, key, value, rule_context)?;
    Ok(true)
}

fn unset_group_config_metadata(
    config: &mut PermissionGroupsConfig,
    group: &str,
    key: &Identifier,
    rule_context: &PermissionRuleContext,
) -> Result<bool, PermissionGroupEditError> {
    let Some(group_config) = config.groups.get_mut(group) else {
        return Err(PermissionGroupEditError::Missing(group.to_owned()));
    };

    Ok(remove_group_config_metadata(
        group_config,
        key,
        rule_context,
    ))
}

fn set_group_config_priority(
    config: &mut PermissionGroupsConfig,
    group: &str,
    priority: i32,
) -> Result<bool, PermissionGroupEditError> {
    let Some(group_config) = config.groups.get_mut(group) else {
        return Err(PermissionGroupEditError::Missing(group.to_owned()));
    };
    if group_config.priority == priority {
        return Ok(false);
    }

    group_config.priority = priority;
    Ok(true)
}

fn push_group_config_permission(
    group_config: &mut PermissionGroupConfig,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
    state: PermissionState,
) -> Result<(), PermissionGroupEditError> {
    if rule_context.is_global() {
        match state {
            PermissionState::Allow => group_config.allow.push(permission.as_str().to_owned()),
            PermissionState::Deny => group_config.deny.push(permission.as_str().to_owned()),
        }
        return Ok(());
    }

    group_config.rules.push(PermissionRuleConfig {
        key: permission.as_str().to_owned(),
        state: permission_rule_state_config(state),
        context: Some(permission_rule_context_config(rule_context)?),
    });
    Ok(())
}

fn push_group_config_metadata(
    group_config: &mut PermissionGroupConfig,
    key: &Identifier,
    value: &PermissionValue,
    rule_context: &PermissionRuleContext,
) -> Result<(), PermissionGroupEditError> {
    group_config.values.push(PermissionValueRuleConfig {
        key: key.to_string(),
        value: value.clone(),
        context: if rule_context.is_global() {
            None
        } else {
            Some(permission_rule_context_config(rule_context)?)
        },
    });
    Ok(())
}

fn group_config_permission_states(
    group_config: &PermissionGroupConfig,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
) -> Vec<PermissionState> {
    let mut states = Vec::new();
    if rule_context.is_global() {
        states.extend(
            group_config
                .allow
                .iter()
                .filter(|key| key.as_str() == permission.as_str())
                .map(|_| PermissionState::Allow),
        );
        states.extend(
            group_config
                .deny
                .iter()
                .filter(|key| key.as_str() == permission.as_str())
                .map(|_| PermissionState::Deny),
        );
    }
    states.extend(
        group_config
            .rules
            .iter()
            .filter(|rule| {
                rule.key == permission.as_str()
                    && permission_rule_config_matches(rule.context.as_ref(), rule_context)
            })
            .map(|rule| permission_state_from_rule_config(rule.state)),
    );

    states
}

fn group_config_metadata_value<'a>(
    group_config: &'a PermissionGroupConfig,
    key: &Identifier,
    rule_context: &PermissionRuleContext,
) -> Option<&'a PermissionValue> {
    group_config
        .values
        .iter()
        .find(|value| {
            value.key == key.to_string()
                && permission_rule_config_matches(value.context.as_ref(), rule_context)
        })
        .map(|value| &value.value)
}

fn remove_group_config_permission(
    group_config: &mut PermissionGroupConfig,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
) -> bool {
    let mut changed = false;
    if rule_context.is_global() {
        let old_allow_len = group_config.allow.len();
        group_config
            .allow
            .retain(|key| key.as_str() != permission.as_str());
        changed |= group_config.allow.len() != old_allow_len;

        let old_deny_len = group_config.deny.len();
        group_config
            .deny
            .retain(|key| key.as_str() != permission.as_str());
        changed |= group_config.deny.len() != old_deny_len;
    }

    let old_rules_len = group_config.rules.len();
    group_config.rules.retain(|rule| {
        rule.key != permission.as_str()
            || !permission_rule_config_matches(rule.context.as_ref(), rule_context)
    });
    changed | (group_config.rules.len() != old_rules_len)
}

fn remove_group_config_metadata(
    group_config: &mut PermissionGroupConfig,
    key: &Identifier,
    rule_context: &PermissionRuleContext,
) -> bool {
    let old_len = group_config.values.len();
    group_config.values.retain(|value| {
        value.key != key.to_string()
            || !permission_rule_config_matches(value.context.as_ref(), rule_context)
    });
    group_config.values.len() != old_len
}

fn permission_rule_context_config(
    rule_context: &PermissionRuleContext,
) -> Result<PermissionRuleContextConfig, PermissionGroupEditError> {
    let mut config = PermissionRuleContextConfig::default();
    append_permission_rule_context_config(&mut config, rule_context)?;
    if config.domain.is_none() && config.world.is_none() && config.custom.is_empty() {
        return Err(PermissionGroupEditError::UnsupportedContext(
            rule_context.clone(),
        ));
    }
    Ok(config)
}

fn append_permission_rule_context_config(
    config: &mut PermissionRuleContextConfig,
    rule_context: &PermissionRuleContext,
) -> Result<(), PermissionGroupEditError> {
    match rule_context {
        PermissionRuleContext::Global => {}
        PermissionRuleContext::Domain(domain) => config.domain = Some(domain.clone()),
        PermissionRuleContext::World(world) => config.world = Some(world.to_string()),
        PermissionRuleContext::Custom { key, value } => {
            config.custom.push(PermissionRuleCustomContextConfig {
                key: key.as_str().to_owned(),
                value: value.clone(),
            });
        }
        PermissionRuleContext::All(contexts) => {
            for context in contexts.iter() {
                append_permission_rule_context_config(config, context)?;
            }
        }
    }
    Ok(())
}

fn permission_rule_config_matches(
    config: Option<&PermissionRuleContextConfig>,
    rule_context: &PermissionRuleContext,
) -> bool {
    match config {
        None => rule_context.is_global(),
        Some(config) => config
            .clone()
            .into_rule_context()
            .is_ok_and(|context| &context == rule_context),
    }
}

fn permission_rule_state_config(state: PermissionState) -> PermissionRuleStateConfig {
    match state {
        PermissionState::Allow => PermissionRuleStateConfig::Allow,
        PermissionState::Deny => PermissionRuleStateConfig::Deny,
    }
}

fn permission_state_from_rule_config(state: PermissionRuleStateConfig) -> PermissionState {
    match state {
        PermissionRuleStateConfig::Allow => PermissionState::Allow,
        PermissionRuleStateConfig::Deny => PermissionState::Deny,
    }
}

fn send_user_info(
    sender: &CommandSender,
    target: &PermissionTarget,
    state: &permission_targets::PermissionTargetState,
) {
    let groups = group_list_text(&state.groups);
    let overrides = permission_entries_text(state.overrides.entries());
    let metadata = metadata_entries_text(state.value_overrides.entries());

    sender.send_message(&TextComponent::plain(format!(
        "{}: groups [{}], direct permissions [{}], metadata [{}]",
        target.name(),
        groups,
        overrides,
        metadata
    )));
}

fn send_permission_check(
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

fn send_metadata_check(
    sender: &CommandSender,
    target: &PermissionTarget,
    key: &Identifier,
    rule_context: &PermissionRuleContext,
    resolution: Option<&PermissionValueResolution>,
) {
    let value = resolution
        .map(|resolution| permission_value_text(resolution.value()))
        .unwrap_or_else(|| "unset".to_owned());
    let detail = resolution.map_or_else(|| "unset".to_owned(), metadata_resolution_text);
    sender.send_message(&TextComponent::plain(format!(
        "{}: metadata '{key}' in {rule_context} is {value} ({detail})",
        target.name()
    )));
}

fn send_set_permission_summary(
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

fn send_unset_permission_summary(
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

fn send_set_metadata_summary(
    sender: &CommandSender,
    key: &Identifier,
    value: &PermissionValue,
    rule_context: &PermissionRuleContext,
    count: usize,
) {
    sender.send_message(&TextComponent::plain(format!(
        "Set metadata '{key}'{} = {} for {}",
        permission_rule_context_suffix(rule_context),
        permission_value_text(value),
        target_count_text(count)
    )));
}

fn send_unset_metadata_summary(
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

fn send_set_group_permission_summary(
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

fn send_set_group_priority_summary(sender: &CommandSender, group: &str, priority: i32) {
    sender.send_message(&TextComponent::plain(format!(
        "Set priority {priority} for group '{group}'"
    )));
}

fn send_group_priority_unchanged_summary(sender: &CommandSender, group: &str, priority: i32) {
    sender.send_message(&TextComponent::plain(format!(
        "Group '{group}' already has priority {priority}"
    )));
}

fn send_add_default_group_summary(sender: &CommandSender, group: &str) {
    sender.send_message(&TextComponent::plain(format!(
        "Added group '{group}' to default groups"
    )));
}

fn send_default_group_already_set_summary(sender: &CommandSender, group: &str) {
    sender.send_message(&TextComponent::plain(format!(
        "Group '{group}' is already a default group"
    )));
}

fn send_remove_default_group_summary(sender: &CommandSender, group: &str) {
    sender.send_message(&TextComponent::plain(format!(
        "Removed group '{group}' from default groups"
    )));
}

fn send_default_group_not_set_summary(sender: &CommandSender, group: &str) {
    sender.send_message(&TextComponent::plain(format!(
        "Group '{group}' is not a default group"
    )));
}

fn send_set_group_metadata_summary(
    sender: &CommandSender,
    group: &str,
    key: &Identifier,
    value: &PermissionValue,
    rule_context: &PermissionRuleContext,
) {
    sender.send_message(&TextComponent::plain(format!(
        "Set metadata '{key}'{} = {} for group '{group}'",
        permission_rule_context_suffix(rule_context),
        permission_value_text(value)
    )));
}

fn send_group_metadata_unchanged_summary(
    sender: &CommandSender,
    group: &str,
    key: &Identifier,
    value: &PermissionValue,
    rule_context: &PermissionRuleContext,
) {
    sender.send_message(&TextComponent::plain(format!(
        "Group '{group}' already sets metadata '{key}'{} = {}",
        permission_rule_context_suffix(rule_context),
        permission_value_text(value)
    )));
}

fn send_group_permission_unchanged_summary(
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

fn send_unset_group_metadata_summary(
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

fn send_group_metadata_not_set_summary(
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

fn send_unset_group_permission_summary(
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

fn send_group_permission_not_set_summary(
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

fn send_background_error(sender: &CommandSender, command: &str, error: CommandError) {
    sender.send_failure_feedback(error.into_feedback(command));
}

fn send_group_update_error(
    sender: &CommandSender,
    error: crate::permission::PermissionGroupUpdateError<PermissionGroupEditError>,
) {
    sender.send_failure(error.to_string());
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

fn group_list_text(groups: &[String]) -> String {
    if groups.is_empty() {
        return "none".to_owned();
    }

    groups.join(", ")
}

fn permission_key_list_text(permissions: &[String]) -> String {
    if permissions.is_empty() {
        return "none".to_owned();
    }

    permissions.join(", ")
}

fn group_rule_list_text(rules: &[PermissionRuleConfig]) -> String {
    if rules.is_empty() {
        return "none".to_owned();
    }

    rules
        .iter()
        .map(|rule| {
            format!(
                "{} {}{}",
                permission_state_text(permission_state_from_rule_config(rule.state)),
                rule.key,
                permission_rule_config_suffix(rule.context.as_ref())
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn group_metadata_list_text(values: &[PermissionValueRuleConfig]) -> String {
    if values.is_empty() {
        return "none".to_owned();
    }

    values
        .iter()
        .map(|value| {
            format!(
                "{} = {}{}",
                value.key,
                permission_value_text(&value.value),
                permission_rule_config_suffix(value.context.as_ref())
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

fn metadata_entries_text(entries: &[PermissionValueEntry]) -> String {
    if entries.is_empty() {
        return "none".to_owned();
    }

    entries
        .iter()
        .map(|entry| {
            format!(
                "{} = {}{}",
                entry.key(),
                permission_value_text(entry.value()),
                permission_rule_context_suffix(entry.context())
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn permission_value_text(value: &PermissionValue) -> String {
    match value {
        PermissionValue::Bool(value) => value.to_string(),
        PermissionValue::Integer(value) => value.to_string(),
        PermissionValue::String(value) => format!("{value:?}"),
    }
}

fn permission_rule_context_suffix(rule_context: &PermissionRuleContext) -> String {
    if rule_context.is_global() {
        String::new()
    } else {
        format!(" ({rule_context})")
    }
}

fn permission_rule_config_suffix(context: Option<&PermissionRuleContextConfig>) -> String {
    match context {
        None => String::new(),
        Some(context) => context.clone().into_rule_context().map_or_else(
            |_| " (invalid context)".to_owned(),
            |context| permission_rule_context_suffix(&context),
        ),
    }
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

fn permission_state_text(state: PermissionState) -> &'static str {
    match state {
        PermissionState::Allow => "allow",
        PermissionState::Deny => "deny",
    }
}

fn permission_check_result_text(state: PermissionState) -> &'static str {
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

fn permission_resolution_source_text(source: &PermissionResolutionSource) -> String {
    match source {
        PermissionResolutionSource::Subject => "direct permission".to_owned(),
        PermissionResolutionSource::Group { name, priority } => {
            format!("group '{name}' priority {priority}")
        }
    }
}

fn metadata_resolution_text(resolution: &PermissionValueResolution) -> String {
    format!(
        "set by {}, rule {} = {}{}, context specificity {}, insertion {}",
        permission_resolution_source_text(resolution.source()),
        resolution.key(),
        permission_value_text(resolution.value()),
        permission_rule_context_suffix(resolution.context()),
        resolution.context_specificity(),
        resolution.insertion_index()
    )
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
        PermissionAssignedGroupParser, PermissionContextKeyParser, PermissionContextValueParser,
        PermissionGroupEditError, PermissionGroupNameParser, PermissionMetadataKeyParser,
        add_default_group_config, assigned_group_suggestions, can_manage_group,
        can_manage_metadata, can_manage_permission, delete_group_config,
        direct_metadata_override_suggestions, direct_permission_override_suggestions,
        group_config_metadata_value, group_config_permission_states, group_metadata_suggestions,
        group_permission_suggestions, metadata_catalog_suggestions, metadata_management_key,
        metadata_resolution_text, permission_check_result_text, permission_context,
        permission_resolution_source_text, permission_rule_context, permission_rule_context_suffix,
        remove_default_group_config, set_group_config_metadata, set_group_config_permission,
        set_group_config_priority, unset_group_config_metadata, unset_group_config_permission,
    };
    use crate::command::graph::{
        CommandArgumentParser, CommandParseErrorKind, ParsedArgument, ParsedArguments,
    };
    use crate::command::reader::CommandReader;
    use crate::command::requirement::{
        CommandInputContext, CommandSourceKind, PermissionExpr, RequirementContext,
    };
    use crate::permission::{
        PermissionContext, PermissionContextCatalog, PermissionContextCatalogSource,
        PermissionContextKey, PermissionEntry, PermissionGroupConfig, PermissionGroupsConfig,
        PermissionKey, PermissionMetadataCatalog, PermissionMetadataCatalogSource,
        PermissionResolutionSource, PermissionRuleConfig, PermissionRuleContext,
        PermissionRuleContextConfig, PermissionRuleCustomContextConfig, PermissionRuleStateConfig,
        PermissionSet, PermissionState, PermissionValue, PermissionValueEntry,
        PermissionValueRuleConfig, PermissionValueSet, parse_permission_value_key,
    };
    use steel_utils::Identifier;

    struct TestContext {
        permissions: PermissionSet,
        metadata_catalog: PermissionMetadataCatalog,
        context_catalog: PermissionContextCatalog,
    }

    impl TestContext {
        fn empty() -> Self {
            Self {
                permissions: PermissionSet::new(),
                metadata_catalog: PermissionMetadataCatalog::new(),
                context_catalog: PermissionContextCatalog::new(),
            }
        }

        fn with_permissions<const N: usize>(permissions: [&str; N]) -> Self {
            Self {
                permissions: PermissionSet::from_entries(
                    permissions.map(|permission| PermissionEntry::allow(key(permission))),
                ),
                metadata_catalog: PermissionMetadataCatalog::new(),
                context_catalog: PermissionContextCatalog::new(),
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

    impl CommandInputContext for TestContext {
        fn permission_metadata_catalog(&self) -> Option<&PermissionMetadataCatalog> {
            Some(&self.metadata_catalog)
        }

        fn permission_context_catalog(&self) -> Option<&PermissionContextCatalog> {
            Some(&self.context_catalog)
        }
    }

    fn key(value: &str) -> PermissionKey {
        PermissionKey::parse(value).expect("permission key parses")
    }

    fn metadata_key(value: &str) -> Identifier {
        parse_permission_value_key(value).expect("metadata key parses")
    }

    fn context_key(value: &str) -> PermissionContextKey {
        PermissionContextKey::parse(value).expect("context key parses")
    }

    fn custom_context(key: &str, value: &str) -> PermissionRuleContext {
        PermissionRuleContext::custom(context_key(key), value).expect("custom context parses")
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
    fn unset_permission_suggestions_include_contextual_overrides() {
        let overrides = PermissionSet::from_entries([
            PermissionEntry::deny_with_context(
                key("steel.fly"),
                PermissionRuleContext::domain("lobby"),
            ),
            PermissionEntry::allow(key("steel.stop")),
        ]);

        assert_eq!(
            suggestion_texts(direct_permission_override_suggestions(
                "steel.f",
                [overrides],
                &TestContext::with_permissions(["steel.permission.manage.steel.*"]),
            )),
            vec!["steel.fly{domain=lobby}"]
        );
    }

    #[test]
    fn permission_context_suffix_describes_non_global_contexts() {
        assert_eq!(
            permission_rule_context_suffix(&PermissionRuleContext::Global),
            ""
        );
        assert_eq!(
            permission_rule_context_suffix(&PermissionRuleContext::domain("lobby")),
            " (domain lobby)"
        );
        assert_eq!(
            permission_rule_context_suffix(&PermissionRuleContext::world(Identifier::new(
                "lobby", "spawn"
            ))),
            " (world lobby:spawn)"
        );
    }

    #[test]
    fn permission_check_result_text_uses_command_language() {
        assert_eq!(
            permission_check_result_text(PermissionState::Allow),
            "allowed"
        );
        assert_eq!(
            permission_check_result_text(PermissionState::Deny),
            "denied"
        );
    }

    #[test]
    fn permission_resolution_source_text_includes_group_priority() {
        assert_eq!(
            permission_resolution_source_text(&PermissionResolutionSource::Subject),
            "direct permission"
        );
        assert_eq!(
            permission_resolution_source_text(&PermissionResolutionSource::Group {
                name: "admin".to_owned(),
                priority: 50,
            }),
            "group 'admin' priority 50"
        );
    }

    #[test]
    fn metadata_key_parser_requests_server_suggestions() {
        let (_, suggestion_type) = PermissionMetadataKeyParser
            .client_parser()
            .into_protocol_argument();
        assert!(matches!(
            suggestion_type,
            Some(steel_protocol::packets::game::SuggestionType::AskServer)
        ));
    }

    #[test]
    fn context_key_and_value_parsers_request_server_suggestions() {
        let (key_argument_type, key_suggestion_type) = PermissionContextKeyParser
            .client_parser()
            .into_protocol_argument();
        assert!(matches!(
            key_argument_type,
            steel_protocol::packets::game::ArgumentType::Identifier
        ));
        assert!(matches!(
            key_suggestion_type,
            Some(steel_protocol::packets::game::SuggestionType::AskServer)
        ));
        let (_, value_suggestion_type) = PermissionContextValueParser::new("context_custom_key")
            .client_parser()
            .into_protocol_argument();
        assert!(matches!(
            value_suggestion_type,
            Some(steel_protocol::packets::game::SuggestionType::AskServer)
        ));
    }

    #[test]
    fn context_key_parser_accepts_namespaced_keys() {
        let mut reader = CommandReader::new("plugin:region");
        let parsed = PermissionContextKeyParser
            .parse(&mut reader, &TestContext::empty())
            .expect("namespaced context key parses");

        assert!(
            matches!(parsed, ParsedArgument::String(ref value) if value == "plugin:region"),
            "expected parsed context key string, got {parsed:?}"
        );
        assert_eq!(reader.remaining(), "");
    }

    #[test]
    fn context_catalog_suggestions_include_known_keys_and_values() {
        let mut context = TestContext::empty();
        context.context_catalog.insert_value(
            context_key("region"),
            "spawn",
            PermissionContextCatalogSource::Config,
        );
        context.context_catalog.insert_value(
            context_key("region"),
            "market",
            PermissionContextCatalogSource::Config,
        );
        context.context_catalog.insert_value(
            context_key("arena"),
            "duel",
            PermissionContextCatalogSource::Config,
        );
        let mut arguments = ParsedArguments::default();
        arguments.insert(
            "context_custom_key",
            ParsedArgument::String("region".to_owned()),
        );

        assert_eq!(
            suggestion_texts(PermissionContextKeyParser.suggest(
                "r",
                &ParsedArguments::default(),
                &context
            )),
            vec!["region"]
        );
        assert_eq!(
            suggestion_texts(
                PermissionContextValueParser::new("context_custom_key")
                    .suggest("s", &arguments, &context)
            ),
            vec!["spawn"]
        );
    }

    #[test]
    fn metadata_catalog_suggestions_only_include_manageable_keys() {
        let mut context = TestContext::with_permissions(["steel.permission.metadata.plugin.*"]);
        context.metadata_catalog.insert(
            metadata_key("plugin:homes"),
            PermissionMetadataCatalogSource::Config,
        );
        context.metadata_catalog.insert(
            metadata_key("other:homes"),
            PermissionMetadataCatalogSource::Config,
        );

        assert_eq!(
            suggestion_texts(PermissionMetadataKeyParser.suggest(
                "",
                &ParsedArguments::default(),
                &context
            )),
            vec!["plugin:homes"]
        );
        assert_eq!(
            suggestion_texts(metadata_catalog_suggestions(
                "plugin:",
                &context.metadata_catalog,
                &context
            )),
            vec!["plugin:homes"]
        );
    }

    #[test]
    fn permission_context_key_parser_rejects_invalid_segments() {
        let mut reader = CommandReader::new("region.spawn");
        let error = PermissionContextKeyParser
            .parse(&mut reader, &TestContext::empty())
            .expect_err("context key should be one permission segment");

        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::InvalidPermissionKey(value) if value == "region.spawn"
        ));
    }

    #[test]
    fn parsed_contexts_can_chain_domain_and_custom_constraints() {
        let permission = key("steel.region.build");
        let mut arguments = ParsedArguments::default();
        arguments.insert("context_domain", ParsedArgument::String("lobby".to_owned()));
        arguments.insert(
            "context_custom_key",
            ParsedArgument::String("region".to_owned()),
        );
        arguments.insert(
            "context_custom_value",
            ParsedArgument::String("spawn".to_owned()),
        );
        let Ok(rule_context) = permission_rule_context(&arguments) else {
            panic!("rule context should parse");
        };
        let Ok(check_context) = permission_context(&arguments) else {
            panic!("check context should parse");
        };
        let permissions = PermissionSet::from_entries([PermissionEntry::allow_with_context(
            permission.clone(),
            rule_context,
        )]);

        assert!(permissions.allows_key_in(&permission, &check_context));
        assert!(!permissions.allows_key_in(&permission, &PermissionContext::for_domain("lobby")));
    }

    #[test]
    fn metadata_resolution_text_describes_winning_value_rule() {
        let homes = metadata_key("plugin:homes");
        let values = PermissionValueSet::from_entries([PermissionValueEntry::new_with_context(
            homes.clone(),
            PermissionRuleContext::domain("lobby"),
            PermissionValue::Integer(10),
        )]);
        let resolution = values
            .resolve_in_detailed(&homes, &PermissionContext::for_domain("lobby"))
            .expect("metadata value resolves");

        assert_eq!(
            metadata_resolution_text(&resolution),
            "set by direct permission, rule plugin:homes = 10 (domain lobby), context specificity 1, insertion 0"
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
    fn group_name_parser_accepts_unknown_group_names() {
        let mut reader = CommandReader::new("builder");
        let parsed = PermissionGroupNameParser
            .parse(&mut reader, &TestContext::empty())
            .expect("group name parses");

        assert!(matches!(parsed, ParsedArgument::String(group) if group == "builder"));
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

    #[test]
    fn manage_metadata_uses_namespaced_key_permission() {
        let context = TestContext::with_permissions(["steel.permission.metadata.plugin.homes"]);
        let homes = metadata_key("plugin:homes");
        let other = metadata_key("plugin:other");

        assert_eq!(
            metadata_management_key(&homes),
            Ok(key("steel.permission.metadata.plugin.homes"))
        );
        assert!(can_manage_metadata(&context, &homes));
        assert!(!can_manage_metadata(&context, &other));
    }

    #[test]
    fn direct_metadata_suggestions_only_include_manageable_overrides() {
        let values = PermissionValueSet::from_entries([
            PermissionValueEntry::new(metadata_key("plugin:homes"), PermissionValue::Integer(10)),
            PermissionValueEntry::new(metadata_key("other:homes"), PermissionValue::Integer(20)),
        ]);

        assert_eq!(
            suggestion_texts(direct_metadata_override_suggestions(
                "",
                [values],
                &TestContext::with_permissions(["steel.permission.metadata.plugin.*"]),
            )),
            vec!["plugin:homes"]
        );
    }

    #[test]
    fn group_config_global_permission_edits_use_allow_and_deny_lists() {
        let mut config = PermissionGroupsConfig::default();
        let permission = key("steel.fly");

        assert_eq!(
            set_group_config_permission(
                &mut config,
                "default",
                &permission,
                &PermissionRuleContext::Global,
                PermissionState::Allow,
            ),
            Ok(true)
        );
        let default = config.groups.get("default").expect("default group exists");
        assert_eq!(default.allow, vec!["steel.fly"]);
        assert!(default.deny.is_empty());
        assert!(default.rules.is_empty());

        assert_eq!(
            set_group_config_permission(
                &mut config,
                "default",
                &permission,
                &PermissionRuleContext::Global,
                PermissionState::Deny,
            ),
            Ok(true)
        );
        let default = config.groups.get("default").expect("default group exists");
        assert!(default.allow.is_empty());
        assert_eq!(default.deny, vec!["steel.fly"]);

        assert_eq!(
            set_group_config_permission(
                &mut config,
                "default",
                &permission,
                &PermissionRuleContext::Global,
                PermissionState::Deny,
            ),
            Ok(false)
        );
        assert_eq!(
            unset_group_config_permission(
                &mut config,
                "default",
                &permission,
                &PermissionRuleContext::Global,
            ),
            Ok(true)
        );
        assert_eq!(
            unset_group_config_permission(
                &mut config,
                "default",
                &permission,
                &PermissionRuleContext::Global,
            ),
            Ok(false)
        );
    }

    #[test]
    fn group_config_contextual_permission_edits_use_structured_rules() {
        let mut config = PermissionGroupsConfig::default();
        let permission = key("steel.fly");
        let lobby = PermissionRuleContext::domain("lobby");

        assert_eq!(
            set_group_config_permission(
                &mut config,
                "default",
                &permission,
                &lobby,
                PermissionState::Allow,
            ),
            Ok(true)
        );
        let default = config.groups.get("default").expect("default group exists");
        assert!(default.allow.is_empty());
        assert!(default.deny.is_empty());
        assert_eq!(default.rules.len(), 1);
        assert_eq!(default.rules[0].key, "steel.fly");
        assert_eq!(default.rules[0].state, PermissionRuleStateConfig::Allow);
        assert_eq!(
            default.rules[0].context,
            Some(PermissionRuleContextConfig {
                domain: Some("lobby".to_owned()),
                world: None,
                custom: Vec::new(),
            })
        );

        assert_eq!(
            set_group_config_permission(
                &mut config,
                "default",
                &permission,
                &lobby,
                PermissionState::Deny,
            ),
            Ok(true)
        );
        let default = config.groups.get("default").expect("default group exists");
        assert_eq!(default.rules.len(), 1);
        assert_eq!(default.rules[0].state, PermissionRuleStateConfig::Deny);

        assert_eq!(
            group_config_permission_states(default, &permission, &lobby),
            vec![PermissionState::Deny]
        );
    }

    #[test]
    fn group_config_custom_context_permission_edits_use_structured_rules() {
        let mut config = PermissionGroupsConfig::default();
        let permission = key("steel.region.build");
        let spawn_region = custom_context("region", "spawn");

        assert_eq!(
            set_group_config_permission(
                &mut config,
                "default",
                &permission,
                &spawn_region,
                PermissionState::Allow,
            ),
            Ok(true)
        );

        let default = config.groups.get("default").expect("default group exists");
        assert_eq!(default.rules.len(), 1);
        assert_eq!(default.rules[0].key, "steel.region.build");
        assert_eq!(default.rules[0].state, PermissionRuleStateConfig::Allow);
        assert_eq!(
            default.rules[0].context,
            Some(PermissionRuleContextConfig {
                domain: None,
                world: None,
                custom: vec![PermissionRuleCustomContextConfig {
                    key: "region".to_owned(),
                    value: "spawn".to_owned(),
                }],
            })
        );
        assert_eq!(
            group_config_permission_states(default, &permission, &spawn_region),
            vec![PermissionState::Allow]
        );
    }

    #[test]
    fn group_config_metadata_edits_use_value_rules() {
        let mut config = PermissionGroupsConfig::default();
        let homes = metadata_key("plugin:homes");
        let lobby = PermissionRuleContext::domain("lobby");

        assert_eq!(
            set_group_config_metadata(
                &mut config,
                "default",
                &homes,
                &PermissionValue::Integer(10),
                &PermissionRuleContext::Global,
            ),
            Ok(true)
        );
        assert_eq!(
            set_group_config_metadata(
                &mut config,
                "default",
                &homes,
                &PermissionValue::Integer(5),
                &lobby,
            ),
            Ok(true)
        );
        let default = config.groups.get("default").expect("default group exists");
        assert_eq!(default.values.len(), 2);
        assert_eq!(
            group_config_metadata_value(default, &homes, &PermissionRuleContext::Global),
            Some(&PermissionValue::Integer(10))
        );
        assert_eq!(
            group_config_metadata_value(default, &homes, &lobby),
            Some(&PermissionValue::Integer(5))
        );

        assert_eq!(
            set_group_config_metadata(
                &mut config,
                "default",
                &homes,
                &PermissionValue::Integer(5),
                &lobby,
            ),
            Ok(false)
        );
    }

    #[test]
    fn group_config_custom_context_metadata_edits_use_value_rules() {
        let mut config = PermissionGroupsConfig::default();
        let homes = metadata_key("plugin:homes");
        let spawn_region = custom_context("region", "spawn");

        assert_eq!(
            set_group_config_metadata(
                &mut config,
                "default",
                &homes,
                &PermissionValue::Integer(10),
                &spawn_region,
            ),
            Ok(true)
        );

        let default = config.groups.get("default").expect("default group exists");
        assert_eq!(default.values.len(), 1);
        assert_eq!(default.values[0].key, "plugin:homes");
        assert_eq!(default.values[0].value, PermissionValue::Integer(10));
        assert_eq!(
            default.values[0].context,
            Some(PermissionRuleContextConfig {
                domain: None,
                world: None,
                custom: vec![PermissionRuleCustomContextConfig {
                    key: "region".to_owned(),
                    value: "spawn".to_owned(),
                }],
            })
        );
        assert_eq!(
            group_config_metadata_value(default, &homes, &spawn_region),
            Some(&PermissionValue::Integer(10))
        );
    }

    #[test]
    fn group_config_metadata_unset_is_context_exact() {
        let mut config = PermissionGroupsConfig::default();
        let homes = metadata_key("plugin:homes");
        let global = PermissionRuleContext::Global;
        let lobby = PermissionRuleContext::domain("lobby");

        set_group_config_metadata(
            &mut config,
            "default",
            &homes,
            &PermissionValue::Integer(10),
            &global,
        )
        .expect("global metadata stores");
        set_group_config_metadata(
            &mut config,
            "default",
            &homes,
            &PermissionValue::Integer(5),
            &lobby,
        )
        .expect("contextual metadata stores");

        assert_eq!(
            unset_group_config_metadata(&mut config, "default", &homes, &global),
            Ok(true)
        );
        let default = config.groups.get("default").expect("default group exists");
        assert_eq!(default.values.len(), 1);
        assert_eq!(
            group_config_metadata_value(default, &homes, &lobby),
            Some(&PermissionValue::Integer(5))
        );
    }

    #[test]
    fn group_config_priority_edit_updates_group_priority() {
        let mut config = PermissionGroupsConfig::default();
        let default = config.groups.get("default").expect("default group exists");
        assert_eq!(default.priority, 0);

        assert_eq!(
            set_group_config_priority(&mut config, "default", 50),
            Ok(true)
        );
        let default = config.groups.get("default").expect("default group exists");
        assert_eq!(default.priority, 50);
        assert_eq!(
            set_group_config_priority(&mut config, "default", 50),
            Ok(false)
        );
    }

    #[test]
    fn group_config_priority_edit_reports_missing_group() {
        let mut config = PermissionGroupsConfig::default();

        assert_eq!(
            set_group_config_priority(&mut config, "missing", 10),
            Err(PermissionGroupEditError::Missing("missing".to_owned()))
        );
    }

    #[test]
    fn default_group_config_add_and_remove_update_default_groups() {
        let mut config = PermissionGroupsConfig::default();
        config
            .groups
            .insert("builder".to_owned(), PermissionGroupConfig::default());

        assert_eq!(add_default_group_config(&mut config, "builder"), Ok(true));
        assert_eq!(config.default_groups, vec!["default", "builder"]);
        assert_eq!(add_default_group_config(&mut config, "builder"), Ok(false));
        assert_eq!(
            remove_default_group_config(&mut config, "builder"),
            Ok(true)
        );
        assert_eq!(config.default_groups, vec!["default"]);
        assert_eq!(
            remove_default_group_config(&mut config, "builder"),
            Ok(false)
        );
    }

    #[test]
    fn default_group_config_edits_report_missing_groups() {
        let mut config = PermissionGroupsConfig::default();

        assert_eq!(
            add_default_group_config(&mut config, "missing"),
            Err(PermissionGroupEditError::Missing("missing".to_owned()))
        );
        assert_eq!(
            remove_default_group_config(&mut config, "missing"),
            Err(PermissionGroupEditError::Missing("missing".to_owned()))
        );
    }

    #[test]
    fn delete_group_config_removes_non_default_group() {
        let mut config = PermissionGroupsConfig::default();
        config
            .groups
            .insert("builder".to_owned(), PermissionGroupConfig::default());

        assert_eq!(delete_group_config(&mut config, "builder"), Ok(()));
        assert!(!config.groups.contains_key("builder"));
    }

    #[test]
    fn delete_group_config_rejects_required_group() {
        let mut config = PermissionGroupsConfig::default();

        assert_eq!(
            delete_group_config(&mut config, "op"),
            Err(PermissionGroupEditError::Required("op".to_owned()))
        );
    }

    #[test]
    fn delete_group_config_rejects_default_group() {
        let mut config = PermissionGroupsConfig::default();

        assert_eq!(
            delete_group_config(&mut config, "default"),
            Err(PermissionGroupEditError::Default("default".to_owned()))
        );
    }

    #[test]
    fn delete_group_config_reports_missing_group() {
        let mut config = PermissionGroupsConfig::default();

        assert_eq!(
            delete_group_config(&mut config, "missing"),
            Err(PermissionGroupEditError::Missing("missing".to_owned()))
        );
    }

    #[test]
    fn group_config_unset_is_context_exact() {
        let mut config = PermissionGroupsConfig::default();
        let permission = key("steel.fly");
        let global = PermissionRuleContext::Global;
        let lobby = PermissionRuleContext::domain("lobby");

        set_group_config_permission(
            &mut config,
            "default",
            &permission,
            &global,
            PermissionState::Allow,
        )
        .expect("global permission stores");
        set_group_config_permission(
            &mut config,
            "default",
            &permission,
            &lobby,
            PermissionState::Deny,
        )
        .expect("contextual permission stores");

        assert_eq!(
            unset_group_config_permission(&mut config, "default", &permission, &global),
            Ok(true)
        );
        let default = config.groups.get("default").expect("default group exists");
        assert!(default.allow.is_empty());
        assert_eq!(default.rules.len(), 1);
        assert_eq!(default.rules[0].state, PermissionRuleStateConfig::Deny);
    }

    #[test]
    fn group_config_edit_reports_missing_group() {
        let mut config = PermissionGroupsConfig::default();

        assert_eq!(
            set_group_config_permission(
                &mut config,
                "missing",
                &key("steel.fly"),
                &PermissionRuleContext::Global,
                PermissionState::Allow,
            ),
            Err(PermissionGroupEditError::Missing("missing".to_owned()))
        );
    }

    #[test]
    fn group_permission_suggestions_only_include_manageable_group_rules() {
        let group = PermissionGroupConfig {
            priority: 0,
            allow: vec![
                "steel.fly".to_owned(),
                "minecraft.command.gamemode".to_owned(),
            ],
            deny: vec!["steel.stop".to_owned()],
            rules: vec![PermissionRuleConfig {
                key: "steel.chat".to_owned(),
                state: PermissionRuleStateConfig::Allow,
                context: Some(PermissionRuleContextConfig {
                    domain: Some("lobby".to_owned()),
                    world: None,
                    custom: Vec::new(),
                }),
            }],
            values: Vec::new(),
        };

        assert_eq!(
            suggestion_texts(group_permission_suggestions(
                "steel.",
                &group,
                &TestContext::with_permissions(["steel.permission.manage.steel.*"]),
            )),
            vec!["steel.chat{domain=lobby}", "steel.fly", "steel.stop"]
        );
    }

    #[test]
    fn group_metadata_suggestions_only_include_manageable_group_values() {
        let group = PermissionGroupConfig {
            priority: 0,
            allow: Vec::new(),
            deny: Vec::new(),
            rules: Vec::new(),
            values: vec![
                PermissionValueRuleConfig {
                    key: "plugin:homes".to_owned(),
                    value: PermissionValue::Integer(10),
                    context: None,
                },
                PermissionValueRuleConfig {
                    key: "other:homes".to_owned(),
                    value: PermissionValue::Integer(20),
                    context: None,
                },
            ],
        };

        assert_eq!(
            suggestion_texts(group_metadata_suggestions(
                "",
                &group,
                &TestContext::with_permissions(["steel.permission.metadata.plugin.*"]),
            )),
            vec!["plugin:homes"]
        );
    }
}
