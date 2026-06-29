use std::{fmt, sync::Arc};

use glam::DVec3;
use simdnbt::owned::{NbtCompound, NbtTag};
use steel_registry::{
    REGISTRY, biome::BiomeRef, blocks::BlockRef, enchantment::EnchantmentRef,
    entity_type::EntityTypeRef, items::ItemRef, structure::StructureRef,
};
use steel_utils::{BlockPos, BlockStateId, Identifier, nbt::NbtPath, types::GameType};
use text_components::TextComponent;
use uuid::Uuid;

use crate::chunk::heightmap::HeightmapType;
use crate::command::context::EntityAnchor;
use crate::entity::SharedEntity;
use crate::permission::{
    PermissionKey, PermissionKeyError, PermissionMetadataExpression, PermissionRuleExpression,
    PermissionSegment,
};
use crate::player::Player;
use crate::scoreboard::ScoreHolder;
use crate::world::World;

/// A parsed command argument value.
#[derive(Clone)]
pub enum ParsedArgument {
    /// Boolean argument.
    Bool(bool),
    /// Entity anchor argument.
    Anchor(EntityAnchor),
    /// 32-bit signed integer argument.
    I32(i32),
    /// 64-bit signed integer argument.
    I64(i64),
    /// 32-bit floating-point argument.
    F32(f32),
    /// 64-bit floating-point argument.
    F64(f64),
    /// String-like argument.
    String(String),
    /// Namespaced identifier argument.
    Identifier(Identifier),
    /// Permission key argument.
    PermissionKey(PermissionKey),
    /// Permission rule expression argument.
    PermissionRuleExpression(PermissionRuleExpression),
    /// Permission metadata expression argument.
    PermissionMetadataExpression(PermissionMetadataExpression),
    /// Game mode argument.
    GameMode(GameType),
    /// Player target argument.
    Players(Vec<Arc<Player>>),
    /// Permission-management player target argument.
    PermissionTargets(Vec<PermissionTarget>),
    /// Living entity target argument.
    Entities(Vec<SharedEntity>),
    /// Entity type argument.
    EntityType(EntityTypeRef),
    /// Item argument.
    Item(ItemRef),
    /// Item slot range argument.
    ItemSlots(ItemSlotRangeArgumentValue),
    /// Item predicate argument.
    ItemPredicate(ItemPredicateArgumentValue),
    /// Enchantment argument.
    Enchantment(EnchantmentRef),
    /// Biome or biome tag argument.
    Biome(BiomeArgumentValue),
    /// Block predicate argument.
    BlockPredicate(BlockPredicateArgumentValue),
    /// NBT path argument.
    NbtPath(NbtPath),
    /// Structure or structure tag argument.
    Structure(StructureArgumentValue),
    /// Loaded world argument.
    World(Arc<World>),
    /// 3D vector argument.
    Vec3(DVec3),
    /// Block position argument.
    BlockPos(BlockPos),
    /// Heightmap type argument.
    Heightmap(HeightmapType),
    /// Rotation argument.
    Rotation((f32, f32)),
    /// Text component argument.
    Component(Box<TextComponent>),
    /// Scoreboard objective name argument.
    ScoreboardObjective(String),
    /// Score holder argument.
    ScoreHolders(ScoreHolderArgumentValue),
    /// Integer range argument.
    IntRange(IntRangeArgumentValue),
}

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

/// Item slot range command argument value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemSlotRangeArgumentValue {
    name: String,
    slots: Vec<i32>,
}

/// Item predicate command argument value.
#[derive(Clone, Debug)]
pub struct ItemPredicateArgumentValue {
    target: ItemPredicateTarget,
    conditions: Vec<ItemPredicateCondition>,
}

/// Item predicate target selector.
#[derive(Clone, Debug)]
pub enum ItemPredicateTarget {
    /// Any item type.
    Any,
    /// A single item type.
    Item(ItemRef),
    /// An item tag.
    Tag {
        /// Tag key without the leading `#`.
        key: Identifier,
        /// Items in the tag.
        items: Vec<ItemRef>,
    },
}

/// One AND-ed item predicate condition group.
#[derive(Clone, Debug)]
pub struct ItemPredicateCondition {
    alternatives: Vec<ItemPredicateTerm>,
}

/// One item predicate term.
#[derive(Clone, Debug)]
pub enum ItemPredicateTerm {
    /// Component presence test.
    ComponentPresence {
        /// Component key.
        key: Identifier,
    },
    /// Component exact-value test using parsed SNBT.
    ComponentValue {
        /// Component key.
        key: Identifier,
        /// Expected component value.
        value: NbtTag,
    },
    /// Component predicate test using parsed SNBT.
    PredicateValue {
        /// Predicate key.
        key: Identifier,
        /// Expected predicate value.
        value: NbtTag,
    },
    /// Negated test term.
    Not(Box<ItemPredicateTerm>),
}

/// Player target for permission-management commands.
#[derive(Clone)]
pub struct PermissionTarget {
    uuid: Option<Uuid>,
    name: String,
}

/// Score holder command argument value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScoreHolderArgumentValue {
    /// Explicit score holders.
    Holders(Vec<ScoreHolder>),
    /// Wildcard holder expansion.
    Wildcard,
}

impl ScoreHolderArgumentValue {
    /// Returns explicit holders.
    #[must_use]
    pub fn holders(&self) -> Option<&[ScoreHolder]> {
        match self {
            Self::Holders(holders) => Some(holders),
            Self::Wildcard => None,
        }
    }

    /// Returns whether this argument is a wildcard.
    #[must_use]
    pub const fn is_wildcard(&self) -> bool {
        matches!(self, Self::Wildcard)
    }
}

/// Scoreboard objective name command argument value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoreboardObjectiveName(String);

impl ScoreboardObjectiveName {
    /// Creates an objective name value.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Returns the objective name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Inclusive integer range command argument value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntRangeArgumentValue {
    min: Option<i32>,
    max: Option<i32>,
}

impl IntRangeArgumentValue {
    /// Creates an integer range.
    #[must_use]
    pub const fn new(min: Option<i32>, max: Option<i32>) -> Self {
        Self { min, max }
    }

    /// Creates an exact-value range.
    #[must_use]
    pub const fn exactly(value: i32) -> Self {
        Self {
            min: Some(value),
            max: Some(value),
        }
    }

    /// Returns whether `value` matches this range.
    #[must_use]
    pub fn matches(self, value: i32) -> bool {
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

impl PermissionTarget {
    /// Creates a target from an online player.
    #[must_use]
    pub fn online(player: &Player) -> Self {
        Self {
            uuid: Some(player.gameprofile.id),
            name: player.gameprofile.name.clone(),
        }
    }

    /// Creates a target from a known offline profile.
    #[must_use]
    pub fn offline(uuid: Uuid, name: impl Into<String>) -> Self {
        Self {
            uuid: Some(uuid),
            name: name.into(),
        }
    }

    /// Creates a target from a name that still needs profile resolution.
    #[must_use]
    pub fn unresolved(name: impl Into<String>) -> Self {
        Self {
            uuid: None,
            name: name.into(),
        }
    }

    /// Returns the target UUID when the profile has already been resolved.
    #[must_use]
    pub const fn uuid(&self) -> Option<Uuid> {
        self.uuid
    }

    /// Returns whether the target already has a resolved UUID.
    #[must_use]
    pub const fn is_resolved(&self) -> bool {
        self.uuid.is_some()
    }

    /// Returns the target display name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Debug for PermissionTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PermissionTarget")
            .field("uuid", &self.uuid)
            .field("name", &self.name)
            .finish()
    }
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

impl ItemSlotRangeArgumentValue {
    /// Creates an item slot range value.
    #[must_use]
    pub fn new(name: impl Into<String>, slots: Vec<i32>) -> Self {
        Self {
            name: name.into(),
            slots,
        }
    }

    /// Returns the vanilla slot range name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the vanilla slot IDs in this range.
    #[must_use]
    pub fn slots(&self) -> &[i32] {
        &self.slots
    }
}

impl ItemPredicateArgumentValue {
    /// Creates an item predicate value.
    #[must_use]
    pub fn new(target: ItemPredicateTarget, conditions: Vec<ItemPredicateCondition>) -> Self {
        Self { target, conditions }
    }

    /// Returns the item target selector.
    #[must_use]
    pub const fn target(&self) -> &ItemPredicateTarget {
        &self.target
    }

    /// Returns the AND-ed condition groups.
    #[must_use]
    pub fn conditions(&self) -> &[ItemPredicateCondition] {
        &self.conditions
    }
}

impl ItemPredicateCondition {
    /// Creates an item predicate condition group.
    #[must_use]
    pub fn new(alternatives: Vec<ItemPredicateTerm>) -> Self {
        Self { alternatives }
    }

    /// Returns the OR-ed alternatives in this condition group.
    #[must_use]
    pub fn alternatives(&self) -> &[ItemPredicateTerm] {
        &self.alternatives
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

impl fmt::Debug for ParsedArgument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(value) => f.debug_tuple("Bool").field(value).finish(),
            Self::Anchor(value) => f.debug_tuple("Anchor").field(value).finish(),
            Self::I32(value) => f.debug_tuple("I32").field(value).finish(),
            Self::I64(value) => f.debug_tuple("I64").field(value).finish(),
            Self::F32(value) => f.debug_tuple("F32").field(value).finish(),
            Self::F64(value) => f.debug_tuple("F64").field(value).finish(),
            Self::String(value) => f.debug_tuple("String").field(value).finish(),
            Self::Identifier(value) => f.debug_tuple("Identifier").field(value).finish(),
            Self::PermissionKey(value) => f.debug_tuple("PermissionKey").field(value).finish(),
            Self::PermissionRuleExpression(value) => f
                .debug_tuple("PermissionRuleExpression")
                .field(value)
                .finish(),
            Self::PermissionMetadataExpression(value) => f
                .debug_tuple("PermissionMetadataExpression")
                .field(value)
                .finish(),
            Self::GameMode(value) => f.debug_tuple("GameMode").field(value).finish(),
            Self::Players(value) => f
                .debug_struct("Players")
                .field("count", &value.len())
                .finish(),
            Self::PermissionTargets(value) => f
                .debug_struct("PermissionTargets")
                .field("count", &value.len())
                .finish(),
            Self::Entities(value) => f
                .debug_struct("Entities")
                .field("count", &value.len())
                .finish(),
            Self::EntityType(value) => f.debug_tuple("EntityType").field(&value.key).finish(),
            Self::Item(value) => f.debug_tuple("Item").field(&value.key).finish(),
            Self::ItemSlots(value) => f.debug_tuple("ItemSlots").field(value).finish(),
            Self::ItemPredicate(value) => f.debug_tuple("ItemPredicate").field(value).finish(),
            Self::Enchantment(value) => f.debug_tuple("Enchantment").field(&value.key).finish(),
            Self::Biome(value) => f.debug_tuple("Biome").field(value).finish(),
            Self::BlockPredicate(value) => f.debug_tuple("BlockPredicate").field(value).finish(),
            Self::NbtPath(value) => f.debug_tuple("NbtPath").field(value).finish(),
            Self::Structure(value) => f.debug_tuple("Structure").field(value).finish(),
            Self::World(value) => f.debug_tuple("World").field(&value.key).finish(),
            Self::Vec3(value) => f.debug_tuple("Vec3").field(value).finish(),
            Self::BlockPos(value) => f.debug_tuple("BlockPos").field(value).finish(),
            Self::Heightmap(value) => f.debug_tuple("Heightmap").field(value).finish(),
            Self::Rotation(value) => f.debug_tuple("Rotation").field(value).finish(),
            Self::Component(_) => f.debug_tuple("Component").finish(),
            Self::ScoreboardObjective(value) => {
                f.debug_tuple("ScoreboardObjective").field(value).finish()
            }
            Self::ScoreHolders(value) => f.debug_tuple("ScoreHolders").field(value).finish(),
            Self::IntRange(value) => f.debug_tuple("IntRange").field(value).finish(),
        }
    }
}

/// Typed command argument storage.
#[derive(Clone, Debug, Default)]
pub struct ParsedArguments {
    values: Vec<ParsedArgumentEntry>,
}

#[derive(Clone, Debug)]
struct ParsedArgumentEntry {
    name: String,
    value: ParsedArgument,
}

impl ParsedArguments {
    /// Stores an argument value.
    pub fn insert(&mut self, name: impl Into<String>, value: ParsedArgument) {
        let name = name.into();
        if let Some(existing) = self.values.iter_mut().find(|entry| entry.name == name) {
            existing.value = value;
            return;
        }

        self.values.push(ParsedArgumentEntry { name, value });
    }

    /// Returns a typed argument by name.
    ///
    /// # Errors
    ///
    /// Returns an error when the argument is missing or has the wrong type.
    pub fn get<T: FromParsedArgument>(&self, name: &str) -> Result<T, ParsedArgumentError> {
        let value = self
            .values
            .iter()
            .rev()
            .find_map(|entry| (entry.name == name).then_some(&entry.value))
            .ok_or_else(|| ParsedArgumentError::Missing(name.to_owned()))?;

        T::from_parsed_argument(value).ok_or_else(|| ParsedArgumentError::WrongType {
            name: name.to_owned(),
            expected: T::TYPE_NAME,
            actual: value.type_name(),
        })
    }
}

impl ParsedArgument {
    const fn type_name(&self) -> &'static str {
        match self {
            Self::Bool(_) => "bool",
            Self::Anchor(_) => "anchor",
            Self::I32(_) => "i32",
            Self::I64(_) => "i64",
            Self::F32(_) => "f32",
            Self::F64(_) => "f64",
            Self::String(_) => "string",
            Self::Identifier(_) => "identifier",
            Self::PermissionKey(_) => "permission_key",
            Self::PermissionRuleExpression(_) => "permission_rule_expression",
            Self::PermissionMetadataExpression(_) => "permission_metadata_expression",
            Self::GameMode(_) => "gamemode",
            Self::Players(_) => "players",
            Self::PermissionTargets(_) => "permission_targets",
            Self::Entities(_) => "entities",
            Self::EntityType(_) => "entity_type",
            Self::Item(_) => "item",
            Self::ItemSlots(_) => "item_slots",
            Self::ItemPredicate(_) => "item_predicate",
            Self::Enchantment(_) => "enchantment",
            Self::Biome(_) => "biome",
            Self::BlockPredicate(_) => "block_predicate",
            Self::NbtPath(_) => "nbt_path",
            Self::Structure(_) => "structure",
            Self::World(_) => "world",
            Self::Vec3(_) => "vec3",
            Self::BlockPos(_) => "block_pos",
            Self::Heightmap(_) => "heightmap",
            Self::Rotation(_) => "rotation",
            Self::Component(_) => "component",
            Self::ScoreboardObjective(_) => "scoreboard_objective",
            Self::ScoreHolders(_) => "score_holders",
            Self::IntRange(_) => "int_range",
        }
    }
}

/// Conversion from a parsed argument.
pub trait FromParsedArgument: Sized {
    /// Expected type name for diagnostics.
    const TYPE_NAME: &'static str;

    /// Converts from a parsed argument if the type matches.
    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self>;
}

/// A parsed argument value that can become one permission path segment.
pub trait CommandPermissionArgument: FromParsedArgument {
    /// Converts this argument value to one permission segment.
    ///
    /// # Errors
    ///
    /// Returns an error if the argument value cannot form a valid permission
    /// segment.
    fn permission_segment(&self) -> Result<PermissionSegment, PermissionKeyError>;

    /// Returns finite permission segments this argument can publish to the permission catalog.
    ///
    /// Dynamic arguments can return an empty slice when their value set is
    /// discovered from runtime state instead of a closed enum.
    fn catalog_permission_segments() -> &'static [&'static str] {
        &[]
    }
}

impl FromParsedArgument for bool {
    const TYPE_NAME: &'static str = "bool";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Bool(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for EntityAnchor {
    const TYPE_NAME: &'static str = "anchor";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Anchor(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for i32 {
    const TYPE_NAME: &'static str = "i32";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::I32(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for i64 {
    const TYPE_NAME: &'static str = "i64";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::I64(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for f32 {
    const TYPE_NAME: &'static str = "f32";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::F32(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for f64 {
    const TYPE_NAME: &'static str = "f64";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::F64(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for String {
    const TYPE_NAME: &'static str = "string";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::String(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for ScoreHolderArgumentValue {
    const TYPE_NAME: &'static str = "score_holders";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::ScoreHolders(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for ScoreboardObjectiveName {
    const TYPE_NAME: &'static str = "scoreboard_objective";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::ScoreboardObjective(value) = value else {
            return None;
        };
        Some(Self::new(value.to_owned()))
    }
}

impl FromParsedArgument for IntRangeArgumentValue {
    const TYPE_NAME: &'static str = "int_range";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::IntRange(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for Identifier {
    const TYPE_NAME: &'static str = "identifier";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Identifier(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for PermissionKey {
    const TYPE_NAME: &'static str = "permission_key";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::PermissionKey(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for PermissionRuleExpression {
    const TYPE_NAME: &'static str = "permission_rule_expression";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::PermissionRuleExpression(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for PermissionMetadataExpression {
    const TYPE_NAME: &'static str = "permission_metadata_expression";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::PermissionMetadataExpression(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for GameType {
    const TYPE_NAME: &'static str = "gamemode";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::GameMode(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl CommandPermissionArgument for GameType {
    fn permission_segment(&self) -> Result<PermissionSegment, PermissionKeyError> {
        PermissionSegment::parse(self.name())
    }

    fn catalog_permission_segments() -> &'static [&'static str] {
        &["survival", "creative", "adventure", "spectator"]
    }
}

impl FromParsedArgument for Vec<Arc<Player>> {
    const TYPE_NAME: &'static str = "players";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Players(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for Vec<PermissionTarget> {
    const TYPE_NAME: &'static str = "permission_targets";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::PermissionTargets(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for Vec<SharedEntity> {
    const TYPE_NAME: &'static str = "entities";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Entities(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for EntityTypeRef {
    const TYPE_NAME: &'static str = "entity_type";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::EntityType(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for ItemRef {
    const TYPE_NAME: &'static str = "item";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Item(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for ItemSlotRangeArgumentValue {
    const TYPE_NAME: &'static str = "item_slots";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::ItemSlots(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for ItemPredicateArgumentValue {
    const TYPE_NAME: &'static str = "item_predicate";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::ItemPredicate(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for EnchantmentRef {
    const TYPE_NAME: &'static str = "enchantment";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Enchantment(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for StructureArgumentValue {
    const TYPE_NAME: &'static str = "structure";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Structure(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for BiomeArgumentValue {
    const TYPE_NAME: &'static str = "biome";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Biome(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for BlockPredicateArgumentValue {
    const TYPE_NAME: &'static str = "block_predicate";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::BlockPredicate(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for NbtPath {
    const TYPE_NAME: &'static str = "nbt_path";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::NbtPath(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for Arc<World> {
    const TYPE_NAME: &'static str = "world";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::World(value) = value else {
            return None;
        };
        Some(Arc::clone(value))
    }
}

impl FromParsedArgument for DVec3 {
    const TYPE_NAME: &'static str = "vec3";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Vec3(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for BlockPos {
    const TYPE_NAME: &'static str = "block_pos";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::BlockPos(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for HeightmapType {
    const TYPE_NAME: &'static str = "heightmap";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Heightmap(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for (f32, f32) {
    const TYPE_NAME: &'static str = "rotation";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Rotation(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for TextComponent {
    const TYPE_NAME: &'static str = "component";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Component(value) = value else {
            return None;
        };
        Some(value.as_ref().clone())
    }
}

/// Error returned when reading a typed parsed argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParsedArgumentError {
    /// The requested argument was not parsed.
    Missing(String),
    /// The requested argument has a different type.
    WrongType {
        /// Argument name.
        name: String,
        /// Expected type name.
        expected: &'static str,
        /// Actual type name.
        actual: &'static str,
    },
}
