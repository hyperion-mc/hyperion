#![doc = include_str!("../README.md")]

/// The Nerd Font's rocket character.
pub const NERD_ROCKET: char = '\u{F14DE}';

/// The Nerd Font's fail character.
pub const FAIL_ROCKET: char = '\u{ea87}';

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[expect(clippy::print_stdout)]
    fn print_chars() {
        println!("Rocket: {NERD_ROCKET}");
        println!("Fail: {FAIL_ROCKET}");
    }
}
