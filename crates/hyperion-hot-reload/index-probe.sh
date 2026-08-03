#!/usr/bin/env bash
# Answers one question: do a host binary and a dlopened module dylib draw component
# indices from one shared pool?
#
# `flecs_ecs`'s derive emits a `static INDEX` per component type, initialised from a
# process-global `INDEX_POOL`, and that index is a slot in the world's component array.
# Two copies of `flecs_ecs` in one process is two pools, so a module writes into a slot
# the host never filled -- consistently on each side, with nothing raising an error.
# Nothing in the hot-reload gate detects this; see `docs/hot-reload.md`.
#
# The probe binary reports what it measured and asserts on it, so this script's job is
# only to build both halves with the recipe that makes the pool shared and then run it.
set -euo pipefail

cd "$(dirname "$0")/../.."
root=$(pwd)
target=${CARGO_TARGET_DIR:-$root/target}

case "$(uname -s)" in
  Darwin) ext=dylib ;;
  *)      ext=so ;;
esac
host_triple=$(rustc -vV | sed -n 's/^host: //p')
sysroot_lib="$(rustc --print sysroot)/lib/rustlib/$host_triple/lib"

# `-C prefer-dynamic` is what makes both halves resolve `hyperion` -- and through it the
# one `flecs_ecs` that owns the pool -- to a shared image rather than each linking its
# own static copy. The two rpaths are what lets the result start: with prefer-dynamic,
# libstd is a dylib in the sysroot and rustc adds no rpath for it, and the workspace's
# own dylibs live beside the binary.
#
# `--undefined-version` is for the version script `flecs_ecs`'s build.rs installs. It
# names four globs (`ecs_*`, `flecs_*`, `Ecs*`, `FLECS_*`) and lld/bfd treat a pattern
# that matches nothing as an error by default.
#
# There is deliberately no `--allow-shlib-undefined` here. It used to be required
# because `simulation/metadata/mod.rs` handed every metadata component a blanket
# `impl PartialOrd where $type: PartialOrd`, unsatisfiable for the seven whose inner
# type is a glam `Quat` or `Vec3`; rustc never codegened those `partial_cmp` bodies and
# still listed them in the dylib's export table. That impl is gone, so the flag is too.
# If it comes back, so does the blanket impl.
flags="--cfg tokio_unstable -C prefer-dynamic"
flags="$flags -C link-arg=-Wl,-rpath,$sysroot_lib -C link-arg=-Wl,-rpath,$target/debug"
# `deps/` as well as `debug/`, because the two dylibs are named differently
# there. Cargo gives a workspace member's dylib an unhashed copy in
# `target/debug` (`libhyperion.so`), but a dependency's dylib exists only in
# `target/debug/deps` under its metadata hash -- and the DT_NEEDED entry names
# the hashed file. So `libhyperion.so` resolved and `libflecs_ecs-<hash>.so`
# did not, which is the one library the whole probe is about.
flags="$flags -C link-arg=-Wl,-rpath,$target/debug/deps"
if [ "$ext" = so ]; then
  flags="$flags -C link-arg=-Wl,--undefined-version"
fi
export RUSTFLAGS="$flags"

cargo build -q \
  -p hyperion-hot-reload-index-probe \
  -p hyperion-hot-reload-index-probe-module

exec "$target/debug/hot-reload-index-probe" \
  "$target/debug/libhyperion_hot_reload_index_probe_module.$ext"
