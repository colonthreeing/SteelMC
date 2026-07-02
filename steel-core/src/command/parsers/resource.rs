//! Registry resource command argument parsers.

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use steel_registry::{
    REGISTRY, RegistryExt, TaggedRegistryExt, biome::BiomeRef, entity_type::EntityTypeRef,
    structure::StructureRef,
};
use steel_utils::Identifier;

use crate::command::{
    graph::{
        CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
        CommandParseErrorKind, ParsedArgument, ParsedArguments,
    },
    reader::CommandReader,
    requirement::CommandInputContext,
    suggestions::matches_suggestion_substr,
};

/// Biome command argument value: either one biome or a biome tag.
#[derive(Clone, Debug)]
pub enum BiomeArgumentValue {
    /// A single biome key.
    Biome(BiomeRef),
    /// A biome tag and its resolved entries.
    Tag {
        /// Tag key without the leading `#`.
        key: Identifier,
        /// Biomes in the tag.
        biomes: Vec<BiomeRef>,
    },
}

/// Structure command argument value: either one structure or a structure tag.
#[derive(Clone, Debug)]
pub enum StructureArgumentValue {
    /// A single structure key.
    Structure(StructureRef),
    /// A structure tag and its resolved entries.
    Tag {
        /// Tag key without the leading `#`.
        key: Identifier,
        /// Structures in the tag.
        structures: Vec<StructureRef>,
    },
}

impl StructureArgumentValue {
    /// Structure keys to scan.
    #[must_use]
    pub fn structure_keys(&self) -> Vec<Identifier> {
        match self {
            Self::Structure(structure) => vec![structure.key.clone()],
            Self::Tag { structures, .. } => structures
                .iter()
                .map(|structure| structure.key.clone())
                .collect(),
        }
    }

    /// Printable command target name.
    #[must_use]
    pub fn printable_name(&self, found_structure: &Identifier) -> String {
        match self {
            Self::Structure(structure) => structure.key.to_string(),
            Self::Tag { key, .. } => format!("#{key} ({found_structure})"),
        }
    }

    /// Printable command target without resolved found entry.
    #[must_use]
    pub fn query_name(&self) -> String {
        match self {
            Self::Structure(structure) => structure.key.to_string(),
            Self::Tag { key, .. } => format!("#{key}"),
        }
    }
}

impl BiomeArgumentValue {
    /// Returns whether this value matches `biome`.
    #[must_use]
    pub fn matches_biome(&self, biome: BiomeRef) -> bool {
        match self {
            Self::Biome(expected) => expected.key == biome.key,
            Self::Tag { biomes, .. } => biomes.iter().any(|entry| entry.key == biome.key),
        }
    }
}

/// Summonable entity type argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct EntitySummonParser;

impl CommandArgumentParser for EntitySummonParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_token()?;
        let Some(entity_type) = resolve_summon_entity_type(&raw) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidEntityType(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::EntityType(entity_type))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(
            ArgumentType::Resource {
                identifier: "minecraft:entity_type",
            },
            Some(SuggestionType::SummonableEntities),
        )
    }

    fn parsed_type(&self) -> &'static str {
        "entity_type"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let stripped_prefix = prefix.strip_prefix("minecraft:").unwrap_or(prefix);
        REGISTRY
            .entity_types
            .iter()
            .filter(|(_, entity_type)| can_summon_entity_type(entity_type))
            .map(|(_, entity_type)| SuggestionEntry::new(entity_type.key.to_string()))
            .filter(|suggestion| {
                let text = suggestion
                    .text
                    .strip_prefix("minecraft:")
                    .unwrap_or(&suggestion.text);
                matches_suggestion_substr(stripped_prefix, text)
            })
            .collect()
    }
}

pub(crate) fn parse_resource_identifier(input: &str) -> Option<Identifier> {
    let (namespace, path) = match input.split_once(':') {
        Some(("", path)) => (Identifier::VANILLA_NAMESPACE, path),
        Some((namespace, path)) => (namespace, path),
        None => (Identifier::VANILLA_NAMESPACE, input),
    };

    Identifier::validate(namespace, path)
        .then(|| Identifier::new(namespace.to_owned(), path.to_owned()))
}

fn resolve_summon_entity_type(input: &str) -> Option<EntityTypeRef> {
    let key = parse_resource_identifier(input)?;
    REGISTRY
        .entity_types
        .by_key(&key)
        .filter(|entity_type| can_summon_entity_type(entity_type))
}

fn can_summon_entity_type(entity_type: EntityTypeRef) -> bool {
    entity_type.summonable
}

/// Item resource argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct ItemParser;

impl CommandArgumentParser for ItemParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_token()?;
        let Some(key) = parse_resource_identifier(&raw) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidItem(raw),
                cursor,
            ));
        };

        let Some(item) = REGISTRY.items.by_key(&key) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidItem(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::Item(item))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::ItemStack, Some(SuggestionType::AskServer))
    }

    fn parsed_type(&self) -> &'static str {
        "item"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let stripped_prefix = prefix.strip_prefix("minecraft:").unwrap_or(prefix);
        REGISTRY
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
            .collect()
    }
}

/// Enchantment argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct EnchantmentParser;

impl CommandArgumentParser for EnchantmentParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_token()?;
        let key = raw.strip_prefix("minecraft:").unwrap_or(&raw).to_owned();

        let Some(enchantment) = REGISTRY.enchantments.by_key(&Identifier::vanilla(key)) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidEnchantment(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::Enchantment(enchantment))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(
            ArgumentType::Resource {
                identifier: "minecraft:enchantment",
            },
            Some(SuggestionType::AskServer),
        )
    }

    fn parsed_type(&self) -> &'static str {
        "enchantment"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let stripped_prefix = prefix.strip_prefix("minecraft:").unwrap_or(prefix);
        REGISTRY
            .enchantments
            .iter()
            .map(|(_, enchantment)| SuggestionEntry::new(enchantment.key.to_string()))
            .filter(|suggestion| {
                let text = suggestion
                    .text
                    .strip_prefix("minecraft:")
                    .unwrap_or(&suggestion.text);
                matches_suggestion_substr(stripped_prefix, text)
            })
            .collect()
    }
}

/// Biome or biome tag argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct BiomeParser;

impl CommandArgumentParser for BiomeParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_token()?;

        if let Some(tag) = raw.strip_prefix('#') {
            let Some(key) = parse_resource_identifier(tag) else {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidBiome(raw),
                    cursor,
                ));
            };
            let Some(biomes) = REGISTRY.biomes.get_tag(&key) else {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidBiome(raw),
                    cursor,
                ));
            };
            if biomes.is_empty() {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidBiome(raw),
                    cursor,
                ));
            }
            return Ok(ParsedArgument::Biome(BiomeArgumentValue::Tag {
                key,
                biomes,
            }));
        }

        let Some(key) = parse_resource_identifier(&raw) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidBiome(raw),
                cursor,
            ));
        };

        let Some(biome) = REGISTRY.biomes.by_key(&key) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidBiome(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::Biome(BiomeArgumentValue::Biome(biome)))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(
            ArgumentType::ResourceOrTag {
                identifier: "minecraft:worldgen/biome",
            },
            Some(SuggestionType::AskServer),
        )
    }

    fn parsed_type(&self) -> &'static str {
        "biome"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        if prefix.starts_with('#') {
            let stripped_prefix = prefix
                .strip_prefix("#minecraft:")
                .or_else(|| prefix.strip_prefix('#'))
                .unwrap_or(prefix);
            return REGISTRY
                .biomes
                .tag_keys()
                .filter_map(|key| {
                    let key = key.to_string();
                    let text = key.strip_prefix("minecraft:").unwrap_or(&key);
                    matches_suggestion_substr(stripped_prefix, text)
                        .then(|| SuggestionEntry::new(format!("#{key}")))
                })
                .collect();
        }

        let stripped_prefix = prefix.strip_prefix("minecraft:").unwrap_or(prefix);
        let mut suggestions = Vec::new();
        suggestions.extend(
            REGISTRY
                .biomes
                .iter()
                .map(|(_, biome)| SuggestionEntry::new(biome.key.to_string()))
                .filter(|suggestion| {
                    let text = suggestion
                        .text
                        .strip_prefix("minecraft:")
                        .unwrap_or(&suggestion.text);
                    matches_suggestion_substr(stripped_prefix, text)
                }),
        );
        suggestions.extend(
            REGISTRY
                .biomes
                .tag_keys()
                .map(|key| SuggestionEntry::new(format!("#{key}")))
                .filter(|suggestion| {
                    let text = suggestion
                        .text
                        .strip_prefix("#minecraft:")
                        .unwrap_or(&suggestion.text);
                    matches_suggestion_substr(stripped_prefix, text)
                }),
        );
        suggestions
    }
}

/// Structure or structure tag argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct StructureParser;

impl CommandArgumentParser for StructureParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_token()?;

        if let Some(tag) = raw.strip_prefix('#') {
            let Some(key) = parse_resource_identifier(tag) else {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidStructure(raw),
                    cursor,
                ));
            };
            let Some(structures) = REGISTRY.structures.get_tag(&key) else {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidStructure(raw),
                    cursor,
                ));
            };
            if structures.is_empty() {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidStructure(raw),
                    cursor,
                ));
            }
            return Ok(ParsedArgument::Structure(StructureArgumentValue::Tag {
                key,
                structures,
            }));
        }

        let Some(key) = parse_resource_identifier(&raw) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidStructure(raw),
                cursor,
            ));
        };

        let Some(structure) = REGISTRY.structures.by_key(&key) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidStructure(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::Structure(
            StructureArgumentValue::Structure(structure),
        ))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(
            ArgumentType::ResourceOrTagKey {
                identifier: "minecraft:worldgen/structure",
            },
            Some(SuggestionType::AskServer),
        )
    }

    fn parsed_type(&self) -> &'static str {
        "structure"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        if prefix.starts_with('#') {
            let stripped_prefix = prefix
                .strip_prefix("#minecraft:")
                .or_else(|| prefix.strip_prefix('#'))
                .unwrap_or(prefix);
            return REGISTRY
                .structures
                .tag_keys()
                .filter_map(|key| {
                    let key = key.to_string();
                    let text = key.strip_prefix("minecraft:").unwrap_or(&key);
                    matches_suggestion_substr(stripped_prefix, text)
                        .then(|| SuggestionEntry::new(format!("#{key}")))
                })
                .collect();
        }

        let stripped_prefix = prefix.strip_prefix("minecraft:").unwrap_or(prefix);
        let mut suggestions = Vec::new();
        suggestions.extend(
            REGISTRY
                .structures
                .iter()
                .map(|(_, structure)| SuggestionEntry::new(structure.key.to_string()))
                .filter(|suggestion| {
                    let text = suggestion
                        .text
                        .strip_prefix("minecraft:")
                        .unwrap_or(&suggestion.text);
                    matches_suggestion_substr(stripped_prefix, text)
                }),
        );
        suggestions.extend(
            REGISTRY
                .structures
                .tag_keys()
                .map(|key| SuggestionEntry::new(format!("#{key}")))
                .filter(|suggestion| {
                    let text = suggestion
                        .text
                        .strip_prefix("#minecraft:")
                        .unwrap_or(&suggestion.text);
                    matches_suggestion_substr(stripped_prefix, text)
                }),
        );
        suggestions
    }
}
