//! Does the same input produce the same game twice?
//!
//! Every other generated test in this directory rests on this one. A property
//! test that runs a script and checks an invariant is only meaningful if the
//! script determines the run; if it does not, a failure is unreproducible, a
//! shrunk counterexample is fiction, and the honest response to a red build is
//! to run it again. That failure mode is worth finding on purpose.
//!
//! The comparison is bit exact and per tick. Floats go in by their bits rather
//! than through a tolerance, because a tolerance would hide precisely the slow
//! divergence this exists to catch, and the tick index is recorded so a failure
//! names where the two runs parted rather than only that they did.

// The generator's arithmetic is modular by definition: an LCG is its overflow,
// and every `as usize` below feeds an index the driver takes modulo the number
// of players anyway.
#![expect(
    clippy::cast_possible_truncation,
    reason = "the generator's values are bounded well inside every target it is cast to"
)]

mod harness;

use harness::{Action, Fingerprint, Game, Script};

/// A deterministic script generator.
///
/// An LCG rather than proptest: a replay test wants the *same* long script
/// every run, and wants it without a shrinker deciding to try a shorter one.
/// The constants are Numerical Recipes'.
struct Rng(u64);

impl Rng {
    const fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 11
    }

    const fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn unit(&mut self) -> f32 {
        self.below(1_000_000) as f32 / 1_000_000.0
    }
}

/// A dense script: hits, ability uses, movement and kit changes, mixed with
/// ticks, long enough to run a whole match and start the next one.
fn scripted(seed: u64, players: usize, actions: usize) -> Script {
    let mut rng = Rng(seed);
    let actions = (0..actions)
        .map(|_| match rng.below(12) {
            0..=5 => Action::Tick,
            6 | 7 => Action::Hit {
                attacker: rng.below(8) as usize,
                victim: rng.below(8) as usize,
                amount: rng.unit() * 30.0,
                kind: rng.below(256) as u8,
            },
            8 | 9 => Action::UseSlot {
                player: rng.below(8) as usize,
                slot: rng.below(9) as u8,
            },
            10 => Action::Move {
                player: rng.below(8) as usize,
                to: glam::Vec3::new(
                    rng.unit().mul_add(128.0, -64.0),
                    rng.unit().mul_add(128.0, -32.0),
                    rng.unit().mul_add(128.0, -64.0),
                ),
            },
            _ => Action::SelectKit {
                player: rng.below(8) as usize,
                kit: rng.below(64) as usize,
            },
        })
        .collect();
    Script { players, actions }
}

/// Report the first tick at which two runs disagree, if any.
fn first_divergence(left: &[Fingerprint], right: &[Fingerprint]) -> Option<String> {
    if left.len() != right.len() {
        return Some(format!(
            "the two runs produced {} and {} ticks",
            left.len(),
            right.len()
        ));
    }
    left.iter()
        .zip(right)
        .enumerate()
        .find(|(_, (a, b))| a != b)
        .map(|(tick, (a, b))| {
            let what = match (a.world == b.world, a.calls == b.calls) {
                (false, false) => "the world state and the calls to the server",
                (false, true) => "the world state",
                _ => "the calls to the server, with the world state agreeing",
            };
            format!("tick {tick}: {what} differ ({a:?} against {b:?})")
        })
}

#[test]
fn replaying_a_script_reproduces_every_tick() {
    for seed in [1u64, 7, 99, 12_345, 0xdead_beef] {
        let script = scripted(seed, 4, 900);

        let first = Game::from_script(&script).run_recording(&script);
        let second = Game::from_script(&script).run_recording(&script);

        assert!(
            !first.is_empty(),
            "seed {seed} produced a script with no ticks in it"
        );
        assert!(
            first_divergence(&first, &second).is_none(),
            "seed {seed} is not reproducible: {}",
            first_divergence(&first, &second).unwrap_or_default()
        );
    }
}

/// The same script run four times, not two.
///
/// Two runs agreeing can be luck when the source of nondeterminism is a hash
/// seed or an allocator address that happens to land the same way. Four runs
/// interleaved with unrelated worlds is a much less comfortable place for that
/// kind of bug to hide.
#[test]
fn a_script_is_reproducible_across_several_runs_and_unrelated_worlds() {
    let script = scripted(2_024, 5, 700);
    let reference = Game::from_script(&script).run_recording(&script);

    for attempt in 0..3 {
        // A whole unrelated world in between, so any per-process state that
        // leaks between worlds -- an entity id counter, a static, a cached
        // component id -- has a chance to shift.
        let noise = scripted(attempt + 500, 3, 200);
        drop(Game::from_script(&noise).run_recording(&noise));

        let again = Game::from_script(&script).run_recording(&script);
        assert!(
            first_divergence(&reference, &again).is_none(),
            "attempt {attempt} diverged: {}",
            first_divergence(&reference, &again).unwrap_or_default()
        );
    }
}

/// The exact sequence of calls that reached the server is reproduced, not just
/// a hash of it.
///
/// The hash says two runs differ; this says what the first difference was,
/// which is the part somebody debugging actually needs. It is also a stronger
/// claim than state equality: two runs can arrive at the same world having told
/// the clients different things on the way, and only the log notices.
#[test]
fn the_call_log_is_reproduced_call_for_call() {
    let script = scripted(31_337, 4, 800);

    let first = Game::from_script(&script);
    first.run(&script);
    let second = Game::from_script(&script);
    second.run(&script);

    let left = first.server.calls();
    let right = second.server.calls();

    for (index, (a, b)) in left.iter().zip(&right).enumerate() {
        assert_eq!(a, b, "call {index} differs between two runs of one script");
    }
    assert_eq!(
        left.len(),
        right.len(),
        "the two runs made different numbers of calls to the server"
    );
    assert!(
        !left.is_empty(),
        "the script never reached the server, so this proves nothing"
    );
}

/// An empty script leaves the world exactly as it was built.
///
/// The base case, and the one that catches a `Game::from_script` that is itself
/// nondeterministic -- which would make every test above pass for the wrong
/// reason, since both sides would be equally wrong.
#[test]
fn two_freshly_built_worlds_are_identical() {
    let script = Script {
        players: 4,
        actions: Vec::new(),
    };
    assert_eq!(
        Game::from_script(&script).fingerprint(),
        Game::from_script(&script).fingerprint()
    );
}
