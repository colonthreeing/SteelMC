//! Vanilla-compatible NBT helpers.

mod path;
mod snbt;

use simdnbt::owned::{NbtCompound, NbtList, NbtTag};

pub use path::{
    NbtPath, NbtPathError, NbtPathMutationError, parse_nbt_path, parse_nbt_path_argument,
};
pub use snbt::{
    SnbtError, parse_snbt, parse_snbt_argument, parse_snbt_compound, parse_snbt_compound_argument,
};

/// Mirrors vanilla `NbtUtils.compareNbt`.
///
/// `expected == None` is treated as a wildcard. Compound tags are partial maps:
/// every entry in `expected` must be present and recursively match in `actual`.
/// When `partial_list_matches` is true, each expected list entry may match any
/// actual list entry, in any order.
#[must_use]
pub fn compare_nbt(
    expected: Option<&NbtTag>,
    actual: Option<&NbtTag>,
    partial_list_matches: bool,
) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    let Some(actual) = actual else {
        return false;
    };

    match (expected, actual) {
        (NbtTag::Compound(expected), NbtTag::Compound(actual)) => {
            compare_compounds(expected, actual, partial_list_matches)
        }
        (NbtTag::List(expected), NbtTag::List(actual)) if partial_list_matches => {
            compare_lists_partially(expected, actual)
        }
        _ => expected == actual,
    }
}

/// Compares two compound tags with vanilla `NbtUtils.compareNbt` semantics.
#[must_use]
pub fn compare_nbt_compounds(
    expected: &NbtCompound,
    actual: &NbtCompound,
    partial_list_matches: bool,
) -> bool {
    compare_compounds(expected, actual, partial_list_matches)
}

fn compare_compounds(
    expected: &NbtCompound,
    actual: &NbtCompound,
    partial_list_matches: bool,
) -> bool {
    if actual.len() < expected.len() {
        return false;
    }

    expected.iter().all(|(key, expected_tag)| {
        compare_nbt(
            Some(expected_tag),
            actual.get(&key.to_str()),
            partial_list_matches,
        )
    })
}

fn compare_lists_partially(expected: &NbtList, actual: &NbtList) -> bool {
    let expected_tags = list_as_tags(expected);
    let actual_tags = list_as_tags(actual);
    if expected_tags.is_empty() {
        return actual_tags.is_empty();
    }
    if actual_tags.len() < expected_tags.len() {
        return false;
    }

    expected_tags.iter().all(|expected_tag| {
        actual_tags
            .iter()
            .any(|actual_tag| compare_nbt(Some(expected_tag), Some(actual_tag), true))
    })
}

fn list_as_tags(list: &NbtList) -> Vec<NbtTag> {
    list.as_nbt_tags()
        .into_iter()
        .map(unwrap_list_wrapper)
        .collect()
}

fn unwrap_list_wrapper(tag: NbtTag) -> NbtTag {
    match tag {
        NbtTag::Compound(mut compound) if compound.len() == 1 && compound.contains("") => {
            let Some(value) = compound.take("") else {
                return NbtTag::Compound(compound);
            };
            value
        }
        tag => tag,
    }
}

#[cfg(test)]
mod tests {
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
    fn absent_expected_is_wildcard() {
        assert!(compare_nbt(None, None, true));
        assert!(compare_nbt(None, Some(&NbtTag::Int(1)), true));
    }

    #[test]
    fn present_expected_requires_actual() {
        assert!(!compare_nbt(Some(&NbtTag::Int(1)), None, true));
    }

    #[test]
    fn compounds_match_partially() {
        let expected = compound([("id", NbtTag::String("minecraft:chest".into()))]);
        let actual = compound([
            ("id", NbtTag::String("minecraft:chest".into())),
            ("x", NbtTag::Int(12)),
        ]);
        assert!(compare_nbt(Some(&expected), Some(&actual), true));
    }

    #[test]
    fn compounds_require_matching_entries() {
        let expected = compound([("id", NbtTag::String("minecraft:barrel".into()))]);
        let actual = compound([("id", NbtTag::String("minecraft:chest".into()))]);
        assert!(!compare_nbt(Some(&expected), Some(&actual), true));
    }

    #[test]
    fn partial_lists_match_any_order() {
        let expected = list([
            compound([("name", NbtTag::String("second".into()))]),
            compound([("name", NbtTag::String("first".into()))]),
        ]);
        let actual = list([
            compound([
                ("name", NbtTag::String("first".into())),
                ("extra", NbtTag::Byte(1)),
            ]),
            compound([("name", NbtTag::String("second".into()))]),
            compound([("name", NbtTag::String("third".into()))]),
        ]);

        assert!(compare_nbt(Some(&expected), Some(&actual), true));
        assert!(!compare_nbt(Some(&expected), Some(&actual), false));
    }

    #[test]
    fn partial_lists_match_vanilla_wrapped_heterogeneous_values() {
        let expected = list([NbtTag::String("two".into())]);
        let actual = list([NbtTag::Int(1), NbtTag::String("two".into())]);

        assert!(compare_nbt(Some(&expected), Some(&actual), true));
    }

    #[test]
    fn empty_partial_list_only_matches_empty_list() {
        let expected = list([]);
        let empty = list([]);
        let non_empty = list([NbtTag::Int(1)]);

        assert!(compare_nbt(Some(&expected), Some(&empty), true));
        assert!(!compare_nbt(Some(&expected), Some(&non_empty), true));
    }

    #[test]
    fn scalars_use_exact_tag_equality() {
        assert!(compare_nbt(
            Some(&NbtTag::Int(1)),
            Some(&NbtTag::Int(1)),
            true,
        ));
        assert!(!compare_nbt(
            Some(&NbtTag::Int(1)),
            Some(&NbtTag::Long(1)),
            true,
        ));
    }
}
