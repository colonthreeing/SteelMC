//! Block command argument parsers.

use simdnbt::owned::NbtCompound;
use steel_protocol::packets::game::{ArgumentType, SuggestionEntry};
use steel_registry::{REGISTRY, RegistryExt, TaggedRegistryExt, blocks::BlockRef};
use steel_utils::{BlockStateId, Identifier, nbt::parse_snbt_compound_argument};

use crate::command::{
    graph::{
        CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
        CommandParseErrorKind, ParsedArgument, ParsedArguments,
    },
    reader::{CommandReader, StringMode},
    requirement::CommandInputContext,
};

/// Block predicate command argument value.
#[derive(Clone, Debug)]
pub enum BlockPredicateArgumentValue {
    /// A concrete block plus the explicitly selected state properties.
    Block {
        /// Block type to match.
        block: BlockRef,
        /// Block state after applying selected properties over the block default.
        state: BlockStateId,
        /// Explicitly selected properties.
        properties: Vec<(String, String)>,
        /// Optional block entity NBT predicate.
        nbt: Option<NbtCompound>,
    },
    /// A block tag plus vague properties validated against each matched block.
    Tag {
        /// Tag key without the leading `#`.
        key: Identifier,
        /// Blocks in the tag.
        blocks: Vec<BlockRef>,
        /// Properties to test against each matched block.
        properties: Vec<(String, String)>,
        /// Optional block entity NBT predicate.
        nbt: Option<NbtCompound>,
    },
}

impl BlockPredicateArgumentValue {
    /// Returns the optional block entity NBT predicate.
    #[must_use]
    pub const fn nbt(&self) -> Option<&NbtCompound> {
        match self {
            Self::Block { nbt, .. } | Self::Tag { nbt, .. } => nbt.as_ref(),
        }
    }

    /// Returns whether this predicate has an NBT component.
    #[must_use]
    pub const fn requires_nbt(&self) -> bool {
        self.nbt().is_some()
    }

    /// Returns whether this predicate matches the given block state, ignoring NBT.
    #[must_use]
    pub fn matches_state(&self, state: BlockStateId) -> bool {
        match self {
            Self::Block {
                block, properties, ..
            } => REGISTRY.blocks.by_state_id(state).is_some_and(|actual| {
                actual == *block && state_properties_match(state, properties)
            }),
            Self::Tag {
                blocks, properties, ..
            } => REGISTRY.blocks.by_state_id(state).is_some_and(|actual| {
                blocks.iter().any(|block| *block == actual)
                    && state_properties_match(state, properties)
            }),
        }
    }
}

fn state_properties_match(state: BlockStateId, expected: &[(String, String)]) -> bool {
    if expected.is_empty() {
        return true;
    }

    let properties = REGISTRY.blocks.get_properties(state);
    expected.iter().all(|(name, value)| {
        properties
            .iter()
            .any(|(actual_name, actual_value)| actual_name == name && actual_value == value)
    })
}

/// Block predicate argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct BlockPredicateParser;

impl CommandArgumentParser for BlockPredicateParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let start = reader.absolute_cursor();
        let mut parser = BlockPredicateSyntax::new(reader.remaining());
        let value = parser.parse().map_err(|error| {
            CommandParseError::new(
                CommandParseErrorKind::InvalidBlockPredicate(error.message),
                start + error.cursor,
            )
        })?;

        advance_reader(reader, parser.cursor);
        Ok(ParsedArgument::BlockPredicate(value))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::BlockPredicate, None)
    }

    fn parsed_type(&self) -> &'static str {
        "block_predicate"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        if prefix.contains(['[', '{']) {
            return Vec::new();
        }

        if let Some(tag_prefix) = prefix.strip_prefix('#') {
            let stripped_tag_prefix = tag_prefix.strip_prefix("minecraft:").unwrap_or(tag_prefix);
            return REGISTRY
                .blocks
                .tag_keys()
                .filter(|key| {
                    let key = key.to_string();
                    key.strip_prefix("minecraft:")
                        .unwrap_or(&key)
                        .starts_with(stripped_tag_prefix)
                })
                .map(|key| SuggestionEntry::new(format!("#{key}")))
                .collect();
        }

        let stripped_prefix = prefix.strip_prefix("minecraft:").unwrap_or(prefix);
        REGISTRY
            .blocks
            .iter()
            .map(|(_, block)| block.key.to_string())
            .filter(|key| {
                key.strip_prefix("minecraft:")
                    .unwrap_or(key)
                    .starts_with(stripped_prefix)
            })
            .map(SuggestionEntry::new)
            .collect()
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

struct BlockPredicateSyntax<'a> {
    input: &'a str,
    cursor: usize,
}

impl<'a> BlockPredicateSyntax<'a> {
    const fn new(input: &'a str) -> Self {
        Self { input, cursor: 0 }
    }

    fn parse(&mut self) -> Result<BlockPredicateArgumentValue, BlockPredicateParseError> {
        if self.peek() == Some('#') {
            self.read();
            return self.parse_tag_predicate();
        }

        self.parse_block_predicate()
    }

    fn parse_block_predicate(
        &mut self,
    ) -> Result<BlockPredicateArgumentValue, BlockPredicateParseError> {
        let id_cursor = self.cursor;
        let key = self.read_identifier()?;
        let Some(block) = REGISTRY.blocks.by_key(&key) else {
            return Err(self.error_at(id_cursor, format!("unknown block '{key}'")));
        };

        let properties = if self.peek() == Some('[') {
            self.read_block_properties(block)?
        } else {
            Vec::new()
        };
        let state = REGISTRY
            .blocks
            .state_id_from_block_defaulted_properties(
                block,
                properties
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.as_str())),
            )
            .ok_or_else(|| self.error_at(id_cursor, format!("invalid block state '{key}'")))?;
        let nbt = if self.peek() == Some('{') {
            Some(self.read_nbt()?)
        } else {
            None
        };

        Ok(BlockPredicateArgumentValue::Block {
            block,
            state,
            properties,
            nbt,
        })
    }

    fn parse_tag_predicate(
        &mut self,
    ) -> Result<BlockPredicateArgumentValue, BlockPredicateParseError> {
        let tag_cursor = self.cursor.saturating_sub(1);
        let key = self.read_identifier()?;
        let Some(blocks) = REGISTRY.blocks.get_tag(&key) else {
            return Err(self.error_at(tag_cursor, format!("unknown block tag '#{key}'")));
        };
        let properties = if self.peek() == Some('[') {
            self.read_vague_properties()?
        } else {
            Vec::new()
        };
        let nbt = if self.peek() == Some('{') {
            Some(self.read_nbt()?)
        } else {
            None
        };

        Ok(BlockPredicateArgumentValue::Tag {
            key,
            blocks,
            properties,
            nbt,
        })
    }

    fn read_block_properties(
        &mut self,
        block: BlockRef,
    ) -> Result<Vec<(String, String)>, BlockPredicateParseError> {
        self.expect_raw('[')?;
        let mut properties = Vec::new();
        self.skip_whitespace();

        while self.peek().is_some_and(|ch| ch != ']') {
            self.skip_whitespace();
            let key_cursor = self.cursor;
            let key = self.read_brigadier_string()?;
            if properties.iter().any(|(existing, _)| existing == &key) {
                return Err(self.error_at(key_cursor, format!("duplicate property '{key}'")));
            }
            let Some(property) = block
                .properties
                .iter()
                .copied()
                .find(|property| property.get_name() == key)
            else {
                return Err(self.error_at(
                    key_cursor,
                    format!("unknown property '{}' for block '{}'", key, block.key),
                ));
            };

            self.skip_whitespace();
            self.expect_raw('=')?;
            self.skip_whitespace();
            let value_cursor = self.cursor;
            let value = self.read_brigadier_string()?;
            if !property
                .get_possible_values()
                .iter()
                .any(|possible| *possible == value)
            {
                return Err(self.error_at(
                    value_cursor,
                    format!(
                        "invalid value '{}' for property '{}' on block '{}'",
                        value, key, block.key
                    ),
                ));
            }
            properties.push((key, value));

            self.skip_whitespace();
            if self.consume_raw(',') {
                continue;
            }
            if self.peek() != Some(']') {
                return Err(self.error("expected ',' or ']' in block properties"));
            }
        }

        self.expect_raw(']')?;
        Ok(properties)
    }

    fn read_vague_properties(&mut self) -> Result<Vec<(String, String)>, BlockPredicateParseError> {
        self.expect_raw('[')?;
        let mut properties = Vec::new();
        self.skip_whitespace();

        while self.peek().is_some_and(|ch| ch != ']') {
            self.skip_whitespace();
            let key_cursor = self.cursor;
            let key = self.read_brigadier_string()?;
            if properties.iter().any(|(existing, _)| existing == &key) {
                return Err(self.error_at(key_cursor, format!("duplicate property '{key}'")));
            }

            self.skip_whitespace();
            self.expect_raw('=')?;
            self.skip_whitespace();
            let value = self.read_brigadier_string()?;
            properties.push((key, value));

            self.skip_whitespace();
            if self.consume_raw(',') {
                continue;
            }
            if self.peek() != Some(']') {
                return Err(self.error("expected ',' or ']' in block properties"));
            }
        }

        self.expect_raw(']')?;
        Ok(properties)
    }

    fn read_nbt(&mut self) -> Result<simdnbt::owned::NbtCompound, BlockPredicateParseError> {
        let nbt_cursor = self.cursor;
        let (nbt, consumed) =
            parse_snbt_compound_argument(&self.input[self.cursor..]).map_err(|error| {
                self.error_at(
                    nbt_cursor + error.cursor(),
                    format!("invalid block entity NBT: {}", error.message()),
                )
            })?;
        self.cursor += consumed;
        Ok(nbt)
    }

    fn read_identifier(&mut self) -> Result<Identifier, BlockPredicateParseError> {
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
        parse_identifier(raw)
            .ok_or_else(|| self.error_at(start, format!("invalid identifier '{raw}'")))
    }

    fn read_brigadier_string(&mut self) -> Result<String, BlockPredicateParseError> {
        let mut reader = CommandReader::with_offset(&self.input[self.cursor..], self.cursor);
        let value = reader
            .read_string(StringMode::QuotablePhrase)
            .map_err(|error| {
                self.error_at(
                    error.cursor(),
                    format!("invalid property string: {:?}", error.kind()),
                )
            })?;
        self.cursor += reader.cursor();
        Ok(value)
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

    fn expect_raw(&mut self, expected: char) -> Result<(), BlockPredicateParseError> {
        if self.consume_raw(expected) {
            return Ok(());
        }

        Err(self.error(format!("expected '{expected}'")))
    }

    fn error(&self, message: impl Into<String>) -> BlockPredicateParseError {
        self.error_at(self.cursor, message)
    }

    fn error_at(&self, cursor: usize, message: impl Into<String>) -> BlockPredicateParseError {
        BlockPredicateParseError {
            cursor,
            message: message.into(),
        }
    }
}

fn parse_identifier(raw: &str) -> Option<Identifier> {
    let (namespace, path) = raw
        .split_once(':')
        .map_or((Identifier::VANILLA_NAMESPACE, raw), |(namespace, path)| {
            (namespace, path)
        });
    if namespace.is_empty() || path.is_empty() || raw.matches(':').count() > 1 {
        return None;
    }
    Identifier::validate(namespace, path)
        .then(|| Identifier::new(namespace.to_owned(), path.to_owned()))
}

#[derive(Debug)]
struct BlockPredicateParseError {
    cursor: usize,
    message: String,
}
