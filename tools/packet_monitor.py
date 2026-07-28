#!/usr/bin/env python3
"""A reusable packet monitor for skin and visual end-to-end checks.

Every gate that asks what a player looks like reads the same two clientbound
packets, and until this module each kept its own copy of the decoder. The
copies drifted: `parse_player_info_update` lived in both `identity-check.py`
and `smash-selector.py`, and the `SetEntityData` reader understood only a
player's custom name, never the skin-overlay mask that decides whether hats
render. One decoder here, imported by each gate, is edited in one place when
the wire changes.

What it decodes and accumulates:

  - `PlayerInfoUpdate` (0x46) -> the tab-list profile a client renders another
    player from, including the `textures` property and its Mojang signature.
    A skin only every other player can see is an unsigned property, so the
    signature is kept, not just the value.
  - `SetEntityData` (0x63) -> entity metadata, including the byte at index 16
    (`Avatar.DATA_PLAYER_MODE_CUSTOMISATION` on 26.2), whose bits are the
    second skin layer: cape, jacket, sleeves, trouser legs and the hat. A zero
    there renders a player with no overlays at all.
  - `AddEntity` (0x01) -> the uuid the entity carries, which is the only thing
    that ties a tab-list profile id to the entity whose metadata holds its
    overlay mask. Without it a test knows a profile has a skin but not which
    body wears it.

The accumulator merges field by field across packets, because a
`PlayerInfoUpdate` that carries only a gamemode says nothing about a profile's
textures and must not erase them.
"""

from __future__ import annotations

import importlib.util
import pathlib
import struct

TOOLS = pathlib.Path(__file__).resolve().parent


def _load(name, filename):
    spec = importlib.util.spec_from_file_location(name, TOOLS / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


_base = _load("client_26_2", "client-26.2.py")
_match = _load("smash_match", "smash-match.py")

take_var_int = _base.take_var_int
take_string = _base.take_string
take_nbt_string = _match.take_nbt_string

# Clientbound play ids in protocol 776, from
# crates/hyperion-minecraft-proto/src/generated/packet_id.rs.
S2C_ADD_ENTITY = 0x01
S2C_PLAYER_INFO_REMOVE = 0x45
S2C_PLAYER_INFO_UPDATE = 0x46
S2C_RESPAWN = 0x52
S2C_SET_ENTITY_DATA = 0x63

# `PlayerInfoActions`, the same bit order as
# `ClientboundPlayerInfoUpdatePacket.Action`.
ADD_PLAYER = 1 << 0
INITIALIZE_CHAT = 1 << 1
UPDATE_GAME_MODE = 1 << 2
UPDATE_LISTED = 1 << 3
UPDATE_LATENCY = 1 << 4
UPDATE_DISPLAY_NAME = 1 << 5
UPDATE_LIST_ORDER = 1 << 6
UPDATE_HAT = 1 << 7

# `EntityDataSerializers` ids. The client rejects a value whose serializer
# disagrees with the field, so reading a field back is a check on both halves.
SER_BYTE = 0
SER_VARINT = 1
SER_FLOAT = 3
SER_OPTIONAL_COMPONENT = 6
SER_BOOLEAN = 8

# `ClientboundSetEntityDataPacket.EOF_MARKER`.
METADATA_EOF = 0xFF

# `net.minecraft.world.entity.player.Player` field indices on 26.2, from
# crates/hyperion/src/simulation/metadata/player.rs. The overlay mask moved to
# 16 when `Avatar` was inserted between `LivingEntity` and `Player`; the old
# number (17) is now absorption hearts.
DATA_PLAYER_MODE_CUSTOMISATION = 16

# The bits of that mask, from the same source.
SKIN_PART_CAPE = 0x01
SKIN_PART_JACKET = 0x02
SKIN_PART_LEFT_SLEEVE = 0x04
SKIN_PART_RIGHT_SLEEVE = 0x08
SKIN_PART_LEFT_PANTS = 0x10
SKIN_PART_RIGHT_PANTS = 0x20
SKIN_PART_HAT = 0x40
# Every overlay a vanilla client turns on by default, which is what a spawned
# player should carry so other players see the whole skin and not a bald base.
ALL_SKIN_PARTS = 0x7F


def _take_optional_string(payload, offset):
    present = payload[offset]
    offset += 1
    if not present:
        return None, offset
    return take_string(payload, offset)


def parse_player_info_update(payload):
    """Every entry in one `PlayerInfoUpdate`, as far as the actions describe.

    Nothing in the body says how long an entry is, so a reader that stops
    understanding a field loses the rest of the packet. Raising rather than
    returning what was read so far is deliberate: a half-read packet is a
    disagreement about the wire format, not a shorter list of players.
    """
    actions = payload[0]
    count, offset = take_var_int(payload, 1)
    entries = []
    for _ in range(count):
        entry = {"uuid": payload[offset : offset + 16].hex(), "properties": {}}
        offset += 16
        if actions & ADD_PLAYER:
            entry["name"], offset = take_string(payload, offset)
            properties, offset = take_var_int(payload, offset)
            for _ in range(properties):
                name, offset = take_string(payload, offset)
                value, offset = take_string(payload, offset)
                signature, offset = _take_optional_string(payload, offset)
                entry["properties"][name] = (value, signature)
        if actions & INITIALIZE_CHAT:
            raise ValueError("this server does not sign chat, so it cannot send a session")
        if actions & UPDATE_GAME_MODE:
            entry["game_mode"], offset = take_var_int(payload, offset)
        if actions & UPDATE_LISTED:
            entry["listed"] = bool(payload[offset])
            offset += 1
        if actions & UPDATE_LATENCY:
            _, offset = take_var_int(payload, offset)
        if actions & UPDATE_DISPLAY_NAME:
            present = payload[offset]
            offset += 1
            if present:
                raise ValueError("PlayerInfoUpdate carried a display name")
        if actions & UPDATE_LIST_ORDER:
            _, offset = take_var_int(payload, offset)
        if actions & UPDATE_HAT:
            offset += 1
        entries.append(entry)
    if offset != len(payload):
        raise ValueError(
            "PlayerInfoUpdate has %d trailing byte(s); the action set and this "
            "reader disagree" % (len(payload) - offset)
        )
    return actions, entries


def decode_entity_metadata(payload):
    """The entity id and every field of one `SetEntityData` this reader knows.

    The run is self-delimiting only when every length is known, so an entry
    whose serializer is not handled below ends the read rather than being
    guessed past. The handled set covers what a player's metadata carries up to
    and including the overlay mask: the scalar serializers (byte, var int,
    float, boolean) and the optional chat component a custom name rides as.
    """
    entity_id, offset = take_var_int(payload)
    fields = {}
    while offset < len(payload):
        index = payload[offset]
        offset += 1
        if index == METADATA_EOF:
            break
        serializer, offset = take_var_int(payload, offset)
        if serializer == SER_BYTE:
            fields[index] = payload[offset]
            offset += 1
        elif serializer == SER_VARINT:
            fields[index], offset = take_var_int(payload, offset)
        elif serializer == SER_FLOAT:
            (fields[index],) = struct.unpack_from(">f", payload, offset)
            offset += 4
        elif serializer == SER_BOOLEAN:
            fields[index] = bool(payload[offset])
            offset += 1
        elif serializer == SER_OPTIONAL_COMPONENT:
            present = payload[offset]
            offset += 1
            if present:
                text, offset = take_nbt_string(payload, offset)
                fields[index] = text
            else:
                fields[index] = None
        else:
            fields["_stopped_at_serializer"] = serializer
            break
    return entity_id, fields


class Monitor:
    """Accumulated, queryable state from a client's received packets.

    A client feeds every packet it reads to `feed`; the monitor keeps only the
    ones a skin or visual assertion needs. Nothing here drives the client or
    touches the socket, so one monitor can sit behind any of the scripted
    clients in this directory.
    """

    def __init__(self):
        # profile id (hex uuid) -> tab-list entry, properties merged.
        self.roster = {}
        # entity id -> decoded metadata fields, merged.
        self.metadata = {}
        # profile id (hex uuid) -> entity id, from `AddEntity`.
        self.entity_of_profile = {}
        # How many times this client was told to throw its world away.
        self.respawns = 0

    def feed(self, packet_id, payload):
        if packet_id == S2C_ADD_ENTITY:
            entity_id, offset = take_var_int(payload)
            uuid = payload[offset : offset + 16].hex()
            self.entity_of_profile[uuid] = entity_id
        elif packet_id == S2C_SET_ENTITY_DATA:
            entity_id, fields = decode_entity_metadata(payload)
            self.metadata.setdefault(entity_id, {}).update(fields)
        elif packet_id == S2C_RESPAWN:
            self.respawns += 1
        elif packet_id == S2C_PLAYER_INFO_UPDATE:
            _actions, entries = parse_player_info_update(payload)
            for entry in entries:
                known = self.roster.setdefault(entry["uuid"], {"properties": {}})
                for key, value in entry.items():
                    if key == "properties":
                        known["properties"].update(value)
                    else:
                        known[key] = value

    def profile(self, profile_id):
        return self.roster.get(profile_id, {})

    def texture_of(self, profile_id):
        """The `(value, signature)` of a profile's `textures` property, or None."""
        return self.profile(profile_id).get("properties", {}).get("textures")

    def skin_parts_of(self, profile_id):
        """The overlay mask on a profile's entity, or None if none was sent."""
        entity_id = self.entity_of_profile.get(profile_id)
        if entity_id is None:
            return None
        return self.metadata.get(entity_id, {}).get(DATA_PLAYER_MODE_CUSTOMISATION)

    def view_of(self, profile_id):
        """One structured snapshot of how this client sees `profile_id`."""
        value_sig = self.texture_of(profile_id)
        value, signature = value_sig if value_sig else (None, None)
        parts = self.skin_parts_of(profile_id)
        return {
            "profile_id": profile_id,
            "entity_id": self.entity_of_profile.get(profile_id),
            "textures_value": value,
            "textures_signature": signature,
            "has_signature": signature is not None,
            "skin_parts": parts,
            "hat_shown": parts is not None and bool(parts & SKIN_PART_HAT),
            "all_parts_shown": parts is not None and (parts & ALL_SKIN_PARTS) == ALL_SKIN_PARTS,
        }
