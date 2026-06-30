use std::{error::Error, fmt};

pub use crate::equipment::EquipmentSlotGroup;
use crate::{
    REGISTRY, RegistryExt, TaggedRegistryExt, blocks::block_state_ext::BlockStateExt,
    item_stack::ItemStack,
};
use rand::RngExt;
use rustc_hash::FxHashMap;
use simdnbt::owned::{NbtCompound, NbtTag};
use steel_utils::{BlockStateId, Identifier};

/// Entity target for loot context lookups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LootContextEntity {
    /// The entity being looted (killed mob, block entity owner).
    This,
    /// The entity that killed the target.
    Killer,
    /// The direct attacker (e.g., arrow, not the player who shot it).
    DirectKiller,
    /// The player who dealt the final damage.
    KillerPlayer,
    /// The entity interacting with a block/entity.
    Interacting,
}

/// Dye/banner color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DyeColor {
    White,
    Orange,
    Magenta,
    LightBlue,
    Yellow,
    Lime,
    Pink,
    Gray,
    LightGray,
    Cyan,
    Purple,
    Blue,
    Brown,
    Green,
    Red,
    Black,
}

/// The type of loot table, determining when/how it's used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LootType {
    Block,
    Entity,
    Chest,
    Fishing,
    Gift,
    Archaeology,
    Vault,
    Shearing,
    Equipment,
    Selector,
    EntityInteract,
    BlockInteract,
    Barter,
}

/// A number provider that can be constant or random.
#[derive(Debug, Clone)]
pub enum NumberProvider {
    Constant(f32),
    Uniform {
        min: f32,
        max: f32,
    },
    Binomial {
        n: i32,
        p: f32,
    },
    /// Get value from entity scoreboard score.
    Score {
        target: ScoreboardTarget,
        score: &'static str,
        scale: f32,
    },
    /// Get value from command storage.
    Storage {
        storage: Identifier,
        path: &'static str,
    },
    /// Get enchantment level from context tool.
    EnchantmentLevel {
        enchantment: Identifier,
    },
}

/// Target for scoreboard number provider.
#[derive(Debug, Clone, Copy)]
pub enum ScoreboardTarget {
    /// The entity being looted.
    This,
    /// The entity that killed the target.
    Killer,
    /// The direct killer (e.g., arrow vs player).
    DirectKiller,
    /// The player who dealt the last damage.
    KillerPlayer,
    /// A fixed entity name.
    Fixed(&'static str),
}

impl NumberProvider {
    /// Get a value from this provider, reporting context-dependent providers
    /// that Steel cannot evaluate without silently substituting a fallback.
    ///
    /// # Errors
    ///
    /// Returns an error for provider types whose backing runtime foundation is
    /// not available to the checked evaluator.
    pub fn try_get<R: rand::Rng>(
        &self,
        rng: &mut R,
        ctx: Option<&LootContextRef<'_>>,
    ) -> Result<f32, LootConditionEvaluationError> {
        match self {
            Self::Constant(v) => Ok(*v),
            Self::Uniform { min, max } => Ok(rng.random_range(*min..=*max)),
            Self::Binomial { n, p } => {
                let mut count = 0;
                for _ in 0..*n {
                    if rng.random::<f32>() < *p {
                        count += 1;
                    }
                }
                Ok(count as f32)
            }
            Self::Score { .. } => Err(LootConditionEvaluationError::UnsupportedNumberProvider(
                "score",
            )),
            Self::Storage { .. } => Err(LootConditionEvaluationError::UnsupportedNumberProvider(
                "storage",
            )),
            Self::EnchantmentLevel { enchantment } => Ok(ctx
                .and_then(|c| c.tool)
                .map(|t| t.get_enchantment_level(enchantment) as f32)
                .unwrap_or(0.0)),
        }
    }

    /// Get a value from this provider using the given RNG.
    pub fn get<R: rand::Rng>(&self, rng: &mut R, ctx: Option<&LootContextRef<'_>>) -> f32 {
        match self {
            Self::Constant(v) => *v,
            Self::Uniform { min, max } => rng.random_range(*min..=*max),
            Self::Binomial { n, p } => {
                let mut count = 0;
                for _ in 0..*n {
                    if rng.random::<f32>() < *p {
                        count += 1;
                    }
                }
                count as f32
            }
            Self::Score { .. } => {
                // TODO: Implement when scoreboard system is available
                let _ = ctx;
                0.0
            }
            Self::Storage { .. } => {
                // TODO: Implement when command storage system is available
                let _ = ctx;
                0.0
            }
            Self::EnchantmentLevel { enchantment } => ctx
                .and_then(|c| c.tool)
                .map(|t| t.get_enchantment_level(enchantment) as f32)
                .unwrap_or(0.0),
        }
    }

    /// Get a value without context (for backwards compatibility).
    pub fn get_simple(&self, rng: &mut impl rand::Rng) -> f32 {
        match self {
            Self::Constant(v) => *v,
            Self::Uniform { min, max } => rng.random_range(*min..=*max),
            Self::Binomial { n, p } => {
                let mut count = 0;
                for _ in 0..*n {
                    if rng.random::<f32>() < *p {
                        count += 1;
                    }
                }
                count as f32
            }
            // Context-dependent providers return 0 when no context available
            Self::Score { .. } | Self::Storage { .. } | Self::EnchantmentLevel { .. } => 0.0,
        }
    }

    /// Get the value as an integer.
    pub fn get_int(&self, rng: &mut impl rand::Rng) -> i32 {
        self.get_simple(rng).floor() as i32
    }

    /// Get the value as an integer with context.
    pub fn get_int_with_ctx<R: rand::Rng>(
        &self,
        rng: &mut R,
        ctx: Option<&LootContextRef<'_>>,
    ) -> i32 {
        self.get(rng, ctx).floor() as i32
    }
}

/// A range for number comparisons (used in ValueCheck, TimeCheck, EntityScores).
#[derive(Debug, Clone)]
pub struct NumberProviderRange {
    pub min: Option<NumberProvider>,
    pub max: Option<NumberProvider>,
}

impl NumberProviderRange {
    /// Check if a value is within this range without silently substituting
    /// unsupported number providers.
    ///
    /// # Errors
    ///
    /// Returns an error when a range bound depends on a provider Steel cannot
    /// evaluate in checked mode.
    pub fn try_test<R: rand::Rng>(
        &self,
        value: f32,
        rng: &mut R,
        ctx: Option<&LootContextRef<'_>>,
    ) -> Result<bool, LootConditionEvaluationError> {
        if let Some(min) = &self.min
            && value < min.try_get(rng, ctx)?
        {
            return Ok(false);
        }
        if let Some(max) = &self.max
            && value > max.try_get(rng, ctx)?
        {
            return Ok(false);
        }
        Ok(true)
    }

    /// Check if a value is within this range.
    pub fn test(&self, value: f32, rng: &mut impl rand::Rng) -> bool {
        if let Some(min) = &self.min
            && value < min.get_simple(rng)
        {
            return false;
        }
        if let Some(max) = &self.max
            && value > max.get_simple(rng)
        {
            return false;
        }
        true
    }

    /// Create an exact match range.
    pub const fn exact(value: f32) -> Self {
        Self {
            min: Some(NumberProvider::Constant(value)),
            max: Some(NumberProvider::Constant(value)),
        }
    }

    /// Create an at-least range.
    pub const fn at_least(min: f32) -> Self {
        Self {
            min: Some(NumberProvider::Constant(min)),
            max: None,
        }
    }

    /// Create an at-most range.
    pub const fn at_most(max: f32) -> Self {
        Self {
            min: None,
            max: Some(NumberProvider::Constant(max)),
        }
    }

    /// Create a between range.
    pub const fn between(min: f32, max: f32) -> Self {
        Self {
            min: Some(NumberProvider::Constant(min)),
            max: Some(NumberProvider::Constant(max)),
        }
    }
}

/// Reference to loot context for number provider evaluation.
/// This is a simplified view to avoid circular references.
pub struct LootContextRef<'a> {
    pub tool: Option<&'a ItemStack>,
    // Add more fields as needed for Score/Storage providers
}

/// Context for loot table evaluation, containing all relevant game state.
///
/// This mirrors vanilla's `LootContext` / `LootParams` system.
pub struct LootContext<'a, R: rand::Rng> {
    /// Random number generator.
    pub rng: &'a mut R,
    /// Luck value (e.g., from Luck of the Sea enchantment).
    pub luck: f32,
    /// The block state being broken (for block loot tables).
    pub block_state: Option<BlockStateId>,
    /// The tool used to break the block.
    pub tool: Option<&'a ItemStack>,
    /// Explosion radius if caused by an explosion.
    pub explosion_radius: Option<f32>,
    /// Whether the entity was killed by a player.
    pub killed_by_player: bool,

    /// World position where the loot is generated (block position or entity death location).
    pub origin: Option<(f64, f64, f64)>,
    /// Current game time in ticks (for TimeCheck condition).
    pub game_time: Option<i64>,
    /// Current weather state.
    pub weather: Option<WeatherState>,
    /// The entity being looted (the killed mob, block entity owner, etc.).
    /// This is a type-erased pointer; actual entity data depends on game implementation.
    pub this_entity: Option<EntityRef<'a>>,
    /// The entity that killed this_entity (for mob loot tables).
    pub killer_entity: Option<EntityRef<'a>>,
    /// The direct attacker entity (e.g., arrow, not the player who shot it).
    pub direct_killer_entity: Option<EntityRef<'a>>,
    /// The player who dealt the final damage (may be different from killer).
    pub last_damage_player: Option<EntityRef<'a>>,
    /// Damage source information for entity deaths.
    pub damage_source: Option<DamageSourceInfo<'a>>,
    /// Block entity reference for container/block loot.
    pub block_entity: Option<BlockEntityRef<'a>>,
    /// The entity interacting with a block/entity (e.g., player opening a chest).
    pub interacting_entity: Option<EntityRef<'a>>,
}

/// Weather state for WeatherCheck condition.
#[derive(Debug, Clone, Copy, Default)]
pub struct WeatherState {
    pub raining: bool,
    pub thundering: bool,
}

/// A reference to an entity for loot context.
/// This is intentionally opaque - the actual entity type depends on game implementation.
#[derive(Debug, Clone, Copy)]
pub struct EntityRef<'a> {
    /// Type identifier for the entity.
    pub entity_type: Option<&'a Identifier>,
    /// Entity flags for predicate checking.
    pub flags: EntityRefFlags,
    /// Equipment slots for equipment predicates.
    pub equipment: Option<&'a EntityEquipmentRef<'a>>,
    /// Entity name (for copy_name function).
    pub custom_name: Option<&'a str>,
}

/// Entity flags for predicate checking.
#[derive(Debug, Clone, Copy, Default)]
pub struct EntityRefFlags {
    pub is_on_fire: bool,
    pub is_sneaking: bool,
    pub is_sprinting: bool,
    pub is_swimming: bool,
    pub is_baby: bool,
}

/// Equipment references for an entity.
#[derive(Debug, Clone, Copy)]
pub struct EntityEquipmentRef<'a> {
    pub mainhand: Option<&'a ItemStack>,
    pub offhand: Option<&'a ItemStack>,
    pub head: Option<&'a ItemStack>,
    pub chest: Option<&'a ItemStack>,
    pub legs: Option<&'a ItemStack>,
    pub feet: Option<&'a ItemStack>,
}

/// Damage source information for loot context.
#[derive(Debug, Clone, Copy)]
pub struct DamageSourceInfo<'a> {
    /// The damage type identifier.
    pub damage_type: Option<&'a Identifier>,
    /// Tags associated with this damage source.
    pub tags: &'a [Identifier],
    /// Whether this is direct damage (not from a projectile).
    pub is_direct: bool,
}

/// A reference to a block entity for loot context.
#[derive(Debug, Clone, Copy)]
pub struct BlockEntityRef<'a> {
    /// The block entity type identifier.
    pub block_entity_type: Option<&'a Identifier>,
    /// Custom name of the block entity (for copy_name).
    pub custom_name: Option<&'a str>,
    /// Inventory contents (for dynamic/slots entries).
    pub inventory: Option<&'a [ItemStack]>,
}

impl<'a, R: rand::Rng> LootContext<'a, R> {
    /// Create a new loot context with just an RNG.
    pub fn new(rng: &'a mut R) -> Self {
        Self {
            rng,
            luck: 0.0,
            block_state: None,
            tool: None,
            explosion_radius: None,
            killed_by_player: false,
            origin: None,
            game_time: None,
            weather: None,
            this_entity: None,
            killer_entity: None,
            direct_killer_entity: None,
            last_damage_player: None,
            damage_source: None,
            block_entity: None,
            interacting_entity: None,
        }
    }

    /// Set the luck value.
    pub fn with_luck(mut self, luck: f32) -> Self {
        self.luck = luck;
        self
    }

    /// Set the block state.
    pub fn with_block_state(mut self, state: BlockStateId) -> Self {
        self.block_state = Some(state);
        self
    }

    /// Set the tool used.
    pub fn with_tool(mut self, tool: &'a ItemStack) -> Self {
        self.tool = Some(tool);
        self
    }

    /// Set the explosion radius.
    pub fn with_explosion(mut self, radius: f32) -> Self {
        self.explosion_radius = Some(radius);
        self
    }

    /// Set whether killed by player.
    pub fn with_killed_by_player(mut self, killed: bool) -> Self {
        self.killed_by_player = killed;
        self
    }

    /// Set the world origin position.
    pub fn with_origin(mut self, x: f64, y: f64, z: f64) -> Self {
        self.origin = Some((x, y, z));
        self
    }

    /// Set the game time.
    pub fn with_game_time(mut self, time: i64) -> Self {
        self.game_time = Some(time);
        self
    }

    /// Set the weather state.
    pub fn with_weather(mut self, weather: WeatherState) -> Self {
        self.weather = Some(weather);
        self
    }

    /// Set the entity being looted.
    pub fn with_this_entity(mut self, entity: EntityRef<'a>) -> Self {
        self.this_entity = Some(entity);
        self
    }

    /// Set the killer entity.
    pub fn with_killer_entity(mut self, entity: EntityRef<'a>) -> Self {
        self.killer_entity = Some(entity);
        self
    }

    /// Set the direct killer entity (e.g., projectile).
    pub fn with_direct_killer_entity(mut self, entity: EntityRef<'a>) -> Self {
        self.direct_killer_entity = Some(entity);
        self
    }

    /// Set the player who dealt the final damage.
    pub fn with_last_damage_player(mut self, entity: EntityRef<'a>) -> Self {
        self.last_damage_player = Some(entity);
        self
    }

    /// Set the damage source information.
    pub fn with_damage_source(mut self, damage_source: DamageSourceInfo<'a>) -> Self {
        self.damage_source = Some(damage_source);
        self
    }

    /// Set the block entity reference.
    pub fn with_block_entity(mut self, block_entity: BlockEntityRef<'a>) -> Self {
        self.block_entity = Some(block_entity);
        self
    }

    /// Set the interacting entity (e.g., player opening a chest).
    pub fn with_interacting_entity(mut self, entity: EntityRef<'a>) -> Self {
        self.interacting_entity = Some(entity);
        self
    }

    /// Get the level of an enchantment on the tool by identifier.
    pub fn get_enchantment_level_by_id(&self, enchantment: &Identifier) -> i32 {
        self.tool
            .map(|t| t.get_enchantment_level(enchantment))
            .unwrap_or(0)
    }

    /// Get an entity reference by target.
    pub fn get_entity(&self, target: LootContextEntity) -> Option<EntityRef<'a>> {
        match target {
            LootContextEntity::This => self.this_entity,
            LootContextEntity::Killer => self.killer_entity,
            LootContextEntity::DirectKiller => self.direct_killer_entity,
            LootContextEntity::KillerPlayer => self.last_damage_player,
            LootContextEntity::Interacting => self.interacting_entity,
        }
    }
}

/// A property check for block state conditions.
#[derive(Debug, Clone)]
pub struct PropertyCheck {
    pub name: &'static str,
    pub value: &'static str,
}

/// A condition that must be met for a loot entry or pool to apply.
#[derive(Debug, Clone)]
#[expect(clippy::large_enum_variant)]
pub enum LootCondition {
    /// The loot survives explosion damage (random chance based on explosion radius).
    /// Vanilla: 1/radius chance to pass. If no explosion, always passes.
    SurvivesExplosion,
    /// Check block state properties match expected values.
    BlockStateProperty {
        block: Identifier,
        properties: &'static [PropertyCheck],
    },
    /// Simple random chance (0.0 to 1.0).
    RandomChance(f32),
    /// Random chance affected by an enchantment (e.g., looting).
    /// Vanilla 1.21+: uses enchanted_chance which can be constant or linear.
    RandomChanceWithEnchantedBonus {
        enchantment: Identifier,
        unenchanted_chance: f32,
        /// For linear formula: chance = base + per_level * (level - 1)
        enchanted_chance: EnchantedChance,
    },
    /// Match tool predicate - checks if the tool matches certain criteria.
    MatchTool(ToolPredicate),
    /// Table bonus condition - chance based on enchantment level from a table.
    /// The chances array is indexed by enchantment level (0 = no enchant, 1 = level 1, etc.)
    TableBonus {
        enchantment: Identifier,
        chances: &'static [f32],
    },
    /// Inverted condition - passes if the inner condition fails.
    Inverted(&'static LootCondition),
    /// Any of the conditions pass (OR logic).
    AnyOf(&'static [LootCondition]),
    /// All of the conditions pass (AND logic).
    AllOf(&'static [LootCondition]),
    /// Killed by player condition.
    KilledByPlayer,
    /// Entity properties condition - checks entity predicates.
    EntityProperties {
        entity: LootContextEntity,
        predicate: EntityPredicate,
    },
    /// Damage source properties condition - checks how the entity was damaged.
    DamageSourceProperties { predicate: DamageSourcePredicate },
    /// Location check condition - checks the location predicate.
    LocationCheck {
        offset_x: i32,
        offset_y: i32,
        offset_z: i32,
        predicate: LocationPredicate,
    },
    /// Weather check condition - checks current weather.
    WeatherCheck {
        raining: Option<bool>,
        thundering: Option<bool>,
    },
    /// Time check condition - checks game time.
    TimeCheck {
        value: NumberProviderRange,
        period: Option<i64>,
    },
    /// Value check condition - compares a number provider value against a range.
    ValueCheck {
        value: NumberProvider,
        range: NumberProviderRange,
    },
    /// Check if a specific enchantment is active.
    EnchantmentActiveCheck {
        enchantment: Identifier,
        active: bool,
    },
    /// Check entity scoreboard scores.
    EntityScores {
        entity: LootContextEntity,
        scores: &'static [(&'static str, NumberProviderRange)],
    },
    /// Reference to a named condition in the registry.
    Reference(Identifier),
}

/// Error returned by checked loot condition evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LootConditionEvaluationError {
    /// The condition needs context that the caller did not provide.
    MissingContext(&'static str),
    /// The condition type needs a runtime foundation Steel does not expose to
    /// the checked evaluator yet.
    UnsupportedCondition(&'static str),
    /// The number provider type needs a runtime foundation Steel does not
    /// expose to the checked evaluator yet.
    UnsupportedNumberProvider(&'static str),
}

impl fmt::Display for LootConditionEvaluationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingContext(context) => write!(f, "missing loot context '{context}'"),
            Self::UnsupportedCondition(condition) => {
                write!(f, "unsupported loot condition '{condition}'")
            }
            Self::UnsupportedNumberProvider(provider) => {
                write!(f, "unsupported loot number provider '{provider}'")
            }
        }
    }
}

impl Error for LootConditionEvaluationError {}

/// Runtime-owned loot predicate condition decoded from command/data SNBT.
#[derive(Debug, Clone)]
pub enum RuntimeLootCondition {
    /// The loot survives explosion damage.
    SurvivesExplosion,
    /// Simple random chance condition.
    RandomChance(RuntimeNumberProvider),
    /// Inverted condition.
    Inverted(Box<RuntimeLootCondition>),
    /// Any child condition passes.
    AnyOf(Vec<RuntimeLootCondition>),
    /// All child conditions pass.
    AllOf(Vec<RuntimeLootCondition>),
    /// Killed by player condition.
    KilledByPlayer,
    /// Weather condition.
    WeatherCheck {
        /// Optional raining state.
        raining: Option<bool>,
        /// Optional thundering state.
        thundering: Option<bool>,
    },
    /// Value check condition.
    ValueCheck {
        /// Number provider to evaluate.
        value: RuntimeNumberProvider,
        /// Accepted value range.
        range: RuntimeNumberRange,
    },
    /// Reference to another predicate.
    Reference(Identifier),
    /// Known vanilla condition whose runtime foundation is not available yet.
    Unsupported(&'static str),
}

/// Runtime-owned number provider decoded for command-visible loot predicates.
#[derive(Debug, Clone)]
pub enum RuntimeNumberProvider {
    /// Constant number.
    Constant(f32),
    /// Uniform random number between two provider values.
    Uniform {
        /// Minimum value provider.
        min: Box<RuntimeNumberProvider>,
        /// Maximum value provider.
        max: Box<RuntimeNumberProvider>,
    },
    /// Binomial distribution.
    Binomial {
        /// Trial count provider.
        n: Box<RuntimeNumberProvider>,
        /// Success probability provider.
        p: Box<RuntimeNumberProvider>,
    },
    /// Sum of child providers.
    Sum(Vec<RuntimeNumberProvider>),
    /// Known vanilla provider whose runtime foundation is not available yet.
    Unsupported(&'static str),
}

/// Runtime-owned numeric range.
#[derive(Debug, Clone, Default)]
pub struct RuntimeNumberRange {
    /// Optional minimum bound.
    pub min: Option<RuntimeNumberProvider>,
    /// Optional maximum bound.
    pub max: Option<RuntimeNumberProvider>,
}

/// Error while decoding a runtime loot predicate condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LootPredicateDecodeError {
    /// The top-level value was not a condition compound or inline all-of list.
    ExpectedCondition,
    /// A required field was missing.
    MissingField(&'static str),
    /// A field had the wrong shape.
    InvalidField(&'static str),
    /// A resource identifier was malformed.
    InvalidIdentifier(String),
    /// The condition type is not registered by vanilla.
    UnknownCondition(String),
    /// The number provider type is not registered by vanilla.
    UnknownNumberProvider(String),
}

impl fmt::Display for LootPredicateDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExpectedCondition => write!(f, "expected a loot condition compound or list"),
            Self::MissingField(field) => write!(f, "missing field '{field}'"),
            Self::InvalidField(field) => write!(f, "invalid field '{field}'"),
            Self::InvalidIdentifier(value) => write!(f, "invalid identifier '{value}'"),
            Self::UnknownCondition(value) => write!(f, "unknown loot condition '{value}'"),
            Self::UnknownNumberProvider(value) => {
                write!(f, "unknown loot number provider '{value}'")
            }
        }
    }
}

impl Error for LootPredicateDecodeError {}

impl RuntimeLootCondition {
    /// Decodes vanilla `LootItemCondition.DIRECT_CODEC` command/data shape.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is malformed or names an unknown vanilla
    /// condition type.
    pub fn decode(tag: &NbtTag) -> Result<Self, LootPredicateDecodeError> {
        match tag {
            NbtTag::Compound(compound) => Self::decode_compound(compound),
            NbtTag::List(list) => {
                let mut terms = Vec::new();
                for term in list.as_nbt_tags() {
                    terms.push(Self::decode(&term)?);
                }
                Ok(Self::AllOf(terms))
            }
            _ => Err(LootPredicateDecodeError::ExpectedCondition),
        }
    }

    fn decode_compound(compound: &NbtCompound) -> Result<Self, LootPredicateDecodeError> {
        let condition = required_identifier(compound, "condition")?;
        let Some(condition_name) = vanilla_path(&condition) else {
            return Err(LootPredicateDecodeError::UnknownCondition(
                condition.to_string(),
            ));
        };

        match condition_name {
            "survives_explosion" => Ok(Self::SurvivesExplosion),
            "random_chance" => Ok(Self::RandomChance(RuntimeNumberProvider::decode(
                required_field(compound, "chance")?,
            )?)),
            "inverted" => Ok(Self::Inverted(Box::new(Self::decode(required_field(
                compound, "term",
            )?)?))),
            "any_of" => Ok(Self::AnyOf(decode_condition_terms(compound, "terms")?)),
            "all_of" => Ok(Self::AllOf(decode_condition_terms(compound, "terms")?)),
            "killed_by_player" => Ok(Self::KilledByPlayer),
            "weather_check" => Ok(Self::WeatherCheck {
                raining: optional_bool(compound, "raining")?,
                thundering: optional_bool(compound, "thundering")?,
            }),
            "value_check" => Ok(Self::ValueCheck {
                value: RuntimeNumberProvider::decode(required_field(compound, "value")?)?,
                range: RuntimeNumberRange::decode(required_field(compound, "range")?)?,
            }),
            "reference" => Ok(Self::Reference(required_identifier(compound, "name")?)),
            "random_chance_with_enchanted_bonus" => {
                Ok(Self::Unsupported("random_chance_with_enchanted_bonus"))
            }
            "entity_properties" => Ok(Self::Unsupported("entity_properties")),
            "entity_scores" => Ok(Self::Unsupported("entity_scores")),
            "block_state_property" => Ok(Self::Unsupported("block_state_property")),
            "match_tool" => Ok(Self::Unsupported("match_tool")),
            "table_bonus" => Ok(Self::Unsupported("table_bonus")),
            "damage_source_properties" => Ok(Self::Unsupported("damage_source_properties")),
            "location_check" => Ok(Self::Unsupported("location_check")),
            "time_check" => Ok(Self::Unsupported("time_check")),
            "enchantment_active_check" => Ok(Self::Unsupported("enchantment_active_check")),
            "environment_attribute_check" => Ok(Self::Unsupported("environment_attribute_check")),
            _ => Err(LootPredicateDecodeError::UnknownCondition(
                condition.to_string(),
            )),
        }
    }

    /// Evaluates this predicate against a checked loot context.
    ///
    /// # Errors
    ///
    /// Returns an error when the predicate needs a runtime foundation Steel
    /// cannot evaluate correctly yet.
    pub fn try_test<R: rand::Rng>(
        &self,
        ctx: &mut LootContext<'_, R>,
    ) -> Result<bool, LootConditionEvaluationError> {
        self.try_test_with_depth(ctx, 0)
    }

    fn try_test_with_depth<R: rand::Rng>(
        &self,
        ctx: &mut LootContext<'_, R>,
        reference_depth: usize,
    ) -> Result<bool, LootConditionEvaluationError> {
        const MAX_REFERENCE_DEPTH: usize = 64;

        match self {
            Self::SurvivesExplosion => {
                if let Some(radius) = ctx.explosion_radius {
                    Ok(ctx.rng.random::<f32>() <= (1.0 / radius))
                } else {
                    Ok(true)
                }
            }
            Self::RandomChance(chance) => Ok(ctx.rng.random::<f32>() < chance.try_get(ctx)?),
            Self::Inverted(inner) => Ok(!inner.try_test_with_depth(ctx, reference_depth)?),
            Self::AnyOf(conditions) => {
                for condition in conditions {
                    if condition.try_test_with_depth(ctx, reference_depth)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::AllOf(conditions) => {
                for condition in conditions {
                    if !condition.try_test_with_depth(ctx, reference_depth)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::KilledByPlayer => Ok(ctx.killed_by_player),
            Self::WeatherCheck {
                raining,
                thundering,
            } => {
                let Some(weather) = ctx.weather else {
                    return Err(LootConditionEvaluationError::MissingContext("weather"));
                };
                Ok(raining.is_none_or(|r| r == weather.raining)
                    && thundering.is_none_or(|t| t == weather.thundering))
            }
            Self::ValueCheck { value, range } => {
                let value = value.try_get(ctx)?;
                range.try_test(value, ctx)
            }
            Self::Reference(name) => {
                if reference_depth >= MAX_REFERENCE_DEPTH {
                    return Err(LootConditionEvaluationError::UnsupportedCondition(
                        "reference_cycle",
                    ));
                }
                let Some(predicate) = REGISTRY.loot_predicates.by_key(name) else {
                    return Ok(false);
                };
                predicate
                    .condition
                    .try_test_with_depth(ctx, reference_depth + 1)
            }
            Self::Unsupported(condition) => Err(
                LootConditionEvaluationError::UnsupportedCondition(condition),
            ),
        }
    }
}

impl RuntimeNumberProvider {
    fn decode(tag: &NbtTag) -> Result<Self, LootPredicateDecodeError> {
        if let Some(value) = numeric_tag_as_f32(tag) {
            return Ok(Self::Constant(value));
        }

        let Some(compound) = tag.compound() else {
            return Err(LootPredicateDecodeError::InvalidField("number_provider"));
        };

        let provider_type = if let Some(value) = compound.string("type") {
            required_identifier_value(value.to_str().as_ref())?
        } else if compound.contains("min") && compound.contains("max") {
            Identifier::vanilla_static("uniform")
        } else {
            return Err(LootPredicateDecodeError::MissingField("type"));
        };
        let Some(provider_name) = vanilla_path(&provider_type) else {
            return Err(LootPredicateDecodeError::UnknownNumberProvider(
                provider_type.to_string(),
            ));
        };

        match provider_name {
            "constant" => Ok(Self::Constant(required_number(compound, "value")?)),
            "uniform" => Ok(Self::Uniform {
                min: Box::new(Self::decode(required_field(compound, "min")?)?),
                max: Box::new(Self::decode(required_field(compound, "max")?)?),
            }),
            "binomial" => Ok(Self::Binomial {
                n: Box::new(Self::decode(required_field(compound, "n")?)?),
                p: Box::new(Self::decode(required_field(compound, "p")?)?),
            }),
            "sum" => Ok(Self::Sum(decode_number_terms(compound, "summands")?)),
            "score" => Ok(Self::Unsupported("score")),
            "storage" => Ok(Self::Unsupported("storage")),
            "enchantment_level" => Ok(Self::Unsupported("enchantment_level")),
            "environment_attribute" => Ok(Self::Unsupported("environment_attribute")),
            _ => Err(LootPredicateDecodeError::UnknownNumberProvider(
                provider_type.to_string(),
            )),
        }
    }

    fn try_get<R: rand::Rng>(
        &self,
        ctx: &mut LootContext<'_, R>,
    ) -> Result<f32, LootConditionEvaluationError> {
        match self {
            Self::Constant(value) => Ok(*value),
            Self::Uniform { min, max } => {
                let min = min.try_get(ctx)?;
                let max = max.try_get(ctx)?;
                if min >= max {
                    return Ok(min);
                }
                Ok(ctx.rng.random_range(min..=max))
            }
            Self::Binomial { n, p } => {
                let n = n.try_get(ctx)?.floor() as i32;
                let p = p.try_get(ctx)?;
                let mut count = 0;
                for _ in 0..n.max(0) {
                    if ctx.rng.random::<f32>() < p {
                        count += 1;
                    }
                }
                Ok(count as f32)
            }
            Self::Sum(providers) => {
                let mut sum = 0.0;
                for provider in providers {
                    sum += provider.try_get(ctx)?;
                }
                Ok(sum)
            }
            Self::Unsupported(provider) => Err(
                LootConditionEvaluationError::UnsupportedNumberProvider(provider),
            ),
        }
    }
}

impl RuntimeNumberRange {
    fn decode(tag: &NbtTag) -> Result<Self, LootPredicateDecodeError> {
        if let Some(value) = numeric_tag_as_f32(tag) {
            return Ok(Self {
                min: Some(RuntimeNumberProvider::Constant(value)),
                max: Some(RuntimeNumberProvider::Constant(value)),
            });
        }

        let Some(compound) = tag.compound() else {
            return Err(LootPredicateDecodeError::InvalidField("range"));
        };
        Ok(Self {
            min: compound
                .get("min")
                .map(RuntimeNumberProvider::decode)
                .transpose()?,
            max: compound
                .get("max")
                .map(RuntimeNumberProvider::decode)
                .transpose()?,
        })
    }

    fn try_test<R: rand::Rng>(
        &self,
        value: f32,
        ctx: &mut LootContext<'_, R>,
    ) -> Result<bool, LootConditionEvaluationError> {
        if let Some(min) = &self.min
            && value < min.try_get(ctx)?
        {
            return Ok(false);
        }
        if let Some(max) = &self.max
            && value > max.try_get(ctx)?
        {
            return Ok(false);
        }
        Ok(true)
    }
}

fn decode_condition_terms(
    compound: &NbtCompound,
    field: &'static str,
) -> Result<Vec<RuntimeLootCondition>, LootPredicateDecodeError> {
    let Some(terms) = required_field(compound, field)?.list() else {
        return Err(LootPredicateDecodeError::InvalidField(field));
    };
    let mut decoded = Vec::new();
    for term in terms.as_nbt_tags() {
        decoded.push(RuntimeLootCondition::decode(&term)?);
    }
    Ok(decoded)
}

fn decode_number_terms(
    compound: &NbtCompound,
    field: &'static str,
) -> Result<Vec<RuntimeNumberProvider>, LootPredicateDecodeError> {
    let Some(terms) = required_field(compound, field)?.list() else {
        return Err(LootPredicateDecodeError::InvalidField(field));
    };
    let mut decoded = Vec::new();
    for term in terms.as_nbt_tags() {
        decoded.push(RuntimeNumberProvider::decode(&term)?);
    }
    Ok(decoded)
}

fn required_field<'a>(
    compound: &'a NbtCompound,
    field: &'static str,
) -> Result<&'a NbtTag, LootPredicateDecodeError> {
    compound
        .get(field)
        .ok_or(LootPredicateDecodeError::MissingField(field))
}

fn required_identifier(
    compound: &NbtCompound,
    field: &'static str,
) -> Result<Identifier, LootPredicateDecodeError> {
    let Some(value) = compound.string(field) else {
        return Err(LootPredicateDecodeError::MissingField(field));
    };
    required_identifier_value(value.to_str().as_ref())
}

fn required_identifier_value(value: &str) -> Result<Identifier, LootPredicateDecodeError> {
    let normalized = if value.contains(':') {
        value.to_owned()
    } else {
        format!("minecraft:{value}")
    };
    normalized
        .parse()
        .map_err(|_| LootPredicateDecodeError::InvalidIdentifier(value.to_owned()))
}

fn vanilla_path(identifier: &Identifier) -> Option<&str> {
    (identifier.namespace == Identifier::VANILLA_NAMESPACE).then_some(identifier.path.as_ref())
}

fn required_number(
    compound: &NbtCompound,
    field: &'static str,
) -> Result<f32, LootPredicateDecodeError> {
    let tag = required_field(compound, field)?;
    numeric_tag_as_f32(tag).ok_or(LootPredicateDecodeError::InvalidField(field))
}

fn numeric_tag_as_f32(tag: &NbtTag) -> Option<f32> {
    match tag {
        NbtTag::Byte(value) => Some(f32::from(*value)),
        NbtTag::Short(value) => Some(f32::from(*value)),
        NbtTag::Int(value) => Some(*value as f32),
        NbtTag::Long(value) => Some(*value as f32),
        NbtTag::Float(value) => Some(*value),
        NbtTag::Double(value) => Some(*value as f32),
        _ => None,
    }
}

fn optional_bool(
    compound: &NbtCompound,
    field: &'static str,
) -> Result<Option<bool>, LootPredicateDecodeError> {
    let Some(tag) = compound.get(field) else {
        return Ok(None);
    };
    match tag {
        NbtTag::Byte(0) => Ok(Some(false)),
        NbtTag::Byte(1) => Ok(Some(true)),
        _ => Err(LootPredicateDecodeError::InvalidField(field)),
    }
}

/// Enchanted chance calculation method.
#[derive(Debug, Clone, Copy)]
pub enum EnchantedChance {
    /// Constant chance regardless of enchantment level.
    Constant(f32),
    /// Linear formula: base + per_level_above_first * (level - 1)
    Linear {
        base: f32,
        per_level_above_first: f32,
    },
}

/// Predicate for matching tools.
#[derive(Debug, Clone)]
pub enum ToolPredicate {
    /// Match a specific item.
    Item(Identifier),
    /// Match items with a specific enchantment at minimum level.
    HasEnchantment {
        enchantment: Identifier,
        min_level: i32,
    },
    /// Match items in a tag.
    Tag(Identifier),
    /// Always matches (no predicate specified).
    Any,
}

/// Predicate for checking location/block properties.
#[derive(Debug, Clone)]
pub struct LocationPredicate {
    pub block: Option<BlockPredicate>,
}

/// Predicate for checking block properties.
#[derive(Debug, Clone)]
pub struct BlockPredicate {
    pub blocks: Option<Identifier>,
    pub state: &'static [(&'static str, &'static str)],
}

/// Predicate for checking entity properties.
#[derive(Debug, Clone)]
pub struct EntityPredicate {
    pub entity_type: Option<Identifier>,
    pub flags: Option<EntityFlags>,
    pub equipment: Option<EntityEquipment>,
}

/// Entity flags (is_on_fire, is_sneaking, etc.)
#[derive(Debug, Clone)]
pub struct EntityFlags {
    pub is_on_fire: Option<bool>,
    pub is_sneaking: Option<bool>,
    pub is_sprinting: Option<bool>,
    pub is_swimming: Option<bool>,
    pub is_baby: Option<bool>,
}

/// Entity equipment predicate
#[derive(Debug, Clone)]
pub struct EntityEquipment {
    pub mainhand: Option<ToolPredicate>,
    pub offhand: Option<ToolPredicate>,
    pub head: Option<ToolPredicate>,
    pub chest: Option<ToolPredicate>,
    pub legs: Option<ToolPredicate>,
    pub feet: Option<ToolPredicate>,
}

/// Predicate for checking damage source properties.
#[derive(Debug, Clone)]
pub struct DamageSourcePredicate {
    /// Tags that must be present on the damage source.
    pub tags: &'static [DamageTagPredicate],
    /// Source entity predicate (e.g., the player/mob that caused damage).
    pub source_entity: Option<EntityPredicate>,
    /// Direct entity predicate (e.g., the arrow/fireball).
    pub direct_entity: Option<EntityPredicate>,
    /// Whether damage bypasses armor.
    pub is_direct: Option<bool>,
}

/// A tag check for damage source.
#[derive(Debug, Clone)]
pub struct DamageTagPredicate {
    pub id: Identifier,
    pub expected: bool,
}

impl LootCondition {
    /// Test if this condition passes without silently accepting unsupported
    /// condition types.
    ///
    /// Use this for command-visible predicates where permissive fallbacks would
    /// be observable. The existing [`Self::test`] method is retained for the
    /// current generated loot-table path until those TODO-backed semantics are
    /// replaced end to end.
    ///
    /// # Errors
    ///
    /// Returns an error when the condition or one of its number providers needs
    /// a runtime foundation Steel cannot evaluate correctly yet.
    pub fn try_test<R: rand::Rng>(
        &self,
        ctx: &mut LootContext<'_, R>,
    ) -> Result<bool, LootConditionEvaluationError> {
        match self {
            LootCondition::SurvivesExplosion => {
                if let Some(radius) = ctx.explosion_radius {
                    Ok(ctx.rng.random::<f32>() <= (1.0 / radius))
                } else {
                    Ok(true)
                }
            }
            LootCondition::BlockStateProperty { block, properties } => {
                let Some(state) = ctx.block_state else {
                    return Ok(false);
                };
                let state_block = state.get_block();
                if state_block.key != *block {
                    return Ok(false);
                }
                for prop in *properties {
                    let Some(value) = state.get_property_str(prop.name) else {
                        return Ok(false);
                    };
                    if value != prop.value {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            LootCondition::RandomChance(chance) => Ok(ctx.rng.random::<f32>() < *chance),
            LootCondition::RandomChanceWithEnchantedBonus {
                enchantment,
                unenchanted_chance,
                enchanted_chance,
            } => {
                let level = ctx.get_enchantment_level_by_id(enchantment);
                let effective_chance = if level > 0 {
                    match enchanted_chance {
                        EnchantedChance::Constant(c) => *c,
                        EnchantedChance::Linear {
                            base,
                            per_level_above_first,
                        } => base + per_level_above_first * (level - 1) as f32,
                    }
                } else {
                    *unenchanted_chance
                };
                Ok(ctx.rng.random::<f32>() < effective_chance)
            }
            LootCondition::MatchTool(predicate) => Ok(if let Some(tool) = ctx.tool {
                predicate.test(tool, ctx)
            } else {
                matches!(predicate, ToolPredicate::Any)
            }),
            LootCondition::TableBonus {
                enchantment,
                chances,
            } => {
                let level = ctx.get_enchantment_level_by_id(enchantment);
                let index = (level as usize).min(chances.len().saturating_sub(1));
                let chance = chances.get(index).copied().unwrap_or(0.0);
                Ok(ctx.rng.random::<f32>() < chance)
            }
            LootCondition::Inverted(inner) => Ok(!inner.try_test(ctx)?),
            LootCondition::AnyOf(conditions) => {
                for condition in *conditions {
                    if condition.try_test(ctx)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            LootCondition::AllOf(conditions) => {
                for condition in *conditions {
                    if !condition.try_test(ctx)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            LootCondition::KilledByPlayer => Ok(ctx.killed_by_player),
            LootCondition::EntityProperties { entity, predicate } => {
                let Some(entity) = ctx.get_entity(*entity) else {
                    return Ok(false);
                };
                Ok(predicate.test(entity, ctx))
            }
            LootCondition::DamageSourceProperties { predicate } => Ok(predicate.test(ctx)),
            LootCondition::LocationCheck { .. } => Err(
                LootConditionEvaluationError::UnsupportedCondition("location_check"),
            ),
            LootCondition::WeatherCheck {
                raining,
                thundering,
            } => {
                let Some(weather) = ctx.weather else {
                    return Err(LootConditionEvaluationError::MissingContext("weather"));
                };
                Ok(raining.is_none_or(|r| r == weather.raining)
                    && thundering.is_none_or(|t| t == weather.thundering))
            }
            LootCondition::TimeCheck { value, period } => {
                let Some(game_time) = ctx.game_time else {
                    return Err(LootConditionEvaluationError::MissingContext("game_time"));
                };
                let time = if let Some(p) = period {
                    game_time % p
                } else {
                    game_time
                };
                let ctx_ref = LootContextRef { tool: ctx.tool };
                value.try_test(time as f32, ctx.rng, Some(&ctx_ref))
            }
            LootCondition::ValueCheck { value, range } => {
                let ctx_ref = LootContextRef { tool: ctx.tool };
                let value = value.try_get(ctx.rng, Some(&ctx_ref))?;
                range.try_test(value, ctx.rng, Some(&ctx_ref))
            }
            LootCondition::EnchantmentActiveCheck {
                enchantment,
                active,
            } => {
                let level = ctx.get_enchantment_level_by_id(enchantment);
                let is_active = level > 0;
                Ok(is_active == *active)
            }
            LootCondition::EntityScores { .. } => Err(
                LootConditionEvaluationError::UnsupportedCondition("entity_scores"),
            ),
            LootCondition::Reference(_) => Err(LootConditionEvaluationError::UnsupportedCondition(
                "reference",
            )),
        }
    }

    /// Test if this condition passes given the loot context.
    pub fn test<R: rand::Rng>(&self, ctx: &mut LootContext<'_, R>) -> bool {
        match self {
            LootCondition::SurvivesExplosion => {
                if let Some(radius) = ctx.explosion_radius {
                    // Vanilla: 1/radius chance to survive
                    ctx.rng.random::<f32>() <= (1.0 / radius)
                } else {
                    true // No explosion, always survives
                }
            }
            LootCondition::BlockStateProperty { block, properties } => {
                if let Some(state) = ctx.block_state {
                    let state_block = state.get_block();
                    // Check block matches
                    if state_block.key != *block {
                        return false;
                    }
                    // Check all properties match
                    for prop in *properties {
                        if let Some(value) = state.get_property_str(prop.name) {
                            if value != prop.value {
                                return false;
                            }
                        } else {
                            return false; // Property doesn't exist
                        }
                    }
                    true
                } else {
                    false // No block state in context
                }
            }
            LootCondition::RandomChance(chance) => ctx.rng.random::<f32>() < *chance,
            LootCondition::RandomChanceWithEnchantedBonus {
                enchantment,
                unenchanted_chance,
                enchanted_chance,
            } => {
                let level = ctx.get_enchantment_level_by_id(enchantment);
                let effective_chance = if level > 0 {
                    match enchanted_chance {
                        EnchantedChance::Constant(c) => *c,
                        EnchantedChance::Linear {
                            base,
                            per_level_above_first,
                        } => base + per_level_above_first * (level - 1) as f32,
                    }
                } else {
                    *unenchanted_chance
                };
                ctx.rng.random::<f32>() < effective_chance
            }
            LootCondition::MatchTool(predicate) => {
                if let Some(tool) = ctx.tool {
                    predicate.test(tool, ctx)
                } else {
                    // No tool in context - only passes if predicate is Any
                    matches!(predicate, ToolPredicate::Any)
                }
            }
            LootCondition::TableBonus {
                enchantment,
                chances,
            } => {
                let level = ctx.get_enchantment_level_by_id(enchantment);
                let index = (level as usize).min(chances.len().saturating_sub(1));
                let chance = chances.get(index).copied().unwrap_or(0.0);
                ctx.rng.random::<f32>() < chance
            }
            LootCondition::Inverted(inner) => !inner.test(ctx),
            LootCondition::AnyOf(conditions) => conditions.iter().any(|c| c.test(ctx)),
            LootCondition::AllOf(conditions) => conditions.iter().all(|c| c.test(ctx)),
            LootCondition::KilledByPlayer => ctx.killed_by_player,
            LootCondition::EntityProperties { entity, predicate } => {
                let Some(entity) = ctx.get_entity(*entity) else {
                    return false;
                };
                predicate.test(entity, ctx)
            }
            LootCondition::DamageSourceProperties { predicate } => predicate.test(ctx),
            LootCondition::LocationCheck { .. } => {
                // TODO: Implement when world position data is available in context
                true
            }
            LootCondition::WeatherCheck {
                raining,
                thundering,
            } => {
                let weather = ctx.weather.unwrap_or_default();
                raining.is_none_or(|r| r == weather.raining)
                    && thundering.is_none_or(|t| t == weather.thundering)
            }
            LootCondition::TimeCheck { value, period } => {
                let game_time = ctx.game_time.unwrap_or(0);
                let time = if let Some(p) = period {
                    game_time % p
                } else {
                    game_time
                };
                value.test(time as f32, ctx.rng)
            }
            LootCondition::ValueCheck { value, range } => {
                let v = value.get_simple(ctx.rng);
                range.test(v, ctx.rng)
            }
            LootCondition::EnchantmentActiveCheck {
                enchantment,
                active,
            } => {
                let level = ctx.get_enchantment_level_by_id(enchantment);
                let is_active = level > 0;
                is_active == *active
            }
            LootCondition::EntityScores { .. } => {
                // TODO: Implement when scoreboard system is available
                true
            }
            LootCondition::Reference(_name) => {
                // TODO: Implement condition registry lookup
                // For now, return true (permissive)
                true
            }
        }
    }
}

impl ToolPredicate {
    /// Test if the tool matches this predicate.
    pub fn test<R: rand::Rng>(&self, tool: &ItemStack, _ctx: &LootContext<'_, R>) -> bool {
        match self {
            ToolPredicate::Item(item_id) => tool.item.key == *item_id,
            ToolPredicate::HasEnchantment {
                enchantment,
                min_level,
            } => tool_enchantment_matches(tool, enchantment, *min_level),
            ToolPredicate::Tag(tag) => {
                // Check if the tool's item is in the specified tag
                REGISTRY.items.is_in_tag(tool.item, tag)
            }
            ToolPredicate::Any => true,
        }
    }
}

fn tool_enchantment_matches(
    tool: &ItemStack,
    enchantment_or_tag: &Identifier,
    min_level: i32,
) -> bool {
    if tool.get_enchantment_level(enchantment_or_tag) >= min_level {
        return true;
    }

    let Some(enchantments) = tool.get_enchantments() else {
        return false;
    };

    for (key, level) in enchantments.iter() {
        if *level < min_level as u32 {
            continue;
        }
        let Some(enchantment) = REGISTRY.enchantments.by_key(key) else {
            continue;
        };
        if REGISTRY
            .enchantments
            .is_in_tag(enchantment, enchantment_or_tag)
        {
            return true;
        }
    }

    false
}

impl EntityPredicate {
    fn test<R: rand::Rng>(&self, entity: EntityRef<'_>, ctx: &LootContext<'_, R>) -> bool {
        if let Some(entity_type) = &self.entity_type
            && entity.entity_type != Some(entity_type)
        {
            return false;
        }

        if let Some(flags) = &self.flags
            && !flags.test(entity.flags)
        {
            return false;
        }

        if let Some(equipment) = &self.equipment
            && !equipment.test(entity.equipment, ctx)
        {
            return false;
        }

        true
    }
}

impl EntityFlags {
    fn test(&self, flags: EntityRefFlags) -> bool {
        self.is_on_fire
            .is_none_or(|expected| expected == flags.is_on_fire)
            && self
                .is_sneaking
                .is_none_or(|expected| expected == flags.is_sneaking)
            && self
                .is_sprinting
                .is_none_or(|expected| expected == flags.is_sprinting)
            && self
                .is_swimming
                .is_none_or(|expected| expected == flags.is_swimming)
            && self
                .is_baby
                .is_none_or(|expected| expected == flags.is_baby)
    }
}

impl EntityEquipment {
    fn test<R: rand::Rng>(
        &self,
        equipment: Option<&EntityEquipmentRef<'_>>,
        ctx: &LootContext<'_, R>,
    ) -> bool {
        let has_predicate = self.mainhand.is_some()
            || self.offhand.is_some()
            || self.head.is_some()
            || self.chest.is_some()
            || self.legs.is_some()
            || self.feet.is_some();
        if !has_predicate {
            return true;
        }

        let Some(equipment) = equipment else {
            return false;
        };

        slot_predicate_matches(&self.mainhand, equipment.mainhand, ctx)
            && slot_predicate_matches(&self.offhand, equipment.offhand, ctx)
            && slot_predicate_matches(&self.head, equipment.head, ctx)
            && slot_predicate_matches(&self.chest, equipment.chest, ctx)
            && slot_predicate_matches(&self.legs, equipment.legs, ctx)
            && slot_predicate_matches(&self.feet, equipment.feet, ctx)
    }
}

fn slot_predicate_matches<R: rand::Rng>(
    predicate: &Option<ToolPredicate>,
    item_stack: Option<&ItemStack>,
    ctx: &LootContext<'_, R>,
) -> bool {
    let Some(predicate) = predicate else {
        return true;
    };
    let Some(item_stack) = item_stack else {
        return false;
    };
    predicate.test(item_stack, ctx)
}

impl DamageSourcePredicate {
    fn test<R: rand::Rng>(&self, ctx: &LootContext<'_, R>) -> bool {
        let Some(damage_source) = ctx.damage_source else {
            return false;
        };

        for tag in self.tags {
            if damage_source_has_tag(damage_source, &tag.id) != tag.expected {
                return false;
            }
        }

        if let Some(expected) = self.is_direct
            && damage_source.is_direct != expected
        {
            return false;
        }

        if let Some(predicate) = &self.source_entity {
            let Some(entity) = ctx.killer_entity else {
                return false;
            };
            if !predicate.test(entity, ctx) {
                return false;
            }
        }

        if let Some(predicate) = &self.direct_entity {
            let Some(entity) = ctx.direct_killer_entity else {
                return false;
            };
            if !predicate.test(entity, ctx) {
                return false;
            }
        }

        true
    }
}

fn damage_source_has_tag(damage_source: DamageSourceInfo<'_>, tag: &Identifier) -> bool {
    if let Some(damage_type) = damage_source.damage_type
        && let Some(damage_type) = REGISTRY.damage_types.by_key(damage_type)
    {
        return REGISTRY.damage_types.is_in_tag(damage_type, tag);
    }

    damage_source.tags.iter().any(|candidate| candidate == tag)
}

/// Options for selecting enchantments - either a tag reference or explicit list.
#[derive(Debug, Clone)]
pub enum EnchantmentOptions {
    /// Reference to an enchantment tag (e.g., "on_random_loot").
    Tag(Identifier),
    /// Explicit list of enchantment IDs.
    List(&'static [Identifier]),
}

/// A function with optional conditions.
#[derive(Debug, Clone)]
pub struct ConditionalLootFunction {
    pub function: LootFunction,
    pub conditions: &'static [LootCondition],
}

/// A function that modifies loot items.
#[derive(Debug, Clone)]
pub enum LootFunction {
    /// Set the count of the item.
    SetCount { count: NumberProvider, add: bool },
    /// Apply explosion decay - each item has 1/radius chance to survive.
    ExplosionDecay,
    /// Apply bonus count based on enchantment level.
    ApplyBonus {
        enchantment: Identifier,
        formula: BonusFormula,
    },
    /// Increase count based on enchantment (like looting).
    EnchantedCountIncrease {
        enchantment: Identifier,
        count: NumberProvider,
        limit: i32,
    },
    /// Limit the count to a range.
    LimitCount { min: Option<i32>, max: Option<i32> },
    /// Set the damage of the item (0.0 = broken, 1.0 = full durability).
    SetDamage { damage: NumberProvider, add: bool },
    /// Enchant the item randomly with enchantments from options.
    EnchantRandomly { options: EnchantmentOptions },
    /// Enchant the item as if using an enchanting table at the specified level.
    EnchantWithLevels {
        levels: NumberProvider,
        options: EnchantmentOptions,
    },
    /// Copy components from the block entity to the item.
    CopyComponents {
        source: CopySource,
        include: &'static [Identifier],
    },
    /// Copy block state properties to the item.
    CopyState {
        block: Identifier,
        properties: &'static [&'static str],
    },
    /// Set components on the item.
    SetComponents { components: &'static str },
    /// Set custom NBT data on the item (merges with existing custom_data).
    SetCustomData { tag: &'static str },
    /// Smelt the item (convert raw to cooked, ore to ingot, etc.).
    FurnaceSmelt { use_input_count: bool },
    /// Create an exploration map pointing to a structure.
    ExplorationMap {
        destination: Identifier,
        decoration: Identifier,
        zoom: i32,
        skip_existing_chunks: bool,
    },
    /// Set the custom name of the item.
    SetName {
        name: &'static str,
        target: NameTarget,
    },
    /// Set the ominous bottle amplifier.
    SetOminousBottleAmplifier { amplifier: NumberProvider },
    /// Set the potion type.
    SetPotion { id: Identifier },
    /// Set the suspicious stew effects.
    SetStewEffect { effects: &'static [StewEffect] },
    /// Set the instrument for goat horns.
    SetInstrument { options: Identifier },
    /// Set enchantments on the item.
    SetEnchantments {
        enchantments: &'static [(Identifier, NumberProvider)],
        add: bool,
    },
    /// Change the item type entirely.
    SetItem { item: Identifier },
    /// Copy name from source entity/block to item.
    CopyName { source: CopySource },
    /// Add lore lines to the item.
    SetLore {
        lore: &'static [&'static str],
        mode: ListOperation,
    },
    /// Set container inventory contents.
    SetContents {
        entries: &'static [LootEntry],
        component_type: Identifier,
    },
    /// Modify existing container contents.
    ModifyContents {
        modifier: &'static [ConditionalLootFunction],
        component_type: Identifier,
    },
    /// Set container's loot table reference.
    SetLootTable {
        loot_table: Identifier,
        seed: Option<i64>,
    },
    /// Set attribute modifiers on the item.
    SetAttributes {
        modifiers: &'static [AttributeModifier],
        replace: bool,
    },
    /// Fill player head with texture from entity.
    FillPlayerHead { entity: LootContextEntity },
    /// Copy NBT/custom data from source.
    CopyCustomData {
        source: CopySource,
        operations: &'static [CopyDataOperation],
    },
    /// Set banner pattern layers.
    SetBannerPattern {
        patterns: &'static [BannerPattern],
        append: bool,
    },
    /// Set firework rocket properties.
    SetFireworks {
        explosions: Option<&'static [FireworkExplosion]>,
        flight_duration: Option<i32>,
    },
    /// Set firework star explosion properties.
    SetFireworkExplosion { explosion: FireworkExplosion },
    /// Set book cover (title/author for written books).
    SetBookCover {
        title: Option<&'static str>,
        author: Option<&'static str>,
        generation: Option<i32>,
    },
    /// Set written book page contents.
    SetWrittenBookPages {
        pages: &'static [&'static str],
        mode: ListOperation,
    },
    /// Set writable book page contents.
    SetWritableBookPages {
        pages: &'static [&'static str],
        mode: ListOperation,
    },
    /// Toggle tooltip visibility.
    ToggleTooltips {
        toggles: &'static [(Identifier, bool)],
    },
    /// Set custom model data.
    SetCustomModelData { value: NumberProvider },
    /// Discard/delete the item entirely.
    Discard,
    /// Reference to a named function in the registry.
    Reference(Identifier),
    /// Apply multiple functions in sequence.
    Sequence {
        functions: &'static [ConditionalLootFunction],
    },
    /// Conditionally apply function to specific item predicate matches.
    Filtered {
        item_filter: ToolPredicate,
        modifier: &'static ConditionalLootFunction,
    },
}

/// Operation mode for list modifications (lore, book pages).
#[derive(Debug, Clone, Copy)]
pub enum ListOperation {
    /// Replace all existing entries.
    ReplaceAll,
    /// Replace a section of entries.
    ReplaceSection { offset: i32, size: Option<i32> },
    /// Insert before existing entries.
    InsertBefore { offset: i32 },
    /// Insert after existing entries.
    InsertAfter { offset: i32 },
    /// Append to the end.
    Append,
}

/// An attribute modifier for SetAttributes function.
#[derive(Debug, Clone)]
pub struct AttributeModifier {
    pub attribute: Identifier,
    pub operation: AttributeOperation,
    pub amount: NumberProvider,
    pub id: Identifier,
    pub slot: EquipmentSlotGroup,
}

/// Attribute modifier operation type.
#[expect(clippy::enum_variant_names, reason = "matches Vanilla naming")]
#[derive(Debug, Clone, Copy)]
pub enum AttributeOperation {
    AddValue,
    AddMultipliedBase,
    AddMultipliedTotal,
}

/// Copy data operation for CopyCustomData.
#[derive(Debug, Clone)]
pub struct CopyDataOperation {
    pub source_path: &'static str,
    pub target_path: &'static str,
    pub op: CopyDataOp,
}

/// Operation type for data copying.
#[derive(Debug, Clone, Copy)]
pub enum CopyDataOp {
    Replace,
    Append,
    Merge,
}

/// A banner pattern layer.
#[derive(Debug, Clone)]
pub struct BannerPattern {
    pub pattern: Identifier,
    pub color: DyeColor,
}

/// A firework explosion definition.
#[derive(Debug, Clone)]
pub struct FireworkExplosion {
    pub shape: FireworkShape,
    pub colors: &'static [i32],
    pub fade_colors: &'static [i32],
    pub has_trail: bool,
    pub has_twinkle: bool,
}

/// Firework explosion shape.
#[derive(Debug, Clone, Copy)]
pub enum FireworkShape {
    SmallBall,
    LargeBall,
    Star,
    Creeper,
    Burst,
}

/// Formula types for apply_bonus function.
#[derive(Debug, Clone, Copy)]
pub enum BonusFormula {
    /// Ore drops formula: count * (max(0, random(0..level+2) - 1) + 1)
    OreDrops,
    /// Uniform bonus: count + random(0..bonusMultiplier * level + 1)
    UniformBonusCount { bonus_multiplier: i32 },
    /// Binomial with bonus count: for each of (level + extra) trials, probability p to add 1
    BinomialWithBonusCount { extra: i32, probability: f32 },
}

/// Source for copying components.
#[derive(Debug, Clone, Copy)]
pub enum CopySource {
    BlockEntity,
    This,
    Attacker,
    DirectAttacker,
}

/// Target for set_name function.
#[derive(Debug, Clone, Copy)]
pub enum NameTarget {
    CustomName,
    ItemName,
}

/// A stew effect for suspicious stew.
#[derive(Debug, Clone)]
pub struct StewEffect {
    pub effect_type: Identifier,
    pub duration: NumberProvider,
}

/// A loot table entry that can generate items.
#[derive(Debug, Clone)]
pub enum LootEntry {
    /// Drop a specific item.
    Item {
        name: Identifier,
        weight: i32,
        quality: i32,
        conditions: &'static [LootCondition],
        functions: &'static [ConditionalLootFunction],
    },
    /// Reference another loot table by name.
    LootTableRef {
        name: Identifier,
        weight: i32,
        quality: i32,
        conditions: &'static [LootCondition],
        functions: &'static [ConditionalLootFunction],
    },
    /// Inline loot table (embedded pools directly in entry).
    InlineLootTable {
        pools: &'static [LootPool],
        weight: i32,
        quality: i32,
        conditions: &'static [LootCondition],
        functions: &'static [ConditionalLootFunction],
    },
    /// Drop items from a tag.
    Tag {
        name: Identifier,
        expand: bool,
        weight: i32,
        quality: i32,
        conditions: &'static [LootCondition],
        functions: &'static [ConditionalLootFunction],
    },
    /// Try children in order, use first that matches.
    Alternatives {
        children: &'static [LootEntry],
        conditions: &'static [LootCondition],
    },
    /// Use all children.
    Group {
        children: &'static [LootEntry],
        conditions: &'static [LootCondition],
    },
    /// Use children in sequence until one fails.
    Sequence {
        children: &'static [LootEntry],
        conditions: &'static [LootCondition],
    },
    /// Empty entry (no drop).
    Empty {
        weight: i32,
        conditions: &'static [LootCondition],
    },
    /// Dynamic content (e.g., block entity contents).
    Dynamic {
        name: Identifier,
        conditions: &'static [LootCondition],
    },
    /// Select items from specific block entity slots.
    Slots {
        /// Slots to select from (can be single slot or range).
        slots: SlotRange,
        conditions: &'static [LootCondition],
        functions: &'static [ConditionalLootFunction],
    },
}

/// A range of slots for the Slots entry type.
#[derive(Debug, Clone, Copy)]
pub enum SlotRange {
    /// A single specific slot index.
    Single(i32),
    /// A range of slots (inclusive).
    Range { min: i32, max: i32 },
    /// All contents slots.
    Contents,
    /// Specific named slots (for entities).
    Named(&'static [&'static str]),
}

impl LootEntry {
    /// Get the weight of this entry for random selection.
    pub fn weight(&self) -> i32 {
        match self {
            Self::Item { weight, .. } => *weight,
            Self::LootTableRef { weight, .. } => *weight,
            Self::InlineLootTable { weight, .. } => *weight,
            Self::Tag { weight, .. } => *weight,
            Self::Empty { weight, .. } => *weight,
            // Composite entries don't have weight
            Self::Alternatives { .. }
            | Self::Group { .. }
            | Self::Sequence { .. }
            | Self::Dynamic { .. }
            | Self::Slots { .. } => 1,
        }
    }

    /// Get the quality modifier for luck-based weight adjustment.
    pub fn quality(&self) -> i32 {
        match self {
            Self::Item { quality, .. } => *quality,
            Self::LootTableRef { quality, .. } => *quality,
            Self::InlineLootTable { quality, .. } => *quality,
            Self::Tag { quality, .. } => *quality,
            Self::Empty { .. }
            | Self::Alternatives { .. }
            | Self::Group { .. }
            | Self::Sequence { .. }
            | Self::Dynamic { .. }
            | Self::Slots { .. } => 0,
        }
    }

    /// Get the effective weight adjusted for luck.
    /// Formula: max(floor(weight + quality * luck), 0)
    pub fn effective_weight(&self, luck: f32) -> i32 {
        let base = self.weight() as f32;
        let quality = self.quality() as f32;
        (base + quality * luck).floor().max(0.0) as i32
    }

    /// Get the conditions for this entry.
    pub fn conditions(&self) -> &'static [LootCondition] {
        match self {
            Self::Item { conditions, .. } => conditions,
            Self::LootTableRef { conditions, .. } => conditions,
            Self::InlineLootTable { conditions, .. } => conditions,
            Self::Tag { conditions, .. } => conditions,
            Self::Alternatives { conditions, .. } => conditions,
            Self::Group { conditions, .. } => conditions,
            Self::Sequence { conditions, .. } => conditions,
            Self::Empty { conditions, .. } => conditions,
            Self::Dynamic { conditions, .. } => conditions,
            Self::Slots { conditions, .. } => conditions,
        }
    }

    /// Get the functions for this entry.
    pub fn functions(&self) -> &'static [ConditionalLootFunction] {
        match self {
            Self::Item { functions, .. } => functions,
            Self::LootTableRef { functions, .. } => functions,
            Self::InlineLootTable { functions, .. } => functions,
            Self::Tag { functions, .. } => functions,
            Self::Slots { functions, .. } => functions,
            Self::Empty { .. }
            | Self::Alternatives { .. }
            | Self::Group { .. }
            | Self::Sequence { .. }
            | Self::Dynamic { .. } => &[],
        }
    }
}

/// A pool of loot entries with roll counts.
#[derive(Debug, Clone)]
pub struct LootPool {
    pub rolls: NumberProvider,
    pub bonus_rolls: f32,
    pub entries: &'static [LootEntry],
    pub conditions: &'static [LootCondition],
    pub functions: &'static [ConditionalLootFunction],
}

/// A complete loot table definition.
#[derive(Debug)]
pub struct LootTable {
    pub key: Identifier,
    pub loot_type: LootType,
    pub pools: &'static [LootPool],
    pub functions: &'static [ConditionalLootFunction],
    pub random_sequence: Option<Identifier>,
}

impl LootTable {
    /// Generate random items from this loot table.
    ///
    /// # Arguments
    /// * `ctx` - The loot context containing RNG, luck, block state, tool, etc.
    ///
    /// This follows vanilla's approach:
    /// 1. For each pool, check conditions
    /// 2. Roll `rolls + floor(bonus_rolls * luck)` times
    /// 3. Each roll does weighted random selection among valid entries
    /// 4. Apply entry-level functions to each item
    /// 5. Apply pool-level functions to all items from that pool
    /// 6. Apply table-level functions to all items from the table
    pub fn get_random_items<R: rand::Rng>(&self, ctx: &mut LootContext<'_, R>) -> Vec<ItemStack> {
        let mut result = Vec::new();
        for pool in self.pools {
            pool.add_random_items(ctx, &mut result);
        }

        // Apply table-level functions to all items
        if !self.functions.is_empty() {
            for item in &mut result {
                for cond_func in self.functions {
                    if cond_func.conditions.iter().all(|c| c.test(ctx)) {
                        cond_func.function.apply(item, ctx);
                    }
                }
            }
            // Remove items with zero count after applying functions
            result.retain(|item| item.count > 0);
        }

        result
    }
}

impl LootPool {
    /// Add random items from this pool to the result.
    fn add_random_items<R: rand::Rng>(
        &self,
        ctx: &mut LootContext<'_, R>,
        result: &mut Vec<ItemStack>,
    ) {
        // Check pool conditions
        for condition in self.conditions {
            if !condition.test(ctx) {
                return;
            }
        }

        // Track where items from this pool start
        let start_index = result.len();

        // Calculate number of rolls
        let roll_count = self.rolls.get_int(ctx.rng) + (self.bonus_rolls * ctx.luck).floor() as i32;

        for _ in 0..roll_count {
            self.add_random_item(ctx, result);
        }

        // Apply pool-level functions to all items generated by this pool
        if !self.functions.is_empty() {
            for item in result.iter_mut().skip(start_index) {
                for cond_func in self.functions {
                    if cond_func.conditions.iter().all(|c| c.test(ctx)) {
                        cond_func.function.apply(item, ctx);
                    }
                }
            }
            // Remove items with zero count after applying functions
            result.retain(|item| item.count > 0);
        }
    }

    /// Select and add a single random item from this pool.
    fn add_random_item<R: rand::Rng>(
        &self,
        ctx: &mut LootContext<'_, R>,
        result: &mut Vec<ItemStack>,
    ) {
        // Collect valid entries with their effective weights
        let mut valid_entries: Vec<(&LootEntry, i32)> = Vec::new();
        let mut total_weight = 0;

        for entry in self.entries {
            // Check entry conditions
            let passes_conditions = entry.conditions().iter().all(|c| c.test(ctx));

            if !passes_conditions {
                continue;
            }

            let weight = entry.effective_weight(ctx.luck);
            if weight > 0 {
                valid_entries.push((entry, weight));
                total_weight += weight;
            }
        }

        if total_weight == 0 || valid_entries.is_empty() {
            return;
        }

        // Weighted random selection
        let selected = if valid_entries.len() == 1 {
            valid_entries[0].0
        } else {
            let mut index = ctx.rng.random_range(0..total_weight);
            let mut selected_entry = valid_entries[0].0;
            for (entry, weight) in &valid_entries {
                index -= weight;
                if index < 0 {
                    selected_entry = entry;
                    break;
                }
            }
            selected_entry
        };

        // Generate item(s) from the selected entry
        selected.create_items(ctx, result);
    }
}

impl LootEntry {
    /// Create items from this entry and add them to the result.
    fn create_items<R: rand::Rng>(
        &self,
        ctx: &mut LootContext<'_, R>,
        result: &mut Vec<ItemStack>,
    ) {
        match self {
            LootEntry::Item {
                name, functions, ..
            } => {
                if let Some(item_ref) = REGISTRY.items.by_key(name) {
                    let mut item = ItemStack::new(item_ref);

                    // Apply functions
                    for cond_func in *functions {
                        if cond_func.conditions.iter().all(|c| c.test(ctx)) {
                            cond_func.function.apply(&mut item, ctx);
                        }
                    }

                    if item.count > 0 {
                        result.push(item);
                    }
                }
            }
            LootEntry::LootTableRef {
                name, functions, ..
            } => {
                // Recursively get items from referenced loot table
                if let Some(table) = REGISTRY.loot_tables.by_key(name) {
                    let mut items = table.get_random_items(ctx);
                    // Apply functions to all items from the referenced table
                    for item in &mut items {
                        for cond_func in *functions {
                            if cond_func.conditions.iter().all(|c| c.test(ctx)) {
                                cond_func.function.apply(item, ctx);
                            }
                        }
                    }
                    result.extend(items.into_iter().filter(|i| i.count > 0));
                }
            }
            LootEntry::InlineLootTable {
                pools, functions, ..
            } => {
                // Process inline loot table pools directly
                let mut items = Vec::new();
                for pool in *pools {
                    pool.add_random_items(ctx, &mut items);
                }
                // Apply functions to all items from the inline table
                for item in &mut items {
                    for cond_func in *functions {
                        if cond_func.conditions.iter().all(|c| c.test(ctx)) {
                            cond_func.function.apply(item, ctx);
                        }
                    }
                }
                result.extend(items.into_iter().filter(|i| i.count > 0));
            }
            LootEntry::Tag {
                name,
                expand,
                functions,
                ..
            } => {
                // Get all items in the tag
                if let Some(items) = REGISTRY.items.get_tag(name) {
                    if *expand {
                        // Pick one random item from the tag (weighted equally)
                        if !items.is_empty() {
                            let index = ctx.rng.random_range(0..items.len());
                            let mut item = ItemStack::new(items[index]);
                            for cond_func in *functions {
                                if cond_func.conditions.iter().all(|c| c.test(ctx)) {
                                    cond_func.function.apply(&mut item, ctx);
                                }
                            }
                            if item.count > 0 {
                                result.push(item);
                            }
                        }
                    } else {
                        // Drop all items from the tag
                        for item_ref in items {
                            let mut item = ItemStack::new(item_ref);
                            for cond_func in *functions {
                                if cond_func.conditions.iter().all(|c| c.test(ctx)) {
                                    cond_func.function.apply(&mut item, ctx);
                                }
                            }
                            if item.count > 0 {
                                result.push(item);
                            }
                        }
                    }
                }
            }
            LootEntry::Alternatives { children, .. } => {
                // Try children in order, use first that passes conditions and produces items
                for child in *children {
                    // Check child's conditions first
                    let passes_conditions = child.conditions().iter().all(|c| c.test(ctx));
                    if !passes_conditions {
                        continue; // Try next alternative
                    }

                    let before_len = result.len();
                    child.create_items(ctx, result);
                    if result.len() > before_len {
                        break; // First successful child that produced items, stop
                    }
                }
            }
            LootEntry::Group { children, .. } => {
                // Use all children that pass their conditions
                for child in *children {
                    let passes_conditions = child.conditions().iter().all(|c| c.test(ctx));
                    if passes_conditions {
                        child.create_items(ctx, result);
                    }
                }
            }
            LootEntry::Sequence { children, .. } => {
                // Use children in sequence until one fails its conditions
                // Note: Unlike Alternatives, Sequence stops when conditions FAIL,
                // not when items are produced. A child can produce nothing but still "succeed".
                for child in *children {
                    let passes_conditions = child.conditions().iter().all(|c| c.test(ctx));
                    if !passes_conditions {
                        break; // Condition failed, stop sequence
                    }
                    child.create_items(ctx, result);
                }
            }
            LootEntry::Empty { .. } => {
                // Empty entry produces nothing
            }
            LootEntry::Dynamic { name, .. } => {
                // Dynamic entries are used for block entity contents (like shulker boxes)
                // The name identifies what content to retrieve:
                // - "contents" = block entity inventory contents
                // - Other names may exist for specific use cases
                //
                // TODO: Implement when block entity system supports inventory retrieval
                // This requires:
                // 1. Block entity reference in LootContext
                // 2. Method to get inventory contents from block entity
                // 3. Adding those items to the result
                let _ = name;
            }
            LootEntry::Slots {
                slots, functions, ..
            } => {
                // Slots entries select items from specific block entity slots
                // TODO: Implement when block entity system supports slot access
                // This requires:
                // 1. Block entity reference in LootContext
                // 2. Method to get items from specific slots
                // 3. Apply functions to each retrieved item
                let _ = slots;
                let _ = functions;
            }
        }
    }
}

impl LootFunction {
    /// Apply this function to modify the item stack.
    ///
    /// This modifies the item in place. Functions can change:
    /// - Count (SetCount, ExplosionDecay, ApplyBonus, etc.)
    /// - Damage/durability (SetDamage)
    /// - Enchantments (EnchantRandomly, EnchantWithLevels, SetEnchantments)
    /// - Components/NBT (CopyComponents, SetComponents, CopyState)
    /// - Item type (FurnaceSmelt)
    /// - And more...
    pub fn apply<R: rand::Rng>(&self, item: &mut ItemStack, ctx: &mut LootContext<'_, R>) {
        match self {
            LootFunction::SetCount {
                count: provider,
                add,
            } => {
                let value = provider.get_int(ctx.rng);
                if *add {
                    item.count += value;
                } else {
                    item.count = value;
                }
            }
            LootFunction::ExplosionDecay => {
                if let Some(radius) = ctx.explosion_radius {
                    // Each item has 1/radius chance to survive
                    let probability = 1.0 / radius;
                    let mut result_count = 0;
                    for _ in 0..item.count {
                        if ctx.rng.random::<f32>() <= probability {
                            result_count += 1;
                        }
                    }
                    item.count = result_count;
                }
            }
            LootFunction::ApplyBonus {
                enchantment,
                formula,
            } => {
                let level = ctx.get_enchantment_level_by_id(enchantment);
                item.count = formula.apply(item.count, level, ctx.rng);
            }
            LootFunction::EnchantedCountIncrease {
                enchantment,
                count: provider,
                limit,
            } => {
                let level = ctx.get_enchantment_level_by_id(enchantment);
                if level > 0 {
                    let bonus = (provider.get_simple(ctx.rng) * level as f32).round() as i32;
                    let bonus = if *limit > 0 { bonus.min(*limit) } else { bonus };
                    item.count += bonus;
                }
            }
            LootFunction::LimitCount { min, max } => {
                if let Some(min_val) = min {
                    item.count = item.count.max(*min_val);
                }
                if let Some(max_val) = max {
                    item.count = item.count.min(*max_val);
                }
            }
            LootFunction::SetDamage { damage, add } => {
                item.set_damage_fraction(damage.get_simple(ctx.rng), *add);
            }
            LootFunction::EnchantRandomly { options } => {
                // TODO: Implement when enchantment system is ready
                item.enchant_randomly(options, ctx.rng);
            }
            LootFunction::EnchantWithLevels { levels, options } => {
                // TODO: Implement when enchantment system is ready
                let level = levels.get_int(ctx.rng);
                item.enchant_with_levels(level, options, ctx.rng);
            }
            LootFunction::CopyComponents { source, include } => {
                // TODO: Implement when block entity system is ready
                item.copy_components(*source, include, ctx);
            }
            LootFunction::CopyState { block, properties } => {
                // TODO: Implement block state copying
                item.copy_block_state(block, properties, ctx);
            }
            LootFunction::SetComponents { components } => {
                // TODO: Implement component setting from JSON
                item.set_components_from_json(components);
            }
            LootFunction::SetCustomData { tag } => {
                item.set_custom_data(tag);
            }
            LootFunction::FurnaceSmelt { use_input_count } => {
                item.apply_furnace_smelt(*use_input_count);
            }
            LootFunction::ExplorationMap {
                destination,
                decoration,
                zoom,
                skip_existing_chunks,
            } => {
                // TODO: Implement exploration map creation
                item.create_exploration_map(destination, decoration, *zoom, *skip_existing_chunks);
            }
            LootFunction::SetName { name, target } => {
                // TODO: Implement name setting
                item.set_name(name, *target);
            }
            LootFunction::SetOminousBottleAmplifier { amplifier } => {
                let amp = amplifier.get_int(ctx.rng);
                item.set_ominous_bottle_amplifier(amp);
            }
            LootFunction::SetPotion { id } => {
                item.set_potion(id);
            }
            LootFunction::SetStewEffect { effects } => {
                item.set_stew_effects(effects, ctx.rng);
            }
            LootFunction::SetInstrument { options } => {
                item.set_instrument(options, ctx.rng);
            }
            LootFunction::SetEnchantments { enchantments, add } => {
                let resolved: Vec<_> = enchantments
                    .iter()
                    .map(|(key, provider)| (key.clone(), provider.get_int(ctx.rng).max(0) as u32))
                    .collect();
                item.set_enchantments(&resolved, *add);
            }
            LootFunction::SetItem { item: new_item } => {
                item.set_item(new_item);
            }
            LootFunction::CopyName { source } => {
                item.copy_name(*source, ctx);
            }
            LootFunction::SetLore { lore, mode } => {
                item.set_lore(lore, *mode);
            }
            LootFunction::SetContents {
                entries,
                component_type,
            } => {
                item.set_contents(entries, component_type, ctx);
            }
            LootFunction::ModifyContents {
                modifier,
                component_type,
            } => {
                item.modify_contents(modifier, component_type, ctx);
            }
            LootFunction::SetLootTable { loot_table, seed } => {
                item.set_loot_table(loot_table, *seed);
            }
            LootFunction::SetAttributes { modifiers, replace } => {
                item.set_attributes(modifiers, *replace, ctx.rng);
            }
            LootFunction::FillPlayerHead { entity } => {
                item.fill_player_head(*entity, ctx);
            }
            LootFunction::CopyCustomData { source, operations } => {
                item.copy_custom_data(*source, operations, ctx);
            }
            LootFunction::SetBannerPattern { patterns, append } => {
                item.set_banner_pattern(patterns, *append);
            }
            LootFunction::SetFireworks {
                explosions,
                flight_duration,
            } => {
                item.set_fireworks(*explosions, *flight_duration);
            }
            LootFunction::SetFireworkExplosion { explosion } => {
                item.set_firework_explosion(explosion);
            }
            LootFunction::SetBookCover {
                title,
                author,
                generation,
            } => {
                item.set_book_cover(*title, *author, *generation);
            }
            LootFunction::SetWrittenBookPages { pages, mode } => {
                item.set_written_book_pages(pages, *mode);
            }
            LootFunction::SetWritableBookPages { pages, mode } => {
                item.set_writable_book_pages(pages, *mode);
            }
            LootFunction::ToggleTooltips { toggles } => {
                item.toggle_tooltips(toggles);
            }
            LootFunction::SetCustomModelData { value } => {
                item.set_custom_model_data(value.get_int(ctx.rng));
            }
            LootFunction::Discard => {
                item.count = 0;
            }
            LootFunction::Reference(_name) => {
                // TODO: Implement function registry lookup
            }
            LootFunction::Sequence { functions } => {
                for cond_func in *functions {
                    if cond_func.conditions.iter().all(|c| c.test(ctx)) {
                        cond_func.function.apply(item, ctx);
                    }
                }
            }
            LootFunction::Filtered {
                item_filter,
                modifier,
            } => {
                if item_filter.test(item, ctx) && modifier.conditions.iter().all(|c| c.test(ctx)) {
                    modifier.function.apply(item, ctx);
                }
            }
        }
    }
}

impl BonusFormula {
    /// Apply the bonus formula to calculate new count.
    pub fn apply<R: rand::Rng>(&self, count: i32, level: i32, rng: &mut R) -> i32 {
        match self {
            BonusFormula::OreDrops => {
                if level > 0 {
                    // Vanilla: count * (max(0, random(0..level+2) - 1) + 1)
                    let bonus = rng.random_range(0..level + 2) - 1;
                    let multiplier = bonus.max(0) + 1;
                    count * multiplier
                } else {
                    count
                }
            }
            BonusFormula::UniformBonusCount { bonus_multiplier } => {
                // Vanilla: count + random(0..bonusMultiplier * level + 1)
                if level > 0 {
                    count + rng.random_range(0..bonus_multiplier * level + 1)
                } else {
                    count
                }
            }
            BonusFormula::BinomialWithBonusCount { extra, probability } => {
                // Vanilla: for each of (level + extra) trials, probability p to add 1
                let trials = level + extra;
                let mut bonus = 0;
                for _ in 0..trials {
                    if rng.random::<f32>() < *probability {
                        bonus += 1;
                    }
                }
                count + bonus
            }
        }
    }
}

pub type LootTableRef = &'static LootTable;

/// A named loot predicate in the predicate registry.
#[derive(Debug)]
pub struct LootPredicate {
    pub key: Identifier,
    pub condition: RuntimeLootCondition,
}

pub type LootPredicateRef = &'static LootPredicate;

/// Registry for loot predicates.
pub struct LootPredicateRegistry {
    predicates_by_id: Vec<LootPredicateRef>,
    predicates_by_key: FxHashMap<Identifier, usize>,
    allows_registering: bool,
}

impl LootPredicateRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            predicates_by_id: Vec::new(),
            predicates_by_key: FxHashMap::default(),
            allows_registering: true,
        }
    }

    pub fn register(&mut self, predicate: LootPredicateRef) -> usize {
        assert!(
            self.allows_registering,
            "Cannot register loot predicates after the registry has been frozen"
        );

        let id = self.predicates_by_id.len();
        self.predicates_by_key.insert(predicate.key.clone(), id);
        self.predicates_by_id.push(predicate);
        id
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, LootPredicateRef)> + '_ {
        self.predicates_by_id
            .iter()
            .enumerate()
            .map(|(id, &predicate)| (id, predicate))
    }
}

impl Default for LootPredicateRegistry {
    fn default() -> Self {
        Self::new()
    }
}

crate::impl_registry!(
    LootPredicateRegistry,
    LootPredicate,
    predicates_by_id,
    predicates_by_key,
    loot_predicates
);

/// Registry for loot tables.
pub struct LootTableRegistry {
    tables_by_id: Vec<LootTableRef>,
    tables_by_key: FxHashMap<Identifier, usize>,
    allows_registering: bool,
}

impl LootTableRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tables_by_id: Vec::new(),
            tables_by_key: FxHashMap::default(),
            allows_registering: true,
        }
    }

    pub fn register(&mut self, table: LootTableRef) -> usize {
        assert!(
            self.allows_registering,
            "Cannot register loot tables after the registry has been frozen"
        );

        let id = self.tables_by_id.len();
        self.tables_by_key.insert(table.key.clone(), id);
        self.tables_by_id.push(table);
        id
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, LootTableRef)> + '_ {
        self.tables_by_id
            .iter()
            .enumerate()
            .map(|(id, &table)| (id, table))
    }
}

impl Default for LootTableRegistry {
    fn default() -> Self {
        Self::new()
    }
}

crate::impl_registry!(
    LootTableRegistry,
    LootTable,
    tables_by_id,
    tables_by_key,
    loot_tables
);

#[cfg(test)]
mod tests {
    use crate::{test_support::init_test_registry, vanilla_loot_tables};

    use super::*;
    use rand::SeedableRng;
    use simdnbt::owned::NbtList;

    fn test_rng() -> rand::rngs::StdRng {
        rand::rngs::StdRng::seed_from_u64(12345)
    }

    fn init_test_registries() {
        init_test_registry();
    }

    fn condition_compound(condition: &str) -> NbtCompound {
        let mut compound = NbtCompound::new();
        compound.insert("condition", NbtTag::String(condition.to_owned().into()));
        compound
    }

    fn condition_tag(condition: &str) -> NbtTag {
        NbtTag::Compound(condition_compound(condition))
    }

    #[test]
    fn runtime_loot_condition_decodes_and_evaluates_supported_conditions() {
        init_test_registries();
        let condition = RuntimeLootCondition::decode(&condition_tag("minecraft:killed_by_player"))
            .expect("killed_by_player decodes");
        assert!(matches!(condition, RuntimeLootCondition::KilledByPlayer));

        let mut rng = test_rng();
        let mut ctx = LootContext::new(&mut rng).with_killed_by_player(true);
        assert_eq!(condition.try_test(&mut ctx), Ok(true));

        let mut rng = test_rng();
        let mut ctx = LootContext::new(&mut rng);
        assert_eq!(condition.try_test(&mut ctx), Ok(false));
    }

    #[test]
    fn runtime_loot_condition_decodes_inline_list_as_all_of() {
        init_test_registries();
        let tag = NbtTag::List(NbtList::Compound(vec![
            condition_compound("minecraft:killed_by_player"),
            condition_compound("minecraft:survives_explosion"),
        ]));

        let condition = RuntimeLootCondition::decode(&tag).expect("inline list decodes");
        let RuntimeLootCondition::AllOf(terms) = condition else {
            panic!("expected inline list to decode as all_of");
        };
        assert_eq!(terms.len(), 2);
        assert!(matches!(terms[0], RuntimeLootCondition::KilledByPlayer));
        assert!(matches!(terms[1], RuntimeLootCondition::SurvivesExplosion));
    }

    #[test]
    fn runtime_loot_condition_rejects_unknown_conditions() {
        init_test_registries();
        assert!(matches!(
            RuntimeLootCondition::decode(&condition_tag("minecraft:not_real")),
            Err(LootPredicateDecodeError::UnknownCondition(value))
                if value == "minecraft:not_real"
        ));
    }

    #[test]
    fn runtime_loot_condition_decodes_known_unsupported_conditions() {
        init_test_registries();
        let condition = RuntimeLootCondition::decode(&condition_tag("minecraft:entity_scores"))
            .expect("known unsupported condition decodes");
        assert!(matches!(
            condition,
            RuntimeLootCondition::Unsupported("entity_scores")
        ));

        let mut rng = test_rng();
        let mut ctx = LootContext::new(&mut rng);
        assert_eq!(
            condition.try_test(&mut ctx),
            Err(LootConditionEvaluationError::UnsupportedCondition(
                "entity_scores"
            ))
        );
    }

    #[test]
    fn runtime_uniform_number_provider_matches_vanilla_degenerate_range() {
        init_test_registries();
        let provider = RuntimeNumberProvider::Uniform {
            min: Box::new(RuntimeNumberProvider::Constant(5.0)),
            max: Box::new(RuntimeNumberProvider::Constant(2.0)),
        };
        let mut rng = test_rng();
        let mut ctx = LootContext::new(&mut rng);

        assert_eq!(provider.try_get(&mut ctx), Ok(5.0));
    }

    #[test]
    fn checked_condition_evaluation_reports_unsupported_conditions() {
        init_test_registries();
        let mut rng = test_rng();
        let mut ctx = LootContext::new(&mut rng);

        let condition = LootCondition::Reference(Identifier::vanilla_static("missing"));
        assert_eq!(
            condition.try_test(&mut ctx),
            Err(LootConditionEvaluationError::UnsupportedCondition(
                "reference"
            ))
        );

        let condition = LootCondition::EntityScores {
            entity: LootContextEntity::This,
            scores: &[],
        };
        assert_eq!(
            condition.try_test(&mut ctx),
            Err(LootConditionEvaluationError::UnsupportedCondition(
                "entity_scores"
            ))
        );

        let condition = LootCondition::LocationCheck {
            offset_x: 0,
            offset_y: 0,
            offset_z: 0,
            predicate: LocationPredicate { block: None },
        };
        assert_eq!(
            condition.try_test(&mut ctx),
            Err(LootConditionEvaluationError::UnsupportedCondition(
                "location_check"
            ))
        );
    }

    #[test]
    fn checked_condition_evaluation_reports_unsupported_number_providers() {
        init_test_registries();
        let mut rng = test_rng();
        let mut ctx = LootContext::new(&mut rng);

        let condition = LootCondition::ValueCheck {
            value: NumberProvider::Score {
                target: ScoreboardTarget::Fixed("Steve"),
                score: "kills",
                scale: 1.0,
            },
            range: NumberProviderRange::at_least(1.0),
        };

        assert_eq!(
            condition.try_test(&mut ctx),
            Err(LootConditionEvaluationError::UnsupportedNumberProvider(
                "score"
            ))
        );
    }

    #[test]
    fn checked_condition_evaluation_preserves_supported_condition_results() {
        init_test_registries();
        let mut rng = test_rng();
        let mut ctx = LootContext::new(&mut rng).with_killed_by_player(true);
        assert_eq!(LootCondition::KilledByPlayer.try_test(&mut ctx), Ok(true));

        let mut rng = test_rng();
        let mut ctx = LootContext::new(&mut rng);
        assert_eq!(LootCondition::KilledByPlayer.try_test(&mut ctx), Ok(false));

        let condition = LootCondition::WeatherCheck {
            raining: Some(false),
            thundering: Some(false),
        };
        assert_eq!(
            condition.try_test(&mut ctx),
            Err(LootConditionEvaluationError::MissingContext("weather"))
        );

        let mut rng = test_rng();
        let mut ctx = LootContext::new(&mut rng).with_weather(WeatherState::default());
        assert_eq!(condition.try_test(&mut ctx), Ok(true));

        let condition = LootCondition::TimeCheck {
            value: NumberProviderRange::between(10.0, 20.0),
            period: None,
        };
        assert_eq!(
            condition.try_test(&mut ctx),
            Err(LootConditionEvaluationError::MissingContext("game_time"))
        );

        let mut rng = test_rng();
        let mut ctx = LootContext::new(&mut rng).with_game_time(15);
        assert_eq!(condition.try_test(&mut ctx), Ok(true));
    }

    #[test]
    fn test_oak_log_loot() {
        init_test_registries();
        let mut rng = test_rng();

        let mut ctx = LootContext::new(&mut rng);
        let items = vanilla_loot_tables::BLOCKS_OAK_LOG.get_random_items(&mut ctx);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].count, 1);
        assert_eq!(items[0].item.key, Identifier::vanilla_static("oak_log"));
    }

    #[test]
    fn test_diamond_ore_loot_no_silk_touch() {
        // Without silk touch, diamond ore should drop diamond (not the ore block)
        init_test_registries();
        let mut rng = test_rng();

        let mut ctx = LootContext::new(&mut rng);
        let items = vanilla_loot_tables::BLOCKS_DIAMOND_ORE.get_random_items(&mut ctx);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].count, 1);
        // Without silk touch enchantment, diamond ore drops diamond
        assert_eq!(items[0].item.key, Identifier::vanilla_static("diamond"));
    }

    #[test]
    fn test_grass_block_loot_no_silk_touch() {
        // Without silk touch, grass block should drop dirt
        init_test_registries();
        let mut rng = test_rng();

        let mut ctx = LootContext::new(&mut rng);
        let items = vanilla_loot_tables::BLOCKS_GRASS_BLOCK.get_random_items(&mut ctx);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].count, 1);
        // Without silk touch, grass block drops dirt
        assert_eq!(items[0].item.key, Identifier::vanilla_static("dirt"));
    }

    #[test]
    fn test_stone_loot_no_silk_touch() {
        // Without silk touch, stone should drop cobblestone
        init_test_registries();
        let mut rng = test_rng();

        let mut ctx = LootContext::new(&mut rng);
        let items = vanilla_loot_tables::BLOCKS_STONE.get_random_items(&mut ctx);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].count, 1);
        // Without silk touch, stone drops cobblestone
        assert_eq!(items[0].item.key, Identifier::vanilla_static("cobblestone"));
    }

    #[test]
    fn test_pig_loot_drops_raw_porkchop_when_not_on_fire() {
        init_test_registries();
        let mut rng = test_rng();
        let pig_key = Identifier::vanilla_static("pig");
        let pig = EntityRef {
            entity_type: Some(&pig_key),
            flags: EntityRefFlags::default(),
            equipment: None,
            custom_name: None,
        };

        let mut ctx = LootContext::new(&mut rng).with_this_entity(pig);
        let items = vanilla_loot_tables::ENTITIES_PIG.get_random_items(&mut ctx);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].item.key, Identifier::vanilla_static("porkchop"));
        assert!((1..=3).contains(&items[0].count));
    }

    #[test]
    fn test_pig_loot_smelt_condition_uses_entity_fire_flag() {
        init_test_registries();
        let mut rng = test_rng();
        let pig_key = Identifier::vanilla_static("pig");
        let pig = EntityRef {
            entity_type: Some(&pig_key),
            flags: EntityRefFlags {
                is_on_fire: true,
                ..EntityRefFlags::default()
            },
            equipment: None,
            custom_name: None,
        };

        let mut ctx = LootContext::new(&mut rng).with_this_entity(pig);
        let items = vanilla_loot_tables::ENTITIES_PIG.get_random_items(&mut ctx);

        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].item.key,
            Identifier::vanilla_static("cooked_porkchop")
        );
        assert!((1..=3).contains(&items[0].count));
    }

    #[test]
    fn test_explosion_decay_function() {
        // Test the explosion_decay function directly
        init_test_registries();

        // explosion_decay reduces count based on 1/radius probability per item
        let cond_func = ConditionalLootFunction {
            function: LootFunction::ExplosionDecay,
            conditions: &[],
        };

        let mut total_survived = 0;
        let initial_count = 10;

        for seed in 0u64..100 {
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
            let mut ctx = LootContext::new(&mut rng).with_explosion(4.0);
            let mut item = ItemStack::with_count(&crate::vanilla_items::ITEMS.stone, initial_count);
            cond_func.function.apply(&mut item, &mut ctx);
            total_survived += item.count;
        }

        // With 10 items each trial, 100 trials = 1000 items total
        // Each has 25% (1/4.0) chance to survive = ~250 expected
        // Allow for variance: 150-350 range
        assert!(
            total_survived > 150 && total_survived < 350,
            "Expected ~250 items with explosion decay (25% of 1000), got {}",
            total_survived
        );
    }

    #[test]
    fn test_survives_explosion_condition() {
        init_test_registries();

        // Test that survives_explosion condition works
        // Gravel has survives_explosion on its alternatives
        let mut survived = 0;
        for seed in 0..100 {
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
            let mut ctx = LootContext::new(&mut rng).with_explosion(4.0);
            let items = vanilla_loot_tables::BLOCKS_GRAVEL.get_random_items(&mut ctx);
            if !items.is_empty() {
                survived += 1;
            }
        }

        // With radius 4.0, ~25% should survive
        assert!(
            survived > 10 && survived < 50,
            "Expected ~25% survival rate, got {}%",
            survived
        );
    }
}
