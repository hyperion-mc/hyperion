use flecs_ecs::{core::ComponentOrPairId, macros::Component};
use hyperion_minecraft_proto::generated::registry::EntityType;

/// Declare [`EntityKind`] and the list of every one of them together.
///
/// A macro so that [`EntityKind::ALL`] cannot fall behind the enum. The list is
/// what makes the name-to-kind direction possible at all, and a hand-written
/// copy of 125 variants is a copy that is wrong the first time somebody adds a
/// mob and does not think to scroll down.
macro_rules! entity_kinds {
    ($($name:ident),* $(,)?) => {
        /// What kind of thing an entity is.
        ///
        /// The discriminants are flecs enum tags and carry no protocol meaning.
        /// They used to be sent as `minecraft:entity_type` ids, which worked
        /// only because 1.20.1 happened to number that registry the way this
        /// list is ordered; [`EntityKind::entity_type`] is now the only thing
        /// allowed to produce an id.
        #[derive(Component, Copy, Clone, Debug, PartialEq, Eq, Hash)]
        #[repr(C)]
        #[flecs(meta)]
        pub enum EntityKind {
            $($name),*
        }

        impl EntityKind {
            /// Every kind, in declaration order.
            pub const ALL: &'static [Self] = &[$(Self::$name),*];
        }
    };
}

entity_kinds! {
    Allay, AreaEffectCloud, ArmorStand, Arrow, Axolotl, Bat,
    Bee, Blaze, BlockDisplay, Boat, Camel, Cat,
    CaveSpider, ChestBoat, ChestMinecart, Chicken, Cod, CommandBlockMinecart,
    Cow, Creeper, Dolphin, Donkey, DragonFireball, Drowned,
    Egg, ElderGuardian, EndCrystal, EnderDragon, EnderPearl, Enderman,
    Endermite, Evoker, EvokerFangs, ExperienceBottle, ExperienceOrb, EyeOfEnder,
    FallingBlock, FireworkRocket, Fox, Frog, FurnaceMinecart, Ghast,
    Giant, GlowItemFrame, GlowSquid, Goat, Guardian, Hoglin,
    HopperMinecart, Horse, Husk, Illusioner, Interaction, IronGolem,
    Item, ItemDisplay, ItemFrame, Fireball, LeashKnot, Lightning,
    Llama, LlamaSpit, MagmaCube, Marker, Minecart, Mooshroom,
    Mule, Ocelot, Painting, Panda, Parrot, Phantom,
    Pig, Piglin, PiglinBrute, Pillager, PolarBear, Potion,
    Pufferfish, Rabbit, Ravager, Salmon, Sheep, Shulker,
    ShulkerBullet, Silverfish, Skeleton, SkeletonHorse, Slime, SmallFireball,
    Sniffer, SnowGolem, Snowball, SpawnerMinecart, SpectralArrow, Spider,
    Squid, Stray, Strider, Tadpole, TextDisplay, Tnt,
    TntMinecart, TraderLlama, Trident, TropicalFish, Turtle, Vex,
    Villager, Vindicator, WanderingTrader, Warden, Witch, Wither,
    WitherSkeleton, WitherSkull, Wolf, Zoglin, Zombie, ZombieHorse,
    ZombieVillager, ZombifiedPiglin, Player, FishingBobber, Gui,
}

impl EntityKind {
    /// The Minecraft entity type this kind spawns as.
    ///
    /// `None` for the four kinds this protocol version has no type for, named
    /// in the last arm. A caller that has to send an id should say so with
    /// [`Self::expect_entity_type`] rather than substituting another type,
    /// which would put the wrong thing in the world instead of nothing.
    ///
    /// Every arm names a constant from the generated table, so a version bump
    /// that renames or drops a type stops this file compiling rather than
    /// leaving a stale id to be discovered on the wire.
    /// The kind whose entity type is named `name`, such as `minecraft:creeper`.
    ///
    /// The reverse of [`Self::entity_type`], done by searching [`Self::ALL`]
    /// rather than by a second table. A table would be a second place the
    /// pairing is written, and the day the two disagree the wrong mob appears
    /// in the world with nothing to say why.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        let wanted = EntityType::from_name(name)?;
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.entity_type() == Some(wanted))
    }

    #[must_use]
    pub const fn entity_type(self) -> Option<EntityType> {
        let entity_type = match self {
            Self::Allay => EntityType::Allay,
            Self::AreaEffectCloud => EntityType::AreaEffectCloud,
            Self::ArmorStand => EntityType::ArmorStand,
            Self::Arrow => EntityType::Arrow,
            Self::Axolotl => EntityType::Axolotl,
            Self::Bat => EntityType::Bat,
            Self::Bee => EntityType::Bee,
            Self::Blaze => EntityType::Blaze,
            Self::BlockDisplay => EntityType::BlockDisplay,
            Self::Camel => EntityType::Camel,
            Self::Cat => EntityType::Cat,
            Self::CaveSpider => EntityType::CaveSpider,
            Self::ChestMinecart => EntityType::ChestMinecart,
            Self::Chicken => EntityType::Chicken,
            Self::Cod => EntityType::Cod,
            Self::CommandBlockMinecart => EntityType::CommandBlockMinecart,
            Self::Cow => EntityType::Cow,
            Self::Creeper => EntityType::Creeper,
            Self::Dolphin => EntityType::Dolphin,
            Self::Donkey => EntityType::Donkey,
            Self::DragonFireball => EntityType::DragonFireball,
            Self::Drowned => EntityType::Drowned,
            Self::Egg => EntityType::Egg,
            Self::ElderGuardian => EntityType::ElderGuardian,
            Self::EndCrystal => EntityType::EndCrystal,
            Self::EnderDragon => EntityType::EnderDragon,
            Self::EnderPearl => EntityType::EnderPearl,
            Self::Enderman => EntityType::Enderman,
            Self::Endermite => EntityType::Endermite,
            Self::Evoker => EntityType::Evoker,
            Self::EvokerFangs => EntityType::EvokerFangs,
            Self::ExperienceBottle => EntityType::ExperienceBottle,
            Self::ExperienceOrb => EntityType::ExperienceOrb,
            Self::EyeOfEnder => EntityType::EyeOfEnder,
            Self::FallingBlock => EntityType::FallingBlock,
            Self::FireworkRocket => EntityType::FireworkRocket,
            Self::Fox => EntityType::Fox,
            Self::Frog => EntityType::Frog,
            Self::FurnaceMinecart => EntityType::FurnaceMinecart,
            Self::Ghast => EntityType::Ghast,
            Self::Giant => EntityType::Giant,
            Self::GlowItemFrame => EntityType::GlowItemFrame,
            Self::GlowSquid => EntityType::GlowSquid,
            Self::Goat => EntityType::Goat,
            Self::Guardian => EntityType::Guardian,
            Self::Hoglin => EntityType::Hoglin,
            Self::HopperMinecart => EntityType::HopperMinecart,
            Self::Horse => EntityType::Horse,
            Self::Husk => EntityType::Husk,
            Self::Illusioner => EntityType::Illusioner,
            Self::Interaction => EntityType::Interaction,
            Self::IronGolem => EntityType::IronGolem,
            Self::Item => EntityType::Item,
            Self::ItemDisplay => EntityType::ItemDisplay,
            Self::ItemFrame => EntityType::ItemFrame,
            Self::Fireball => EntityType::Fireball,
            Self::LeashKnot => EntityType::LeashKnot,
            Self::Lightning => EntityType::LightningBolt,
            Self::Llama => EntityType::Llama,
            Self::LlamaSpit => EntityType::LlamaSpit,
            Self::MagmaCube => EntityType::MagmaCube,
            Self::Marker => EntityType::Marker,
            Self::Minecart => EntityType::Minecart,
            Self::Mooshroom => EntityType::Mooshroom,
            Self::Mule => EntityType::Mule,
            Self::Ocelot => EntityType::Ocelot,
            Self::Painting => EntityType::Painting,
            Self::Panda => EntityType::Panda,
            Self::Parrot => EntityType::Parrot,
            Self::Phantom => EntityType::Phantom,
            Self::Pig => EntityType::Pig,
            Self::Piglin => EntityType::Piglin,
            Self::PiglinBrute => EntityType::PiglinBrute,
            Self::Pillager => EntityType::Pillager,
            Self::PolarBear => EntityType::PolarBear,
            Self::Pufferfish => EntityType::Pufferfish,
            Self::Rabbit => EntityType::Rabbit,
            Self::Ravager => EntityType::Ravager,
            Self::Salmon => EntityType::Salmon,
            Self::Sheep => EntityType::Sheep,
            Self::Shulker => EntityType::Shulker,
            Self::ShulkerBullet => EntityType::ShulkerBullet,
            Self::Silverfish => EntityType::Silverfish,
            Self::Skeleton => EntityType::Skeleton,
            Self::SkeletonHorse => EntityType::SkeletonHorse,
            Self::Slime => EntityType::Slime,
            Self::SmallFireball => EntityType::SmallFireball,
            Self::Sniffer => EntityType::Sniffer,
            Self::SnowGolem => EntityType::SnowGolem,
            Self::Snowball => EntityType::Snowball,
            Self::SpawnerMinecart => EntityType::SpawnerMinecart,
            Self::SpectralArrow => EntityType::SpectralArrow,
            Self::Spider => EntityType::Spider,
            Self::Squid => EntityType::Squid,
            Self::Stray => EntityType::Stray,
            Self::Strider => EntityType::Strider,
            Self::Tadpole => EntityType::Tadpole,
            Self::TextDisplay => EntityType::TextDisplay,
            Self::Tnt => EntityType::Tnt,
            Self::TntMinecart => EntityType::TntMinecart,
            Self::TraderLlama => EntityType::TraderLlama,
            Self::Trident => EntityType::Trident,
            Self::TropicalFish => EntityType::TropicalFish,
            Self::Turtle => EntityType::Turtle,
            Self::Vex => EntityType::Vex,
            Self::Villager => EntityType::Villager,
            Self::Vindicator => EntityType::Vindicator,
            Self::WanderingTrader => EntityType::WanderingTrader,
            Self::Warden => EntityType::Warden,
            Self::Witch => EntityType::Witch,
            Self::Wither => EntityType::Wither,
            Self::WitherSkeleton => EntityType::WitherSkeleton,
            Self::WitherSkull => EntityType::WitherSkull,
            Self::Wolf => EntityType::Wolf,
            Self::Zoglin => EntityType::Zoglin,
            Self::Zombie => EntityType::Zombie,
            Self::ZombieHorse => EntityType::ZombieHorse,
            Self::ZombieVillager => EntityType::ZombieVillager,
            Self::ZombifiedPiglin => EntityType::ZombifiedPiglin,
            Self::Player => EntityType::Player,
            Self::FishingBobber => EntityType::FishingBobber,
            // Nothing in 26.2 to point at: `minecraft:boat` and
            // `minecraft:chest_boat` became one type per wood in 1.21.2,
            // `minecraft:potion` split into splash and lingering, and `Gui` is
            // this server's holder for an open inventory rather than anything a
            // client is ever told about.
            Self::Boat | Self::ChestBoat | Self::Potion | Self::Gui => return None,
        };
        Some(entity_type)
    }

    /// [`Self::entity_type`], for a caller with no way to carry on without one.
    ///
    /// # Panics
    /// Panics naming the kind. Reaching this means something enqueued a spawn
    /// for a kind no client can be told about, which is a bug in the caller
    /// rather than a condition to recover from.
    #[must_use]
    pub fn expect_entity_type(self) -> EntityType {
        self.entity_type().unwrap_or_else(|| {
            panic!("{self:?} has no minecraft:entity_type in this version, so it cannot be spawned")
        })
    }
}
