//! The names in every registry a 26.2 client must be told about before it can
//! join, in the order the server sends them.
//!
//! Only names. The contents stay on the client: a client that reported
//! `minecraft:core` in [`super::KNOWN_PACKS`] already has the data, so
//! `RegistrySynchronization.packRegistry` writes each element with an empty
//! payload and the client fills it in from its own copy. That is the whole
//! reason this table can be names alone -- see the `RegistryEntry::data`
//! documentation in the proto crate.
//!
//! Both lists were read off the 26.2 server jar rather than transcribed:
//! the registry order is `RegistryDataLoader.SYNCHRONIZED_REGISTRIES`, and the
//! entry names are the files the vanilla data generator writes under
//! `data/minecraft/<registry>/`, sorted by name.
//!
//! This is a stopgap. `hyperion-minecraft-proto`'s generated tables cover the
//! 95 *static* registries but not these 29 dynamic ones, so once that crate
//! generates registry contents this file should be deleted in favour of it.

/// A synchronised registry: element names in network-id order.
pub struct Registry {
    /// Registry name, e.g. `minecraft:dimension_type`.
    pub name: &'static str,
    /// Element names; the index into this slice is the network id.
    pub entries: &'static [&'static str],
}

impl Registry {
    /// The network id of `name`, if this registry has it.
    ///
    /// Linear, because these are looked up a handful of times per join rather
    /// than per packet.
    #[must_use]
    pub fn id_of(&self, name: &str) -> Option<i32> {
        let index = self.entries.iter().position(|entry| *entry == name)?;
        i32::try_from(index).ok()
    }
}

/// `minecraft:worldgen/biome` (66 entries).
pub static WORLDGEN_BIOME: Registry = Registry {
    name: "minecraft:worldgen/biome",
    entries: &[
        "minecraft:badlands",
        "minecraft:bamboo_jungle",
        "minecraft:basalt_deltas",
        "minecraft:beach",
        "minecraft:birch_forest",
        "minecraft:cherry_grove",
        "minecraft:cold_ocean",
        "minecraft:crimson_forest",
        "minecraft:dark_forest",
        "minecraft:deep_cold_ocean",
        "minecraft:deep_dark",
        "minecraft:deep_frozen_ocean",
        "minecraft:deep_lukewarm_ocean",
        "minecraft:deep_ocean",
        "minecraft:desert",
        "minecraft:dripstone_caves",
        "minecraft:end_barrens",
        "minecraft:end_highlands",
        "minecraft:end_midlands",
        "minecraft:eroded_badlands",
        "minecraft:flower_forest",
        "minecraft:forest",
        "minecraft:frozen_ocean",
        "minecraft:frozen_peaks",
        "minecraft:frozen_river",
        "minecraft:grove",
        "minecraft:ice_spikes",
        "minecraft:jagged_peaks",
        "minecraft:jungle",
        "minecraft:lukewarm_ocean",
        "minecraft:lush_caves",
        "minecraft:mangrove_swamp",
        "minecraft:meadow",
        "minecraft:mushroom_fields",
        "minecraft:nether_wastes",
        "minecraft:ocean",
        "minecraft:old_growth_birch_forest",
        "minecraft:old_growth_pine_taiga",
        "minecraft:old_growth_spruce_taiga",
        "minecraft:pale_garden",
        "minecraft:plains",
        "minecraft:river",
        "minecraft:savanna",
        "minecraft:savanna_plateau",
        "minecraft:small_end_islands",
        "minecraft:snowy_beach",
        "minecraft:snowy_plains",
        "minecraft:snowy_slopes",
        "minecraft:snowy_taiga",
        "minecraft:soul_sand_valley",
        "minecraft:sparse_jungle",
        "minecraft:stony_peaks",
        "minecraft:stony_shore",
        "minecraft:sulfur_caves",
        "minecraft:sunflower_plains",
        "minecraft:swamp",
        "minecraft:taiga",
        "minecraft:the_end",
        "minecraft:the_void",
        "minecraft:warm_ocean",
        "minecraft:warped_forest",
        "minecraft:windswept_forest",
        "minecraft:windswept_gravelly_hills",
        "minecraft:windswept_hills",
        "minecraft:windswept_savanna",
        "minecraft:wooded_badlands",
    ],
};

/// `minecraft:chat_type` (7 entries).
pub static CHAT_TYPE: Registry = Registry {
    name: "minecraft:chat_type",
    entries: &[
        "minecraft:chat",
        "minecraft:emote_command",
        "minecraft:msg_command_incoming",
        "minecraft:msg_command_outgoing",
        "minecraft:say_command",
        "minecraft:team_msg_command_incoming",
        "minecraft:team_msg_command_outgoing",
    ],
};

/// `minecraft:trim_pattern` (18 entries).
pub static TRIM_PATTERN: Registry = Registry {
    name: "minecraft:trim_pattern",
    entries: &[
        "minecraft:bolt",
        "minecraft:coast",
        "minecraft:dune",
        "minecraft:eye",
        "minecraft:flow",
        "minecraft:host",
        "minecraft:raiser",
        "minecraft:rib",
        "minecraft:sentry",
        "minecraft:shaper",
        "minecraft:silence",
        "minecraft:snout",
        "minecraft:spire",
        "minecraft:tide",
        "minecraft:vex",
        "minecraft:ward",
        "minecraft:wayfinder",
        "minecraft:wild",
    ],
};

/// `minecraft:trim_material` (11 entries).
pub static TRIM_MATERIAL: Registry = Registry {
    name: "minecraft:trim_material",
    entries: &[
        "minecraft:amethyst",
        "minecraft:copper",
        "minecraft:diamond",
        "minecraft:emerald",
        "minecraft:gold",
        "minecraft:iron",
        "minecraft:lapis",
        "minecraft:netherite",
        "minecraft:quartz",
        "minecraft:redstone",
        "minecraft:resin",
    ],
};

/// `minecraft:wolf_variant` (9 entries).
pub static WOLF_VARIANT: Registry = Registry {
    name: "minecraft:wolf_variant",
    entries: &[
        "minecraft:ashen",
        "minecraft:black",
        "minecraft:chestnut",
        "minecraft:pale",
        "minecraft:rusty",
        "minecraft:snowy",
        "minecraft:spotted",
        "minecraft:striped",
        "minecraft:woods",
    ],
};

/// `minecraft:wolf_sound_variant` (7 entries).
pub static WOLF_SOUND_VARIANT: Registry = Registry {
    name: "minecraft:wolf_sound_variant",
    entries: &[
        "minecraft:angry",
        "minecraft:big",
        "minecraft:classic",
        "minecraft:cute",
        "minecraft:grumpy",
        "minecraft:puglin",
        "minecraft:sad",
    ],
};

/// `minecraft:pig_variant` (3 entries).
pub static PIG_VARIANT: Registry = Registry {
    name: "minecraft:pig_variant",
    entries: &["minecraft:cold", "minecraft:temperate", "minecraft:warm"],
};

/// `minecraft:pig_sound_variant` (3 entries).
pub static PIG_SOUND_VARIANT: Registry = Registry {
    name: "minecraft:pig_sound_variant",
    entries: &["minecraft:big", "minecraft:classic", "minecraft:mini"],
};

/// `minecraft:frog_variant` (3 entries).
pub static FROG_VARIANT: Registry = Registry {
    name: "minecraft:frog_variant",
    entries: &["minecraft:cold", "minecraft:temperate", "minecraft:warm"],
};

/// `minecraft:cat_variant` (11 entries).
pub static CAT_VARIANT: Registry = Registry {
    name: "minecraft:cat_variant",
    entries: &[
        "minecraft:all_black",
        "minecraft:black",
        "minecraft:british_shorthair",
        "minecraft:calico",
        "minecraft:jellie",
        "minecraft:persian",
        "minecraft:ragdoll",
        "minecraft:red",
        "minecraft:siamese",
        "minecraft:tabby",
        "minecraft:white",
    ],
};

/// `minecraft:cat_sound_variant` (2 entries).
pub static CAT_SOUND_VARIANT: Registry = Registry {
    name: "minecraft:cat_sound_variant",
    entries: &["minecraft:classic", "minecraft:royal"],
};

/// `minecraft:cow_sound_variant` (2 entries).
pub static COW_SOUND_VARIANT: Registry = Registry {
    name: "minecraft:cow_sound_variant",
    entries: &["minecraft:classic", "minecraft:moody"],
};

/// `minecraft:cow_variant` (3 entries).
pub static COW_VARIANT: Registry = Registry {
    name: "minecraft:cow_variant",
    entries: &["minecraft:cold", "minecraft:temperate", "minecraft:warm"],
};

/// `minecraft:chicken_sound_variant` (2 entries).
pub static CHICKEN_SOUND_VARIANT: Registry = Registry {
    name: "minecraft:chicken_sound_variant",
    entries: &["minecraft:classic", "minecraft:picky"],
};

/// `minecraft:chicken_variant` (3 entries).
pub static CHICKEN_VARIANT: Registry = Registry {
    name: "minecraft:chicken_variant",
    entries: &["minecraft:cold", "minecraft:temperate", "minecraft:warm"],
};

/// `minecraft:zombie_nautilus_variant` (2 entries).
pub static ZOMBIE_NAUTILUS_VARIANT: Registry = Registry {
    name: "minecraft:zombie_nautilus_variant",
    entries: &["minecraft:temperate", "minecraft:warm"],
};

/// `minecraft:painting_variant` (51 entries).
pub static PAINTING_VARIANT: Registry = Registry {
    name: "minecraft:painting_variant",
    entries: &[
        "minecraft:alban",
        "minecraft:aztec",
        "minecraft:aztec2",
        "minecraft:backyard",
        "minecraft:baroque",
        "minecraft:bomb",
        "minecraft:bouquet",
        "minecraft:burning_skull",
        "minecraft:bust",
        "minecraft:cavebird",
        "minecraft:changing",
        "minecraft:cotan",
        "minecraft:courbet",
        "minecraft:creebet",
        "minecraft:dennis",
        "minecraft:donkey_kong",
        "minecraft:earth",
        "minecraft:endboss",
        "minecraft:fern",
        "minecraft:fighters",
        "minecraft:finding",
        "minecraft:fire",
        "minecraft:graham",
        "minecraft:humble",
        "minecraft:kebab",
        "minecraft:lowmist",
        "minecraft:match",
        "minecraft:meditative",
        "minecraft:orb",
        "minecraft:owlemons",
        "minecraft:passage",
        "minecraft:pigscene",
        "minecraft:plant",
        "minecraft:pointer",
        "minecraft:pond",
        "minecraft:pool",
        "minecraft:prairie_ride",
        "minecraft:sea",
        "minecraft:skeleton",
        "minecraft:skull_and_roses",
        "minecraft:stage",
        "minecraft:sunflowers",
        "minecraft:sunset",
        "minecraft:tides",
        "minecraft:unpacked",
        "minecraft:void",
        "minecraft:wanderer",
        "minecraft:wasteland",
        "minecraft:water",
        "minecraft:wind",
        "minecraft:wither",
    ],
};

/// `minecraft:sulfur_cube_archetype` (12 entries).
pub static SULFUR_CUBE_ARCHETYPE: Registry = Registry {
    name: "minecraft:sulfur_cube_archetype",
    entries: &[
        "minecraft:bouncy",
        "minecraft:explosive",
        "minecraft:fast_flat",
        "minecraft:fast_sliding",
        "minecraft:high_resistance",
        "minecraft:hot",
        "minecraft:light",
        "minecraft:regular",
        "minecraft:slow_bouncy",
        "minecraft:slow_flat",
        "minecraft:slow_sliding",
        "minecraft:sticky",
    ],
};

/// `minecraft:dimension_type` (4 entries).
pub static DIMENSION_TYPE: Registry = Registry {
    name: "minecraft:dimension_type",
    entries: &[
        "minecraft:overworld",
        "minecraft:overworld_caves",
        "minecraft:the_end",
        "minecraft:the_nether",
    ],
};

/// `minecraft:damage_type` (51 entries).
pub static DAMAGE_TYPE: Registry = Registry {
    name: "minecraft:damage_type",
    entries: &[
        "minecraft:arrow",
        "minecraft:bad_respawn_point",
        "minecraft:cactus",
        "minecraft:campfire",
        "minecraft:cramming",
        "minecraft:dragon_breath",
        "minecraft:drown",
        "minecraft:dry_out",
        "minecraft:ender_pearl",
        "minecraft:explosion",
        "minecraft:fall",
        "minecraft:falling_anvil",
        "minecraft:falling_block",
        "minecraft:falling_stalactite",
        "minecraft:fireball",
        "minecraft:fireworks",
        "minecraft:fly_into_wall",
        "minecraft:freeze",
        "minecraft:generic",
        "minecraft:generic_kill",
        "minecraft:hot_floor",
        "minecraft:in_fire",
        "minecraft:in_wall",
        "minecraft:indirect_magic",
        "minecraft:lava",
        "minecraft:lightning_bolt",
        "minecraft:mace_smash",
        "minecraft:magic",
        "minecraft:mob_attack",
        "minecraft:mob_attack_no_aggro",
        "minecraft:mob_projectile",
        "minecraft:on_fire",
        "minecraft:out_of_world",
        "minecraft:outside_border",
        "minecraft:player_attack",
        "minecraft:player_explosion",
        "minecraft:sonic_boom",
        "minecraft:spear",
        "minecraft:spit",
        "minecraft:stalagmite",
        "minecraft:starve",
        "minecraft:sting",
        "minecraft:sulfur_cube_hot",
        "minecraft:sweet_berry_bush",
        "minecraft:thorns",
        "minecraft:thrown",
        "minecraft:trident",
        "minecraft:unattributed_fireball",
        "minecraft:wind_charge",
        "minecraft:wither",
        "minecraft:wither_skull",
    ],
};

/// `minecraft:banner_pattern` (43 entries).
pub static BANNER_PATTERN: Registry = Registry {
    name: "minecraft:banner_pattern",
    entries: &[
        "minecraft:base",
        "minecraft:border",
        "minecraft:bricks",
        "minecraft:circle",
        "minecraft:creeper",
        "minecraft:cross",
        "minecraft:curly_border",
        "minecraft:diagonal_left",
        "minecraft:diagonal_right",
        "minecraft:diagonal_up_left",
        "minecraft:diagonal_up_right",
        "minecraft:flow",
        "minecraft:flower",
        "minecraft:globe",
        "minecraft:gradient",
        "minecraft:gradient_up",
        "minecraft:guster",
        "minecraft:half_horizontal",
        "minecraft:half_horizontal_bottom",
        "minecraft:half_vertical",
        "minecraft:half_vertical_right",
        "minecraft:mojang",
        "minecraft:piglin",
        "minecraft:rhombus",
        "minecraft:skull",
        "minecraft:small_stripes",
        "minecraft:square_bottom_left",
        "minecraft:square_bottom_right",
        "minecraft:square_top_left",
        "minecraft:square_top_right",
        "minecraft:straight_cross",
        "minecraft:stripe_bottom",
        "minecraft:stripe_center",
        "minecraft:stripe_downleft",
        "minecraft:stripe_downright",
        "minecraft:stripe_left",
        "minecraft:stripe_middle",
        "minecraft:stripe_right",
        "minecraft:stripe_top",
        "minecraft:triangle_bottom",
        "minecraft:triangle_top",
        "minecraft:triangles_bottom",
        "minecraft:triangles_top",
    ],
};

/// `minecraft:enchantment` (43 entries).
pub static ENCHANTMENT: Registry = Registry {
    name: "minecraft:enchantment",
    entries: &[
        "minecraft:aqua_affinity",
        "minecraft:bane_of_arthropods",
        "minecraft:binding_curse",
        "minecraft:blast_protection",
        "minecraft:breach",
        "minecraft:channeling",
        "minecraft:density",
        "minecraft:depth_strider",
        "minecraft:efficiency",
        "minecraft:feather_falling",
        "minecraft:fire_aspect",
        "minecraft:fire_protection",
        "minecraft:flame",
        "minecraft:fortune",
        "minecraft:frost_walker",
        "minecraft:impaling",
        "minecraft:infinity",
        "minecraft:knockback",
        "minecraft:looting",
        "minecraft:loyalty",
        "minecraft:luck_of_the_sea",
        "minecraft:lunge",
        "minecraft:lure",
        "minecraft:mending",
        "minecraft:multishot",
        "minecraft:piercing",
        "minecraft:power",
        "minecraft:projectile_protection",
        "minecraft:protection",
        "minecraft:punch",
        "minecraft:quick_charge",
        "minecraft:respiration",
        "minecraft:riptide",
        "minecraft:sharpness",
        "minecraft:silk_touch",
        "minecraft:smite",
        "minecraft:soul_speed",
        "minecraft:sweeping_edge",
        "minecraft:swift_sneak",
        "minecraft:thorns",
        "minecraft:unbreaking",
        "minecraft:vanishing_curse",
        "minecraft:wind_burst",
    ],
};

/// `minecraft:jukebox_song` (22 entries).
pub static JUKEBOX_SONG: Registry = Registry {
    name: "minecraft:jukebox_song",
    entries: &[
        "minecraft:11",
        "minecraft:13",
        "minecraft:5",
        "minecraft:blocks",
        "minecraft:bounce",
        "minecraft:cat",
        "minecraft:chirp",
        "minecraft:creator",
        "minecraft:creator_music_box",
        "minecraft:far",
        "minecraft:lava_chicken",
        "minecraft:mall",
        "minecraft:mellohi",
        "minecraft:otherside",
        "minecraft:pigstep",
        "minecraft:precipice",
        "minecraft:relic",
        "minecraft:stal",
        "minecraft:strad",
        "minecraft:tears",
        "minecraft:wait",
        "minecraft:ward",
    ],
};

/// `minecraft:instrument` (8 entries).
pub static INSTRUMENT: Registry = Registry {
    name: "minecraft:instrument",
    entries: &[
        "minecraft:admire_goat_horn",
        "minecraft:call_goat_horn",
        "minecraft:dream_goat_horn",
        "minecraft:feel_goat_horn",
        "minecraft:ponder_goat_horn",
        "minecraft:seek_goat_horn",
        "minecraft:sing_goat_horn",
        "minecraft:yearn_goat_horn",
    ],
};

/// `minecraft:test_environment` (1 entries).
pub static TEST_ENVIRONMENT: Registry = Registry {
    name: "minecraft:test_environment",
    entries: &["minecraft:default"],
};

/// `minecraft:test_instance` (1 entries).
pub static TEST_INSTANCE: Registry = Registry {
    name: "minecraft:test_instance",
    entries: &["minecraft:always_pass"],
};

/// `minecraft:dialog` (3 entries).
pub static DIALOG: Registry = Registry {
    name: "minecraft:dialog",
    entries: &[
        "minecraft:custom_options",
        "minecraft:quick_actions",
        "minecraft:server_links",
    ],
};

/// `minecraft:world_clock` (2 entries).
pub static WORLD_CLOCK: Registry = Registry {
    name: "minecraft:world_clock",
    entries: &["minecraft:overworld", "minecraft:the_end"],
};

/// `minecraft:timeline` (4 entries).
pub static TIMELINE: Registry = Registry {
    name: "minecraft:timeline",
    entries: &[
        "minecraft:day",
        "minecraft:early_game",
        "minecraft:moon",
        "minecraft:villager_schedule",
    ],
};

/// Every synchronised registry, in `RegistryDataLoader.SYNCHRONIZED_REGISTRIES`
/// order.
pub static SYNCHRONIZED: &[&Registry] = &[
    &WORLDGEN_BIOME,
    &CHAT_TYPE,
    &TRIM_PATTERN,
    &TRIM_MATERIAL,
    &WOLF_VARIANT,
    &WOLF_SOUND_VARIANT,
    &PIG_VARIANT,
    &PIG_SOUND_VARIANT,
    &FROG_VARIANT,
    &CAT_VARIANT,
    &CAT_SOUND_VARIANT,
    &COW_SOUND_VARIANT,
    &COW_VARIANT,
    &CHICKEN_SOUND_VARIANT,
    &CHICKEN_VARIANT,
    &ZOMBIE_NAUTILUS_VARIANT,
    &PAINTING_VARIANT,
    &SULFUR_CUBE_ARCHETYPE,
    &DIMENSION_TYPE,
    &DAMAGE_TYPE,
    &BANNER_PATTERN,
    &ENCHANTMENT,
    &JUKEBOX_SONG,
    &INSTRUMENT,
    &TEST_ENVIRONMENT,
    &TEST_INSTANCE,
    &DIALOG,
    &WORLD_CLOCK,
    &TIMELINE,
];
