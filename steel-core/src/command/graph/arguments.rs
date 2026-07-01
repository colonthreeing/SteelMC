use std::{fmt, sync::Arc};

use glam::DVec3;
use simdnbt::owned::{NbtCompound, NbtList, NbtTag};
use steel_registry::{
    REGISTRY, RegistryExt, TaggedRegistryExt,
    attribute::{AttributeModifierOperation, AttributeRef},
    biome::BiomeRef,
    blocks::BlockRef,
    data_components::{ComponentData, vanilla_components},
    enchantment::EnchantmentRef,
    entity_type::EntityTypeRef,
    equipment::EquipmentSlotGroup,
    item_stack::ItemStack,
    items::ItemRef,
    loot_table::RuntimeLootCondition,
    structure::StructureRef,
};
use steel_utils::{BlockPos, BlockStateId, Identifier, nbt::NbtPath, types::GameType};
use text_components::TextComponent;
use uuid::Uuid;

use crate::chunk::heightmap::HeightmapType;
use crate::command::context::EntityAnchor;
use crate::command::parsers::{
    EntityTargetArgumentValue, PermissionTargetArgumentValue, PlayerTargetArgumentValue,
    WorldArgumentValue,
};
use crate::entity::SharedEntity;
use crate::permission::{
    PermissionKey, PermissionKeyError, PermissionMetadataExpression, PermissionRuleExpression,
    PermissionSegment,
};
use crate::player::Player;

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
    /// Runtime-resolved player target argument.
    PlayerTargets(PlayerTargetArgumentValue),
    /// Permission-management player target argument.
    PermissionTargets(PermissionTargetArgumentValue),
    /// Living entity target argument.
    Entities(Vec<SharedEntity>),
    /// Runtime-resolved entity target argument.
    EntityTargets(EntityTargetArgumentValue),
    /// Entity type argument.
    EntityType(EntityTypeRef),
    /// Item argument.
    Item(ItemRef),
    /// Item stack argument.
    ItemStack(ItemStack),
    /// Item slot range argument.
    ItemSlots(ItemSlotRangeArgumentValue),
    /// Item predicate argument.
    ItemPredicate(ItemPredicateArgumentValue),
    /// Loot predicate argument.
    LootPredicate(LootPredicateArgumentValue),
    /// Command function or function tag argument.
    CommandFunction(CommandFunctionArgumentValue),
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
    World(WorldArgumentValue),
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
    /// Double range argument.
    DoubleRange(DoubleRangeArgumentValue),
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

/// Loot predicate command argument value.
#[derive(Clone, Debug)]
pub enum LootPredicateArgumentValue {
    /// A named predicate in the predicate registry.
    Reference(Identifier),
    /// An inline predicate value, decoded by the runtime evaluator.
    Inline(RuntimeLootCondition),
}

/// Command function argument value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandFunctionArgumentValue {
    /// A single command function ID.
    Function(Identifier),
    /// A command function tag ID, parsed without the leading `#`.
    Tag(Identifier),
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

/// Error while evaluating an item predicate against a stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemPredicateMatchError {
    /// The `minecraft:count` pseudo-component/predicate value is malformed.
    MalformedCountPredicate,
    /// The data component predicate value is malformed for this predicate type.
    MalformedComponentPredicate(Identifier),
    /// Steel cannot evaluate an exact value for this component yet.
    UnsupportedComponentValue(Identifier),
    /// Steel cannot evaluate this data component predicate yet.
    UnsupportedComponentPredicate(Identifier),
}

/// Player target for permission-management commands.
#[derive(Clone)]
pub struct PermissionTarget {
    uuid: Option<Uuid>,
    name: String,
}

/// Score holder command argument value.
#[derive(Clone, Debug)]
pub enum ScoreHolderArgumentValue {
    /// A direct holder name, resolved against online players at execution time.
    Name(String),
    /// A UUID holder, resolved against live entities at execution time.
    Uuid {
        /// Parsed UUID.
        uuid: Uuid,
        /// Original token, used as the fallback holder name.
        raw: String,
    },
    /// Entity selector resolved to scoreboard holders at execution time.
    Selector(EntityTargetArgumentValue),
    /// Wildcard holder expansion.
    Wildcard,
}

impl ScoreHolderArgumentValue {
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

/// Inclusive double range command argument value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DoubleRangeArgumentValue {
    min: Option<f64>,
    max: Option<f64>,
}

impl DoubleRangeArgumentValue {
    /// Creates a double range.
    #[must_use]
    pub const fn new(min: Option<f64>, max: Option<f64>) -> Self {
        Self { min, max }
    }

    /// Creates an exact-value range.
    #[must_use]
    pub const fn exactly(value: f64) -> Self {
        Self {
            min: Some(value),
            max: Some(value),
        }
    }

    /// Returns whether `value` matches this range.
    #[must_use]
    pub fn matches(self, value: f64) -> bool {
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

    /// Returns whether this predicate matches `stack`.
    ///
    /// Steel currently supports the vanilla item/tag/type checks, component
    /// presence, exact-value checks for implemented data components, and the
    /// `minecraft:count`, `minecraft:damage`, `minecraft:enchantments`, and
    /// `minecraft:stored_enchantments`, and `minecraft:attribute_modifiers`
    /// predicate types.
    ///
    /// # Errors
    ///
    /// Returns an error when the predicate requests component semantics Steel
    /// cannot evaluate correctly yet.
    pub fn matches_stack(&self, stack: &ItemStack) -> Result<bool, ItemPredicateMatchError> {
        if !self.target_matches(stack) {
            return Ok(false);
        }

        for condition in &self.conditions {
            if !condition.matches_stack(stack)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn target_matches(&self, stack: &ItemStack) -> bool {
        match &self.target {
            ItemPredicateTarget::Any => true,
            ItemPredicateTarget::Item(item) => stack.is(item),
            ItemPredicateTarget::Tag { items, .. } => {
                items.iter().any(|item| stack.item().key == item.key)
            }
        }
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

    fn matches_stack(&self, stack: &ItemStack) -> Result<bool, ItemPredicateMatchError> {
        for alternative in &self.alternatives {
            if alternative.matches_stack(stack)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl ItemPredicateTerm {
    fn matches_stack(&self, stack: &ItemStack) -> Result<bool, ItemPredicateMatchError> {
        match self {
            Self::ComponentPresence { key } => Ok(component_presence_matches(key, stack)),
            Self::ComponentValue { key, value } => component_value_matches(key, value, stack),
            Self::PredicateValue { key, value } => predicate_value_matches(key, value, stack),
            Self::Not(term) => Ok(!term.matches_stack(stack)?),
        }
    }
}

fn component_presence_matches(key: &Identifier, stack: &ItemStack) -> bool {
    is_count_key(key) || stack.has_component(key)
}

fn component_value_matches(
    key: &Identifier,
    value: &NbtTag,
    stack: &ItemStack,
) -> Result<bool, ItemPredicateMatchError> {
    if is_count_key(key) {
        return count_range_matches(value, stack.count());
    }

    let Some(actual) = stack.get_effective_value_raw(key) else {
        return Ok(false);
    };
    if matches!(actual, ComponentData::Todo) {
        return Err(ItemPredicateMatchError::UnsupportedComponentValue(
            key.clone(),
        ));
    }

    let Some(entry) = REGISTRY.data_components.by_key(key) else {
        return Ok(false);
    };
    let actual = (entry.nbt_writer)(actual);
    Ok(actual == *value)
}

fn predicate_value_matches(
    key: &Identifier,
    value: &NbtTag,
    stack: &ItemStack,
) -> Result<bool, ItemPredicateMatchError> {
    if is_count_key(key) {
        return count_range_matches(value, stack.count());
    }

    if is_damage_predicate_key(key) {
        return damage_predicate_matches(value, stack);
    }

    if is_enchantments_predicate_key(key) {
        return enchantments_predicate_matches(value, stack);
    }

    if is_stored_enchantments_predicate_key(key) {
        return stored_enchantments_predicate_matches(value, stack);
    }

    if is_attribute_modifiers_predicate_key(key) {
        return attribute_modifiers_predicate_matches(value, stack);
    }

    if is_empty_compound(value) {
        return Ok(stack.has_component(key));
    }

    Err(ItemPredicateMatchError::UnsupportedComponentPredicate(
        key.clone(),
    ))
}

fn count_range_matches(value: &NbtTag, count: i32) -> Result<bool, ItemPredicateMatchError> {
    Ok(parse_count_range(value)?.matches(count))
}

fn damage_predicate_matches(
    value: &NbtTag,
    stack: &ItemStack,
) -> Result<bool, ItemPredicateMatchError> {
    let NbtTag::Compound(compound) = value else {
        return Err(malformed_component_predicate("damage"));
    };

    let Some(damage) = stack.get(vanilla_components::DAMAGE).copied() else {
        return Ok(false);
    };

    let damage_range = compound
        .get("damage")
        .map(parse_damage_predicate_range)
        .transpose()?
        .unwrap_or_else(CountRange::any);
    let durability_range = compound
        .get("durability")
        .map(parse_damage_predicate_range)
        .transpose()?
        .unwrap_or_else(CountRange::any);
    let max_damage = stack
        .get(vanilla_components::MAX_DAMAGE)
        .copied()
        .unwrap_or(0);

    Ok(damage_range.matches(damage) && durability_range.matches(max_damage - damage))
}

fn parse_damage_predicate_range(value: &NbtTag) -> Result<CountRange, ItemPredicateMatchError> {
    parse_count_range(value).map_err(|_| malformed_component_predicate("damage"))
}

fn enchantments_predicate_matches(
    value: &NbtTag,
    stack: &ItemStack,
) -> Result<bool, ItemPredicateMatchError> {
    let predicates = parse_enchantment_predicates(value, "enchantments")?;
    let Some(enchantments) = stack.get(vanilla_components::ENCHANTMENTS) else {
        return Ok(false);
    };

    Ok(predicates
        .iter()
        .all(|predicate| predicate.matches(enchantments)))
}

fn stored_enchantments_predicate_matches(
    value: &NbtTag,
    stack: &ItemStack,
) -> Result<bool, ItemPredicateMatchError> {
    let predicates = parse_enchantment_predicates(value, "stored_enchantments")?;
    let Some(enchantments) = stack.get(vanilla_components::STORED_ENCHANTMENTS) else {
        return Ok(false);
    };

    Ok(predicates
        .iter()
        .all(|predicate| predicate.matches(enchantments)))
}

fn parse_enchantment_predicates(
    value: &NbtTag,
    path: &'static str,
) -> Result<Vec<EnchantmentPredicate>, ItemPredicateMatchError> {
    match value {
        NbtTag::List(NbtList::Empty) => Ok(Vec::new()),
        NbtTag::List(NbtList::Compound(compounds)) => compounds
            .iter()
            .map(|compound| parse_enchantment_predicate(compound, path))
            .collect(),
        _ => Err(malformed_component_predicate(path)),
    }
}

fn parse_enchantment_predicate(
    compound: &NbtCompound,
    path: &'static str,
) -> Result<EnchantmentPredicate, ItemPredicateMatchError> {
    let enchantments = compound
        .get("enchantments")
        .map(|value| parse_enchantment_holder_set(value, path))
        .transpose()?;
    let level = compound
        .get("levels")
        .map(|value| parse_enchantment_predicate_range(value, path))
        .transpose()?
        .unwrap_or_else(CountRange::any);

    Ok(EnchantmentPredicate {
        enchantments,
        level,
    })
}

fn parse_enchantment_holder_set(
    value: &NbtTag,
    path: &'static str,
) -> Result<Vec<EnchantmentRef>, ItemPredicateMatchError> {
    match value {
        NbtTag::String(value) => parse_enchantment_holder(&value.to_string(), path),
        NbtTag::List(NbtList::Empty) => Ok(Vec::new()),
        NbtTag::List(NbtList::String(values)) => {
            let mut enchantments = Vec::new();
            for value in values {
                enchantments.extend(parse_enchantment_holder(&value.to_string(), path)?);
            }
            Ok(enchantments)
        }
        _ => Err(malformed_component_predicate(path)),
    }
}

fn parse_enchantment_holder(
    value: &str,
    path: &'static str,
) -> Result<Vec<EnchantmentRef>, ItemPredicateMatchError> {
    let Some(tag) = value.strip_prefix('#') else {
        let Some(key) = parse_identifier_with_default_namespace(value) else {
            return Err(malformed_component_predicate(path));
        };
        let Some(enchantment) = REGISTRY.enchantments.by_key(&key) else {
            return Err(malformed_component_predicate(path));
        };
        return Ok(vec![enchantment]);
    };

    let Some(key) = parse_identifier_with_default_namespace(tag) else {
        return Err(malformed_component_predicate(path));
    };
    REGISTRY
        .enchantments
        .get_tag(&key)
        .ok_or_else(|| malformed_component_predicate(path))
}

fn parse_enchantment_predicate_range(
    value: &NbtTag,
    path: &'static str,
) -> Result<CountRange, ItemPredicateMatchError> {
    parse_count_range(value).map_err(|_| malformed_component_predicate(path))
}

#[derive(Clone, Debug)]
struct EnchantmentPredicate {
    enchantments: Option<Vec<EnchantmentRef>>,
    level: CountRange,
}

impl EnchantmentPredicate {
    fn matches(&self, enchantments: &vanilla_components::ItemEnchantments) -> bool {
        if let Some(expected) = &self.enchantments {
            return expected.iter().any(|enchantment| {
                let level = enchantments.get_level(&enchantment.key);
                level != 0 && self.level.matches_u32(level)
            });
        }

        if !self.level.is_any() {
            return enchantments
                .iter()
                .any(|(_, level)| self.level.matches_u32(*level));
        }

        !enchantments.is_empty()
    }
}

fn attribute_modifiers_predicate_matches(
    value: &NbtTag,
    stack: &ItemStack,
) -> Result<bool, ItemPredicateMatchError> {
    let predicate = parse_attribute_modifiers_predicate(value)?;
    let Some(modifiers) = stack.get(vanilla_components::ATTRIBUTE_MODIFIERS) else {
        return Ok(false);
    };

    Ok(predicate.matches(modifiers))
}

fn parse_attribute_modifiers_predicate(
    value: &NbtTag,
) -> Result<AttributeModifiersPredicate, ItemPredicateMatchError> {
    let NbtTag::Compound(compound) = value else {
        return Err(malformed_component_predicate("attribute_modifiers"));
    };

    let modifiers = compound
        .get("modifiers")
        .map(parse_attribute_modifier_collection_predicate)
        .transpose()?;
    Ok(AttributeModifiersPredicate { modifiers })
}

fn parse_attribute_modifier_collection_predicate(
    value: &NbtTag,
) -> Result<AttributeModifierCollectionPredicate, ItemPredicateMatchError> {
    let NbtTag::Compound(compound) = value else {
        return Err(malformed_component_predicate("attribute_modifiers"));
    };

    let contains = compound
        .get("contains")
        .map(parse_attribute_modifier_predicate_list)
        .transpose()?
        .unwrap_or_default();
    let counts = compound
        .get("count")
        .map(parse_attribute_modifier_count_predicates)
        .transpose()?
        .unwrap_or_default();
    let size = compound
        .get("size")
        .map(parse_attribute_modifier_size_range)
        .transpose()?;

    Ok(AttributeModifierCollectionPredicate {
        contains,
        counts,
        size,
    })
}

fn parse_attribute_modifier_predicate_list(
    value: &NbtTag,
) -> Result<Vec<AttributeModifierEntryPredicate>, ItemPredicateMatchError> {
    match value {
        NbtTag::List(NbtList::Empty) => Ok(Vec::new()),
        NbtTag::List(NbtList::Compound(compounds)) => compounds
            .iter()
            .map(parse_attribute_modifier_entry_predicate)
            .collect(),
        _ => Err(malformed_component_predicate("attribute_modifiers")),
    }
}

fn parse_attribute_modifier_count_predicates(
    value: &NbtTag,
) -> Result<Vec<AttributeModifierCountPredicate>, ItemPredicateMatchError> {
    match value {
        NbtTag::List(NbtList::Empty) => Ok(Vec::new()),
        NbtTag::List(NbtList::Compound(compounds)) => compounds
            .iter()
            .map(parse_attribute_modifier_count_predicate)
            .collect(),
        _ => Err(malformed_component_predicate("attribute_modifiers")),
    }
}

fn parse_attribute_modifier_count_predicate(
    compound: &NbtCompound,
) -> Result<AttributeModifierCountPredicate, ItemPredicateMatchError> {
    let Some(test) = compound.get("test") else {
        return Err(malformed_component_predicate("attribute_modifiers"));
    };
    let NbtTag::Compound(test) = test else {
        return Err(malformed_component_predicate("attribute_modifiers"));
    };
    let Some(count) = compound.get("count") else {
        return Err(malformed_component_predicate("attribute_modifiers"));
    };

    Ok(AttributeModifierCountPredicate {
        test: parse_attribute_modifier_entry_predicate(test)?,
        count: parse_attribute_modifier_size_range(count)?,
    })
}

fn parse_attribute_modifier_entry_predicate(
    compound: &NbtCompound,
) -> Result<AttributeModifierEntryPredicate, ItemPredicateMatchError> {
    let attribute = compound
        .get("attribute")
        .map(parse_attribute_holder_set)
        .transpose()?;
    let id = compound.get("id").map(parse_identifier_tag).transpose()?;
    let amount = compound
        .get("amount")
        .map(parse_attribute_modifier_amount_range)
        .transpose()?
        .unwrap_or_else(FloatRange::any);
    let operation = compound
        .get("operation")
        .map(parse_attribute_modifier_operation)
        .transpose()?;
    let slot = compound
        .get("slot")
        .map(parse_equipment_slot_group)
        .transpose()?;

    Ok(AttributeModifierEntryPredicate {
        attribute,
        id,
        amount,
        operation,
        slot,
    })
}

fn parse_attribute_holder_set(
    value: &NbtTag,
) -> Result<Vec<AttributeRef>, ItemPredicateMatchError> {
    match value {
        NbtTag::String(value) => parse_attribute_holder(&value.to_string()),
        NbtTag::List(NbtList::Empty) => Ok(Vec::new()),
        NbtTag::List(NbtList::String(values)) => {
            let mut attributes = Vec::new();
            for value in values {
                attributes.extend(parse_attribute_holder(&value.to_string())?);
            }
            Ok(attributes)
        }
        _ => Err(malformed_component_predicate("attribute_modifiers")),
    }
}

fn parse_attribute_holder(value: &str) -> Result<Vec<AttributeRef>, ItemPredicateMatchError> {
    if value.starts_with('#') {
        return Err(malformed_component_predicate("attribute_modifiers"));
    }

    let Some(key) = parse_identifier_with_default_namespace(value) else {
        return Err(malformed_component_predicate("attribute_modifiers"));
    };
    let Some(attribute) = REGISTRY.attributes.by_key(&key) else {
        return Err(malformed_component_predicate("attribute_modifiers"));
    };
    Ok(vec![attribute])
}

fn parse_identifier_tag(value: &NbtTag) -> Result<Identifier, ItemPredicateMatchError> {
    let NbtTag::String(value) = value else {
        return Err(malformed_component_predicate("attribute_modifiers"));
    };
    parse_identifier_with_default_namespace(&value.to_string())
        .ok_or_else(|| malformed_component_predicate("attribute_modifiers"))
}

fn parse_attribute_modifier_operation(
    value: &NbtTag,
) -> Result<AttributeModifierOperation, ItemPredicateMatchError> {
    let NbtTag::String(value) = value else {
        return Err(malformed_component_predicate("attribute_modifiers"));
    };
    AttributeModifierOperation::by_name(&value.to_string())
        .ok_or_else(|| malformed_component_predicate("attribute_modifiers"))
}

fn parse_equipment_slot_group(
    value: &NbtTag,
) -> Result<EquipmentSlotGroup, ItemPredicateMatchError> {
    let NbtTag::String(value) = value else {
        return Err(malformed_component_predicate("attribute_modifiers"));
    };
    EquipmentSlotGroup::by_name(&value.to_string())
        .ok_or_else(|| malformed_component_predicate("attribute_modifiers"))
}

fn parse_attribute_modifier_size_range(
    value: &NbtTag,
) -> Result<CountRange, ItemPredicateMatchError> {
    parse_count_range(value).map_err(|_| malformed_component_predicate("attribute_modifiers"))
}

fn parse_attribute_modifier_amount_range(
    value: &NbtTag,
) -> Result<FloatRange, ItemPredicateMatchError> {
    if let Some(value) = nbt_number(value) {
        return Ok(FloatRange::exactly(value));
    }

    let NbtTag::Compound(compound) = value else {
        return Err(malformed_component_predicate("attribute_modifiers"));
    };

    let min = compound.get("min").map(nbt_number_required).transpose()?;
    let max = compound.get("max").map(nbt_number_required).transpose()?;
    if let (Some(min), Some(max)) = (min, max)
        && min > max
    {
        return Err(malformed_component_predicate("attribute_modifiers"));
    }

    Ok(FloatRange::new(min, max))
}

#[derive(Clone, Debug)]
struct AttributeModifiersPredicate {
    modifiers: Option<AttributeModifierCollectionPredicate>,
}

impl AttributeModifiersPredicate {
    fn matches(&self, modifiers: &vanilla_components::ItemAttributeModifiers) -> bool {
        self.modifiers
            .as_ref()
            .is_none_or(|predicate| predicate.matches(modifiers))
    }
}

#[derive(Clone, Debug)]
struct AttributeModifierCollectionPredicate {
    contains: Vec<AttributeModifierEntryPredicate>,
    counts: Vec<AttributeModifierCountPredicate>,
    size: Option<CountRange>,
}

impl AttributeModifierCollectionPredicate {
    fn matches(&self, modifiers: &vanilla_components::ItemAttributeModifiers) -> bool {
        if !self.contains_matches(&modifiers.modifiers) {
            return false;
        }
        if !self.counts_matches(&modifiers.modifiers) {
            return false;
        }
        self.size
            .is_none_or(|range| range.matches_usize(modifiers.modifiers.len()))
    }

    fn contains_matches(
        &self,
        modifiers: &[vanilla_components::ItemAttributeModifierEntry],
    ) -> bool {
        let mut remaining = self.contains.clone();
        for modifier in modifiers {
            remaining.retain(|predicate| !predicate.matches(modifier));
            if remaining.is_empty() {
                return true;
            }
        }
        remaining.is_empty()
    }

    fn counts_matches(&self, modifiers: &[vanilla_components::ItemAttributeModifierEntry]) -> bool {
        self.counts.iter().all(|predicate| {
            let count = modifiers
                .iter()
                .filter(|modifier| predicate.test.matches(modifier))
                .count();
            predicate.count.matches_usize(count)
        })
    }
}

#[derive(Clone, Debug)]
struct AttributeModifierCountPredicate {
    test: AttributeModifierEntryPredicate,
    count: CountRange,
}

#[derive(Clone, Debug)]
struct AttributeModifierEntryPredicate {
    attribute: Option<Vec<AttributeRef>>,
    id: Option<Identifier>,
    amount: FloatRange,
    operation: Option<AttributeModifierOperation>,
    slot: Option<EquipmentSlotGroup>,
}

impl AttributeModifierEntryPredicate {
    fn matches(&self, modifier: &vanilla_components::ItemAttributeModifierEntry) -> bool {
        if self.attribute.as_ref().is_some_and(|attributes| {
            !attributes
                .iter()
                .any(|entry| entry.key == modifier.attribute.key)
        }) {
            return false;
        }
        if self.id.as_ref().is_some_and(|id| id != &modifier.id) {
            return false;
        }
        if !self.amount.matches(modifier.amount) {
            return false;
        }
        if self
            .operation
            .is_some_and(|operation| operation != modifier.operation)
        {
            return false;
        }
        !self.slot.is_some_and(|slot| slot != modifier.slot)
    }
}

#[derive(Clone, Copy, Debug)]
struct FloatRange {
    min: Option<f64>,
    max: Option<f64>,
}

impl FloatRange {
    const fn any() -> Self {
        Self {
            min: None,
            max: None,
        }
    }

    const fn exactly(value: f64) -> Self {
        Self {
            min: Some(value),
            max: Some(value),
        }
    }

    const fn new(min: Option<f64>, max: Option<f64>) -> Self {
        Self { min, max }
    }

    fn matches(self, value: f64) -> bool {
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

fn malformed_component_predicate(path: &'static str) -> ItemPredicateMatchError {
    ItemPredicateMatchError::MalformedComponentPredicate(Identifier::vanilla_static(path))
}

fn is_count_key(key: &Identifier) -> bool {
    key.namespace == Identifier::VANILLA_NAMESPACE && key.path == "count"
}

fn is_damage_predicate_key(key: &Identifier) -> bool {
    key.namespace == Identifier::VANILLA_NAMESPACE && key.path == "damage"
}

fn is_enchantments_predicate_key(key: &Identifier) -> bool {
    key.namespace == Identifier::VANILLA_NAMESPACE && key.path == "enchantments"
}

fn is_stored_enchantments_predicate_key(key: &Identifier) -> bool {
    key.namespace == Identifier::VANILLA_NAMESPACE && key.path == "stored_enchantments"
}

fn is_attribute_modifiers_predicate_key(key: &Identifier) -> bool {
    key.namespace == Identifier::VANILLA_NAMESPACE && key.path == "attribute_modifiers"
}

fn is_empty_compound(value: &NbtTag) -> bool {
    matches!(value, NbtTag::Compound(compound) if compound.is_empty())
}

fn parse_identifier_with_default_namespace(value: &str) -> Option<Identifier> {
    let (namespace, path) = match value.split_once(':') {
        Some(("", path)) => (Identifier::VANILLA_NAMESPACE, path),
        Some((namespace, path)) => (namespace, path),
        None => (Identifier::VANILLA_NAMESPACE, value),
    };

    Identifier::validate(namespace, path)
        .then(|| Identifier::new(namespace.to_owned(), path.to_owned()))
}

#[derive(Clone, Copy, Debug)]
struct CountRange {
    min: Option<i32>,
    max: Option<i32>,
}

impl CountRange {
    const fn any() -> Self {
        Self {
            min: None,
            max: None,
        }
    }

    const fn exactly(value: i32) -> Self {
        Self {
            min: Some(value),
            max: Some(value),
        }
    }

    const fn new(min: Option<i32>, max: Option<i32>) -> Self {
        Self { min, max }
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

    fn matches_u32(self, value: u32) -> bool {
        let value = i64::from(value);
        if let Some(min) = self.min
            && value < i64::from(min)
        {
            return false;
        }
        if let Some(max) = self.max
            && value > i64::from(max)
        {
            return false;
        }
        true
    }

    fn matches_usize(self, value: usize) -> bool {
        if let Some(min) = self.min
            && min >= 0
            && value < min as usize
        {
            return false;
        }
        if let Some(max) = self.max {
            let Ok(max) = usize::try_from(max) else {
                return false;
            };
            if value > max {
                return false;
            }
        }
        true
    }

    const fn is_any(self) -> bool {
        self.min.is_none() && self.max.is_none()
    }
}

fn parse_count_range(value: &NbtTag) -> Result<CountRange, ItemPredicateMatchError> {
    if let Some(value) = nbt_integer(value) {
        return Ok(CountRange::exactly(value));
    }

    let NbtTag::Compound(compound) = value else {
        return Err(ItemPredicateMatchError::MalformedCountPredicate);
    };

    let min = compound.get("min").map(nbt_integer_required).transpose()?;
    let max = compound.get("max").map(nbt_integer_required).transpose()?;
    if let (Some(min), Some(max)) = (min, max)
        && min > max
    {
        return Err(ItemPredicateMatchError::MalformedCountPredicate);
    }

    Ok(CountRange::new(min, max))
}

fn nbt_integer_required(value: &NbtTag) -> Result<i32, ItemPredicateMatchError> {
    nbt_integer(value).ok_or(ItemPredicateMatchError::MalformedCountPredicate)
}

fn nbt_integer(value: &NbtTag) -> Option<i32> {
    match value {
        NbtTag::Byte(value) => Some(i32::from(*value)),
        NbtTag::Short(value) => Some(i32::from(*value)),
        NbtTag::Int(value) => Some(*value),
        NbtTag::Long(value) => i32::try_from(*value).ok(),
        _ => None,
    }
}

fn nbt_number_required(value: &NbtTag) -> Result<f64, ItemPredicateMatchError> {
    nbt_number(value).ok_or_else(|| malformed_component_predicate("attribute_modifiers"))
}

fn nbt_number(value: &NbtTag) -> Option<f64> {
    match value {
        NbtTag::Byte(value) => Some(f64::from(*value)),
        NbtTag::Short(value) => Some(f64::from(*value)),
        NbtTag::Int(value) => Some(f64::from(*value)),
        NbtTag::Long(value) => Some(*value as f64),
        NbtTag::Float(value) => Some(f64::from(*value)),
        NbtTag::Double(value) => Some(*value),
        _ => None,
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
            Self::PlayerTargets(value) => {
                f.debug_tuple("PlayerTargets").field(&value.raw()).finish()
            }
            Self::PermissionTargets(value) => f
                .debug_tuple("PermissionTargets")
                .field(&value.raw())
                .finish(),
            Self::Entities(value) => f
                .debug_struct("Entities")
                .field("count", &value.len())
                .finish(),
            Self::EntityTargets(value) => {
                f.debug_tuple("EntityTargets").field(&value.raw()).finish()
            }
            Self::EntityType(value) => f.debug_tuple("EntityType").field(&value.key).finish(),
            Self::Item(value) => f.debug_tuple("Item").field(&value.key).finish(),
            Self::ItemStack(value) => f.debug_tuple("ItemStack").field(value).finish(),
            Self::ItemSlots(value) => f.debug_tuple("ItemSlots").field(value).finish(),
            Self::ItemPredicate(value) => f.debug_tuple("ItemPredicate").field(value).finish(),
            Self::LootPredicate(value) => f.debug_tuple("LootPredicate").field(value).finish(),
            Self::CommandFunction(value) => f.debug_tuple("CommandFunction").field(value).finish(),
            Self::Enchantment(value) => f.debug_tuple("Enchantment").field(&value.key).finish(),
            Self::Biome(value) => f.debug_tuple("Biome").field(value).finish(),
            Self::BlockPredicate(value) => f.debug_tuple("BlockPredicate").field(value).finish(),
            Self::NbtPath(value) => f.debug_tuple("NbtPath").field(value).finish(),
            Self::Structure(value) => f.debug_tuple("Structure").field(value).finish(),
            Self::World(value) => f.debug_tuple("World").field(value).finish(),
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
            Self::DoubleRange(value) => f.debug_tuple("DoubleRange").field(value).finish(),
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
            Self::PlayerTargets(_) => "player_targets",
            Self::PermissionTargets(_) => "permission_targets",
            Self::Entities(_) => "entities",
            Self::EntityTargets(_) => "entity_targets",
            Self::EntityType(_) => "entity_type",
            Self::Item(_) => "item",
            Self::ItemStack(_) => "item_stack",
            Self::ItemSlots(_) => "item_slots",
            Self::ItemPredicate(_) => "item_predicate",
            Self::LootPredicate(_) => "loot_predicate",
            Self::CommandFunction(_) => "command_function",
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
            Self::DoubleRange(_) => "double_range",
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

impl FromParsedArgument for DoubleRangeArgumentValue {
    const TYPE_NAME: &'static str = "double_range";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::DoubleRange(value) = value else {
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

impl FromParsedArgument for PlayerTargetArgumentValue {
    const TYPE_NAME: &'static str = "player_targets";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::PlayerTargets(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for PermissionTargetArgumentValue {
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

impl FromParsedArgument for EntityTargetArgumentValue {
    const TYPE_NAME: &'static str = "entity_targets";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::EntityTargets(value) = value else {
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

impl FromParsedArgument for ItemStack {
    const TYPE_NAME: &'static str = "item_stack";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::ItemStack(value) = value else {
            return None;
        };
        Some(value.clone())
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

impl FromParsedArgument for LootPredicateArgumentValue {
    const TYPE_NAME: &'static str = "loot_predicate";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::LootPredicate(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for CommandFunctionArgumentValue {
    const TYPE_NAME: &'static str = "command_function";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::CommandFunction(value) = value else {
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

impl FromParsedArgument for WorldArgumentValue {
    const TYPE_NAME: &'static str = "world";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::World(value) = value else {
            return None;
        };
        Some(value.clone())
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
