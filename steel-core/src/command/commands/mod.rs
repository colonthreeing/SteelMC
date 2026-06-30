//! Command implementations.

use text_components::TextComponent;

use crate::command::{
    CommandRegistration, CommandRegistrationError,
    error::CommandError,
    graph::{ItemPredicateMatchError, ParsedArgumentError},
};

mod permission_targets;

type RegistrationFactory = fn() -> Result<CommandRegistration, CommandRegistrationError>;

fn invalid_parsed_argument(error: ParsedArgumentError) -> CommandError {
    CommandError::InvalidConsumption(Some(format!("{error:?}")))
}

pub(in crate::command::commands) fn item_predicate_match_error(
    error: ItemPredicateMatchError,
) -> CommandError {
    let message = match error {
        ItemPredicateMatchError::MalformedCountPredicate => {
            "malformed minecraft:count item predicate".to_owned()
        }
        ItemPredicateMatchError::MalformedComponentPredicate(key) => {
            format!("malformed item component predicate '{key}'")
        }
        ItemPredicateMatchError::UnsupportedComponentValue(key) => {
            format!("unsupported item component value predicate '{key}'")
        }
        ItemPredicateMatchError::UnsupportedComponentPredicate(key) => {
            format!("unsupported item component predicate '{key}'")
        }
    };
    CommandError::failure(TextComponent::from(message))
}

include!(concat!(env!("OUT_DIR"), "/built_in_commands.rs"));

pub(super) fn registrations() -> Result<Vec<CommandRegistration>, CommandRegistrationError> {
    BUILT_IN_COMMANDS
        .iter()
        .map(|registration| registration())
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn generated_builtin_command_manifest_is_sorted_and_excludes_helpers() {
        assert_eq!(
            super::BUILT_IN_COMMAND_MODULES.len(),
            super::BUILT_IN_COMMANDS.len()
        );
        assert!(
            super::BUILT_IN_COMMAND_MODULES
                .windows(2)
                .all(|names| names[0] < names[1])
        );
        assert!(super::BUILT_IN_COMMAND_MODULES.contains(&"clear"));
        assert!(super::BUILT_IN_COMMAND_MODULES.contains(&"bossbar"));
        assert!(super::BUILT_IN_COMMAND_MODULES.contains(&"steelperms"));
        assert!(super::BUILT_IN_COMMAND_MODULES.contains(&"stopwatch"));
        assert!(super::BUILT_IN_COMMAND_MODULES.contains(&"xp"));
        assert!(!super::BUILT_IN_COMMAND_MODULES.contains(&"permission_targets"));
    }
}
