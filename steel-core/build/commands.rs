//! Code generation for built-in command registration.
//!
//! Scans `src/command/commands/*.rs` for modules exposing
//! `pub(crate) const REGISTRATION` and `pub(crate) fn command()`, then generates
//! the built-in command module declarations plus registration factory list.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

pub fn build() -> String {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let commands_dir = PathBuf::from(&manifest_dir).join("src/command/commands");
    let pattern = commands_dir.join("*.rs");
    let pattern = pattern
        .to_str()
        .expect("command path should be valid UTF-8");

    let mut commands = Vec::new();
    for entry in glob::glob(pattern).unwrap_or_else(|_| panic!("Failed to glob {pattern}")) {
        let path = entry.expect("Failed to read command glob entry");
        let module_name = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or_else(|| panic!("Invalid command module path {}", path.display()));
        if module_name == "mod" {
            continue;
        }
        syn::parse_str::<syn::Ident>(module_name)
            .unwrap_or_else(|error| panic!("Invalid command module name '{module_name}': {error}"));
        let metadata = command_module_metadata(&path);
        if !metadata.has_registration_spec {
            assert!(
                !metadata.has_command_builder,
                "Command module {} exposes command() but has no pub(crate) REGISTRATION spec",
                path.display()
            );
            continue;
        }
        assert!(
            metadata.has_command_builder,
            "Command module {} has REGISTRATION but no crate-visible command() builder",
            path.display()
        );

        commands.push(CommandModule {
            name: module_name.to_owned(),
            path: path.to_string_lossy().into_owned(),
        });
    }

    commands.sort_by(|left, right| left.name.cmp(&right.name));

    let modules = commands
        .iter()
        .map(|command| {
            let name = &command.name;
            let path = &command.path;
            format!("#[path = {path:?}]\npub(crate) mod {name};")
        })
        .collect::<Vec<_>>()
        .join("\n");

    let names = commands
        .iter()
        .map(|command| format!("{:?}", command.name))
        .collect::<Vec<_>>()
        .join(", ");

    let registration_functions = commands
        .iter()
        .map(|command| {
            let name = &command.name;
            let function = format!("register_{name}");
            format!(
                "fn {function}() -> Result<CommandRegistration, CommandRegistrationError> {{\n    {name}::REGISTRATION.register({name}::command())\n}}"
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let registrations = commands
        .iter()
        .map(|command| format!("register_{}", command.name))
        .collect::<Vec<_>>()
        .join(",\n    ");

    format!(
        r#"// Generated built-in command registration manifest.

{modules}

{registration_functions}

#[cfg(test)]
pub(super) const BUILT_IN_COMMAND_MODULES: &[&str] = &[
    {names}
];

const BUILT_IN_COMMANDS: &[RegistrationFactory] = &[
    {registrations}
];
"#
    )
}

struct CommandModule {
    name: String,
    path: String,
}

struct CommandModuleMetadata {
    has_registration_spec: bool,
    has_command_builder: bool,
}

fn command_module_metadata(path: &Path) -> CommandModuleMetadata {
    let file = parse_file(path);

    let mut metadata = CommandModuleMetadata {
        has_registration_spec: false,
        has_command_builder: false,
    };
    for item in file.items {
        match item {
            syn::Item::Const(registration)
                if registration.ident == "REGISTRATION"
                    && matches_crate_visibility(&registration.vis) =>
            {
                metadata.has_registration_spec = true;
            }
            syn::Item::Fn(function)
                if function.sig.ident == "command" && matches_crate_visibility(&function.vis) =>
            {
                metadata.has_command_builder = true;
            }
            _ => {}
        }
    }
    metadata
}

fn parse_file(path: &Path) -> syn::File {
    let content = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("Failed to read {}: {error}", path.display()));
    syn::parse_file(&content)
        .unwrap_or_else(|error| panic!("Failed to parse {}: {error}", path.display()))
}

fn matches_crate_visibility(visibility: &syn::Visibility) -> bool {
    let syn::Visibility::Restricted(restricted) = visibility else {
        return false;
    };
    restricted.path.is_ident("crate")
}
