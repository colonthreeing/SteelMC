//! Item stack command argument parser.

use std::{collections::HashSet, io::Cursor};

use simdnbt::owned::NbtTag;
use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use steel_registry::{
    REGISTRY, RegistryExt,
    data_components::{
        ComponentData, ComponentDataDiscriminant, ComponentEntry, DataComponentPatch,
        DataComponentType,
    },
    item_stack::ItemStack,
};
use steel_utils::{Identifier, nbt::parse_snbt_argument};

use crate::command::{
    graph::{
        CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
        CommandParseErrorKind, ParsedArgument, ParsedArguments,
    },
    parsers::parse_resource_identifier,
    reader::CommandReader,
    requirement::CommandInputContext,
    suggestions::matches_suggestion_substr,
};

/// Vanilla item stack argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct ItemStackParser;

impl CommandArgumentParser for ItemStackParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let start = reader.absolute_cursor();
        let mut parser = ItemStackSyntax::new(reader.remaining());
        let value = parser.parse().map_err(|error| {
            CommandParseError::new(
                CommandParseErrorKind::InvalidItemStack(error.message),
                start + error.cursor,
            )
        })?;

        advance_reader(reader, parser.cursor);
        Ok(ParsedArgument::ItemStack(value))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::ItemStack, Some(SuggestionType::AskServer))
    }

    fn parsed_type(&self) -> &'static str {
        "item_stack"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        if let Some((base, component_prefix, removed)) = component_suggestion_prefix(prefix) {
            let mut suggestions = component_suggestions(base, component_prefix, removed);
            if !removed && component_prefix.is_empty() {
                suggestions.push(SuggestionEntry::new(format!("{base}!")));
            }
            return suggestions;
        }

        if prefix.contains('[') {
            return Vec::new();
        }

        let stripped_prefix = prefix.strip_prefix("minecraft:").unwrap_or(prefix);
        let mut suggestions = REGISTRY
            .items
            .iter()
            .map(|(_, item)| SuggestionEntry::new(item.key.to_string()))
            .filter(|suggestion| {
                let text = suggestion
                    .text
                    .strip_prefix("minecraft:")
                    .unwrap_or(&suggestion.text);
                matches_suggestion_substr(stripped_prefix, text)
            })
            .collect::<Vec<_>>();

        if parse_resource_identifier(prefix)
            .is_some_and(|key| REGISTRY.items.by_key(&key).is_some())
        {
            suggestions.push(SuggestionEntry::new(format!("{prefix}[")));
        }

        suggestions
    }
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

fn component_suggestion_prefix(prefix: &str) -> Option<(&str, &str, bool)> {
    let _ = prefix.rfind('[')?;
    let mut start = 0;
    let mut removed = false;
    for (index, ch) in prefix.char_indices() {
        if matches!(ch, '[' | ',') {
            start = index + ch.len_utf8();
            removed = false;
        } else if ch == '!' {
            start = index + ch.len_utf8();
            removed = true;
        }
    }

    let segment = &prefix[start..];
    if segment.contains(['=', ']']) {
        return None;
    }

    let trimmed = segment.trim_start();
    let base_end = start + segment.len() - trimmed.len();
    Some((&prefix[..base_end], trimmed, removed))
}

fn component_suggestions(base: &str, prefix: &str, removed: bool) -> Vec<SuggestionEntry> {
    let stripped_prefix = prefix.strip_prefix("minecraft:").unwrap_or(prefix);
    (0..REGISTRY.data_components.len())
        .filter_map(|id| REGISTRY.data_components.by_id(id))
        .filter(|component| {
            let key = component.key.to_string();
            let text = key.strip_prefix("minecraft:").unwrap_or(&key);
            matches_suggestion_substr(stripped_prefix, text)
        })
        .map(|component| {
            let suffix = if removed { "" } else { "=" };
            SuggestionEntry::new(format!("{base}{}{suffix}", component.key))
        })
        .collect()
}

struct ItemStackSyntax<'a> {
    input: &'a str,
    cursor: usize,
}

impl<'a> ItemStackSyntax<'a> {
    const fn new(input: &'a str) -> Self {
        Self { input, cursor: 0 }
    }

    fn parse(&mut self) -> Result<ItemStack, ItemStackParseError> {
        let item_cursor = self.cursor;
        let item_key = self.read_identifier()?;
        let Some(item) = REGISTRY.items.by_key(&item_key) else {
            return Err(self.error_at(item_cursor, format!("unknown item '{item_key}'")));
        };

        let patch = if self.peek() == Some('[') {
            self.parse_components()?
        } else {
            DataComponentPatch::new()
        };

        Ok(ItemStack::with_count_and_patch(item, 1, patch))
    }

    fn parse_components(&mut self) -> Result<DataComponentPatch, ItemStackParseError> {
        self.expect_raw('[')?;
        let mut patch = DataComponentPatch::new();
        let mut seen = HashSet::new();

        self.skip_whitespace();
        while self.peek().is_some_and(|ch| ch != ']') {
            let removed = self.consume_raw('!');
            let key_cursor = self.cursor_after_whitespace();
            let key = self.read_identifier()?;
            let Some(entry) = REGISTRY.data_components.by_key(&key) else {
                return Err(self.error_at(key_cursor, format!("unknown item component '{key}'")));
            };

            if !seen.insert(key.clone()) {
                return Err(self.error_at(key_cursor, format!("repeated item component '{key}'")));
            }

            if removed {
                patch.remove(DataComponentType::<()>::new(key));
            } else {
                if entry.expected_discriminant == ComponentDataDiscriminant::Todo {
                    return Err(
                        self.error_at(key_cursor, format!("unsupported item component '{key}'"))
                    );
                }
                self.expect_raw('=')?;
                let value_cursor = self.cursor_after_whitespace();
                let value = self.read_nbt(&key)?;
                let data = decode_component_value(entry, &key, value).ok_or_else(|| {
                    self.error_at(value_cursor, format!("malformed item component '{key}'"))
                })?;
                if !patch.set_raw(key, data) {
                    return Err(self.error_at(
                        value_cursor,
                        "component value did not match the registered component type",
                    ));
                }
            }

            self.skip_whitespace();
            if !self.consume_raw(',') {
                break;
            }
            self.skip_whitespace();
            if !self.can_read() {
                return Err(self.error("expected item component"));
            }
        }

        self.expect_raw(']')?;
        Ok(patch)
    }

    fn read_nbt(&mut self, key: &Identifier) -> Result<NbtTag, ItemStackParseError> {
        let value_cursor = self.cursor;
        let (nbt, consumed) = parse_snbt_argument(&self.input[self.cursor..]).map_err(|error| {
            self.error_at(
                value_cursor + error.cursor(),
                format!("invalid item component '{key}' value: {}", error.message()),
            )
        })?;
        self.cursor += consumed;
        Ok(nbt)
    }

    fn read_identifier(&mut self) -> Result<Identifier, ItemStackParseError> {
        self.skip_whitespace();
        let start = self.cursor;
        while self
            .peek()
            .is_some_and(|ch| Identifier::valid_char(ch) || ch == ':')
        {
            self.read();
        }
        if self.cursor == start {
            return Err(self.error_at(start, "expected identifier"));
        }

        let raw = &self.input[start..self.cursor];
        parse_resource_identifier(raw)
            .ok_or_else(|| self.error_at(start, format!("invalid identifier '{raw}'")))
    }

    fn can_read(&self) -> bool {
        self.cursor < self.input.len()
    }

    fn cursor_after_whitespace(&self) -> usize {
        let mut cursor = self.cursor;
        while self.input[cursor..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
        {
            let Some(ch) = self.input[cursor..].chars().next() else {
                break;
            };
            cursor += ch.len_utf8();
        }
        cursor
    }

    fn peek(&self) -> Option<char> {
        self.input[self.cursor..].chars().next()
    }

    fn read(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.cursor += ch.len_utf8();
        Some(ch)
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.read();
        }
    }

    fn consume_raw(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.read();
            return true;
        }

        false
    }

    fn expect_raw(&mut self, expected: char) -> Result<(), ItemStackParseError> {
        self.skip_whitespace();
        if self.consume_raw(expected) {
            return Ok(());
        }

        Err(self.error(format!("expected '{expected}'")))
    }

    fn error(&self, message: impl Into<String>) -> ItemStackParseError {
        self.error_at(self.cursor, message)
    }

    fn error_at(&self, cursor: usize, message: impl Into<String>) -> ItemStackParseError {
        ItemStackParseError {
            cursor,
            message: message.into(),
        }
    }
}

fn decode_component_value(
    entry: &ComponentEntry,
    key: &Identifier,
    value: NbtTag,
) -> Option<ComponentData> {
    let mut bytes = Vec::new();
    value.write(&mut bytes);
    let mut cursor = Cursor::new(bytes.as_slice());
    let borrowed = simdnbt::borrow::read_tag(&mut cursor).ok()?;
    let data = (entry.nbt_reader)(borrowed.as_tag())?;
    if entry.validates(&data) {
        Some(data)
    } else {
        log::warn!("item component parser decoded {key} with mismatched component data");
        None
    }
}

#[derive(Debug)]
struct ItemStackParseError {
    cursor: usize,
    message: String,
}
