//! Shared command data-source helpers.

use std::borrow::Cow;

use simdnbt::owned::{NbtCompound, NbtTag};
use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use steel_utils::{BlockPos, Identifier, nbt::NbtPath};
use text_components::{TextComponent, translation::TranslatedMessage};

use crate::command::{
    context::CommandContext,
    error::CommandError,
    graph::{
        CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
        CommandParseErrorKind, ParsedArgument, ParsedArgumentError, ParsedArguments,
    },
    parsers::{parse_resource_identifier, resolve_required_entity_targets},
    reader::CommandReader,
    requirement::CommandInputContext,
    suggestions::matches_suggestion_substr,
};

#[derive(Clone, Copy, Debug, Default)]
pub(in crate::command::commands) struct StorageKeyParser;

impl CommandArgumentParser for StorageKeyParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_token()?;
        let Some(key) = parse_resource_identifier(&raw) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidIdentifier(raw),
                cursor,
            ));
        };

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
        let Some(server) = context.server() else {
            return Vec::new();
        };

        server
            .command_storage
            .keys()
            .into_iter()
            .map(|key| key.to_string())
            .filter(|key| matches_suggestion_substr(prefix, key))
            .map(SuggestionEntry::new)
            .collect()
    }
}

pub(in crate::command::commands) fn block_entity_full_nbt(
    block_entity: &dyn crate::block_entity::BlockEntity,
) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    let entity_pos = block_entity.get_block_pos();
    nbt.insert("id", block_entity.get_type().key.to_string());
    nbt.insert("x", entity_pos.x());
    nbt.insert("y", entity_pos.y());
    nbt.insert("z", entity_pos.z());
    block_entity.save_additional(&mut nbt);
    nbt
}

pub(in crate::command::commands) fn data_source_argument_compound(
    context: &CommandContext,
    arguments: &ParsedArguments,
) -> Result<NbtCompound, CommandError> {
    let compound = data_source_compound(context, arguments)?;
    match arguments.get::<NbtPath>("path") {
        Ok(path) => single_compound_at_path(compound, &path),
        Err(ParsedArgumentError::Missing(_)) => Ok(compound),
        Err(error) => Err(super::invalid_parsed_argument(error)),
    }
}

fn data_source_compound(
    context: &CommandContext,
    arguments: &ParsedArguments,
) -> Result<NbtCompound, CommandError> {
    if let Ok(pos) = arguments.get::<BlockPos>("sourcePos") {
        return block_data_source_compound(context, pos);
    }

    match arguments.get::<Identifier>("source") {
        Ok(source) => return Ok(context.server.command_storage.get(&source)),
        Err(ParsedArgumentError::WrongType { .. }) => {
            return entity_data_source_compound(context, arguments);
        }
        Err(ParsedArgumentError::Missing(_)) => {}
    }

    Err(super::invalid_parsed_argument(
        ParsedArgumentError::Missing("source".to_owned()),
    ))
}

fn block_data_source_compound(
    context: &CommandContext,
    pos: BlockPos,
) -> Result<NbtCompound, CommandError> {
    loaded_block_position(context, pos)?;
    let Some(block_entity) = context.world.get_block_entity(pos) else {
        return Err(block_data_invalid_error());
    };

    let block_entity = block_entity.lock();
    Ok(block_entity_full_nbt(&*block_entity))
}

fn entity_data_source_compound(
    context: &CommandContext,
    arguments: &ParsedArguments,
) -> Result<NbtCompound, CommandError> {
    let mut entities = resolve_required_entity_targets(arguments, "source", context)?;
    if entities.len() != 1 {
        return Err(super::invalid_parsed_argument(
            ParsedArgumentError::WrongType {
                name: "source".to_owned(),
                expected: "single_entity",
                actual: "entities",
            },
        ));
    }
    Ok(entities.remove(0).nbt_for_data_compare())
}

fn single_compound_at_path(
    compound: NbtCompound,
    path: &NbtPath,
) -> Result<NbtCompound, CommandError> {
    let root = NbtTag::Compound(compound);
    let tags = path.get(&root);
    let Some(tag) = tags.first() else {
        return Err(nothing_found_error(path));
    };
    if tags.len() > 1 {
        return Err(multiple_tags_error());
    }
    let NbtTag::Compound(compound) = tag else {
        return Err(argument_not_compound_error(nbt_tag_type_name(tag)));
    };
    Ok(compound.clone())
}

fn loaded_block_position(context: &CommandContext, pos: BlockPos) -> Result<(), CommandError> {
    if !context.world.is_full_chunk_loaded_at(pos) {
        return Err(position_error("argument.pos.unloaded"));
    }
    if !context.world.is_in_valid_bounds(pos) {
        return Err(position_error("argument.pos.outofworld"));
    }
    Ok(())
}

pub(in crate::command::commands) fn position_error(key: &'static str) -> CommandError {
    CommandError::failure(TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed(key),
        fallback: None,
        args: None,
    }))
}

pub(in crate::command::commands) fn block_data_invalid_error() -> CommandError {
    CommandError::failure(TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed("commands.data.block.invalid"),
        fallback: None,
        args: None,
    }))
}

fn nothing_found_error(path: &NbtPath) -> CommandError {
    CommandError::failure(TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed("arguments.nbtpath.nothing_found"),
        fallback: None,
        args: Some(Box::new([TextComponent::from(path.as_str().to_owned())])),
    }))
}

fn multiple_tags_error() -> CommandError {
    CommandError::failure(TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed("commands.data.get.multiple"),
        fallback: None,
        args: None,
    }))
}

fn argument_not_compound_error(tag_type: &'static str) -> CommandError {
    CommandError::failure(TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed("commands.function.error.argument_not_compound"),
        fallback: None,
        args: Some(Box::new([TextComponent::from(tag_type)])),
    }))
}

fn nbt_tag_type_name(tag: &NbtTag) -> &'static str {
    match tag {
        NbtTag::Byte(_) => "BYTE",
        NbtTag::Short(_) => "SHORT",
        NbtTag::Int(_) => "INT",
        NbtTag::Long(_) => "LONG",
        NbtTag::Float(_) => "FLOAT",
        NbtTag::Double(_) => "DOUBLE",
        NbtTag::ByteArray(_) => "BYTE[]",
        NbtTag::String(_) => "STRING",
        NbtTag::List(_) => "LIST",
        NbtTag::Compound(_) => "COMPOUND",
        NbtTag::IntArray(_) => "INT[]",
        NbtTag::LongArray(_) => "LONG[]",
    }
}
