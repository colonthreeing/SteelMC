//! Parsed argument extraction and context assembly for the `steelperms` command.

use steel_utils::Identifier;

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{ParsedArguments, PermissionTarget};
use crate::command::parsers::resolve_required_permission_targets;
use crate::permission::{
    PermissionContext, PermissionKey, PermissionMetadataExpression, PermissionRuleContext,
    PermissionRuleExpression, PermissionMetadataValue,
};

pub(super) fn targets(
    context: &CommandContext,
    arguments: &ParsedArguments,
) -> Result<Vec<PermissionTarget>, CommandError> {
    resolve_required_permission_targets(arguments, "targets", context)
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
    let expression = arguments
        .get::<PermissionMetadataExpression>("metadata")
        .map_err(super::super::invalid_parsed_argument)?;
    Ok(expression.key().clone())
}

pub(super) fn metadata_value(arguments: &ParsedArguments) -> Result<PermissionMetadataValue, CommandError> {
    if let Ok(value) = arguments.get::<i64>("metadata_int_value") {
        return Ok(PermissionMetadataValue::Integer(value));
    }
    if let Ok(value) = arguments.get::<bool>("metadata_bool_value") {
        return Ok(PermissionMetadataValue::Bool(value));
    }
    if let Ok(value) = arguments.get::<String>("metadata_string_value") {
        return Ok(PermissionMetadataValue::String(value));
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
    if let Ok(expression) = arguments.get::<PermissionMetadataExpression>("metadata") {
        contexts.push(expression.context().clone());
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
