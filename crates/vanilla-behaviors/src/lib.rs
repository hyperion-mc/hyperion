use flecs_ecs::core::EntityView;
use hyperion::{
    flecs_ecs::{self, core::EntityViewGet},
    glam::DVec3,
    net::{Compose, ConnectionId},
    simulation::{metadata::living_entity::Health, LastDamaged, Player},
};
use hyperion_utils::EntityExt;
use tracing::warn;
use valence_protocol::{packets::play, VarInt};
use valence_server::GameMode;

pub mod command;
pub mod module;

// /!\ Minecraft version dependent
pub enum DamageType {
    InFire,
    LightningBolt,
    OnFire,
    Lava,
    HotFloor,
    InWall,
    Cramming,
    Drown,
    Starve,
    Cactus,
    Fall,
    FlyIntoWall,
    FellOutOfWorld,
    Generic,
    Magic,
    Wither,
    DragonBreath,
    DryOut,
    SweetBerryBush,
    Freeze,
    Stalagmite,
    FallingBlock,
    FallingAnvil,
    FallingStalactite,
    Sting,
    MobAttack,
    MobAttackNoAggro,
    PlayerAttack,
    Arrow,
    Trident,
    MobProjectile,
    Fireworks,
    UnattributedFireball,
    Fireball,
    WitherSkull,
    Thrown,
    IndirectMagic,
    Thorns,
    Explosion,
    PlayerExplosion,
    SonicBoom,
    BadRespawnPoint,
    OutsideBorder,
    GenericKill,
}

pub struct DamageCause {
    pub damage_type: DamageType,
    pub position: Option<DVec3>,
    pub source_entity: i32,
    pub direct_source: i32,
}

impl DamageCause {
    #[must_use]
    pub const fn new(damage_type: DamageType) -> Self {
        Self {
            damage_type,
            position: Option::None,
            source_entity: 0,
            direct_source: 0,
        }
    }

    pub const fn with_position(&mut self, position: DVec3) -> &mut Self {
        self.position = Option::Some(position);
        self
    }

    pub const fn with_entities(&mut self, source: i32, direct_source: i32) -> &mut Self {
        self.source_entity = source;
        self.direct_source = direct_source;
        self
    }
}

#[must_use]
pub const fn is_invincible(gamemode: &GameMode) -> bool {
    matches!(gamemode, GameMode::Creative | GameMode::Spectator)
}

pub fn damage_player(
    entity: &EntityView<'_>,
    amount: f32,
    damage_cause: DamageCause,
    compose: &Compose,
    system: EntityView<'_>,
) -> bool {
    if entity.has::<Player>() {
        entity.get::<(&mut Health, &mut LastDamaged, &ConnectionId)>(
            |(health, last_damaged, connection)| {
                if !health.is_dead() && compose.global().tick - last_damaged.tick >= 20 {
                    health.damage(amount);
                    last_damaged.tick = compose.global().tick;

                    let pkt_damage_event = play::EntityDamageS2c {
                        entity_id: VarInt(entity.minecraft_id()),
                        source_cause_id: VarInt(damage_cause.source_entity),
                        source_direct_id: VarInt(damage_cause.direct_source),
                        source_type_id: VarInt(damage_cause.damage_type as i32),
                        source_pos: damage_cause.position,
                    };

                    compose
                        .unicast(&pkt_damage_event, *connection, system)
                        .unwrap(); // Should brodcast locally?
                    return true;
                }
                false
            },
        )
    } else {
        warn!("Trying to call a Player only function on an non player entity");
        false
    }
}
