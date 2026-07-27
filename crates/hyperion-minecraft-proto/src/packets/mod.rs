//! Packet bodies, one module per connection state.
//!
//! Layouts the extractor recovered in full are generated from `protocol.json`
//! by `build.rs` and spliced in with `include!`; the rest are hand-written in
//! the same modules, against the same decompiled sources. Which is which is
//! visible in the source: a generated file names the codec it came from in
//! every doc comment.
//!
//! Ids restart at zero in every state and direction, so a struct here is only
//! meaningful alongside the matching [`crate::generated::packet_id`] entry.
//!
//! # Two definitions of eleven packets, for now
//!
//! [`configuration`] and [`play_login`] were hand-written against the same
//! decompiled sources, in parallel with this generator, and landed first.
//! Eleven of their packets are now generated as well, so the crate defines
//! each of those twice: once flat in the hand-written module and once under
//! the state's `clientbound` or `serverbound` module.
//!
//! The generated one is the one to keep, and the reconciliation is the next
//! change rather than this one -- it means rewriting 725 lines of tests that
//! name the hand-written types, which is a review of its own. What is *not*
//! duplicated is the part the generator cannot do: `ClientInformation`,
//! `CustomPayload` and `UpdateTags` all branch on a runtime value.

pub mod common;
pub mod configuration;
pub mod handshake;
pub mod login;
pub mod play;
pub mod play_login;
pub mod status;
