use glam::DVec3;
use steel_utils::BlockPos;

use crate::command::context::{
    CommandContext, EntityAnchor, anchored_position,
};
use crate::command::error::CommandError;
use crate::command::graph::{
    AnchorParser, CommandNodeBuilder, CommandRedirectTarget, CommandResult, ParsedArguments,
    argument, literal,
};
use crate::command::parsers::{
    EntityParser, EntitySummonParser, HeightmapParser, RotationParser, Vec3Parser, WorldParser,
};
use crate::entity::{Mob, SharedEntity};

use super::{
    AxesParser, anchor, entities, entity_type, heightmap, position, position_error, rotation,
    world,
};

pub(super) fn as_operation() -> CommandNodeBuilder {
    literal("as").then(argument("targets", EntityParser::multiple()).forks(
        CommandRedirectTarget::Current,
        fork_as,
    ))
}

pub(super) fn at_operation() -> CommandNodeBuilder {
    literal("at").then(argument("targets", EntityParser::multiple()).forks(
        CommandRedirectTarget::Current,
        fork_at,
    ))
}

pub(super) fn positioned_operation() -> CommandNodeBuilder {
    literal("positioned")
        .then(argument("pos", Vec3Parser).redirects(CommandRedirectTarget::Current, set_position))
        .then(literal("as").then(argument("targets", EntityParser::multiple()).forks(
            CommandRedirectTarget::Current,
            fork_positioned_as,
        )))
        .then(literal("over").then(
            argument("heightmap", HeightmapParser)
                .redirects(CommandRedirectTarget::Current, set_position_over),
        ))
}

pub(super) fn rotated_operation() -> CommandNodeBuilder {
    literal("rotated")
        .then(argument("rot", RotationParser).redirects(CommandRedirectTarget::Current, set_rotation))
        .then(literal("as").then(argument("targets", EntityParser::multiple()).forks(
            CommandRedirectTarget::Current,
            fork_rotated_as,
        )))
}

pub(super) fn facing_operation() -> CommandNodeBuilder {
    literal("facing")
        .then(literal("entity").then(
            argument("targets", EntityParser::multiple()).then(
                argument("anchor", AnchorParser).forks(
                    CommandRedirectTarget::Current,
                    fork_facing_entity,
                ),
            ),
        ))
        .then(argument("pos", Vec3Parser).redirects(CommandRedirectTarget::Current, face_position))
}

pub(super) fn align_operation() -> CommandNodeBuilder {
    literal("align").then(
        argument("axes", AxesParser).redirects(CommandRedirectTarget::Current, align_position),
    )
}

pub(super) fn anchored_operation() -> CommandNodeBuilder {
    literal("anchored").then(
        argument("anchor", AnchorParser).redirects(CommandRedirectTarget::Current, set_anchor),
    )
}

pub(super) fn in_operation() -> CommandNodeBuilder {
    literal("in").then(argument("dimension", WorldParser).redirects(
        CommandRedirectTarget::Current,
        set_world,
    ))
}

pub(super) fn summon_operation() -> CommandNodeBuilder {
    literal("summon").then(argument("entity", EntitySummonParser).redirects(
        CommandRedirectTarget::Current,
        summon_and_redirect,
    ))
}

pub(super) fn on_relations() -> CommandNodeBuilder {
    literal("on")
        .then(literal("owner").forks(CommandRedirectTarget::Current, fork_on_owner))
        .then(literal("leasher").forks(CommandRedirectTarget::Current, fork_on_leasher))
        .then(literal("target").forks(CommandRedirectTarget::Current, fork_on_target))
        .then(literal("attacker").forks(CommandRedirectTarget::Current, fork_on_attacker))
        .then(literal("vehicle").forks(CommandRedirectTarget::Current, fork_on_vehicle))
        .then(literal("controller").forks(CommandRedirectTarget::Current, fork_on_controller))
        .then(literal("origin").forks(CommandRedirectTarget::Current, fork_on_origin))
        .then(literal("passengers").forks(CommandRedirectTarget::Current, fork_on_passengers))
}

fn fork_as(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<Vec<CommandContext>, CommandError> {
    Ok(entities(context, arguments)?
        .into_iter()
        .map(|entity| context.clone().with_entity(entity))
        .collect())
}

fn fork_at(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<Vec<CommandContext>, CommandError> {
    Ok(entities(context, arguments)?
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
    Ok(entities(context, arguments)?
        .into_iter()
        .map(|entity| context.clone().with_position(entity.position()))
        .collect())
}

fn fork_rotated_as(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<Vec<CommandContext>, CommandError> {
    Ok(entities(context, arguments)?
        .into_iter()
        .map(|entity| context.clone().with_rotation(entity.rotation()))
        .collect())
}

fn fork_facing_entity(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<Vec<CommandContext>, CommandError> {
    let anchor = anchor(arguments)?;
    Ok(entities(context, arguments)?
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
        .map_err(super::super::invalid_parsed_argument)?;
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
    let entity = super::super::summon::create_entity(context, entity_type(arguments)?, context.position)?;
    *context = context.clone().with_entity(entity);
    Ok(CommandResult::success())
}

fn fork_on_attacker(
    context: &mut CommandContext,
    _arguments: &ParsedArguments,
) -> Result<Vec<CommandContext>, CommandError> {
    Ok(one_relation_context(
        context,
        context.entity.as_ref().and_then(attacker_relation),
    ))
}

fn fork_on_owner(
    context: &mut CommandContext,
    _arguments: &ParsedArguments,
) -> Result<Vec<CommandContext>, CommandError> {
    Ok(one_relation_context(
        context,
        context
            .entity
            .as_ref()
            .and_then(|entity| entity.owning_entity()),
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

fn fork_on_origin(
    context: &mut CommandContext,
    _arguments: &ParsedArguments,
) -> Result<Vec<CommandContext>, CommandError> {
    Ok(one_relation_context(
        context,
        context
            .entity
            .as_ref()
            .and_then(|entity| entity.origin_entity()),
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
        context.entity.as_ref().and_then(target_relation),
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

fn attacker_relation(entity: &SharedEntity) -> Option<SharedEntity> {
    entity
        .as_attackable()
        .and_then(|attackable| attackable.last_attacker())
}

fn target_relation(entity: &SharedEntity) -> Option<SharedEntity> {
    entity
        .as_targeting()
        .and_then(|targeting| targeting.target_entity())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Weak};

    use glam::DVec3;
    use steel_registry::{
        entity_type::EntityTypeRef, test_support::init_test_registry, vanilla_entities,
    };

    use super::{attacker_relation, target_relation};
    use crate::entity::{
        Attackable, Entity, EntityBase, EntityCapabilities, SharedEntity, Targeting,
    };

    struct RelationTestEntity {
        base: EntityBase,
        attacker: Option<SharedEntity>,
        target: Option<SharedEntity>,
        attackable: bool,
        targeting: bool,
    }

    impl RelationTestEntity {
        fn shared(
            id: i32,
            attacker: Option<SharedEntity>,
            target: Option<SharedEntity>,
            attackable: bool,
            targeting: bool,
        ) -> SharedEntity {
            Arc::new(Self {
                base: EntityBase::new(id, DVec3::ZERO, vanilla_entities::ITEM.dimensions, Weak::new()),
                attacker,
                target,
                attackable,
                targeting,
            })
        }
    }

    impl Entity for RelationTestEntity {
        fn base(&self) -> &EntityBase {
            &self.base
        }

        fn entity_type(&self) -> EntityTypeRef {
            &vanilla_entities::ITEM
        }

        fn capabilities(&self) -> EntityCapabilities<'_> {
            let mut capabilities = EntityCapabilities::none();
            if self.attackable {
                capabilities = capabilities.with_attackable(self);
            }
            if self.targeting {
                capabilities = capabilities.with_targeting(self);
            }
            capabilities
        }
    }

    impl Attackable for RelationTestEntity {
        fn last_attacker(&self) -> Option<SharedEntity> {
            self.attacker.clone()
        }
    }

    impl Targeting for RelationTestEntity {
        fn target_entity(&self) -> Option<SharedEntity> {
            self.target.clone()
        }
    }

    #[test]
    fn execute_on_attacker_uses_attackable_capability_without_living_behavior() {
        init_test_registry();
        let attacker = RelationTestEntity::shared(2, None, None, false, false);
        let entity =
            RelationTestEntity::shared(1, Some(Arc::clone(&attacker)), None, true, false);

        assert!(entity.as_living_entity().is_none());
        let resolved = attacker_relation(&entity).expect("attackable relation resolves attacker");

        assert!(Arc::ptr_eq(&resolved, &attacker));
    }

    #[test]
    fn execute_on_target_uses_targeting_capability_without_mob_behavior() {
        init_test_registry();
        let target = RelationTestEntity::shared(2, None, None, false, false);
        let entity = RelationTestEntity::shared(1, None, Some(Arc::clone(&target)), false, true);

        assert!(entity.as_mob().is_none());
        let resolved = target_relation(&entity).expect("targeting relation resolves target");

        assert!(Arc::ptr_eq(&resolved, &target));
    }
}
