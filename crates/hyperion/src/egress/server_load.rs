//! The host process's own CPU and memory, as two boss bars.
//!
//! Server telemetry is not a game concept, which is why it lives here and not
//! in an event crate, and it is drawn on the one surface a player already has
//! that can carry a number without covering anything up.
//!
//! # What a full bar means
//!
//! A boss bar is a fraction and neither of these quantities is one. A process
//! can use more than one core's worth of CPU -- 200% is two cores' worth,
//! which is what `top` says and what somebody asking "how loaded is the
//! server" means -- and it can allocate until the machine refuses. So both
//! bars are drawn against the point where the machine stops being able to give
//! more, every core it has and every byte of physical memory it has, and both
//! print that ceiling beside the reading:
//!
//! ```text
//! CPU 240% of 1600%
//! MEM 3.20 GiB of 128.0 GiB
//! ```
//!
//! which makes the choice checkable rather than a matter of taste:
//!
//! > **the fill is the quotient of the two numbers in the bar's own label.**
//!
//! `tools/hud-check.py` reads both numbers off the wire and divides. A ceiling
//! nobody printed cannot satisfy that, and neither can one invented to make
//! the bar look busy. It is also what forces the label to stay unnormalised:
//! the honest figure is the numerator, and the fill is that same fact taken
//! against the denominator standing next to it.
//!
//! # Why `getrusage` and not `ps`
//!
//! `ru_utime + ru_stime` is a cumulative counter, so a rate over a window is
//! the only reading it can give and the first sample can only report that it
//! has nothing yet. `ps` answers with a lifetime average instead, which is a
//! different question wearing the same units: a server that was busy for a
//! minute an hour ago reads busy under `ps` forever.

use std::{
    mem::MaybeUninit,
    time::{Duration, Instant},
};

use flecs_ecs::prelude::*;
use hyperion_minecraft_proto::{
    packets::play::player::{BossBarColor, BossBarOverlay},
    text::NamedColor,
};

use crate::egress::boss_bar::{BossBar, Progress, ShownTo, Style, Text, Title};

/// How long a CPU window is.
///
/// A rate needs a window to mean anything, and a window shorter than a tick on
/// a 20 tps server measures where the scheduler happened to be rather than how
/// busy the server is.
const WINDOW: Duration = Duration::from_secs(1);

/// How many steps a bar's fill is quantised to.
///
/// The bar is 182 pixels wide, so a step is under three pixels; what the
/// quantisation buys is the other direction, which is that a reading drifting
/// by a hundredth of a core does not cost every viewer a packet. See
/// hyperion#1018, which measured the same effect on the match bar.
const STEPS: f32 = 64.0;

/// A bar's two halves: what it says and how full it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Readout {
    /// The label, carrying the unnormalised reading and its ceiling.
    pub title: Text,
    /// `0.0..=1.0`.
    pub fill: f32,
}

/// The rolling sample the two bars are drawn from.
#[derive(Component, Debug, Default)]
pub struct Load {
    /// When the open window started, and the CPU counter it started at.
    window: Option<(Instant, Duration)>,
    /// Cores' worth of CPU over the last closed window.
    ///
    /// `None` until one has closed. One sample of a cumulative counter has no
    /// rate in it, and answering with the lifetime average instead is exactly
    /// the `ps` mistake this module exists to avoid.
    pub cores_used: Option<f64>,
    /// Resident set size in bytes, or `None` where the platform will not say.
    pub resident: Option<u64>,
}

/// The two bar entities, so `/serverload` can find them.
#[derive(Component, Debug, Clone, Copy)]
pub struct LoadBars {
    /// The CPU bar.
    pub cpu: Entity,
    /// The memory bar.
    pub memory: Entity,
}

impl Load {
    /// Fold one reading in, closing the window if it is due.
    ///
    /// Returns whether anything a bar draws has changed, so an idle server
    /// rewrites nothing.
    fn absorb(&mut self, now: Instant, cpu: Option<Duration>, resident: Option<u64>) -> bool {
        let mut moved = self.resident != resident;
        self.resident = resident;

        let Some(cpu) = cpu else {
            return moved;
        };
        match self.window {
            None => self.window = Some((now, cpu)),
            Some((started, before)) => {
                let elapsed = now.duration_since(started);
                if elapsed < WINDOW {
                    return moved;
                }
                let used = cpu.saturating_sub(before).as_secs_f64() / elapsed.as_secs_f64();
                moved = true;
                self.cores_used = Some(used);
                self.window = Some((now, cpu));
            }
        }
        moved
    }
}

/// The CPU bar for `cores_used` cores' worth against a machine of `cores`.
///
/// The label is the unnormalised figure and its ceiling; the fill is the one
/// divided by the other. Both numbers are percentages so that the ceiling
/// reads as what it is -- sixteen cores is 1600% -- rather than as a bare
/// count the reader has to convert.
#[must_use]
pub fn cpu_readout(cores_used: Option<f64>, cores: f64) -> Readout {
    let ceiling = percent(cores);
    let Some(used) = cores_used else {
        return Readout {
            title: Text::text("CPU sampling").color(NamedColor::Gray),
            fill: 0.0,
        };
    };
    let used_percent = percent(used);
    Readout {
        title: Text::text(format!("CPU {used_percent}% of {ceiling}%")).color(NamedColor::Aqua),
        fill: quantise(ratio(f64::from(used_percent), f64::from(ceiling))),
    }
}

/// The memory bar for `resident` bytes against `total` bytes of physical
/// memory.
///
/// Resident set and not virtual size, because resident is what the kernel
/// counts when it decides who to kill.
///
/// Two decimals on the reading and one on the ceiling: the reading is the
/// number being watched and needs the resolution, and the ceiling is a fixed
/// property of the box that only has to be recognisable. Both are far finer
/// than one step of the fill, which is what lets the gate divide one by the
/// other and get the fill back.
#[must_use]
pub fn memory_readout(resident: Option<u64>, total: Option<u64>) -> Readout {
    let (Some(resident), Some(total)) = (resident, total) else {
        return Readout {
            title: Text::text("MEM unavailable").color(NamedColor::Gray),
            fill: 0.0,
        };
    };
    let used = gibibytes(resident);
    let ceiling = gibibytes(total);
    Readout {
        title: Text::text(format!("MEM {used:.2} GiB of {ceiling:.1} GiB"))
            .color(NamedColor::Green),
        fill: quantise(ratio(used, ceiling)),
    }
}

/// Cores' worth as a whole percentage, which is the unit `top` reports in.
///
/// Saturating rather than wrapping at both ends: a machine with more than
/// forty million cores is allowed to have a wrong bar, and a negative rate,
/// which a clock that went backwards could produce, reads as idle.
fn percent(cores: f64) -> u32 {
    let scaled = (cores * 100.0).round();
    if !scaled.is_finite() || scaled <= 0.0 {
        return 0;
    }
    if scaled >= f64::from(u32::MAX) {
        return u32::MAX;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "bounded into 0..u32::MAX by the two tests above"
    )]
    let whole = scaled as u32;
    whole
}

fn gibibytes(bytes: u64) -> f64 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a byte count under 2^53 is exact"
    )]
    let bytes = bytes as f64;
    bytes / 1_073_741_824.0
}

/// `part` out of `whole`, clamped, and zero rather than a division by zero.
fn ratio(part: f64, whole: f64) -> f32 {
    let value = part / whole;
    if value.is_finite() {
        #[expect(clippy::cast_possible_truncation, reason = "clamped to 0..=1")]
        let value = value.clamp(0.0, 1.0) as f32;
        value
    } else {
        0.0
    }
}

/// `fraction` snapped down to a whole [`STEPS`] step.
fn quantise(fraction: f32) -> f32 {
    (fraction.clamp(0.0, 1.0) * STEPS).floor() / STEPS
}

/// The resident set size out of the text of `/proc/self/statm`.
///
/// A pure function over a string with a fixture beside it, because the gates
/// run on darwin where `/proc` does not exist: this parser cannot be exercised
/// on the machine that decides whether the change is good, and an untested
/// `/proc` reader shipped to a Linux fleet is the risk in this file.
///
/// The layout is `proc(5)`'s: `size resident shared text lib data dt`, in
/// pages. Field **one**, and the fixture is built so that every field holds a
/// different number and only the right index gives the right answer.
#[must_use]
pub fn resident_from_statm(text: &str, page_size: u64) -> Option<u64> {
    let pages: u64 = text.split_ascii_whitespace().nth(1)?.parse().ok()?;
    pages.checked_mul(page_size)
}

/// How many cores' worth of CPU this machine can give.
///
/// `available_parallelism` and not the raw processor count, because it follows
/// a cgroup CPU quota: in a container the honest ceiling is what the container
/// is allowed, not what the host has.
fn cores() -> f64 {
    std::thread::available_parallelism().map_or(1.0, |count| {
        #[expect(clippy::cast_precision_loss, reason = "no machine has 2^53 cores")]
        let count = count.get() as f64;
        count
    })
}

/// This process's CPU time so far, user plus system.
fn cpu_time() -> Option<Duration> {
    let mut usage = MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `getrusage` writes one `struct rusage` through the pointer it is
    // given and reads nothing through it; `RUSAGE_SELF` is one of its two
    // defined `who` arguments.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: the call returned zero, which is defined as having written the
    // whole struct.
    let usage = unsafe { usage.assume_init() };
    Some(elapsed(usage.ru_utime) + elapsed(usage.ru_stime))
}

fn elapsed(value: libc::timeval) -> Duration {
    let seconds = u64::try_from(value.tv_sec).unwrap_or(0);
    let micros = u32::try_from(value.tv_usec).unwrap_or(0);
    Duration::new(seconds, micros.saturating_mul(1_000))
}

/// Physical memory on this machine, in bytes.
fn total_memory() -> Option<u64> {
    // SAFETY: `sysconf` takes an int and returns a long; it has no
    // preconditions and touches no memory the caller owns.
    let pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) };
    // SAFETY: as above.
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    let pages = u64::try_from(pages).ok()?;
    let size = u64::try_from(size).ok()?;
    pages.checked_mul(size)
}

/// The system page size, for turning `/proc/self/statm`'s pages into bytes.
#[cfg(target_os = "linux")]
fn page_size() -> Option<u64> {
    // SAFETY: see `total_memory`.
    u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok()
}

/// This process's resident set size, in bytes.
#[cfg(target_os = "linux")]
fn resident_bytes() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/statm").ok()?;
    resident_from_statm(&text, page_size()?)
}

/// This process's resident set size, in bytes.
#[cfg(target_os = "macos")]
fn resident_bytes() -> Option<u64> {
    let mut info = MaybeUninit::<libc::proc_taskinfo>::zeroed();
    let size = i32::try_from(size_of::<libc::proc_taskinfo>()).ok()?;
    // SAFETY: `proc_pidinfo` writes at most `size` bytes of `struct
    // proc_taskinfo` through the pointer, which points at exactly that much
    // owned, zeroed storage.
    let written = unsafe {
        libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDTASKINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    // A short write means the kernel's struct is not the one `libc` describes,
    // and reading `pti_resident_size` out of it would be reading whichever
    // field happens to land at that offset instead.
    if written != size {
        return None;
    }
    // SAFETY: the call wrote the whole struct, as just checked.
    Some(unsafe { info.assume_init() }.pti_resident_size)
}

/// This process's resident set size, in bytes.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn resident_bytes() -> Option<u64> {
    None
}

/// Show or hide the load bars for `viewer`, and say which it just did.
///
/// One audience, shared by both bars, so they cannot get out of step: this
/// adds and removes `(ShownTo, viewer)` and everything else -- the `Add`
/// packets, the `Remove` packets, the diffing in between -- falls out of
/// `egress::boss_bar`.
#[must_use]
pub fn toggle(world: WorldRef<'_>, viewer: Entity) -> bool {
    world.get::<&LoadBars>(|bars| {
        let showing = world
            .entity_from_id(bars.cpu)
            .has((id::<ShownTo>(), viewer));
        for bar in [bars.cpu, bars.memory] {
            let bar = world.entity_from_id(bar);
            if showing {
                bar.remove((id::<ShownTo>(), viewer));
            } else {
                bar.add((id::<ShownTo>(), viewer));
            }
        }
        !showing
    })
}

#[derive(Component)]
pub struct ServerLoadModule;

impl Module for ServerLoadModule {
    fn module(world: &World) {
        world.component::<Load>().add_trait::<flecs::Singleton>();
        world
            .component::<LoadBars>()
            .add_trait::<flecs::Singleton>();
        world.set(Load::default());

        // Two bars, made once and shown to nobody. A bar with an empty
        // audience costs one query row a tick and sends nothing at all, which
        // is what makes "opt in" a relationship rather than a lifecycle.
        //
        // One colour each and no thresholds. A colour change is an alarm, and
        // an alarm at a fraction nobody can justify is noise on a readout that
        // is already carrying the number. `Progress` and not a notched overlay
        // for the same reason: notches would claim the quantity comes in
        // twenty pieces, and it does not.
        let cpu = world
            .entity_named("hyperion::server_load::cpu")
            .add(id::<BossBar>())
            .set(Title(cpu_readout(None, cores()).title))
            .set(Progress(0.0))
            .set(Style {
                colour: BossBarColor::Blue,
                overlay: BossBarOverlay::Progress,
            })
            .id();
        let memory = world
            .entity_named("hyperion::server_load::memory")
            .add(id::<BossBar>())
            .set(Title(memory_readout(None, None).title))
            .set(Progress(0.0))
            .set(Style {
                colour: BossBarColor::Green,
                overlay: BossBarOverlay::Progress,
            })
            .id();
        world.set(LoadBars { cpu, memory });

        // PreStore, so a reading taken this tick is on the bar's components
        // before `boss_bar_sync` runs in OnStore and reaches the wire the same
        // tick rather than the next one.
        world
            .system_named::<&mut Load>("server_load_sample")
            .kind(id::<flecs::pipeline::PreStore>())
            .each_iter(|it, _, load| {
                if !load.absorb(Instant::now(), cpu_time(), resident_bytes()) {
                    return;
                }
                let world = it.world();
                let cpu = cpu_readout(load.cores_used, cores());
                let memory = memory_readout(load.resident, total_memory());
                world.get::<&LoadBars>(|bars| {
                    for (bar, readout) in [(bars.cpu, &cpu), (bars.memory, &memory)] {
                        world
                            .entity_from_id(bar)
                            .set(Title(readout.title.clone()))
                            .set(Progress(readout.fill));
                    }
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The label is what `top` would say, and the fill is that against the
    /// machine. Getting these two the same way round is the whole point of the
    /// bar.
    #[test]
    fn the_cpu_label_is_unnormalised_and_the_fill_is_not() {
        let readout = cpu_readout(Some(2.4), 16.0);
        assert_eq!(readout.title.plain(), "CPU 240% of 1600%");
        // 240/1600 = 0.15, which is 9.6 steps, so it draws at nine.
        assert!((readout.fill - 9.0 / 64.0).abs() < 1e-6, "{}", readout.fill);
        assert!((0.0..=1.0).contains(&readout.fill));
    }

    /// A process over a whole core still reports what it is using. The fill
    /// has nowhere left to go and says so by being full; the label does not
    /// lie to keep it company.
    #[test]
    fn a_bar_that_is_full_still_says_how_far_past_full_it_is() {
        let readout = cpu_readout(Some(2.4), 1.0);
        assert_eq!(readout.title.plain(), "CPU 240% of 100%");
        assert!((readout.fill - 1.0).abs() < 1e-6, "{}", readout.fill);
    }

    /// One sample of a cumulative counter has no rate in it.
    #[test]
    fn the_first_sample_reports_nothing_rather_than_an_average() {
        let mut load = Load::default();
        let start = Instant::now();
        assert!(!load.absorb(start, Some(Duration::from_secs(90)), None));
        assert_eq!(load.cores_used, None);
        assert_eq!(
            cpu_readout(load.cores_used, 16.0).title.plain(),
            "CPU sampling"
        );

        // A window shorter than `WINDOW` is still not a reading.
        assert!(!load.absorb(
            start + Duration::from_millis(500),
            Some(Duration::from_secs(91)),
            None
        ));
        assert_eq!(load.cores_used, None);

        // Two seconds of CPU across two seconds of wall clock is one core.
        assert!(load.absorb(
            start + Duration::from_secs(2),
            Some(Duration::from_secs(92)),
            None
        ));
        assert_eq!(load.cores_used, Some(1.0));
        assert_eq!(
            cpu_readout(load.cores_used, 16.0).title.plain(),
            "CPU 100% of 1600%"
        );
    }

    /// The property the whole ceiling choice is defensible under: whatever the
    /// numbers, the fill is the quotient of the two in the label.
    #[test]
    fn the_fill_is_the_quotient_of_the_two_numbers_in_the_label() {
        for (used, cores) in [
            (0.0_f64, 1.0_f64),
            (0.005, 8.0),
            (1.0, 8.0),
            (7.9, 8.0),
            (12.34, 16.0),
        ] {
            let readout = cpu_readout(Some(used), cores);
            let label = readout.title.plain();
            let (numerator, denominator) = label
                .strip_prefix("CPU ")
                .and_then(|rest| rest.split_once("% of "))
                .expect(&label);
            let numerator: f64 = numerator.parse().expect(&label);
            let denominator: f64 = denominator.trim_end_matches('%').parse().expect(&label);
            let quoted = numerator / denominator;
            assert!(
                (f64::from(readout.fill) - quoted).abs() <= 1.0 / 64.0,
                "{label} draws at {} but its own numbers say {quoted}",
                readout.fill
            );
        }
    }

    /// The reading is the second field, and every other field in the fixture
    /// holds a different number so an off-by-one cannot pass.
    #[test]
    fn statm_reads_the_resident_field_and_not_a_neighbour() {
        // `size resident shared text lib data dt`, in pages.
        let statm = "70086 6789 4321 21 0 1234 0\n";
        assert_eq!(resident_from_statm(statm, 4096), Some(6789 * 4096));
        assert_eq!(resident_from_statm(statm, 16384), Some(6789 * 16384));
    }

    /// A `/proc` read that is not what this expects reports nothing rather
    /// than a number, because a wrong memory bar is worse than an absent one.
    #[test]
    fn a_statm_line_that_is_not_one_reads_as_nothing() {
        assert_eq!(resident_from_statm("", 4096), None);
        assert_eq!(resident_from_statm("70086", 4096), None);
        assert_eq!(resident_from_statm("70086 nonsense", 4096), None);
        assert_eq!(resident_from_statm("70086 -1 4321", 4096), None);
        // Pages times page size has to fit in the units it is reported in.
        assert_eq!(resident_from_statm("1 18446744073709551615 2", 4096), None);
    }

    #[test]
    fn the_memory_label_carries_its_own_ceiling() {
        let readout = memory_readout(Some(3_435_973_836), Some(137_438_953_472));
        assert_eq!(readout.title.plain(), "MEM 3.20 GiB of 128.0 GiB");
        assert!((f64::from(readout.fill) - 0.025).abs() <= 1.0 / 64.0);
        assert_eq!(
            memory_readout(None, Some(1)).title.plain(),
            "MEM unavailable"
        );
    }

    /// The real samplers answer on the machine the tests run on. Neither is a
    /// number this can predict, so the claim is only that the platform path
    /// exists and is plausible.
    #[test]
    fn this_process_can_read_its_own_cpu_and_memory() {
        assert!(cpu_time().is_some(), "getrusage said nothing");
        let resident = resident_bytes().expect("this platform has an rss reader");
        assert!(
            resident > 1 << 20,
            "{resident} bytes resident is not a process"
        );
        let total = total_memory().expect("this platform reports its memory");
        assert!(total > resident, "{resident} resident of {total} total");
    }
}
