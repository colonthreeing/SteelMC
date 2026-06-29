//! Handler for the "execute" command.
//!
//! Store callbacks, scoreboards, data/NBT paths, predicates, functions, item
//! predicates, block predicates, region comparisons, stopwatch predicates, and
//! heightmap positioning are not registered here yet because their backing
//! foundations are not implemented in Steel's command/runtime layer.

use std::sync::Arc;

use glam::DVec3;
use steel_protocol::packets::game::{ArgumentType, SuggestionEntry};
use steel_registry::entity_type::EntityTypeRef;
use steel_utils::translations;
use text_components::TextComponent;

use crate::command::CommandRegistrationSpec;
use crate::command::context::{CommandContext, EntityAnchor, anchored_position};
use crate::command::error::CommandError;
use crate::command::graph::{
    AnchorParser, CommandArgumentClientParser, CommandArgumentParser, CommandNodeBuilder,
    CommandParseError, CommandParseErrorKind, CommandRedirectTarget, CommandResult,
    ParsedArgument, ParsedArguments, argument, literal,
};
use crate::command::parsers::{EntityParser, EntitySummonParser, RotationParser, Vec3Parser, WorldParser};
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
