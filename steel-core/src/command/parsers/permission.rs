//! Permission command argument parsers.

use std::collections::BTreeSet;

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};

use crate::{
    command::{
        graph::{
            CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
            CommandParseErrorKind, ParsedArgument, ParsedArguments,
        },
        reader::{CommandReader, StringMode},
        requirement::CommandInputContext,
    },
    permission::{PermissionContextKey, PermissionKey, PermissionRuleExpression},
};

/// Permission key argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct PermissionKeyParser;

impl CommandArgumentParser for PermissionKeyParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let value = reader.read_token()?;
        let permission = PermissionKey::parse(value.clone()).map_err(|_| {
            CommandParseError::new(CommandParseErrorKind::InvalidPermissionKey(value), cursor)
        })?;

        Ok(ParsedArgument::PermissionKey(permission))
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
        "permission_key"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let Some(catalog) = context.permission_catalog() else {
            return Vec::new();
        };

        catalog
            .suggestions(prefix)
            .into_iter()
            .map(SuggestionEntry::new)
            .collect()
    }
}

/// Permission rule expression argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct PermissionRuleExpressionParser;

impl CommandArgumentParser for PermissionRuleExpressionParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let value = reader.read_token()?;
        let expression = PermissionRuleExpression::parse(value).map_err(|error| {
            CommandParseError::new(
                CommandParseErrorKind::InvalidPermissionExpression(error.to_string()),
                cursor,
            )
        })?;

        Ok(ParsedArgument::PermissionRuleExpression(expression))
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
        "permission_rule_expression"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        permission_rule_expression_suggestions(prefix, context)
            .into_iter()
            .map(SuggestionEntry::new)
            .collect()
    }
}

fn permission_rule_expression_suggestions(
    prefix: &str,
    context: &dyn CommandInputContext,
) -> Vec<String> {
    let Some((permission_key, context_prefix)) = prefix.split_once('{') else {
        let Some(catalog) = context.permission_catalog() else {
            return Vec::new();
        };
        return catalog.suggestions(prefix);
    };
    if context_prefix.contains('}') || PermissionKey::parse(permission_key).is_err() {
        return Vec::new();
    }

    let (completed_entries, current_entry) = context_prefix
        .rsplit_once(',')
        .map_or(("", context_prefix), |(completed, current)| {
            (&context_prefix[..completed.len() + 1], current)
        });
    let expression_prefix = format!("{permission_key}{{{completed_entries}");
    let completed_keys = completed_permission_rule_context_keys(completed_entries);

    let Some((context_key, value_prefix)) = current_entry.split_once('=') else {
        return permission_rule_context_key_suggestions(
            &expression_prefix,
            current_entry,
            &completed_keys,
            context,
        );
    };
    if completed_keys.contains(context_key) {
        return Vec::new();
    }

    permission_rule_context_value_suggestions(
        &expression_prefix,
        context_key,
        value_prefix,
        context,
    )
}

fn completed_permission_rule_context_keys(completed_entries: &str) -> BTreeSet<String> {
    completed_entries
        .trim_end_matches(',')
        .split(',')
        .filter_map(|entry| {
            let (key, _) = entry.split_once('=')?;
            (!key.is_empty()).then(|| key.to_owned())
        })
        .collect()
}

fn permission_rule_context_key_suggestions(
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

fn permission_rule_context_value_suggestions(
    expression_prefix: &str,
    context_key: &str,
    value_prefix: &str,
    context: &dyn CommandInputContext,
) -> Vec<String> {
    match context_key {
        "domain" => permission_rule_domain_value_suggestions(
            expression_prefix,
            context_key,
            value_prefix,
            context,
        ),
        "world" => permission_rule_world_value_suggestions(
            expression_prefix,
            context_key,
            value_prefix,
            context,
        ),
        custom_key => permission_rule_custom_context_value_suggestions(
            expression_prefix,
            custom_key,
            value_prefix,
            context,
        ),
    }
}

fn permission_rule_domain_value_suggestions(
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

fn permission_rule_world_value_suggestions(
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

fn permission_rule_custom_context_value_suggestions(
    expression_prefix: &str,
    context_key: &str,
    value_prefix: &str,
    context: &dyn CommandInputContext,
) -> Vec<String> {
    let Some(catalog) = context.permission_context_catalog() else {
        return Vec::new();
    };
    let Ok(key) = PermissionContextKey::parse(context_key.to_owned()) else {
        return Vec::new();
    };

    catalog
        .value_suggestions(&key, value_prefix)
        .into_iter()
        .map(|value| format!("{expression_prefix}{context_key}={value}}}"))
        .collect()
}

/// Permission group name argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct PermissionGroupParser;

impl CommandArgumentParser for PermissionGroupParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let value = reader.read_string(StringMode::SingleWord)?;
        let Some(server) = context.server() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("server"),
                cursor,
            ));
        };
        if !server.permission_groups.contains_group(&value) {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidPermissionGroup(value),
                cursor,
            ));
        }

        Ok(ParsedArgument::String(value))
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
