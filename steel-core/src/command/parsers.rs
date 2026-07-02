//! Graph-native command argument parsers.

mod block;
mod function;
mod game;
mod item_predicate;
mod item_stack;
mod loot_predicate;
mod nbt;
mod permission;
mod position;
mod resource;
mod scoreboard;
mod selector;
mod slot;
mod target;
mod text;
mod world;

pub use block::{BlockPredicateArgumentValue, BlockPredicateParser};
pub use function::{CommandFunctionArgumentValue, CommandFunctionParser};
pub use game::GameModeParser;
pub use item_predicate::{
    ItemPredicateArgumentValue, ItemPredicateCondition, ItemPredicateMatchError,
    ItemPredicateParser, ItemPredicateTarget, ItemPredicateTerm,
};
pub use item_stack::ItemStackParser;
pub use loot_predicate::{LootPredicateArgumentValue, LootPredicateParser};
pub use nbt::{NbtCompoundParser, NbtPathParser};
pub(crate) use permission::permission_rule_expression_suggestions;
pub use permission::{PermissionGroupParser, PermissionKeyParser, PermissionRuleExpressionParser};
pub use position::{BlockPosParser, HeightmapParser, RotationParser, Vec3Parser};
pub(crate) use resource::parse_resource_identifier;
pub use resource::{
    BiomeArgumentValue, BiomeParser, EnchantmentParser, EntitySummonParser, ItemParser,
    StructureArgumentValue, StructureParser,
};
pub use scoreboard::{
    DoubleRangeArgumentValue, DoubleRangeParser, IntRangeArgumentValue, IntRangeParser,
    ObjectiveParser, ScoreHolderArgumentValue, ScoreHolderParser, ScoreboardObjectiveName,
};
pub(crate) use scoreboard::{ScoreHolderWildcardExpansion, resolve_score_holders};
pub use slot::{ItemSlotRangeArgumentValue, ItemSlotsParser};
pub use target::{
    EntityParser, EntityTargetArgumentValue, PermissionTargetArgumentValue, PermissionTargetParser,
    PlayerParser, PlayerTargetArgumentValue,
};
pub(crate) use target::{
    resolve_optional_entity_targets, resolve_optional_permission_targets,
    resolve_optional_player_targets, resolve_required_entity_targets,
    resolve_required_permission_targets, resolve_required_player_targets,
};
pub use text::{ComponentParser, TimeParser};
pub use world::{DomainParser, WorldArgumentValue, WorldParser};

#[cfg(test)]
mod tests;
