//! Handler for the "gamerule" command.
use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    BoolParser, CommandNodeBuilder, CommandResult, IntegerParser, ParsedArgumentError,
    ParsedArguments, argument, literal,
};
use steel_registry::REGISTRY;
use steel_registry::game_rules::{GameRuleRef, GameRuleType, GameRuleValue};
use steel_utils::translations;
use text_components::TextComponent;

/// Returns the handler for the "gamerule" command.
#[must_use]
pub fn command() -> CommandNodeBuilder {
    let mut command = literal("gamerule");

    for (_, rule) in REGISTRY.game_rules.iter() {
        let rule_name = rule.key.path.to_string();
        let rule_node = match rule.value_type {
            GameRuleType::Bool => literal(rule_name)
                .executes(move |context, _| query_rule(context, rule))
                .then(
                    argument("value", BoolParser).executes(move |context, arguments| {
                        set_bool_rule(context, arguments, rule)
                    }),
                ),
            GameRuleType::Int => literal(rule_name)
                .executes(move |context, _| query_rule(context, rule))
                .then(
                    argument(
                        "value",
                        IntegerParser::bounded(rule.min_value, rule.max_value),
                    )
                    .executes(move |context, arguments| set_int_rule(context, arguments, rule)),
                ),
        };
        command = command.then(rule_node);
    }

    command
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "Command executors use a shared fallible callback signature."
)]
fn query_rule(
    context: &mut CommandContext,
    rule: GameRuleRef,
) -> Result<CommandResult, CommandError> {
    let world = &context.world;
    let rule_name = rule.key.path.to_string();
    let value = world.get_game_rule(rule);

    context.sender.send_message(
        &translations::COMMANDS_GAMERULE_QUERY
            .message([
                TextComponent::from(rule_name),
                TextComponent::from(value.to_string()),
            ])
            .into(),
    );

    Ok(CommandResult::success())
}

fn set_bool_rule(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    rule: GameRuleRef,
) -> Result<CommandResult, CommandError> {
    let value = arguments
        .get::<bool>("value")
        .map_err(invalid_parsed_argument)?;

    set_rule(context, rule, GameRuleValue::Bool(value), value.to_string())
}

fn set_int_rule(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    rule: GameRuleRef,
) -> Result<CommandResult, CommandError> {
    let value = arguments
        .get::<i32>("value")
        .map_err(invalid_parsed_argument)?;

    set_rule(context, rule, GameRuleValue::Int(value), value.to_string())
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "Command executors use a shared fallible callback signature."
)]
fn set_rule(
    context: &mut CommandContext,
    rule: GameRuleRef,
    value: GameRuleValue,
    displayed_value: String,
) -> Result<CommandResult, CommandError> {
    let world = &context.world;
    let rule_name = rule.key.path.to_string();

    world.set_game_rule(rule, value);

    context.sender.send_message(
        &translations::COMMANDS_GAMERULE_SET
            .message([
                TextComponent::from(rule_name),
                TextComponent::from(displayed_value),
            ])
            .into(),
    );

    Ok(CommandResult::success())
}

fn invalid_parsed_argument(error: ParsedArgumentError) -> CommandError {
    CommandError::InvalidConsumption(Some(format!("{error:?}")))
}
