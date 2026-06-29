//! NBT command argument parsers.

use steel_protocol::packets::game::ArgumentType;
use steel_utils::nbt::parse_nbt_path_argument;

use crate::command::{
    graph::{
        CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
        CommandParseErrorKind, ParsedArgument,
    },
    reader::CommandReader,
    requirement::CommandInputContext,
};

/// NBT path argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct NbtPathParser;

impl CommandArgumentParser for NbtPathParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let start = reader.absolute_cursor();
        let (path, consumed) = parse_nbt_path_argument(reader.remaining()).map_err(|error| {
            CommandParseError::new(
                CommandParseErrorKind::InvalidNbtPath(error.message().to_owned()),
                start + error.cursor(),
            )
        })?;
        advance_reader(reader, consumed);
        Ok(ParsedArgument::NbtPath(path))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::NbtPath, None)
    }

    fn parsed_type(&self) -> &'static str {
        "nbt_path"
    }

    fn examples(&self) -> &'static [&'static str] {
        &["foo", "foo.bar", "foo[0]", "[0]", "[]", "{foo:\"bar\"}"]
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
