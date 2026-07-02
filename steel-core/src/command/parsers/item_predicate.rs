//! Item predicate command argument parser and evaluator.

use simdnbt::owned::{NbtCompound, NbtList, NbtTag};
use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use steel_registry::{
    REGISTRY, RegistryExt, TaggedRegistryExt,
    attribute::{AttributeModifierOperation, AttributeRef},
    data_components::{ComponentData, vanilla_components},
    enchantment::EnchantmentRef,
    equipment::EquipmentSlotGroup,
    item_stack::ItemStack,
    items::ItemRef,
};
use steel_utils::{Identifier, nbt::parse_snbt_argument};

const VANILLA_DATA_COMPONENT_PREDICATE_KEYS: &[&str] = &[
    "damage",
    "enchantments",
    "stored_enchantments",
    "potion_contents",
    "custom_data",
    "container",
    "bundle_contents",
    "firework_explosion",
    "fireworks",
    "writable_book_content",
    "written_book_content",
    "attribute_modifiers",
    "trim",
    "jukebox_playable",
    "villager/variant",
];

use crate::command::{
    graph::{
        CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
        CommandParseErrorKind, ParsedArgument, ParsedArguments,
    },
    parsers::parse_resource_identifier,
    reader::CommandReader,
    requirement::CommandInputContext,
    suggestions::matches_suggestion_substr,
};

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
    let mut suggestions = Vec::new();

    for key in item_predicate_term_keys() {
        let text = key.strip_prefix("minecraft:").unwrap_or(&key);
        if matches_suggestion_substr(stripped_prefix, text) {
            push_unique_suggestion(&mut suggestions, format!("{base}{key}"));
        }
    }

    suggestions
}

fn push_unique_suggestion(suggestions: &mut Vec<SuggestionEntry>, text: String) {
    if !suggestions.iter().any(|suggestion| suggestion.text == text) {
        suggestions.push(SuggestionEntry::new(text));
    }
}

fn item_predicate_term_keys() -> impl Iterator<Item = String> {
    data_component_keys()
        .chain(std::iter::once(count_key().to_string()))
        .chain(vanilla_data_component_predicate_keys())
}

fn data_component_keys() -> impl Iterator<Item = String> {
    (0..REGISTRY.data_components.len())
        .filter_map(|id| REGISTRY.data_components.by_id(id))
        .filter(|component| component.is_persistent())
        .map(|component| component.key.to_string())
}

fn vanilla_data_component_predicate_keys() -> impl Iterator<Item = String> {
    VANILLA_DATA_COMPONENT_PREDICATE_KEYS
        .iter()
        .map(|path| Identifier::vanilla_static(path).to_string())
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
        if is_count_key(key) || is_persistent_component_key(key) {
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

fn is_persistent_component_key(key: &Identifier) -> bool {
    REGISTRY
        .data_components
        .by_key(key)
        .is_some_and(|component| component.is_persistent())
}

fn is_vanilla_data_component_predicate_key(key: &Identifier) -> bool {
    key.namespace == Identifier::VANILLA_NAMESPACE
        && VANILLA_DATA_COMPONENT_PREDICATE_KEYS
            .iter()
            .any(|path| key.path == *path)
}

fn is_empty_compound(value: &NbtTag) -> bool {
    matches!(value, NbtTag::Compound(compound) if compound.is_empty())
}

#[derive(Debug)]
struct ItemPredicateParseError {
    cursor: usize,
    message: String,
}
