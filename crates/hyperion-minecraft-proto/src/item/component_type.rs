//! The 111 data component types of protocol 776, and their wire shapes.
//!
//! Ids are positional in the `minecraft:data_component_type` registry, so the
//! discriminants here are the ids on the wire, and the names are read from the
//! generated registry table rather than restated.
//!
//! Every shape is transcribed from the `StreamCodec` the server builds for that
//! type in `net.minecraft.core.component.DataComponents`. Where a registration
//! has no `.networkSynchronized(...)`,
//! `DataComponentType.Builder.build` substitutes
//! `ByteBufCodecs.fromCodecWithRegistries(codec)`, which writes the value as a
//! single NBT tag; those seven are marked below.
//!
//! Composite shapes are `static` items rather than expressions inside
//! [`ComponentType::shape`]: a shape that mentions another shape by value can
//! only be built in a `static`, and `MOB_EFFECT_DETAILS` has to be one anyway
//! because it contains itself.

use crate::{
    Error, Reader, Result,
    generated::registry,
    item::shape::{OutOfRange, Shape},
};

// ---------------------------------------------------------------------------
// Shapes shared by several component types.
// ---------------------------------------------------------------------------

/// `SoundEvent.STREAM_CODEC`: a holder whose inline form is an `Identifier`
/// plus an optional fixed range.
static SOUND_EVENT: Shape = Shape::Holder(&Shape::Seq(&[Shape::Str, Shape::Optional(&Shape::Int)]));

/// `ItemStackTemplate.STREAM_CODEC`.
///
/// Note the field order: item id first, then count. That is the opposite of
/// `ItemStack.STREAM_CODEC`, which leads with the count so that a zero can end
/// the read early. Stacks nested inside components all take the template form,
/// which has no empty encoding and so needs no leading count.
static STACK_TEMPLATE: Shape = Shape::Seq(&[Shape::VarInt, Shape::VarInt, Shape::Patch]);

/// `MobEffectInstance.Details.STREAM_CODEC`, which contains itself: when a
/// shorter effect masks a longer one, the longer one rides along inside it.
static MOB_EFFECT_DETAILS: Shape = Shape::Seq(&[
    Shape::VarInt,
    Shape::VarInt,
    Shape::Byte,
    Shape::Byte,
    Shape::Byte,
    Shape::Optional(&MOB_EFFECT_DETAILS),
]);

/// `MobEffectInstance.STREAM_CODEC`: the effect holder, then its details.
static MOB_EFFECT_INSTANCE: Shape = Shape::Seq(&[Shape::VarInt, MOB_EFFECT_DETAILS]);

/// `ConsumeEffect.STREAM_CODEC`: a `minecraft:consume_effect_type` registry id
/// dispatching to that effect's codec. Variant order is the registry's own,
/// from the data generator's `registries.json`.
static CONSUME_EFFECT: Shape = Shape::Dispatch {
    variants: &[
        // apply_effects: the effects, then a probability.
        Shape::Seq(&[Shape::List(&MOB_EFFECT_INSTANCE), Shape::Int]),
        // remove_effects
        Shape::HolderSet,
        // clear_all_effects
        Shape::Unit,
        // teleport_randomly: a diameter.
        Shape::Int,
        // play_sound
        SOUND_EVENT,
    ],
    // A registry lookup, which throws on an id it does not know.
    out_of_range: OutOfRange::Reject,
};

/// `Filterable.streamCodec(...)` over a string: the raw text, then the
/// server-filtered replacement when there is one.
static FILTERABLE_STR: Shape = Shape::Seq(&[Shape::Str, Shape::Optional(&Shape::Str)]);

/// `TrimMaterial.STREAM_CODEC`: a holder over the asset group and description.
static TRIM_MATERIAL: Shape = Shape::Holder(&Shape::Seq(&[
    // MaterialAssetGroup: a base suffix plus per-equipment overrides.
    Shape::Seq(&[Shape::Str, Shape::Map(&Shape::Str, &Shape::Str)]),
    Shape::Nbt,
]));

/// `FireworkExplosion.STREAM_CODEC`.
static FIREWORK_EXPLOSION: Shape = Shape::Seq(&[
    Shape::VarInt,
    Shape::List(&Shape::Int),
    Shape::List(&Shape::Int),
    Shape::Byte,
    Shape::Byte,
]);

/// `ByteBufCodecs.GAME_PROFILE_PROPERTIES`: name, value, and an optional
/// signature per property.
static PROFILE_PROPERTIES: Shape = Shape::List(&Shape::Seq(&[
    Shape::Str,
    Shape::Str,
    Shape::Optional(&Shape::Str),
]));

/// `KineticWeapon.Condition.STREAM_CODEC`.
static KINETIC_CONDITION: Shape = Shape::Seq(&[Shape::VarInt, Shape::Int, Shape::Int]);

/// `AdventureModePredicate.STREAM_CODEC`, shared by `can_place_on` and
/// `can_break`.
///
/// The deepest shape in the protocol: it reaches back into the component table
/// through [`Shape::TypedComponent`], so an item that restricts where it may be
/// placed carries components inside its own components.
static ADVENTURE_PREDICATE: Shape = Shape::List(&Shape::Seq(&[
    Shape::Optional(&Shape::HolderSet),
    // StatePropertiesPredicate: a property name, then either an exact value or
    // a min/max range.
    Shape::Optional(&Shape::List(&Shape::Seq(&[
        Shape::Str,
        Shape::Either(
            &Shape::Str,
            &Shape::Seq(&[Shape::Optional(&Shape::Str), Shape::Optional(&Shape::Str)]),
        ),
    ]))),
    // NbtPredicate.
    Shape::Optional(&Shape::Nbt),
    // DataComponentMatchers: exact component values, then predicates over them.
    Shape::Seq(&[
        Shape::List(&Shape::TypedComponent),
        // Every `DataComponentPredicate.Type` builds its stream codec with
        // `fromCodecWithRegistries`, so whichever predicate the key names, the
        // value after it is a plain NBT tag. That is what keeps this bounded:
        // otherwise it would need the whole predicate registry too.
        Shape::List(&Shape::Seq(&[
            Shape::Either(&Shape::VarInt, &Shape::VarInt),
            Shape::Nbt,
        ])),
    ]),
]));

// ---------------------------------------------------------------------------
// Shapes used by exactly one component type.
// ---------------------------------------------------------------------------

/// `UseEffects.STREAM_CODEC`: can sprint, interact vibrations, speed multiplier.
static USE_EFFECTS: Shape = Shape::Seq(&[Shape::Byte, Shape::Byte, Shape::Int]);

/// `ItemAttributeModifiers.STREAM_CODEC`.
static ATTRIBUTE_MODIFIERS: Shape = Shape::List(&Shape::Seq(&[
    Shape::VarInt,
    // AttributeModifier: id, amount, operation.
    Shape::Seq(&[Shape::Str, Shape::Long, Shape::VarInt]),
    Shape::VarInt,
    Shape::Dispatch {
        // Display: `default` and `hidden` carry nothing, `override` carries a
        // replacement line of text.
        variants: &[Shape::Unit, Shape::Unit, Shape::Nbt],
        // `ByIdMap.continuous(..., OutOfBoundsStrategy.ZERO)`, so an unknown
        // display type reads as `default` and consumes no further bytes rather
        // than failing the packet.
        out_of_range: OutOfRange::Clamp,
    },
]));

/// `CustomModelData.STREAM_CODEC`: floats, flags, strings, colors.
static CUSTOM_MODEL_DATA: Shape = Shape::Seq(&[
    Shape::List(&Shape::Int),
    Shape::List(&Shape::Byte),
    Shape::List(&Shape::Str),
    Shape::List(&Shape::Int),
]);

/// `TooltipDisplay.STREAM_CODEC`: hide everything, then the component types to
/// hide individually.
static TOOLTIP_DISPLAY: Shape = Shape::Seq(&[Shape::Byte, Shape::List(&Shape::VarInt)]);

/// `FoodProperties.DIRECT_STREAM_CODEC`: nutrition, saturation, can always eat.
static FOOD: Shape = Shape::Seq(&[Shape::VarInt, Shape::Int, Shape::Byte]);

/// `Consumable.STREAM_CODEC`.
static CONSUMABLE: Shape = Shape::Seq(&[
    Shape::Int,
    Shape::VarInt,
    SOUND_EVENT,
    Shape::Byte,
    Shape::List(&CONSUME_EFFECT),
]);

/// `UseCooldown.STREAM_CODEC`: seconds, then an optional shared cooldown group.
static USE_COOLDOWN: Shape = Shape::Seq(&[Shape::Int, Shape::Optional(&Shape::Str)]);

/// `Tool.STREAM_CODEC`.
static TOOL: Shape = Shape::Seq(&[
    // Rule: the blocks it applies to, an optional speed, and an optional
    // override of whether the drops count as correctly mined.
    Shape::List(&Shape::Seq(&[
        Shape::HolderSet,
        Shape::Optional(&Shape::Int),
        Shape::Optional(&Shape::Byte),
    ])),
    Shape::Int,
    Shape::VarInt,
    Shape::Byte,
]);

/// `Weapon.STREAM_CODEC`: durability cost per attack, and how long a hit
/// disables blocking for.
static WEAPON: Shape = Shape::Seq(&[Shape::VarInt, Shape::Int]);

/// `AttackRange.STREAM_CODEC`: min and max reach, the same again for creative,
/// a hitbox margin, and a mob factor.
static ATTACK_RANGE: Shape = Shape::Seq(&[
    Shape::Int,
    Shape::Int,
    Shape::Int,
    Shape::Int,
    Shape::Int,
    Shape::Int,
]);

/// `Equippable.STREAM_CODEC`: slot, equip sound, asset, camera overlay, allowed
/// entities, five flags, and the shearing sound.
static EQUIPPABLE: Shape = Shape::Seq(&[
    Shape::VarInt,
    SOUND_EVENT,
    Shape::Optional(&Shape::Str),
    Shape::Optional(&Shape::Str),
    Shape::Optional(&Shape::HolderSet),
    Shape::Byte,
    Shape::Byte,
    Shape::Byte,
    Shape::Byte,
    Shape::Byte,
    SOUND_EVENT,
]);

/// `BlocksAttacks.STREAM_CODEC`.
static BLOCKS_ATTACKS: Shape = Shape::Seq(&[
    Shape::Int,
    Shape::Int,
    // DamageReduction: blocking angle, damage types, base, factor.
    Shape::List(&Shape::Seq(&[
        Shape::Int,
        Shape::Optional(&Shape::HolderSet),
        Shape::Int,
        Shape::Int,
    ])),
    // ItemDamageFunction: threshold, base, factor.
    Shape::Seq(&[Shape::Int, Shape::Int, Shape::Int]),
    Shape::Optional(&Shape::HolderSet),
    Shape::Optional(&SOUND_EVENT),
    Shape::Optional(&SOUND_EVENT),
]);

/// `PiercingWeapon.STREAM_CODEC`.
static PIERCING_WEAPON: Shape = Shape::Seq(&[
    Shape::Byte,
    Shape::Byte,
    Shape::Optional(&SOUND_EVENT),
    Shape::Optional(&SOUND_EVENT),
]);

/// `KineticWeapon.STREAM_CODEC`.
static KINETIC_WEAPON: Shape = Shape::Seq(&[
    Shape::VarInt,
    Shape::VarInt,
    Shape::Optional(&KINETIC_CONDITION),
    Shape::Optional(&KINETIC_CONDITION),
    Shape::Optional(&KINETIC_CONDITION),
    Shape::Int,
    Shape::Int,
    Shape::Optional(&SOUND_EVENT),
    Shape::Optional(&SOUND_EVENT),
]);

/// `SwingAnimation.STREAM_CODEC`: animation type and duration.
static SWING_ANIMATION: Shape = Shape::Seq(&[Shape::VarInt, Shape::VarInt]);

/// `PotionContents.STREAM_CODEC`.
static POTION_CONTENTS: Shape = Shape::Seq(&[
    Shape::Optional(&Shape::VarInt),
    Shape::Optional(&Shape::Int),
    Shape::List(&MOB_EFFECT_INSTANCE),
    Shape::Optional(&Shape::Str),
]);

/// `SuspiciousStewEffects.STREAM_CODEC`: effect and duration per entry.
static SUSPICIOUS_STEW_EFFECTS: Shape = Shape::List(&Shape::Seq(&[Shape::VarInt, Shape::VarInt]));

/// `WrittenBookContent.STREAM_CODEC`.
static WRITTEN_BOOK_CONTENT: Shape = Shape::Seq(&[
    FILTERABLE_STR,
    Shape::Str,
    Shape::VarInt,
    // Pages are filterable text components rather than filterable strings.
    Shape::List(&Shape::Seq(&[Shape::Nbt, Shape::Optional(&Shape::Nbt)])),
    Shape::Byte,
]);

/// `ArmorTrim.STREAM_CODEC`: the material, then the pattern.
static TRIM: Shape = Shape::Seq(&[
    TRIM_MATERIAL,
    // TrimPattern: asset id, description, decal flag.
    Shape::Holder(&Shape::Seq(&[Shape::Str, Shape::Nbt, Shape::Byte])),
]);

/// `TypedEntityData.streamCodec(...)`: the type id, then the rest of the tag.
static TYPED_ENTITY_DATA: Shape = Shape::Seq(&[Shape::VarInt, Shape::Nbt]);

/// `Instrument.STREAM_CODEC`: sound, use duration, range, description.
static INSTRUMENT: Shape = Shape::Holder(&Shape::Seq(&[
    SOUND_EVENT,
    Shape::Int,
    Shape::Int,
    Shape::Nbt,
]));

/// `JukeboxSong.STREAM_CODEC`: sound, description, length, comparator output.
static JUKEBOX_PLAYABLE: Shape = Shape::Holder(&Shape::Seq(&[
    SOUND_EVENT,
    Shape::Nbt,
    Shape::Int,
    Shape::VarInt,
]));

/// `LodestoneTracker.STREAM_CODEC`: an optional `GlobalPos`, then whether the
/// compass keeps tracking it.
static LODESTONE_TRACKER: Shape = Shape::Seq(&[
    // GlobalPos: the dimension key, then a packed block position.
    Shape::Optional(&Shape::Seq(&[Shape::Str, Shape::Long])),
    Shape::Byte,
]);

/// `Fireworks.STREAM_CODEC`: flight duration, then the explosions.
static FIREWORKS: Shape = Shape::Seq(&[Shape::VarInt, Shape::List(&FIREWORK_EXPLOSION)]);

/// `ResolvableProfile.STREAM_CODEC`.
static PROFILE: Shape = Shape::Seq(&[
    Shape::Either(
        // A resolved GameProfile: id, name, properties.
        &Shape::Seq(&[Shape::Uuid, Shape::Str, PROFILE_PROPERTIES]),
        // A partial one, where either half may still be unresolved.
        &Shape::Seq(&[
            Shape::Optional(&Shape::Str),
            Shape::Optional(&Shape::Uuid),
            PROFILE_PROPERTIES,
        ]),
    ),
    // PlayerSkin.Patch: body, cape and elytra textures, then the model type.
    Shape::Seq(&[
        Shape::Optional(&Shape::Str),
        Shape::Optional(&Shape::Str),
        Shape::Optional(&Shape::Str),
        Shape::Optional(&Shape::Byte),
    ]),
]);

/// `BannerPatternLayers.STREAM_CODEC`: a pattern holder and a colour per layer.
static BANNER_PATTERNS: Shape = Shape::List(&Shape::Seq(&[
    // BannerPattern: asset id and translation key when sent inline.
    Shape::Holder(&Shape::Seq(&[Shape::Str, Shape::Str])),
    Shape::VarInt,
]));

/// `Bees.STREAM_CODEC`: each occupant's entity data and its hive timers.
static BEES: Shape = Shape::List(&Shape::Seq(&[
    TYPED_ENTITY_DATA,
    Shape::VarInt,
    Shape::VarInt,
]));

/// `ItemContainerContents.STREAM_CODEC`: up to 256 slots, each possibly empty.
static CONTAINER: Shape = Shape::List(&Shape::Optional(&STACK_TEMPLATE));

/// `PaintingVariant.DIRECT_STREAM_CODEC`: width, height, asset, title, author.
static PAINTING_VARIANT: Shape = Shape::Holder(&Shape::Seq(&[
    Shape::VarInt,
    Shape::VarInt,
    Shape::Str,
    Shape::Optional(&Shape::Nbt),
    Shape::Optional(&Shape::Nbt),
]));

/// A data component type, identified by its `minecraft:data_component_type`
/// registry id.
///
/// The discriminants are those ids, so a value round-trips through
/// [`ComponentType::id`] and [`ComponentType::from_id`] without a lookup table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum ComponentType {
    /// `minecraft:custom_data`
    CustomData = 0,
    /// `minecraft:max_stack_size`
    MaxStackSize = 1,
    /// `minecraft:max_damage`
    MaxDamage = 2,
    /// `minecraft:damage`
    Damage = 3,
    /// `minecraft:unbreakable`
    Unbreakable = 4,
    /// `minecraft:use_effects`
    UseEffects = 5,
    /// `minecraft:custom_name`
    CustomName = 6,
    /// `minecraft:minimum_attack_charge`
    MinimumAttackCharge = 7,
    /// `minecraft:damage_type`
    DamageType = 8,
    /// `minecraft:item_name`
    ItemName = 9,
    /// `minecraft:item_model`
    ItemModel = 10,
    /// `minecraft:lore`
    Lore = 11,
    /// `minecraft:rarity`
    Rarity = 12,
    /// `minecraft:enchantments`
    Enchantments = 13,
    /// `minecraft:can_place_on`
    CanPlaceOn = 14,
    /// `minecraft:can_break`
    CanBreak = 15,
    /// `minecraft:attribute_modifiers`
    AttributeModifiers = 16,
    /// `minecraft:custom_model_data`
    CustomModelData = 17,
    /// `minecraft:tooltip_display`
    TooltipDisplay = 18,
    /// `minecraft:repair_cost`
    RepairCost = 19,
    /// `minecraft:creative_slot_lock`
    CreativeSlotLock = 20,
    /// `minecraft:enchantment_glint_override`
    EnchantmentGlintOverride = 21,
    /// `minecraft:intangible_projectile`
    IntangibleProjectile = 22,
    /// `minecraft:food`
    Food = 23,
    /// `minecraft:consumable`
    Consumable = 24,
    /// `minecraft:use_remainder`
    UseRemainder = 25,
    /// `minecraft:use_cooldown`
    UseCooldown = 26,
    /// `minecraft:damage_resistant`
    DamageResistant = 27,
    /// `minecraft:tool`
    Tool = 28,
    /// `minecraft:weapon`
    Weapon = 29,
    /// `minecraft:attack_range`
    AttackRange = 30,
    /// `minecraft:enchantable`
    Enchantable = 31,
    /// `minecraft:equippable`
    Equippable = 32,
    /// `minecraft:repairable`
    Repairable = 33,
    /// `minecraft:glider`
    Glider = 34,
    /// `minecraft:tooltip_style`
    TooltipStyle = 35,
    /// `minecraft:death_protection`
    DeathProtection = 36,
    /// `minecraft:blocks_attacks`
    BlocksAttacks = 37,
    /// `minecraft:piercing_weapon`
    PiercingWeapon = 38,
    /// `minecraft:kinetic_weapon`
    KineticWeapon = 39,
    /// `minecraft:swing_animation`
    SwingAnimation = 40,
    /// `minecraft:additional_trade_cost`
    AdditionalTradeCost = 41,
    /// `minecraft:stored_enchantments`
    StoredEnchantments = 42,
    /// `minecraft:dye`
    Dye = 43,
    /// `minecraft:dyed_color`
    DyedColor = 44,
    /// `minecraft:map_color`
    MapColor = 45,
    /// `minecraft:map_id`
    MapId = 46,
    /// `minecraft:map_decorations`
    MapDecorations = 47,
    /// `minecraft:map_post_processing`
    MapPostProcessing = 48,
    /// `minecraft:charged_projectiles`
    ChargedProjectiles = 49,
    /// `minecraft:bundle_contents`
    BundleContents = 50,
    /// `minecraft:potion_contents`
    PotionContents = 51,
    /// `minecraft:potion_duration_scale`
    PotionDurationScale = 52,
    /// `minecraft:suspicious_stew_effects`
    SuspiciousStewEffects = 53,
    /// `minecraft:writable_book_content`
    WritableBookContent = 54,
    /// `minecraft:written_book_content`
    WrittenBookContent = 55,
    /// `minecraft:trim`
    Trim = 56,
    /// `minecraft:debug_stick_state`
    DebugStickState = 57,
    /// `minecraft:entity_data`
    EntityData = 58,
    /// `minecraft:bucket_entity_data`
    BucketEntityData = 59,
    /// `minecraft:block_entity_data`
    BlockEntityData = 60,
    /// `minecraft:instrument`
    Instrument = 61,
    /// `minecraft:provides_trim_material`
    ProvidesTrimMaterial = 62,
    /// `minecraft:ominous_bottle_amplifier`
    OminousBottleAmplifier = 63,
    /// `minecraft:jukebox_playable`
    JukeboxPlayable = 64,
    /// `minecraft:provides_banner_patterns`
    ProvidesBannerPatterns = 65,
    /// `minecraft:recipes`
    Recipes = 66,
    /// `minecraft:lodestone_tracker`
    LodestoneTracker = 67,
    /// `minecraft:firework_explosion`
    FireworkExplosion = 68,
    /// `minecraft:fireworks`
    Fireworks = 69,
    /// `minecraft:profile`
    Profile = 70,
    /// `minecraft:note_block_sound`
    NoteBlockSound = 71,
    /// `minecraft:banner_patterns`
    BannerPatterns = 72,
    /// `minecraft:base_color`
    BaseColor = 73,
    /// `minecraft:pot_decorations`
    PotDecorations = 74,
    /// `minecraft:container`
    Container = 75,
    /// `minecraft:block_state`
    BlockState = 76,
    /// `minecraft:bees`
    Bees = 77,
    /// `minecraft:sulfur_cube_content`
    SulfurCubeContent = 78,
    /// `minecraft:lock`
    Lock = 79,
    /// `minecraft:container_loot`
    ContainerLoot = 80,
    /// `minecraft:break_sound`
    BreakSound = 81,
    /// `minecraft:villager/variant`
    VillagerVariant = 82,
    /// `minecraft:wolf/variant`
    WolfVariant = 83,
    /// `minecraft:wolf/sound_variant`
    WolfSoundVariant = 84,
    /// `minecraft:wolf/collar`
    WolfCollar = 85,
    /// `minecraft:fox/variant`
    FoxVariant = 86,
    /// `minecraft:salmon/size`
    SalmonSize = 87,
    /// `minecraft:parrot/variant`
    ParrotVariant = 88,
    /// `minecraft:tropical_fish/pattern`
    TropicalFishPattern = 89,
    /// `minecraft:tropical_fish/base_color`
    TropicalFishBaseColor = 90,
    /// `minecraft:tropical_fish/pattern_color`
    TropicalFishPatternColor = 91,
    /// `minecraft:mooshroom/variant`
    MooshroomVariant = 92,
    /// `minecraft:rabbit/variant`
    RabbitVariant = 93,
    /// `minecraft:pig/variant`
    PigVariant = 94,
    /// `minecraft:pig/sound_variant`
    PigSoundVariant = 95,
    /// `minecraft:cow/variant`
    CowVariant = 96,
    /// `minecraft:cow/sound_variant`
    CowSoundVariant = 97,
    /// `minecraft:chicken/variant`
    ChickenVariant = 98,
    /// `minecraft:chicken/sound_variant`
    ChickenSoundVariant = 99,
    /// `minecraft:zombie_nautilus/variant`
    ZombieNautilusVariant = 100,
    /// `minecraft:frog/variant`
    FrogVariant = 101,
    /// `minecraft:horse/variant`
    HorseVariant = 102,
    /// `minecraft:painting/variant`
    PaintingVariant = 103,
    /// `minecraft:llama/variant`
    LlamaVariant = 104,
    /// `minecraft:axolotl/variant`
    AxolotlVariant = 105,
    /// `minecraft:cat/variant`
    CatVariant = 106,
    /// `minecraft:cat/sound_variant`
    CatSoundVariant = 107,
    /// `minecraft:cat/collar`
    CatCollar = 108,
    /// `minecraft:sheep/color`
    SheepColor = 109,
    /// `minecraft:shulker/color`
    ShulkerColor = 110,
}

/// Every type, in registry order.
static ALL: &[ComponentType] = &[
    ComponentType::CustomData,
    ComponentType::MaxStackSize,
    ComponentType::MaxDamage,
    ComponentType::Damage,
    ComponentType::Unbreakable,
    ComponentType::UseEffects,
    ComponentType::CustomName,
    ComponentType::MinimumAttackCharge,
    ComponentType::DamageType,
    ComponentType::ItemName,
    ComponentType::ItemModel,
    ComponentType::Lore,
    ComponentType::Rarity,
    ComponentType::Enchantments,
    ComponentType::CanPlaceOn,
    ComponentType::CanBreak,
    ComponentType::AttributeModifiers,
    ComponentType::CustomModelData,
    ComponentType::TooltipDisplay,
    ComponentType::RepairCost,
    ComponentType::CreativeSlotLock,
    ComponentType::EnchantmentGlintOverride,
    ComponentType::IntangibleProjectile,
    ComponentType::Food,
    ComponentType::Consumable,
    ComponentType::UseRemainder,
    ComponentType::UseCooldown,
    ComponentType::DamageResistant,
    ComponentType::Tool,
    ComponentType::Weapon,
    ComponentType::AttackRange,
    ComponentType::Enchantable,
    ComponentType::Equippable,
    ComponentType::Repairable,
    ComponentType::Glider,
    ComponentType::TooltipStyle,
    ComponentType::DeathProtection,
    ComponentType::BlocksAttacks,
    ComponentType::PiercingWeapon,
    ComponentType::KineticWeapon,
    ComponentType::SwingAnimation,
    ComponentType::AdditionalTradeCost,
    ComponentType::StoredEnchantments,
    ComponentType::Dye,
    ComponentType::DyedColor,
    ComponentType::MapColor,
    ComponentType::MapId,
    ComponentType::MapDecorations,
    ComponentType::MapPostProcessing,
    ComponentType::ChargedProjectiles,
    ComponentType::BundleContents,
    ComponentType::PotionContents,
    ComponentType::PotionDurationScale,
    ComponentType::SuspiciousStewEffects,
    ComponentType::WritableBookContent,
    ComponentType::WrittenBookContent,
    ComponentType::Trim,
    ComponentType::DebugStickState,
    ComponentType::EntityData,
    ComponentType::BucketEntityData,
    ComponentType::BlockEntityData,
    ComponentType::Instrument,
    ComponentType::ProvidesTrimMaterial,
    ComponentType::OminousBottleAmplifier,
    ComponentType::JukeboxPlayable,
    ComponentType::ProvidesBannerPatterns,
    ComponentType::Recipes,
    ComponentType::LodestoneTracker,
    ComponentType::FireworkExplosion,
    ComponentType::Fireworks,
    ComponentType::Profile,
    ComponentType::NoteBlockSound,
    ComponentType::BannerPatterns,
    ComponentType::BaseColor,
    ComponentType::PotDecorations,
    ComponentType::Container,
    ComponentType::BlockState,
    ComponentType::Bees,
    ComponentType::SulfurCubeContent,
    ComponentType::Lock,
    ComponentType::ContainerLoot,
    ComponentType::BreakSound,
    ComponentType::VillagerVariant,
    ComponentType::WolfVariant,
    ComponentType::WolfSoundVariant,
    ComponentType::WolfCollar,
    ComponentType::FoxVariant,
    ComponentType::SalmonSize,
    ComponentType::ParrotVariant,
    ComponentType::TropicalFishPattern,
    ComponentType::TropicalFishBaseColor,
    ComponentType::TropicalFishPatternColor,
    ComponentType::MooshroomVariant,
    ComponentType::RabbitVariant,
    ComponentType::PigVariant,
    ComponentType::PigSoundVariant,
    ComponentType::CowVariant,
    ComponentType::CowSoundVariant,
    ComponentType::ChickenVariant,
    ComponentType::ChickenSoundVariant,
    ComponentType::ZombieNautilusVariant,
    ComponentType::FrogVariant,
    ComponentType::HorseVariant,
    ComponentType::PaintingVariant,
    ComponentType::LlamaVariant,
    ComponentType::AxolotlVariant,
    ComponentType::CatVariant,
    ComponentType::CatSoundVariant,
    ComponentType::CatCollar,
    ComponentType::SheepColor,
    ComponentType::ShulkerColor,
];

// The names come from the generated registry rather than being restated, so
// they can only be right. This fires the day a protocol bump changes the count,
// which is exactly when the shapes below need rereading against the new jar.
const _: () = assert!(registry::DATA_COMPONENT_TYPE.entries.len() == ALL.len());

impl ComponentType {
    /// Every component type this protocol version defines, in id order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        ALL
    }

    /// The id this type has on the wire.
    #[must_use]
    pub const fn id(self) -> i32 {
        self as i32
    }

    /// The type with this wire id, if the protocol defines one.
    #[must_use]
    pub fn from_id(id: i32) -> Option<Self> {
        usize::try_from(id).ok().and_then(|id| ALL.get(id)).copied()
    }

    /// The registry name, such as `minecraft:custom_name`.
    #[must_use]
    pub fn name(self) -> &'static str {
        // Ids are positional in the registry, and the const assertion above
        // pins the two tables to the same length.
        registry::DATA_COMPONENT_TYPE.entries[self as usize]
    }

    /// Read a component type id.
    ///
    /// # Errors
    /// Returns [`Error::InvalidEnum`] for an id this protocol version does not
    /// define. There is no way to recover from one: the value that follows an
    /// unknown type has no discoverable length, so the read cannot continue.
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        let id = reader.var_int()?;
        Self::from_id(id).ok_or(Error::InvalidEnum {
            name: "data component type",
            value: id,
        })
    }

    /// How this type's value is laid out on the wire.
    #[must_use]
    pub fn shape(self) -> Shape {
        match self {
            // `custom_data`: no network codec; the persistent codec writes a tag
            // `custom_name`: a text component, which is a tag on the wire
            // `intangible_projectile`: no network codec; `Unit.CODEC` becomes an empty compound
            // `map_decorations`: no network codec; the persistent codec writes a tag
            // `debug_stick_state`: no network codec; the persistent codec writes a tag
            // `bucket_entity_data`: `CustomData.STREAM_CODEC` is a bare compound tag
            // `recipes`: no network codec; the persistent codec writes a tag
            // `lock`: no network codec; the persistent codec writes a tag
            // `container_loot`: no network codec; the persistent codec writes a tag
            Self::CustomData
            | Self::CustomName
            | Self::ItemName
            | Self::IntangibleProjectile
            | Self::MapDecorations
            | Self::DebugStickState
            | Self::BucketEntityData
            | Self::Recipes
            | Self::Lock
            | Self::ContainerLoot => Shape::Nbt,
            // `additional_trade_cost`: network-only; it has no persistent codec at all
            Self::MaxStackSize
            | Self::MaxDamage
            | Self::Damage
            | Self::DamageType
            | Self::Rarity
            | Self::RepairCost
            | Self::Enchantable
            | Self::AdditionalTradeCost
            | Self::Dye
            | Self::MapId
            | Self::MapPostProcessing
            | Self::OminousBottleAmplifier
            | Self::BaseColor
            | Self::VillagerVariant
            | Self::WolfVariant
            | Self::WolfSoundVariant
            | Self::WolfCollar
            | Self::FoxVariant
            | Self::SalmonSize
            | Self::ParrotVariant
            | Self::TropicalFishPattern
            | Self::TropicalFishBaseColor
            | Self::TropicalFishPatternColor
            | Self::MooshroomVariant
            | Self::RabbitVariant
            | Self::PigVariant
            | Self::PigSoundVariant
            | Self::CowVariant
            | Self::CowSoundVariant
            | Self::ChickenVariant
            | Self::ChickenSoundVariant
            | Self::ZombieNautilusVariant
            | Self::FrogVariant
            | Self::HorseVariant
            | Self::LlamaVariant
            | Self::AxolotlVariant
            | Self::CatVariant
            | Self::CatSoundVariant
            | Self::CatCollar
            | Self::SheepColor
            | Self::ShulkerColor => Shape::VarInt,
            Self::Unbreakable | Self::CreativeSlotLock | Self::Glider => Shape::Unit,
            Self::UseEffects => USE_EFFECTS,

            Self::MinimumAttackCharge
            | Self::DyedColor
            | Self::MapColor
            | Self::PotionDurationScale => Shape::Int,
            Self::ItemModel | Self::TooltipStyle | Self::NoteBlockSound => Shape::Str,
            Self::Lore => Shape::List(&Shape::Nbt),
            // `enchantments`: enchantment id to level
            Self::Enchantments | Self::StoredEnchantments => {
                Shape::Map(&Shape::VarInt, &Shape::VarInt)
            }
            Self::CanPlaceOn | Self::CanBreak => ADVENTURE_PREDICATE,
            Self::AttributeModifiers => ATTRIBUTE_MODIFIERS,
            Self::CustomModelData => CUSTOM_MODEL_DATA,
            Self::TooltipDisplay => TOOLTIP_DISPLAY,
            Self::EnchantmentGlintOverride => Shape::Byte,
            Self::Food => FOOD,
            Self::Consumable => CONSUMABLE,
            Self::UseRemainder | Self::SulfurCubeContent => STACK_TEMPLATE,
            Self::UseCooldown => USE_COOLDOWN,
            Self::DamageResistant | Self::Repairable | Self::ProvidesBannerPatterns => {
                Shape::HolderSet
            }
            Self::Tool => TOOL,
            Self::Weapon => WEAPON,
            Self::AttackRange => ATTACK_RANGE,
            Self::Equippable => EQUIPPABLE,
            Self::DeathProtection => Shape::List(&CONSUME_EFFECT),
            Self::BlocksAttacks => BLOCKS_ATTACKS,
            Self::PiercingWeapon => PIERCING_WEAPON,
            Self::KineticWeapon => KINETIC_WEAPON,
            Self::SwingAnimation => SWING_ANIMATION,
            Self::ChargedProjectiles | Self::BundleContents => Shape::List(&STACK_TEMPLATE),
            Self::PotionContents => POTION_CONTENTS,
            Self::SuspiciousStewEffects => SUSPICIOUS_STEW_EFFECTS,
            Self::WritableBookContent => Shape::List(&FILTERABLE_STR),
            Self::WrittenBookContent => WRITTEN_BOOK_CONTENT,
            Self::Trim => TRIM,
            Self::EntityData | Self::BlockEntityData => TYPED_ENTITY_DATA,
            Self::Instrument => INSTRUMENT,
            Self::ProvidesTrimMaterial => TRIM_MATERIAL,
            Self::JukeboxPlayable => JUKEBOX_PLAYABLE,
            Self::LodestoneTracker => LODESTONE_TRACKER,
            Self::FireworkExplosion => FIREWORK_EXPLOSION,
            Self::Fireworks => FIREWORKS,
            Self::Profile => PROFILE,
            Self::BannerPatterns => BANNER_PATTERNS,
            Self::PotDecorations => Shape::List(&Shape::VarInt),
            Self::Container => CONTAINER,
            Self::BlockState => Shape::Map(&Shape::Str, &Shape::Str),
            Self::Bees => BEES,
            Self::BreakSound => SOUND_EVENT,
            Self::PaintingVariant => PAINTING_VARIANT,
        }
    }
}
