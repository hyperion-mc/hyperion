use valence_protocol::math::DVec3;

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
