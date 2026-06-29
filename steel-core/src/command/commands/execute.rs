//! Handler for the "execute" command.
//!
//! Bossbar store targets, predicates, functions, item predicates, and stopwatch
//! predicates are not registered here yet because their backing foundations are
//! not implemented in Steel's command/runtime layer.

use std::{borrow::Cow, sync::Arc};

use glam::DVec3;
use simdnbt::owned::{NbtCompound, NbtTag};
use steel_registry::blocks::block_state_ext::BlockStateExt;
use steel_registry::entity_type::EntityTypeRef;
use steel_utils::{
    BlockPos, Identifier,
    nbt::{NbtPath, compare_nbt},
    translations,
};
use text_components::TextComponent;
use text_components::translation::TranslatedMessage;

use crate::chunk::heightmap::HeightmapType;
use crate::command::CommandRegistrationSpec;
use crate::command::context::{CommandContext, EntityAnchor};
use crate::command::error::CommandError;
use crate::command::graph::{
    BiomeArgumentValue, BlockPredicateArgumentValue, CommandNodeBuilder, CommandRedirectTarget,
    CommandResult, IntRangeArgumentValue, ParsedArguments, ScoreHolderArgumentValue,
    ScoreboardObjectiveName, argument, literal,
};
use crate::command::parsers::{
    BiomeParser, BlockPosParser, BlockPredicateParser, EntityParser, IntRangeParser, NbtPathParser,
    ObjectiveParser, ScoreHolderParser, WorldParser,
};
use crate::entity::SharedEntity;
use crate::scoreboard::{ScoreHolder, Scoreboard, ScoreboardObjective};
use crate::world::World;

#[path = "execute/store.rs"]
mod store;
#[path = "execute/parsers.rs"]
mod execute_parsers;
#[path = "execute/source.rs"]
mod source;

use self::execute_parsers::{AxesParser, StorageKeyParser};

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Handler for the "execute" command.
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
    literal("execute")
        .then(literal("run").redirects(CommandRedirectTarget::All, |_, _| {
            Ok(CommandResult::success())
        }))
        .then(conditionals("if", true))
        .then(conditionals("unless", false))
        .then(source::as_operation())
        .then(source::at_operation())
        .then(
            literal("store")
                .then(store::target("result", true))
                .then(store::target("success", false)),
        )
        .then(source::positioned_operation())
        .then(source::rotated_operation())
        .then(source::facing_operation())
        .then(source::align_operation())
        .then(source::anchored_operation())
        .then(source::in_operation())
        .then(source::summon_operation())
        .then(source::on_relations())
}

fn conditionals(name: &'static str, expected: bool) -> CommandNodeBuilder {
    literal(name)
        .then(literal("block").then(
            argument("pos", BlockPosParser).then(
                argument("block", BlockPredicateParser)
                    .executes(move |context, arguments| {
                        execute_block_condition(context, arguments, expected)
                    })
                    .forks(CommandRedirectTarget::Current, move |context, arguments| {
                        fork_block_condition(context, arguments, expected)
                    }),
            ),
        ))
        .then(literal("entity").then(
            argument("entities", EntityParser::multiple())
                .executes(move |context, arguments| {
                    execute_entity_condition(context, arguments, expected)
                })
                .forks(CommandRedirectTarget::Current, move |context, arguments| {
                    fork_entity_condition(context, arguments, expected)
                }),
        ))
        .then(literal("dimension").then(
            argument("dimension", WorldParser)
                .executes(move |context, arguments| {
                    execute_dimension_condition(context, arguments, expected)
                })
                .forks(CommandRedirectTarget::Current, move |context, arguments| {
                    fork_dimension_condition(context, arguments, expected)
                }),
        ))
        .then(literal("biome").then(
            argument("pos", BlockPosParser).then(
                argument("biome", BiomeParser)
                    .executes(move |context, arguments| {
                        execute_biome_condition(context, arguments, expected)
                    })
                    .forks(CommandRedirectTarget::Current, move |context, arguments| {
                        fork_biome_condition(context, arguments, expected)
                    }),
            ),
        ))
        .then(literal("loaded").then(
            argument("pos", BlockPosParser)
                .executes(move |context, arguments| {
                    execute_loaded_condition(context, arguments, expected)
                })
                .forks(CommandRedirectTarget::Current, move |context, arguments| {
                    fork_loaded_condition(context, arguments, expected)
                }),
        ))
        .then(literal("score").then(
            argument("target", ScoreHolderParser::one()).then(
                argument("targetObjective", ObjectiveParser)
                    .then(score_comparison("=", ScoreComparison::Equal, expected))
                    .then(score_comparison("<", ScoreComparison::Less, expected))
                    .then(score_comparison("<=", ScoreComparison::LessOrEqual, expected))
                    .then(score_comparison(">", ScoreComparison::Greater, expected))
                    .then(score_comparison(">=", ScoreComparison::GreaterOrEqual, expected))
                    .then(literal("matches").then(
                        argument("range", IntRangeParser)
                            .executes(move |context, arguments| {
                                execute_score_range_condition(context, arguments, expected)
                            })
                            .forks(CommandRedirectTarget::Current, move |context, arguments| {
                                fork_score_range_condition(context, arguments, expected)
                            }),
                    )),
            ),
        ))
        .then(
            literal("data")
                .then(literal("block").then(
                    argument("pos", BlockPosParser).then(
                        argument("path", NbtPathParser)
                            .executes(move |context, arguments| {
                                execute_block_data_condition(context, arguments, expected)
                            })
                            .forks(CommandRedirectTarget::Current, move |context, arguments| {
                                fork_block_data_condition(context, arguments, expected)
                            }),
                    ),
                ))
                .then(literal("entity").then(
                    argument("source", EntityParser::one()).then(
                        argument("path", NbtPathParser)
                            .executes(move |context, arguments| {
                                execute_entity_data_condition(context, arguments, expected)
                            })
                            .forks(CommandRedirectTarget::Current, move |context, arguments| {
                                fork_entity_data_condition(context, arguments, expected)
                            }),
                    ),
                ))
                .then(literal("storage").then(
                    argument("source", StorageKeyParser).then(
                        argument("path", NbtPathParser)
                            .executes(move |context, arguments| {
                                execute_storage_data_condition(context, arguments, expected)
                            })
                            .forks(CommandRedirectTarget::Current, move |context, arguments| {
                                fork_storage_data_condition(context, arguments, expected)
                            }),
                    ),
                )),
        )
        .then(literal("blocks").then(
            argument("start", BlockPosParser).then(
                argument("end", BlockPosParser).then(
                    argument("destination", BlockPosParser)
                        .then(blocks_conditional("all", expected, false))
                        .then(blocks_conditional("masked", expected, true)),
                ),
            ),
        ))
}

fn blocks_conditional(name: &'static str, expected: bool, skip_air: bool) -> CommandNodeBuilder {
    literal(name)
        .executes(move |context, arguments| {
            execute_blocks_condition(context, arguments, expected, skip_air)
        })
        .forks(CommandRedirectTarget::Current, move |context, arguments| {
            fork_blocks_condition(context, arguments, expected, skip_air)
        })
}

fn score_comparison(
    name: &'static str,
    comparison: ScoreComparison,
    expected: bool,
) -> CommandNodeBuilder {
    literal(name).permission_path_passthrough().then(
        argument("source", ScoreHolderParser::one()).then(
            argument("sourceObjective", ObjectiveParser)
                .executes(move |context, arguments| {
                    execute_score_comparison_condition(context, arguments, expected, comparison)
                })
                .forks(CommandRedirectTarget::Current, move |context, arguments| {
                    fork_score_comparison_condition(context, arguments, expected, comparison)
                }),
        ),
    )
}

fn execute_entity_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<CommandResult, CommandError> {
    let count = entities(arguments)?.len();
    if expected {
        if count == 0 {
            return Err(conditional_failed(count));
        }
        send_condition_pass_count(context, count);
        return Ok(CommandResult {
            success_count: success_count(count),
        });
    }

    if count == 0 {
        send_condition_pass(context);
        Ok(CommandResult::success())
    } else {
        Err(conditional_failed(count))
    }
}

fn fork_entity_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<Vec<CommandContext>, CommandError> {
    let matches = !entities(arguments)?.is_empty();
    Ok(if matches == expected {
        vec![context.clone()]
    } else {
        Vec::new()
    })
}

#[derive(Clone, Copy, Debug)]
enum ScoreComparison {
    Equal,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

impl ScoreComparison {
    fn test(self, left: i32, right: i32) -> bool {
        match self {
            Self::Equal => left == right,
            Self::Less => left < right,
            Self::LessOrEqual => left <= right,
            Self::Greater => left > right,
            Self::GreaterOrEqual => left >= right,
        }
    }
}

fn execute_score_comparison_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
    comparison: ScoreComparison,
) -> Result<CommandResult, CommandError> {
    let matches = score_comparison_matches(context, arguments, comparison)?;
    execute_simple_condition(context, matches, expected)
}

fn fork_score_comparison_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
    comparison: ScoreComparison,
) -> Result<Vec<CommandContext>, CommandError> {
    let matches = score_comparison_matches(context, arguments, comparison)?;
    Ok(if matches == expected {
        vec![context.clone()]
    } else {
        Vec::new()
    })
}

fn score_comparison_matches(
    context: &CommandContext,
    arguments: &ParsedArguments,
    comparison: ScoreComparison,
) -> Result<bool, CommandError> {
    let target = single_score_holder(arguments, "target")?;
    let target_objective = scoreboard_objective(context, arguments, "targetObjective")?;
    let source = single_score_holder(arguments, "source")?;
    let source_objective = scoreboard_objective(context, arguments, "sourceObjective")?;

    Ok(compare_scores(
        &context.server.scoreboard,
        &target,
        &target_objective,
        &source,
        &source_objective,
        comparison,
    ))
}

fn compare_scores(
    scoreboard: &Scoreboard,
    target: &ScoreHolder,
    target_objective: &ScoreboardObjective,
    source: &ScoreHolder,
    source_objective: &ScoreboardObjective,
    comparison: ScoreComparison,
) -> bool {
    let Some(left) = scoreboard.score(target, target_objective) else {
        return false;
    };
    let Some(right) = scoreboard.score(source, source_objective) else {
        return false;
    };
    comparison.test(left, right)
}

fn execute_score_range_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<CommandResult, CommandError> {
    let matches = score_range_matches(context, arguments)?;
    execute_simple_condition(context, matches, expected)
}

fn fork_score_range_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<Vec<CommandContext>, CommandError> {
    let matches = score_range_matches(context, arguments)?;
    Ok(if matches == expected {
        vec![context.clone()]
    } else {
        Vec::new()
    })
}

fn score_range_matches(
    context: &CommandContext,
    arguments: &ParsedArguments,
) -> Result<bool, CommandError> {
    let target = single_score_holder(arguments, "target")?;
    let objective = scoreboard_objective(context, arguments, "targetObjective")?;
    let range = int_range(arguments)?;

    Ok(score_matches_range(
        &context.server.scoreboard,
        &target,
        &objective,
        range,
    ))
}

fn score_matches_range(
    scoreboard: &Scoreboard,
    target: &ScoreHolder,
    objective: &ScoreboardObjective,
    range: IntRangeArgumentValue,
) -> bool {
    scoreboard
        .score(target, objective)
        .is_some_and(|score| range.matches(score))
}

fn execute_dimension_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<CommandResult, CommandError> {
    let matches = same_world(&context.world, &world(arguments)?);
    if matches == expected {
        send_condition_pass(context);
        Ok(CommandResult::success())
    } else {
        Err(conditional_failed(0))
    }
}

fn fork_dimension_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<Vec<CommandContext>, CommandError> {
    let matches = same_world(&context.world, &world(arguments)?);
    Ok(if matches == expected {
        vec![context.clone()]
    } else {
        Vec::new()
    })
}

fn execute_loaded_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<CommandResult, CommandError> {
    let matches = context
        .world
        .is_entity_ticking_chunk_loaded(block_position(arguments)?);
    if matches == expected {
        send_condition_pass(context);
        Ok(CommandResult::success())
    } else {
        Err(conditional_failed(0))
    }
}

fn fork_loaded_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<Vec<CommandContext>, CommandError> {
    let matches = context
        .world
        .is_entity_ticking_chunk_loaded(block_position(arguments)?);
    Ok(if matches == expected {
        vec![context.clone()]
    } else {
        Vec::new()
    })
}

fn execute_biome_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<CommandResult, CommandError> {
    let matches = biome_condition_matches(context, arguments)?;
    if matches == expected {
        send_condition_pass(context);
        Ok(CommandResult::success())
    } else {
        Err(conditional_failed(0))
    }
}

fn fork_biome_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<Vec<CommandContext>, CommandError> {
    let matches = biome_condition_matches(context, arguments)?;
    Ok(if matches == expected {
        vec![context.clone()]
    } else {
        Vec::new()
    })
}

fn biome_condition_matches(
    context: &CommandContext,
    arguments: &ParsedArguments,
) -> Result<bool, CommandError> {
    let pos = loaded_block_position(context, arguments)?;
    let Some(biome) = context.world.biome_at(pos) else {
        return Err(position_error("argument.pos.unloaded"));
    };

    Ok(biome_value(arguments)?.matches_biome(biome))
}

fn execute_block_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<CommandResult, CommandError> {
    let matches = block_condition_matches(context, arguments)?;
    if matches == expected {
        send_condition_pass(context);
        Ok(CommandResult::success())
    } else {
        Err(conditional_failed(0))
    }
}

fn fork_block_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<Vec<CommandContext>, CommandError> {
    let matches = block_condition_matches(context, arguments)?;
    Ok(if matches == expected {
        vec![context.clone()]
    } else {
        Vec::new()
    })
}

fn block_condition_matches(
    context: &CommandContext,
    arguments: &ParsedArguments,
) -> Result<bool, CommandError> {
    let pos = loaded_block_position(context, arguments)?;
    let predicate = block_predicate(arguments)?;
    let state = context.world.get_block_state(pos);
    if !predicate.matches_state(state) {
        return Ok(false);
    }
    let Some(expected_nbt) = predicate.nbt() else {
        return Ok(true);
    };

    let Some(block_entity) = context.world.get_block_entity(pos) else {
        return Ok(false);
    };
    let block_entity = block_entity.lock();
    let actual = block_entity_full_nbt(&*block_entity);

    let expected = NbtTag::Compound(expected_nbt.clone());
    let actual = NbtTag::Compound(actual);
    Ok(compare_nbt(Some(&expected), Some(&actual), true))
}

fn execute_block_data_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<CommandResult, CommandError> {
    let count = block_data_match_count(context, arguments)?;
    if expected {
        if count == 0 {
            return Err(conditional_failed(count));
        }
        send_condition_pass_count(context, count);
        return Ok(CommandResult {
            success_count: success_count(count),
        });
    }

    if count == 0 {
        send_condition_pass(context);
        Ok(CommandResult::success())
    } else {
        Err(conditional_failed(count))
    }
}

fn fork_block_data_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<Vec<CommandContext>, CommandError> {
    let matches = block_data_match_count(context, arguments)? > 0;
    Ok(if matches == expected {
        vec![context.clone()]
    } else {
        Vec::new()
    })
}

fn block_data_match_count(
    context: &CommandContext,
    arguments: &ParsedArguments,
) -> Result<usize, CommandError> {
    let pos = loaded_block_position(context, arguments)?;
    let Some(block_entity) = context.world.get_block_entity(pos) else {
        return Err(block_data_invalid_error());
    };

    let block_entity = block_entity.lock();
    let tag = NbtTag::Compound(block_entity_full_nbt(&*block_entity));
    Ok(nbt_path(arguments)?.count_matching(&tag))
}

fn execute_entity_data_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<CommandResult, CommandError> {
    let count = entity_data_match_count(arguments)?;
    if expected {
        if count == 0 {
            return Err(conditional_failed(count));
        }
        send_condition_pass_count(context, count);
        return Ok(CommandResult {
            success_count: success_count(count),
        });
    }

    if count == 0 {
        send_condition_pass(context);
        Ok(CommandResult::success())
    } else {
        Err(conditional_failed(count))
    }
}

fn execute_storage_data_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<CommandResult, CommandError> {
    let count = storage_data_match_count(context, arguments)?;
    if expected {
        if count == 0 {
            return Err(conditional_failed(count));
        }
        send_condition_pass_count(context, count);
        return Ok(CommandResult {
            success_count: success_count(count),
        });
    }

    if count == 0 {
        send_condition_pass(context);
        Ok(CommandResult::success())
    } else {
        Err(conditional_failed(count))
    }
}

fn fork_storage_data_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<Vec<CommandContext>, CommandError> {
    let matches = storage_data_match_count(context, arguments)? > 0;
    Ok(if matches == expected {
        vec![context.clone()]
    } else {
        Vec::new()
    })
}

fn storage_data_match_count(
    context: &CommandContext,
    arguments: &ParsedArguments,
) -> Result<usize, CommandError> {
    let source = storage_id(arguments, "source")?;
    let tag = NbtTag::Compound(context.server.command_storage.get(&source));
    Ok(nbt_path(arguments)?.count_matching(&tag))
}

fn fork_entity_data_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<Vec<CommandContext>, CommandError> {
    let matches = entity_data_match_count(arguments)? > 0;
    Ok(if matches == expected {
        vec![context.clone()]
    } else {
        Vec::new()
    })
}

fn entity_data_match_count(arguments: &ParsedArguments) -> Result<usize, CommandError> {
    let entity = source_entity(arguments)?;
    let tag = NbtTag::Compound(entity.nbt_for_data_compare());
    Ok(nbt_path(arguments)?.count_matching(&tag))
}

fn block_entity_full_nbt(block_entity: &dyn crate::block_entity::BlockEntity) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    let entity_pos = block_entity.get_block_pos();
    nbt.insert("id", block_entity.get_type().key.to_string());
    nbt.insert("x", entity_pos.x());
    nbt.insert("y", entity_pos.y());
    nbt.insert("z", entity_pos.z());
    block_entity.save_additional(&mut nbt);
    nbt
}

fn execute_blocks_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
    skip_air: bool,
) -> Result<CommandResult, CommandError> {
    let count = matching_block_region_count(context, arguments, skip_air)?;
    if expected {
        if let Some(count) = count {
            send_condition_pass_count(context, count);
            return Ok(CommandResult {
                success_count: success_count(count),
            });
        }
        return Err(conditional_failed(0));
    }

    if let Some(count) = count {
        Err(conditional_failed(count))
    } else {
        send_condition_pass(context);
        Ok(CommandResult::success())
    }
}

fn fork_blocks_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
    skip_air: bool,
) -> Result<Vec<CommandContext>, CommandError> {
    let matches = matching_block_region_count(context, arguments, skip_air)?.is_some();
    Ok(if matches == expected {
        vec![context.clone()]
    } else {
        Vec::new()
    })
}

fn matching_block_region_count(
    context: &CommandContext,
    arguments: &ParsedArguments,
    skip_air: bool,
) -> Result<Option<usize>, CommandError> {
    const MAX_BLOCKS_REGION: i64 = 32_768;

    let start = loaded_named_block_position(context, arguments, "start")?;
    let end = loaded_named_block_position(context, arguments, "end")?;
    let destination = loaded_named_block_position(context, arguments, "destination")?;
    let source_region = BlockRegion::from_corners(start, end);
    let area = source_region.area();
    if area > MAX_BLOCKS_REGION {
        return Err(blocks_too_big_error(area));
    }

    let offset_x = destination.x() - source_region.min.x();
    let offset_y = destination.y() - source_region.min.y();
    let offset_z = destination.z() - source_region.min.z();
    let mut count = 0;
    for z in source_region.min.z()..=source_region.max.z() {
        for y in source_region.min.y()..=source_region.max.y() {
            for x in source_region.min.x()..=source_region.max.x() {
                let source_pos = BlockPos::new(x, y, z);
                let source_state = context.world.get_block_state(source_pos);
                if skip_air && source_state.is_air() {
                    continue;
                }

                let destination_pos = source_pos.offset(offset_x, offset_y, offset_z);
                if source_state != context.world.get_block_state(destination_pos) {
                    return Ok(None);
                }
                if !block_entities_match(&context.world, source_pos, destination_pos) {
                    return Ok(None);
                }

                count += 1;
            }
        }
    }

    Ok(Some(count))
}

fn block_entities_match(world: &Arc<World>, source_pos: BlockPos, destination_pos: BlockPos) -> bool {
    let Some(source_entity) = world.get_block_entity(source_pos) else {
        return true;
    };
    let Some(destination_entity) = world.get_block_entity(destination_pos) else {
        return false;
    };
    if Arc::ptr_eq(&source_entity, &destination_entity) {
        return true;
    }

    let source_entity = source_entity.lock();
    let destination_entity = destination_entity.lock();
    if source_entity.get_type() != destination_entity.get_type() {
        return false;
    }

    let mut source_nbt = NbtCompound::new();
    source_entity.save_additional(&mut source_nbt);
    let mut destination_nbt = NbtCompound::new();
    destination_entity.save_additional(&mut destination_nbt);
    source_nbt == destination_nbt
}

fn loaded_block_position(
    context: &CommandContext,
    arguments: &ParsedArguments,
) -> Result<BlockPos, CommandError> {
    loaded_named_block_position(context, arguments, "pos")
}

fn loaded_named_block_position(
    context: &CommandContext,
    arguments: &ParsedArguments,
    name: &'static str,
) -> Result<BlockPos, CommandError> {
    let pos = named_block_position(arguments, name)?;
    if !context.world.is_full_chunk_loaded_at(pos) {
        return Err(position_error("argument.pos.unloaded"));
    }
    if !context.world.is_in_valid_bounds(pos) {
        return Err(position_error("argument.pos.outofworld"));
    }
    Ok(pos)
}

fn position_error(key: &'static str) -> CommandError {
    CommandError::failure(TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed(key),
        fallback: None,
        args: None,
    }))
}

fn block_data_invalid_error() -> CommandError {
    CommandError::failure(TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed("commands.data.block.invalid"),
        fallback: None,
        args: None,
    }))
}

fn blocks_too_big_error(area: i64) -> CommandError {
    CommandError::failure(TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed("commands.execute.blocks.toobig"),
        fallback: None,
        args: Some(Box::new([
            TextComponent::from("32768"),
            TextComponent::from(area.to_string()),
        ])),
    }))
}

fn entities(arguments: &ParsedArguments) -> Result<Vec<SharedEntity>, CommandError> {
    arguments
        .get::<Vec<SharedEntity>>("targets")
        .or_else(|_| arguments.get::<Vec<SharedEntity>>("entities"))
        .map_err(super::invalid_parsed_argument)
}

fn anchor(arguments: &ParsedArguments) -> Result<EntityAnchor, CommandError> {
    arguments
        .get::<EntityAnchor>("anchor")
        .map_err(super::invalid_parsed_argument)
}

fn position(arguments: &ParsedArguments) -> Result<DVec3, CommandError> {
    arguments
        .get::<DVec3>("pos")
        .map_err(super::invalid_parsed_argument)
}

fn block_position(arguments: &ParsedArguments) -> Result<BlockPos, CommandError> {
    named_block_position(arguments, "pos")
}

fn named_block_position(
    arguments: &ParsedArguments,
    name: &'static str,
) -> Result<BlockPos, CommandError> {
    arguments
        .get::<BlockPos>(name)
        .map_err(super::invalid_parsed_argument)
}

fn biome_value(arguments: &ParsedArguments) -> Result<BiomeArgumentValue, CommandError> {
    arguments
        .get::<BiomeArgumentValue>("biome")
        .map_err(super::invalid_parsed_argument)
}

fn block_predicate(arguments: &ParsedArguments) -> Result<BlockPredicateArgumentValue, CommandError> {
    arguments
        .get::<BlockPredicateArgumentValue>("block")
        .map_err(super::invalid_parsed_argument)
}

fn nbt_path(arguments: &ParsedArguments) -> Result<NbtPath, CommandError> {
    arguments
        .get::<NbtPath>("path")
        .map_err(super::invalid_parsed_argument)
}

fn double(arguments: &ParsedArguments, name: &str) -> Result<f64, CommandError> {
    arguments
        .get::<f64>(name)
        .map_err(super::invalid_parsed_argument)
}

fn scoreboard_objective(
    context: &CommandContext,
    arguments: &ParsedArguments,
    name: &str,
) -> Result<ScoreboardObjective, CommandError> {
    let objective_name = arguments
        .get::<ScoreboardObjectiveName>(name)
        .map_err(super::invalid_parsed_argument)?;
    context
        .server
        .scoreboard
        .objective(objective_name.as_str())
        .ok_or_else(|| {
            CommandError::failure(
                translations::ARGUMENTS_OBJECTIVE_NOT_FOUND
                    .message([TextComponent::from(objective_name.as_str().to_owned())]),
            )
        })
}

fn score_holders(
    arguments: &ParsedArguments,
    name: &str,
) -> Result<ScoreHolderArgumentValue, CommandError> {
    arguments
        .get::<ScoreHolderArgumentValue>(name)
        .map_err(super::invalid_parsed_argument)
}

fn single_score_holder(
    arguments: &ParsedArguments,
    name: &str,
) -> Result<ScoreHolder, CommandError> {
    let holders = score_holders(arguments, name)?;
    let Some(holders) = holders.holders() else {
        return Err(no_score_holders());
    };
    let [holder] = holders else {
        return Err(no_score_holders());
    };
    Ok(holder.to_owned())
}

fn score_holders_or_tracked(
    context: &CommandContext,
    arguments: &ParsedArguments,
    name: &str,
) -> Result<Vec<ScoreHolder>, CommandError> {
    match score_holders(arguments, name)? {
        ScoreHolderArgumentValue::Holders(holders) if holders.is_empty() => {
            Err(no_score_holders())
        }
        ScoreHolderArgumentValue::Holders(holders) => Ok(holders),
        ScoreHolderArgumentValue::Wildcard => {
            let holders = context.server.scoreboard.tracked_holders();
            if holders.is_empty() {
                Err(no_score_holders())
            } else {
                Ok(holders)
            }
        }
    }
}

fn no_score_holders() -> CommandError {
    CommandError::failure(TextComponent::from(
        &translations::ARGUMENT_ENTITY_NOTFOUND_ENTITY,
    ))
}

fn int_range(arguments: &ParsedArguments) -> Result<IntRangeArgumentValue, CommandError> {
    arguments
        .get::<IntRangeArgumentValue>("range")
        .map_err(super::invalid_parsed_argument)
}

fn source_entity(arguments: &ParsedArguments) -> Result<SharedEntity, CommandError> {
    single_entity(arguments, "source")
}

fn single_entity(arguments: &ParsedArguments, name: &'static str) -> Result<SharedEntity, CommandError> {
    let mut entities = arguments
        .get::<Vec<SharedEntity>>(name)
        .map_err(super::invalid_parsed_argument)?;
    if entities.len() != 1 {
        return Err(super::invalid_parsed_argument(
            crate::command::graph::ParsedArgumentError::WrongType {
                name: name.to_owned(),
                expected: "single_entity",
                actual: "entities",
            },
        ));
    }
    Ok(entities.remove(0))
}

fn heightmap(arguments: &ParsedArguments) -> Result<HeightmapType, CommandError> {
    arguments
        .get::<HeightmapType>("heightmap")
        .map_err(super::invalid_parsed_argument)
}

fn rotation(arguments: &ParsedArguments) -> Result<(f32, f32), CommandError> {
    arguments
        .get::<(f32, f32)>("rot")
        .map_err(super::invalid_parsed_argument)
}

fn world(arguments: &ParsedArguments) -> Result<Arc<World>, CommandError> {
    arguments
        .get::<Arc<World>>("dimension")
        .map_err(super::invalid_parsed_argument)
}

fn entity_type(arguments: &ParsedArguments) -> Result<EntityTypeRef, CommandError> {
    arguments
        .get::<EntityTypeRef>("entity")
        .map_err(super::invalid_parsed_argument)
}

fn storage_id(arguments: &ParsedArguments, name: &str) -> Result<Identifier, CommandError> {
    arguments
        .get::<Identifier>(name)
        .map_err(super::invalid_parsed_argument)
}

fn same_world(left: &Arc<World>, right: &Arc<World>) -> bool {
    left.key == right.key
}

fn success_count(count: usize) -> i32 {
    i32::try_from(count).map_or(i32::MAX, |count| count)
}

fn conditional_failed(count: usize) -> CommandError {
    if count == 0 {
        return CommandError::failure(translations::COMMANDS_EXECUTE_CONDITIONAL_FAIL.msg());
    }

    CommandError::failure(
        translations::COMMANDS_EXECUTE_CONDITIONAL_FAIL_COUNT
            .message([TextComponent::from(success_count(count).to_string())]),
    )
}

fn execute_simple_condition(
    context: &CommandContext,
    matches: bool,
    expected: bool,
) -> Result<CommandResult, CommandError> {
    if matches == expected {
        send_condition_pass(context);
        Ok(CommandResult::success())
    } else {
        Err(conditional_failed(0))
    }
}

fn send_condition_pass(context: &CommandContext) {
    context.sender.send_message(
        &TextComponent::from(&translations::COMMANDS_EXECUTE_CONDITIONAL_PASS),
    );
}

fn send_condition_pass_count(context: &CommandContext, count: usize) {
    context.sender.send_message(
        &translations::COMMANDS_EXECUTE_CONDITIONAL_PASS_COUNT
            .message([TextComponent::from(success_count(count).to_string())])
            .into(),
    );
}

#[derive(Clone, Copy, Debug)]
struct BlockRegion {
    min: BlockPos,
    max: BlockPos,
}

impl BlockRegion {
    fn from_corners(first: BlockPos, second: BlockPos) -> Self {
        Self {
            min: BlockPos::min(first, second),
            max: BlockPos::max(first, second),
        }
    }

    fn area(self) -> i64 {
        let x_span = i64::from(self.max.x() - self.min.x()) + 1;
        let y_span = i64::from(self.max.y() - self.min.y()) + 1;
        let z_span = i64::from(self.max.z() - self.min.z()) + 1;
        x_span * y_span * z_span
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Weak};

    use glam::DVec3;
    use simdnbt::owned::NbtTag;
    use steel_registry::{entity_type::EntityTypeRef, test_support::init_test_registry, vanilla_entities};
    use steel_utils::{Identifier, nbt::parse_nbt_path};

    use crate::command::graph::CommandGraph;
    use crate::command::requirement::{
        CommandInputContext, CommandSourceKind, PermissionExpr, RequirementContext,
    };
    use crate::command::storage::CommandStorage;
    use crate::entity::{Entity, EntityBase};
    use crate::scoreboard::{ScoreHolder, Scoreboard};

    struct TestContext;

    struct StoreTestEntity {
        base: EntityBase,
    }

    impl StoreTestEntity {
        fn shared(id: i32, position: DVec3) -> Arc<Self> {
            Arc::new(Self {
                base: EntityBase::new(id, position, vanilla_entities::ITEM.dimensions, Weak::new()),
            })
        }
    }

    impl Entity for StoreTestEntity {
        fn base(&self) -> &EntityBase {
            &self.base
        }

        fn entity_type(&self) -> EntityTypeRef {
            &vanilla_entities::ITEM
        }
    }

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
            .expect("execute command registers")
    }

    #[test]
    fn loaded_condition_parses_direct_and_redirect_forms() {
        let graph = graph();
        let context = TestContext;

        let direct = graph
            .parse("execute if loaded 0 64 0", &context)
            .expect("direct loaded conditional parses");
        assert_eq!(direct.path(), ["execute", "if", "loaded", "pos"]);

        let redirected = graph
            .parse("execute unless loaded 0 64 0 run seed", &context)
            .expect("redirected loaded conditional parses");
        assert_eq!(redirected.path(), ["execute", "unless", "loaded", "pos"]);
    }

    #[test]
    fn biome_condition_parses_direct_and_redirect_forms() {
        init_test_registry();
        let graph = graph();
        let context = TestContext;

        let direct = graph
            .parse("execute if biome 0 64 0 plains", &context)
            .expect("direct biome conditional parses");
        assert_eq!(direct.path(), ["execute", "if", "biome", "pos", "biome"]);

        let redirected = graph
            .parse("execute unless biome 0 64 0 #is_overworld run seed", &context)
            .expect("redirected biome conditional parses");
        assert_eq!(
            redirected.path(),
            ["execute", "unless", "biome", "pos", "biome"]
        );
    }

    #[test]
    fn block_condition_parses_direct_and_redirect_forms() {
        init_test_registry();
        let graph = graph();
        let context = TestContext;

        let direct = graph
            .parse("execute if block 0 64 0 stone", &context)
            .expect("direct block conditional parses");
        assert_eq!(direct.path(), ["execute", "if", "block", "pos", "block"]);

        let redirected = graph
            .parse(
                "execute unless block 0 64 0 oak_log[axis=y]{id:'minecraft:barrel'} run seed",
                &context,
            )
            .expect("redirected block conditional parses");
        assert_eq!(
            redirected.path(),
            ["execute", "unless", "block", "pos", "block"]
        );
    }

    #[test]
    fn data_block_condition_parses_direct_and_redirect_forms() {
        let graph = graph();
        let context = TestContext;

        let direct = graph
            .parse(
                "execute if data block 0 64 0 Items[{id:\"minecraft:stone\"}].Count",
                &context,
            )
            .expect("direct data block conditional parses");
        assert_eq!(
            direct.path(),
            ["execute", "if", "data", "block", "pos", "path"]
        );

        let redirected = graph
            .parse("execute unless data block 0 64 0 Items[] run seed", &context)
            .expect("redirected data block conditional parses");
        assert_eq!(
            redirected.path(),
            ["execute", "unless", "data", "block", "pos", "path"]
        );
    }

    #[test]
    fn data_storage_condition_parses_direct_and_redirect_forms() {
        let graph = graph();
        let context = TestContext;

        let direct = graph
            .parse("execute if data storage steel:data value", &context)
            .expect("direct data storage conditional parses");
        assert_eq!(
            direct.path(),
            ["execute", "if", "data", "storage", "source", "path"]
        );

        let redirected = graph
            .parse("execute unless data storage steel:data value run seed", &context)
            .expect("redirected data storage conditional parses");
        assert_eq!(
            redirected.path(),
            ["execute", "unless", "data", "storage", "source", "path"]
        );
    }

    #[test]
    fn data_condition_suggests_entity_accessor() {
        let graph = graph();
        let context = TestContext;

        let suggestions = graph
            .suggest("execute if data ", &context)
            .expect("data condition suggestions");
        let suggestions = suggestions
            .suggestions
            .iter()
            .map(|suggestion| suggestion.text.as_str())
            .collect::<Vec<_>>();

        assert!(suggestions.contains(&"entity"));
    }

    #[test]
    fn score_condition_parses_comparison_and_range_forms() {
        let graph = graph();
        let context = TestContext;

        let comparison = graph
            .parse("execute if score Steve kills = Alex kills", &context)
            .expect("score comparison conditional parses");
        assert_eq!(
            comparison.path(),
            [
                "execute",
                "if",
                "score",
                "target",
                "targetObjective",
                "=",
                "source",
                "sourceObjective"
            ]
        );

        let range = graph
            .parse("execute unless score Steve kills matches 1.. run seed", &context)
            .expect("score range conditional parses");
        assert_eq!(
            range.path(),
            [
                "execute",
                "unless",
                "score",
                "target",
                "targetObjective",
                "matches",
                "range"
            ]
        );
    }

    #[test]
    fn store_score_parses_redirect_form() {
        let graph = graph();
        let context = TestContext;

        let parsed = graph
            .parse("execute store result score Steve kills run seed", &context)
            .expect("store score parses");
        assert_eq!(
            parsed.path(),
            ["execute", "store", "result", "score", "targets", "objective"]
        );
    }

    #[test]
    fn store_block_data_parses_redirect_form() {
        let graph = graph();
        let context = TestContext;

        let parsed = graph
            .parse(
                "execute store result block 0 64 0 Items[0].Count int 1 run seed",
                &context,
            )
            .expect("store block data parses");
        assert_eq!(
            parsed.path(),
            [
                "execute",
                "store",
                "result",
                "block",
                "targetPos",
                "path",
                "int",
                "scale"
            ]
        );
    }

    #[test]
    fn store_storage_data_parses_redirect_form() {
        let graph = graph();
        let context = TestContext;

        let parsed = graph
            .parse(
                "execute store result storage steel:data value int 1 run seed",
                &context,
            )
            .expect("store storage data parses");
        assert_eq!(
            parsed.path(),
            [
                "execute", "store", "result", "storage", "target", "path", "int", "scale"
            ]
        );
    }

    #[test]
    fn store_data_type_converts_scaled_value() {
        assert_eq!(super::store::StoreDataType::Int.tag(3, 2.5), NbtTag::Int(7));
        assert_eq!(
            super::store::StoreDataType::Long.tag(-3, 2.5),
            NbtTag::Long(-7)
        );
        assert_eq!(
            super::store::StoreDataType::Byte.tag(258, 1.0),
            NbtTag::Byte(2)
        );
        assert_eq!(
            super::store::StoreDataType::Short.tag(65_538, 1.0),
            NbtTag::Short(2)
        );
        assert_eq!(
            super::store::StoreDataType::Float.tag(3, 0.5),
            NbtTag::Float(1.5)
        );
        assert_eq!(
            super::store::StoreDataType::Double.tag(3, 0.5),
            NbtTag::Double(1.5)
        );
    }

    #[test]
    fn store_storage_data_value_updates_command_storage() {
        let storage = CommandStorage::new();
        let key = Identifier::new_static("steel", "data");
        let path = parse_nbt_path("value").expect("path parses");

        super::store::store_storage_data_value(&storage, &key, &path, NbtTag::Int(7))
            .expect("storage write succeeds");

        let data = storage.get(&key);
        assert_eq!(data.get("value"), Some(&NbtTag::Int(7)));
    }

    #[test]
    fn store_entity_data_value_updates_entity_data() {
        init_test_registry();

        let entity = StoreTestEntity::shared(1, DVec3::ZERO);
        let path = parse_nbt_path("Air").expect("path parses");

        super::store::store_entity_data_value(entity.as_ref(), &path, NbtTag::Int(123))
            .expect("entity data updates");

        assert_eq!(entity.air_supply(), 123);
    }

    #[test]
    fn score_comparison_requires_both_scores() {
        let scoreboard = Scoreboard::new();
        let kills = scoreboard
            .add_objective("kills")
            .expect("objective should be added");
        let steve = ScoreHolder::new("Steve");
        let alex = ScoreHolder::new("Alex");

        scoreboard
            .set_score(&steve, &kills, 3)
            .expect("score should be writable");
        assert!(!super::compare_scores(
            &scoreboard,
            &steve,
            &kills,
            &alex,
            &kills,
            super::ScoreComparison::Greater,
        ));

        scoreboard
            .set_score(&alex, &kills, 2)
            .expect("score should be writable");
        assert!(super::compare_scores(
            &scoreboard,
            &steve,
            &kills,
            &alex,
            &kills,
            super::ScoreComparison::Greater,
        ));
    }

    #[test]
    fn store_score_value_writes_all_holders() {
        let scoreboard = Scoreboard::new();
        let objective = scoreboard
            .add_objective("result")
            .expect("objective should be added");
        let holders = [ScoreHolder::new("Steve"), ScoreHolder::new("Alex")];

        super::store::store_score_value(&scoreboard, &holders, &objective, 11)
            .expect("store should write scores");

        assert_eq!(scoreboard.score(&holders[0], &objective), Some(11));
        assert_eq!(scoreboard.score(&holders[1], &objective), Some(11));
    }

    #[test]
    fn positioned_over_parses_redirect_form() {
        let graph = graph();
        let context = TestContext;

        let parsed = graph
            .parse("execute positioned over motion_blocking run seed", &context)
            .expect("positioned over parses");
        assert_eq!(parsed.path(), ["execute", "positioned", "over", "heightmap"]);
    }

    #[test]
    fn blocks_condition_parses_direct_and_redirect_forms() {
        let graph = graph();
        let context = TestContext;

        let direct = graph
            .parse("execute if blocks 0 64 0 1 64 1 10 64 10 all", &context)
            .expect("direct blocks conditional parses");
        assert_eq!(
            direct.path(),
            [
                "execute",
                "if",
                "blocks",
                "start",
                "end",
                "destination",
                "all"
            ]
        );

        let redirected = graph
            .parse("execute unless blocks 0 64 0 1 64 1 10 64 10 masked run seed", &context)
            .expect("redirected blocks conditional parses");
        assert_eq!(
            redirected.path(),
            [
                "execute",
                "unless",
                "blocks",
                "start",
                "end",
                "destination",
                "masked"
            ]
        );
    }

    #[test]
    fn on_relations_parse_vanilla_relation_names() {
        let graph = graph();
        let context = TestContext;

        for relation in [
            "owner",
            "leasher",
            "target",
            "attacker",
            "vehicle",
            "controller",
            "origin",
            "passengers",
        ] {
            let parsed = graph
                .parse(&format!("execute on {relation} run seed"), &context)
                .expect("relation redirect parses");
            assert_eq!(parsed.path(), ["execute", "on", relation]);
        }
    }
}
