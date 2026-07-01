use std::{borrow::Cow, sync::Arc};

use simdnbt::owned::{NbtCompound, NbtTag};
use steel_registry::{
    REGISTRY, RegistryExt, blocks::block_state_ext::BlockStateExt, loot_table::LootContext,
};
use steel_utils::{BlockPos, Identifier, nbt::compare_nbt, translations};
use text_components::{TextComponent, translation::TranslatedMessage};

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandFunctionArgumentValue, CommandNodeBuilder, CommandRedirectExecution,
    CommandRedirectTarget, CommandResult, DoubleRangeArgumentValue, IntRangeArgumentValue,
    LootPredicateArgumentValue, ParsedArguments, argument, literal,
};
use crate::command::loot::{CommandLootRandom, command_loot_entity_ref, command_loot_weather};
use crate::command::parsers::{
    BiomeParser, BlockPosParser, BlockPredicateParser, CommandFunctionParser, DoubleRangeParser,
    EntityParser, IntRangeParser, ItemPredicateParser, ItemSlotsParser, LootPredicateParser,
    NbtPathParser, ObjectiveParser, ScoreHolderParser, WorldParser,
};
use crate::command::{CommandExecutionBudget, CommandFunctionConditionResult};
use crate::entity::EntityCommandItemSlotResult;
use crate::scoreboard::{ScoreHolder, Scoreboard, ScoreboardObjective};
use crate::world::World;

use super::{
    StorageKeyParser, biome_value, block_data_invalid_error, block_entity_full_nbt,
    block_position, block_predicate, double_range, int_range, item_predicate, item_slots,
    loaded_named_block_position, loot_predicate, nbt_path, optional_entities, position_error,
    required_entities, same_world, scoreboard_objective, single_score_holder, source_entity,
    storage_id, world,
};
use super::super::item_predicate_match_error;
use super::super::stopwatch::{StopwatchIdParser, stopwatch_does_not_exist};

pub(super) fn conditionals(name: &'static str, expected: bool) -> CommandNodeBuilder {
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
        .then(literal("predicate").then(
            argument("predicate", LootPredicateParser)
                .executes(move |context, arguments| {
                    execute_predicate_condition(context, arguments, expected)
                })
                .forks(CommandRedirectTarget::Current, move |context, arguments| {
                    fork_predicate_condition(context, arguments, expected)
                }),
        ))
        .then(literal("function").then(argument("name", CommandFunctionParser).forks_with_budget_and_execution(
            CommandRedirectTarget::Current,
            move |context, arguments, budget, execution| {
                fork_function_condition(context, arguments, expected, budget, execution)
            },
        )))
        .then(
            literal("items")
                .then(literal("entity").then(
                    argument("entities", EntityParser::multiple()).then(
                        argument("slots", ItemSlotsParser).then(
                            argument("item_predicate", ItemPredicateParser)
                                .executes(move |context, arguments| {
                                    execute_entity_items_condition(context, arguments, expected)
                                })
                                .forks(
                                    CommandRedirectTarget::Current,
                                    move |context, arguments| {
                                        fork_entity_items_condition(context, arguments, expected)
                                    },
                                ),
                        ),
                    ),
                ))
                .then(literal("block").then(
                    argument("pos", BlockPosParser).then(
                        argument("slots", ItemSlotsParser).then(
                            argument("item_predicate", ItemPredicateParser)
                                .executes(move |context, arguments| {
                                    execute_block_items_condition(context, arguments, expected)
                                })
                                .forks(
                                    CommandRedirectTarget::Current,
                                    move |context, arguments| {
                                        fork_block_items_condition(context, arguments, expected)
                                    },
                                ),
                        ),
                    ),
                ))
        )
        .then(literal("stopwatch").then(
            argument("id", StopwatchIdParser::existing()).then(
                argument("range", DoubleRangeParser)
                    .executes(move |context, arguments| {
                        execute_stopwatch_condition(context, arguments, expected)
                    })
                    .forks(CommandRedirectTarget::Current, move |context, arguments| {
                        fork_stopwatch_condition(context, arguments, expected)
                    }),
            ),
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
    let count = optional_entities(context, arguments)?.len();
    if expected {
        if count == 0 {
            return Err(conditional_failed(count));
        }
        send_condition_pass_count(context, count);
        return Ok(CommandResult::from_success_count(success_count(count)));
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
    let matches = !optional_entities(context, arguments)?.is_empty();
    Ok(if matches == expected {
        vec![context.clone()]
    } else {
        Vec::new()
    })
}

fn execute_predicate_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<CommandResult, CommandError> {
    let matches = predicate_condition_matches(context, arguments)?;
    execute_simple_condition(context, matches, expected)
}

fn fork_predicate_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<Vec<CommandContext>, CommandError> {
    let matches = predicate_condition_matches(context, arguments)?;
    Ok(if matches == expected {
        vec![context.clone()]
    } else {
        Vec::new()
    })
}

fn fork_function_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
    budget: &mut CommandExecutionBudget,
    execution: CommandRedirectExecution,
) -> Result<Vec<CommandContext>, CommandError> {
    let Some(result) = function_condition_result(context, arguments, budget, execution)? else {
        return Ok(Vec::new());
    };
    Ok(if (result != 0) == expected {
        vec![context.clone()]
    } else {
        Vec::new()
    })
}

fn function_condition_result(
    context: &CommandContext,
    arguments: &ParsedArguments,
    budget: &mut CommandExecutionBudget,
    execution: CommandRedirectExecution,
) -> Result<Option<i32>, CommandError> {
    let functions = {
        let registry = context.server.command_functions.read();
        registry
            .resolve_argument(&command_function_argument(arguments)?)
            .map_err(|error| CommandError::failure(error.to_string()))?
    };
    let dispatcher = context.server.command_dispatcher.read().clone();
    match dispatcher.run_functions_for_condition(&functions, context, budget, execution.is_forked())? {
        CommandFunctionConditionResult::NoFunctions => Ok(None),
        CommandFunctionConditionResult::Callback(result) => Ok(Some(result.result)),
    }
}

fn command_function_argument(
    arguments: &ParsedArguments,
) -> Result<CommandFunctionArgumentValue, CommandError> {
    arguments
        .get::<CommandFunctionArgumentValue>("name")
        .map_err(super::super::invalid_parsed_argument)
}

fn predicate_condition_matches(
    context: &CommandContext,
    arguments: &ParsedArguments,
) -> Result<bool, CommandError> {
    let predicate = loot_predicate(arguments)?;
    let condition = match &predicate {
        LootPredicateArgumentValue::Reference(id) => {
            let Some(predicate) = REGISTRY.loot_predicates.by_key(id) else {
                return Ok(false);
            };
            &predicate.condition
        }
        LootPredicateArgumentValue::Inline(condition) => condition,
    };

    let mut random = context.world.random().lock();
    let mut rng = CommandLootRandom::new(&mut random);
    let mut loot_context = LootContext::new(&mut rng)
        .with_origin(context.position.x, context.position.y, context.position.z)
        .with_game_time(context.world.game_time())
        .with_weather(command_loot_weather(&context.world));
    if let Some(entity) = &context.entity {
        loot_context = loot_context.with_this_entity(command_loot_entity_ref(entity.as_ref()));
    }

    condition
        .try_test(&mut loot_context)
        .map_err(loot_predicate_evaluation_error)
}

fn loot_predicate_evaluation_error(error: impl std::fmt::Display) -> CommandError {
    CommandError::failure(TextComponent::from(format!(
        "unsupported loot predicate: {error}"
    )))
}

fn execute_entity_items_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<CommandResult, CommandError> {
    let count = entity_items_match_count(context, arguments)?;
    if expected {
        if count == 0 {
            return Err(conditional_failed(count));
        }
        send_condition_pass_count(context, count);
        return Ok(CommandResult::from_success_count(success_count(count)));
    }

    if count == 0 {
        send_condition_pass(context);
        Ok(CommandResult::success())
    } else {
        Err(conditional_failed(count))
    }
}

fn fork_entity_items_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<Vec<CommandContext>, CommandError> {
    let matches = entity_items_match_count(context, arguments)? > 0;
    Ok(if matches == expected {
        vec![context.clone()]
    } else {
        Vec::new()
    })
}

pub(super) fn entity_items_match_count(
    context: &dyn crate::command::requirement::CommandInputContext,
    arguments: &ParsedArguments,
) -> Result<usize, CommandError> {
    let targets = required_entities(context, arguments)?;
    let slots = item_slots(arguments)?;
    let predicate = item_predicate(arguments)?;
    let mut count = 0usize;

    for entity in &targets {
        for &slot_id in slots.slots() {
            let mut slot_count = Ok(0usize);
            let result = entity.with_command_item_slot(slot_id, &mut |item| {
                slot_count = predicate
                    .matches_stack(item)
                    .map_err(item_predicate_match_error)
                    .map(|matches| {
                        if matches {
                            usize::try_from(item.count()).map_or(0, |count| count)
                        } else {
                            0
                        }
                    });
            });

            match result {
                EntityCommandItemSlotResult::Found => {
                    count = count.saturating_add(slot_count?);
                }
                EntityCommandItemSlotResult::Missing => {}
                EntityCommandItemSlotResult::Unsupported => {
                    return Err(unsupported_entity_item_slot(slots.name()));
                }
            }
        }
    }

    Ok(count)
}

fn execute_block_items_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<CommandResult, CommandError> {
    let count = block_items_match_count(context, arguments)?;
    if expected {
        if count == 0 {
            return Err(conditional_failed(count));
        }
        send_condition_pass_count(context, count);
        return Ok(CommandResult::from_success_count(success_count(count)));
    }

    if count == 0 {
        send_condition_pass(context);
        Ok(CommandResult::success())
    } else {
        Err(conditional_failed(count))
    }
}

fn fork_block_items_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<Vec<CommandContext>, CommandError> {
    let matches = block_items_match_count(context, arguments)? > 0;
    Ok(if matches == expected {
        vec![context.clone()]
    } else {
        Vec::new()
    })
}

fn block_items_match_count(
    context: &CommandContext,
    arguments: &ParsedArguments,
) -> Result<usize, CommandError> {
    let pos = loaded_block_position(context, arguments)?;
    let Some(block_entity) = context.world.get_block_entity(pos) else {
        return Err(item_source_not_a_container(pos));
    };

    let block_entity = block_entity.lock();
    let Some(container) = block_entity.as_container() else {
        return Err(item_source_not_a_container(pos));
    };

    let slots = item_slots(arguments)?;
    let predicate = item_predicate(arguments)?;
    let mut count = 0usize;
    for &slot_id in slots.slots() {
        let Ok(slot) = usize::try_from(slot_id) else {
            continue;
        };
        if slot >= container.get_container_size() {
            continue;
        }

        let item = container.get_item(slot);
        if predicate
            .matches_stack(item)
            .map_err(item_predicate_match_error)?
        {
            count = count.saturating_add(usize::try_from(item.count()).map_or(0, |count| count));
        }
    }

    Ok(count)
}

#[derive(Clone, Copy, Debug)]
pub(super) enum ScoreComparison {
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
    let target = single_score_holder(context, arguments, "target")?;
    let target_objective = scoreboard_objective(context, arguments, "targetObjective")?;
    let source = single_score_holder(context, arguments, "source")?;
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

pub(super) fn compare_scores(
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
    let target = single_score_holder(context, arguments, "target")?;
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

fn execute_stopwatch_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<CommandResult, CommandError> {
    let matches = stopwatch_matches(context, arguments)?;
    execute_simple_condition(context, matches, expected)
}

fn fork_stopwatch_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<Vec<CommandContext>, CommandError> {
    let matches = stopwatch_matches(context, arguments)?;
    Ok(if matches == expected {
        vec![context.clone()]
    } else {
        Vec::new()
    })
}

fn stopwatch_matches(
    context: &CommandContext,
    arguments: &ParsedArguments,
) -> Result<bool, CommandError> {
    let id = stopwatch_id(arguments)?;
    let range = double_range(arguments)?;
    let elapsed_seconds = context
        .server
        .stopwatches
        .read()
        .get(&id)
        .ok_or_else(|| stopwatch_does_not_exist(&id))?
        .elapsed_seconds();

    Ok(stopwatch_elapsed_matches_range(elapsed_seconds, range))
}

fn stopwatch_id(arguments: &ParsedArguments) -> Result<Identifier, CommandError> {
    arguments
        .get::<Identifier>("id")
        .map_err(super::super::invalid_parsed_argument)
}

fn stopwatch_elapsed_matches_range(elapsed_seconds: f64, range: DoubleRangeArgumentValue) -> bool {
    range.matches(elapsed_seconds)
}

fn execute_dimension_condition(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    expected: bool,
) -> Result<CommandResult, CommandError> {
    let matches = same_world(&context.world, &world(context, arguments)?);
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
    let matches = same_world(&context.world, &world(context, arguments)?);
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
        return Ok(CommandResult::from_success_count(success_count(count)));
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
    let count = entity_data_match_count(context, arguments)?;
    if expected {
        if count == 0 {
            return Err(conditional_failed(count));
        }
        send_condition_pass_count(context, count);
        return Ok(CommandResult::from_success_count(success_count(count)));
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
        return Ok(CommandResult::from_success_count(success_count(count)));
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
    let matches = entity_data_match_count(context, arguments)? > 0;
    Ok(if matches == expected {
        vec![context.clone()]
    } else {
        Vec::new()
    })
}

fn entity_data_match_count(
    context: &dyn crate::command::requirement::CommandInputContext,
    arguments: &ParsedArguments,
) -> Result<usize, CommandError> {
    let entity = source_entity(context, arguments)?;
    let tag = NbtTag::Compound(entity.nbt_for_data_compare());
    Ok(nbt_path(arguments)?.count_matching(&tag))
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
            return Ok(CommandResult::from_success_count(success_count(count)));
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

fn item_source_not_a_container(pos: BlockPos) -> CommandError {
    CommandError::failure(TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed("commands.item.source.not_a_container"),
        fallback: None,
        args: Some(Box::new([
            TextComponent::from(pos.x().to_string()),
            TextComponent::from(pos.y().to_string()),
            TextComponent::from(pos.z().to_string()),
        ])),
    }))
}

fn unsupported_entity_item_slot(slot: &str) -> CommandError {
    CommandError::failure(TextComponent::from(format!(
        "unsupported entity item slot '{slot}'"
    )))
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
