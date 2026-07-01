//! Argument parsers for the `steelperms` command.

use std::collections::BTreeSet;

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};

use crate::command::graph::{
    CommandArgumentClientParser, CommandArgumentParser, CommandParseError, CommandParseErrorKind,
    ParsedArgument, ParsedArguments,
};
use crate::command::parsers::{
    PermissionGroupParser, PermissionRuleExpressionParser, resolve_optional_permission_targets,
};
use crate::command::reader::{CommandReader, StringMode};
use crate::command::requirement::CommandInputContext;
use crate::permission::{
    PermissionContextKey, PermissionMetadataCatalog, PermissionMetadataExpression,
    PermissionSegment,
    parse_permission_value_key,
};

use super::access::{
    assigned_group_suggestions, can_manage_metadata, direct_metadata_override_suggestions,
    direct_permission_override_suggestions, group_metadata_suggestions, group_permission_suggestions,
    metadata_catalog_suggestions,
};
use super::permission_targets;

#[derive(Clone, Copy, Debug)]
pub(super) struct PermissionOverrideParser {
    targets_argument: &'static str,
}

impl PermissionOverrideParser {
    pub(super) const fn new(targets_argument: &'static str) -> Self {
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
        let Some(server) = context.server() else {
            return Vec::new();
        };
        let Ok(targets) = resolve_optional_permission_targets(
            arguments,
            self.targets_argument,
            context,
        ) else {
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
pub(super) struct PermissionAssignedGroupParser {
    targets_argument: &'static str,
}

impl PermissionAssignedGroupParser {
    pub(super) const fn new(targets_argument: &'static str) -> Self {
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
        let Some(server) = context.server() else {
            return Vec::new();
        };
        let Ok(targets) = resolve_optional_permission_targets(
            arguments,
            self.targets_argument,
            context,
        ) else {
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
pub(super) struct PermissionGroupNameParser;

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
pub(super) struct PermissionGroupRuleParser {
    group_argument: &'static str,
}

impl PermissionGroupRuleParser {
    pub(super) const fn new(group_argument: &'static str) -> Self {
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
pub(super) struct PermissionMetadataExpressionParser;

impl CommandArgumentParser for PermissionMetadataExpressionParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let value = reader.read_token()?;
        let expression = PermissionMetadataExpression::parse(value).map_err(|error| {
            CommandParseError::new(
                CommandParseErrorKind::InvalidPermissionMetadataExpression(error.to_string()),
                cursor,
            )
        })?;

        Ok(ParsedArgument::PermissionMetadataExpression(expression))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(
            ArgumentType::String {
                behavior: steel_protocol::packets::game::ArgumentStringTypeBehavior::GreedyPhrase,
            },
            Some(SuggestionType::AskServer),
        )
    }

    fn parsed_type(&self) -> &'static str {
        "permission_metadata_expression"
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

        metadata_expression_suggestions(prefix, catalog, context)
    }
}

fn metadata_expression_suggestions(
    prefix: &str,
    catalog: &PermissionMetadataCatalog,
    context: &dyn CommandInputContext,
) -> Vec<SuggestionEntry> {
    metadata_expression_suggestion_texts(prefix, catalog, context)
        .into_iter()
        .map(SuggestionEntry::new)
        .collect()
}

fn metadata_expression_suggestion_texts(
    prefix: &str,
    catalog: &PermissionMetadataCatalog,
    context: &dyn CommandInputContext,
) -> Vec<String> {
    let Some((metadata_key, context_prefix)) = prefix.split_once('{') else {
        return metadata_catalog_suggestions(prefix, catalog, context)
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect();
    };
    let Ok(parsed_key) = parse_permission_value_key(metadata_key) else {
        return Vec::new();
    };
    if context_prefix.contains('}') || !can_manage_metadata(context, &parsed_key) {
        return Vec::new();
    }

    let (completed_entries, current_entry) =
        context_prefix
            .rsplit_once(',')
            .map_or(("", context_prefix), |(completed, current)| {
                (&context_prefix[..completed.len() + 1], current)
            });
    let expression_prefix = format!("{metadata_key}{{{completed_entries}");
    let completed_keys = completed_metadata_expression_context_keys(completed_entries);

    let Some((context_key, value_prefix)) = current_entry.split_once('=') else {
        return metadata_expression_context_key_suggestions(
            &expression_prefix,
            current_entry,
            &completed_keys,
            context,
        );
    };
    if completed_keys.contains(context_key) {
        return Vec::new();
    }

    metadata_expression_context_value_suggestions(
        &expression_prefix,
        context_key,
        value_prefix,
        context,
    )
}

fn completed_metadata_expression_context_keys(completed_entries: &str) -> BTreeSet<String> {
    completed_entries
        .trim_end_matches(',')
        .split(',')
        .filter_map(|entry| {
            let (key, _) = entry.split_once('=')?;
            (!key.is_empty()).then(|| key.to_owned())
        })
        .collect()
}

fn metadata_expression_context_key_suggestions(
    expression_prefix: &str,
    key_prefix: &str,
    completed_keys: &BTreeSet<String>,
    context: &dyn CommandInputContext,
) -> Vec<String> {
    let mut suggestions = ["domain", "world"]
        .into_iter()
        .filter(|key| !completed_keys.contains(*key))
        .filter(|key| key.starts_with(key_prefix))
        .map(|key| format!("{expression_prefix}{key}="))
        .collect::<Vec<_>>();

    if let Some(catalog) = context.permission_context_catalog() {
        suggestions.extend(
            catalog
                .key_suggestions(key_prefix)
                .into_iter()
                .filter(|key| !completed_keys.contains(key))
                .map(|key| format!("{expression_prefix}{key}=")),
        );
    }
    suggestions.sort();
    suggestions.dedup();
    suggestions
}

fn metadata_expression_context_value_suggestions(
    expression_prefix: &str,
    context_key: &str,
    value_prefix: &str,
    context: &dyn CommandInputContext,
) -> Vec<String> {
    match context_key {
        "domain" => metadata_expression_domain_value_suggestions(
            expression_prefix,
            context_key,
            value_prefix,
            context,
        ),
        "world" => metadata_expression_world_value_suggestions(
            expression_prefix,
            context_key,
            value_prefix,
            context,
        ),
        custom_key => metadata_expression_custom_context_value_suggestions(
            expression_prefix,
            custom_key,
            value_prefix,
            context,
        ),
    }
}

fn metadata_expression_domain_value_suggestions(
    expression_prefix: &str,
    context_key: &str,
    value_prefix: &str,
    context: &dyn CommandInputContext,
) -> Vec<String> {
    let Some(server) = context.server() else {
        return Vec::new();
    };

    server
        .worlds
        .domain_names()
        .filter(|domain| domain.starts_with(value_prefix))
        .map(|domain| format!("{expression_prefix}{context_key}={domain}}}"))
        .collect()
}

fn metadata_expression_world_value_suggestions(
    expression_prefix: &str,
    context_key: &str,
    value_prefix: &str,
    context: &dyn CommandInputContext,
) -> Vec<String> {
    let Some(server) = context.server() else {
        return Vec::new();
    };

    server
        .worlds
        .keys()
        .map(ToString::to_string)
        .filter(|world| world.starts_with(value_prefix))
        .map(|world| format!("{expression_prefix}{context_key}={world}}}"))
        .collect()
}

fn metadata_expression_custom_context_value_suggestions(
    expression_prefix: &str,
    context_key: &str,
    value_prefix: &str,
    context: &dyn CommandInputContext,
) -> Vec<String> {
    let Ok(context_key) = PermissionContextKey::parse(context_key) else {
        return Vec::new();
    };
    let Some(catalog) = context.permission_context_catalog() else {
        return Vec::new();
    };

    catalog
        .value_suggestions(&context_key, value_prefix)
        .into_iter()
        .map(|value| format!("{expression_prefix}{}={value}}}", context_key.as_str()))
        .collect()
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PermissionMetadataOverrideParser {
    targets_argument: &'static str,
}

impl PermissionMetadataOverrideParser {
    pub(super) const fn new(targets_argument: &'static str) -> Self {
        Self { targets_argument }
    }
}

impl CommandArgumentParser for PermissionMetadataOverrideParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        PermissionMetadataExpressionParser.parse(reader, context)
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        PermissionMetadataExpressionParser.client_parser()
    }

    fn parsed_type(&self) -> &'static str {
        PermissionMetadataExpressionParser.parsed_type()
    }

    fn suggest(
        &self,
        prefix: &str,
        arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let Some(server) = context.server() else {
            return Vec::new();
        };
        let Ok(targets) = resolve_optional_permission_targets(
            arguments,
            self.targets_argument,
            context,
        ) else {
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
pub(super) struct PermissionGroupMetadataParser {
    group_argument: &'static str,
}

impl PermissionGroupMetadataParser {
    pub(super) const fn new(group_argument: &'static str) -> Self {
        Self { group_argument }
    }
}

impl CommandArgumentParser for PermissionGroupMetadataParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        PermissionMetadataExpressionParser.parse(reader, context)
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        PermissionMetadataExpressionParser.client_parser()
    }

    fn parsed_type(&self) -> &'static str {
        PermissionMetadataExpressionParser.parsed_type()
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
