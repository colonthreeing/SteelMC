use std::{fmt, io::Cursor, sync::Arc};

use simdnbt::{
    borrow::read_compound as read_borrowed_compound,
    owned::NbtTag,
};
use steel_utils::{
    BlockPos, Identifier,
    nbt::{NbtPath, NbtPathMutationError},
};
use crate::command::context::{CommandContext, CommandResultCallback};
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandNodeBuilder, CommandRedirectTarget, CommandResult, DoubleParser, ParsedArguments,
    argument, literal,
};
use crate::command::parsers::{
    BlockPosParser, EntityParser, NbtPathParser, ObjectiveParser, ScoreHolderParser,
};
use crate::command::storage::CommandStorage;
use crate::scoreboard::{ScoreHolder, Scoreboard, ScoreboardError, ScoreboardObjective};
use crate::world::World;

use super::{
    StorageKeyParser, block_data_invalid_error, block_entity_full_nbt, double,
    loaded_named_block_position, nbt_path, score_holders_or_tracked, scoreboard_objective,
    single_entity, storage_id,
};
use super::super::bossbar::{BossBarIdParser, bossbar_does_not_exist};

pub(super) fn target(name: &'static str, store_result: bool) -> CommandNodeBuilder {
    literal(name)
        .then(literal("score").then(
            argument("targets", ScoreHolderParser::multiple()).then(
                argument("objective", ObjectiveParser).redirects(
                    CommandRedirectTarget::Current,
                    move |context, arguments| store_score(context, arguments, store_result),
                ),
            ),
        ))
        .then(literal("bossbar").then(
            argument("id", BossBarIdParser::existing())
                .then(literal("value").redirects(
                    CommandRedirectTarget::Current,
                    move |context, arguments| {
                        store_bossbar(context, arguments, true, store_result)
                    },
                ))
                .then(literal("max").redirects(
                    CommandRedirectTarget::Current,
                    move |context, arguments| {
                        store_bossbar(context, arguments, false, store_result)
                    },
                )),
        ))
        .then(literal("block").then(
            argument("targetPos", BlockPosParser).then(
                argument("path", NbtPathParser)
                    .then(store_block_data_type("int", StoreDataType::Int, store_result))
                    .then(store_block_data_type(
                        "float",
                        StoreDataType::Float,
                        store_result,
                    ))
                    .then(store_block_data_type(
                        "short",
                        StoreDataType::Short,
                        store_result,
                    ))
                    .then(store_block_data_type("long", StoreDataType::Long, store_result))
                    .then(store_block_data_type(
                        "double",
                        StoreDataType::Double,
                        store_result,
                    ))
                    .then(store_block_data_type("byte", StoreDataType::Byte, store_result)),
            ),
        ))
        .then(literal("storage").then(
            argument("target", StorageKeyParser).then(
                argument("path", NbtPathParser)
                    .then(store_storage_data_type(
                        "int",
                        StoreDataType::Int,
                        store_result,
                    ))
                    .then(store_storage_data_type(
                        "float",
                        StoreDataType::Float,
                        store_result,
                    ))
                    .then(store_storage_data_type(
                        "short",
                        StoreDataType::Short,
                        store_result,
                    ))
                    .then(store_storage_data_type(
                        "long",
                        StoreDataType::Long,
                        store_result,
                    ))
                    .then(store_storage_data_type(
                        "double",
                        StoreDataType::Double,
                        store_result,
                    ))
                    .then(store_storage_data_type(
                        "byte",
                        StoreDataType::Byte,
                        store_result,
                    )),
            ),
        ))
        .then(literal("entity").then(
            argument("target", EntityParser::one()).then(
                argument("path", NbtPathParser)
                    .then(store_entity_data_type(
                        "int",
                        StoreDataType::Int,
                        store_result,
                    ))
                    .then(store_entity_data_type(
                        "float",
                        StoreDataType::Float,
                        store_result,
                    ))
                    .then(store_entity_data_type(
                        "short",
                        StoreDataType::Short,
                        store_result,
                    ))
                    .then(store_entity_data_type(
                        "long",
                        StoreDataType::Long,
                        store_result,
                    ))
                    .then(store_entity_data_type(
                        "double",
                        StoreDataType::Double,
                        store_result,
                    ))
                    .then(store_entity_data_type(
                        "byte",
                        StoreDataType::Byte,
                        store_result,
                    )),
            ),
        ))
}

fn store_block_data_type(
    name: &'static str,
    data_type: StoreDataType,
    store_result: bool,
) -> CommandNodeBuilder {
    literal(name).then(argument("scale", DoubleParser::new()).redirects(
        CommandRedirectTarget::Current,
        move |context, arguments| store_block_data(context, arguments, data_type, store_result),
    ))
}

fn store_storage_data_type(
    name: &'static str,
    data_type: StoreDataType,
    store_result: bool,
) -> CommandNodeBuilder {
    literal(name).then(argument("scale", DoubleParser::new()).redirects(
        CommandRedirectTarget::Current,
        move |context, arguments| store_storage_data(context, arguments, data_type, store_result),
    ))
}

fn store_entity_data_type(
    name: &'static str,
    data_type: StoreDataType,
    store_result: bool,
) -> CommandNodeBuilder {
    literal(name).then(argument("scale", DoubleParser::new()).redirects(
        CommandRedirectTarget::Current,
        move |context, arguments| store_entity_data(context, arguments, data_type, store_result),
    ))
}

#[derive(Clone, Copy, Debug)]
pub(super) enum StoreDataType {
    Byte,
    Short,
    Int,
    Long,
    Float,
    Double,
}

impl StoreDataType {
    pub(super) fn tag(self, value: i32, scale: f64) -> NbtTag {
        let scaled = f64::from(value) * scale;
        match self {
            Self::Byte => NbtTag::Byte(scaled as i32 as u8 as i8),
            Self::Short => NbtTag::Short(scaled as i32 as u16 as i16),
            Self::Int => NbtTag::Int(scaled as i32),
            Self::Long => NbtTag::Long(scaled as i64),
            Self::Float => NbtTag::Float(scaled as f32),
            Self::Double => NbtTag::Double(scaled),
        }
    }
}

fn store_block_data(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    data_type: StoreDataType,
    store_result: bool,
) -> Result<CommandResult, CommandError> {
    let pos = loaded_named_block_position(context, arguments, "targetPos")?;
    if context.world.get_block_entity(pos).is_none() {
        return Err(block_data_invalid_error());
    }

    let path = nbt_path(arguments)?;
    let scale = double(arguments, "scale")?;
    let world = Arc::clone(&context.world);
    let callback = CommandResultCallback::new(move |result| {
        let value = if store_result {
            result.result
        } else {
            i32::from(result.success)
        };
        let tag = data_type.tag(value, scale);
        if let Err(error) = store_block_data_value(&world, pos, &path, tag) {
            log::warn!("Failed to store execute command result in block data: {error}");
        }
    });
    context.chain_result_callback(callback);
    Ok(CommandResult::success())
}

fn store_bossbar(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    store_into_value: bool,
    store_result: bool,
) -> Result<CommandResult, CommandError> {
    let id = arguments
        .get::<Identifier>("id")
        .map_err(super::super::invalid_parsed_argument)?;
    if context.server.boss_bars.read().get(&id).is_none() {
        return Err(bossbar_does_not_exist(&id));
    }

    let server = Arc::clone(&context.server);
    let callback = CommandResultCallback::new(move |result| {
        let value = if store_result {
            result.result
        } else {
            i32::from(result.success)
        };
        let mutation = {
            let mut boss_bars = server.boss_bars.write();
            if store_into_value {
                boss_bars.set_value(&id, value)
            } else {
                boss_bars.set_max(&id, value)
            }
        };
        match mutation {
            Ok(mutation) => server.send_bossbar_broadcasts(mutation.broadcasts),
            Err(error) => {
                log::warn!("Failed to store execute command result in boss bar: {error:?}");
            }
        }
    });
    context.chain_result_callback(callback);
    Ok(CommandResult::success())
}

fn store_score(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    store_result: bool,
) -> Result<CommandResult, CommandError> {
    let objective = scoreboard_objective(context, arguments, "objective")?;
    let holders = score_holders_or_tracked(context, arguments, "targets")?;
    let server = Arc::clone(&context.server);
    let callback = CommandResultCallback::new(move |result| {
        let value = if store_result {
            result.result
        } else {
            i32::from(result.success)
        };
        if let Err(error) = store_score_value(&server.scoreboard, &holders, &objective, value) {
            log::warn!("Failed to store execute command result in scoreboard: {error}");
        }
    });
    context.chain_result_callback(callback);
    Ok(CommandResult::success())
}

fn store_storage_data(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    data_type: StoreDataType,
    store_result: bool,
) -> Result<CommandResult, CommandError> {
    let target = storage_id(arguments, "target")?;
    let path = nbt_path(arguments)?;
    let scale = double(arguments, "scale")?;
    let server = Arc::clone(&context.server);
    let callback = CommandResultCallback::new(move |result| {
        let value = if store_result {
            result.result
        } else {
            i32::from(result.success)
        };
        let tag = data_type.tag(value, scale);
        if let Err(error) = store_storage_data_value(&server.command_storage, &target, &path, tag) {
            log::warn!("Failed to store execute command result in command storage: {error}");
        }
    });
    context.chain_result_callback(callback);
    Ok(CommandResult::success())
}

fn store_entity_data(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    data_type: StoreDataType,
    store_result: bool,
) -> Result<CommandResult, CommandError> {
    let target = single_entity(context, arguments, "target")?;
    let path = nbt_path(arguments)?;
    let scale = double(arguments, "scale")?;
    let callback = CommandResultCallback::new(move |result| {
        let value = if store_result {
            result.result
        } else {
            i32::from(result.success)
        };
        let tag = data_type.tag(value, scale);
        if let Err(error) = store_entity_data_value(target.as_ref(), &path, tag) {
            log::warn!("Failed to store execute command result in entity data: {error}");
        }
    });
    context.chain_result_callback(callback);
    Ok(CommandResult::success())
}

pub(super) fn store_entity_data_value(
    entity: &dyn crate::entity::Entity,
    path: &NbtPath,
    value: NbtTag,
) -> Result<(), StoreEntityDataError> {
    if entity.as_player().is_some() {
        return Err(StoreEntityDataError::InvalidPlayer);
    }

    let mut tag = NbtTag::Compound(entity.nbt_for_data_compare());
    path.set(&mut tag, value).map_err(StoreEntityDataError::Path)?;
    let NbtTag::Compound(data) = tag else {
        return Err(StoreEntityDataError::ExpectedCompoundRoot);
    };

    entity
        .load_data_command_nbt(&data)
        .map_err(StoreEntityDataError::Load)?;
    Ok(())
}

#[derive(Debug)]
pub(super) enum StoreEntityDataError {
    InvalidPlayer,
    Path(NbtPathMutationError),
    ExpectedCompoundRoot,
    Load(crate::entity::EntityDataLoadError),
}

impl fmt::Display for StoreEntityDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPlayer => write!(f, "cannot modify player entity data"),
            Self::Path(error) => write!(f, "{error}"),
            Self::ExpectedCompoundRoot => write!(f, "NBT path mutation replaced the root compound"),
            Self::Load(error) => write!(f, "{error}"),
        }
    }
}

pub(super) fn store_storage_data_value(
    storage: &CommandStorage,
    id: &Identifier,
    path: &NbtPath,
    value: NbtTag,
) -> Result<(), StoreStorageDataError> {
    let mut tag = NbtTag::Compound(storage.get(id));
    path.set(&mut tag, value)
        .map_err(StoreStorageDataError::Path)?;
    let NbtTag::Compound(data) = tag else {
        return Err(StoreStorageDataError::ExpectedCompoundRoot);
    };

    storage.set(id.clone(), data);
    Ok(())
}

#[derive(Debug)]
pub(super) enum StoreStorageDataError {
    Path(NbtPathMutationError),
    ExpectedCompoundRoot,
}

impl fmt::Display for StoreStorageDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(error) => write!(f, "{error}"),
            Self::ExpectedCompoundRoot => write!(f, "NBT path mutation replaced the root compound"),
        }
    }
}

pub(super) fn store_score_value(
    scoreboard: &Scoreboard,
    holders: &[ScoreHolder],
    objective: &ScoreboardObjective,
    value: i32,
) -> Result<(), ScoreboardError> {
    for holder in holders {
        scoreboard.set_score(holder, objective, value)?;
    }
    Ok(())
}

fn store_block_data_value(
    world: &Arc<World>,
    pos: BlockPos,
    path: &NbtPath,
    value: NbtTag,
) -> Result<(), StoreBlockDataError> {
    let Some(block_entity) = world.get_block_entity(pos) else {
        return Err(StoreBlockDataError::MissingBlockEntity);
    };

    let mut block_entity = block_entity.lock();
    let mut tag = NbtTag::Compound(block_entity_full_nbt(&*block_entity));
    path.set(&mut tag, value)
        .map_err(StoreBlockDataError::Path)?;
    let NbtTag::Compound(data) = tag else {
        return Err(StoreBlockDataError::ExpectedCompoundRoot);
    };

    let mut nbt_bytes = Vec::new();
    data.write(&mut nbt_bytes);
    let borrowed = read_borrowed_compound(&mut Cursor::new(&nbt_bytes))
        .map_err(|_| StoreBlockDataError::InvalidWrittenNbt)?;
    block_entity.load_additional(&borrowed);
    block_entity.set_changed();

    if let Some(update_tag) = block_entity.get_update_tag() {
        world.broadcast_block_entity_update(pos, block_entity.get_type(), update_tag);
    }

    Ok(())
}

#[derive(Debug)]
enum StoreBlockDataError {
    MissingBlockEntity,
    Path(NbtPathMutationError),
    ExpectedCompoundRoot,
    InvalidWrittenNbt,
}

impl fmt::Display for StoreBlockDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingBlockEntity => write!(f, "block entity no longer exists"),
            Self::Path(error) => write!(f, "{error}"),
            Self::ExpectedCompoundRoot => write!(f, "NBT path mutation replaced the root compound"),
            Self::InvalidWrittenNbt => {
                write!(f, "mutated block entity NBT could not be reborrowed")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use simdnbt::owned::NbtTag;

    use super::StoreDataType;

    #[test]
    fn store_data_type_casts_match_java_narrowing_boundaries() {
        assert_eq!(StoreDataType::Byte.tag(128, 1.0), NbtTag::Byte(-128));
        assert_eq!(StoreDataType::Byte.tag(i32::MAX, 1e20), NbtTag::Byte(-1));
        assert_eq!(StoreDataType::Byte.tag(i32::MIN, 1e20), NbtTag::Byte(0));
        assert_eq!(
            StoreDataType::Short.tag(32_768, 1.0),
            NbtTag::Short(-32_768)
        );
        assert_eq!(StoreDataType::Int.tag(i32::MAX, 1e20), NbtTag::Int(i32::MAX));
        assert_eq!(
            StoreDataType::Long.tag(i32::MAX, 1e20),
            NbtTag::Long(i64::MAX)
        );
        assert_eq!(StoreDataType::Float.tag(3, 0.5), NbtTag::Float(1.5));
        assert_eq!(StoreDataType::Double.tag(3, 0.5), NbtTag::Double(1.5));
    }
}
