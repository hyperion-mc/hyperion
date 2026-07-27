# Hyperion with no operating system

Experimental. Nothing here is on any normal build path, and none of it is
merged.

## What this achieved: the protocol layer boots on a unikernel and answers a real client

`nix build .#bare-metal` produces an `x86_64-unknown-hermit` machine image;
`nix run .#bare-metal-vm` boots it under QEMU, where it brings up virtio-net,
takes a DHCP lease and answers a Minecraft server-list ping decoded with the
same `valence_protocol` the real server uses. Real output is in
[Evidence](#evidence).

What that does **not** mean: the game server does not build. Eleven of
twenty-four workspace crates compile for the target and thirteen do not,
including `hyperion` itself. The gap and its causes are in
[The blocker survey](#the-blocker-survey).

## What to reach for: Hermit, the only live Rust unikernel with a real `std`

The choice matters less than it looks, because only two candidates ship a
`std`, and only one of them has virtio-net drivers you can read.

| Option | Version / date checked 2026-07-27 | What it gives | What it costs |
| --- | --- | --- | --- |
| **Hermit** (`hermit-os/kernel`, `hermit-os/hermit-rs`) | kernel **v0.13.2**, 2026-03-13; both repos pushed 2026-07-27 | A genuine `std` behind four Tier-3 triples (`x86_64`, `aarch64`, `aarch64_be`, `riscv64gc`), upstream `library/std/src/sys/pal/hermit`. virtio-net, virtio-fs, virtio-vsock, smoltcp 0.13, DHCPv4 on by default. QEMU, Firecracker and uhyve in CI. | Tier 3, so `-Z build-std`. Kernel pins its own monthly nightly. `std::process` absent entirely; `std::fs` missing rename, symlink, canonicalize and all locking. **Tokio only through two unmaintained forks.** Nothing in nixpkgs. |
| **Motor OS** (`moturus/motor-os`) | Tier-3 target `x86_64-unknown-motor` since 2025-10-17; pushed 2026-07-27 | Rust microkernel, VM-only, `std` upstream, smoltcp, ~100 ms boot, ~10 Gbps guest↔host. | No DHCP (static IPs only), Tokio only partly ported, essentially one maintainer, no security audit by its own account. |
| **Unikraft** | v0.21.0, 2026-04/05; project healthy | Mature C unikernel; runs ordinary musl Rust binaries through its binary-compatibility layer. | **Its Rust integration is dead.** `unikraft/lib-rust` last pushed 2024-01-02; the catalog's Rust examples are all pinned to Rust 1.75 (Dec 2023); the `x86_64-unikraft-linux-musl` target's own rustc docs say linking needs a KraftKit shim and point at an issue closed in 2023 with no successor. |
| **`no_std` + `virtio-drivers` + `smoltcp`** | `virtio-drivers` 0.13.0 (2026-03-03), `smoltcp` 0.13.1 (2026-04-30) | Works on stable against Tier-2 `x86_64-unknown-none`, no `build-std`, no custom JSON target. | No `std`, ever. You write the boot path, the allocator and the event loop. smoltcp was still fixing TCP panics in April 2026. |
| **Nanos/OPS, OSv** | nanos 0.1.55 (2026-04-26); OSv last tag 2022-12-20, master alive | Run unmodified Linux ELFs, so "Rust support" is `--target x86_64-unknown-linux-musl`. | The kernel is an opaque C blob; no Rust-level control of the virtio stack. |
| **Firecracker + minimal Linux** | v1.16.1, 2026-07-02 | The boring baseline, and **the only option where virtio-mem actually works** (guest driver landed 1.14.0; needs `CONFIG_VIRTIO_MEM`, Linux ≥5.16). | It is a Linux VM. Nothing bare-metal about the guest. |

### virtio-mem does not exist in Rust guest land

The brief assumed virtio-mem alongside virtio-net. It is not available on any
Rust unikernel. Hermit's `src/drivers/` has `console`, `fs`, `net`, `vsock` and
nothing else; the only matches for `virtio.mem` in the whole kernel tree are
three occurrences of `virtio_mem_barrier`, a memory *barrier* helper. The
`virtio-drivers` crate has no virtio-mem driver, no issue and no PR proposing
one; ballooning is an open PR (#251, 2026-06-15) and unmerged. If elastic guest
memory is a requirement, that is a from-scratch driver plus kernel allocator
work, or it is Firecracker with a Linux guest.

### `crates.io` is a dead end for Hermit, and says so

```
error: This crate is no longer distributed via crates.io. Use the crate via Git instead.
 --> ~/.cargo/registry/src/index.crates.io-.../hermit-0.13.0/src/lib.rs:4:1
```

Both `hermit` and `hermit-kernel` on crates.io are `compile_error!` stubs. The
dependency has to be a git tag.

## The toolchain finding that costs a day if you meet it the hard way

**Hyperion's pinned `nightly-2025-02-22` builds a Hermit image that boots,
prints, and then fails every socket call.** It is not a networking problem and
it does not look like a toolchain problem:

```
[    0.409679][0][INFO  hermit    ] Jumping into application
[app] hermit unikernel up

thread 'main' panicked at src/main.rs:8:54:
bind: Kind(Uncategorized)
```

The kernel and the application implement two halves of one `hermit-abi`. The
application's `std` is compiled from that nightly's `rust-src`, which pulls
`hermit-abi 0.4.0`; kernel 0.13.2 answers `hermit-abi 0.5`. Boot and `println!`
happen to line up across that gap and sockets do not. Rebuilding the identical
source on `nightly-2026-07-01` — the kernel's own pin — binds first try.

So `bare-metal/` carries its own `rust-toolchain.toml`. Two toolchains in one
repo is a cost, and the alternative is worse: moving the whole workspace onto a
2026 nightly to satisfy a target nobody ships yet.

`nightly-2025-02-22` is also simply too old for the current `hermit` crate's
build dependencies, which is the failure you hit first:

```
error: rustc 1.87.0-nightly is not supported by the following package:
  home@0.5.12 requires rustc 1.88
```

### Unstable features relied on

Exactly one: `-Z build-std=std,panic_abort`, plus the `rust-src` and
`llvm-tools` components. No feature gates in any source file, no custom JSON
target. `-Z build-std` is a funded 2026 Rust Project Goal with RFC 3873
accepted, but its goal page states that using `std` with *custom* targets is
out of scope — builtin Tier-3 targets like Hermit's are the ones on the
stabilisation path, which is the right side of that line to be on.

There is a second route worth knowing: `hermit-os/rust-std-hermit` publishes an
installable `rust-std` component tracking stable point releases (1.97.1,
2026-07-20), which removes `-Z build-std` entirely. This branch does not use it,
because it would pair a stable-built `std` with a nightly-built kernel — the
exact pairing whose ABI drift is documented above.

## The blocker survey: 11 of 24 crates compile, and four dependencies explain the rest

Method, so the numbers can be read correctly:

```sh
RUSTFLAGS="--cfg tokio_unstable" cargo +nightly-2025-02-22 build \
  -Z build-std=std,panic_abort --target x86_64-unknown-hermit -p <crate>
```

This is a *reachability* survey run per package, so a failure is the **first**
wall, not a complete list: cargo stops at the first crate that will not build,
and anything behind it is untested. Read a "blocked by socket2" row as "gets no
further than socket2", not as "socket2 is the only problem".

The survey ran on `nightly-2025-02-22` deliberately, to answer "what does the
tree as it stands do", not "what could it do". Since compilation does not
exercise the syscall ABI, the 2025 toolchain gives the same answer here as the
2026 one would.

### Compiles for `x86_64-unknown-hermit` today (11)

`hyperion-minecraft-proto`, `hyperion-nerd-font`, `hyperion-scheduled`,
`hyperion-stats`, `simd-utils`, `geometry`, `hyperion-proto`,
`hyperion-palette`, `hyperion-text`, `hyperion-crafting`, `packet-channel`.

Two of those are more interesting than they look. `hyperion-palette` pulls
`valence_protocol` with the `compression` feature, so **the whole Minecraft
protocol codec cross-compiles unmodified** — that is what made the demo
possible. `hyperion-crafting` and `packet-channel` pull `bevy` with
`multi_threaded`, so **bevy_ecs cross-compiles too**.

### Does not compile (13)

| Crate | Stops at | Why | Fixable how |
| --- | --- | --- | --- |
| `bvh-region` | `wait-timeout` | `proptest` is in `[dependencies]`, not `[dev-dependencies]`; it pulls `rusty-fork` → `wait-timeout`, which has a `sys` module for unix and windows only. | Move `proptest` to `[dev-dependencies]`. This is a bug on Linux too — it ships a test framework into release builds. |
| `hyperion-utils` | `socket2`, `openssl-sys` | `reqwest` with default features on, so `default-tls` → `native-tls` → `openssl-sys`, despite `rustls-tls` also being requested. | `default-features = false` on `reqwest` removes openssl outright. `socket2` needs Hermit's fork. |
| `hyperion-command`, `hyperion-gui`, `hyperion-item`, `hyperion-genmap`, `hyperion-clap`, `hyperion-permission` | `socket2`, `openssl-sys`, `libz-ng-sys`, `ring`, `wait-timeout` | All reach these through `hyperion` or `hyperion-utils`. None has a blocker of its own. | Fix the four below and re-survey. |
| `hyperion-proxy`, `hyperion-proxy-module` | `socket2` | `tokio` with `net`. | Hermit's `socket2` and `tokio` forks. |
| `hyperion` | `socket2`, `openssl-sys`, `libz-ng-sys`, `ring`, `wait-timeout` | Everything at once, plus more behind it. | See below. |
| `bedwars` | as `hyperion`, plus `std::os::unix` | `error[E0433]: could not find 'unix' in 'os'`. | A platform seam call, which is what `hyperion-platform` is for. |

The four dependencies that cause almost every failure:

| Dependency | Reached via | Nature | Cost to fix |
| --- | --- | --- | --- |
| `socket2` | `tokio` (net), `reqwest` | `error: Socket2 doesn't support the compile target` | Low. `hermit-os/socket2` fork exists and is what hermit-rs patches in. |
| `openssl-sys` | `reqwest` default features | C library, needs a cross toolchain | Low, and worth doing anyway: the tree asks for `rustls-tls` and gets openssl as well. |
| `libz-ng-sys` | `flate2` with `zlib-ng` in `hyperion` | C library | Medium. Dropping to `flate2`'s pure-Rust `miniz_oxide` backend costs compression throughput on the hot egress path. |
| `ring` | rustls crypto provider | C and assembly | Medium. `aws-lc-rs` is also C; `rustls` has no pure-Rust provider in this tree. |

### Blockers the survey never reached, from reading the manifests

These are **unverified**. Each sits behind one of the four above, so no build
has ever gotten far enough to confirm or refute them. Listing them because they
are the ones that decide whether the server can ever boot, not because they have
been measured:

| Dependency | Used by | Why it looks fatal |
| --- | --- | --- |
| `heed` (LMDB) | `hyperion` player DB, `hyperion-permission` | C library, `mmap`, file locking. Hermit's `std::fs` has no locking at all. |
| `memmap2` | `hyperion` Anvil region reader | `mmap` of a file; there is no file. |
| `libdeflater` | `hyperion` chunk compression | C library. |
| `tikv-jemallocator` | `hyperion`, `bedwars` | C allocator; a unikernel supplies its own. Already `cfg`'d off on Windows, so the seam exists. |
| `ndarray` with `blas` | `hyperion` | Requires a system BLAS. |
| `tracing-tracy` | `hyperion` | Profiler client over a socket. |
| `valence_anvil` | `hyperion` | Reads region files from disk. |
| `directories` | `hyperion-utils` | Asks the OS where the home directory is. |
| `tokio` | everywhere | Runs on Hermit **only** via `hermit-os/tokio`, pinned to 1.45.0, last commit 2025-05-08 and unmaintained. The tree pins tokio 1.45.0, so the versions line up today and will not stay lined up. `mio` and `polling` have genuine upstream Hermit support; tokio does not. |

### The honest verdict

Of the three outcomes the brief offered, this is the middle one, and nearer its
lower end than its upper. A meaningful subset — the protocol codec, the ECS, the
geometry and BVH primitives, and the wire format — compiles and one of them
demonstrably runs. The server does not, and the distance to it is not a
weekend: `heed`, `memmap2`, `libdeflater` and `ndarray+blas` are four C
libraries that a unikernel with no filesystem cannot host, so closing that gap
means replacing persistence and chunk storage, not porting them.

The realistic next milestone is not "the server boots". It is **the proxy**,
which needs only `socket2` + `tokio` forks and has no filesystem dependency
worth the name.

## The seam: five things, named after what differs

`crates/hyperion-platform` is deliberately not a portability layer. Hermit
supplies a real `std`, so almost nothing needs wrapping. The survey found five
places where a hosted OS and a unikernel genuinely disagree, and the crate is
those five and nothing else:

| Module | Hosted | Unikernel |
| --- | --- | --- |
| `limits::raise_open_files` | `setrlimit(RLIMIT_NOFILE)` | no such limit; reports the ceiling asked for |
| `clock::wall_clock` | `Some(SystemTime::now())` | `None` — no RTC unless the hypervisor gives one |
| `storage::store` | the filesystem | RAM, seeded by the image, `is_persistent() == false` |
| `net` | `std::net` plus `AF_UNIX` | `std::net` over virtio-net only |
| `parallelism::available` | CPUs, as a hint | vCPUs handed over at boot, exactly |

Plus a `CAPABILITIES` constant, so a call site asks "is there a filesystem?"
rather than "am I on Unix?". The first question survives a new platform; the
second does not.

The backend is one `cfg` arm in `src/backend.rs`. Adding a third platform is a
new `backend/*.rs`, that arm, and a row in the table. Hosted is the default and
is byte-for-byte the behaviour hyperion already had.

### What the seam is not yet

**It is not wired into `hyperion`.** Not one existing crate was changed. Wiring
it in would buy nothing today — `hyperion` is blocked on four C libraries, not
on `cfg(unix)` — and would put a diff into files three other streams are
editing. The demonstration that the seam works is `bare-metal/hyperion-unikernel`,
which uses it on both platforms and reports different, correct answers on each.

The call sites it is *for* are already identified, and they are few:

| File | What it does |
| --- | --- |
| `crates/hyperion/src/lib.rs:37,94` | `libc::getrlimit`/`setrlimit`, already `#[cfg(unix)]`-gated → `limits::raise_open_files` |
| `crates/hyperion/src/lib.rs:205` | `std::thread::Builder` with an explicit stack → `parallelism::spawn_worker` |
| `crates/hyperion/src/common/config.rs:80,94` | `fs::create_dir_all`, `fs::write` → `storage::store` |
| `crates/hyperion/src/storage/db.rs:24` | `fs::create_dir_all` for LMDB → `storage`, once LMDB itself is replaced |
| `crates/hyperion/src/simulation/blocks/region.rs:30,148` | `memmap2` + `File::open` → `storage`, same caveat |
| `crates/hyperion-proxy/src/main.rs:6,8,125` | `TcpListener`, `UnixListener`, `lookup_host` → `net::supports_unix_sockets`, `net::supports_dns` |
| `events/bedwars/src/lib.rs` | `std::os::unix` |

That is the whole OS surface of the tree. It is smaller than the dependency
list suggests, which is the encouraging part of this exercise: hyperion's own
code is close to portable, and its dependencies are not.

## Nix

Three cargo invocations happen inside one `nix build`:

1. the application, from `bare-metal/Cargo.lock`;
2. the standard library, because `-Z build-std` compiles `std` from source and
   resolves its own lockfile out of `rust-src`;
3. the Hermit kernel, which the `hermit` crate's `build.rs` builds by shelling
   out to a nested cargo in the kernel's source tree.

The third one is why this is a single `cargo vendor --sync` producing one
vendor directory rather than three `fetchCargoVendor` calls: that build script
strips **every** `CARGO_*` and `RUST_*` variable from its environment before
running, so `$HOME/.cargo/config.toml` is the only channel that reaches it.

Three further things the sandbox needed, each a one-line comment in
`nix/bare-metal.nix`:

- `CARGO_NET_GIT_FETCH_WITH_CLI=true` — cargo's bundled libgit2 cannot complete
  a TLS handshake in a fixed-output derivation; the `git` binary can.
- `dontFixup = true` on the vendor derivation — `patchShebangs` rewrites
  vendored CI scripts to a bash store path, which both adds a store reference a
  fixed-output derivation may not have and invalidates cargo's checksums.
- A `rustup` shim — the kernel's `xtask` runs `rustup target add
  x86_64-unknown-none` unconditionally. The shim answers that and *fails loudly*
  on anything else, so a future rustup call does not silently no-op.

`RUSTFLAGS=""` is set on the build. The repo's `.cargo/config.toml` puts
`-Ctarget-cpu=native` in `[build] rustflags`, which applies to every target, so
any cross-compile emits host-CPU instructions and dies inside `core`:

```
'apple-m4' is not a recognized processor for this target (ignoring processor)
rustc-LLVM ERROR: 64-bit code requested on a subtarget that doesn't support it!
```

`RUSTFLAGS` overrides it, which is why `.cargo/config.toml` is untouched — no
existing build changes behaviour.

## Evidence

Everything below is real output, on `aarch64-darwin`, cross-compiling to
`x86_64-unknown-hermit`.

### `nix build .#bare-metal`

```
$ nix build .#bare-metal --no-link -L
...
hyperion-unikernel>    Compiling valence_protocol v0.2.0-alpha.1+mc.1.20.1 (https://github.com/TestingPlant/valence?branch=feat-bytes#fb792dcb)
hyperion-unikernel>    Compiling hyperion-unikernel v0.1.0 (/nix/var/nix/builds/nix-50057-576481663/source/bare-metal/hyperion-unikernel)
hyperion-unikernel>     Finished `release` profile [optimized] target(s) in 2m 00s
hyperion-unikernel> Running phase: installPhase
$ echo $?
0
```

### `nix run .#bare-metal-vm`, then a Minecraft ping from the host

The guest console. Note virtio-net negotiating features, DHCP, and the
platform seam reporting the unikernel's capability set rather than the hosted
one:

```
[    0.286342][0][INFO  pci       ] Virtio network driver initialized.
[    0.289592][0][INFO  network   ] Try to initialize network!
[    0.290297][0][INFO  device    ] MAC address: 52-54-00-12-34-56
[    0.291930][0][INFO  device    ] ChecksumCapabilities { ipv4: Both, udp: Both, tcp: Both, icmpv4: Both, icmpv6: Both }
[    0.292994][0][INFO  device    ] MTU: 1514 bytes
[    0.302772][0][INFO  network   ] DHCP config acquired!
[    0.303104][0][INFO  network   ] IP address:   192.168.76.9/24
[    0.303530][0][INFO  network   ] Gateway:      192.168.76.2
[    0.310927][0][INFO  hermit    ] Jumping into application
[hyperion] platform: unikernel
[hyperion] capabilities: Capabilities { persistent_storage: false, unix_sockets: false, dns: false, trustworthy_wall_clock: false, adjustable_file_limit: false, subprocesses: false }
[hyperion] parallelism: 2
[hyperion] wall clock: unavailable on this platform
[hyperion] listening on 0.0.0.0:25565 after 13.166ms
[hyperion] accepted Ok(192.168.76.2:53071)
[hyperion] handshake: protocol=763 host=127.0.0.1 port=25599 next=Status
[hyperion] sent status
[hyperion] ponged 0xdeadbeefcafef00d
```

The client's side of the same exchange:

```
=== minecraft ping from host, 127.0.0.1:25599 ===
STATUS: {
  "version": {
    "name": "hyperion/unikernel",
    "protocol": 763
  },
  "players": {
    "max": 10000,
    "online": 0,
    "sample": []
  },
  "description": {
    "text": "hyperion on unikernel, no operating system"
  }
}
PONG: 0xdeadbeefcafef00d in 3.6 ms
client rc=0
```

Boot to listening socket: **13 ms**.

### The same binary on the host

```
$ HYPERION_PORT=25599 ./bare-metal/target/debug/hyperion-unikernel
[hyperion] platform: hosted
[hyperion] capabilities: Capabilities { persistent_storage: true, unix_sockets: true, dns: true, trustworthy_wall_clock: true, adjustable_file_limit: true, subprocesses: true }
[hyperion] parallelism: 18
[hyperion] wall clock: SystemTime { tv_sec: 1785138928, tv_nsec: 86423000 }
[hyperion] listening on 0.0.0.0:25599 after 229.25µs
[hyperion] accepted Ok(127.0.0.1:61665)
[hyperion] handshake: protocol=763 host=127.0.0.1 port=25599 next=Status
[hyperion] sent status
[hyperion] ponged 0xdeadbeefcafef00d
```

One source file, two platforms, different and correct answers from the seam.

### Lints

```
$ cargo clippy -p hyperion-platform --all-targets   # rc=0
$ cd bare-metal && cargo clippy                     # rc=0
```

## What was not verified

Stated plainly, because these are the parts a reader cannot see for themselves.

- **`nix build .#default` fails, and already did.** The error is
  `A hash was specified for divan-0.1.17, but there is no corresponding git
  dependency`. Reproduced on unmodified `313503c` in a detached worktree, so it
  predates this branch. It does mean the existing nix path could not be used as
  a regression check here.
- **No full `cargo build --workspace` was run** on this branch. `cargo metadata`
  resolves and `hyperion-platform` builds and lints clean, but the claim "the
  normal build is unchanged" rests on the diff — no existing crate's source was
  touched — rather than on a measurement.
- **Linux was not tested.** Everything was built and booted on `aarch64-darwin`
  cross-compiling to `x86_64-unknown-hermit`. QEMU ran without KVM, on
  `-cpu Skylake-Client`.
- **Only `x86_64-unknown-hermit`.** The aarch64 and riscv64 Hermit triples were
  never attempted.
- **The blocker table's second half is unmeasured.** `heed`, `memmap2`,
  `libdeflater`, `ndarray+blas`, `tracing-tracy` and `valence_anvil` are
  reasoned from their manifests. No build reached them.
- **No login, no gameplay, no second connection.** The demo serves the status
  handshake and nothing else, one connection at a time. Nothing was measured
  under load, and no packet larger than a status response has crossed the
  virtio-net link.
- **Compression is off** in the demo, and the open-coded framing in
  `bare-metal/hyperion-unikernel/src/main.rs` is only correct because of that.
- **The Hermit forks were not exercised.** `hermit-os/socket2` and
  `hermit-os/tokio` are named as the route to the proxy on the strength of
  reading hermit-rs's `[patch.crates-io]` and its example set. Neither was
  built here.
- **Reproducibility of the vendor hash is untested** across cargo versions. It
  is a fixed-output derivation over `cargo vendor` output, and a toolchain bump
  will change it.
