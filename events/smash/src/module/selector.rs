//! The kit selector: a ring of podiums in the middle of the lobby, one per
//! mob, and you right-click the one you want.
//!
//! ## What Mineplex did
//!
//! The SSM waiting lobby put one mob per kit on a pedestal and you clicked the
//! mob you wanted to be. The Mineplex Wiki describes the waiting lobby's
//! centrepiece as "mobs ... standing atop pedestals of coloured wool and stone
//! brick slabs, allowing you to click/right-click on them to select kits", and
//! its kit page notes that in SSM's case the pedestals carried "each individual
//! mob" rather than the stand-in zombies other games used. It also records what
//! the wool was for: the colour said what you had to do to unlock the kit, with
//! free kits yellow, gem kits green and achievement kits purple.
//!
//! This rebuilds both halves of that:
//!
//! * A podium is a wool block with the kit's own mob standing on it, and the
//!   mob is what you right-click. Not a head block, not an armour stand: the
//!   real entity, which is what makes the ring readable from across the lobby.
//! * The wool's colour is the status readout. Mineplex used it for "can you
//!   afford this"; this uses it for "has somebody taken this", which is the
//!   question a player in a filling lobby is actually asking.
//!
//! ## One player per mob
//!
//! Nothing found says whether Mineplex reserved a kit. Neither the wiki's kit
//! and waiting-lobby pages nor the SSM forum threads mention a reservation in
//! either direction, and since kits were bought per account with gems, a rule
//! that let one player lock another out of something they had paid for would
//! be a strange one to have shipped. So treat exclusivity as this server's
//! choice and not as a reconstruction: it exists because the operator asked
//! for a selector that answers when you click a mob somebody else has, and
//! "answers" is only meaningful if the answer is no.
//!
//! The claim lasts exactly as long as the player does, and it is enforced only
//! where a kit can still be changed. [`crate::module::lobby::choose`] already
//! refuses any change once the match commits, so past that point the claim is
//! frozen for the rest of the match without a second rule saying so, and a
//! player who disconnects frees their mob immediately because the edge that
//! was the claim goes with the entity. No disconnect handler exists here, and
//! that is the point: there is nothing to clean up.
//!
//! ## Everything here is relations
//!
//! A podium *is* its `(Offers, kit)` edge; there is no table from block
//! position to kit name and no id anybody has to keep in step. A mob being
//! taken *is* somebody's `(Playing, kit)` edge; see
//! [`crate::module::kit::claims`] for why that is derived on demand and never
//! written down.
//!
//! `/kit` and `/kits` still work and are still what the tests and a screen
//! reader use, but nobody should have to type a command to play.

use flecs_ecs::prelude::*;
use glam::IVec3;

use crate::{
    flecs_ext::EntityViewExt,
    module::{
        kit::{self, KitMob, KitName},
        lobby, sound,
    },
    server::{Channel, NamedColor, PlayerId, ServerHandle, Sound, SoundCategory, Text},
};

/// Tag on a selector podium.
#[derive(Component, Debug)]
pub struct Podium;

/// Relationship: `(Offers, kit)` on a podium.
///
/// The podium's entire identity. A click resolves to a kit by following this
/// edge, so moving a podium, renaming a kit or reordering the roster cannot
/// desynchronise anything: there is no second place the pairing is written.
#[derive(Component, Debug)]
pub struct Offers;

/// Where a podium stands, in world block coordinates.
///
/// One block: `base`, the coloured wool. The kit's own [`KitMob`] stands on top
/// of it, at [`Plinth::stand`], and is a real entity rather than a block.
///
/// Both are clickable, and that is deliberate. Clicking the mob is the point
/// and is what a player does. Clicking the wool is the same selection through
/// a surface that cannot fail to render, and it is what the unit tests use,
/// because a mob needs a host to exist and the game half is testable without
/// one.
///
/// Making the mob clickable at all took a change to the engine: hyperion routed
/// no entity-interaction packet, so a right-click on an entity arrived and fell
/// through the dispatch table. `PacketId::Interact` is routed now, and since
/// 26.2 split attacking out into `Attack` that packet means a right-click and
/// nothing else.
#[derive(Component, Debug, Copy, Clone, PartialEq, Eq)]
pub struct Plinth {
    pub base: IVec3,
}

impl Plinth {
    /// The block the kit's mob stands in.
    #[must_use]
    pub const fn stand(self) -> IVec3 {
        IVec3::new(self.base.x, self.base.y + 1, self.base.z)
    }

    /// Whether a click on `position` is a click on this podium.
    ///
    /// The wool and the block the mob stands in, so a click that misses the mob
    /// by a pixel and lands on the block behind it still means what the player
    /// meant.
    #[must_use]
    pub fn covers(self, position: IVec3) -> bool {
        position == self.base || position == self.stand()
    }
}

/// The wool a free podium stands on.
///
/// Green and red rather than anything cleverer, because this is the whole
/// answer to "how is a player told a mob is taken". It has to survive being
/// read at a glance from across the lobby in the last four seconds of a
/// countdown, by somebody who is not reading chat and has no reason to expect
/// a message. Colour is the only channel that works under those conditions,
/// and it says *which* mob is gone rather than only that something was
/// refused. The action bar line below explains a refusal that has already
/// happened; the wool is what stops the refusal being a surprise.
///
/// Wool specifically because that is what Mineplex's pedestals were made of
/// and used for exactly this job, and because the hub is already furnished in
/// quartz and sea lanterns: a sea-lantern base would read as more lobby
/// scenery rather than as a thing to click.
pub const FREE_BLOCK: &str = "minecraft:lime_wool";

/// The wool a taken podium stands on. See [`FREE_BLOCK`].
pub const TAKEN_BLOCK: &str = "minecraft:red_wool";

/// The colour a free mob's nameplate is drawn in, and a taken one's.
///
/// The same pair as [`FREE_BLOCK`] and [`TAKEN_BLOCK`], on purpose: the wool
/// and the nameplate are one signal drawn at two heights rather than two facts
/// that can drift, and both are computed from [`kit::claims`] by the two
/// functions below. What the second height buys is reach. Wool is at a
/// player's feet in the middle of a lobby people are standing around in, and a
/// nameplate renders over whatever is in front of it from anywhere in the hub.
pub const FREE_COLOR: NamedColor = NamedColor::Green;
pub const TAKEN_COLOR: NamedColor = NamedColor::Red;

/// The y of a podium's wool, in the hub's local coordinates.
///
/// The hub floor's top face is y 64, so a player stands at 65 and this is
/// level with their feet. The icon at 66 is then at chest height, which is
/// where a crosshair naturally falls.
pub const PLINTH_Y: i32 = 65;

/// Blocks between one podium and the next, along the ring.
///
/// The radius is derived from this and the roster size rather than fixed, so
/// the ring grows when a kit is added instead of eventually putting two
/// podiums in the same block. A constant gap is also what a player reads as
/// a row of things rather than a crowd.
pub const GAP: f32 = 3.5;

/// Outside the raised centre and outside the ring of hub spawn points, so a
/// small roster still makes a ring somebody can walk around and nobody spawns
/// inside a podium.
pub const MIN_RADIUS: f32 = 8.0;

/// Inside the hub's glass wall, which stands at radius 19.
pub const MAX_RADIUS: f32 = 17.0;

/// Where the podiums stand, relative to the hub's own origin.
///
/// Pure, and in registration order, which makes the ring stable across boots:
/// a player who learns where their mob is finds it in the same place tomorrow.
#[must_use]
pub fn ring(count: usize) -> Vec<IVec3> {
    if count == 0 {
        return Vec::new();
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "a roster of 2^24 kits is not a thing"
    )]
    let n = count as f32;
    let radius = (GAP * n / core::f32::consts::TAU).clamp(MIN_RADIUS, MAX_RADIUS);

    (0..count)
        .map(|index| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "index is smaller than the count above"
            )]
            let angle = core::f32::consts::TAU * index as f32 / n;
            #[expect(
                clippy::cast_possible_truncation,
                reason = "radius is clamped to 17 blocks"
            )]
            let at = IVec3::new(
                (radius * angle.cos()).round() as i32,
                PLINTH_Y,
                (radius * angle.sin()).round() as i32,
            );
            at
        })
        .collect()
}

/// Stand up one podium per registered kit, centred on `origin`.
///
/// Any podiums already standing are torn down first, so calling this twice
/// rebuilds the ring rather than producing two of it.
///
/// # Panics
/// If two podiums would occupy the same block. That is a roster too big for
/// [`MAX_RADIUS`] at [`GAP`] spacing, and it is a failure worth having at boot
/// rather than a kit nobody can click.
pub fn build(world: &World, origin: IVec3) {
    let mut standing = Vec::new();
    world
        .query::<()>()
        .with(Podium::id())
        .build()
        .each_entity(|podium, ()| standing.push(podium.id()));
    for podium in standing {
        world.entity_from_id(podium).destruct();
    }

    let kits = kit::registry(world);
    let mut placed: Vec<IVec3> = Vec::with_capacity(kits.len());

    for (kit, at) in kits.iter().zip(ring(kits.len())) {
        let base = at + origin;
        assert!(
            !placed.contains(&base),
            "two kit podiums want the block at {base}; the roster has outgrown the ring"
        );
        placed.push(base);

        world
            .entity()
            .add(Podium::id())
            .set(Plinth { base })
            .add((Offers, *kit));
    }
}

/// Every podium, with the kit it offers and where it stands.
#[must_use]
pub fn podiums(world: &World) -> Vec<(Entity, Plinth, Entity)> {
    let mut found = Vec::new();
    world
        .query::<&Plinth>()
        .with(Podium::id())
        .build()
        .each_entity(|podium, plinth| {
            let Some(kit) = podium.find_target(Offers, |_| true) else {
                return;
            };
            found.push((podium.id(), *plinth, kit.id()));
        });
    found
}

/// The podium whose plinth or icon block is at `position`.
#[must_use]
pub fn podium_at(world: &World, position: IVec3) -> Option<(Entity, Entity)> {
    podiums(world)
        .into_iter()
        .find(|(_, plinth, _)| plinth.covers(position))
        .map(|(podium, _, kit)| (podium, kit))
}

/// What the plinth block under each podium should be, right now.
///
/// Recomputed from the live claims rather than remembered, and returned rather
/// than written, so the rule is testable without a Minecraft world anywhere
/// near it. The host compares this against the blocks actually in the world and
/// writes the ones that differ.
#[must_use]
pub fn plinths(world: &World) -> Vec<(IVec3, &'static str)> {
    let claims = kit::claims(world);
    podiums(world)
        .into_iter()
        .map(|(_, plinth, kit)| {
            let taken = claims.iter().any(|claim| claim.kit == kit);
            (plinth.base, if taken { TAKEN_BLOCK } else { FREE_BLOCK })
        })
        .collect()
}

/// What one podium's mob is called, and in what colour. Pure.
///
/// The kit's name and nothing else. Not the holder's, though the holder is
/// known here and `taken_message` already formats one: fifteen mobs each
/// captioned with somebody's IGN is a paragraph strung across the middle of
/// the hub, and it answers a question a player only asks after they have been
/// refused. The colour carries the claim in no characters at all, which is
/// what keeps a ring of fifteen readable. Whose it is stays where it was, on
/// the action bar of the click that was refused.
#[must_use]
pub fn nameplate(kit: &str, taken: bool) -> Text {
    Text::text(kit.to_owned()).color(if taken { TAKEN_COLOR } else { FREE_COLOR })
}

/// What the mob on each podium should currently be called.
///
/// Derived from the live claims on every call and returned rather than
/// written, exactly as [`plinths`] is and for the same reason: the rule is
/// then testable with no Minecraft world anywhere near it, and the host is
/// what compares this against the names the mobs are wearing.
#[must_use]
pub fn nameplates(world: &World) -> Vec<(Entity, Text)> {
    let claims = kit::claims(world);
    podiums(world)
        .into_iter()
        .filter_map(|(podium, _, kit)| {
            let name = world
                .entity_from_id(kit)
                .try_get::<&KitName>(|name| name.0)?;
            let taken = claims.iter().any(|claim| claim.kit == kit);
            Some((podium, nameplate(name, taken)))
        })
        .collect()
}

/// Which mob stands on each podium, and where.
///
/// Names rather than anything typed, because the game half must not know what
/// a hyperion entity is: the host turns each name into a real mob. See
/// [`crate::terrain`].
#[must_use]
pub fn mobs(world: &World) -> Vec<(Entity, IVec3, &'static str)> {
    podiums(world)
        .into_iter()
        .map(|(podium, plinth, kit)| {
            let mob = world
                .entity_from_id(kit)
                .try_get::<&KitMob>(|mob| mob.0)
                .unwrap_or(kit::DEFAULT_MOB);
            (podium, plinth.stand(), mob)
        })
        .collect()
}

/// One podium, as something outside the world can read.
///
/// Built by walking the same edges everything else here walks, so a reader gets
/// the live answer and not a snapshot somebody remembered to refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// The kit's [`KitName`].
    pub name: &'static str,
    /// Where the mob stands. Clicking the mob is the point; clicking this
    /// block does the same thing, which is what a scripted client uses.
    pub click: IVec3,
    /// The wool under it, whose colour is the free-or-taken signal.
    pub base: IVec3,
    /// The block the wool is currently made of, so a reader can check the
    /// colour it was told against the colour the world is showing.
    pub wool: &'static str,
    /// The mob standing on it, by vanilla entity id.
    pub mob: &'static str,
    /// The name the mob is wearing over its head, as plain text.
    ///
    /// The rendered string and not the component, because what a gate can
    /// read off the wire is a name and a colour, and the colour is
    /// [`wool`](Self::wool)'s question already. A client checks this against
    /// the `custom_name` in the mob's entity metadata.
    pub label: String,
    /// The vanilla sound event picking this mob plays, read off the kit's own
    /// `(PlaysOnSelect, sound)` edge.
    ///
    /// Published so that a gate can hold the sound it hears against the
    /// server's own declaration rather than against a table of fifteen strings
    /// copied into Python, which would be a second registry and would go on
    /// passing after the first one changed.
    pub select_sound: Option<&'static str>,
    /// Whoever is playing this mob, by name. Derived from `(Playing, kit)`.
    pub held_by: Option<String>,
}

/// Every podium, for a reader outside the game.
///
/// This is what `/podiums` answers with and what the end to end gate drives
/// itself from. It exists so that nothing outside this file has to know the
/// ring's geometry: a gate that recomputed the podium positions in Python
/// would be a second source of truth for the one thing this module owns, and
/// it would keep passing after the ring moved.
#[must_use]
pub fn manifest(world: &World) -> Vec<Offer> {
    let claims = kit::claims(world);
    podiums(world)
        .into_iter()
        .filter_map(|(_, plinth, kit)| {
            let entry = world.entity_from_id(kit);
            let name = entry.try_get::<&KitName>(|name| name.0)?;
            let taken = claims.iter().any(|claim| claim.kit == kit);
            Some(Offer {
                name,
                click: plinth.stand(),
                base: plinth.base,
                wool: if taken { TAKEN_BLOCK } else { FREE_BLOCK },
                mob: entry
                    .try_get::<&KitMob>(|mob| mob.0)
                    .unwrap_or(kit::DEFAULT_MOB),
                label: nameplate(name, taken).plain(),
                select_sound: sound::declared(entry, sound::PlaysOnSelect)
                    .map(|declared| declared.id),
                held_by: claims
                    .iter()
                    .find(|claim| claim.kit == kit)
                    .map(|claim| world.entity_from_id(claim.player).name()),
            })
        })
        .collect()
}

/// Relationship: `(StandsOn, podium)` on a podium's mob.
///
/// The other half of the pair [`Offers`] starts. A click arrives naming an
/// entity and nothing else, and this is what turns that entity back into a
/// kit: mob to podium to kit, two edges, no table keyed on entity id and
/// nothing to invalidate when a mob is respawned somewhere else.
#[derive(Component, Debug)]
pub struct StandsOn;

/// A right-click on the mob `target`, from `player`.
///
/// Returns `false` when that entity is not a podium's mob, which is every
/// other entity in the world including every other player.
#[expect(
    clippy::must_use_candidate,
    reason = "called for the selection it makes; the bool only says whether the click was aimed \
              at a podium, and both callers in `input.rs` are answering every entity and every \
              block in the world"
)]
pub fn click_mob(world: &World, player: EntityView<'_>, target: Entity) -> bool {
    let Some(podium) = world
        .try_get_alive(target)
        .and_then(|mob| mob.find_target(StandsOn, |_| true))
    else {
        return false;
    };
    let Some(kit) = podium.find_target(Offers, |_| true) else {
        return false;
    };
    take(world, player, kit.id());
    true
}

/// A right-click on the block at `position`, from `player`.
///
/// Returns `false` when that block is not part of a podium, which is every
/// other block in the world.
#[expect(clippy::must_use_candidate, reason = "see `click_mob`")]
pub fn click(world: &World, player: EntityView<'_>, position: IVec3) -> bool {
    let Some((_, kit)) = podium_at(world, position) else {
        return false;
    };
    take(world, player, kit);
    true
}

/// What both surfaces do once they know which kit was asked for.
///
/// One function, so a click on the mob and a click on the wool under it cannot
/// come to different answers.
fn take(world: &World, player: EntityView<'_>, kit: Entity) {
    if let Err(reason) = lobby::choose(world, player, world.entity_from_id(kit)) {
        refuse(world, player, &reason);
    }
    // On success `choose` has already sent the confirmation and the hotbar.
}

/// Tell `player` why the click did nothing.
///
/// The action bar, not chat. A refusal is about the click that just happened
/// and stops mattering a second later, and chat during a countdown is where a
/// line goes to be missed. It is the second-line answer in any case: the wool
/// under the mob was already red before the click, so this explains a refusal
/// rather than delivering the news.
///
/// And a sound, which is the first-line answer: a player whose click did
/// nothing learns that from their ears before they have read anything. It is
/// [`sound::SELECTION_REFUSED`] and not the mob's own voice, because hearing
/// the Wolf when you failed to become the Wolf is the wrong answer said
/// confidently. To the clicker alone, for the same reason the selection sound
/// is.
fn refuse(world: &World, player: EntityView<'_>, reason: &str) {
    let Some(id) = player.try_get::<&PlayerId>(|id| *id) else {
        return;
    };
    world.get::<&ServerHandle>(|server| {
        server.play_sound_to(id, Sound::new(sound::SELECTION_REFUSED, SoundCategory::Ui));
        server.send_message(
            id,
            Channel::ActionBar,
            Text::text(reason.to_owned()).color(NamedColor::Red),
        );
    });
}

/// The refusal a player sees when somebody else got there first.
///
/// A function rather than a `format!` at the call site because two surfaces
/// produce it -- a podium click and `/kit` -- and a player who is told two
/// different things by two paths through the same rule will believe the rule
/// is two rules.
#[must_use]
pub fn taken_message(world: &World, kit: EntityView<'_>, holder: Entity) -> String {
    let name = kit.try_get::<&KitName>(|name| name.0).unwrap_or("That kit");
    format!(
        "{name} is already taken by {}.",
        world.entity_from_id(holder).name()
    )
}

#[derive(Component)]
pub struct SelectorModule;

impl Module for SelectorModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Selector");

        world.component::<Podium>();
        world.component::<Plinth>();
        // Exclusive: a podium offers exactly one mob, so re-pointing one is a
        // single `add` rather than a remove-then-add pair that can be
        // interrupted halfway and leave a podium offering two kits.
        world.component::<Offers>().add(flecs::Exclusive);
        // Exclusive for the same reason: a mob stands on one podium, and a mob
        // that claimed to stand on two would offer two kits from one click.
        //
        // Deleting the podium deletes the mob standing on it, which is what
        // makes `build` a rebuild rather than a leak: tearing the ring down
        // leaves no mobs behind for flecs to hand back to a later query, and
        // there is no teardown loop here that somebody has to remember to
        // extend when a podium grows a second thing attached to it.
        world
            .component::<StandsOn>()
            .add(flecs::Exclusive)
            .add_trait::<(flecs::OnDeleteTarget, flecs::Delete)>();
    }
}
