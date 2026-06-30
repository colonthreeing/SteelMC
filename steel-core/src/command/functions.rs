//! Server command function registry.

use std::{collections::HashMap, sync::Arc};

use steel_utils::Identifier;

/// One registered command function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandFunction {
    id: Identifier,
    commands: Arc<[String]>,
}

impl CommandFunction {
    /// Creates a command function from parsed command lines.
    #[must_use]
    pub fn new(id: Identifier, commands: impl Into<Arc<[String]>>) -> Self {
        Self {
            id,
            commands: commands.into(),
        }
    }

    /// Returns this function's identifier.
    #[must_use]
    pub const fn id(&self) -> &Identifier {
        &self.id
    }

    /// Returns command lines in execution order.
    #[must_use]
    pub fn commands(&self) -> &[String] {
        &self.commands
    }
}

/// Server-level command function registry.
#[derive(Clone, Debug, Default)]
pub struct CommandFunctionRegistry {
    functions: HashMap<Identifier, CommandFunction>,
    tags: HashMap<Identifier, Vec<Identifier>>,
}

impl CommandFunctionRegistry {
    /// Registers or replaces one command function.
    pub fn insert_function(&mut self, function: CommandFunction) {
        self.functions.insert(function.id.clone(), function);
    }

    /// Registers or replaces a command function tag.
    pub fn insert_tag(&mut self, id: Identifier, functions: Vec<Identifier>) {
        self.tags.insert(id, functions);
    }

    /// Returns one command function by ID.
    #[must_use]
    pub fn function(&self, id: &Identifier) -> Option<CommandFunction> {
        self.functions.get(id).cloned()
    }

    /// Returns functions referenced by a tag, or an empty list for an unknown tag.
    #[must_use]
    pub fn tag(&self, id: &Identifier) -> Vec<CommandFunction> {
        self.tags.get(id).map_or_else(Vec::new, |functions| {
            functions
                .iter()
                .filter_map(|function| self.function(function))
                .collect()
        })
    }

    /// Returns registered function IDs.
    pub fn function_ids(&self) -> Vec<&Identifier> {
        sorted_ids(self.functions.keys())
    }

    /// Returns registered tag IDs.
    pub fn tag_ids(&self) -> Vec<&Identifier> {
        sorted_ids(self.tags.keys())
    }
}

fn sorted_ids<'a>(ids: impl Iterator<Item = &'a Identifier>) -> Vec<&'a Identifier> {
    let mut ids = ids.collect::<Vec<_>>();
    ids.sort_by_cached_key(|id| id.to_string());
    ids
}

#[cfg(test)]
mod tests {
    use steel_utils::Identifier;

    use super::{CommandFunction, CommandFunctionRegistry};

    #[test]
    fn registry_resolves_functions_and_tags() {
        let first = Identifier::new_static("test", "first");
        let second = Identifier::new_static("test", "second");
        let tag = Identifier::new_static("test", "load");
        let mut registry = CommandFunctionRegistry::default();

        registry.insert_function(CommandFunction::new(
            first.clone(),
            vec!["say one".to_owned()],
        ));
        registry.insert_function(CommandFunction::new(
            second.clone(),
            vec!["say two".to_owned()],
        ));
        registry.insert_tag(tag.clone(), vec![second.clone(), first.clone()]);

        assert_eq!(
            registry.function(&first).map(|function| function.id),
            Some(first)
        );
        assert_eq!(
            registry
                .tag(&tag)
                .into_iter()
                .map(|function| function.id)
                .collect::<Vec<_>>(),
            vec![second, Identifier::new_static("test", "first")]
        );
        assert!(
            registry
                .tag(&Identifier::new_static("test", "missing"))
                .is_empty()
        );
    }
}
