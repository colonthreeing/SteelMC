//! Position and rotation command argument parsers.

use std::f32::consts::PI;

use glam::DVec3;
use steel_protocol::packets::game::ArgumentType;
use steel_utils::BlockPos;

use crate::command::{
    context::EntityAnchor,
    graph::{
        CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
        CommandParseErrorKind, ParsedArgument,
    },
    reader::CommandReader,
    requirement::CommandInputContext,
};
use crate::entity::Entity;

/// 3D position argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct Vec3Parser;

impl CommandArgumentParser for Vec3Parser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let x = reader.read_token()?;
        reader.expect_whitespace()?;
        let y = reader.read_token()?;
        reader.expect_whitespace()?;
        let z = reader.read_token()?;
        let raw = format!("{x} {y} {z}");

        if x.starts_with('^') {
            let Some(pos) = parse_local_coordinates((&x, &y, &z), context) else {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidVec3(raw),
                    cursor,
                ));
            };

            return Ok(ParsedArgument::Vec3(pos));
        }

        let Some(origin) = context.position() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("position"),
                cursor,
            ));
        };
        let Some(x) = parse_vec3_coordinate::<false>(&x, origin.x) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidVec3(raw),
                cursor,
            ));
        };
        let Some(y) = parse_vec3_coordinate::<true>(&y, origin.y) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidVec3(raw),
                cursor,
            ));
        };
        let Some(z) = parse_vec3_coordinate::<false>(&z, origin.z) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidVec3(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::Vec3(DVec3::new(x, y, z)))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::Vec3, None)
    }

    fn parsed_type(&self) -> &'static str {
        "vec3"
    }
}

/// Block position argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct BlockPosParser;

impl CommandArgumentParser for BlockPosParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let x = reader.read_token()?;
        reader.expect_whitespace()?;
        let y = reader.read_token()?;
        reader.expect_whitespace()?;
        let z = reader.read_token()?;
        let raw = format!("{x} {y} {z}");

        if x.starts_with('^') {
            let Some(pos) = parse_local_coordinates((&x, &y, &z), context) else {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidBlockPos(raw),
                    cursor,
                ));
            };

            return Ok(ParsedArgument::BlockPos(BlockPos::containing(
                pos.x, pos.y, pos.z,
            )));
        }

        let Some(origin) = context.position() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("position"),
                cursor,
            ));
        };
        let Some(x) = parse_block_coordinate(&x, origin.x) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidBlockPos(raw),
                cursor,
            ));
        };
        let Some(y) = parse_block_coordinate(&y, origin.y) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidBlockPos(raw),
                cursor,
            ));
        };
        let Some(z) = parse_block_coordinate(&z, origin.z) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidBlockPos(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::BlockPos(BlockPos::containing(x, y, z)))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::BlockPos, None)
    }

    fn parsed_type(&self) -> &'static str {
        "block_pos"
    }
}

/// Rotation argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct RotationParser;

impl CommandArgumentParser for RotationParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let yaw = reader.read_token()?;
        reader.expect_whitespace()?;
        let pitch = reader.read_token()?;
        let raw = format!("{yaw} {pitch}");

        let (origin_yaw, origin_pitch) = context.rotation().unwrap_or((0.0, 0.0));
        let Some(yaw) = parse_rotation_coordinate(&yaw, origin_yaw) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidRotation(raw),
                cursor,
            ));
        };
        let Some(pitch) = parse_rotation_coordinate(&pitch, origin_pitch) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidRotation(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::Rotation(normalize_rotation((yaw, pitch))))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::Rotation, None)
    }

    fn parsed_type(&self) -> &'static str {
        "rotation"
    }
}

fn parse_block_coordinate(value: &str, origin: f64) -> Option<f64> {
    if value.starts_with('^') {
        return None;
    }

    if let Some(offset) = value.strip_prefix('~') {
        if offset.is_empty() {
            Some(origin)
        } else {
            Some(origin + offset.parse::<f64>().ok()?)
        }
    } else {
        Some(f64::from(value.parse::<i32>().ok()?))
    }
}

fn parse_vec3_coordinate<const IS_Y: bool>(value: &str, origin: f64) -> Option<f64> {
    if let Some(offset) = value.strip_prefix('~') {
        let offset = if offset.is_empty() {
            0.0
        } else {
            offset.parse().ok()?
        };
        return Some(origin + offset);
    }

    let mut parsed = value.parse::<f64>().ok()?;
    if !IS_Y && !value.contains('.') {
        parsed += 0.5;
    }

    Some(parsed)
}

fn parse_local_coordinates(
    coordinates: (&str, &str, &str),
    context: &dyn CommandInputContext,
) -> Option<DVec3> {
    let (left, up, forwards) = parse_local_coordinate_triplet(coordinates)?;
    let source = anchor_position(context)?;
    let rotation = context.rotation().unwrap_or((0.0, 0.0));

    Some(local_coordinates_to_anchor_position(
        source, rotation, left, up, forwards,
    ))
}

fn parse_local_coordinate_triplet(coordinates: (&str, &str, &str)) -> Option<(f64, f64, f64)> {
    let left = parse_local_coordinate(coordinates.0)?;
    let up = parse_local_coordinate(coordinates.1)?;
    let forwards = parse_local_coordinate(coordinates.2)?;
    Some((left, up, forwards))
}

fn parse_local_coordinate(value: &str) -> Option<f64> {
    let offset = value.strip_prefix('^')?;
    if offset.is_empty() {
        Some(0.0)
    } else {
        offset.parse::<f64>().ok()
    }
}

fn local_coordinates_to_anchor_position(
    source: DVec3,
    rotation: (f32, f32),
    left: f64,
    up: f64,
    forwards: f64,
) -> DVec3 {
    let (yaw, pitch) = rotation;
    let y_cos = ((yaw + 90.0) * PI / 180.0).cos();
    let y_sin = ((yaw + 90.0) * PI / 180.0).sin();
    let x_cos = (-pitch * PI / 180.0).cos();
    let x_sin = (-pitch * PI / 180.0).sin();
    let x_cos_up = ((-pitch + 90.0) * PI / 180.0).cos();
    let x_sin_up = ((-pitch + 90.0) * PI / 180.0).sin();
    let forwards_axis = DVec3::new(
        f64::from(y_cos * x_cos),
        f64::from(x_sin),
        f64::from(y_sin * x_cos),
    );
    let up_axis = DVec3::new(
        f64::from(y_cos * x_cos_up),
        f64::from(x_sin_up),
        f64::from(y_sin * x_cos_up),
    );
    let left_axis = -forwards_axis.cross(up_axis);

    source + left_axis * left + up_axis * up + forwards_axis * forwards
}

fn anchor_position(context: &dyn CommandInputContext) -> Option<DVec3> {
    let position = context.position()?;
    if matches!(context.anchor(), EntityAnchor::Eyes)
        && let Some(player) = context.player()
    {
        return Some(DVec3::new(position.x, player.get_eye_y(), position.z));
    }

    Some(position)
}

fn parse_rotation_coordinate(value: &str, origin: f32) -> Option<f32> {
    if value.starts_with('^') {
        return None;
    }

    if let Some(offset) = value.strip_prefix('~') {
        if offset.is_empty() {
            Some(origin)
        } else {
            Some(origin + offset.parse::<f32>().ok()?)
        }
    } else {
        value.parse::<f32>().ok()
    }
}

fn normalize_rotation((mut yaw, mut pitch): (f32, f32)) -> (f32, f32) {
    yaw = yaw.rem_euclid(360.0);
    if yaw >= 180.0 {
        yaw -= 360.0;
    }
    pitch = pitch.rem_euclid(360.0);
    if pitch >= 180.0 {
        pitch -= 360.0;
    }

    (yaw, pitch)
}
