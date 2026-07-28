//! Where the cursor is in a half-typed command.
//!
//! Split out from the registration in [`crate`] and generic over the node
//! handle, because everything here is arithmetic over a string and a command's
//! shape and that is the part with the off-by-ones in it. The tests below run
//! without a world, a server or a socket.

/// One step of a command's shape, as the walk reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step<T> {
    /// The word a token has to equal, for a subcommand. `None` for a value the
    /// player types.
    pub literal: Option<String>,
    /// The clap id, which is the name a declaration uses to reach this step.
    pub id: String,
    /// The command tree node carrying this step's completion source.
    pub node: T,
    /// A greedy value takes the rest of the line rather than one word.
    ///
    /// clap spells this as an argument accepting more than one value, which is
    /// how `/kit Iron Golem` is one kit name and not two.
    pub greedy: bool,
    /// The steps that may follow this one.
    pub next: Vec<Self>,
}

/// The one thing a suggestion response needs to know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor<T> {
    /// The tree node whose completions to offer.
    pub node: T,
    /// Byte offset into the whole command text where the replaced span starts.
    ///
    /// The client replaces from here to the end of what was typed, so a
    /// suggestion may contain a space: completing `/kit Iron Gol` replaces
    /// `Iron Gol` rather than just `Gol`.
    pub start: usize,
    /// What has already been typed for this value.
    pub prefix: String,
}

/// Byte offset and text of each whitespace-delimited token.
fn tokens(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (index, character) in text.char_indices() {
        if character.is_whitespace() {
            if let Some(from) = start.take() {
                out.push((from, &text[from..index]));
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(from) = start {
        out.push((from, &text[from..]));
    }
    out
}

/// The step a already-typed token takes.
///
/// A subcommand wins over a value, so `/perms set` follows the `set` literal
/// rather than reading `set` as somebody's name.
fn advance<'a, T>(here: &'a [Step<T>], token: &str) -> Option<&'a Step<T>> {
    here.iter()
        .find(|step| {
            step.literal
                .as_deref()
                .is_some_and(|literal| literal.eq_ignore_ascii_case(token))
        })
        .or_else(|| here.iter().find(|step| step.literal.is_none()))
}

/// Which node `text` is asking for completions of, and the span a suggestion
/// replaces.
///
/// `None` when nothing should be offered: the cursor is still inside the
/// command's own name, which the client completes from the graph without asking
/// anybody, or it has reached a point where only subcommand literals follow,
/// which the client also has.
#[must_use]
pub fn locate<T: Copy>(steps: &[Step<T>], text: &str) -> Option<Cursor<T>> {
    if !text.starts_with('/') {
        return None;
    }

    let words = tokens(text);
    if words.is_empty() {
        return None;
    }

    // An open token is one the cursor is still inside, which is any run of
    // non-whitespace the text ends on. A trailing space closes it and starts
    // the next value empty.
    let open = !text.ends_with(char::is_whitespace);

    if open && words.len() == 1 {
        // Still typing the command's name. Not ours to answer.
        return None;
    }

    // Tokens after the command name that are finished, so the walk consumes
    // them rather than completing them.
    let complete = if open {
        words.len() - 2
    } else {
        words.len() - 1
    };

    let (start, prefix) = if open {
        let (from, _) = words[words.len() - 1];
        (from, text[from..].to_owned())
    } else {
        (text.len(), String::new())
    };

    let mut here = steps;
    for position in 0..complete {
        let (from, token) = words[position + 1];
        let step = advance(here, token)?;

        if step.greedy {
            // The greedy value runs from its first word to the cursor, and the
            // suggestion replaces all of it. Without this `/kit Iron Gol`
            // completes to `/kit Iron Iron Golem`.
            return Some(Cursor {
                node: step.node,
                start: from,
                prefix: text[from..].to_owned(),
            });
        }

        here = &step.next;
    }

    let step = here.iter().find(|step| step.literal.is_none())?;
    Some(Cursor {
        node: step.node,
        start,
        prefix,
    })
}

#[cfg(test)]
mod tests {
    use super::{Cursor, Step, locate};

    fn value(id: &str, node: u32, greedy: bool, next: Vec<Step<u32>>) -> Step<u32> {
        Step {
            literal: None,
            id: id.to_owned(),
            node,
            greedy,
            next,
        }
    }

    fn literal(name: &str, node: u32, next: Vec<Step<u32>>) -> Step<u32> {
        Step {
            literal: Some(name.to_owned()),
            id: name.to_owned(),
            node,
            greedy: false,
            next,
        }
    }

    /// `/kit`, whose one argument takes every remaining word because half the
    /// kit names have a space in them.
    fn kit() -> Vec<Step<u32>> {
        vec![value("name", 1, true, vec![])]
    }

    /// `/perms set <player> <group>` and `/perms get <player>`.
    fn perms() -> Vec<Step<u32>> {
        vec![
            literal("set", 10, vec![value("player", 11, false, vec![value(
                "group",
                12,
                false,
                vec![],
            )])]),
            literal("get", 20, vec![value("player", 21, false, vec![])]),
        ]
    }

    #[test]
    fn typing_the_command_name_is_the_clients_own_business() {
        assert_eq!(locate(&kit(), "/ki"), None);
        assert_eq!(locate(&kit(), "/kit"), None);
    }

    #[test]
    fn a_trailing_space_opens_the_first_argument() {
        assert_eq!(
            locate(&kit(), "/kit "),
            Some(Cursor {
                node: 1,
                start: 5,
                prefix: String::new(),
            })
        );
    }

    #[test]
    fn a_partial_word_is_the_prefix_and_names_its_own_span() {
        assert_eq!(
            locate(&kit(), "/kit Iron"),
            Some(Cursor {
                node: 1,
                start: 5,
                prefix: "Iron".to_owned(),
            })
        );
    }

    #[test]
    fn a_greedy_argument_spans_every_word_it_has_taken() {
        // The span starts at `Iron`, not at `Gol`, so the client replaces the
        // whole name rather than appending to it.
        assert_eq!(
            locate(&kit(), "/kit Iron Gol"),
            Some(Cursor {
                node: 1,
                start: 5,
                prefix: "Iron Gol".to_owned(),
            })
        );
        assert_eq!(
            locate(&kit(), "/kit Iron "),
            Some(Cursor {
                node: 1,
                start: 5,
                prefix: "Iron ".to_owned(),
            })
        );
    }

    #[test]
    fn extra_whitespace_does_not_shift_the_span() {
        assert_eq!(
            locate(&kit(), "/kit   Iron"),
            Some(Cursor {
                node: 1,
                start: 7,
                prefix: "Iron".to_owned(),
            })
        );
    }

    #[test]
    fn a_position_offering_only_subcommands_is_answered_by_the_graph() {
        assert_eq!(locate(&perms(), "/perms "), None);
        assert_eq!(locate(&perms(), "/perms s"), None);
    }

    #[test]
    fn a_subcommand_literal_is_consumed_rather_than_read_as_a_value() {
        assert_eq!(
            locate(&perms(), "/perms set "),
            Some(Cursor {
                node: 11,
                start: 11,
                prefix: String::new(),
            })
        );
        assert_eq!(
            locate(&perms(), "/perms get Bo"),
            Some(Cursor {
                node: 21,
                start: 11,
                prefix: "Bo".to_owned(),
            })
        );
    }

    #[test]
    fn the_walk_reaches_the_second_argument_of_a_subcommand() {
        assert_eq!(
            locate(&perms(), "/perms set Bob "),
            Some(Cursor {
                node: 12,
                start: 15,
                prefix: String::new(),
            })
        );
        assert_eq!(
            locate(&perms(), "/perms set Bob Adm"),
            Some(Cursor {
                node: 12,
                start: 15,
                prefix: "Adm".to_owned(),
            })
        );
    }

    #[test]
    fn running_off_the_end_of_a_command_offers_nothing() {
        assert_eq!(locate(&perms(), "/perms set Bob Admin extra"), None);
        assert_eq!(locate(&perms(), "/perms get Bob "), None);
    }

    #[test]
    fn text_that_is_not_a_command_is_refused() {
        assert_eq!(locate(&kit(), "kit Iron"), None);
        assert_eq!(locate(&kit(), ""), None);
    }
}
