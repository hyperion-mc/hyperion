# Building a Tier-3 Rust target with a custom sysroot is exactly the kind of
# thing that works on the machine it was invented on and nowhere else, which is
# why it lives here rather than in a shell script.
#
# Three cargo invocations happen inside one build:
#
#   1. the application, from bare-metal/Cargo.lock;
#   2. the standard library, because -Z build-std compiles std from source and
#      resolves its own lockfile out of rust-src;
#   3. the Hermit kernel, which the `hermit` crate's build script builds by
#      shelling out to a nested cargo in the kernel's source tree.
#
# All three read one vendor directory, produced by a single `cargo vendor
# --sync`, because the third one strips every CARGO_* and RUST_* variable from
# its environment before running and can only be reached through
# $HOME/.cargo/config.toml.
{
  cacert,
  fetchFromGitHub,
  fetchurl,
  git,
  lib,
  qemu,
  rustToolchain,
  stdenvNoCC,
  writeShellApplication,
  writeShellScriptBin,
}:
let
  target = "x86_64-unknown-hermit";

  # Pinned to what the `hermit` crate at tag hermit-0.13.2 carries as its kernel
  # submodule. Cargo's vendoring does not follow submodules, so the kernel is
  # fetched separately and handed to the build script with HERMIT_MANIFEST_DIR.
  kernelSrc = fetchFromGitHub {
    owner = "hermit-os";
    repo = "kernel";
    rev = "f51061476ecaa2066779c473a683d8b35b315d9b";
    hash = "sha256-k6vf89oW/69exw8qblFu6v5n/+DK/t5LC+MaMijF+Es=";
  };

  # The loader is what QEMU boots as -kernel; it unpacks the image and jumps
  # into it. Taken prebuilt because building it means a fourth toolchain.
  loader = fetchurl {
    url = "https://github.com/hermit-os/loader/releases/download/v0.5.6/hermit-loader-x86_64";
    hash = "sha256-GF9+yEOhhISqchguynf/PgcMBBqOo18rG0FTCNZz1Uk=";
  };

  rustSrc = "${rustToolchain}/lib/rustlib/src/rust/library";

  # The kernel's xtask runs `rustup target add x86_64-unknown-none` before
  # building. There is no rustup here and the toolchain already carries that
  # target, so answer yes and get out of the way. Anything else rustup is asked
  # to do is a real gap, so fail loudly rather than silently succeeding.
  rustupShim = writeShellScriptBin "rustup" ''
    if [ "$1" = "target" ] && [ "$2" = "add" ]; then
      exit 0
    fi
    echo "nix/bare-metal.nix: unhandled rustup invocation: $*" >&2
    exit 1
  '';

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../bare-metal
      ../crates/hyperion-platform
    ];
  };

  # One vendor directory for all three cargo runs. `--sync` is the only way to
  # get a single consistent set: vendoring each manifest separately produces
  # three `config.toml` fragments that have to be merged by hand, and any
  # mistake in the merge shows up as a network fetch in a sandbox.
  vendor = stdenvNoCC.mkDerivation {
    name = "hyperion-bare-metal-vendor";
    inherit src;

    # A fixed-output derivation has network access but nothing else from the
    # host, so the certificate bundle and git have to be named explicitly.
    nativeBuildInputs = [
      cacert
      git
      rustToolchain
    ];

    buildPhase = ''
      runHook preBuild
      export HOME="$TMPDIR/home"
      export SSL_CERT_FILE="${cacert}/etc/ssl/certs/ca-bundle.crt"
      # cargo's bundled libgit2 cannot complete a TLS handshake in the sandbox;
      # the git binary can.
      export CARGO_NET_GIT_FETCH_WITH_CLI=true
      mkdir -p "$out"
      cargo vendor --locked \
        --manifest-path bare-metal/Cargo.toml \
        --sync ${rustSrc}/Cargo.toml \
        --sync ${kernelSrc}/Cargo.toml \
        --sync ${kernelSrc}/hermit-builtins/Cargo.toml \
        --sync ${kernelSrc}/hermit-macro/Cargo.toml \
        "$out/vendor" > "$out/config.toml"
      # A fixed-output derivation may not reference store paths, and cargo writes
      # the absolute vendor directory into the config it emits. Leave a
      # placeholder for the consumer to fill in.
      substituteInPlace "$out/config.toml" --replace-fail "$out/vendor" "@vendor@"
      runHook postBuild
    '';

    dontInstall = true;
    # patchShebangs would rewrite vendored CI scripts to point at a bash in the
    # store, which both adds a reference and invalidates cargo's checksums.
    dontFixup = true;

    outputHashAlgo = "sha256";
    outputHashMode = "recursive";
    outputHash = "sha256-bCu5zrjxq78+s5dkFiYMJynosdTKHe1bKf5uWWbTzNk=";
  };

  image = stdenvNoCC.mkDerivation {
    pname = "hyperion-unikernel";
    version = "0.1.0";
    inherit src;

    nativeBuildInputs = [
      rustToolchain
      rustupShim
    ];

    # The repo's .cargo/config.toml adds -Ctarget-cpu=native to every build,
    # which makes any cross-compile emit host-CPU instructions. RUSTFLAGS wins
    # over it, so set it here rather than editing a file every other build
    # reads.
    RUSTFLAGS = "";

    HERMIT_MANIFEST_DIR = kernelSrc;

    buildPhase = ''
      runHook preBuild

      export HOME="$TMPDIR/home"
      export CARGO_HOME="$HOME/.cargo"
      mkdir -p "$CARGO_HOME"
      substitute ${vendor}/config.toml "$CARGO_HOME/config.toml" \
        --replace-fail '@vendor@' '${vendor}/vendor'

      cargo build \
        --offline \
        --manifest-path bare-metal/Cargo.toml \
        --release \
        --target ${target} \
        -Z build-std=std,panic_abort

      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall
      mkdir -p "$out/share/hyperion"
      cp bare-metal/target/${target}/release/hyperion-unikernel "$out/share/hyperion/image"
      cp ${loader} "$out/share/hyperion/loader"
      runHook postInstall
    '';

    passthru = { inherit kernelSrc loader vendor; };

    meta = {
      description = "hyperion's protocol layer as a Hermit unikernel image";
      platforms = lib.platforms.all;
    };
  };

  # Host port differs from the guest's on purpose: 25565 is usually already
  # taken by a real server on a developer's machine, and forwarding onto it
  # would silently talk to that instead of the VM.
  vm = writeShellApplication {
    name = "hyperion-bare-metal-vm";
    runtimeInputs = [ qemu ];
    text = ''
      host_port="''${HYPERION_HOST_PORT:-25599}"
      echo "booting hyperion unikernel; ping 127.0.0.1:$host_port as a Minecraft server" >&2
      exec qemu-system-x86_64 \
        -display none -serial stdio \
        -kernel ${image}/share/hyperion/loader \
        -initrd ${image}/share/hyperion/image \
        -smp "''${HYPERION_SMP:-2}" -m "''${HYPERION_MEM:-1024M}" \
        -cpu Skylake-Client \
        -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
        -netdev "user,id=net0,hostfwd=tcp::$host_port-:25565,net=192.168.76.0/24,dhcpstart=192.168.76.9" \
        -device virtio-net-pci,netdev=net0,disable-legacy=on,packed=on,mq=on \
        "$@"
    '';
  };
in
{
  inherit image vm;
}
