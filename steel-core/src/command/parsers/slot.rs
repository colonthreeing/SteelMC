//! Slot command argument parsers.

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry};

use crate::command::{
    graph::{
        CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
        CommandParseErrorKind, ItemSlotRangeArgumentValue, ParsedArgument, ParsedArguments,
    },
    reader::CommandReader,
    requirement::CommandInputContext,
};

/// Item slot range argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct ItemSlotsParser;

impl CommandArgumentParser for ItemSlotsParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_token()?;
        let Some(slots) = item_slot_range(&raw) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidItemSlot(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::ItemSlots(slots))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::ItemSlots, None)
    }

    fn parsed_type(&self) -> &'static str {
        "item_slots"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        item_slot_names()
            .into_iter()
            .filter(|name| name.starts_with(prefix))
            .map(SuggestionEntry::new)
            .collect()
    }
}

fn item_slot_range(name: &str) -> Option<ItemSlotRangeArgumentValue> {
    if name == "contents" {
        return Some(slot_range(name, [0]));
    }

    if let Some(range) = prefixed_range(name, "container.", 0, 54) {
        return Some(range);
    }
    if let Some(range) = prefixed_range(name, "hotbar.", 0, 9) {
        return Some(range);
    }
    if let Some(range) = prefixed_range(name, "inventory.", 9, 27) {
        return Some(range);
    }
    if let Some(range) = prefixed_range(name, "enderchest.", 200, 27) {
        return Some(range);
    }
    if let Some(range) = prefixed_range(name, "mob.inventory.", 300, 8) {
        return Some(range);
    }
    if let Some(range) = prefixed_range(name, "horse.", 500, 15) {
        return Some(range);
    }
    if let Some(range) = prefixed_range(name, "player.crafting.", 500, 4) {
        return Some(range);
    }

    match name {
        "weapon" | "weapon.mainhand" => Some(slot_range(name, [98])),
        "weapon.offhand" => Some(slot_range(name, [99])),
        "weapon.*" => Some(slot_range(name, [98, 99])),
        "armor.head" => Some(slot_range(name, [103])),
        "armor.chest" => Some(slot_range(name, [102])),
        "armor.legs" => Some(slot_range(name, [101])),
        "armor.feet" => Some(slot_range(name, [100])),
        "armor.body" => Some(slot_range(name, [105])),
        "armor.*" => Some(slot_range(name, [103, 102, 101, 100, 105])),
        "saddle" => Some(slot_range(name, [106])),
        "horse.chest" | "player.cursor" => Some(slot_range(name, [499])),
        _ => None,
    }
}

fn prefixed_range(
    name: &str,
    prefix: &'static str,
    offset: i32,
    size: i32,
) -> Option<ItemSlotRangeArgumentValue> {
    let suffix = name.strip_prefix(prefix)?;
    if suffix == "*" {
        return Some(ItemSlotRangeArgumentValue::new(
            name,
            (offset..offset + size).collect(),
        ));
    }

    let index = suffix.parse::<i32>().ok()?;
    if !(0..size).contains(&index) {
        return None;
    }

    Some(slot_range(name, [offset + index]))
}

fn slot_range<const N: usize>(name: &str, slots: [i32; N]) -> ItemSlotRangeArgumentValue {
    ItemSlotRangeArgumentValue::new(name, slots.into())
}

fn item_slot_names() -> Vec<String> {
    let mut names = Vec::new();
    names.push("contents".to_owned());
    push_prefixed_names(&mut names, "container.", 54);
    push_prefixed_names(&mut names, "hotbar.", 9);
    push_prefixed_names(&mut names, "inventory.", 27);
    push_prefixed_names(&mut names, "enderchest.", 27);
    push_prefixed_names(&mut names, "mob.inventory.", 8);
    push_prefixed_names(&mut names, "horse.", 15);
    names.extend([
        "weapon".to_owned(),
        "weapon.mainhand".to_owned(),
        "weapon.offhand".to_owned(),
        "weapon.*".to_owned(),
        "armor.head".to_owned(),
        "armor.chest".to_owned(),
        "armor.legs".to_owned(),
        "armor.feet".to_owned(),
        "armor.body".to_owned(),
        "armor.*".to_owned(),
        "saddle".to_owned(),
        "horse.chest".to_owned(),
        "player.cursor".to_owned(),
    ]);
    push_prefixed_names(&mut names, "player.crafting.", 4);
    names
}

fn push_prefixed_names(names: &mut Vec<String>, prefix: &str, size: i32) {
    for index in 0..size {
        names.push(format!("{prefix}{index}"));
    }
    names.push(format!("{prefix}*"));
}
