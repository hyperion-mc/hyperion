//! A component, written back out as the section-sign codes a page renders.
//!
//! The console reads replies off the wire as `SystemChat`, which carries a
//! component: a tree with colour and decoration as *fields*. The page renders
//! section signs. So something has to bridge the two, and
//! [`Component::plain`](hyperion::hyperion_minecraft_proto::text::Component::plain)
//! is not it -- it throws every field away, which turns a red refusal into a
//! sentence in the same colour as everything else and loses the one thing that
//! made it read as a refusal.
//!
//! Most replies survive `plain` by accident, because
//! [`hyperion::net::agnostic::chat`] builds its component from a string that
//! already has `§c` inside it. The ones that do not are exactly the ones a
//! game built properly, through a typed seam, which is the direction the
//! workspace is moving in.
//!
//! This is lossy in the other direction and deliberately so. Hover text, click
//! actions, fonts and true-colour RGB have no legacy spelling; RGB is mapped to
//! its nearest named colour rather than dropped, because a wrong-ish red reads
//! far closer to the intent than no colour at all.

use hyperion::hyperion_minecraft_proto::text::{Component, NamedColor, Rgb24, TextColor};

/// The character the client's legacy formatter looks for. Spelled as an escape
/// for the same reason `hyperion::simulation::chat` spells it that way.
const SECTION_SIGN: char = '\u{a7}';

/// `component`, flattened into text with legacy codes.
#[must_use]
pub fn render(component: &Component<'_>) -> String {
    let mut out = String::new();

    // `runs` is the component already flattened with inheritance applied, so
    // this never walks the tree itself. Walking it was the first version and
    // it got nesting wrong at depth three: a child inherits its parent's
    // style, a sibling must not, and re-deriving that by hand is exactly the
    // work `runs` has already done correctly.
    for run in component.runs() {
        // A colour code resets every decoration in the legacy scheme, so the
        // colour is written first and the decorations after it. The other
        // order silently drops the decorations, which renders as "nearly
        // right".
        //
        // `§r` first, because a run carries its whole resolved style and the
        // previous run's codes are still in force at this point.
        out.push(SECTION_SIGN);
        out.push('r');

        if let Some(colour) = run.style.color.map(code_for) {
            out.push(SECTION_SIGN);
            out.push(colour);
        }
        for (enabled, code) in [
            (run.style.bold, 'l'),
            (run.style.italic, 'o'),
            (run.style.underlined, 'n'),
            (run.style.strikethrough, 'm'),
            (run.style.obfuscated, 'k'),
        ] {
            if enabled == Some(true) {
                out.push(SECTION_SIGN);
                out.push(code);
            }
        }

        out.push_str(&run.text);
    }

    out
}

/// The legacy code for a colour.
///
/// Total, because every colour has an answer: a named one has its own code and
/// a true-colour one has its nearest swatch. There is deliberately no `None`
/// here -- an `Option` would invite a caller to drop the colour, and dropping
/// it renders as "this line was never coloured", which is a worse lie than a
/// slightly wrong red.
fn code_for(colour: TextColor) -> char {
    let named = match colour {
        TextColor::Named(named) => named,
        // No legacy code exists, so the nearest named one is the closest
        // truthful answer. The alternative is dropping the colour, which reads
        // as "this line was never coloured".
        TextColor::Rgb(rgb) => nearest_named(rgb),
    };

    match named {
        NamedColor::Black => '0',
        NamedColor::DarkBlue => '1',
        NamedColor::DarkGreen => '2',
        NamedColor::DarkAqua => '3',
        NamedColor::DarkRed => '4',
        NamedColor::DarkPurple => '5',
        NamedColor::Gold => '6',
        NamedColor::Gray => '7',
        NamedColor::DarkGray => '8',
        NamedColor::Blue => '9',
        NamedColor::Green => 'a',
        NamedColor::Aqua => 'b',
        NamedColor::Red => 'c',
        NamedColor::LightPurple => 'd',
        NamedColor::Yellow => 'e',
        NamedColor::White => 'f',
    }
}

/// The named colour closest to `rgb`, by squared distance in plain RGB.
///
/// Not a perceptual space. This is picking one of sixteen swatches for a
/// console pane, and the difference between a plain and a perceptual metric at
/// that resolution is not something a person reading chat can see.
fn nearest_named(rgb: Rgb24) -> NamedColor {
    const SWATCHES: [(NamedColor, [i32; 3]); 16] = [
        (NamedColor::Black, [0x00, 0x00, 0x00]),
        (NamedColor::DarkBlue, [0x00, 0x00, 0xaa]),
        (NamedColor::DarkGreen, [0x00, 0xaa, 0x00]),
        (NamedColor::DarkAqua, [0x00, 0xaa, 0xaa]),
        (NamedColor::DarkRed, [0xaa, 0x00, 0x00]),
        (NamedColor::DarkPurple, [0xaa, 0x00, 0xaa]),
        (NamedColor::Gold, [0xff, 0xaa, 0x00]),
        (NamedColor::Gray, [0xaa, 0xaa, 0xaa]),
        (NamedColor::DarkGray, [0x55, 0x55, 0x55]),
        (NamedColor::Blue, [0x55, 0x55, 0xff]),
        (NamedColor::Green, [0x55, 0xff, 0x55]),
        (NamedColor::Aqua, [0x55, 0xff, 0xff]),
        (NamedColor::Red, [0xff, 0x55, 0x55]),
        (NamedColor::LightPurple, [0xff, 0x55, 0xff]),
        (NamedColor::Yellow, [0xff, 0xff, 0x55]),
        (NamedColor::White, [0xff, 0xff, 0xff]),
    ];

    let channels = rgb.channels();
    let want = [
        i32::from(channels[0]),
        i32::from(channels[1]),
        i32::from(channels[2]),
    ];
    SWATCHES
        .into_iter()
        .min_by_key(|(_named, swatch)| {
            (0..3)
                .map(|axis| (swatch[axis] - want[axis]).pow(2))
                .sum::<i32>()
        })
        .map_or(NamedColor::White, |(named, _swatch)| named)
}

#[cfg(test)]
mod tests {
    use hyperion::hyperion_minecraft_proto::text::{
        Component, NamedColor, Rgb24, Style, TextColor,
    };

    use super::render;

    fn coloured(colour: TextColor) -> Style<'static> {
        Style {
            color: Some(colour),
            ..Style::new()
        }
    }

    #[test]
    fn plain_text_gets_one_reset_and_nothing_else() {
        assert_eq!(render(&Component::text("hello")), "\u{a7}rhello");
    }

    #[test]
    fn a_colour_becomes_its_legacy_code() {
        let component =
            Component::text("nope").with_style(coloured(TextColor::Named(NamedColor::Red)));
        assert_eq!(render(&component), "\u{a7}r\u{a7}cnope");
    }

    #[test]
    fn a_sibling_does_not_inherit_the_previous_run() {
        // The bug this exists to prevent: without the reset per run, "after"
        // is drawn red because "before" was.
        let component = Component::text("")
            .append(
                Component::text("before").with_style(coloured(TextColor::Named(NamedColor::Red))),
            )
            .append(Component::text("after"));
        // One reset, not two: the empty root carries no text, and `runs`
        // only emits a run for a non-empty `Contents::Text`, so the root
        // contributes nothing to render. Asserting two was this test's own
        // authoring error and it is the only thing that was ever red here.
        assert_eq!(render(&component), "\u{a7}r\u{a7}cbefore\u{a7}rafter");
    }

    #[test]
    fn true_colour_falls_back_to_the_nearest_swatch() {
        // Dropping it would render as "this line was never coloured", which is
        // a worse lie than a slightly wrong red.
        let component = Component::text("close enough")
            .with_style(coloured(TextColor::Rgb(Rgb24::new(0xf0, 0x50, 0x50))));
        assert_eq!(render(&component), "\u{a7}r\u{a7}cclose enough");
    }
}
