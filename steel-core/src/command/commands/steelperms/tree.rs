//! Command tree shape for the `steelperms` command.

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    BoolParser, CommandArgumentParser, CommandNodeBuilder, CommandResult, IntegerParser,
    LongParser, ParsedArguments, StringParser, argument, literal,
};
use crate::command::parsers::{
    DomainParser, PermissionGroupParser, PermissionRuleExpressionParser, PermissionTargetParser,
    WorldParser,
};
use crate::command::reader::StringMode;

use super::parsers::{
    PermissionAssignedGroupParser, PermissionContextKeyParser, PermissionContextValueParser,
    PermissionGroupMetadataParser, PermissionGroupNameParser, PermissionGroupRuleParser,
    PermissionMetadataKeyParser, PermissionMetadataOverrideParser, PermissionOverrideParser,
};
use super::{
    add_default_group, add_group, allow_group_permission, allow_permission, check_metadata,
    check_permission, create_group, delete_group, deny_group_permission, deny_permission, group_info,
    group_list, remove_default_group, remove_group, set_group_metadata, set_group_priority,
    set_metadata, unset_group_metadata, unset_group_permission, unset_metadata, unset_permission,
    user_info,
};

pub(super) fn command() -> CommandNodeBuilder {
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
