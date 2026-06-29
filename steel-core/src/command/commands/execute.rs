//! Handler for the "execute" command.
//!
//! Store callbacks, scoreboards, data/NBT paths, predicates, functions, item
//! predicates, block predicates, and stopwatch predicates are not registered
//! here yet because their backing foundations are not implemented in Steel's
//! command/runtime layer.

use std::{borrow::Cow, sync::Arc};

use glam::DVec3;
use simdnbt::owned::{NbtCompound, NbtTag};
use steel_protocol::packets::game::{ArgumentType, SuggestionEntry};
use steel_registry::blocks::block_state_ext::BlockStateExt;
use steel_registry::entity_type::EntityTypeRef;
use steel_utils::{BlockPos, nbt::compare_nbt, translations};
use text_components::TextComponent;
use text_components::translation::TranslatedMessage;

use crate::chunk::heightmap::HeightmapType;
use crate::command::CommandRegistrationSpec;
use crate::command::context::{CommandContext, EntityAnchor, anchored_position};
use crate::command::error::CommandError;
use crate::command::graph::{
    AnchorParser, BiomeArgumentValue, BlockPredicateArgumentValue, CommandArgumentClientParser,
    CommandArgumentParser, CommandNodeBuilder, CommandParseError, CommandParseErrorKind,
    CommandRedirectTarget, CommandResult, ParsedArgument, ParsedArguments, argument, literal,
};
use crate::command::parsers::{
    BiomeParser, BlockPosParser, BlockPredicateParser, EntityParser, EntitySummonParser,
    HeightmapParser, RotationParser, Vec3Parser, WorldParser,
};
use crate::command::reader::CommandReader;
use crate::command::requirement::CommandInputContext;
use crate::entity::{Mob, SharedEntity};
use crate::world::World;

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
        .then(
            literal("as")
                .then(argument("targets", EntityParser::multiple()).forks(
                    CommandRedirectTarget::Current,
                    fork_as,
                )),
        )
        .then(
            literal("at")
                .then(argument("targets", EntityParser::multiple()).forks(
                    CommandRedirectTarget::Current,
                    fork_at,
                )),
        )
        .then(
            literal("positioned")
                .then(
                    argument("pos", Vec3Parser)
                        .redirects(CommandRedirectTarget::Current, set_position),
                )
                .then(
                    literal("as").then(argument("targets", EntityParser::multiple()).forks(
                        CommandRedirectTarget::Current,
                        fork_positioned_as,
                    )),
                )
                .then(
                    literal("over").then(argument("heightmap", HeightmapParser).redirects(
                        CommandRedirectTarget::Current,
                        set_position_over,
                    )),
                ),
        )
        .then(
            literal("rotated")
                .then(
                    argument("rot", RotationParser)
                        .redirects(CommandRedirectTarget::Current, set_rotation),
                )
                .then(
                    literal("as").then(argument("targets", EntityParser::multiple()).forks(
                        CommandRedirectTarget::Current,
                        fork_rotated_as,
                    )),
                ),
        )
        .then(
            literal("facing")
                .then(
                    literal("entity").then(
                        argument("targets", EntityParser::multiple()).then(
                            argument("anchor", AnchorParser)
                                .forks(CommandRedirectTarget::Current, fork_facing_entity),
                        ),
                    ),
                )
                .then(
                    argument("pos", Vec3Parser)
                        .redirects(CommandRedirectTarget::Current, face_position),
                ),
        )
        .then(
            literal("align")
                .then(argument("axes", AxesParser).redirects(
                    CommandRedirectTarget::Current,
                    align_position,
                )),
        )
        .then(
            literal("anchored")
                .then(argument("anchor", AnchorParser).redirects(
                    CommandRedirectTarget::Current,
                    set_anchor,
                )),
        )
        .then(
            literal("in")
                .then(argument("dimension", WorldParser).redirects(
                    CommandRedirectTarget::Current,
                    set_world,
                )),
        )
        .then(
            literal("summon")
                .then(argument("entity", EntitySummonParser).redirects(
                    CommandRedirectTarget::Current,
                    summon_and_redirect,
                )),
        )
        .then(on_relations())
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

fn on_relations() -> CommandNodeBuilder {
    literal("on")
        .then(literal("attacker").forks(CommandRedirectTarget::Current, fork_on_attacker))
        .then(literal("controller").forks(CommandRedirectTarget::Current, fork_on_controller))
        .then(literal("leasher").forks(CommandRedirectTarget::Current, fork_on_leasher))
        .then(literal("passengers").forks(CommandRedirectTarget::Current, fork_on_passengers))
        .then(literal("target").forks(CommandRedirectTarget::Current, fork_on_target))
        .then(literal("vehicle").forks(CommandRedirectTarget::Current, fork_on_vehicle))
}

fn fork_as(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<Vec<CommandContext>, CommandError> {
    Ok(entities(arguments)?
        .into_iter()
        .map(|entity| context.clone().with_entity(entity))
        .collect())
}

fn fork_at(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<Vec<CommandContext>, CommandError> {
    Ok(entities(arguments)?
        .into_iter()
        .filter_map(|entity| {
            let world = entity.level()?;
            Some(
                context
                    .clone()
                    .with_world(world)
                    .with_position(entity.position())
                    .with_rotation(entity.rotation()),
            )
        })
        .collect())
}

fn fork_positioned_as(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<Vec<CommandContext>, CommandError> {
    Ok(entities(arguments)?
        .into_iter()
        .map(|entity| context.clone().with_position(entity.position()))
        .collect())
}

fn fork_rotated_as(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<Vec<CommandContext>, CommandError> {
    Ok(entities(arguments)?
        .into_iter()
        .map(|entity| context.clone().with_rotation(entity.rotation()))
        .collect())
}

fn fork_facing_entity(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<Vec<CommandContext>, CommandError> {
    let anchor = anchor(arguments)?;
    Ok(entities(arguments)?
        .into_iter()
        .map(|entity| {
            let target = anchored_position(entity.position(), Some(entity.as_ref()), anchor);
            context.clone().facing_position(target)
        })
        .collect())
}

fn set_position(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    *context = context
        .clone()
        .with_position(position(arguments)?)
        .with_anchor(EntityAnchor::Feet);
    Ok(CommandResult::success())
}

fn set_position_over(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let heightmap_type = heightmap(arguments)?;
    let x = context.position.x.floor() as i32;
    let z = context.position.z.floor() as i32;
    if !context
        .world
        .is_full_chunk_loaded_at(BlockPos::new(x, 0, z))
    {
        return Err(position_error("argument.pos.unloaded"));
    }
    let Some(height) = context.world.heightmap_first_available(heightmap_type, x, z) else {
        return Err(position_error("argument.pos.unloaded"));
    };

    *context = context
        .clone()
        .with_position(DVec3::new(context.position.x, f64::from(height), context.position.z));
    Ok(CommandResult::success())
}

fn set_rotation(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    *context = context.clone().with_rotation(rotation(arguments)?);
    Ok(CommandResult::success())
}

fn face_position(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    *context = context.clone().facing_position(position(arguments)?);
    Ok(CommandResult::success())
}

fn align_position(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let axes = arguments
        .get::<String>("axes")
        .map_err(super::invalid_parsed_argument)?;
    let mut position = context.position;
    if axes.contains('x') {
        position.x = position.x.floor();
    }
    if axes.contains('y') {
        position.y = position.y.floor();
    }
    if axes.contains('z') {
        position.z = position.z.floor();
    }
    *context = context.clone().with_position(position);
    Ok(CommandResult::success())
}

fn set_anchor(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    *context = context.clone().with_anchor(anchor(arguments)?);
    Ok(CommandResult::success())
}

fn set_world(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    *context = context.clone().with_world(world(arguments)?);
    Ok(CommandResult::success())
}

fn summon_and_redirect(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let entity = super::summon::create_entity(context, entity_type(arguments)?, context.position)?;
    *context = context.clone().with_entity(entity);
    Ok(CommandResult::success())
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
    let mut actual = NbtCompound::new();
    let entity_pos = block_entity.get_block_pos();
    actual.insert("id", block_entity.get_type().key.to_string());
    actual.insert("x", entity_pos.x());
    actual.insert("y", entity_pos.y());
    actual.insert("z", entity_pos.z());
    block_entity.save_additional(&mut actual);

    let expected = NbtTag::Compound(expected_nbt.clone());
    let actual = NbtTag::Compound(actual);
    Ok(compare_nbt(Some(&expected), Some(&actual), true))
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

fn fork_on_attacker(
    context: &mut CommandContext,
    _arguments: &ParsedArguments,
) -> Result<Vec<CommandContext>, CommandError> {
    Ok(one_relation_context(
        context,
        context
            .entity
            .as_ref()
            .and_then(|entity| entity.as_living_entity())
            .and_then(|living| living.living_base().last_hurt_by_mob()),
    ))
}

fn fork_on_controller(
    context: &mut CommandContext,
    _arguments: &ParsedArguments,
) -> Result<Vec<CommandContext>, CommandError> {
    Ok(one_relation_context(
        context,
        context
            .entity
            .as_ref()
            .and_then(|entity| entity.controlling_passenger()),
    ))
}

fn fork_on_leasher(
    context: &mut CommandContext,
    _arguments: &ParsedArguments,
) -> Result<Vec<CommandContext>, CommandError> {
    Ok(one_relation_context(
        context,
        context
            .entity
            .as_ref()
            .and_then(|entity| entity.as_mob())
            .and_then(Mob::leash_holder),
    ))
}

fn fork_on_passengers(
    context: &mut CommandContext,
    _arguments: &ParsedArguments,
) -> Result<Vec<CommandContext>, CommandError> {
    Ok(context.entity.as_ref().map_or_else(Vec::new, |entity| {
        entity
            .passengers()
            .into_iter()
            .filter(|passenger| !passenger.is_removed())
            .map(|passenger| context.clone().with_entity(passenger))
            .collect()
    }))
}

fn fork_on_target(
    context: &mut CommandContext,
    _arguments: &ParsedArguments,
) -> Result<Vec<CommandContext>, CommandError> {
    Ok(one_relation_context(
        context,
        context
            .entity
            .as_ref()
            .and_then(|entity| entity.as_mob())
            .and_then(Mob::target),
    ))
}

fn fork_on_vehicle(
    context: &mut CommandContext,
    _arguments: &ParsedArguments,
) -> Result<Vec<CommandContext>, CommandError> {
    Ok(one_relation_context(
        context,
        context.entity.as_ref().and_then(|entity| entity.vehicle()),
    ))
}

fn one_relation_context(context: &CommandContext, entity: Option<SharedEntity>) -> Vec<CommandContext> {
    entity
        .filter(|entity| !entity.is_removed())
        .map_or_else(Vec::new, |entity| vec![context.clone().with_entity(entity)])
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

#[derive(Clone, Copy, Debug, Default)]
struct AxesParser;

impl CommandArgumentParser for AxesParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let axes = reader.read_token()?;
        if !is_valid_axes(&axes) {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidSwizzle(axes),
                cursor,
            ));
        }

        Ok(ParsedArgument::String(axes))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::Swizzle, None)
    }

    fn parsed_type(&self) -> &'static str {
        "string"
    }

    fn examples(&self) -> &'static [&'static str] {
        &["x", "xz", "xyz"]
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        ["x", "y", "z", "xy", "xz", "yz", "xyz"]
            .into_iter()
            .filter(|axes| axes.starts_with(prefix))
            .map(SuggestionEntry::new)
            .collect()
    }
}

fn is_valid_axes(axes: &str) -> bool {
    if axes.is_empty() {
        return false;
    }

    let mut seen_x = false;
    let mut seen_y = false;
    let mut seen_z = false;
    for axis in axes.chars() {
        match axis {
            'x' if !seen_x => seen_x = true,
            'y' if !seen_y => seen_y = true,
            'z' if !seen_z => seen_z = true,
            _ => return false,
        }
    }
    true
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
    use glam::DVec3;
    use steel_registry::test_support::init_test_registry;

    use crate::command::graph::CommandGraph;
    use crate::command::requirement::{
        CommandInputContext, CommandSourceKind, PermissionExpr, RequirementContext,
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
}
