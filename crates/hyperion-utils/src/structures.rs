use valence_protocol::math::DVec3;

// /!\ Minecraft version dependent
pub enum DamageType {
    Arrow,
    BadRespawnPoint,
    Cactus,
    Cramming,
    DragonBreath,
    DryOut,
    Drown,
    Explosion,
    Fall,
    FallingAnvil,
    FallingBlock,
    FallingStalactite,
    Fireball,
    Fireworks,
    FlyIntoWall,
    Freeze,
    Generic,
    GenericKill,
    HotFloor,
    InFire,
    InWall,
    IndirectMagic,
    Lava,
    LightningBolt,
    Magic,
    MobAttack,
    MobAttackNoAggro,
    MobProjectile,
    OnFire,
    OutOfWorld,
    OutsideBorder,
    PlayerAttack,
    PlayerExplosion,
    SonicBoom,
    Stalagmite,
    Sting,
    Starve,
    SweetBerryBush,
    Thorns,
    Thrown,
    Trident,
    UnattributedFireball,
    Wither,
    WitherSkull,
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
            source_entity: -1,
            direct_source: -1,
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
