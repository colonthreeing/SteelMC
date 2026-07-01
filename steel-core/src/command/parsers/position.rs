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
    reader::{ARGUMENT_SEPARATOR, CommandReader},
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
        let error_cursor = reader.absolute_cursor();
        let argument_start = reader.cursor();

        if reader.peek() == Some('^') {
            let pos = parse_local_coordinates(
                reader,
                context,
                argument_start,
                error_cursor,
                CommandParseErrorKind::InvalidVec3,
            )?;

            return Ok(ParsedArgument::Vec3(pos));
        }

        let x = parse_world_double_coordinate(
            reader,
            true,
            argument_start,
            error_cursor,
            CommandParseErrorKind::InvalidVec3,
        )?;
        expect_coordinate_separator(
            reader,
            argument_start,
            error_cursor,
            CommandParseErrorKind::InvalidVec3,
        )?;
        let y = parse_world_double_coordinate(
            reader,
            false,
            argument_start,
            error_cursor,
            CommandParseErrorKind::InvalidVec3,
        )?;
        expect_coordinate_separator(
            reader,
            argument_start,
            error_cursor,
            CommandParseErrorKind::InvalidVec3,
        )?;
        let z = parse_world_double_coordinate(
            reader,
            true,
            argument_start,
            error_cursor,
            CommandParseErrorKind::InvalidVec3,
        )?;
        let Some(origin) = context.position() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("position"),
                error_cursor,
            ));
        };

        Ok(ParsedArgument::Vec3(DVec3::new(
            x.resolve(origin.x),
            y.resolve(origin.y),
            z.resolve(origin.z),
        )))
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
        let error_cursor = reader.absolute_cursor();
        let argument_start = reader.cursor();

        if reader.peek() == Some('^') {
            let pos = parse_local_coordinates(
                reader,
                context,
                argument_start,
                error_cursor,
                CommandParseErrorKind::InvalidBlockPos,
            )?;

            return Ok(ParsedArgument::BlockPos(BlockPos::containing(
                pos.x, pos.y, pos.z,
            )));
        }

        let x = parse_world_block_coordinate(
            reader,
            argument_start,
            error_cursor,
            CommandParseErrorKind::InvalidBlockPos,
        )?;
        expect_coordinate_separator(
            reader,
            argument_start,
            error_cursor,
            CommandParseErrorKind::InvalidBlockPos,
        )?;
        let y = parse_world_block_coordinate(
            reader,
            argument_start,
            error_cursor,
            CommandParseErrorKind::InvalidBlockPos,
        )?;
        expect_coordinate_separator(
            reader,
            argument_start,
            error_cursor,
            CommandParseErrorKind::InvalidBlockPos,
        )?;
        let z = parse_world_block_coordinate(
            reader,
            argument_start,
            error_cursor,
            CommandParseErrorKind::InvalidBlockPos,
        )?;
        let Some(origin) = context.position() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("position"),
                error_cursor,
            ));
        };

        Ok(ParsedArgument::BlockPos(BlockPos::containing(
            x.resolve(origin.x),
            y.resolve(origin.y),
            z.resolve(origin.z),
        )))
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
        let error_cursor = reader.absolute_cursor();
        let argument_start = reader.cursor();

        if !reader.can_read() {
            return Err(coordinate_error(
                reader,
                argument_start,
                error_cursor,
                CommandParseErrorKind::InvalidRotation,
            ));
        }

        let yaw = parse_rotation_coordinate(
            reader,
            argument_start,
            error_cursor,
            CommandParseErrorKind::InvalidRotation,
        )?;
        expect_coordinate_separator(
            reader,
            argument_start,
            error_cursor,
            CommandParseErrorKind::InvalidRotation,
        )?;
        let pitch = parse_rotation_coordinate(
            reader,
            argument_start,
            error_cursor,
            CommandParseErrorKind::InvalidRotation,
        )?;
        let (origin_yaw, origin_pitch) = context.rotation().unwrap_or((0.0, 0.0));

        Ok(ParsedArgument::Rotation(normalize_rotation((
            yaw.resolve(origin_yaw),
            pitch.resolve(origin_pitch),
        ))))
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
        .expect_argument_separator()
        .map_err(|_| coordinate_error(reader, argument_start, error_cursor, error_kind))
}

fn coordinate_error(
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

fn numeric_coordinate_error(
    reader: &CommandReader<'_>,
    argument_start: usize,
    number_start: usize,
    error_cursor: usize,
    error_kind: fn(String) -> CommandParseErrorKind,
) -> CommandParseError {
    let end = reader
        .cursor()
        .max(next_coordinate_separator(reader, number_start));
    let raw = reader.input()[argument_start..end].trim_end().to_owned();
    CommandParseError::new(error_kind(raw), error_cursor)
}

fn next_coordinate_separator(reader: &CommandReader<'_>, start: usize) -> usize {
    let Some((offset, _)) = reader.input()[start..]
        .char_indices()
        .find(|(_, ch)| *ch == ARGUMENT_SEPARATOR)
    else {
        return reader.input().len();
    };

    start + offset
}

#[derive(Clone, Copy, Debug)]
struct ParsedCoordinate<T> {
    relative: bool,
    value: T,
}

impl ParsedCoordinate<f64> {
    fn resolve(self, origin: f64) -> f64 {
        if self.relative {
            origin + self.value
        } else {
            self.value
        }
    }
}

impl ParsedCoordinate<f32> {
    fn resolve(self, origin: f32) -> f32 {
        if self.relative {
            origin + self.value
        } else {
            self.value
        }
    }
}

fn parse_world_double_coordinate(
    reader: &mut CommandReader<'_>,
    center: bool,
    argument_start: usize,
    error_cursor: usize,
    error_kind: fn(String) -> CommandParseErrorKind,
) -> Result<ParsedCoordinate<f64>, CommandParseError> {
    if reader.peek() == Some('^') || !reader.can_read() {
        return Err(coordinate_error(
            reader,
            argument_start,
            error_cursor,
            error_kind,
        ));
    }

    let relative = read_relative_prefix(reader);
    let number_start = reader.cursor();
    let value = if reader.can_read() && !reader.is_argument_separator() {
        reader.read_f64().map_err(|_| {
            numeric_coordinate_error(
                reader,
                argument_start,
                number_start,
                error_cursor,
                error_kind,
            )
        })?
    } else {
        0.0
    };
    let number = &reader.input()[number_start..reader.cursor()];
    let value = if !relative && center && !number.contains('.') {
        value + 0.5
    } else {
        value
    };

    Ok(ParsedCoordinate { relative, value })
}

fn parse_world_block_coordinate(
    reader: &mut CommandReader<'_>,
    argument_start: usize,
    error_cursor: usize,
    error_kind: fn(String) -> CommandParseErrorKind,
) -> Result<ParsedCoordinate<f64>, CommandParseError> {
    if reader.peek() == Some('^') || !reader.can_read() {
        return Err(coordinate_error(
            reader,
            argument_start,
            error_cursor,
            error_kind,
        ));
    }

    let relative = read_relative_prefix(reader);
    let number_start = reader.cursor();
    let value = if reader.can_read() && !reader.is_argument_separator() {
        if relative {
            reader.read_f64().map_err(|_| {
                numeric_coordinate_error(
                    reader,
                    argument_start,
                    number_start,
                    error_cursor,
                    error_kind,
                )
            })?
        } else {
            f64::from(reader.read_i32().map_err(|_| {
                numeric_coordinate_error(
                    reader,
                    argument_start,
                    number_start,
                    error_cursor,
                    error_kind,
                )
            })?)
        }
    } else {
        0.0
    };

    Ok(ParsedCoordinate { relative, value })
}

fn parse_local_coordinates(
    reader: &mut CommandReader<'_>,
    context: &dyn CommandInputContext,
    argument_start: usize,
    error_cursor: usize,
    error_kind: fn(String) -> CommandParseErrorKind,
) -> Result<DVec3, CommandParseError> {
    let left = parse_local_coordinate(reader, argument_start, error_cursor, error_kind)?;
    expect_coordinate_separator(reader, argument_start, error_cursor, error_kind)?;
    let up = parse_local_coordinate(reader, argument_start, error_cursor, error_kind)?;
    expect_coordinate_separator(reader, argument_start, error_cursor, error_kind)?;
    let forwards = parse_local_coordinate(reader, argument_start, error_cursor, error_kind)?;
    let Some(source) = anchor_position(context) else {
        return Err(coordinate_error(
            reader,
            argument_start,
            error_cursor,
            error_kind,
        ));
    };
    let rotation = context.rotation().unwrap_or((0.0, 0.0));

    Ok(local_coordinates_to_anchor_position(
        source, rotation, left, up, forwards,
    ))
}

fn parse_local_coordinate(
    reader: &mut CommandReader<'_>,
    argument_start: usize,
    error_cursor: usize,
    error_kind: fn(String) -> CommandParseErrorKind,
) -> Result<f64, CommandParseError> {
    if reader.peek() != Some('^') {
        return Err(coordinate_error(
            reader,
            argument_start,
            error_cursor,
            error_kind,
        ));
    }

    let _ = reader.read();
    if !reader.can_read() || reader.is_argument_separator() {
        return Ok(0.0);
    }

    let number_start = reader.cursor();
    reader.read_f64().map_err(|_| {
        numeric_coordinate_error(
            reader,
            argument_start,
            number_start,
            error_cursor,
            error_kind,
        )
    })
}

fn parse_rotation_coordinate(
    reader: &mut CommandReader<'_>,
    argument_start: usize,
    error_cursor: usize,
    error_kind: fn(String) -> CommandParseErrorKind,
) -> Result<ParsedCoordinate<f32>, CommandParseError> {
    if reader.peek() == Some('^') || !reader.can_read() {
        return Err(coordinate_error(
            reader,
            argument_start,
            error_cursor,
            error_kind,
        ));
    }

    let relative = read_relative_prefix(reader);
    let number_start = reader.cursor();
    let value = if reader.can_read() && !reader.is_argument_separator() {
        reader.read_f32().map_err(|_| {
            numeric_coordinate_error(
                reader,
                argument_start,
                number_start,
                error_cursor,
                error_kind,
            )
        })?
    } else {
        0.0
    };

    Ok(ParsedCoordinate { relative, value })
}

fn read_relative_prefix(reader: &mut CommandReader<'_>) -> bool {
    if reader.peek() != Some('~') {
        return false;
    }

    let _ = reader.read();
    true
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
