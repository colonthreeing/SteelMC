use std::path::PathBuf;

use super::domain::{
    AbilitiesFile, PLAYER_STORAGE_VERSION, PlayerDataFile, RootVehicleFile, SlotFile,
    decode_player_file, encode_player_file, item_to_nbt_bytes,
};
use super::global::{
    GLOBAL_PLAYER_DATA_VERSION, GLOBAL_STORAGE_VERSION, GlobalPlayerDataFile, decode_global_file,
    encode_global_file,
};
use super::known::{
    KNOWN_PLAYERS_STORAGE_VERSION, KnownPlayersFile, decode_known_players_file,
    encode_known_players_file,
};
use super::permissions::{
    PlayerPermissionEntryFile, PlayerPermissionMetadataEntryFile, PlayerPermissionsFile,
    serialize_player_permissions_file, set_player_permission_entry,
};
use super::*;
use crate::chunk_saver::PersistentEntity;
use crate::config::StorageSelection;
use crate::entity::DEFAULT_MAX_AIR_SUPPLY;
use crate::permission::{
    PermissionContextKey, PermissionEntry, PermissionKey, PermissionMetadataEntry,
    PermissionMetadataSet, PermissionMetadataValue, PermissionRuleContext, PermissionSet,
    parse_permission_metadata_key,
};
use crate::player::known_players::{KnownPlayer, KnownPlayers};
use crate::player::player_data::PLAYER_DATA_VERSION;
use steel_registry::item_stack::ItemStack;
use steel_registry::test_support::init_test_registry;
use steel_registry::vanilla_items::ITEMS;
use steel_utils::Identifier;
use tokio::{fs, io};
use uuid::Uuid;

fn temp_storage_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "steelmc-player-data-storage-{name}-{}",
        Uuid::new_v4()
    ))
}

fn sample_player_file(data_version: i32) -> PlayerDataFile {
    PlayerDataFile {
        data_version,
        pos: [1.0, 2.0, 3.0],
        motion: [0.0, 0.0, 0.0],
        rotation: [90.0, 10.0],
        on_ground: true,
        fall_flying: false,
        remaining_fire_ticks: 0,
        ticks_frozen: 0,
        is_in_powder_snow: false,
        was_in_powder_snow: false,
        has_visual_fire: false,
        health: 20.0,
        game_mode: 2,
        prev_game_mode: Some(0),
        abilities: AbilitiesFile {
            invulnerable: false,
            flying: false,
            may_fly: false,
            instabuild: false,
            may_build: true,
            flying_speed: 0.05,
            walking_speed: 0.1,
        },
        inventory: Vec::new(),
        ender_chest: Vec::new(),
        selected_slot: 4,
        world: "lobby:void".to_owned(),
        food_level: 20,
        food_saturation_level: 5.0,
        food_exhaustion_level: 0.0,
        food_tick_timer: 0,
        experience_level: 7,
        experience_progress: 0.5,
        experience_total: 32,
        score: 9,
        root_vehicle: None,
    }
}

fn sample_persistent_entity() -> PersistentEntity {
    PersistentEntity {
        entity_type: Identifier::vanilla_static("minecart"),
        uuid: [7; 16],
        pos: [4.0, 65.0, 6.0],
        motion: [0.0, 0.0, 0.0],
        rotation: [45.0, 0.0],
        fall_distance: 0.0,
        remaining_fire_ticks: 0,
        ticks_frozen: 0,
        is_in_powder_snow: false,
        was_in_powder_snow: false,
        has_visual_fire: false,
        on_ground: true,
        no_gravity: false,
        invulnerable: false,
        air_supply: DEFAULT_MAX_AIR_SUPPLY,
        portal_cooldown: 0,
        custom_name_nbt: Vec::new(),
        custom_name_visible: false,
        silent: false,
        glowing: false,
        tags: Vec::new(),
        custom_data_nbt: Vec::new(),
        nbt_data: Vec::new(),
        passengers: Vec::new(),
    }
}

#[test]
fn player_file_roundtrip_preserves_domain_world_data() {
    let file = sample_player_file(PLAYER_DATA_VERSION);

    let encoded = encode_player_file(&file).expect("player file should encode");
    let decoded = decode_player_file(&encoded).expect("player file should decode");

    assert_eq!(
        u16::from_le_bytes([encoded[4], encoded[5]]),
        PLAYER_STORAGE_VERSION
    );
    assert_eq!(decoded.world, "lobby:void");
    assert_eq!(decoded.game_mode, 2);
    assert_eq!(decoded.selected_slot, 4);
    assert_eq!(decoded.experience_level, 7);
}

#[test]
fn player_file_roundtrip_preserves_ender_chest() {
    init_test_registry();

    let mut file = sample_player_file(PLAYER_DATA_VERSION);
    file.ender_chest = vec![SlotFile {
        slot: 5,
        item_nbt: item_to_nbt_bytes(&ItemStack::with_count(&ITEMS.stone, 2))
            .expect("item stack should encode"),
    }];

    let encoded = encode_player_file(&file).expect("player file should encode");
    let decoded = decode_player_file(&encoded).expect("player file should decode");
    let persistent = decoded
        .into_persistent()
        .expect("player file should convert");

    assert_eq!(persistent.ender_chest.len(), 1);
    assert_eq!(persistent.ender_chest[0].slot, 5);
    assert_eq!(
        persistent.ender_chest[0].item,
        ItemStack::with_count(&ITEMS.stone, 2)
    );
}

#[test]
fn player_file_roundtrip_preserves_absent_previous_game_mode() {
    let mut file = sample_player_file(PLAYER_DATA_VERSION);
    file.prev_game_mode = None;

    let encoded = encode_player_file(&file).expect("player file should encode");
    let decoded = decode_player_file(&encoded).expect("player file should decode");
    let persistent = decoded
        .into_persistent()
        .expect("player file should convert");

    assert_eq!(persistent.prev_game_mode, None);
}

#[test]
fn global_file_roundtrip_preserves_last_active_domain() {
    let file = GlobalPlayerDataFile {
        data_version: GLOBAL_PLAYER_DATA_VERSION,
        last_active_domain: "minecraft".to_owned(),
    };

    let encoded = encode_global_file(&file).expect("global file should encode");
    let decoded = decode_global_file(&encoded).expect("global file should decode");

    assert_eq!(
        u16::from_le_bytes([encoded[4], encoded[5]]),
        GLOBAL_STORAGE_VERSION
    );
    assert_eq!(decoded.last_active_domain, "minecraft");
}

#[test]
fn player_permissions_file_roundtrip_preserves_permissions() {
    let uuid = Uuid::from_u128(1);
    let values = PermissionMetadataSet::from_entries([
        PermissionMetadataEntry::new(
            parse_permission_metadata_key("steel:homes").expect("metadata key parses"),
            PermissionMetadataValue::Integer(10),
        ),
        PermissionMetadataEntry::new_with_context(
            parse_permission_metadata_key("steel:chat_color").expect("metadata key parses"),
            PermissionRuleContext::domain("lobby"),
            PermissionMetadataValue::String("green".to_owned()),
        ),
    ]);
    let data = PlayerPermissionData {
        groups: vec!["op".to_owned()],
        permissions: PermissionSet::from_entries([
            PermissionEntry::allow(
                PermissionKey::parse("minecraft.command.give").expect("key parses"),
            ),
            PermissionEntry::allow_with_context(
                PermissionKey::parse("steel.region.build").expect("key parses"),
                PermissionRuleContext::all([
                    PermissionRuleContext::world(Identifier::new("lobby", "spawn")),
                    PermissionRuleContext::custom(
                        PermissionContextKey::parse("region").expect("context key parses"),
                        "spawn",
                    )
                    .expect("custom context parses"),
                ])
                .expect("context chain parses"),
            ),
            PermissionEntry::deny_with_context(
                PermissionKey::parse("minecraft.command.stop").expect("key parses"),
                PermissionRuleContext::world(Identifier::new("lobby", "spawn")),
            ),
        ]),
        metadata: values,
    };

    let mut file = PlayerPermissionsFile::default();
    set_player_permission_entry(&mut file, uuid, &data);
    let written =
        serialize_player_permissions_file(&file).expect("player permissions should serialize");
    assert!(written.contains("[players.\"00000000-0000-0000-0000-000000000001\"]"));
    assert!(written.contains("allow = [\n    \"minecraft.command.give\","));
    assert!(written.contains("deny = [\n    \"minecraft.command.stop{world=lobby:spawn}\","));
    assert!(
        written.contains("    { key = \"steel:chat_color{domain=lobby}\", value = \"green\" },")
    );

    let parsed: PlayerPermissionsFile =
        toml::from_str(&written).expect("written player permissions should parse");
    let decoded = parsed
        .into_player_permission_data()
        .expect("player permissions should convert");

    assert_eq!(decoded, vec![(uuid, data)]);
}

#[test]
fn player_permissions_file_rejects_invalid_permission_expression() {
    let uuid = Uuid::from_u128(1);
    let file = PlayerPermissionEntryFile {
        groups: Vec::new(),
        allow: vec!["minecraft.command.stop{world=lobby:spawn/extra}".to_owned()],
        deny: Vec::new(),
        metadata: Vec::new(),
    };

    let error = file
        .into_player_permission_data(uuid)
        .expect_err("invalid loaded world context should be rejected");

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn player_permissions_file_rejects_invalid_metadata_expression() {
    let uuid = Uuid::from_u128(1);
    let file = PlayerPermissionEntryFile {
        groups: Vec::new(),
        allow: Vec::new(),
        deny: Vec::new(),
        metadata: vec![PlayerPermissionMetadataEntryFile {
            key: "steel:homes:limit".to_owned(),
            value: PermissionMetadataValue::Integer(10),
        }],
    };

    let error = file
        .into_player_permission_data(uuid)
        .expect_err("invalid metadata expression should be rejected");

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[tokio::test]
async fn update_global_merges_with_current_file_under_lock() {
    let root = temp_storage_root("global-update");
    let storage = PlayerDataStorage::new(root.clone(), StorageSelection::default_player_file())
        .await
        .expect("storage should initialize");
    let uuid = Uuid::from_u128(1);

    storage
        .save_global(
            uuid,
            &GlobalPlayerData {
                last_active_domain: "overworld".to_owned(),
            },
        )
        .await
        .expect("global data should save");

    storage
        .update_global(uuid, |global| {
            let mut global = global.expect("global data should exist");
            global.last_active_domain = "nether".to_owned();
            Ok::<_, io::Error>(global)
        })
        .await
        .expect("global data should update");

    let data = storage
        .load_global(uuid)
        .await
        .expect("global data should load")
        .expect("global data should exist");
    assert_eq!(data.last_active_domain, "nether");

    let _ = fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn player_permission_index_loads_persisted_permission_state() {
    let root = temp_storage_root("global-permissions");
    let storage = PlayerDataStorage::new(root.clone(), StorageSelection::default_player_file())
        .await
        .expect("storage should initialize");
    let op_uuid = Uuid::from_u128(1);
    let default_uuid = Uuid::from_u128(2);
    let op_permissions = PermissionSet::from_entries([PermissionEntry::allow(
        PermissionKey::parse("minecraft.command.give").expect("permission key parses"),
    )]);

    storage
        .save_player_permissions_if_current(
            op_uuid,
            &PlayerPermissionData {
                groups: vec!["op".to_owned()],
                permissions: op_permissions.clone(),
                metadata: PermissionMetadataSet::from_entries([PermissionMetadataEntry::new(
                    parse_permission_metadata_key("steel:homes")
                        .expect("permission metadata key parses"),
                    PermissionMetadataValue::Integer(10),
                )]),
            },
            || true,
        )
        .await
        .expect("op player permissions should save");
    storage
        .save_player_permissions_if_current(
            default_uuid,
            &PlayerPermissionData {
                groups: Vec::new(),
                permissions: PermissionSet::default(),
                metadata: PermissionMetadataSet::default(),
            },
            || true,
        )
        .await
        .expect("default player permissions should save");

    let index = storage
        .load_player_permission_states()
        .await
        .expect("permission index should load");

    let op_state = index.get(op_uuid).expect("op state should be indexed");
    assert_eq!(op_state.groups(), &["op".to_owned()]);
    assert_eq!(op_state.overrides(), &op_permissions);
    assert_eq!(
        op_state
            .metadata_overrides()
            .resolve(
                &parse_permission_metadata_key("steel:homes")
                    .expect("permission metadata key parses")
            )
            .and_then(PermissionMetadataValue::as_i64),
        Some(10)
    );
    assert!(index.get(default_uuid).is_none());

    let _ = fs::remove_dir_all(root).await;
}

#[test]
fn known_players_file_roundtrip_preserves_entries() {
    let players = KnownPlayers::from_entries([
        KnownPlayer::new(Uuid::from_u128(1), "Steve"),
        KnownPlayer::new(Uuid::from_u128(2), "Alex"),
    ]);

    let file = KnownPlayersFile::from_known_players(&players);
    let encoded = encode_known_players_file(&file).expect("known players file should encode");
    let decoded = decode_known_players_file(&encoded).expect("known players file should decode");
    let decoded = decoded
        .into_known_players()
        .expect("known players file should convert");

    assert_eq!(
        u16::from_le_bytes([encoded[4], encoded[5]]),
        KNOWN_PLAYERS_STORAGE_VERSION
    );
    assert_eq!(decoded, players);
}

#[tokio::test]
async fn stale_global_save_is_skipped() {
    let root = temp_storage_root("global");
    let storage = PlayerDataStorage::new(root.clone(), StorageSelection::default_player_file())
        .await
        .expect("storage should initialize");
    let uuid = Uuid::new_v4();
    let current = GlobalPlayerData {
        last_active_domain: "minecraft".to_owned(),
    };
    let stale = GlobalPlayerData {
        last_active_domain: "lobby".to_owned(),
    };

    assert!(
        storage
            .save_global_if_current(uuid, &current, || true)
            .await
            .expect("current save should succeed")
    );
    assert!(
        !storage
            .save_global_if_current(uuid, &stale, || false)
            .await
            .expect("stale save should be skipped")
    );

    let loaded = storage
        .load_global(uuid)
        .await
        .expect("global data should load")
        .expect("global data should exist");
    assert_eq!(loaded.last_active_domain, current.last_active_domain);

    let _ = fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn stale_player_permissions_save_is_skipped() {
    let root = temp_storage_root("permissions");
    let storage = PlayerDataStorage::new(root.clone(), StorageSelection::default_player_file())
        .await
        .expect("storage should initialize");
    let uuid = Uuid::new_v4();
    let current = PlayerPermissionData {
        groups: vec!["default".to_owned()],
        permissions: PermissionSet::default(),
        metadata: PermissionMetadataSet::default(),
    };
    let stale = PlayerPermissionData {
        groups: vec!["op".to_owned()],
        permissions: PermissionSet::default(),
        metadata: PermissionMetadataSet::default(),
    };

    assert!(
        storage
            .save_player_permissions_if_current(uuid, &current, || true)
            .await
            .expect("current player permissions should save")
    );
    assert!(
        !storage
            .save_player_permissions_if_current(uuid, &stale, || false)
            .await
            .expect("stale player permissions should be skipped")
    );

    let loaded = storage
        .load_player_permissions(uuid)
        .await
        .expect("player permissions should load")
        .expect("player permissions should exist");
    assert_eq!(loaded, current);

    let _ = fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn stale_known_players_save_is_skipped() {
    let root = temp_storage_root("known");
    let storage = PlayerDataStorage::new(root.clone(), StorageSelection::default_player_file())
        .await
        .expect("storage should initialize");
    let current = KnownPlayers::from_entries([KnownPlayer::new(Uuid::from_u128(1), "Steve")]);
    let stale = KnownPlayers::from_entries([KnownPlayer::new(Uuid::from_u128(2), "Alex")]);

    assert!(
        storage
            .save_known_players_if_current(&current, || true)
            .await
            .expect("current known-player save should succeed")
    );
    assert!(
        !storage
            .save_known_players_if_current(&stale, || false)
            .await
            .expect("stale known-player save should be skipped")
    );

    let loaded = storage
        .load_known_players()
        .await
        .expect("known players should load");
    assert_eq!(loaded, current);

    let _ = fs::remove_dir_all(root).await;
}

#[test]
fn player_file_roundtrip_preserves_root_vehicle() {
    let mut file = sample_player_file(PLAYER_DATA_VERSION);
    file.root_vehicle = Some(RootVehicleFile {
        attach: [3; 16],
        entity: sample_persistent_entity(),
    });

    let encoded = encode_player_file(&file).expect("player file should encode");
    let decoded = decode_player_file(&encoded).expect("player file should decode");
    let persistent = decoded
        .into_persistent()
        .expect("player file should convert");

    let Some(root_vehicle) = persistent.root_vehicle else {
        panic!("root vehicle should survive roundtrip");
    };
    assert_eq!(root_vehicle.attach, [3; 16]);
    assert_eq!(root_vehicle.entity.uuid, [7; 16]);
    assert_eq!(
        root_vehicle.entity.entity_type,
        Identifier::vanilla_static("minecart")
    );
    assert_eq!(
        root_vehicle.entity.pos.map(f64::to_bits),
        [4.0_f64.to_bits(), 65.0_f64.to_bits(), 6.0_f64.to_bits()]
    );
}

#[test]
fn stale_player_payload_version_is_rejected() {
    let file = sample_player_file(PLAYER_DATA_VERSION - 1);

    let error = file
        .into_persistent()
        .expect_err("stale payload should fail");

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}
