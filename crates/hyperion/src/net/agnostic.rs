//! Agnostic networking primitives. Translates to correct protocol version.

mod chat;
pub use chat::{Chat, chat};

mod sound;
pub use sound::{RANGE_PER_VOLUME, Sound, SoundBuilder, SoundCategory, sound};
