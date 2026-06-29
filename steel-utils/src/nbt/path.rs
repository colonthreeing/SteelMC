use std::{error::Error, fmt};

use simdnbt::owned::{NbtCompound, NbtTag};

use super::{compare_nbt, parse_snbt_compound_argument};

/// Error returned when parsing an NBT path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NbtPathError {
    cursor: usize,
    message: String,
}

impl NbtPathError {
    fn new(cursor: usize, message: impl Into<String>) -> Self {
        Self {
            cursor,
            message: message.into(),
        }
    }

    /// Returns the byte cursor where parsing failed.
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// Returns the parse failure message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for NbtPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "NBT path parse error at byte {}: {}",
            self.cursor, self.message
        )
    }
}

impl Error for NbtPathError {}

/// Parsed vanilla NBT path.
#[derive(Clone, Debug, PartialEq)]
pub struct NbtPath {
    original: String,
    nodes: Vec<NbtPathNode>,
}

impl NbtPath {
    /// Returns the path as written in command input.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.original
    }

    /// Returns cloned tags selected by this path.
    #[must_use]
    pub fn get(&self, tag: &NbtTag) -> Vec<NbtTag> {
        let mut tags = vec![tag.clone()];
        for node in &self.nodes {
            tags = node.get(&tags);
            if tags.is_empty() {
                break;
            }
        }
        tags
    }

    /// Returns the number of tags matched by this path.
    #[must_use]
    pub fn count_matching(&self, tag: &NbtTag) -> usize {
        let mut tags = vec![tag.clone()];
        for node in &self.nodes {
            tags = node.get(&tags);
            if tags.is_empty() {
                return 0;
            }
        }
        tags.len()
    }
}

/// Parses one complete NBT path.
///
/// # Errors
///
/// Returns an error when the input is not a valid NBT path or has trailing data.
pub fn parse_nbt_path(input: &str) -> Result<NbtPath, NbtPathError> {
    let (path, cursor) = parse_nbt_path_argument(input)?;
    if cursor != input.len() {
        return Err(NbtPathError::new(cursor, "trailing data"));
    }
    Ok(path)
}

/// Parses one NBT path and returns the byte cursor consumed by it.
///
/// # Errors
///
/// Returns an error when the input does not start with a valid NBT path.
pub fn parse_nbt_path_argument(input: &str) -> Result<(NbtPath, usize), NbtPathError> {
    let mut parser = Parser::new(input);
    let path = parser.parse()?;
    Ok((path, parser.cursor))
}

#[derive(Clone, Debug, PartialEq)]
enum NbtPathNode {
    CompoundChild(String),
    MatchObject { name: String, pattern: NbtCompound },
    MatchRootObject(NbtCompound),
    AllElements,
    IndexedElement(i32),
    MatchElement(NbtCompound),
}

impl NbtPathNode {
    fn get(&self, input: &[NbtTag]) -> Vec<NbtTag> {
        let mut output = Vec::new();
        for tag in input {
            match self {
                Self::CompoundChild(name) => {
                    if let NbtTag::Compound(compound) = tag
                        && let Some(child) = compound.get(name)
                    {
                        output.push(child.clone());
                    }
                }
                Self::MatchObject { name, pattern } => {
                    if let NbtTag::Compound(compound) = tag
                        && let Some(child) = compound.get(name)
                        && compound_pattern_matches(pattern, child)
                    {
                        output.push(child.clone());
                    }
                }
                Self::MatchRootObject(pattern) => {
                    if compound_pattern_matches(pattern, tag) {
                        output.push(tag.clone());
                    }
                }
                Self::AllElements => {
                    output.extend(collection_elements(tag));
                }
                Self::IndexedElement(index) => {
                    let elements = collection_elements(tag);
                    if let Some(element) = indexed_element(&elements, *index) {
                        output.push(element);
                    }
                }
                Self::MatchElement(pattern) => {
                    output.extend(
                        collection_elements(tag)
                            .into_iter()
                            .filter(|tag| compound_pattern_matches(pattern, tag)),
                    );
                }
            }
        }
        output
    }
}

fn compound_pattern_matches(pattern: &NbtCompound, tag: &NbtTag) -> bool {
    compare_nbt(Some(&NbtTag::Compound(pattern.clone())), Some(tag), true)
}

fn indexed_element(elements: &[NbtTag], index: i32) -> Option<NbtTag> {
    let actual_index = if index < 0 {
        elements.len().checked_add_signed(index as isize)?
    } else {
        usize::try_from(index).ok()?
    };
    elements.get(actual_index).cloned()
}

fn collection_elements(tag: &NbtTag) -> Vec<NbtTag> {
    match tag {
        NbtTag::List(list) => list.as_nbt_tags(),
        NbtTag::ByteArray(values) => values
            .iter()
            .map(|value| NbtTag::Byte(*value as i8))
            .collect(),
        NbtTag::IntArray(values) => values.iter().copied().map(NbtTag::Int).collect(),
        NbtTag::LongArray(values) => values.iter().copied().map(NbtTag::Long).collect(),
        _ => Vec::new(),
    }
}

struct Parser<'a> {
    input: &'a str,
    cursor: usize,
}

impl<'a> Parser<'a> {
    const fn new(input: &'a str) -> Self {
        Self { input, cursor: 0 }
    }

    fn parse(&mut self) -> Result<NbtPath, NbtPathError> {
        let start = self.cursor;
        let mut nodes = Vec::new();
        let mut first_node = true;

        while self.can_read() && self.peek() != Some(' ') {
            nodes.push(self.parse_node(first_node)?);
            first_node = false;

            if self.can_read() {
                let Some(next) = self.peek() else {
                    break;
                };
                if next != ' ' && next != '[' && next != '{' {
                    self.expect_char('.')?;
                }
            }
        }

        if nodes.is_empty() {
            return Err(self.error("expected NBT path"));
        }

        Ok(NbtPath {
            original: self.input[start..self.cursor].to_owned(),
            nodes,
        })
    }

    fn parse_node(&mut self, first_node: bool) -> Result<NbtPathNode, NbtPathError> {
        match self.peek() {
            Some('"' | '\'') => {
                let name = self.parse_quoted_string()?;
                self.read_object_node(name)
            }
            Some('[') => self.parse_element_node(),
            Some('{') => {
                if !first_node {
                    return Err(self.error("invalid NBT path node"));
                }
                let pattern = self.parse_compound_pattern()?;
                Ok(NbtPathNode::MatchRootObject(pattern))
            }
            Some(_) => {
                let name = self.parse_unquoted_name()?;
                self.read_object_node(name)
            }
            None => Err(self.error("expected NBT path node")),
        }
    }

    fn read_object_node(&mut self, name: String) -> Result<NbtPathNode, NbtPathError> {
        if name.is_empty() {
            return Err(self.error("expected NBT path node"));
        }
        if self.peek() == Some('{') {
            let pattern = self.parse_compound_pattern()?;
            Ok(NbtPathNode::MatchObject { name, pattern })
        } else {
            Ok(NbtPathNode::CompoundChild(name))
        }
    }

    fn parse_element_node(&mut self) -> Result<NbtPathNode, NbtPathError> {
        self.expect_char('[')?;
        match self.peek() {
            Some('{') => {
                let pattern = self.parse_compound_pattern()?;
                self.expect_char(']')?;
                Ok(NbtPathNode::MatchElement(pattern))
            }
            Some(']') => {
                self.read();
                Ok(NbtPathNode::AllElements)
            }
            _ => {
                let index = self.parse_i32()?;
                self.expect_char(']')?;
                Ok(NbtPathNode::IndexedElement(index))
            }
        }
    }

    fn parse_compound_pattern(&mut self) -> Result<NbtCompound, NbtPathError> {
        let start = self.cursor;
        let (compound, consumed) =
            parse_snbt_compound_argument(&self.input[start..]).map_err(|error| {
                NbtPathError::new(start + error.cursor(), error.message().to_owned())
            })?;
        self.cursor += consumed;
        Ok(compound)
    }

    fn parse_unquoted_name(&mut self) -> Result<String, NbtPathError> {
        let start = self.cursor;
        while self.peek().is_some_and(is_allowed_in_unquoted_name) {
            self.read();
        }
        if self.cursor == start {
            return Err(self.error("invalid NBT path node"));
        }
        Ok(self.input[start..self.cursor].to_owned())
    }

    fn parse_quoted_string(&mut self) -> Result<String, NbtPathError> {
        let Some(terminator) = self.peek().filter(|ch| matches!(ch, '"' | '\'')) else {
            return Err(self.error("expected quoted string"));
        };
        let quote_cursor = self.cursor;
        self.read();

        let mut value = String::new();
        while let Some(ch) = self.read() {
            match ch {
                ch if ch == terminator => return Ok(value),
                '\\' => {
                    let escaped = self
                        .read()
                        .ok_or_else(|| self.error_at(quote_cursor, "unclosed quoted string"))?;
                    if escaped != terminator && escaped != '\\' {
                        return Err(self.error(format!("invalid escape '{escaped}'")));
                    }
                    value.push(escaped);
                }
                _ => value.push(ch),
            }
        }

        Err(self.error_at(quote_cursor, "unclosed quoted string"))
    }

    fn parse_i32(&mut self) -> Result<i32, NbtPathError> {
        let start = self.cursor;
        if self.peek() == Some('-') {
            self.read();
        }
        let digit_start = self.cursor;
        while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
            self.read();
        }
        if self.cursor == digit_start {
            return Err(self.error_at(start, "expected list index"));
        }
        self.input[start..self.cursor]
            .parse()
            .map_err(|_| self.error_at(start, "invalid list index"))
    }

    const fn can_read(&self) -> bool {
        self.cursor < self.input.len()
    }

    fn peek(&self) -> Option<char> {
        self.input[self.cursor..].chars().next()
    }

    fn read(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.cursor += ch.len_utf8();
        Some(ch)
    }

    fn expect_char(&mut self, expected: char) -> Result<(), NbtPathError> {
        if self.peek() == Some(expected) {
            self.read();
            Ok(())
        } else {
            Err(self.error(format!("expected '{expected}'")))
        }
    }

    fn error(&self, message: impl Into<String>) -> NbtPathError {
        NbtPathError::new(self.cursor, message)
    }

    fn error_at(&self, cursor: usize, message: impl Into<String>) -> NbtPathError {
        NbtPathError::new(cursor, message)
    }
}

fn is_allowed_in_unquoted_name(ch: char) -> bool {
    !matches!(ch, ' ' | '"' | '\'' | '[' | ']' | '.' | '{' | '}')
}

#[cfg(test)]
mod tests {
    use simdnbt::owned::{NbtCompound, NbtList, NbtTag};

    use super::*;

    fn compound(entries: impl IntoIterator<Item = (&'static str, NbtTag)>) -> NbtTag {
        let mut compound = NbtCompound::new();
        for (key, tag) in entries {
            compound.insert(key, tag);
        }
        NbtTag::Compound(compound)
    }

    fn list(entries: impl IntoIterator<Item = NbtTag>) -> NbtTag {
        NbtTag::List(NbtList::from(entries.into_iter().collect::<Vec<_>>()))
    }

    #[test]
    fn parses_path_argument_without_consuming_separator() {
        let (path, cursor) = parse_nbt_path_argument("foo.bar run").expect("path parses");

        assert_eq!(path.as_str(), "foo.bar");
        assert_eq!(cursor, 7);
    }

    #[test]
    fn counts_compound_child_matches() {
        let path = parse_nbt_path("foo.bar").expect("path parses");
        let tag = compound([("foo", compound([("bar", NbtTag::Int(3))]))]);

        assert_eq!(path.count_matching(&tag), 1);
        assert_eq!(path.get(&tag), vec![NbtTag::Int(3)]);
    }

    #[test]
    fn counts_list_index_and_wildcard_matches() {
        let path = parse_nbt_path("items[1].id").expect("path parses");
        let wildcard = parse_nbt_path("items[].id").expect("path parses");
        let tag = compound([(
            "items",
            list([
                compound([("id", NbtTag::String("first".into()))]),
                compound([("id", NbtTag::String("second".into()))]),
            ]),
        )]);

        assert_eq!(path.get(&tag), vec![NbtTag::String("second".into())]);
        assert_eq!(wildcard.count_matching(&tag), 2);
    }

    #[test]
    fn negative_indices_select_from_end() {
        let path = parse_nbt_path("items[-1]").expect("path parses");
        let tag = compound([(
            "items",
            list([NbtTag::Int(1), NbtTag::Int(2), NbtTag::Int(3)]),
        )]);

        assert_eq!(path.get(&tag), vec![NbtTag::Int(3)]);
    }

    #[test]
    fn predicate_nodes_use_partial_compound_matching() {
        let path = parse_nbt_path("items[{id:\"minecraft:stone\"}].Count").expect("path parses");
        let tag = compound([(
            "items",
            list([
                compound([
                    ("id", NbtTag::String("minecraft:dirt".into())),
                    ("Count", NbtTag::Byte(1)),
                ]),
                compound([
                    ("id", NbtTag::String("minecraft:stone".into())),
                    ("Count", NbtTag::Byte(4)),
                    ("Slot", NbtTag::Byte(0)),
                ]),
            ]),
        )]);

        assert_eq!(path.get(&tag), vec![NbtTag::Byte(4)]);
    }

    #[test]
    fn root_predicate_matches_root_compound() {
        let path = parse_nbt_path("{id:\"minecraft:barrel\"}").expect("path parses");
        let tag = compound([
            ("id", NbtTag::String("minecraft:barrel".into())),
            ("x", NbtTag::Int(4)),
        ]);

        assert_eq!(path.count_matching(&tag), 1);
    }
}
