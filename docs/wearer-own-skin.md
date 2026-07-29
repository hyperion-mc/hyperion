# A kit wearer sees their own login skin, not the kit's, on their own avatar

Tracks hyperion-mc/hyperion#1056. This is the definitive verdict, read from the
decompiled 26.2 client rather than inferred. Reproduce the evidence with:

    nix build .#minecraft-client-skin-sources
    # -> net/minecraft/client/player/AbstractClientPlayer.java
    #    net/minecraft/client/multiplayer/PlayerInfo.java
    #    net/minecraft/client/multiplayer/ClientPacketListener.java

## The wire is not the problem

Every other player sees the kit skin. The `smash-skin-e2e` gate proves it on the
wire: another player's tab-list profile for the wearer carries the committed
`textures` value, its signature verifies under Mojang's key (SIGNED), and the
entity's overlay mask is `0x7F`. The wearer's own tab-list entry carries the
custom skin too. So the server sends everything correctly; the discrepancy is
entirely in how the client renders the wearer's *own* avatar.

## The client caches the local player's skin and never invalidates it

Three decompiled facts, in the order they compose:

- **`AbstractClientPlayer.getSkin()` reads from a cached `PlayerInfo`.** It calls
  `getPlayerInfo()`, which is `if (this.playerInfo == null) this.playerInfo =
  connection.getPlayerInfo(getUUID()); return this.playerInfo;`. The field is
  populated once, lazily, and there is no setter anywhere that nulls it.

- **That `PlayerInfo` memoizes its skin from the profile it was built with.**
  `PlayerInfo.getSkin()` is `if (this.skinLookup == null) this.skinLookup =
  createSkinLookup(this.profile); return this.skinLookup.get();`. The lookup is
  built once from `this.profile` and cached.

- **No tab-list packet touches either cache for a player that already exists.**
  `ClientPacketListener.handlePlayerInfoRemove` only does
  `this.playerInfoMap.remove(profileId)`. `handlePlayerInfoUpdate` builds a new
  `PlayerInfo` and inserts it with `putIfAbsent`. Neither reaches into
  `this.minecraft.player` to reset its cached `playerInfo` field.

So hyperion's `roster::refresh` sequence for the wearer -- `PlayerInfoRemove`
then `PlayerInfoUpdate(ADD_PLAYER)` with the new textures -- updates the map the
tab list reads, but the wearer's `LocalPlayer` still holds the old `PlayerInfo`
reference and renders the old skin. For a real premium player that old skin is
their own account skin, which is exactly the reported "I see my own skin, not
the kit's."

Note the local player would accept even an *unsigned* skin: `createSkinLookup`
passes `requireSecure = !minecraft.isLocalPlayer(profile.id())`, which is `false`
for yourself. Signing is not the wearer's problem; the stale cache is.

## The only thing that gives the local player a fresh skin is a new LocalPlayer

`this.minecraft.player` is replaced only by `handleLogin` (join) and
`handleRespawn`. `handleRespawn` builds a `newPlayer` via
`gameMode.createPlayer(...)`, whose `playerInfo` is null and so re-reads the
updated map -- the skin refreshes. But it is not cheap even within one
dimension: `handleRespawn` unconditionally calls `setClientLoaded(false)` and
`startWaitingForNewLevel(...)`, and `startWaitingForNewLevel` does
`setScreenAndShow(new LevelLoadingScreen(...))`. The wearer gets the loading
screen, and hyperion's proxy does not re-offer the entity subscriptions the
client drops, which is the empty-hub failure `roster.rs` already documents.

## Verdict

There is no refresh path lighter than a Respawn. No packet resets
`AbstractClientPlayer.playerInfo` short of rebuilding `LocalPlayer`, and the only
packets that rebuild it are Login and Respawn. `roster.rs`'s decision to leave
the wearer's own view alone is therefore correct, not a shortcut.

The options, least disruptive first:

1. **Assign the skin before the wearer enters the world.** If the player's skin
   is set before the play-state `Login` packet, the first `LocalPlayer` is built
   with the right `PlayerInfo` and the wearer sees it immediately, no respawn.
   This fixes the *join-time* skin only. It does nothing for a mid-session kit
   change, which is the smash case (kits are picked in the hub after join).

2. **Respawn the wearer on a mid-session skin change.** Same dimension, keep
   metadata (`shouldKeep(2)`). Refreshes the skin at the cost of the loading
   screen and the dropped entity subscriptions above -- the trade `roster.rs`
   declined. Only worth it if the operator values the wearer seeing their own
   kit skin over a seamless kit switch.

3. **Leave it.** The wearer keeps their login skin on themselves; everyone else
   sees the kit skin. This is current behaviour.

Recommendation: keep option 3 as the default and, if the wearer's own kit skin
matters, take option 1 for a fixed join-time kit rather than option 2. A
mid-session own-avatar skin change is not achievable without a visible respawn,
and that is a property of the client, not of hyperion.

## Verifying the render itself needs a real client

A scripted client has no renderer, so it can assert what the server sent but not
what the wearer sees. The vehicle for the render check is the
`tools/hyperion-pilot-mod` Fabric mod (26.2), which exposes a control socket and
screenshot RPC. That path is semi-automated: the operator loads the mod into
their real client and an agent drives it and reads back screenshots. A headless
software-GL client in a `nixosTest` is not currently practical for 26.2, so the
pilot mod is the recommended vehicle if option 1 or 2 is ever implemented and
someone wants to confirm the pixels.
