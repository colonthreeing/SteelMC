//! Argument parsers for the `steelperms` command.

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};

use crate::command::graph::{
    CommandArgumentClientParser, CommandArgumentParser, CommandParseError, CommandParseErrorKind,
    ParsedArgument, ParsedArguments, PermissionTarget,
};
use crate::command::parsers::{PermissionGroupParser, PermissionRuleExpressionParser};
use crate::command::reader::{CommandReader, StringMode};
use crate::command::requirement::CommandInputContext;
use crate::permission::{PermissionContextKey, PermissionSegment, parse_permission_value_key};

use super::access::{
    assigned_group_suggestions, direct_metadata_override_suggestions,
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
pub(super) struct PermissionContextKeyParser;

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
pub(super) struct PermissionContextValueParser {
    key_argument: &'static str,
}

impl PermissionContextValueParser {
    pub(super) const fn new(key_argument: &'static str) -> Self {
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
pub(super) struct PermissionMetadataKeyParser;

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
