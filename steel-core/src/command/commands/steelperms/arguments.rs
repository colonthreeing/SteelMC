//! Parsed argument extraction and context assembly for the `steelperms` command.

use std::sync::Arc;

use steel_utils::Identifier;

use crate::command::error::CommandError;
use crate::command::graph::{ParsedArguments, PermissionTarget};
use crate::permission::{
    PermissionContext, PermissionContextKey, PermissionKey, PermissionRuleContext,
    PermissionRuleExpression, PermissionValue,
};
use crate::world::World;

pub(super) fn targets(arguments: &ParsedArguments) -> Result<Vec<PermissionTarget>, CommandError> {
    let targets = arguments
        .get::<Vec<PermissionTarget>>("targets")
        .map_err(super::super::invalid_parsed_argument)?;
    if targets.is_empty() {
        return Err(CommandError::failure("No players matched"));
    }

    Ok(targets)
}

pub(super) fn group(arguments: &ParsedArguments) -> Result<String, CommandError> {
    arguments
        .get::<String>("group")
        .map_err(super::super::invalid_parsed_argument)
}

pub(super) fn group_priority(arguments: &ParsedArguments) -> Result<i32, CommandError> {
    arguments
        .get::<i32>("priority")
        .map_err(super::super::invalid_parsed_argument)
}

pub(super) fn permission(arguments: &ParsedArguments) -> Result<PermissionKey, CommandError> {
    if let Ok(expression) = arguments.get::<PermissionRuleExpression>("permission") {
        return Ok(expression.key().clone());
    }

    arguments
        .get::<PermissionKey>("permission")
        .map_err(super::super::invalid_parsed_argument)
}

pub(super) fn metadata_key(arguments: &ParsedArguments) -> Result<Identifier, CommandError> {
    arguments
        .get::<Identifier>("metadata_key")
        .map_err(super::super::invalid_parsed_argument)
}

pub(super) fn metadata_value(arguments: &ParsedArguments) -> Result<PermissionValue, CommandError> {
    if let Ok(value) = arguments.get::<i64>("metadata_int_value") {
        return Ok(PermissionValue::Integer(value));
    }
    if let Ok(value) = arguments.get::<bool>("metadata_bool_value") {
        return Ok(PermissionValue::Bool(value));
    }
    if let Ok(value) = arguments.get::<String>("metadata_string_value") {
        return Ok(PermissionValue::String(value));
    }

    Err(CommandError::failure("Missing metadata value"))
}

pub(super) fn permission_rule_context(
    arguments: &ParsedArguments,
) -> Result<PermissionRuleContext, CommandError> {
    let mut contexts = Vec::new();
    if let Ok(expression) = arguments.get::<PermissionRuleExpression>("permission") {
        contexts.push(expression.context().clone());
    }
    if let Ok(domain) = arguments.get::<String>("context_domain") {
        contexts.push(PermissionRuleContext::domain(domain));
    }
    if let Ok(world) = arguments.get::<Arc<World>>("context_world") {
        contexts.push(PermissionRuleContext::world(world.key.clone()));
    }
    if let Some(custom) = custom_permission_rule_context(arguments)? {
        contexts.push(custom);
    }

    PermissionRuleContext::all(contexts).map_err(|error| CommandError::failure(error.to_string()))
}

pub(super) fn permission_context(
    arguments: &ParsedArguments,
) -> Result<PermissionContext, CommandError> {
    let rule_context = permission_rule_context(arguments)?;
    permission_context_from_rule_context(&rule_context)
}

fn permission_context_from_rule_context(
    rule_context: &PermissionRuleContext,
) -> Result<PermissionContext, CommandError> {
    let mut context = PermissionContext::global();
    append_permission_context_from_rule_context(&mut context, rule_context)?;
    Ok(context)
}

fn append_permission_context_from_rule_context(
    context: &mut PermissionContext,
    rule_context: &PermissionRuleContext,
) -> Result<(), CommandError> {
    match rule_context {
        PermissionRuleContext::Global => {}
        PermissionRuleContext::Domain(domain) => {
            *context = PermissionContext::for_domain(domain.clone());
        }
        PermissionRuleContext::World(world) => {
            *context =
                PermissionContext::for_world(world.namespace.as_ref().to_owned(), world.clone());
        }
        PermissionRuleContext::Custom { key, value } => {
            context
                .add_custom_context(key.clone(), value.clone())
                .map_err(|error| CommandError::failure(error.to_string()))?;
        }
        PermissionRuleContext::All(contexts) => {
            for rule_context in contexts.iter() {
                append_permission_context_from_rule_context(context, rule_context)?;
            }
        }
    }
    Ok(())
}

fn custom_permission_rule_context(
    arguments: &ParsedArguments,
) -> Result<Option<PermissionRuleContext>, CommandError> {
    let Some((key, value)) = custom_permission_context(arguments)? else {
        return Ok(None);
    };

    PermissionRuleContext::custom(key, value)
        .map(Some)
        .map_err(|error| CommandError::failure(error.to_string()))
}

fn custom_permission_context(
    arguments: &ParsedArguments,
) -> Result<Option<(PermissionContextKey, String)>, CommandError> {
    let Ok(key) = arguments.get::<String>("context_custom_key") else {
        return Ok(None);
    };
    let value = arguments
        .get::<String>("context_custom_value")
        .map_err(super::super::invalid_parsed_argument)?;
    let key = PermissionContextKey::parse(key)
        .map_err(|error| CommandError::failure(format!("Invalid context key: {error}")))?;
    Ok(Some((key, value)))
}
