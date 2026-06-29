//! Game-mode command argument parser.

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry};
use steel_utils::types::GameType;

use crate::command::{
    graph::{
        CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
        CommandParseErrorKind, ParsedArgument, ParsedArguments,
    },
    reader::{CommandReader, StringMode},
    requirement::CommandInputContext,
    suggestions::matches_suggestion_substr,
};

/// Game mode argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct GameModeParser;

impl CommandArgumentParser for GameModeParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let value = reader.read_string(StringMode::SingleWord)?;

        let game_mode = match value.to_lowercase().as_str() {
            "survival" | "0" => GameType::Survival,
            "creative" | "1" => GameType::Creative,
            "adventure" | "2" => GameType::Adventure,
            "spectator" | "3" => GameType::Spectator,
            _ => {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidGameMode(value),
                    cursor,
                ));
            }
        };

        Ok(ParsedArgument::GameMode(game_mode))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::Gamemode, None)
    }

    fn parsed_type(&self) -> &'static str {
        "gamemode"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        ["survival", "creative", "adventure", "spectator"]
            .into_iter()
            .filter(|suggestion| matches_suggestion_substr(prefix, suggestion))
            .map(SuggestionEntry::new)
            .collect()
    }
}
