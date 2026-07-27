#!/usr/bin/env bash
# Builds four successive versions of the demo game module and drives one running world
# through all of them, printing what the reload gate decided each time.
#
# Cargo features stand in for editing the source between builds. Each build is copied to
# its own path: dlopen caches by path, so reusing one filename gets the first mapping back.
set -euo pipefail

cd "$(dirname "$0")/../.."
root=$(pwd)
out=${HOT_RELOAD_DEMO_OUT:-$root/target/hot-reload-demo}
mkdir -p "$out"

case "$(uname -s)" in
  Darwin) ext=dylib; host_triple=$(rustc -vV | sed -n 's/^host: //p') ;;
  *)      ext=so;     host_triple=$(rustc -vV | sed -n 's/^host: //p') ;;
esac
sysroot_lib="$(rustc --print sysroot)/lib/rustlib/$host_triple/lib"

# Host and every module must resolve `hyperion-hot-reload` -- and through it `flecs_ecs`
# and flecs's C globals -- to one shared dylib. Without prefer-dynamic each artifact links
# its own static copy, which the ABI token then refuses at load time.
export RUSTFLAGS="--cfg tokio_unstable -Ctarget-cpu=native -C prefer-dynamic \
  -C link-arg=-Wl,-rpath,$sysroot_lib -C link-arg=-Wl,-rpath,$root/target/debug"

build() {
  local label=$1; shift
  echo "--- building $label ---" >&2
  cargo build -q -p hyperion-hot-reload-demo-module "$@"
  cp "target/debug/libhyperion_hot_reload_demo_module.$ext" "$out/$label.$ext"
}

cargo build -q -p hyperion-hot-reload-demo
build v1-baseline
build v2-code-only         --features tuned
build v3-layout-change     --features health-f32
build v4-with-migration    --features health-f32,migration

exec ./target/debug/hot-reload-demo \
  "$out/v1-baseline.$ext" \
  "$out/v2-code-only.$ext" \
  "$out/v3-layout-change.$ext" \
  "$out/v4-with-migration.$ext"
