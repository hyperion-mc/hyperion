//! Wire tests for the hand-written player-facing play packets.
//!
//! # Where the expected bytes came from
//!
//! Every vector here was produced by the 26.2 server's own encoders. A small
//! Java program was compiled against the classes inside
//! `META-INF/versions/26.2/server-26.2.jar` (with `META-INF/libraries` on the
//! classpath), bootstrapped with `SharedConstants.tryDetectVersion()` and
//! `Bootstrap.bootStrap()`, and each packet encoded through its own
//! `STREAM_CODEC` into a `RegistryFriendlyByteBuf`, printing the readable bytes
//! as hex. The Java value each vector came from is written out beside it.
//!
//! So a passing test says this crate agrees with Mojang's encoder, not that it
//! agrees with itself. The one exception is [`display_slot_ids`], which covers
//! an enum rather than a packet and so has no `STREAM_CODEC` to run; its two
//! vectors are declaration indices read off `DisplaySlot`, and the test says so.
//!
//! # Why one test compares documents rather than bytes
//!
//! `CompoundTag` is a `HashMap`, so the key order in a vanilla NBT encoding is
//! whatever the hash table produced and no implementation can reproduce it on
//! purpose. Every vector below whose components collapse to bare strings is
//! therefore byte-exact; [`system_chat_styled`] is the one that does not, and
//! it compares decoded values instead. [`crate::text`]'s own tests make the
//! same split for the same reason.

use hyperion_minecraft_proto::{
    Decode, Encode, Reader, Uuid, Writer,
    packets::{
        play::{
            clientbound::{SetDisplayObjective, SystemChat},
            player::{
                AbilityFlags, ArgumentType, CommandNode, CommandNodeStub, Commands, DisplaySlot,
                NumberFormat, ObjectiveDisplay, ObjectiveRenderType, PlayerAbilities,
                PlayerInfoActions, PlayerInfoEntry, PlayerInfoUpdate, PlayerProfile, SetObjective,
                SetPlayerTeam, SetScore, StringArgumentKind, TeamCollisionRule, TeamColor,
                TeamOptions, TeamParameters, TeamVisibility,
            },
        },
        play_login::GameType,
    },
    text::{Component, NamedColor, Style, TextColor},
    types::game_profile::Property,
};

/// The harness prints hex, so the fixtures are hex.
fn hex(text: &str) -> Vec<u8> {
    assert!(
        text.len().is_multiple_of(2),
        "hex fixture has an odd length"
    );
    (0..text.len() / 2)
        .map(|index| {
            u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).expect("hex fixture digit")
        })
        .collect()
}

fn encode<T: Encode>(value: &T) -> Vec<u8> {
    let mut writer = Writer::new();
    value.encode(&mut writer).expect("encode");
    writer.into_vec()
}

fn decode<'a, T: Decode<'a>>(bytes: &'a [u8]) -> T {
    let mut reader = Reader::new(bytes);
    let value = T::decode(&mut reader).expect("decode");
    reader.finish().expect("packet body fully consumed");
    value
}

/// Assert `value` encodes to exactly `bytes` and that `bytes` decode back to it.
fn round_trip<'a, T>(value: &T, bytes: &'a [u8])
where
    T: Encode + Decode<'a> + PartialEq + std::fmt::Debug,
{
    assert_eq!(encode(value), bytes, "encoding mismatch");
    assert_eq!(&decode::<T>(bytes), value, "decoding mismatch");
}

/// Assert `value` encodes to exactly `bytes`, and that decoding those bytes and
/// encoding the result reproduces them.
///
/// For a [`PlayerInfoUpdate`] whose actions select only some fields, the rest
/// never reach the wire, so a decode cannot recover them and comparing the
/// decoded value against `value` would only be testing the defaults this file
/// chose. Re-encoding is the round trip the packet actually has.
fn re_encodes<'a, T>(value: &T, bytes: &'a [u8])
where
    T: Encode + Decode<'a> + std::fmt::Debug,
{
    assert_eq!(encode(value), bytes, "encoding mismatch");
    assert_eq!(encode(&decode::<T>(bytes)), bytes, "re-encoding mismatch");
}

/// `UUID.fromString("00112233-4455-6677-8899-aabbccddeeff")`, the id every
/// player fixture uses. Every byte is distinct, so a transposed half shows up.
const PROFILE_ID: Uuid = Uuid(0x0011_2233_4455_6677_8899_aabb_ccdd_eeff);

/// `new GameProfile(PROFILE_ID, "Notch", <one signed textures property>)`.
fn notch_profile() -> PlayerProfile<'static> {
    PlayerProfile {
        name: "Notch",
        properties: vec![Property {
            name: "textures",
            value: "dGV4dHVyZQ==",
            signature: Some("c2lnbmF0dXJl"),
        }],
    }
}

/// `new Entry(PROFILE_ID, profile, true, 42, CREATIVE, literal("Notch"), true,
/// 7, null)`.
fn notch_entry() -> PlayerInfoEntry<'static> {
    PlayerInfoEntry {
        profile_id: PROFILE_ID,
        profile: Some(notch_profile()),
        chat_session: None,
        game_mode: GameType::Creative,
        listed: true,
        latency: 42,
        display_name: Some(Component::text("Notch")),
        list_order: 7,
        show_hat: true,
    }
}

// --- player info update ---------------------------------------------------

/// The bitmask and the per-entry field selection have to agree, and this is the
/// case that proves it: two actions whose ordinals are not adjacent, so a mask
/// built from the wrong bit positions and a field list built from the wrong
/// order fail independently.
///
/// `EnumSet.of(ADD_PLAYER, UPDATE_LATENCY)` is ordinals 0 and 4, so the mask is
/// `0x11`. Each entry then carries its uuid, the `ADD_PLAYER` payload and the
/// `UPDATE_LATENCY` varint, and nothing else: no game mode, no listed flag, no
/// display name.
#[test]
fn player_info_update_two_actions() {
    let packet = PlayerInfoUpdate {
        actions: PlayerInfoActions::ADD_PLAYER.union(PlayerInfoActions::UPDATE_LATENCY),
        entries: vec![notch_entry()],
    };
    assert_eq!(packet.actions.to_bits(), 0x11);
    re_encodes(
        &packet,
        &hex(
            "11\
             01\
             00112233445566778899aabbccddeeff\
             054e6f746368\
             01\
             0874657874757265730c64475634644856795a513d3d010c63326c6e626d463064584a6c\
             2a",
        ),
    );
}

/// The other half of the same proof: two actions in the *high* bits, whose
/// payloads are single bytes. A mask numbered from the wrong end would put
/// these at `0x11` too.
///
/// `EnumSet.of(UPDATE_LISTED, UPDATE_HAT)` is ordinals 3 and 7, so `0x88`.
#[test]
fn player_info_update_high_bits() {
    let packet = PlayerInfoUpdate {
        actions: PlayerInfoActions::UPDATE_LISTED.union(PlayerInfoActions::UPDATE_HAT),
        entries: vec![notch_entry()],
    };
    assert_eq!(packet.actions.to_bits(), 0x88);
    re_encodes(&packet, &hex("880100112233445566778899aabbccddeeff0101"));
}

/// Every action but `INITIALIZE_CHAT`, whose payload is a signed profile key
/// the harness has no way to build. Seven of eight bits, so `0xfd`.
#[test]
fn player_info_update_every_action() {
    let actions = PlayerInfoActions::ADD_PLAYER
        .union(PlayerInfoActions::UPDATE_GAME_MODE)
        .union(PlayerInfoActions::UPDATE_LISTED)
        .union(PlayerInfoActions::UPDATE_LATENCY)
        .union(PlayerInfoActions::UPDATE_DISPLAY_NAME)
        .union(PlayerInfoActions::UPDATE_LIST_ORDER)
        .union(PlayerInfoActions::UPDATE_HAT);
    assert_eq!(actions.to_bits(), 0xfd);

    round_trip(
        &PlayerInfoUpdate {
            actions,
            entries: vec![notch_entry()],
        },
        &hex(
            "fd\
             01\
             00112233445566778899aabbccddeeff\
             054e6f746368\
             01\
             0874657874757265730c64475634644856795a513d3d010c63326c6e626d463064584a6c\
             01\
             01\
             2a\
             010800054e6f746368\
             07\
             01",
        ),
    );
}

/// The two absent-value shapes: a profile with no properties, and an
/// `UPDATE_DISPLAY_NAME` whose value is null. Both are a bare zero byte, and
/// nothing in the body distinguishes them from a missing field, which is why
/// they are worth pinning.
#[test]
fn player_info_update_absent_values() {
    round_trip(
        &PlayerInfoUpdate {
            actions: PlayerInfoActions::ADD_PLAYER.union(PlayerInfoActions::UPDATE_DISPLAY_NAME),
            entries: vec![PlayerInfoEntry {
                profile_id: PROFILE_ID,
                profile: Some(PlayerProfile {
                    name: "Bare",
                    properties: Vec::new(),
                }),
                ..PlayerInfoEntry::default()
            }],
        },
        &hex("210100112233445566778899aabbccddeeff04426172650000"),
    );
}

/// No actions and no entries: two zero bytes, and the smallest thing this
/// packet can be.
///
/// The first of those two is the whole action bitmask, so this is also what
/// pins its width. `Action` has eight constants and
/// `positiveCeilDiv(8, 8)` is one byte; a ninth would make it two and shift
/// every entry in every other packet, and this test is where that shows up.
#[test]
fn player_info_update_empty() {
    round_trip(
        &PlayerInfoUpdate {
            actions: PlayerInfoActions::NONE,
            entries: Vec::new(),
        },
        &hex("0000"),
    );
}

/// An entry the actions promise a profile for and that has none cannot be
/// written, because a reader sizes the entry off the mask alone and would read
/// the next entry's uuid as this one's name.
#[test]
fn player_info_update_rejects_missing_profile() {
    let packet = PlayerInfoUpdate {
        actions: PlayerInfoActions::ADD_PLAYER,
        entries: vec![PlayerInfoEntry {
            profile_id: PROFILE_ID,
            ..PlayerInfoEntry::default()
        }],
    };
    let mut writer = Writer::new();
    assert!(packet.encode(&mut writer).is_err());
}

// --- system chat ----------------------------------------------------------

/// `new ClientboundSystemChatPacket(Component.literal("hello"), false)`.
///
/// A literal with no style collapses to a bare NBT string, so this one is
/// byte-exact.
#[test]
fn system_chat_plain() {
    let text = Component::text("hello");
    round_trip(
        &SystemChat {
            content: text.to_tag(),
            overlay: false,
        },
        &hex("08000568656c6c6f00"),
    );
}

/// `literal("Server").withStyle(GOLD, BOLD).append(literal(" restarting")
/// .withStyle(RED))`, sent as an overlay.
///
/// The compound has four keys, so vanilla's byte order is its `HashMap`'s and
/// not reproducible; the assertion is that both sides are the same NBT
/// document, which [`hyperion_minecraft_proto::nbt::Compound`]'s
/// order-insensitive equality is what makes checkable.
#[test]
fn system_chat_styled() {
    let styled = Component::text("Server")
        .with_style(Style {
            color: Some(TextColor::Named(NamedColor::Gold)),
            bold: Some(true),
            ..Style::new()
        })
        .append(Component::text(" restarting").with_style(Style {
            color: Some(TextColor::Named(NamedColor::Red)),
            ..Style::new()
        }));
    let packet = SystemChat {
        content: styled.to_tag(),
        overlay: true,
    };

    let vanilla = hex(
        "0a\
         080005636f6c6f720004676f6c64\
         09000565787472610a00000001080005636f6c6f72000372656408000474657874000b2072657374617274696e6700\
         080004746578740006536572766572\
         010004626f6c6401\
         00\
         01",
    );

    assert_eq!(
        decode::<SystemChat<'_>>(&vanilla),
        packet,
        "vanilla's bytes are a different document"
    );
    let mine = encode(&packet);
    assert_eq!(
        decode::<SystemChat<'_>>(&mine),
        decode::<SystemChat<'_>>(&vanilla),
        "encoded a different document than vanilla: {mine:02x?}"
    );
    assert!(
        packet.overlay,
        "the overlay flag is the last byte and easy to lose"
    );
}

// --- abilities ------------------------------------------------------------

/// `Abilities` with invulnerable, mayfly and instabuild set but not flying, at
/// vanilla's default speeds.
#[test]
fn player_abilities() {
    round_trip(
        &PlayerAbilities {
            flags: AbilityFlags::INVULNERABLE
                .union(AbilityFlags::CAN_FLY)
                .union(AbilityFlags::INSTABUILD),
            flying_speed: 0.05,
            walking_speed: 0.1,
        },
        &hex("0d3d4ccccd3dcccccd"),
    );
}

// --- command tree ---------------------------------------------------------

/// `/team <name>` with `name` a `StringArgumentType.word()` that asks the
/// server for completions.
///
/// Three nodes: the root, the literal, and an executable argument whose flags
/// are `TYPE_ARGUMENT | FLAG_EXECUTABLE | FLAG_CUSTOM_SUGGESTIONS`, i.e.
/// `0x16`. The argument type is written as its `minecraft:command_argument_type`
/// id, which is 5 for `brigadier:string` in 776 and is the number that moved
/// from 1.20.1.
#[test]
fn commands_literal_and_argument() {
    let packet = Commands {
        nodes: vec![
            CommandNode {
                children: vec![1],
                ..CommandNode::default()
            },
            CommandNode {
                children: vec![2],
                stub: CommandNodeStub::Literal { name: "team" },
                ..CommandNode::default()
            },
            CommandNode {
                stub: CommandNodeStub::Argument {
                    name: "name",
                    parser: ArgumentType::String(StringArgumentKind::SingleWord),
                    suggestions: Some(
                        hyperion_minecraft_proto::Identifier::new("minecraft:ask_server")
                            .expect("valid identifier"),
                    ),
                },
                executable: true,
                ..CommandNode::default()
            },
        ],
        root_index: 0,
    };
    round_trip(
        &packet,
        &hex(
            "03\
             000101\
             010102047465616d\
             1600046e616d650500146d696e6563726166743a61736b5f736572766572\
             00",
        ),
    );
}

// --- scoreboard -----------------------------------------------------------

/// A score with both optional fields present, the second of them a
/// `FixedFormat` whose id comes out of `minecraft:number_format_type`.
#[test]
fn set_score_with_display_and_format() {
    round_trip(
        &SetScore {
            owner: "Notch",
            objective_name: "kills",
            score: 7,
            display: Some(Component::text("seven")),
            number_format: Some(NumberFormat::Fixed(Box::new(Component::text("**")))),
        },
        &hex("054e6f746368056b696c6c730701080005736576656e01020800022a2a"),
    );
}

/// A negative score, which is a five-byte `VarInt` and the case a naive
/// zig-zag encoding gets wrong, with both optional fields absent.
#[test]
fn set_score_bare() {
    round_trip(
        &SetScore {
            owner: "Notch",
            objective_name: "kills",
            score: -3,
            display: None,
            number_format: None,
        },
        &hex("054e6f746368056b696c6c73fdffffff0f0000"),
    );
}

/// `METHOD_ADD`, which carries the display fields, against `METHOD_REMOVE`,
/// which ends after the method byte.
#[test]
fn set_objective_add_and_remove() {
    round_trip(
        &SetObjective {
            objective_name: "kills",
            display: Some(ObjectiveDisplay {
                display_name: Component::text("Kills"),
                render_type: ObjectiveRenderType::Hearts,
                number_format: Some(NumberFormat::Blank),
            }),
            change: false,
        },
        &hex("056b696c6c73000800054b696c6c73010100"),
    );
    round_trip(
        &SetObjective {
            objective_name: "kills",
            display: None,
            change: false,
        },
        &hex("056b696c6c7301"),
    );
}

/// The one method that carries both tails, against the two that carry one each.
#[test]
fn set_player_team_methods() {
    round_trip(
        &SetPlayerTeam {
            name: "reds",
            method: SetPlayerTeam::METHOD_ADD,
            parameters: Some(TeamParameters {
                display_name: Component::text("Reds"),
                player_prefix: Component::text("["),
                player_suffix: Component::text("]"),
                name_tag_visibility: TeamVisibility::HideForOtherTeams,
                collision_rule: TeamCollisionRule::PushOwnTeam,
                color: Some(TeamColor::DarkRed),
                options: TeamOptions::ALLOW_FRIENDLY_FIRE
                    .union(TeamOptions::SEE_FRIENDLY_INVISIBLES),
            }),
            players: vec!["Notch", "Bare"],
        },
        &hex("047265647300080004526564730800015b0800015d020301040302054e6f7463680442617265"),
    );
    round_trip(
        &SetPlayerTeam {
            name: "reds",
            method: SetPlayerTeam::METHOD_LEAVE,
            parameters: None,
            players: vec!["Notch"],
        },
        &hex("04726564730401054e6f746368"),
    );
    round_trip(
        &SetPlayerTeam {
            name: "reds",
            method: SetPlayerTeam::METHOD_REMOVE,
            parameters: None,
            players: Vec::new(),
        },
        &hex("047265647301"),
    );
}

/// The two ends of `DisplaySlot`, whose ids are declaration indices.
///
/// Unlike every other vector in this file these two were not printed by the
/// Java harness: `DisplaySlot` is an enum and not a packet, so there is nothing
/// to hand a `STREAM_CODEC`. They are the ordinals of the first and last
/// constant, and what they defend is the sixteen team slots in between staying
/// spelled out. Dropping one shifts `TeamWhite` to 17 and fails here.
#[test]
fn display_slot_ids() {
    round_trip(&DisplaySlot::Sidebar, &hex("01"));
    round_trip(&DisplaySlot::TeamWhite, &hex("12"));
    assert_eq!(DisplaySlot::Sidebar.id(), 1);
    assert_eq!(DisplaySlot::TeamWhite.id(), 18);
}

/// The sidebar slot in the packet that carries it, which is the one call the
/// server makes.
#[test]
fn set_display_objective_sidebar() {
    round_trip(
        &SetDisplayObjective {
            id: DisplaySlot::Sidebar.id(),
            objective_name: "smash",
        },
        &hex("0105736d617368"),
    );
}
