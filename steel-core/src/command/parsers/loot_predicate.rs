//! Loot predicate command argument parser.

use steel_protocol::packets::game::{ArgumentType, SuggestionType};
use steel_registry::loot_table::RuntimeLootCondition;
use steel_utils::nbt::parse_snbt_argument;

use crate::command::{
    graph::{
        CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
        CommandParseErrorKind, LootPredicateArgumentValue, ParsedArgument,
    },
    parsers::parse_resource_identifier,
    reader::CommandReader,
    requirement::CommandInputContext,
};

/// Vanilla loot predicate resource-or-inline argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct LootPredicateParser;

impl CommandArgumentParser for LootPredicateParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let start = reader.absolute_cursor();
        let value = if starts_inline_snbt(reader.remaining()) {
            let (tag, consumed) = parse_snbt_argument(reader.remaining()).map_err(|error| {
                CommandParseError::new(
                    CommandParseErrorKind::InvalidLootPredicate(error.message().to_owned()),
                    start + error.cursor(),
                )
            })?;
            let condition = RuntimeLootCondition::decode(&tag).map_err(|error| {
                CommandParseError::new(
                    CommandParseErrorKind::InvalidLootPredicate(error.to_string()),
                    start,
                )
            })?;
            advance_reader(reader, consumed);
            LootPredicateArgumentValue::Inline(condition)
        } else {
            let raw = reader.read_token()?;
            let Some(identifier) = parse_resource_identifier(&raw) else {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidLootPredicate(raw),
                    start,
                ));
            };
            LootPredicateArgumentValue::Reference(identifier)
        };

        Ok(ParsedArgument::LootPredicate(value))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(
            ArgumentType::LootPredicate,
            Some(SuggestionType::AskServer),
        )
    }

    fn parsed_type(&self) -> &'static str {
        "loot_predicate"
    }

    fn examples(&self) -> &'static [&'static str] {
        &[
            "foo",
            "foo:bar",
            "{condition:\"minecraft:killed_by_player\"}",
            "[{condition:\"minecraft:killed_by_player\"}]",
        ]
    }
}

fn starts_inline_snbt(input: &str) -> bool {
    let Some(ch) = input.chars().next() else {
        return false;
    };
    matches!(ch, '{' | '[')
}

fn advance_reader(reader: &mut CommandReader<'_>, bytes: usize) {
    let Some(consumed) = reader.remaining().get(..bytes) else {
        return;
    };
    let char_count = consumed.chars().count();
    for _ in 0..char_count {
        reader.read();
    }
}

#[cfg(test)]
mod tests {
    use super::starts_inline_snbt;

    #[test]
    fn inline_detection_does_not_treat_identifiers_as_snbt_strings() {
        assert!(!starts_inline_snbt("minecraft:test"));
        assert!(!starts_inline_snbt("test"));
        assert!(starts_inline_snbt(
            "{condition:\"minecraft:killed_by_player\"}"
        ));
        assert!(starts_inline_snbt(
            "[{condition:\"minecraft:killed_by_player\"}]"
        ));
        assert!(!starts_inline_snbt("truefoo"));
        assert!(!starts_inline_snbt("true"));
        assert!(!starts_inline_snbt("1"));
    }
}
