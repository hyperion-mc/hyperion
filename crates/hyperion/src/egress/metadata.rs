//! Tracked-value runs this server sends outside the change-tracking path.

use hyperion_minecraft_proto::packets::play::entity::SetEntityData;

/// One tracked value: index 16, serializer `Byte`, every bit set.
///
/// Index 16 is `Avatar.DATA_PLAYER_MODE_CUSTOMISATION`, the skin overlay mask a
/// client reports in its own settings. It was 17 through 1.21; on 26.2 that
/// index is the player's absorption hearts, so the old number renders a player
/// with no hat and a row of phantom hearts.
///
/// The `0xFF` terminator is not here: [`SetEntityData`] writes it.
const ALL_SKIN_PARTS: &[u8] = &[16, 0, 0xFF];

/// Tracked data showing every part of a player's skin.
///
/// A player who never sent client settings, or whose settings arrived after
/// another player subscribed, would otherwise render with the mask at zero.
#[must_use]
pub const fn show_all(id: i32) -> SetEntityData<'static> {
    SetEntityData {
        id,
        packed_items: ALL_SKIN_PARTS,
    }
}
