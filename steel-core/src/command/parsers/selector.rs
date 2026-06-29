//! Vanilla-style entity selector parsing and resolution.

use std::sync::Arc;

use glam::DVec3;
use rand::seq::SliceRandom;
use simdnbt::owned::NbtCompound;
use steel_registry::{
    REGISTRY, RegistryExt, TaggedRegistryExt, entity_type::EntityTypeRef, vanilla_entities,
};
use steel_utils::{
    Identifier,
    geometry::WorldAabb,
    nbt::{compare_nbt_compounds, parse_snbt_compound_argument},
    types::GameType,
};
use uuid::Uuid;

use crate::{
    command::{
        entity_selector_permission_expr,
        graph::{CommandParseError, CommandParseErrorKind},
        parsers::parse_resource_identifier,
        reader::{CommandReader, StringMode},
        requirement::CommandInputContext,
    },
    entity::{Entity, SharedEntity},
    player::Player,
    scoreboard::{ScoreHolder, Scoreboard},
    server::Server,
};

const SORT_NEAREST: &str = "nearest";
const SORT_FURTHEST: &str = "furthest";
const SORT_RANDOM: &str = "random";
const SORT_ARBITRARY: &str = "arbitrary";

#[derive(Clone, Debug)]
pub(super) struct EntitySelector {
    raw: String,
    kind: SelectorKind,
    max_results: usize,
    includes_entities: bool,
    current_entity: bool,
    world_limited: bool,
    order: SelectorOrder,
    position: SelectorPosition,
    delta: SelectorDelta,
    distance: Option<DoubleRange>,
    level: Option<IntRange>,
    x_rotation: Option<FloatRange>,
    y_rotation: Option<FloatRange>,
    filters: Vec<SelectorFilter>,
}

#[derive(Clone, Debug)]
enum SelectorKind {
    Selector(SelectorType),
    PlayerName(String),
    EntityUuid(Uuid),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectorType {
    AllPlayers,
    AllEntities,
    NearestEntity,
    NearestPlayer,
    RandomPlayer,
    SelfEntity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectorOrder {
    Nearest,
    Furthest,
    Random,
    Arbitrary,
}

#[derive(Clone, Debug, Default)]
struct SelectorPosition {
    x: Option<f64>,
    y: Option<f64>,
    z: Option<f64>,
}

impl SelectorPosition {
    fn apply(&self, base: DVec3) -> DVec3 {
        DVec3::new(
            self.x.unwrap_or(base.x),
            self.y.unwrap_or(base.y),
            self.z.unwrap_or(base.z),
        )
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct SelectorDelta {
    x: Option<f64>,
    y: Option<f64>,
    z: Option<f64>,
}

impl SelectorDelta {
    const fn has_any(self) -> bool {
        self.x.is_some() || self.y.is_some() || self.z.is_some()
    }

    fn aabb(self) -> WorldAabb {
        create_delta_aabb(
            self.x.unwrap_or(0.0),
            self.y.unwrap_or(0.0),
            self.z.unwrap_or(0.0),
        )
    }
}

#[derive(Clone, Debug)]
enum SelectorFilter {
    Alive,
    Name {
        value: String,
        inverted: bool,
    },
    GameMode {
        value: GameType,
        inverted: bool,
    },
    EntityType {
        value: EntityTypeRef,
        inverted: bool,
    },
    EntityTypeTag {
        value: Identifier,
        inverted: bool,
    },
    Tag {
        value: String,
        inverted: bool,
    },
    Nbt {
        value: NbtCompound,
        inverted: bool,
    },
    Scores(Vec<(String, IntRange)>),
}

#[derive(Clone, Copy, Debug)]
struct DoubleRange {
    min: Option<f64>,
    max: Option<f64>,
}

impl DoubleRange {
    fn matches_squared(self, value: f64) -> bool {
        if let Some(min) = self.min
            && value < min * min
        {
            return false;
        }
        if let Some(max) = self.max
            && value > max * max
        {
            return false;
        }
        true
    }
}

#[derive(Clone, Copy, Debug)]
struct FloatRange {
    min: Option<f32>,
    max: Option<f32>,
}

impl FloatRange {
    fn matches_rotation(self, value: f32) -> bool {
        let min = wrap_degrees(self.min.unwrap_or(0.0));
        let max = wrap_degrees(self.max.unwrap_or(359.0));
        let value = wrap_degrees(value);
        if min > max {
            value >= min || value <= max
        } else {
            value >= min && value <= max
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct IntRange {
    min: Option<i32>,
    max: Option<i32>,
}

impl IntRange {
    #[cfg(test)]
    const fn exactly(value: i32) -> Self {
        Self {
            min: Some(value),
            max: Some(value),
        }
    }

    fn matches(self, value: i32) -> bool {
        if let Some(min) = self.min
            && value < min
        {
            return false;
        }
        if let Some(max) = self.max
            && value > max
        {
            return false;
        }
        true
    }
}

#[derive(Clone, Debug, Default)]
struct InvertableOptionState {
    positive_seen: bool,
    negative_seen: bool,
}

impl InvertableOptionState {
    fn parse_element(&mut self, inverted: bool, option: &str) -> Result<(), SelectorParseError> {
        if inverted {
            if self.positive_seen {
                return Err(SelectorParseError::invalid(format!(
                    "option '{option}' cannot be repeated after a positive value"
                )));
            }
            self.negative_seen = true;
        } else {
            if self.positive_seen || self.negative_seen {
                return Err(SelectorParseError::invalid(format!(
                    "option '{option}' cannot add a positive value after another value"
                )));
            }
            self.positive_seen = true;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
struct EntityTypeOptionState {
    invertible: InvertableOptionState,
    tags_seen: Vec<Identifier>,
}

impl EntityTypeOptionState {
    fn parse_element(&mut self, inverted: bool, option: &str) -> Result<(), SelectorParseError> {
        self.invertible.parse_element(inverted, option)
    }

    fn parse_tag(&mut self, tag: &Identifier, option: &str) -> Result<(), SelectorParseError> {
        if self.tags_seen.iter().any(|existing| existing == tag) {
            return Err(SelectorParseError::invalid(format!(
                "option '{option}' cannot repeat tag '#{tag}'"
            )));
        }
        self.invertible.parse_element(true, option)?;
        self.tags_seen.push(tag.clone());
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
struct SelectorOptionState {
    name: InvertableOptionState,
    gamemode: InvertableOptionState,
    entity_type: EntityTypeOptionState,
    distance: bool,
    level: bool,
    x: bool,
    y: bool,
    z: bool,
    dx: bool,
    dy: bool,
    dz: bool,
    x_rotation: bool,
    y_rotation: bool,
    limit: bool,
    sort: bool,
    scores: bool,
}

#[derive(Clone, Debug)]
struct SelectorParseError {
    kind: SelectorParseErrorKind,
    cursor: usize,
}

#[derive(Clone, Debug)]
enum SelectorParseErrorKind {
    NotAllowed,
    Invalid(String),
    Unsupported(String),
}

impl SelectorParseError {
    const fn not_allowed(cursor: usize) -> Self {
        Self {
            kind: SelectorParseErrorKind::NotAllowed,
            cursor,
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self {
            kind: SelectorParseErrorKind::Invalid(message.into()),
            cursor: 0,
        }
    }

    fn invalid_at(message: impl Into<String>, cursor: usize) -> Self {
        Self {
            kind: SelectorParseErrorKind::Invalid(message.into()),
            cursor,
        }
    }

    fn unsupported(option: impl Into<String>, cursor: usize) -> Self {
        Self {
            kind: SelectorParseErrorKind::Unsupported(option.into()),
            cursor,
        }
    }

    fn into_command_error(self, base_cursor: usize) -> CommandParseError {
        let cursor = base_cursor + self.cursor;
        let kind = match self.kind {
            SelectorParseErrorKind::NotAllowed => CommandParseErrorKind::EntitySelectorsNotAllowed,
            SelectorParseErrorKind::Invalid(message) => {
                CommandParseErrorKind::InvalidEntitySelector(message)
            }
            SelectorParseErrorKind::Unsupported(option) => {
                CommandParseErrorKind::UnsupportedEntitySelectorOption(option)
            }
        };
        CommandParseError::new(kind, cursor)
    }
}

impl EntitySelector {
    fn parse(
        raw: String,
        base_cursor: usize,
        allow_selectors: bool,
    ) -> Result<Self, CommandParseError> {
        parse_selector_plan(raw, allow_selectors)
            .map_err(|error| error.into_command_error(base_cursor))
    }

    fn validate_for_argument(
        &self,
        single: bool,
        players_only: bool,
        cursor: usize,
    ) -> Result<(), CommandParseError> {
        if single && self.max_results > 1 {
            let kind = if players_only {
                CommandParseErrorKind::InvalidPlayer(self.raw.clone())
            } else {
                CommandParseErrorKind::InvalidEntity(self.raw.clone())
            };
            return Err(CommandParseError::new(kind, cursor));
        }
        if players_only && self.includes_entities && !self.current_entity {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidPlayer(self.raw.clone()),
                cursor,
            ));
        }
        Ok(())
    }

    pub(super) fn find_players(
        &self,
        context: &dyn CommandInputContext,
        cursor: usize,
    ) -> Result<Vec<Arc<Player>>, CommandParseError> {
        self.check_selector_permission(context, cursor)?;
        let Some(server) = context.server() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("server"),
                cursor,
            ));
        };
        let position = selector_position(self, context, cursor)?;
        let aabb = self.absolute_aabb(position);
        let mut players = match &self.kind {
            SelectorKind::PlayerName(name) => server
                .get_players()
                .into_iter()
                .filter(|player| player_name_matches(&player.gameprofile.name, name))
                .collect::<Vec<_>>(),
            SelectorKind::EntityUuid(uuid) => server
                .get_players()
                .into_iter()
                .filter(|player| player.uuid() == *uuid)
                .collect::<Vec<_>>(),
            SelectorKind::Selector(SelectorType::SelfEntity) => {
                context.player().map_or_else(Vec::new, |player| {
                    if self.matches_entity(player.as_ref(), position, aabb, server) {
                        vec![Arc::clone(player)]
                    } else {
                        Vec::new()
                    }
                })
            }
            SelectorKind::Selector(_) => self.candidate_players(server, context, cursor)?,
        };

        if !matches!(self.kind, SelectorKind::Selector(SelectorType::SelfEntity)) {
            players.retain(|player| self.matches_entity(player.as_ref(), position, aabb, server));
        }
        self.sort_and_limit_players(position, &mut players);
        Ok(players)
    }

    pub(super) fn find_entities(
        &self,
        context: &dyn CommandInputContext,
        cursor: usize,
    ) -> Result<Vec<SharedEntity>, CommandParseError> {
        self.check_selector_permission(context, cursor)?;
        let Some(server) = context.server() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("server"),
                cursor,
            ));
        };
        let position = selector_position(self, context, cursor)?;
        let aabb = self.absolute_aabb(position);
        let mut entities = match &self.kind {
            SelectorKind::PlayerName(name) => server
                .get_players()
                .into_iter()
                .filter(|player| player_name_matches(&player.gameprofile.name, name))
                .map(|player| player as SharedEntity)
                .collect::<Vec<_>>(),
            SelectorKind::EntityUuid(uuid) => find_entity_by_uuid(server, uuid)
                .into_iter()
                .collect::<Vec<_>>(),
            SelectorKind::Selector(SelectorType::SelfEntity) => {
                context.entity().map_or_else(Vec::new, |entity| {
                    if self.matches_entity(entity.as_ref(), position, aabb, server) {
                        vec![Arc::clone(entity)]
                    } else {
                        Vec::new()
                    }
                })
            }
            SelectorKind::Selector(_) if !self.includes_entities => self
                .candidate_players(server, context, cursor)?
                .into_iter()
                .map(|player| player as SharedEntity)
                .collect(),
            SelectorKind::Selector(_) => self.candidate_entities(server, context, cursor)?,
        };

        if !matches!(self.kind, SelectorKind::Selector(SelectorType::SelfEntity)) {
            entities.retain(|entity| self.matches_entity(entity.as_ref(), position, aabb, server));
        }
        self.sort_and_limit_entities(position, &mut entities);
        Ok(entities)
    }

    fn check_selector_permission(
        &self,
        context: &dyn CommandInputContext,
        cursor: usize,
    ) -> Result<(), CommandParseError> {
        if !matches!(self.kind, SelectorKind::Selector(_)) || allow_selectors(context) {
            return Ok(());
        }
        Err(CommandParseError::new(
            CommandParseErrorKind::EntitySelectorsNotAllowed,
            cursor,
        ))
    }

    fn candidate_players(
        &self,
        server: &Arc<Server>,
        context: &dyn CommandInputContext,
        cursor: usize,
    ) -> Result<Vec<Arc<Player>>, CommandParseError> {
        let mut players = server.get_players();
        if self.world_limited {
            let Some(world) = context.world() else {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::MissingCommandContext("world"),
                    cursor,
                ));
            };
            players.retain(|player| Arc::ptr_eq(&player.get_world(), world));
        }
        Ok(players)
    }

    fn candidate_entities(
        &self,
        server: &Arc<Server>,
        context: &dyn CommandInputContext,
        cursor: usize,
    ) -> Result<Vec<SharedEntity>, CommandParseError> {
        if self.world_limited {
            let Some(world) = context.world() else {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::MissingCommandContext("world"),
                    cursor,
                ));
            };
            return Ok(world.get_accessible_entities());
        }

        Ok(server
            .worlds
            .values()
            .flat_map(|world| world.get_accessible_entities())
            .collect())
    }

    fn absolute_aabb(&self, position: DVec3) -> Option<WorldAabb> {
        if self.delta.has_any() {
            return Some(self.delta.aabb().translate(position));
        }
        let max_distance = self.distance.and_then(|distance| distance.max)?;
        Some(
            WorldAabb::from_min_max(
                DVec3::splat(-max_distance),
                DVec3::splat(max_distance + 1.0),
            )
            .translate(position),
        )
    }

    fn requires_position(&self) -> bool {
        self.distance.is_some()
            || self.delta.has_any()
            || self
                .position
                .x
                .is_some_and(|_| self.position.y.is_none() || self.position.z.is_none())
            || self
                .position
                .y
                .is_some_and(|_| self.position.x.is_none() || self.position.z.is_none())
            || self
                .position
                .z
                .is_some_and(|_| self.position.x.is_none() || self.position.y.is_none())
            || matches!(self.order, SelectorOrder::Nearest | SelectorOrder::Furthest)
    }

    fn matches_entity(
        &self,
        entity: &dyn Entity,
        position: DVec3,
        aabb: Option<WorldAabb>,
        server: &Server,
    ) -> bool {
        if let Some(aabb) = aabb
            && !aabb.intersects(entity.bounding_box())
        {
            return false;
        }
        if let Some(distance) = self.distance
            && !distance.matches_squared(entity.position().distance_squared(position))
        {
            return false;
        }
        if let Some(level) = self.level {
            let Some(player) = entity.as_player() else {
                return false;
            };
            if !level.matches(player.experience.lock().level()) {
                return false;
            }
        }
        if let Some(range) = self.x_rotation
            && !range.matches_rotation(entity.rotation().1)
        {
            return false;
        }
        if let Some(range) = self.y_rotation
            && !range.matches_rotation(entity.rotation().0)
        {
            return false;
        }
        self.filters
            .iter()
            .all(|filter| filter.matches(entity, server))
    }

    fn sort_and_limit_players(&self, position: DVec3, players: &mut Vec<Arc<Player>>) {
        match self.order {
            SelectorOrder::Nearest => players.sort_by(|left, right| {
                left.position()
                    .distance_squared(position)
                    .total_cmp(&right.position().distance_squared(position))
            }),
            SelectorOrder::Furthest => players.sort_by(|left, right| {
                right
                    .position()
                    .distance_squared(position)
                    .total_cmp(&left.position().distance_squared(position))
            }),
            SelectorOrder::Random => players.shuffle(&mut rand::rng()),
            SelectorOrder::Arbitrary => {}
        }
        players.truncate(self.max_results);
    }

    fn sort_and_limit_entities(&self, position: DVec3, entities: &mut Vec<SharedEntity>) {
        match self.order {
            SelectorOrder::Nearest => entities.sort_by(|left, right| {
                left.position()
                    .distance_squared(position)
                    .total_cmp(&right.position().distance_squared(position))
            }),
            SelectorOrder::Furthest => entities.sort_by(|left, right| {
                right
                    .position()
                    .distance_squared(position)
                    .total_cmp(&left.position().distance_squared(position))
            }),
            SelectorOrder::Random => entities.shuffle(&mut rand::rng()),
            SelectorOrder::Arbitrary => {}
        }
        entities.truncate(self.max_results);
    }
}

impl SelectorFilter {
    fn matches(&self, entity: &dyn Entity, server: &Server) -> bool {
        match self {
            Self::Alive => entity.is_alive(),
            Self::Name { value, inverted } => entity_name_filter_matches(value, *inverted, entity),
            Self::GameMode { value, inverted } => {
                let matches = entity
                    .as_player()
                    .is_some_and(|player| player.game_mode() == *value);
                matches != *inverted
            }
            Self::EntityType { value, inverted } => {
                let matches = entity.entity_type() == *value;
                matches != *inverted
            }
            Self::EntityTypeTag { value, inverted } => {
                let matches = REGISTRY.entity_types.is_in_tag(entity.entity_type(), value);
                matches != *inverted
            }
            Self::Tag { value, inverted } => {
                let tags = entity.tags();
                let matches = if value.is_empty() {
                    tags.is_empty()
                } else {
                    tags.iter().any(|tag| tag == value)
                };
                matches != *inverted
            }
            Self::Nbt { value, inverted } => entity_nbt_filter_matches(value, *inverted, entity),
            Self::Scores(scores) => {
                let holder_name = entity.scoreboard_name();
                score_filter_matches(scores, &holder_name, &server.scoreboard)
            }
        }
    }
}

fn entity_nbt_filter_matches(expected: &NbtCompound, inverted: bool, entity: &dyn Entity) -> bool {
    let actual = entity.nbt_for_data_compare();
    compare_nbt_compounds(expected, &actual, true) != inverted
}

fn entity_name_filter_matches(value: &str, inverted: bool, entity: &dyn Entity) -> bool {
    (entity.plain_text_name() == value) != inverted
}

fn player_name_matches(actual: &str, expected: &str) -> bool {
    actual.eq_ignore_ascii_case(expected)
}

fn score_filter_matches(
    scores: &[(String, IntRange)],
    holder_name: &str,
    scoreboard: &Scoreboard,
) -> bool {
    let holder = ScoreHolder::new(holder_name.to_owned());
    scores.iter().all(|(objective_name, range)| {
        let Some(objective) = scoreboard.objective(objective_name) else {
            return false;
        };
        scoreboard
            .score(&holder, &objective)
            .is_some_and(|score| range.matches(score))
    })
}

pub(super) fn parse_player_selector(
    reader: &mut CommandReader<'_>,
    context: &dyn CommandInputContext,
    single: bool,
) -> Result<Vec<Arc<Player>>, CommandParseError> {
    let cursor = reader.absolute_cursor();
    let raw = read_selector_argument(reader)?;
    let selector = EntitySelector::parse(raw, cursor, allow_selectors(context))?;
    selector.validate_for_argument(single, true, cursor)?;
    let players = selector.find_players(context, cursor)?;
    if single && players.len() != 1 {
        return Err(CommandParseError::new(
            CommandParseErrorKind::InvalidPlayer(selector.raw),
            cursor,
        ));
    }
    Ok(players)
}

pub(super) fn parse_entity_selector(
    reader: &mut CommandReader<'_>,
    context: &dyn CommandInputContext,
    single: bool,
) -> Result<Vec<SharedEntity>, CommandParseError> {
    let cursor = reader.absolute_cursor();
    let raw = read_selector_argument(reader)?;
    let selector = EntitySelector::parse(raw, cursor, allow_selectors(context))?;
    selector.validate_for_argument(single, false, cursor)?;
    let entities = selector.find_entities(context, cursor)?;
    if single && entities.len() != 1 {
        return Err(CommandParseError::new(
            CommandParseErrorKind::InvalidEntity(selector.raw),
            cursor,
        ));
    }
    Ok(entities)
}

pub(super) fn allow_selectors(context: &dyn CommandInputContext) -> bool {
    let Ok(permission) = entity_selector_permission_expr() else {
        log::error!("built-in entity selector permission key is invalid");
        return false;
    };
    context.has_permission(&permission)
}

pub(super) fn selector_suggestions(
    players_only: bool,
    single: bool,
    context: &dyn CommandInputContext,
) -> Vec<&'static str> {
    if !allow_selectors(context) {
        return Vec::new();
    }
    match (players_only, single) {
        (true, true) => vec!["@p", "@r", "@s"],
        (true, false) => vec!["@a", "@p", "@r", "@s"],
        (false, true) => vec!["@p", "@r", "@s", "@n"],
        (false, false) => vec!["@a", "@e", "@p", "@r", "@s", "@n"],
    }
}

fn read_selector_argument(reader: &mut CommandReader<'_>) -> Result<String, CommandParseError> {
    if reader.peek() != Some('@') {
        return reader.read_string(StringMode::QuotablePhrase);
    }

    let start = reader.absolute_cursor();
    let mut value = String::new();
    let mut option_depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    while let Some(ch) = reader.peek() {
        if option_depth == 0 && !value.is_empty() && ch.is_whitespace() {
            break;
        }
        value.push(ch);
        reader.read();
        if escaped {
            escaped = false;
            continue;
        }
        if quote.is_some() {
            if ch == '\\' {
                escaped = true;
            } else if quote == Some(ch) {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '[' => option_depth += 1,
            ']' if option_depth > 0 => {
                option_depth -= 1;
                if option_depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }
    if value.is_empty() {
        return Err(CommandParseError::new(
            CommandParseErrorKind::ExpectedArgument,
            start,
        ));
    }
    Ok(value)
}

fn is_valid_selector_name(name: &str) -> bool {
    !name.is_empty() && name.encode_utf16().count() <= 16
}

fn parse_selector_plan(
    raw: String,
    allow_selectors: bool,
) -> Result<EntitySelector, SelectorParseError> {
    let mut selector = {
        let mut reader = SelectorReader::new(&raw);
        if reader.peek() == Some('@') {
            if !allow_selectors {
                return Err(SelectorParseError::not_allowed(reader.cursor()));
            }
            reader.read();
            parse_selector_type(&mut reader)?
        } else {
            parse_name_or_uuid(&mut reader)?
        }
    };
    selector.raw = raw;
    Ok(selector)
}

fn parse_name_or_uuid(
    reader: &mut SelectorReader<'_>,
) -> Result<EntitySelector, SelectorParseError> {
    let name = reader.read_remaining();
    if let Ok(uuid) = Uuid::parse_str(&name) {
        return Ok(EntitySelector {
            raw: String::new(),
            kind: SelectorKind::EntityUuid(uuid),
            max_results: 1,
            includes_entities: true,
            current_entity: false,
            world_limited: false,
            order: SelectorOrder::Arbitrary,
            position: SelectorPosition::default(),
            delta: SelectorDelta::default(),
            distance: None,
            level: None,
            x_rotation: None,
            y_rotation: None,
            filters: Vec::new(),
        });
    }
    if !is_valid_selector_name(&name) {
        return Err(SelectorParseError::invalid_at(
            "invalid player name or UUID",
            0,
        ));
    }
    Ok(EntitySelector {
        raw: String::new(),
        kind: SelectorKind::PlayerName(name),
        max_results: 1,
        includes_entities: false,
        current_entity: false,
        world_limited: false,
        order: SelectorOrder::Arbitrary,
        position: SelectorPosition::default(),
        delta: SelectorDelta::default(),
        distance: None,
        level: None,
        x_rotation: None,
        y_rotation: None,
        filters: Vec::new(),
    })
}

fn parse_selector_type(
    reader: &mut SelectorReader<'_>,
) -> Result<EntitySelector, SelectorParseError> {
    let selector_start = reader.cursor();
    let Some(selector_type) = reader.read() else {
        return Err(SelectorParseError::invalid_at(
            "missing selector type",
            selector_start,
        ));
    };

    let mut selector = match selector_type {
        'a' => EntitySelector {
            raw: String::new(),
            kind: SelectorKind::Selector(SelectorType::AllPlayers),
            max_results: usize::MAX,
            includes_entities: false,
            current_entity: false,
            world_limited: false,
            order: SelectorOrder::Arbitrary,
            position: SelectorPosition::default(),
            delta: SelectorDelta::default(),
            distance: None,
            level: None,
            x_rotation: None,
            y_rotation: None,
            filters: Vec::new(),
        },
        'e' => EntitySelector {
            raw: String::new(),
            kind: SelectorKind::Selector(SelectorType::AllEntities),
            max_results: usize::MAX,
            includes_entities: true,
            current_entity: false,
            world_limited: false,
            order: SelectorOrder::Arbitrary,
            position: SelectorPosition::default(),
            delta: SelectorDelta::default(),
            distance: None,
            level: None,
            x_rotation: None,
            y_rotation: None,
            filters: vec![SelectorFilter::Alive],
        },
        'n' => EntitySelector {
            raw: String::new(),
            kind: SelectorKind::Selector(SelectorType::NearestEntity),
            max_results: 1,
            includes_entities: true,
            current_entity: false,
            world_limited: false,
            order: SelectorOrder::Nearest,
            position: SelectorPosition::default(),
            delta: SelectorDelta::default(),
            distance: None,
            level: None,
            x_rotation: None,
            y_rotation: None,
            filters: vec![SelectorFilter::Alive],
        },
        'p' => EntitySelector {
            raw: String::new(),
            kind: SelectorKind::Selector(SelectorType::NearestPlayer),
            max_results: 1,
            includes_entities: false,
            current_entity: false,
            world_limited: false,
            order: SelectorOrder::Nearest,
            position: SelectorPosition::default(),
            delta: SelectorDelta::default(),
            distance: None,
            level: None,
            x_rotation: None,
            y_rotation: None,
            filters: Vec::new(),
        },
        'r' => EntitySelector {
            raw: String::new(),
            kind: SelectorKind::Selector(SelectorType::RandomPlayer),
            max_results: 1,
            includes_entities: false,
            current_entity: false,
            world_limited: false,
            order: SelectorOrder::Random,
            position: SelectorPosition::default(),
            delta: SelectorDelta::default(),
            distance: None,
            level: None,
            x_rotation: None,
            y_rotation: None,
            filters: Vec::new(),
        },
        's' => EntitySelector {
            raw: String::new(),
            kind: SelectorKind::Selector(SelectorType::SelfEntity),
            max_results: 1,
            includes_entities: true,
            current_entity: true,
            world_limited: false,
            order: SelectorOrder::Arbitrary,
            position: SelectorPosition::default(),
            delta: SelectorDelta::default(),
            distance: None,
            level: None,
            x_rotation: None,
            y_rotation: None,
            filters: Vec::new(),
        },
        other => {
            return Err(SelectorParseError::invalid_at(
                format!("unknown selector type '@{other}'"),
                selector_start,
            ));
        }
    };

    if reader.peek() == Some('[') {
        reader.read();
        parse_options(reader, &mut selector)?;
    }
    if reader.can_read() {
        return Err(SelectorParseError::invalid_at(
            "unexpected trailing selector data",
            reader.cursor(),
        ));
    }
    Ok(selector)
}

fn parse_options(
    reader: &mut SelectorReader<'_>,
    selector: &mut EntitySelector,
) -> Result<(), SelectorParseError> {
    let mut state = SelectorOptionState::default();
    reader.skip_whitespace();
    while reader.peek().is_some_and(|ch| ch != ']') {
        reader.skip_whitespace();
        let key_cursor = reader.cursor();
        let key = reader.read_key()?;
        reader.skip_whitespace();
        reader.expect('=')?;
        reader.skip_whitespace();
        parse_option(reader, selector, &mut state, &key, key_cursor)?;
        reader.skip_whitespace();
        match reader.peek() {
            Some(',') => {
                reader.read();
                reader.skip_whitespace();
            }
            Some(']') => break,
            Some(_) => {
                return Err(SelectorParseError::invalid_at(
                    "expected ',' or ']' after selector option",
                    reader.cursor(),
                ));
            }
            None => {
                return Err(SelectorParseError::invalid_at(
                    "expected ']' to end selector options",
                    reader.cursor(),
                ));
            }
        }
    }
    reader.expect(']')?;
    Ok(())
}

fn parse_option(
    reader: &mut SelectorReader<'_>,
    selector: &mut EntitySelector,
    state: &mut SelectorOptionState,
    key: &str,
    key_cursor: usize,
) -> Result<(), SelectorParseError> {
    match key {
        "name" => parse_name_option(reader, selector, state),
        "distance" => parse_distance_option(reader, selector, state, key_cursor),
        "level" => parse_level_option(reader, selector, state, key_cursor),
        "x" => {
            ensure_set_once(&mut state.x, "x", key_cursor)?;
            selector.world_limited = true;
            selector.position.x = Some(reader.read_f64()?);
            Ok(())
        }
        "y" => {
            ensure_set_once(&mut state.y, "y", key_cursor)?;
            selector.world_limited = true;
            selector.position.y = Some(reader.read_f64()?);
            Ok(())
        }
        "z" => {
            ensure_set_once(&mut state.z, "z", key_cursor)?;
            selector.world_limited = true;
            selector.position.z = Some(reader.read_f64()?);
            Ok(())
        }
        "dx" => {
            ensure_set_once(&mut state.dx, "dx", key_cursor)?;
            selector.world_limited = true;
            selector.delta.x = Some(reader.read_f64()?);
            Ok(())
        }
        "dy" => {
            ensure_set_once(&mut state.dy, "dy", key_cursor)?;
            selector.world_limited = true;
            selector.delta.y = Some(reader.read_f64()?);
            Ok(())
        }
        "dz" => {
            ensure_set_once(&mut state.dz, "dz", key_cursor)?;
            selector.world_limited = true;
            selector.delta.z = Some(reader.read_f64()?);
            Ok(())
        }
        "x_rotation" => {
            ensure_set_once(&mut state.x_rotation, "x_rotation", key_cursor)?;
            selector.x_rotation = Some(parse_float_range(&reader.read_raw_value()?)?);
            Ok(())
        }
        "y_rotation" => {
            ensure_set_once(&mut state.y_rotation, "y_rotation", key_cursor)?;
            selector.y_rotation = Some(parse_float_range(&reader.read_raw_value()?)?);
            Ok(())
        }
        "limit" => parse_limit_option(reader, selector, state, key_cursor),
        "sort" => parse_sort_option(reader, selector, state, key_cursor),
        "gamemode" => parse_gamemode_option(reader, selector, state),
        "type" => parse_type_option(reader, selector, state),
        "tag" => parse_tag_option(reader, selector),
        "nbt" => parse_nbt_option(reader, selector),
        "scores" => parse_scores_option(reader, selector, state, key_cursor),
        "team" => Err(SelectorParseError::unsupported(
            "team needs scoreboard team foundation",
            key_cursor,
        )),
        "advancements" => Err(SelectorParseError::unsupported(
            "advancements needs player advancement foundation",
            key_cursor,
        )),
        "predicate" => Err(SelectorParseError::unsupported(
            "predicate needs loot predicate registry foundation",
            key_cursor,
        )),
        _ => Err(SelectorParseError::invalid_at(
            format!("unknown selector option '{key}'"),
            key_cursor,
        )),
    }
}

fn parse_name_option(
    reader: &mut SelectorReader<'_>,
    selector: &mut EntitySelector,
    state: &mut SelectorOptionState,
) -> Result<(), SelectorParseError> {
    let value_cursor = reader.cursor();
    let inverted = reader.read_inversion();
    state
        .name
        .parse_element(inverted, "name")
        .map_err(|error| {
            SelectorParseError::invalid_at(
                match error.kind {
                    SelectorParseErrorKind::Invalid(message) => message,
                    SelectorParseErrorKind::NotAllowed | SelectorParseErrorKind::Unsupported(_) => {
                        "invalid name option".to_owned()
                    }
                },
                value_cursor,
            )
        })?;
    let value = reader.read_string_value()?;
    selector
        .filters
        .push(SelectorFilter::Name { value, inverted });
    Ok(())
}

fn parse_scores_option(
    reader: &mut SelectorReader<'_>,
    selector: &mut EntitySelector,
    state: &mut SelectorOptionState,
    key_cursor: usize,
) -> Result<(), SelectorParseError> {
    ensure_set_once(&mut state.scores, "scores", key_cursor)?;
    let scores = reader.read_scores()?;
    if !scores.is_empty() {
        selector.filters.push(SelectorFilter::Scores(scores));
    }
    Ok(())
}

fn parse_distance_option(
    reader: &mut SelectorReader<'_>,
    selector: &mut EntitySelector,
    state: &mut SelectorOptionState,
    key_cursor: usize,
) -> Result<(), SelectorParseError> {
    ensure_set_once(&mut state.distance, "distance", key_cursor)?;
    let range = parse_double_range(&reader.read_raw_value()?)?;
    if range.min.is_some_and(|value| value < 0.0) || range.max.is_some_and(|value| value < 0.0) {
        return Err(SelectorParseError::invalid_at(
            "distance cannot be negative",
            key_cursor,
        ));
    }
    selector.distance = Some(range);
    selector.world_limited = true;
    Ok(())
}

fn parse_level_option(
    reader: &mut SelectorReader<'_>,
    selector: &mut EntitySelector,
    state: &mut SelectorOptionState,
    key_cursor: usize,
) -> Result<(), SelectorParseError> {
    ensure_set_once(&mut state.level, "level", key_cursor)?;
    let range = parse_int_range(&reader.read_raw_value()?)?;
    if range.min.is_some_and(|value| value < 0) || range.max.is_some_and(|value| value < 0) {
        return Err(SelectorParseError::invalid_at(
            "level cannot be negative",
            key_cursor,
        ));
    }
    selector.level = Some(range);
    selector.includes_entities = false;
    Ok(())
}

fn parse_limit_option(
    reader: &mut SelectorReader<'_>,
    selector: &mut EntitySelector,
    state: &mut SelectorOptionState,
    key_cursor: usize,
) -> Result<(), SelectorParseError> {
    if selector.current_entity {
        return Err(SelectorParseError::invalid_at(
            "limit cannot be used with @s",
            key_cursor,
        ));
    }
    ensure_set_once(&mut state.limit, "limit", key_cursor)?;
    let value = reader.read_i32()?;
    if value < 1 {
        return Err(SelectorParseError::invalid_at(
            "limit must be at least 1",
            key_cursor,
        ));
    }
    selector.max_results = value as usize;
    Ok(())
}

fn parse_sort_option(
    reader: &mut SelectorReader<'_>,
    selector: &mut EntitySelector,
    state: &mut SelectorOptionState,
    key_cursor: usize,
) -> Result<(), SelectorParseError> {
    if selector.current_entity {
        return Err(SelectorParseError::invalid_at(
            "sort cannot be used with @s",
            key_cursor,
        ));
    }
    ensure_set_once(&mut state.sort, "sort", key_cursor)?;
    let value = reader.read_raw_value()?;
    selector.order = match value.as_str() {
        SORT_NEAREST => SelectorOrder::Nearest,
        SORT_FURTHEST => SelectorOrder::Furthest,
        SORT_RANDOM => SelectorOrder::Random,
        SORT_ARBITRARY => SelectorOrder::Arbitrary,
        _ => {
            return Err(SelectorParseError::invalid_at(
                format!("unknown sort '{value}'"),
                key_cursor,
            ));
        }
    };
    Ok(())
}

fn parse_gamemode_option(
    reader: &mut SelectorReader<'_>,
    selector: &mut EntitySelector,
    state: &mut SelectorOptionState,
) -> Result<(), SelectorParseError> {
    let value_cursor = reader.cursor();
    let inverted = reader.read_inversion();
    state.gamemode.parse_element(inverted, "gamemode")?;
    let value = reader.read_raw_value()?;
    let Some(game_mode) = parse_game_mode(&value) else {
        return Err(SelectorParseError::invalid_at(
            format!("invalid game mode '{value}'"),
            value_cursor,
        ));
    };
    selector.includes_entities = false;
    selector.filters.push(SelectorFilter::GameMode {
        value: game_mode,
        inverted,
    });
    Ok(())
}

fn parse_type_option(
    reader: &mut SelectorReader<'_>,
    selector: &mut EntitySelector,
    state: &mut SelectorOptionState,
) -> Result<(), SelectorParseError> {
    let value_cursor = reader.cursor();
    let inverted = reader.read_inversion();
    if reader.peek() == Some('#') {
        reader.read();
        let value = read_identifier(reader, value_cursor)?;
        state.entity_type.parse_tag(&value, "type")?;
        selector
            .filters
            .push(SelectorFilter::EntityTypeTag { value, inverted });
        return Ok(());
    }

    state.entity_type.parse_element(inverted, "type")?;
    let key = read_identifier(reader, value_cursor)?;
    let Some(entity_type) = REGISTRY.entity_types.by_key(&key) else {
        return Err(SelectorParseError::invalid_at(
            format!("invalid entity type '{key}'"),
            value_cursor,
        ));
    };
    if entity_type == &vanilla_entities::PLAYER && !inverted {
        selector.includes_entities = false;
    }
    selector.filters.push(SelectorFilter::EntityType {
        value: entity_type,
        inverted,
    });
    Ok(())
}

fn parse_tag_option(
    reader: &mut SelectorReader<'_>,
    selector: &mut EntitySelector,
) -> Result<(), SelectorParseError> {
    let inverted = reader.read_inversion();
    let value = reader.read_raw_value_allow_empty()?;
    selector
        .filters
        .push(SelectorFilter::Tag { value, inverted });
    Ok(())
}

fn parse_nbt_option(
    reader: &mut SelectorReader<'_>,
    selector: &mut EntitySelector,
) -> Result<(), SelectorParseError> {
    let inverted = reader.read_inversion();
    let value = reader.read_nbt()?;
    selector
        .filters
        .push(SelectorFilter::Nbt { value, inverted });
    Ok(())
}

fn read_identifier(
    reader: &mut SelectorReader<'_>,
    value_cursor: usize,
) -> Result<Identifier, SelectorParseError> {
    let value = reader.read_raw_value()?;
    parse_resource_identifier(&value).ok_or_else(|| {
        SelectorParseError::invalid_at(format!("invalid identifier '{value}'"), value_cursor)
    })
}

fn ensure_set_once(seen: &mut bool, option: &str, cursor: usize) -> Result<(), SelectorParseError> {
    if *seen {
        return Err(SelectorParseError::invalid_at(
            format!("option '{option}' cannot be repeated"),
            cursor,
        ));
    }
    *seen = true;
    Ok(())
}

fn selector_position(
    selector: &EntitySelector,
    context: &dyn CommandInputContext,
    cursor: usize,
) -> Result<DVec3, CommandParseError> {
    if !selector.requires_position() {
        return Ok(selector.position.apply(DVec3::ZERO));
    }
    let Some(position) = context.position() else {
        return Err(CommandParseError::new(
            CommandParseErrorKind::MissingCommandContext("position"),
            cursor,
        ));
    };
    Ok(selector.position.apply(position))
}

fn find_entity_by_uuid(server: &Server, uuid: &Uuid) -> Option<SharedEntity> {
    server
        .worlds
        .values()
        .find_map(|world| world.get_entity_by_uuid(uuid))
}

fn create_delta_aabb(x: f64, y: f64, z: f64) -> WorldAabb {
    let min = DVec3::new(
        if x < 0.0 { x } else { 0.0 },
        if y < 0.0 { y } else { 0.0 },
        if z < 0.0 { z } else { 0.0 },
    );
    let max = DVec3::new(
        if x < 0.0 { 0.0 } else { x } + 1.0,
        if y < 0.0 { 0.0 } else { y } + 1.0,
        if z < 0.0 { 0.0 } else { z } + 1.0,
    );
    WorldAabb::from_min_max(min, max)
}

fn parse_game_mode(value: &str) -> Option<GameType> {
    match value {
        "survival" => Some(GameType::Survival),
        "creative" => Some(GameType::Creative),
        "adventure" => Some(GameType::Adventure),
        "spectator" => Some(GameType::Spectator),
        _ => None,
    }
}

fn parse_double_range(raw: &str) -> Result<DoubleRange, SelectorParseError> {
    let (min, max) = parse_range(raw, |value| value.parse::<f64>())?;
    if let (Some(min), Some(max)) = (min, max)
        && min > max
    {
        return Err(SelectorParseError::invalid("range minimum exceeds maximum"));
    }
    Ok(DoubleRange { min, max })
}

fn parse_float_range(raw: &str) -> Result<FloatRange, SelectorParseError> {
    let (min, max) = parse_range(raw, |value| value.parse::<f32>())?;
    Ok(FloatRange { min, max })
}

fn parse_int_range(raw: &str) -> Result<IntRange, SelectorParseError> {
    let (min, max) = parse_range(raw, |value| value.parse::<i32>())?;
    if let (Some(min), Some(max)) = (min, max)
        && min > max
    {
        return Err(SelectorParseError::invalid("range minimum exceeds maximum"));
    }
    Ok(IntRange { min, max })
}

fn parse_range<T: Copy, E>(
    raw: &str,
    parse: impl Fn(&str) -> Result<T, E>,
) -> Result<(Option<T>, Option<T>), SelectorParseError> {
    if raw.is_empty() {
        return Err(SelectorParseError::invalid("missing range value"));
    }
    let Some((left, right)) = raw.split_once("..") else {
        let value = parse(raw).map_err(|_| SelectorParseError::invalid("invalid range value"))?;
        return Ok((Some(value), Some(value)));
    };
    if left.is_empty() && right.is_empty() {
        return Err(SelectorParseError::invalid("empty range"));
    }
    let min = if left.is_empty() {
        None
    } else {
        Some(parse(left).map_err(|_| SelectorParseError::invalid("invalid range minimum"))?)
    };
    let max = if right.is_empty() {
        None
    } else {
        Some(parse(right).map_err(|_| SelectorParseError::invalid("invalid range maximum"))?)
    };
    Ok((min, max))
}

fn wrap_degrees(value: f32) -> f32 {
    let mut value = value % 360.0;
    if value >= 180.0 {
        value -= 360.0;
    }
    if value < -180.0 {
        value += 360.0;
    }
    value
}

#[derive(Clone)]
struct SelectorReader<'a> {
    input: &'a str,
    cursor: usize,
}

impl<'a> SelectorReader<'a> {
    const fn new(input: &'a str) -> Self {
        Self { input, cursor: 0 }
    }

    const fn cursor(&self) -> usize {
        self.cursor
    }

    const fn can_read(&self) -> bool {
        self.cursor < self.input.len()
    }

    fn remaining(&self) -> &'a str {
        &self.input[self.cursor..]
    }

    fn peek(&self) -> Option<char> {
        self.remaining().chars().next()
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

    fn expect(&mut self, expected: char) -> Result<(), SelectorParseError> {
        if self.peek() == Some(expected) {
            self.read();
            Ok(())
        } else {
            Err(SelectorParseError::invalid_at(
                format!("expected '{expected}'"),
                self.cursor,
            ))
        }
    }

    fn read_remaining(&mut self) -> String {
        let value = self.remaining().to_owned();
        self.cursor = self.input.len();
        value
    }

    fn read_key(&mut self) -> Result<String, SelectorParseError> {
        let start = self.cursor;
        while self
            .peek()
            .is_some_and(|ch| ch != '=' && ch != ',' && ch != ']' && !ch.is_whitespace())
        {
            self.read();
        }
        if self.cursor == start {
            return Err(SelectorParseError::invalid_at(
                "expected selector option name",
                start,
            ));
        }
        Ok(self.input[start..self.cursor].to_owned())
    }

    fn read_scores(&mut self) -> Result<Vec<(String, IntRange)>, SelectorParseError> {
        self.expect('{')?;
        let mut scores = Vec::new();
        self.skip_whitespace();
        while self.peek().is_some_and(|ch| ch != '}') {
            self.skip_whitespace();
            let name_cursor = self.cursor;
            let name = self.read_unquoted_string();
            if name.is_empty() {
                return Err(SelectorParseError::invalid_at(
                    "expected scoreboard objective name",
                    name_cursor,
                ));
            }
            self.skip_whitespace();
            self.expect('=')?;
            self.skip_whitespace();
            let range = parse_int_range(&self.read_score_range()?)?;
            upsert_score_filter(&mut scores, name, range);
            self.skip_whitespace();
            if self.peek() == Some(',') {
                self.read();
                self.skip_whitespace();
            }
        }
        self.expect('}')?;
        Ok(scores)
    }

    fn read_unquoted_string(&mut self) -> String {
        let start = self.cursor;
        while self.peek().is_some_and(is_brigadier_unquoted_char) {
            self.read();
        }
        self.input[start..self.cursor].to_owned()
    }

    fn read_score_range(&mut self) -> Result<String, SelectorParseError> {
        let start = self.cursor;
        while self
            .peek()
            .is_some_and(|ch| ch != ',' && ch != '}' && !ch.is_whitespace())
        {
            self.read();
        }
        if self.cursor == start {
            return Err(SelectorParseError::invalid_at(
                "expected score range",
                start,
            ));
        }
        Ok(self.input[start..self.cursor].to_owned())
    }

    fn read_nbt(&mut self) -> Result<NbtCompound, SelectorParseError> {
        let nbt_cursor = self.cursor;
        let (nbt, consumed) =
            parse_snbt_compound_argument(&self.input[self.cursor..]).map_err(|error| {
                SelectorParseError::invalid_at(
                    format!("invalid entity selector NBT: {}", error.message()),
                    nbt_cursor + error.cursor(),
                )
            })?;
        self.cursor += consumed;
        Ok(nbt)
    }

    fn read_inversion(&mut self) -> bool {
        self.skip_whitespace();
        if self.peek() == Some('!') {
            self.read();
            self.skip_whitespace();
            true
        } else {
            false
        }
    }

    fn read_i32(&mut self) -> Result<i32, SelectorParseError> {
        let cursor = self.cursor;
        let value = self.read_raw_value()?;
        value.parse().map_err(|_| {
            SelectorParseError::invalid_at(format!("invalid integer '{value}'"), cursor)
        })
    }

    fn read_f64(&mut self) -> Result<f64, SelectorParseError> {
        let cursor = self.cursor;
        let value = self.read_raw_value()?;
        value.parse().map_err(|_| {
            SelectorParseError::invalid_at(format!("invalid double '{value}'"), cursor)
        })
    }

    fn read_string_value(&mut self) -> Result<String, SelectorParseError> {
        self.skip_whitespace();
        match self.peek() {
            Some('"') | Some('\'') => self.read_quoted_string(),
            _ => self.read_raw_value(),
        }
    }

    fn read_raw_value(&mut self) -> Result<String, SelectorParseError> {
        let value = self.read_raw_value_allow_empty()?;
        if value.is_empty() {
            return Err(SelectorParseError::invalid_at(
                "expected selector option value",
                self.cursor,
            ));
        }
        Ok(value)
    }

    fn read_raw_value_allow_empty(&mut self) -> Result<String, SelectorParseError> {
        self.skip_whitespace();
        let start = self.cursor;
        while self
            .peek()
            .is_some_and(|ch| ch != ',' && ch != ']' && !ch.is_whitespace())
        {
            self.read();
        }
        Ok(self.input[start..self.cursor].to_owned())
    }

    fn read_quoted_string(&mut self) -> Result<String, SelectorParseError> {
        let start = self.cursor;
        let Some(quote) = self.read() else {
            return Err(SelectorParseError::invalid_at(
                "expected quoted string",
                start,
            ));
        };
        let mut value = String::new();
        while let Some(ch) = self.read() {
            match ch {
                ch if ch == quote => return Ok(value),
                '\\' => {
                    let Some(escaped) = self.read() else {
                        return Err(SelectorParseError::invalid_at("unclosed quote", start));
                    };
                    if escaped != quote && escaped != '\\' {
                        return Err(SelectorParseError::invalid_at(
                            format!("invalid escape '{escaped}'"),
                            self.cursor,
                        ));
                    }
                    value.push(escaped);
                }
                _ => value.push(ch),
            }
        }
        Err(SelectorParseError::invalid_at("unclosed quote", start))
    }
}

fn upsert_score_filter(scores: &mut Vec<(String, IntRange)>, name: String, range: IntRange) {
    if let Some((_, existing)) = scores
        .iter_mut()
        .find(|(existing_name, _)| existing_name == &name)
    {
        *existing = range;
        return;
    }
    scores.push((name, range));
}

fn is_brigadier_unquoted_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '+')
}

#[cfg(test)]
mod tests {
    use std::sync::Weak;

    use glam::DVec3;
    use simdnbt::owned::{NbtCompound, NbtTag};
    use steel_registry::{
        entity_type::EntityTypeRef, test_support::init_test_registry, vanilla_entities,
    };
    use text_components::TextComponent;

    use crate::{
        command::reader::CommandReader,
        entity::{Entity, EntityBase},
        scoreboard::{ScoreHolder, Scoreboard},
    };

    use super::{
        IntRange, SelectorFilter, SelectorParseErrorKind, SelectorType, entity_name_filter_matches,
        entity_nbt_filter_matches, parse_selector_plan, player_name_matches,
        read_selector_argument, score_filter_matches,
    };

    struct SelectorNbtTestEntity {
        base: EntityBase,
    }

    impl SelectorNbtTestEntity {
        fn new() -> Self {
            Self {
                base: EntityBase::new(
                    1,
                    DVec3::ZERO,
                    vanilla_entities::ITEM.dimensions,
                    Weak::new(),
                ),
            }
        }
    }

    impl Entity for SelectorNbtTestEntity {
        fn base(&self) -> &EntityBase {
            &self.base
        }

        fn entity_type(&self) -> EntityTypeRef {
            &vanilla_entities::ITEM
        }
    }

    #[test]
    fn selector_permission_gate_rejects_selector_syntax() {
        let error = parse_selector_plan("@a".to_owned(), false).expect_err("selector rejected");
        assert!(matches!(error.kind, SelectorParseErrorKind::NotAllowed));
    }

    #[test]
    fn selector_direct_names_use_vanilla_name_or_uuid_syntax() {
        let short = parse_selector_plan("ab".to_owned(), false).expect("short name parses");
        assert!(matches!(
            short.kind,
            super::SelectorKind::PlayerName(ref name) if name == "ab"
        ));

        let dashed =
            parse_selector_plan("name-with-dash".to_owned(), false).expect("dashed name parses");
        assert!(matches!(
            dashed.kind,
            super::SelectorKind::PlayerName(ref name) if name == "name-with-dash"
        ));

        let error = parse_selector_plan("way_too_long_player_name".to_owned(), false)
            .expect_err("too-long name is invalid");
        assert!(matches!(error.kind, SelectorParseErrorKind::Invalid(_)));
    }

    #[test]
    fn selector_direct_player_name_matching_is_case_insensitive() {
        assert!(player_name_matches("Steve", "steve"));
        assert!(player_name_matches("STEVE", "Steve"));
        assert!(!player_name_matches("Alex", "Steve"));
    }

    #[test]
    fn selector_parses_vanilla_core_options() {
        init_test_registry();

        let selector = parse_selector_plan(
            "@e[type=pig,distance=..5,limit=2,sort=nearest]".to_owned(),
            true,
        )
        .expect("selector parses");

        assert!(matches!(
            selector.kind,
            super::SelectorKind::Selector(SelectorType::AllEntities)
        ));
        assert_eq!(selector.max_results, 2);
        assert!(selector.includes_entities);
        assert!(selector.world_limited);
        assert!(selector.distance.is_some());
    }

    #[test]
    fn selector_player_filters_limit_to_players() {
        let selector = parse_selector_plan("@a[gamemode=!spectator,level=3..]".to_owned(), true)
            .expect("selector parses");

        assert!(!selector.includes_entities);
        assert!(selector.level.is_some());
    }

    #[test]
    fn selector_parses_name_on_broad_entity_selector() {
        let selector =
            parse_selector_plan("@e[name=Steve]".to_owned(), true).expect("selector parses");

        assert!(selector.includes_entities);
        assert!(selector.filters.iter().any(
            |filter| matches!(filter, SelectorFilter::Name { value, .. } if value == "Steve")
        ));
    }

    #[test]
    fn selector_rejects_duplicate_entity_type_tags() {
        init_test_registry();

        let error = parse_selector_plan(
            "@e[type=#minecraft:skeletons,type=!#minecraft:skeletons]".to_owned(),
            true,
        )
        .expect_err("duplicate type tag is rejected");
        assert!(matches!(error.kind, SelectorParseErrorKind::Invalid(_)));

        parse_selector_plan(
            "@e[type=#minecraft:skeletons,type=!#minecraft:raiders,type=!zombie]".to_owned(),
            true,
        )
        .expect("distinct tags and inverted entities can repeat");
    }

    #[test]
    fn selector_name_filter_matches_custom_entity_names() {
        let entity = SelectorNbtTestEntity::new();
        entity.set_custom_name(Some(TextComponent::plain("Named item")));

        assert!(entity_name_filter_matches("Named item", false, &entity));
        assert!(!entity_name_filter_matches("Item", false, &entity));
        assert!(entity_name_filter_matches("Other", true, &entity));
    }

    #[test]
    fn selector_name_filter_uses_entity_type_plain_name() {
        let entity = SelectorNbtTestEntity::new();

        assert_eq!(entity.plain_text_name(), "Item");
        assert!(entity_name_filter_matches("Item", false, &entity));
    }

    #[test]
    fn selector_parses_score_filters() {
        let selector = parse_selector_plan(
            "@e[scores={kills=1..,deaths=..2,kills=5,}]".to_owned(),
            true,
        )
        .expect("score filter parses");

        let Some(SelectorFilter::Scores(scores)) = selector
            .filters
            .iter()
            .find(|filter| matches!(filter, SelectorFilter::Scores(_)))
        else {
            panic!("expected scores filter");
        };
        assert_eq!(scores.len(), 2);
        assert!(
            scores.iter().any(|(name, range)| name == "kills"
                && range.min == Some(5)
                && range.max == Some(5))
        );
        assert!(
            scores.iter().any(|(name, range)| name == "deaths"
                && range.min.is_none()
                && range.max == Some(2))
        );
    }

    #[test]
    fn selector_parses_repeated_nbt_filters() {
        let selector = parse_selector_plan(
            "@e[nbt={Tags:[\"foo\"]},nbt=!{NoGravity:1b}]".to_owned(),
            true,
        )
        .expect("nbt filters parse");

        let filters = selector
            .filters
            .iter()
            .filter_map(|filter| match filter {
                SelectorFilter::Nbt { inverted, .. } => Some(*inverted),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(filters, vec![false, true]);
    }

    #[test]
    fn selector_argument_reader_keeps_nested_snbt() {
        let mut reader = CommandReader::new("@e[nbt={Tags:[\"foo]bar\"],data:{x:1b}}] next");
        let raw = read_selector_argument(&mut reader).expect("selector argument reads");

        assert_eq!(raw, "@e[nbt={Tags:[\"foo]bar\"],data:{x:1b}}]");
        assert_eq!(reader.remaining(), " next");
    }

    #[test]
    fn selector_argument_reader_reads_quoted_direct_names() {
        let mut reader = CommandReader::new("\"ab\" next");
        let raw = read_selector_argument(&mut reader).expect("selector argument reads");

        assert_eq!(raw, "ab");
        assert_eq!(reader.remaining(), " next");
    }

    #[test]
    fn selector_nbt_filter_matches_entity_compare_data() {
        init_test_registry();
        let entity = SelectorNbtTestEntity::new();
        let mut custom_data = NbtCompound::new();
        custom_data.insert("flag", NbtTag::Byte(1));
        entity.base.set_custom_data(custom_data);

        let mut expected_data = NbtCompound::new();
        expected_data.insert("flag", NbtTag::Byte(1));
        let mut expected = NbtCompound::new();
        expected.insert("data", NbtTag::Compound(expected_data));

        assert!(entity_nbt_filter_matches(&expected, false, &entity));
        assert!(!entity_nbt_filter_matches(&expected, true, &entity));
    }

    #[test]
    fn selector_rejects_repeated_score_filter() {
        let error = parse_selector_plan(
            "@e[scores={kills=1..},scores={deaths=..2}]".to_owned(),
            true,
        )
        .expect_err("scores option can only be used once");
        assert!(matches!(error.kind, SelectorParseErrorKind::Invalid(_)));
    }

    #[test]
    fn selector_score_filter_matches_scoreboard_holder() {
        let scoreboard = Scoreboard::new();
        let kills = scoreboard
            .add_objective("kills")
            .expect("objective should be added");
        let deaths = scoreboard
            .add_objective("deaths")
            .expect("objective should be added");
        let steve = ScoreHolder::new("Steve");
        scoreboard
            .set_score(&steve, &kills, 5)
            .expect("score should be writable");
        scoreboard
            .set_score(&steve, &deaths, 1)
            .expect("score should be writable");

        let filters = vec![
            (kills.name().to_owned(), IntRange::exactly(5)),
            (
                deaths.name().to_owned(),
                IntRange {
                    min: None,
                    max: Some(2),
                },
            ),
        ];
        assert!(score_filter_matches(&filters, steve.name(), &scoreboard));

        let filters = vec![(kills.name().to_owned(), IntRange::exactly(4))];
        assert!(!score_filter_matches(&filters, steve.name(), &scoreboard));

        let filters = vec![("missing".to_owned(), IntRange::exactly(1))];
        assert!(!score_filter_matches(&filters, steve.name(), &scoreboard));
    }

    #[test]
    fn selector_rejects_missing_runtime_foundations_by_option_name() {
        let error = parse_selector_plan("@e[predicate=minecraft:test]".to_owned(), true)
            .expect_err("predicate needs predicate foundation");
        assert!(
            matches!(error.kind, SelectorParseErrorKind::Unsupported(message) if message == "predicate needs loot predicate registry foundation")
        );
    }
}
