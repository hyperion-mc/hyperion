//! Text a player typed, made safe to put inside a component.
//!
//! A `SystemChat` payload built from a literal string is rendered by the
//! client's `StringDecomposer`, which applies legacy section-sign codes as it
//! reads. That is deliberate and load bearing --
//! [`crate::net::agnostic::chat`] carries `§c` for exactly this reason -- and
//! it is why player text cannot be dropped into a component unchanged: to the
//! client there is no difference between a colour the server chose and one a
//! player typed.

/// U+00A7, the character the client's legacy formatter looks for.
///
/// Spelled as an escape rather than as itself so that this stays greppable and
/// so that `nix/text.nix` -- which fails the build on a section sign anywhere
/// in smash's text path or the proto crate -- keeps meaning what it says. That
/// gate is about *emitting* one; this is the one place whose whole job is
/// removing them, and it lives here rather than in an event so no event has to
/// spell the character at all.
const SECTION_SIGN: char = '\u{a7}';

/// `message` with every formatting escape removed.
///
/// Dropped rather than escaped: the legacy scheme has no escape for its own
/// introducer, and no message a person means to send contains one. What this
/// prevents is a client painting its own text -- `§k` scrambles the glyphs,
/// `§0`..`§f` recolours them, and `§4[Server] restarting` is a line that looks
/// like it came from the server. A vanilla client will not send one, which is
/// precisely why leaving it in only ever helps a bot.
#[must_use]
pub fn strip_formatting(message: &str) -> String {
    message.replace(SECTION_SIGN, "")
}

#[cfg(test)]
mod tests {
    use super::strip_formatting;

    #[test]
    fn ordinary_text_is_untouched() {
        assert_eq!(strip_formatting("gg <3 100% !"), "gg <3 100% !");
    }

    #[test]
    fn every_sign_goes_and_nothing_else_does() {
        assert_eq!(
            strip_formatting("\u{a7}4[Server] restarting \u{a7}kNOW"),
            "4[Server] restarting kNOW"
        );
    }

    #[test]
    fn a_trailing_sign_with_no_code_after_it_goes_too() {
        // The client's formatter needs a character after the sign, so a bare
        // trailing one renders as nothing. Removing it anyway keeps this a
        // statement about the character rather than about pairs.
        assert_eq!(strip_formatting("bye\u{a7}"), "bye");
    }
}
