//! Item predicate command argument parser.

use simdnbt::owned::NbtTag;
use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use steel_registry::{REGISTRY, RegistryExt, TaggedRegistryExt};
use steel_utils::{Identifier, nbt::parse_snbt_argument};

use crate::command::{
    graph::{
        CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
        CommandParseErrorKind, ItemPredicateArgumentValue, ItemPredicateCondition,
        ItemPredicateTarget, ItemPredicateTerm, ParsedArgument, ParsedArguments,
    },
    parsers::parse_resource_identifier,
    reader::CommandReader,
    requirement::CommandInputContext,
    suggestions::matches_suggestion_substr,
};

/// Item predicate argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct ItemPredicateParser;

impl CommandArgumentParser for ItemPredicateParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let start = reader.absolute_cursor();
        let mut parser = ItemPredicateSyntax::new(reader.remaining());
        let value = parser.parse().map_err(|error| {
            CommandParseError::new(
                CommandParseErrorKind::InvalidItemPredicate(error.message),
                start + error.cursor,
            )
        })?;

        advance_reader(reader, parser.cursor);
        Ok(ParsedArgument::ItemPredicate(value))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(
            ArgumentType::ItemPredicate,
            Some(SuggestionType::AskServer),
        )
    }

    fn parsed_type(&self) -> &'static str {
        "item_predicate"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        if let Some((base, component_prefix)) = component_suggestion_prefix(prefix) {
            return component_suggestions(base, component_prefix);
        }

        if prefix.contains('[') {
            return Vec::new();
        }

        if let Some(tag_prefix) = prefix.strip_prefix('#') {
            let stripped_tag_prefix = tag_prefix.strip_prefix("minecraft:").unwrap_or(tag_prefix);
            return REGISTRY
                .items
                .tag_keys()
                .filter(|key| {
                    let key = key.to_string();
                    let text = key.strip_prefix("minecraft:").unwrap_or(&key);
                    matches_suggestion_substr(stripped_tag_prefix, text)
                })
                .map(|key| SuggestionEntry::new(format!("#{key}")))
                .collect();
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

        if "*".starts_with(prefix) {
            suggestions.push(SuggestionEntry::new("*"));
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

fn component_suggestion_prefix(prefix: &str) -> Option<(&str, &str)> {
    let _ = prefix.rfind('[')?;
    let mut start = 0;
    for (index, ch) in prefix.char_indices() {
        if matches!(ch, '[' | ',' | '|' | '!') {
            start = index + ch.len_utf8();
        }
    }

    let segment = &prefix[start..];
    if segment.contains(['=', '~', ']']) {
        return None;
    }

    let trimmed = segment.trim_start();
    let base_end = start + segment.len() - trimmed.len();
    Some((&prefix[..base_end], trimmed))
}

fn component_suggestions(base: &str, prefix: &str) -> Vec<SuggestionEntry> {
    let stripped_prefix = prefix.strip_prefix("minecraft:").unwrap_or(prefix);
    let mut suggestions = data_component_keys()
        .filter(|key| {
            let text = key.strip_prefix("minecraft:").unwrap_or(key);
            matches_suggestion_substr(stripped_prefix, text)
        })
        .map(|key| SuggestionEntry::new(format!("{base}{key}")))
        .collect::<Vec<_>>();

    let count = count_key().to_string();
    let count_text = count.strip_prefix("minecraft:").unwrap_or(&count);
    if matches_suggestion_substr(stripped_prefix, count_text) {
        suggestions.push(SuggestionEntry::new(format!("{base}{count}")));
    }

    suggestions
}

fn data_component_keys() -> impl Iterator<Item = String> {
    (0..REGISTRY.data_components.len())
        .filter_map(|id| REGISTRY.data_components.by_id(id))
        .map(|component| component.key.to_string())
}

struct ItemPredicateSyntax<'a> {
    input: &'a str,
    cursor: usize,
}

impl<'a> ItemPredicateSyntax<'a> {
    const fn new(input: &'a str) -> Self {
        Self { input, cursor: 0 }
    }

    fn parse(&mut self) -> Result<ItemPredicateArgumentValue, ItemPredicateParseError> {
        let target = self.parse_target()?;
        let conditions = if self.has_bracket_after_whitespace() {
            self.expect_raw('[')?;
            if self.peek_after_whitespace() == Some(']') {
                self.expect_raw(']')?;
                Vec::new()
            } else {
                let conditions = self.parse_conditions()?;
                self.expect_raw(']')?;
                conditions
            }
        } else {
            Vec::new()
        };

        Ok(ItemPredicateArgumentValue::new(target, conditions))
    }

    fn parse_target(&mut self) -> Result<ItemPredicateTarget, ItemPredicateParseError> {
        self.skip_whitespace();

        if self.consume_raw('*') {
            return Ok(ItemPredicateTarget::Any);
        }

        if self.consume_raw('#') {
            return self.parse_tag_target();
        }

        let item_cursor = self.cursor;
        let key = self.read_identifier()?;
        let Some(item) = REGISTRY.items.by_key(&key) else {
            return Err(self.error_at(item_cursor, format!("unknown item '{key}'")));
        };

        Ok(ItemPredicateTarget::Item(item))
    }

    fn parse_tag_target(&mut self) -> Result<ItemPredicateTarget, ItemPredicateParseError> {
        let tag_cursor = self.cursor.saturating_sub(1);
        let key = self.read_identifier()?;
        let Some(items) = REGISTRY.items.get_tag(&key) else {
            return Err(self.error_at(tag_cursor, format!("unknown item tag '#{key}'")));
        };

        Ok(ItemPredicateTarget::Tag { key, items })
    }

    fn parse_conditions(&mut self) -> Result<Vec<ItemPredicateCondition>, ItemPredicateParseError> {
        let mut conditions = Vec::new();

        loop {
            conditions.push(ItemPredicateCondition::new(self.parse_alternatives()?));
            if !self.consume_raw(',') {
                return Ok(conditions);
            }
        }
    }

    fn parse_alternatives(&mut self) -> Result<Vec<ItemPredicateTerm>, ItemPredicateParseError> {
        let mut alternatives = Vec::new();

        loop {
            alternatives.push(self.parse_term()?);
            if !self.consume_raw('|') {
                return Ok(alternatives);
            }
        }
    }

    fn parse_term(&mut self) -> Result<ItemPredicateTerm, ItemPredicateParseError> {
        if self.consume_raw('!') {
            return Ok(ItemPredicateTerm::Not(Box::new(self.parse_test()?)));
        }

        self.parse_test()
    }

    fn parse_test(&mut self) -> Result<ItemPredicateTerm, ItemPredicateParseError> {
        let key_cursor = self.cursor_after_whitespace();
        let key = self.read_identifier()?;

        if self.consume_raw('=') {
            self.validate_component_key(key_cursor, &key)?;
            return Ok(ItemPredicateTerm::ComponentValue {
                key,
                value: self.read_nbt("component")?,
            });
        }

        if self.consume_raw('~') {
            self.validate_predicate_key(key_cursor, &key)?;
            let value = self.read_nbt("predicate")?;
            if is_component_existence_predicate_key(&key) {
                if !is_empty_compound(&value) {
                    return Err(self.error_at(
                        key_cursor,
                        format!(
                            "item component existence predicate '{key}' requires an empty compound"
                        ),
                    ));
                }
                return Ok(ItemPredicateTerm::ComponentPresence { key });
            }

            return Ok(ItemPredicateTerm::PredicateValue { key, value });
        }

        self.validate_component_key(key_cursor, &key)?;
        Ok(ItemPredicateTerm::ComponentPresence { key })
    }

    fn validate_component_key(
        &self,
        cursor: usize,
        key: &Identifier,
    ) -> Result<(), ItemPredicateParseError> {
        if is_count_key(key) || REGISTRY.data_components.by_key(key).is_some() {
            return Ok(());
        }

        Err(self.error_at(cursor, format!("unknown item component '{key}'")))
    }

    fn validate_predicate_key(
        &self,
        cursor: usize,
        key: &Identifier,
    ) -> Result<(), ItemPredicateParseError> {
        if is_count_key(key)
            || is_vanilla_data_component_predicate_key(key)
            || REGISTRY.data_components.by_key(key).is_some()
        {
            return Ok(());
        }

        Err(self.error_at(cursor, format!("unknown item predicate '{key}'")))
    }

    fn read_nbt(&mut self, description: &str) -> Result<NbtTag, ItemPredicateParseError> {
        let value_cursor = self.cursor;
        let (nbt, consumed) = parse_snbt_argument(&self.input[self.cursor..]).map_err(|error| {
            self.error_at(
                value_cursor + error.cursor(),
                format!("invalid {description} value: {}", error.message()),
            )
        })?;
        self.cursor += consumed;
        Ok(nbt)
    }

    fn read_identifier(&mut self) -> Result<Identifier, ItemPredicateParseError> {
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

    fn has_bracket_after_whitespace(&self) -> bool {
        self.peek_after_whitespace() == Some('[')
    }

    fn peek_after_whitespace(&self) -> Option<char> {
        let mut cursor = self.cursor;
        while self.input[cursor..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
        {
            let ch = self.input[cursor..].chars().next()?;
            cursor += ch.len_utf8();
        }
        self.input[cursor..].chars().next()
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
        self.skip_whitespace();
        if self.peek() == Some(expected) {
            self.read();
            return true;
        }

        false
    }

    fn expect_raw(&mut self, expected: char) -> Result<(), ItemPredicateParseError> {
        if self.consume_raw(expected) {
            return Ok(());
        }

        Err(self.error(format!("expected '{expected}'")))
    }

    fn error(&self, message: impl Into<String>) -> ItemPredicateParseError {
        self.error_at(self.cursor, message)
    }

    fn error_at(&self, cursor: usize, message: impl Into<String>) -> ItemPredicateParseError {
        ItemPredicateParseError {
            cursor,
            message: message.into(),
        }
    }
}

fn count_key() -> Identifier {
    Identifier::vanilla_static("count")
}

fn is_count_key(key: &Identifier) -> bool {
    key.namespace == Identifier::VANILLA_NAMESPACE && key.path == "count"
}

fn is_component_existence_predicate_key(key: &Identifier) -> bool {
    !is_count_key(key)
        && !is_vanilla_data_component_predicate_key(key)
        && REGISTRY.data_components.by_key(key).is_some()
}

fn is_vanilla_data_component_predicate_key(key: &Identifier) -> bool {
    key.namespace == Identifier::VANILLA_NAMESPACE
        && matches!(
            &*key.path,
            "damage"
                | "enchantments"
                | "stored_enchantments"
                | "potion_contents"
                | "custom_data"
                | "container"
                | "bundle_contents"
                | "firework_explosion"
                | "fireworks"
                | "writable_book_content"
                | "written_book_content"
                | "attribute_modifiers"
                | "trim"
                | "jukebox_playable"
                | "villager/variant"
        )
}

fn is_empty_compound(value: &NbtTag) -> bool {
    matches!(value, NbtTag::Compound(compound) if compound.is_empty())
}

#[derive(Debug)]
struct ItemPredicateParseError {
    cursor: usize,
    message: String,
}
