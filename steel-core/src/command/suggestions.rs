//! Shared command suggestion matching helpers.

/// Returns true when `pattern` matches `input` at the start or after a vanilla
/// suggestion word splitter.
#[must_use]
pub(crate) fn matches_suggestion_substr(pattern: &str, input: &str) -> bool {
    let pattern = pattern.to_lowercase();
    let input = input.to_lowercase();
    matches_suggestion_substr_lowercase(&pattern, &input)
}

fn matches_suggestion_substr_lowercase(pattern: &str, input: &str) -> bool {
    if pattern.is_empty() {
        return true;
    }

    let mut start = 0;
    loop {
        if input[start..].starts_with(pattern) {
            return true;
        }

        let Some(next_start) = next_splitter_end(input, start) else {
            return false;
        };
        start = next_start;
    }
}

fn next_splitter_end(input: &str, start: usize) -> Option<usize> {
    input[start..].char_indices().find_map(|(offset, ch)| {
        matches!(ch, '.' | '_' | '/').then_some(start + offset + ch.len_utf8())
    })
}

#[cfg(test)]
mod tests {
    use super::matches_suggestion_substr;

    #[test]
    fn suggestions_match_start_or_after_vanilla_splitters() {
        assert!(matches_suggestion_substr("oak", "oak_planks"));
        assert!(matches_suggestion_substr("planks", "oak_planks"));
        assert!(matches_suggestion_substr(
            "path",
            "minecraft:trial/chambers/path"
        ));
        assert!(matches_suggestion_substr("bar", "foo.bar"));
        assert!(!matches_suggestion_substr("anks", "oak_planks"));
    }

    #[test]
    fn suggestions_match_case_insensitively() {
        assert!(matches_suggestion_substr("Creative", "creative"));
        assert!(matches_suggestion_substr("PLANKS", "oak_planks"));
    }
}
