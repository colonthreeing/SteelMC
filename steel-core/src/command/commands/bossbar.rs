//! Handler for the `bossbar` command.

use std::{borrow::Cow, sync::Arc};

use steel_protocol::packets::game::{
    ArgumentType, BossBarColor, BossBarOverlay, SuggestionEntry, SuggestionType,
};
use steel_utils::Identifier;
use text_components::{
    Modifier, TextComponent, format::Color, interactivity::HoverEvent,
    translation::TranslatedMessage,
};
use uuid::Uuid;

use crate::{
    command::{
        CommandRegistrationSpec,
        context::CommandContext,
        error::CommandError,
        graph::{
            BoolParser, CommandArgumentClientParser, CommandArgumentParser, CommandNodeBuilder,
            CommandParseError, CommandParseErrorKind, CommandResult, IntegerParser,
            ParsedArgument, ParsedArguments, argument, literal,
        },
        parsers::{ComponentParser, PlayerParser, parse_resource_identifier},
        reader::CommandReader,
        requirement::CommandInputContext,
        suggestions::matches_suggestion_substr,
    },
    entity::Entity,
    player::Player,
    server::bossbar::{BossBarError, BossBarMutation, BossBarSnapshot},
};

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Handler for the `bossbar` command.
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
    literal("bossbar")
        .then(
            literal("add").then(
                argument("id", BossBarIdParser::any())
                    .then(argument("name", ComponentParser).executes(create)),
            ),
        )
        .then(literal("remove").then(argument("id", BossBarIdParser::existing()).executes(remove)))
        .then(literal("list").executes(list))
        .then(
            literal("set").then(
                argument("id", BossBarIdParser::existing())
                    .then(literal("name").then(argument("name", ComponentParser).executes(set_name)))
                    .then(
                        literal("color")
                            .then(color_node("pink", BossBarColor::Pink))
                            .then(color_node("blue", BossBarColor::Blue))
                            .then(color_node("red", BossBarColor::Red))
                            .then(color_node("green", BossBarColor::Green))
                            .then(color_node("yellow", BossBarColor::Yellow))
                            .then(color_node("purple", BossBarColor::Purple))
                            .then(color_node("white", BossBarColor::White)),
                    )
                    .then(
                        literal("style")
                            .then(overlay_node("progress", BossBarOverlay::Progress))
                            .then(overlay_node("notched_6", BossBarOverlay::Notched6))
                            .then(overlay_node("notched_10", BossBarOverlay::Notched10))
                            .then(overlay_node("notched_12", BossBarOverlay::Notched12))
                            .then(overlay_node("notched_20", BossBarOverlay::Notched20)),
                    )
                    .then(
                        literal("value").then(
                            argument("value", IntegerParser::bounded(Some(0), None))
                                .executes(set_value),
                        ),
                    )
                    .then(
                        literal("max").then(
                            argument("max", IntegerParser::bounded(Some(1), None))
                                .executes(set_max),
                        ),
                    )
                    .then(
                        literal("visible")
                            .then(argument("visible", BoolParser).executes(set_visible)),
                    )
                    .then(
                        literal("players").executes(clear_players).then(
                            argument("targets", PlayerParser::multiple()).executes(set_players),
                        ),
                    ),
            ),
        )
        .then(
            literal("get").then(
                argument("id", BossBarIdParser::existing())
                    .then(literal("value").executes(get_value))
                    .then(literal("max").executes(get_max))
                    .then(literal("visible").executes(get_visible))
                    .then(literal("players").executes(get_players)),
            ),
        )
}

fn color_node(name: &'static str, color: BossBarColor) -> CommandNodeBuilder {
    literal(name).executes(move |context, arguments| set_color(context, arguments, color))
}

fn overlay_node(name: &'static str, overlay: BossBarOverlay) -> CommandNodeBuilder {
    literal(name).executes(move |context, arguments| set_overlay(context, arguments, overlay))
}

fn create(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let id = bossbar_id(arguments)?;
    let name = arguments
        .get::<TextComponent>("name")
        .map_err(super::invalid_parsed_argument)?;
    let (snapshot, count) = {
        let mut boss_bars = context.server.boss_bars.write();
        let snapshot = boss_bars
            .create(id, name)
            .map_err(bossbar_error)?;
        (snapshot, boss_bars.len())
    };

    context.sender.send_message(&translated(
        "commands.bossbar.create.success",
        [display_name(&snapshot)],
    ));
    Ok(CommandResult::from_usize_success_count(count))
}

fn remove(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let id = bossbar_id(arguments)?;
    let (snapshot, broadcasts, count) = {
        let mut boss_bars = context.server.boss_bars.write();
        let (snapshot, broadcasts) = boss_bars.remove(&id).map_err(bossbar_error)?;
        (snapshot, broadcasts, boss_bars.len())
    };
    context.server.send_bossbar_broadcasts(broadcasts);

    context.sender.send_message(&translated(
        "commands.bossbar.remove.success",
        [display_name(&snapshot)],
    ));
    Ok(CommandResult::from_usize_success_count(count))
}

fn list(
    context: &mut CommandContext,
    _arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let snapshots = context.server.boss_bars.read().snapshots();
    if snapshots.is_empty() {
        context
            .sender
            .send_message(&translated("commands.bossbar.list.bars.none", []));
    } else {
        let display_names = snapshots.iter().map(display_name).collect::<Vec<_>>();
        context.sender.send_message(&translated(
            "commands.bossbar.list.bars.some",
            [
                TextComponent::from(snapshots.len().to_string()),
                format_component_list(display_names),
            ],
        ));
    }

    Ok(CommandResult::from_usize_success_count(snapshots.len()))
}

fn set_name(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let id = bossbar_id(arguments)?;
    let name = arguments
        .get::<TextComponent>("name")
        .map_err(super::invalid_parsed_argument)?;
    let mutation = context
        .server
        .boss_bars
        .write()
        .set_name(&id, name)
        .map_err(bossbar_error)?;
    require_changed(&mutation, "commands.bossbar.set.name.unchanged")?;
    context.server.send_bossbar_broadcasts(mutation.broadcasts);

    context.sender.send_message(&translated(
        "commands.bossbar.set.name.success",
        [display_name(&mutation.snapshot)],
    ));
    Ok(CommandResult::from_return_value(0))
}

fn set_color(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    color: BossBarColor,
) -> Result<CommandResult, CommandError> {
    let id = bossbar_id(arguments)?;
    let mutation = context
        .server
        .boss_bars
        .write()
        .set_color(&id, color)
        .map_err(bossbar_error)?;
    require_changed(&mutation, "commands.bossbar.set.color.unchanged")?;
    context.server.send_bossbar_broadcasts(mutation.broadcasts);

    context.sender.send_message(&translated(
        "commands.bossbar.set.color.success",
        [display_name(&mutation.snapshot)],
    ));
    Ok(CommandResult::from_return_value(0))
}

fn set_overlay(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    overlay: BossBarOverlay,
) -> Result<CommandResult, CommandError> {
    let id = bossbar_id(arguments)?;
    let mutation = context
        .server
        .boss_bars
        .write()
        .set_overlay(&id, overlay)
        .map_err(bossbar_error)?;
    require_changed(&mutation, "commands.bossbar.set.style.unchanged")?;
    context.server.send_bossbar_broadcasts(mutation.broadcasts);

    context.sender.send_message(&translated(
        "commands.bossbar.set.style.success",
        [display_name(&mutation.snapshot)],
    ));
    Ok(CommandResult::from_return_value(0))
}

fn set_value(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let id = bossbar_id(arguments)?;
    let value = integer(arguments, "value")?;
    let mutation = context
        .server
        .boss_bars
        .write()
        .set_value(&id, value)
        .map_err(bossbar_error)?;
    require_changed(&mutation, "commands.bossbar.set.value.unchanged")?;
    context.server.send_bossbar_broadcasts(mutation.broadcasts);

    context.sender.send_message(&translated(
        "commands.bossbar.set.value.success",
        [
            display_name(&mutation.snapshot),
            TextComponent::from(value.to_string()),
        ],
    ));
    Ok(CommandResult::from_return_value(value))
}

fn set_max(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let id = bossbar_id(arguments)?;
    let max = integer(arguments, "max")?;
    let mutation = context
        .server
        .boss_bars
        .write()
        .set_max(&id, max)
        .map_err(bossbar_error)?;
    require_changed(&mutation, "commands.bossbar.set.max.unchanged")?;
    context.server.send_bossbar_broadcasts(mutation.broadcasts);

    context.sender.send_message(&translated(
        "commands.bossbar.set.max.success",
        [
            display_name(&mutation.snapshot),
            TextComponent::from(max.to_string()),
        ],
    ));
    Ok(CommandResult::from_return_value(max))
}

fn set_visible(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let id = bossbar_id(arguments)?;
    let visible = arguments
        .get::<bool>("visible")
        .map_err(super::invalid_parsed_argument)?;
    let mutation = context
        .server
        .boss_bars
        .write()
        .set_visible(&id, visible)
        .map_err(bossbar_error)?;
    if !mutation.changed {
        return Err(unchanged(if visible {
            "commands.bossbar.set.visibility.unchanged.visible"
        } else {
            "commands.bossbar.set.visibility.unchanged.hidden"
        }));
    }
    context.server.send_bossbar_broadcasts(mutation.broadcasts);

    context.sender.send_message(&translated(
        if visible {
            "commands.bossbar.set.visible.success.visible"
        } else {
            "commands.bossbar.set.visible.success.hidden"
        },
        [display_name(&mutation.snapshot)],
    ));
    Ok(CommandResult::from_return_value(0))
}

fn clear_players(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    set_player_uuids(context, arguments, [])
}

fn set_players(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = players(arguments)?;
    let uuids = targets
        .iter()
        .map(|player| player.uuid())
        .collect::<Vec<_>>();
    set_player_uuids(context, arguments, uuids)
}

fn set_player_uuids(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    uuids: impl IntoIterator<Item = Uuid>,
) -> Result<CommandResult, CommandError> {
    let id = bossbar_id(arguments)?;
    let mutation = context
        .server
        .boss_bars
        .write()
        .set_players(&id, uuids)
        .map_err(bossbar_error)?;
    require_changed(&mutation, "commands.bossbar.set.players.unchanged")?;
    context.server.send_bossbar_broadcasts(mutation.broadcasts);

    let players = online_players(context, &mutation.snapshot.players);
    send_players_message(
        context,
        &mutation.snapshot,
        &players,
        "commands.bossbar.set.players.success.none",
        "commands.bossbar.set.players.success.some",
    );
    Ok(CommandResult::from_usize_success_count(players.len()))
}

fn get_value(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let snapshot = snapshot(context, arguments)?;
    context.sender.send_message(&translated(
        "commands.bossbar.get.value",
        [
            display_name(&snapshot),
            TextComponent::from(snapshot.value.to_string()),
        ],
    ));
    Ok(CommandResult::from_return_value(snapshot.value))
}

fn get_max(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let snapshot = snapshot(context, arguments)?;
    context.sender.send_message(&translated(
        "commands.bossbar.get.max",
        [
            display_name(&snapshot),
            TextComponent::from(snapshot.max.to_string()),
        ],
    ));
    Ok(CommandResult::from_return_value(snapshot.max))
}

fn get_visible(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let snapshot = snapshot(context, arguments)?;
    context.sender.send_message(&translated(
        if snapshot.visible {
            "commands.bossbar.get.visible.visible"
        } else {
            "commands.bossbar.get.visible.hidden"
        },
        [display_name(&snapshot)],
    ));
    Ok(CommandResult::from_return_value(i32::from(snapshot.visible)))
}

fn get_players(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let snapshot = snapshot(context, arguments)?;
    let players = online_players(context, &snapshot.players);
    send_players_message(
        context,
        &snapshot,
        &players,
        "commands.bossbar.get.players.none",
        "commands.bossbar.get.players.some",
    );
    Ok(CommandResult::from_usize_success_count(players.len()))
}

fn send_players_message(
    context: &CommandContext,
    snapshot: &BossBarSnapshot,
    players: &[Arc<Player>],
    none_key: &'static str,
    some_key: &'static str,
) {
    if players.is_empty() {
        context
            .sender
            .send_message(&translated(none_key, [display_name(snapshot)]));
        return;
    }

    context.sender.send_message(&translated(
        some_key,
        [
            display_name(snapshot),
            TextComponent::from(players.len().to_string()),
            format_component_list(
                players
                    .iter()
                    .map(|player| TextComponent::from(player.gameprofile.name.clone()))
                    .collect(),
            ),
        ],
    ));
}

fn require_changed(mutation: &BossBarMutation, key: &'static str) -> Result<(), CommandError> {
    if mutation.changed {
        Ok(())
    } else {
        Err(unchanged(key))
    }
}

fn snapshot(
    context: &CommandContext,
    arguments: &ParsedArguments,
) -> Result<BossBarSnapshot, CommandError> {
    let id = bossbar_id(arguments)?;
    context
        .server
        .boss_bars
        .read()
        .get(&id)
        .ok_or_else(|| bossbar_does_not_exist(&id))
}

fn bossbar_id(arguments: &ParsedArguments) -> Result<Identifier, CommandError> {
    arguments
        .get::<Identifier>("id")
        .map_err(super::invalid_parsed_argument)
}

fn integer(arguments: &ParsedArguments, name: &str) -> Result<i32, CommandError> {
    arguments
        .get::<i32>(name)
        .map_err(super::invalid_parsed_argument)
}

fn players(arguments: &ParsedArguments) -> Result<Vec<Arc<Player>>, CommandError> {
    arguments
        .get::<Vec<Arc<Player>>>("targets")
        .map_err(super::invalid_parsed_argument)
}

fn online_players(context: &CommandContext, uuids: &[Uuid]) -> Vec<Arc<Player>> {
    uuids
        .iter()
        .filter_map(|uuid| context.server.get_player_by_uuid(uuid))
        .collect()
}

pub(in crate::command::commands) fn bossbar_does_not_exist(id: &Identifier) -> CommandError {
    CommandError::failure(translated(
        "commands.bossbar.unknown",
        [TextComponent::from(id.to_string())],
    ))
}

fn bossbar_error(error: BossBarError) -> CommandError {
    match error {
        BossBarError::AlreadyExists(id) => CommandError::failure(translated(
            "commands.bossbar.create.failed",
            [TextComponent::from(id.to_string())],
        )),
        BossBarError::Missing(id) => bossbar_does_not_exist(&id),
    }
}

fn unchanged(key: &'static str) -> CommandError {
    CommandError::failure(translated(key, []))
}

fn display_name(snapshot: &BossBarSnapshot) -> TextComponent {
    let id = snapshot.id.to_string();
    TextComponent::plain("[")
        .add_child(snapshot.name.clone())
        .add_child("]")
        .color(display_color(snapshot.color))
        .hover_event(HoverEvent::show_text(TextComponent::from(id.clone())))
        .insertion(id)
}

fn display_color(color: BossBarColor) -> Color {
    match color {
        BossBarColor::Pink => Color::LightPurple,
        BossBarColor::Blue => Color::Blue,
        BossBarColor::Red => Color::Red,
        BossBarColor::Green => Color::Green,
        BossBarColor::Yellow => Color::Yellow,
        BossBarColor::Purple => Color::DarkPurple,
        BossBarColor::White => Color::White,
    }
}

fn format_component_list(components: Vec<TextComponent>) -> TextComponent {
    let mut components = components.into_iter();
    let Some(first) = components.next() else {
        return TextComponent::plain("");
    };

    components.fold(first, |list, component| {
        list.add_child(TextComponent::plain(", ")).add_child(component)
    })
}

fn translated<const N: usize>(key: &'static str, args: [TextComponent; N]) -> TextComponent {
    TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed(key),
        fallback: None,
        args: Some(Box::new(args)),
    })
}

/// Boss bar id parser with server-backed id suggestions.
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::command::commands) struct BossBarIdParser {
    suggest_existing: bool,
}

impl BossBarIdParser {
    /// Creates a parser for a new or existing boss bar id without suggestions.
    #[must_use]
    pub(in crate::command::commands) const fn any() -> Self {
        Self {
            suggest_existing: false,
        }
    }

    /// Creates a parser for an existing boss bar id with server suggestions.
    #[must_use]
    pub(in crate::command::commands) const fn existing() -> Self {
        Self {
            suggest_existing: true,
        }
    }
}

impl CommandArgumentParser for BossBarIdParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_token()?;
        let Some(id) = parse_resource_identifier(&raw) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidIdentifier(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::Identifier(id))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(
            ArgumentType::Identifier,
            self.suggest_existing
                .then_some(SuggestionType::AskServer),
        )
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
        if !self.suggest_existing {
            return Vec::new();
        }

        let Some(server) = context.server() else {
            return Vec::new();
        };

        server
            .boss_bars
            .read()
            .ids()
            .into_iter()
            .map(|id| id.to_string())
            .filter(|id| matches_identifier_suggestion(prefix, id))
            .map(SuggestionEntry::new)
            .collect()
    }

    fn examples(&self) -> &'static [&'static str] {
        &["foo", "minecraft:foo"]
    }
}

fn matches_identifier_suggestion(prefix: &str, value: &str) -> bool {
    let prefix = prefix.strip_prefix("minecraft:").unwrap_or(prefix);
    let value = value.strip_prefix("minecraft:").unwrap_or(value);
    matches_suggestion_substr(prefix, value)
}

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use crate::command::{
        graph::CommandGraph,
        requirement::{CommandInputContext, CommandSourceKind, PermissionExpr, RequirementContext},
    };

    struct TestContext;

    impl RequirementContext for TestContext {
        fn source_kind(&self) -> CommandSourceKind {
            CommandSourceKind::Player
        }

        fn has_permission(&self, _permission: &PermissionExpr) -> bool {
            false
        }
    }

    impl CommandInputContext for TestContext {
        fn position(&self) -> Option<DVec3> {
            Some(DVec3::ZERO)
        }
    }

    fn graph() -> CommandGraph {
        CommandGraph::new()
            .with_root(super::command())
            .expect("bossbar command registers")
    }

    #[test]
    fn bossbar_command_parses_vanilla_branches() {
        let graph = graph();
        let context = TestContext;

        let add = graph
            .parse("bossbar add minecraft:test {text:\"Boss\"}", &context)
            .expect("bossbar add parses");
        assert_eq!(add.path(), ["bossbar", "add", "id", "name"]);

        let color = graph
            .parse("bossbar set minecraft:test color red", &context)
            .expect("bossbar set color parses");
        assert_eq!(color.path(), ["bossbar", "set", "id", "color", "red"]);

        let players = graph
            .parse("bossbar set minecraft:test players", &context)
            .expect("bossbar clear players parses");
        assert_eq!(players.path(), ["bossbar", "set", "id", "players"]);

        let get = graph
            .parse("bossbar get minecraft:test max", &context)
            .expect("bossbar get max parses");
        assert_eq!(get.path(), ["bossbar", "get", "id", "max"]);
    }
}
