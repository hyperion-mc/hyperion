# Packaging for hot reload: three derivations, split by source, so that a
# rules-only change moves exactly one store path.
#
# The whole feature rests on one property. `nix/modules/game-server.nix` puts
# the rules dylib in `X-Reload-Triggers` and the server binary in `ExecStart`,
# and nixpkgs' unit handling reloads a unit whose `[Service]` section is
# unchanged and whose reload triggers differ. So a rules edit may move the rules
# dylib and must not move the server binary. If it moves both, the deploy
# degrades to a restart, every player is dropped, and every gate stays green --
# which is why `checks.hot-reload-source-split` asserts the split instead of
# trusting this file to be right.
#
# A store path is a function of a derivation's inputs, so "which paths move" is
# decided entirely by which sources reach which derivation:
#
#   hyperion-dylibs  crates/*                          an engine change
#   smash-server     crates/* + events/smash           a component or engine change
#   smash-rules      crates/* + events/smash + rules   a rules change
#
# Nobody has to remember the rule. A component's layout lives in the host crate
# and a system's body lives in the rules crate, so the source split is the rule.
#
# See docs/hot-reload.md, "Packaging: three derivations, because two would
# restart", for the measurements behind this.
{ lib, pkgs, workspace, rustToolchain, nativeBuildInputs, root }:
let
  # Read from the manifest rather than restated here, so a new member cannot be
  # silently omitted from the stub list and fail one derivation much later.
  members = (lib.importTOML (root + "/Cargo.toml")).workspace.members;
  crateMembers = lib.filter (member: lib.hasPrefix "crates/" member) members;

  rootFiles = map (file: root + "/${file}") [
    "Cargo.toml"
    "Cargo.lock"
    "rust-toolchain.toml"
    "clippy.toml"
    "rustfmt.toml"
  ];

  # A source tree holding the real code of `realMembers` and a stub for every
  # other workspace member.
  #
  # The stub is not tidiness. Cargo parses every member of a workspace before it
  # builds any of it, and a member with no target file fails the whole resolve:
  #
  #     error: failed to load manifest for workspace member `events/bedwars`
  #     Caused by: no targets specified in the manifest
  #       either src/lib.rs, src/main.rs, a [lib] section, or [[bin]] section
  #       must be present
  #
  # So a derivation cannot simply omit a member's directory. An empty
  # `src/lib.rs` is the smallest thing that lets cargo parse a member it will
  # never build, and its content is constant, which is what keeps the omitted
  # member's real source out of this derivation's inputs.
  #
  # The filtering happens at eval time through `lib.fileset`, whose store path
  # is a function of the included files alone. Filtering inside the builder
  # instead would make the whole tree an input and defeat the entire split.
  # `realDirs` is separate from `realMembers` because not every path
  # dependency is a workspace member: `crates/hyperion-clap-macros` is depended
  # on by `crates/hyperion-clap` and appears in no `members` list, so a fileset
  # built from members alone omits it and cargo fails the whole resolve with
  # `failed to read crates/hyperion-clap-macros/Cargo.toml`. Taking `crates` as
  # a directory covers the members and their private neighbours in one rule.
  mkSource =
    { pname, realDirs, realMembers }:
    let
      filtered = lib.fileset.toSource {
        inherit root;
        fileset = lib.fileset.unions (
          rootFiles
          ++ map (member: root + "/${member}/Cargo.toml") members
          ++ map (dir: root + "/${dir}") realDirs
        );
      };
      stubbed = lib.subtractLists realMembers members;
    in
    pkgs.runCommand "hyperion-source-${pname}" { } ''
      cp -r ${filtered} "$out"
      chmod -R u+w "$out"
      for member in ${lib.escapeShellArgs stubbed}; do
        mkdir -p "$out/$member/src"
        : > "$out/$member/src/lib.rs"
      done
    '';

  dylibSource = mkSource {
    pname = "dylibs";
    realDirs = [ "crates" ];
    realMembers = crateMembers;
  };
  serverSource = mkSource {
    pname = "server";
    realDirs = [ "crates" "events/smash" ];
    realMembers = crateMembers ++ [ "events/smash" ];
  };
  rulesSource = mkSource {
    pname = "rules";
    realDirs = [ "crates" "events/smash" "events/smash-rules" ];
    realMembers = crateMembers ++ [ "events/smash" "events/smash-rules" ];
  };

  profile = "release";
  sysrootLib = "${rustToolchain}/lib/rustlib/${pkgs.stdenv.hostPlatform.rust.rustcTarget}/lib";
  inherit (pkgs.stdenv.hostPlatform) extensions;

  # The link recipe `crates/hyperion-hot-reload/index-probe.sh` proves, with
  # store paths where that script has `target/debug`.
  #
  # `-C prefer-dynamic` is the load-bearing flag: it makes the server binary and
  # the rules dylib resolve `hyperion`, and through it the one `flecs_ecs` that
  # owns the component-index pool, to a shared image rather than each linking
  # its own static copy. `checks.hot-reload-index-probe` is what proves that
  # holds; this is only the flags.
  rustFlags =
    libDirs:
    lib.concatStringsSep " " (
      [ "--cfg tokio_unstable" "-C prefer-dynamic" ]
      ++ map (dir: "-C link-arg=-Wl,-rpath,${dir}") libDirs
      # `flecs_ecs`'s build script installs a version script naming four globs,
      # and ld treats a pattern that matches nothing as an error by default.
      ++ lib.optional pkgs.stdenv.hostPlatform.isElf "-C link-arg=-Wl,--undefined-version"
    );

  common = {
    # The toolchain explicitly: flake.nix's `nativeBuildInputs` is the C and
    # build-tool half and does not carry cargo. Every other cargo build in this
    # flake reaches it through a `writeShellApplication`'s `runtimeInputs`,
    # which these derivations do not go through.
    nativeBuildInputs = nativeBuildInputs ++ [ rustToolchain ];
    # Every C build script in the graph answers `_FORTIFY_SOURCE` with a
    # `#warning` at -O0, and an autoconf probe reading stderr then misreads its
    # own test as a compile failure. `checks.clippy` met this first.
    hardeningDisable = [ "fortify" ];
  };

  setup = source: ''
    cp -r ${source}/. .
    chmod -R u+w .
    ${workspace.cargoConfigScript}
  '';

  # Reuse the engine's artifacts rather than compiling equivalents of them.
  #
  # This is not a build-time optimisation, it is what makes the boundary sound.
  # If the server and the rules each compiled their own `hyperion`, each would
  # get its own `-C metadata` hash -- the source ids differ, because the two
  # source trees are different store paths -- and the mangled symbol names would
  # not match across the `dlopen` boundary. Reusing one target directory means
  # both link artifacts that are the same bytes, so "one copy of flecs in the
  # process" is a fact about the build graph rather than a coincidence between
  # two builds that happen to agree.
  #
  # Artifact mtimes are moved ahead of the sources', because cargo decides
  # freshness by comparing them and everything unpacked from the store shares
  # one normalised timestamp.
  seed = ''
    tar -C . -xf ${hyperion-dylibs}/share/cargo-target.tar
    find target -exec touch -d "2100-01-01" {} +
  '';

  # The engine, built once, as the dylibs everything else resolves at runtime.
  hyperion-dylibs = pkgs.runCommandCC "hyperion-dylibs" common ''
    ${setup dylibSource}
    export RUSTFLAGS=${lib.escapeShellArg (rustFlags [ sysrootLib "$ORIGIN" ])}
    cargo build --profile ${profile} --offline -p hyperion

    mkdir -p "$out/lib" "$out/share"
    # `deps/` as well as the profile directory: cargo leaves an unhashed copy of
    # a workspace member's dylib in `target/${profile}`, but a dependency's
    # dylib only in `deps/`, under the metadata hash DT_NEEDED actually names.
    cp target/${profile}/*${extensions.sharedLibrary} "$out/lib/"
    cp target/${profile}/deps/libflecs_ecs-*${extensions.sharedLibrary} "$out/lib/"
    tar -C target -cf "$out/share/cargo-target.tar" ${profile}
  '';

  # What `ExecStart` names. Moves on a component or engine change, which is
  # exactly when a restart is correct: a component's layout is defined here, and
  # a system compiled against a layout the world no longer holds is memory
  # corruption rather than a stale build.
  smash-server = pkgs.runCommandCC "smash-server" (common // { meta.mainProgram = "smash"; }) ''
    ${setup serverSource}
    ${seed}
    export RUSTFLAGS=${lib.escapeShellArg (rustFlags [ sysrootLib "${hyperion-dylibs}/lib" ])}
    cargo build --profile ${profile} --offline -p smash

    mkdir -p "$out/bin"
    cp target/${profile}/smash "$out/bin/smash"
  '';

  # What `X-Reload-Triggers` names, and the only path a rules-only change is
  # allowed to move.
  smash-rules = pkgs.runCommandCC "smash-rules" common ''
    ${setup rulesSource}
    ${seed}
    export RUSTFLAGS=${lib.escapeShellArg (rustFlags [ sysrootLib "${hyperion-dylibs}/lib" ])}
    cargo build --profile ${profile} --offline -p smash-rules

    mkdir -p "$out/lib"
    cp target/${profile}/libsmash_rules${extensions.sharedLibrary} "$out/lib/"
  '';
in
{
  inherit
    hyperion-dylibs
    smash-server
    smash-rules
    dylibSource
    serverSource
    rulesSource
    ;
}
