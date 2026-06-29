//! Position and rotation command argument parsers.

use std::f32::consts::PI;

use glam::DVec3;
use steel_protocol::packets::game::{ArgumentType, SuggestionEntry};
use steel_utils::BlockPos;

use crate::chunk::heightmap::HeightmapType;
use crate::command::{
    context::anchored_position,
    graph::{
        CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
        CommandParseErrorKind, ParsedArgument, ParsedArguments,
    },
    reader::CommandReader,
    requirement::CommandInputContext,
};

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
        let start = reader.cursor();
        let x = reader.read_token()?;
        expect_coordinate_separator(reader, start, cursor, CommandParseErrorKind::InvalidVec3)?;
        let y = read_coordinate_token(reader, start, cursor, CommandParseErrorKind::InvalidVec3)?;
        expect_coordinate_separator(reader, start, cursor, CommandParseErrorKind::InvalidVec3)?;
        let z = read_coordinate_token(reader, start, cursor, CommandParseErrorKind::InvalidVec3)?;
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
        let start = reader.cursor();
        let x = reader.read_token()?;
        expect_coordinate_separator(
            reader,
            start,
            cursor,
            CommandParseErrorKind::InvalidBlockPos,
        )?;
        let y = read_coordinate_token(
            reader,
            start,
            cursor,
            CommandParseErrorKind::InvalidBlockPos,
        )?;
        expect_coordinate_separator(
            reader,
            start,
            cursor,
            CommandParseErrorKind::InvalidBlockPos,
        )?;
        let z = read_coordinate_token(
            reader,
            start,
            cursor,
            CommandParseErrorKind::InvalidBlockPos,
        )?;
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

/// Heightmap type argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct HeightmapParser;

impl CommandArgumentParser for HeightmapParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_token()?;
        let Some(heightmap) = heightmap_type_from_name(&raw) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidHeightmap(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::Heightmap(heightmap))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::Heightmap, None)
    }

    fn parsed_type(&self) -> &'static str {
        "heightmap"
    }

    fn examples(&self) -> &'static [&'static str] {
        &["world_surface", "motion_blocking"]
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        const HEIGHTMAP_NAMES: &[&str] = &[
            "world_surface",
            "motion_blocking",
            "motion_blocking_no_leaves",
            "ocean_floor",
        ];

        let prefix = prefix.to_ascii_lowercase();
        HEIGHTMAP_NAMES
            .iter()
            .copied()
            .filter(|heightmap| heightmap.starts_with(&prefix))
            .map(SuggestionEntry::new)
            .collect()
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
        let start = reader.cursor();
        let yaw = reader.read_token()?;
        expect_coordinate_separator(
            reader,
            start,
            cursor,
            CommandParseErrorKind::InvalidRotation,
        )?;
        let pitch = read_coordinate_token(
            reader,
            start,
            cursor,
            CommandParseErrorKind::InvalidRotation,
        )?;
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

fn expect_coordinate_separator(
    reader: &mut CommandReader<'_>,
    argument_start: usize,
    error_cursor: usize,
    error_kind: fn(String) -> CommandParseErrorKind,
) -> Result<(), CommandParseError> {
    reader
        .expect_whitespace()
        .map_err(|_| incomplete_coordinate_error(reader, argument_start, error_cursor, error_kind))
}

fn read_coordinate_token(
    reader: &mut CommandReader<'_>,
    argument_start: usize,
    error_cursor: usize,
    error_kind: fn(String) -> CommandParseErrorKind,
) -> Result<String, CommandParseError> {
    reader.read_token().map_err(|error| {
        if reader.cursor() == argument_start {
            error
        } else {
            incomplete_coordinate_error(reader, argument_start, error_cursor, error_kind)
        }
    })
}

fn incomplete_coordinate_error(
    reader: &CommandReader<'_>,
    argument_start: usize,
    error_cursor: usize,
    error_kind: fn(String) -> CommandParseErrorKind,
) -> CommandParseError {
    let raw = reader.input()[argument_start..reader.cursor()]
        .trim_end()
        .to_owned();
    CommandParseError::new(error_kind(raw), error_cursor)
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
    Some(anchored_position(
        position,
        context.entity().map(AsRef::as_ref),
        context.anchor(),
    ))
}

fn heightmap_type_from_name(value: &str) -> Option<HeightmapType> {
    match value.to_ascii_lowercase().as_str() {
        "world_surface" => Some(HeightmapType::WorldSurface),
        "motion_blocking" => Some(HeightmapType::MotionBlocking),
        "motion_blocking_no_leaves" => Some(HeightmapType::MotionBlockingNoLeaves),
        "ocean_floor" => Some(HeightmapType::OceanFloor),
        _ => None,
    }
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
