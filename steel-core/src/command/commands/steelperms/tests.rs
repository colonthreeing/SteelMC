    use super::{
        PermissionAssignedGroupParser, PermissionContextKeyParser, PermissionContextValueParser,
        PermissionGroupEditError, PermissionGroupNameParser, PermissionMetadataKeyParser,
        add_default_group_config, assigned_group_suggestions, can_manage_group,
        can_manage_metadata, can_manage_permission, delete_group_config,
        direct_metadata_override_suggestions, direct_permission_override_suggestions,
        group_config_metadata_value, group_config_permission_states, group_metadata_suggestions,
        group_permission_suggestions, metadata_catalog_suggestions, metadata_management_key,
        metadata_resolution_text, permission_check_result_text, permission_context,
        permission_resolution_source_text, permission_rule_context, permission_rule_context_suffix,
        remove_default_group_config, set_group_config_metadata, set_group_config_permission,
        set_group_config_priority, unset_group_config_metadata, unset_group_config_permission,
    };
    use crate::command::graph::{
        CommandArgumentParser, CommandParseErrorKind, ParsedArgument, ParsedArguments,
    };
    use crate::command::reader::CommandReader;
    use crate::command::requirement::{
        CommandInputContext, CommandSourceKind, PermissionExpr, RequirementContext,
    };
    use crate::permission::{
        PermissionContext, PermissionContextCatalog, PermissionContextCatalogSource,
        PermissionContextKey, PermissionEntry, PermissionGroupConfig, PermissionGroupsConfig,
        PermissionKey, PermissionMetadataCatalog, PermissionMetadataCatalogSource,
        PermissionResolutionSource, PermissionRuleConfig, PermissionRuleContext,
        PermissionRuleContextConfig, PermissionRuleCustomContextConfig, PermissionRuleStateConfig,
        PermissionSet, PermissionState, PermissionValue, PermissionValueEntry,
        PermissionValueRuleConfig, PermissionValueSet, parse_permission_value_key,
    };
    use steel_utils::Identifier;

    struct TestContext {
        permissions: PermissionSet,
        metadata_catalog: PermissionMetadataCatalog,
        context_catalog: PermissionContextCatalog,
    }

    impl TestContext {
        fn empty() -> Self {
            Self {
                permissions: PermissionSet::new(),
                metadata_catalog: PermissionMetadataCatalog::new(),
                context_catalog: PermissionContextCatalog::new(),
            }
        }

        fn with_permissions<const N: usize>(permissions: [&str; N]) -> Self {
            Self {
                permissions: PermissionSet::from_entries(
                    permissions.map(|permission| PermissionEntry::allow(key(permission))),
                ),
                metadata_catalog: PermissionMetadataCatalog::new(),
                context_catalog: PermissionContextCatalog::new(),
            }
        }
    }

    impl RequirementContext for TestContext {
        fn source_kind(&self) -> CommandSourceKind {
            CommandSourceKind::Player
        }

        fn has_permission(&self, permission: &PermissionExpr) -> bool {
            self.permissions.allows(permission)
        }
    }

    impl CommandInputContext for TestContext {
        fn permission_metadata_catalog(&self) -> Option<&PermissionMetadataCatalog> {
            Some(&self.metadata_catalog)
        }

        fn permission_context_catalog(&self) -> Option<&PermissionContextCatalog> {
            Some(&self.context_catalog)
        }
    }

    fn key(value: &str) -> PermissionKey {
        PermissionKey::parse(value).expect("permission key parses")
    }

    fn metadata_key(value: &str) -> Identifier {
        parse_permission_value_key(value).expect("metadata key parses")
    }

    fn context_key(value: &str) -> PermissionContextKey {
        PermissionContextKey::parse(value).expect("context key parses")
    }

    fn custom_context(key: &str, value: &str) -> PermissionRuleContext {
        PermissionRuleContext::custom(context_key(key), value).expect("custom context parses")
    }

    fn suggestion_texts(
        suggestions: Vec<steel_protocol::packets::game::SuggestionEntry>,
    ) -> Vec<String> {
        suggestions
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect()
    }

    #[test]
    fn unset_permission_suggestions_only_include_direct_overrides() {
        let first = PermissionSet::from_entries([
            PermissionEntry::allow(key("minecraft.command.gamemode.creative")),
            PermissionEntry::deny(key("steel.command.steelperms.user.allow")),
        ]);
        let second = PermissionSet::from_entries([
            PermissionEntry::allow(key("minecraft.command.gamemode.survival")),
            PermissionEntry::deny(key("steel.command.steelperms.user.allow")),
        ]);

        assert_eq!(
            suggestion_texts(direct_permission_override_suggestions(
                "minecraft.command.gamemode.",
                [first, second],
                &TestContext::with_permissions([
                    "steel.permission.manage.minecraft.command.gamemode.*",
                ]),
            )),
            vec![
                "minecraft.command.gamemode.creative",
                "minecraft.command.gamemode.survival",
            ]
        );
    }

    #[test]
    fn unset_permission_suggestions_include_contextual_overrides() {
        let overrides = PermissionSet::from_entries([
            PermissionEntry::deny_with_context(
                key("steel.fly"),
                PermissionRuleContext::domain("lobby"),
            ),
            PermissionEntry::allow(key("steel.stop")),
        ]);

        assert_eq!(
            suggestion_texts(direct_permission_override_suggestions(
                "steel.f",
                [overrides],
                &TestContext::with_permissions(["steel.permission.manage.steel.*"]),
            )),
            vec!["steel.fly{domain=lobby}"]
        );
    }

    #[test]
    fn permission_context_suffix_describes_non_global_contexts() {
        assert_eq!(
            permission_rule_context_suffix(&PermissionRuleContext::Global),
            ""
        );
        assert_eq!(
            permission_rule_context_suffix(&PermissionRuleContext::domain("lobby")),
            " (domain lobby)"
        );
        assert_eq!(
            permission_rule_context_suffix(&PermissionRuleContext::world(Identifier::new(
                "lobby", "spawn"
            ))),
            " (world lobby:spawn)"
        );
    }

    #[test]
    fn permission_check_result_text_uses_command_language() {
        assert_eq!(
            permission_check_result_text(PermissionState::Allow),
            "allowed"
        );
        assert_eq!(
            permission_check_result_text(PermissionState::Deny),
            "denied"
        );
    }

    #[test]
    fn permission_resolution_source_text_includes_group_priority() {
        assert_eq!(
            permission_resolution_source_text(&PermissionResolutionSource::Subject),
            "direct permission"
        );
        assert_eq!(
            permission_resolution_source_text(&PermissionResolutionSource::Group {
                name: "admin".to_owned(),
                priority: 50,
            }),
            "group 'admin' priority 50"
        );
    }

    #[test]
    fn metadata_key_parser_requests_server_suggestions() {
        let (_, suggestion_type) = PermissionMetadataKeyParser
            .client_parser()
            .into_protocol_argument();
        assert!(matches!(
            suggestion_type,
            Some(steel_protocol::packets::game::SuggestionType::AskServer)
        ));
    }

    #[test]
    fn context_key_and_value_parsers_request_server_suggestions() {
        let (key_argument_type, key_suggestion_type) = PermissionContextKeyParser
            .client_parser()
            .into_protocol_argument();
        assert!(matches!(
            key_argument_type,
            steel_protocol::packets::game::ArgumentType::Identifier
        ));
        assert!(matches!(
            key_suggestion_type,
            Some(steel_protocol::packets::game::SuggestionType::AskServer)
        ));
        let (_, value_suggestion_type) = PermissionContextValueParser::new("context_custom_key")
            .client_parser()
            .into_protocol_argument();
        assert!(matches!(
            value_suggestion_type,
            Some(steel_protocol::packets::game::SuggestionType::AskServer)
        ));
    }

    #[test]
    fn context_key_parser_accepts_namespaced_keys() {
        let mut reader = CommandReader::new("plugin:region");
        let parsed = PermissionContextKeyParser
            .parse(&mut reader, &TestContext::empty())
            .expect("namespaced context key parses");

        assert!(
            matches!(parsed, ParsedArgument::String(ref value) if value == "plugin:region"),
            "expected parsed context key string, got {parsed:?}"
        );
        assert_eq!(reader.remaining(), "");
    }

    #[test]
    fn context_catalog_suggestions_include_known_keys_and_values() {
        let mut context = TestContext::empty();
        context.context_catalog.insert_value(
            context_key("region"),
            "spawn",
            PermissionContextCatalogSource::Config,
        );
        context.context_catalog.insert_value(
            context_key("region"),
            "market",
            PermissionContextCatalogSource::Config,
        );
        context.context_catalog.insert_value(
            context_key("arena"),
            "duel",
            PermissionContextCatalogSource::Config,
        );
        let mut arguments = ParsedArguments::default();
        arguments.insert(
            "context_custom_key",
            ParsedArgument::String("region".to_owned()),
        );

        assert_eq!(
            suggestion_texts(PermissionContextKeyParser.suggest(
                "r",
                &ParsedArguments::default(),
                &context
            )),
            vec!["region"]
        );
        assert_eq!(
            suggestion_texts(
                PermissionContextValueParser::new("context_custom_key")
                    .suggest("s", &arguments, &context)
            ),
            vec!["spawn"]
        );
    }

    #[test]
    fn metadata_catalog_suggestions_only_include_manageable_keys() {
        let mut context = TestContext::with_permissions(["steel.permission.metadata.plugin.*"]);
        context.metadata_catalog.insert(
            metadata_key("plugin:homes"),
            PermissionMetadataCatalogSource::Config,
        );
        context.metadata_catalog.insert(
            metadata_key("other:homes"),
            PermissionMetadataCatalogSource::Config,
        );

        assert_eq!(
            suggestion_texts(PermissionMetadataKeyParser.suggest(
                "",
                &ParsedArguments::default(),
                &context
            )),
            vec!["plugin:homes"]
        );
        assert_eq!(
            suggestion_texts(metadata_catalog_suggestions(
                "plugin:",
                &context.metadata_catalog,
                &context
            )),
            vec!["plugin:homes"]
        );
    }

    #[test]
    fn permission_context_key_parser_rejects_invalid_segments() {
        let mut reader = CommandReader::new("region.spawn");
        let error = PermissionContextKeyParser
            .parse(&mut reader, &TestContext::empty())
            .expect_err("context key should be one permission segment");

        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::InvalidPermissionKey(value) if value == "region.spawn"
        ));
    }

    #[test]
    fn parsed_contexts_can_chain_domain_and_custom_constraints() {
        let permission = key("steel.region.build");
        let mut arguments = ParsedArguments::default();
        arguments.insert("context_domain", ParsedArgument::String("lobby".to_owned()));
        arguments.insert(
            "context_custom_key",
            ParsedArgument::String("region".to_owned()),
        );
        arguments.insert(
            "context_custom_value",
            ParsedArgument::String("spawn".to_owned()),
        );
        let Ok(rule_context) = permission_rule_context(&arguments) else {
            panic!("rule context should parse");
        };
        let Ok(check_context) = permission_context(&arguments) else {
            panic!("check context should parse");
        };
        let permissions = PermissionSet::from_entries([PermissionEntry::allow_with_context(
            permission.clone(),
            rule_context,
        )]);

        assert!(permissions.allows_key_in(&permission, &check_context));
        assert!(!permissions.allows_key_in(&permission, &PermissionContext::for_domain("lobby")));
    }

    #[test]
    fn metadata_resolution_text_describes_winning_value_rule() {
        let homes = metadata_key("plugin:homes");
        let values = PermissionValueSet::from_entries([PermissionValueEntry::new_with_context(
            homes.clone(),
            PermissionRuleContext::domain("lobby"),
            PermissionValue::Integer(10),
        )]);
        let resolution = values
            .resolve_in_detailed(&homes, &PermissionContext::for_domain("lobby"))
            .expect("metadata value resolves");

        assert_eq!(
            metadata_resolution_text(&resolution),
            "set by direct permission, rule plugin:homes = 10 (domain lobby), context specificity 1, insertion 0"
        );
    }

    #[test]
    fn remove_group_parser_accepts_unknown_group_names() {
        let mut reader = CommandReader::new("legacy");
        let parsed = PermissionAssignedGroupParser::new("targets")
            .parse(&mut reader, &TestContext::empty())
            .expect("group parses");

        assert!(matches!(parsed, ParsedArgument::String(group) if group == "legacy"));
    }

    #[test]
    fn remove_group_parser_rejects_invalid_group_names() {
        let mut reader = CommandReader::new("Legacy");
        let error = PermissionAssignedGroupParser::new("targets")
            .parse(&mut reader, &TestContext::empty())
            .expect_err("uppercase group should be invalid");

        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::InvalidPermissionGroup(group) if group == "Legacy"
        ));
    }

    #[test]
    fn group_name_parser_accepts_unknown_group_names() {
        let mut reader = CommandReader::new("builder");
        let parsed = PermissionGroupNameParser
            .parse(&mut reader, &TestContext::empty())
            .expect("group name parses");

        assert!(matches!(parsed, ParsedArgument::String(group) if group == "builder"));
    }

    #[test]
    fn remove_group_suggestions_only_include_assigned_groups() {
        assert_eq!(
            suggestion_texts(assigned_group_suggestions(
                "v",
                [
                    vec!["vip".to_owned(), "legacy".to_owned()],
                    vec!["vip".to_owned(), "veteran".to_owned()],
                ],
            )),
            vec!["veteran".to_owned(), "vip".to_owned()]
        );
    }

    #[test]
    fn manage_permission_requires_targeted_management_permission() {
        let context = TestContext::with_permissions([
            "steel.permission.manage.minecraft.command.gamemode.creative",
        ]);

        assert!(can_manage_permission(
            &context,
            &key("minecraft.command.gamemode.creative")
        ));
        assert!(!can_manage_permission(
            &context,
            &key("minecraft.command.gamemode.survival")
        ));
    }

    #[test]
    fn manage_group_requires_targeted_group_permission() {
        let context = TestContext::with_permissions(["steel.permission.group.op"]);

        assert!(can_manage_group(&context, "op"));
        assert!(!can_manage_group(&context, "admin"));
    }

    #[test]
    fn manage_metadata_uses_namespaced_key_permission() {
        let context = TestContext::with_permissions(["steel.permission.metadata.plugin.homes"]);
        let homes = metadata_key("plugin:homes");
        let other = metadata_key("plugin:other");

        assert_eq!(
            metadata_management_key(&homes),
            Ok(key("steel.permission.metadata.plugin.homes"))
        );
        assert!(can_manage_metadata(&context, &homes));
        assert!(!can_manage_metadata(&context, &other));
    }

    #[test]
    fn direct_metadata_suggestions_only_include_manageable_overrides() {
        let values = PermissionValueSet::from_entries([
            PermissionValueEntry::new(metadata_key("plugin:homes"), PermissionValue::Integer(10)),
            PermissionValueEntry::new(metadata_key("other:homes"), PermissionValue::Integer(20)),
        ]);

        assert_eq!(
            suggestion_texts(direct_metadata_override_suggestions(
                "",
                [values],
                &TestContext::with_permissions(["steel.permission.metadata.plugin.*"]),
            )),
            vec!["plugin:homes"]
        );
    }

    #[test]
    fn group_config_global_permission_edits_use_allow_and_deny_lists() {
        let mut config = PermissionGroupsConfig::default();
        let permission = key("steel.fly");

        assert_eq!(
            set_group_config_permission(
                &mut config,
                "default",
                &permission,
                &PermissionRuleContext::Global,
                PermissionState::Allow,
            ),
            Ok(true)
        );
        let default = config.groups.get("default").expect("default group exists");
        assert_eq!(default.allow, vec!["steel.fly"]);
        assert!(default.deny.is_empty());
        assert!(default.rules.is_empty());

        assert_eq!(
            set_group_config_permission(
                &mut config,
                "default",
                &permission,
                &PermissionRuleContext::Global,
                PermissionState::Deny,
            ),
            Ok(true)
        );
        let default = config.groups.get("default").expect("default group exists");
        assert!(default.allow.is_empty());
        assert_eq!(default.deny, vec!["steel.fly"]);

        assert_eq!(
            set_group_config_permission(
                &mut config,
                "default",
                &permission,
                &PermissionRuleContext::Global,
                PermissionState::Deny,
            ),
            Ok(false)
        );
        assert_eq!(
            unset_group_config_permission(
                &mut config,
                "default",
                &permission,
                &PermissionRuleContext::Global,
            ),
            Ok(true)
        );
        assert_eq!(
            unset_group_config_permission(
                &mut config,
                "default",
                &permission,
                &PermissionRuleContext::Global,
            ),
            Ok(false)
        );
    }

    #[test]
    fn group_config_contextual_permission_edits_use_structured_rules() {
        let mut config = PermissionGroupsConfig::default();
        let permission = key("steel.fly");
        let lobby = PermissionRuleContext::domain("lobby");

        assert_eq!(
            set_group_config_permission(
                &mut config,
                "default",
                &permission,
                &lobby,
                PermissionState::Allow,
            ),
            Ok(true)
        );
        let default = config.groups.get("default").expect("default group exists");
        assert!(default.allow.is_empty());
        assert!(default.deny.is_empty());
        assert_eq!(default.rules.len(), 1);
        assert_eq!(default.rules[0].key, "steel.fly");
        assert_eq!(default.rules[0].state, PermissionRuleStateConfig::Allow);
        assert_eq!(
            default.rules[0].context,
            Some(PermissionRuleContextConfig {
                domain: Some("lobby".to_owned()),
                world: None,
                custom: Vec::new(),
            })
        );

        assert_eq!(
            set_group_config_permission(
                &mut config,
                "default",
                &permission,
                &lobby,
                PermissionState::Deny,
            ),
            Ok(true)
        );
        let default = config.groups.get("default").expect("default group exists");
        assert_eq!(default.rules.len(), 1);
        assert_eq!(default.rules[0].state, PermissionRuleStateConfig::Deny);

        assert_eq!(
            group_config_permission_states(default, &permission, &lobby),
            vec![PermissionState::Deny]
        );
    }

    #[test]
    fn group_config_custom_context_permission_edits_use_structured_rules() {
        let mut config = PermissionGroupsConfig::default();
        let permission = key("steel.region.build");
        let spawn_region = custom_context("region", "spawn");

        assert_eq!(
            set_group_config_permission(
                &mut config,
                "default",
                &permission,
                &spawn_region,
                PermissionState::Allow,
            ),
            Ok(true)
        );

        let default = config.groups.get("default").expect("default group exists");
        assert_eq!(default.rules.len(), 1);
        assert_eq!(default.rules[0].key, "steel.region.build");
        assert_eq!(default.rules[0].state, PermissionRuleStateConfig::Allow);
        assert_eq!(
            default.rules[0].context,
            Some(PermissionRuleContextConfig {
                domain: None,
                world: None,
                custom: vec![PermissionRuleCustomContextConfig {
                    key: "region".to_owned(),
                    value: "spawn".to_owned(),
                }],
            })
        );
        assert_eq!(
            group_config_permission_states(default, &permission, &spawn_region),
            vec![PermissionState::Allow]
        );
    }

    #[test]
    fn group_config_metadata_edits_use_value_rules() {
        let mut config = PermissionGroupsConfig::default();
        let homes = metadata_key("plugin:homes");
        let lobby = PermissionRuleContext::domain("lobby");

        assert_eq!(
            set_group_config_metadata(
                &mut config,
                "default",
                &homes,
                &PermissionValue::Integer(10),
                &PermissionRuleContext::Global,
            ),
            Ok(true)
        );
        assert_eq!(
            set_group_config_metadata(
                &mut config,
                "default",
                &homes,
                &PermissionValue::Integer(5),
                &lobby,
            ),
            Ok(true)
        );
        let default = config.groups.get("default").expect("default group exists");
        assert_eq!(default.values.len(), 2);
        assert_eq!(
            group_config_metadata_value(default, &homes, &PermissionRuleContext::Global),
            Some(&PermissionValue::Integer(10))
        );
        assert_eq!(
            group_config_metadata_value(default, &homes, &lobby),
            Some(&PermissionValue::Integer(5))
        );

        assert_eq!(
            set_group_config_metadata(
                &mut config,
                "default",
                &homes,
                &PermissionValue::Integer(5),
                &lobby,
            ),
            Ok(false)
        );
    }

    #[test]
    fn group_config_custom_context_metadata_edits_use_value_rules() {
        let mut config = PermissionGroupsConfig::default();
        let homes = metadata_key("plugin:homes");
        let spawn_region = custom_context("region", "spawn");

        assert_eq!(
            set_group_config_metadata(
                &mut config,
                "default",
                &homes,
                &PermissionValue::Integer(10),
                &spawn_region,
            ),
            Ok(true)
        );

        let default = config.groups.get("default").expect("default group exists");
        assert_eq!(default.values.len(), 1);
        assert_eq!(default.values[0].key, "plugin:homes");
        assert_eq!(default.values[0].value, PermissionValue::Integer(10));
        assert_eq!(
            default.values[0].context,
            Some(PermissionRuleContextConfig {
                domain: None,
                world: None,
                custom: vec![PermissionRuleCustomContextConfig {
                    key: "region".to_owned(),
                    value: "spawn".to_owned(),
                }],
            })
        );
        assert_eq!(
            group_config_metadata_value(default, &homes, &spawn_region),
            Some(&PermissionValue::Integer(10))
        );
    }

    #[test]
    fn group_config_metadata_unset_is_context_exact() {
        let mut config = PermissionGroupsConfig::default();
        let homes = metadata_key("plugin:homes");
        let global = PermissionRuleContext::Global;
        let lobby = PermissionRuleContext::domain("lobby");

        set_group_config_metadata(
            &mut config,
            "default",
            &homes,
            &PermissionValue::Integer(10),
            &global,
        )
        .expect("global metadata stores");
        set_group_config_metadata(
            &mut config,
            "default",
            &homes,
            &PermissionValue::Integer(5),
            &lobby,
        )
        .expect("contextual metadata stores");

        assert_eq!(
            unset_group_config_metadata(&mut config, "default", &homes, &global),
            Ok(true)
        );
        let default = config.groups.get("default").expect("default group exists");
        assert_eq!(default.values.len(), 1);
        assert_eq!(
            group_config_metadata_value(default, &homes, &lobby),
            Some(&PermissionValue::Integer(5))
        );
    }

    #[test]
    fn group_config_priority_edit_updates_group_priority() {
        let mut config = PermissionGroupsConfig::default();
        let default = config.groups.get("default").expect("default group exists");
        assert_eq!(default.priority, 0);

        assert_eq!(
            set_group_config_priority(&mut config, "default", 50),
            Ok(true)
        );
        let default = config.groups.get("default").expect("default group exists");
        assert_eq!(default.priority, 50);
        assert_eq!(
            set_group_config_priority(&mut config, "default", 50),
            Ok(false)
        );
    }

    #[test]
    fn group_config_priority_edit_reports_missing_group() {
        let mut config = PermissionGroupsConfig::default();

        assert_eq!(
            set_group_config_priority(&mut config, "missing", 10),
            Err(PermissionGroupEditError::Missing("missing".to_owned()))
        );
    }

    #[test]
    fn default_group_config_add_and_remove_update_default_groups() {
        let mut config = PermissionGroupsConfig::default();
        config
            .groups
            .insert("builder".to_owned(), PermissionGroupConfig::default());

        assert_eq!(add_default_group_config(&mut config, "builder"), Ok(true));
        assert_eq!(config.default_groups, vec!["default", "builder"]);
        assert_eq!(add_default_group_config(&mut config, "builder"), Ok(false));
        assert_eq!(
            remove_default_group_config(&mut config, "builder"),
            Ok(true)
        );
        assert_eq!(config.default_groups, vec!["default"]);
        assert_eq!(
            remove_default_group_config(&mut config, "builder"),
            Ok(false)
        );
    }

    #[test]
    fn default_group_config_edits_report_missing_groups() {
        let mut config = PermissionGroupsConfig::default();

        assert_eq!(
            add_default_group_config(&mut config, "missing"),
            Err(PermissionGroupEditError::Missing("missing".to_owned()))
        );
        assert_eq!(
            remove_default_group_config(&mut config, "missing"),
            Err(PermissionGroupEditError::Missing("missing".to_owned()))
        );
    }

    #[test]
    fn delete_group_config_removes_non_default_group() {
        let mut config = PermissionGroupsConfig::default();
        config
            .groups
            .insert("builder".to_owned(), PermissionGroupConfig::default());

        assert_eq!(delete_group_config(&mut config, "builder"), Ok(()));
        assert!(!config.groups.contains_key("builder"));
    }

    #[test]
    fn delete_group_config_rejects_required_group() {
        let mut config = PermissionGroupsConfig::default();

        assert_eq!(
            delete_group_config(&mut config, "op"),
            Err(PermissionGroupEditError::Required("op".to_owned()))
        );
    }

    #[test]
    fn delete_group_config_rejects_default_group() {
        let mut config = PermissionGroupsConfig::default();

        assert_eq!(
            delete_group_config(&mut config, "default"),
            Err(PermissionGroupEditError::Default("default".to_owned()))
        );
    }

    #[test]
    fn delete_group_config_reports_missing_group() {
        let mut config = PermissionGroupsConfig::default();

        assert_eq!(
            delete_group_config(&mut config, "missing"),
            Err(PermissionGroupEditError::Missing("missing".to_owned()))
        );
    }

    #[test]
    fn group_config_unset_is_context_exact() {
        let mut config = PermissionGroupsConfig::default();
        let permission = key("steel.fly");
        let global = PermissionRuleContext::Global;
        let lobby = PermissionRuleContext::domain("lobby");

        set_group_config_permission(
            &mut config,
            "default",
            &permission,
            &global,
            PermissionState::Allow,
        )
        .expect("global permission stores");
        set_group_config_permission(
            &mut config,
            "default",
            &permission,
            &lobby,
            PermissionState::Deny,
        )
        .expect("contextual permission stores");

        assert_eq!(
            unset_group_config_permission(&mut config, "default", &permission, &global),
            Ok(true)
        );
        let default = config.groups.get("default").expect("default group exists");
        assert!(default.allow.is_empty());
        assert_eq!(default.rules.len(), 1);
        assert_eq!(default.rules[0].state, PermissionRuleStateConfig::Deny);
    }

    #[test]
    fn group_config_edit_reports_missing_group() {
        let mut config = PermissionGroupsConfig::default();

        assert_eq!(
            set_group_config_permission(
                &mut config,
                "missing",
                &key("steel.fly"),
                &PermissionRuleContext::Global,
                PermissionState::Allow,
            ),
            Err(PermissionGroupEditError::Missing("missing".to_owned()))
        );
    }

    #[test]
    fn group_permission_suggestions_only_include_manageable_group_rules() {
        let group = PermissionGroupConfig {
            priority: 0,
            allow: vec![
                "steel.fly".to_owned(),
                "minecraft.command.gamemode".to_owned(),
            ],
            deny: vec!["steel.stop".to_owned()],
            rules: vec![PermissionRuleConfig {
                key: "steel.chat".to_owned(),
                state: PermissionRuleStateConfig::Allow,
                context: Some(PermissionRuleContextConfig {
                    domain: Some("lobby".to_owned()),
                    world: None,
                    custom: Vec::new(),
                }),
            }],
            values: Vec::new(),
        };

        assert_eq!(
            suggestion_texts(group_permission_suggestions(
                "steel.",
                &group,
                &TestContext::with_permissions(["steel.permission.manage.steel.*"]),
            )),
            vec!["steel.chat{domain=lobby}", "steel.fly", "steel.stop"]
        );
    }

    #[test]
    fn group_metadata_suggestions_only_include_manageable_group_values() {
        let group = PermissionGroupConfig {
            priority: 0,
            allow: Vec::new(),
            deny: Vec::new(),
            rules: Vec::new(),
            values: vec![
                PermissionValueRuleConfig {
                    key: "plugin:homes".to_owned(),
                    value: PermissionValue::Integer(10),
                    context: None,
                },
                PermissionValueRuleConfig {
                    key: "other:homes".to_owned(),
                    value: PermissionValue::Integer(20),
                    context: None,
                },
            ],
        };

        assert_eq!(
            suggestion_texts(group_metadata_suggestions(
                "",
                &group,
                &TestContext::with_permissions(["steel.permission.metadata.plugin.*"]),
            )),
            vec!["plugin:homes"]
        );
    }