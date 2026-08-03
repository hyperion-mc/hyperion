# Packaging for hot reload: one cargoUnit graph, three sets of store paths that
# move independently.
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
#   hyperion-dylibs   moves on an engine change
#   <event>-server    moves on a component or engine change
#   <event>-rules     moves on a rules, component or engine change
#
# Nobody has to remember the rule. A component's layout lives in the host crate
# and a system's body lives in the rules crate, so the source split is the rule.
#
# ## Why this is cargoUnit and not cargo (ENG-12078)
#
# It used to be three `runCommandCC` derivations, each running its own
# `cargo build` over a hand-filtered source tree in which every crate the
# derivation did not want was replaced by an empty `lib.rs`. That is how you get
# per-derivation source scoping out of a build system that has none, and it cost
# a whole class of hazard to operate:
#
#   - Cargo resolves features over the packages named on the command line, so
#     the three invocations had to pass one identical `-p` selection string or
#     `flecs_ecs` got a different `-C metadata` in each -- two `libflecs_ecs`
#     in one process, which is two component-index pools indexing one world two
#     different ways (ENG-12053).
#   - RUSTFLAGS folds into every unit's `-C metadata` too, so the rpaths had to
#     be applied with `patchelf` after linking, because putting a store path in
#     RUSTFLAGS recompiled the graph under different mangled symbol names.
#   - The stub trees had to be cleaned out of the shared `target/` seed by hand,
#     or a stub `libsmash_rules.so` shipped beside the engine.
#
# cargoUnit deletes all three. It is one derivation per rustc invocation with
# per-crate source scoping, so "which paths move" is a fact about the unit graph
# rather than something this file arranges. Features are resolved once for the
# whole workspace, so there is exactly one `flecs_ecs` unit by construction --
# `engineDylib` asserts that rather than assuming it. And `-C metadata` comes
# from cargoUnit's own graph identity hash rather than from the flags it passes,
# so an rpath cannot perturb a symbol name.
#
# `smash` is the first consumer, not the shape (ENG-12067). One engine dylib set
# is shared by every event; each event named in `events` gets its own server and
# rules pair.
#
# See docs/hot-reload.md, "Packaging: three derivations, because two would
# restart", for the measurements behind this.
{
  lib,
  pkgs,
  cargoUnit,
  # The same `src`/lock/vendor arguments the ordinary release workspace is built
  # from, so the two read one definition and cannot drift.
  workspaceArgs,
  rustToolchain,
  root,
  # Each entry is one game: `{ name; hostCrate; rulesCrate; }`, where the two
  # crates are workspace-member paths. `name` is what the NixOS module keys on
  # (`/etc/hyperion/<name>-rules.so`) and what the derivations are called.
  events,
}:
let
  # Package name, not directory basename: the two are only equal by convention.
  packageNameOf = member: (lib.importTOML (root + "/${member}/Cargo.toml")).package.name;
  # cargo spells a library after the crate name, which is the package name with
  # dashes folded to underscores.
  libNameOf = package: lib.replaceStrings [ "-" ] [ "_" ] package;

  sysrootLib = "${rustToolchain}/lib/rustlib/${pkgs.stdenv.hostPlatform.rust.rustcTarget}/lib";
  inherit (pkgs.stdenv.hostPlatform) extensions;

  # What `ExecReload` runs. Named once so the NixOS module and this file cannot
  # disagree about it.
  reloadClient = "hyperion-reload-client";

  # The engine crates whose dylib every event resolves at run time.
  #
  # `hyperion` and `hyperion-hot-reload` declare `crate-type = ["rlib",
  # "dylib"]` themselves; `flecs_ecs` is the fork pinned in Cargo.toml, patched
  # for the same reason. It is named here rather than discovered because being
  # in this list is a decision -- a crate joins it by getting a dylib crate type,
  # and that is a deliberate act with a comment in its Cargo.toml.
  enginePackages = [
    "hyperion"
    "hyperion-hot-reload"
    "flecs_ecs"
  ];
  # `hyperion-reload-client` is deliberately NOT in that list, though the
  # cargo-based packaging did put it there. Its reason was the selection string:
  # every package had to be named in one `cargo build` or `flecs_ecs` got a
  # different `-C metadata` per invocation. There is one invocation now, so that
  # reason is gone, and the list means what it says -- crates whose *dylib* an
  # event resolves at run time. The reload client is a dependency-free binary
  # with no dylib, so `engineUnit` would find nothing to gather. It ships from
  # the same output for the reason below.

  # The build every hot-reload artifact comes out of. Separate from the ordinary
  # release workspace on purpose: `-C prefer-dynamic` changes how every artifact
  # links, and hyperion's other packages are static binaries that should stay
  # that way.
  #
  # `-C prefer-dynamic` is the load-bearing flag and cargoUnit cannot infer it.
  # Generating a dylib without it makes rustc statically absorb every dependency
  # into that dylib, and linking an executable without it makes rustc prefer the
  # rlib of a crate offering both -- either way the server binary and the rules
  # dylib each get their own `flecs_ecs`, and the one thing hot reload cannot
  # tolerate is two component-index pools in one process.
  # `checks.hot-reload-index-probe` is what proves it holds; this is the flag.
  #
  # The sysroot rpath is its consequence: prefer-dynamic makes libstd dynamic
  # too, so a linked artifact asks the loader for `libstd-<hash>.so` and finds
  # nothing without this. cargoUnit adds the rpaths for the dylibs inside the
  # graph, whose store paths only it knows.
  #
  # Unlike the RUSTFLAGS string this replaces, neither flag reaches `-C
  # metadata`: cargoUnit derives that from the unit graph, not from the rustc
  # arguments it passes. An rpath here cannot rename a symbol.
  workspaceFor =
    src:
    cargoUnit.buildWorkspace (
      workspaceArgs
      // {
        inherit src;
        workspaceRoot = src;
        pname = "hyperion-hot-reload";
        cargoArgs = [ "--workspace" ];
        extraRustcArgs = [ "-Cprefer-dynamic" ];
        extraLinkRustcArgsForPlatform = _platform: [ "-Clink-arg=-Wl,-rpath,${sysrootLib}" ];
      }
    );

  # A unit key is `<target name>-<version>-<16 hex>`. Matching that shape rather
  # than a bare prefix keeps `flecs_ecs-build-script-run-...` out of the result.
  unitsNamed =
    workspace: libName:
    lib.filter (
      name: builtins.match "${lib.escapeRegex libName}-[0-9][^-]*-[0-9a-f]{16}" name != null
    ) (lib.attrNames workspace.units);

  # The single unit that produces one engine crate's shared library.
  #
  # The `== 1` is the guard, not a defensive length check. Two units for one
  # crate is precisely the ENG-12053 failure -- two `libflecs_ecs`, two
  # `INDEX_POOL`s, a reload that indexes one world two different ways -- and
  # under one feature resolution for the whole workspace it cannot happen. If it
  # ever does, this stops the build and names the crate instead of shipping a
  # server that corrupts on reload.
  engineUnit =
    workspace: package:
    let
      libName = libNameOf package;
      matches = unitsNamed workspace libName;
    in
    if lib.length matches == 1 then
      workspace.units.${lib.head matches}
    else
      throw ''
        hot-reload packaging: expected exactly one `${libName}` unit in the graph, found ${toString (lib.length matches)}:
          ${lib.concatStringsSep "\n  " matches}
        More than one means the workspace resolved that crate two ways, which is two
        copies of its process-global state in one server process (ENG-12053). None
        means the crate is not in the graph under that name.
      '';

  # Every `DT_NEEDED` has to resolve, and the build is the place to find out.
  #
  # This is the guard that would have caught ENG-12053 the day it was written
  # instead of at `ldd` time on a dev box: an artifact asking for a
  # `libflecs_ecs` nobody ships is a dangling reference, and a dangling
  # reference to THAT library specifically means two component-index pools.
  # Written to fail closed. The obvious spelling -- `if ldd f | grep "not
  # found"` -- passes when `ldd` is absent or errors, because then grep matches
  # nothing and the guard's success and its own breakage are the same signal.
  # So: require ldd to succeed, require it to have said something, and only then
  # look for the failure string.
  requireResolved =
    file:
    lib.optionalString pkgs.stdenv.hostPlatform.isElf ''
      if ! ldd ${file} > needed.txt 2> ldd-err.txt; then
        echo "ldd could not read ${file}; this guard cannot run:" >&2
        cat ldd-err.txt >&2
        exit 1
      fi
      if [ ! -s needed.txt ]; then
        echo "ldd reported no libraries at all for ${file}." >&2
        echo "A prefer-dynamic artifact always names at least libstd, so this is the" >&2
        echo "guard breaking rather than the artifact being clean." >&2
        exit 1
      fi
      if grep "not found" needed.txt; then
        echo "unresolved libraries in ${file}; see ENG-12053" >&2
        exit 1
      fi
    '';

  # Symlinks rather than copies, so the bytes of a shared library exist at
  # exactly one store path however many places name it. "One libflecs_ecs" is
  # then a fact about the store and not a claim about this file.
  packagingFor =
    src:
    let
      workspace = workspaceFor src;

      hyperion-dylibs =
        # `runCommandCC`, not `runCommand`: `requireResolved` below needs `ldd`,
        # which comes with the C toolchain. The fail-closed rewrite of that guard
        # is what turned this into a build failure naming `ldd: command not
        # found` rather than a silent pass -- `if ldd f | grep "not found"`
        # succeeds when `ldd` is missing, because then grep matches nothing.
        pkgs.runCommandCC "hyperion-dylibs"
          {
            # No patchelf. The old packaging rewrote rpaths after linking,
            # because a store path in RUSTFLAGS changed every unit's
            # `-C metadata`; cargoUnit derives that from the unit graph instead,
            # so the rpaths are ordinary link args and nothing is rewritten.
            passthru.units = lib.genAttrs enginePackages (engineUnit workspace);
          }
          ''
            mkdir -p "$out/bin" "$out/lib"

            # The reload client ships from here, with the engine, and not from
            # an event's derivation. `ExecReload` names it, and `[Service]`
            # moving is exactly what turns a reload into a restart -- so the
            # path in it must not be a function of any event's source. This
            # output moves only on an engine change, which moves `ExecStart` too
            # and is a restart anyway.
            ln -s ${
              workspace.binaries.${reloadClient} or (throw ''
                hot-reload packaging: the workspace has no binary `${reloadClient}`.
                Got: ${lib.concatStringsSep ", " (lib.attrNames workspace.binaries)}
              '')
            }/bin/${reloadClient} "$out/bin/${reloadClient}"
            ${requireResolved ''"$out/bin/${reloadClient}"''}

            for unit in ${lib.escapeShellArgs (map (package: engineUnit workspace package) enginePackages)}; do
              for shared in "$unit"/lib/*${extensions.sharedLibrary}; do
                [ -e "$shared" ] || continue
                ln -s "$shared" "$out/lib/"
              done
            done
            # An engine crate that stopped emitting a shared library would leave
            # this empty and every event would silently link its own static copy.
            if [ -z "$(ls -A "$out/lib")" ]; then
              echo "no engine shared libraries found; is prefer-dynamic still set?" >&2
              exit 1
            fi
          '';

      forEvent =
        event:
        let
          hostPackage = packageNameOf event.hostCrate;
          rulesPackage = packageNameOf event.rulesCrate;
          rulesLib = "lib${libNameOf rulesPackage}${extensions.sharedLibrary}";

          rulesUnit =
            workspace.libraries.${libNameOf rulesPackage} or (throw ''
              hot-reload packaging: the workspace has no library target `${libNameOf rulesPackage}`.
              Got: ${lib.concatStringsSep ", " (lib.attrNames workspace.libraries)}
            '');

          # What `ExecStart` names. Moves on a component or engine change, which
          # is exactly when a restart is correct: a component's layout is defined
          # in the host crate, and a system compiled against a layout the world no
          # longer holds is memory corruption rather than a stale build.
          server =
            pkgs.runCommandCC "${event.name}-server"
              {
                meta.mainProgram = hostPackage;
                passthru.unit = workspace.binaries.${hostPackage};
              }
              ''
                mkdir -p "$out/bin"
                ln -s ${workspace.binaries.${hostPackage}}/bin/${hostPackage} "$out/bin/${hostPackage}"
                ${requireResolved ''"$out/bin/${hostPackage}"''}
              '';

          # What `X-Reload-Triggers` names, and the only path a rules-only change
          # is allowed to move. Renamed to the unhashed spelling the NixOS module
          # installs as `/etc/hyperion/<event>-rules.so`; cargoUnit's own filename
          # carries the unit hash, which would put a build identity in a path an
          # operator reads.
          rules =
            pkgs.runCommandCC "${event.name}-rules"
              {
                # passthru rather than a convention a consumer has to know:
                # cargo spells the file after the crate, and `nix/fleet` would
                # otherwise be a second place that has to agree with cargo
                # about it.
                passthru = {
                  dylibName = rulesLib;
                  unit = rulesUnit;
                };
              }
              ''
                mkdir -p "$out/lib"
                shared=$(echo ${rulesUnit}/lib/lib${libNameOf rulesPackage}-*${extensions.sharedLibrary})
                if [ ! -f "$shared" ]; then
                  echo "the ${rulesPackage} unit produced no shared library:" >&2
                  ls -la ${rulesUnit}/lib >&2
                  echo "A rules crate must be crate-type = [\"dylib\"]." >&2
                  exit 1
                fi
                ln -s "$shared" "$out/lib/${rulesLib}"
                ${requireResolved ''"$out/lib/${rulesLib}"''}
              '';
        in
        {
          inherit server rules rulesLib;
        };
      # The shared-pool probe, built from the units the server actually links
      # rather than from a second compile with hand-written RUSTFLAGS.
      #
      # It used to be `crates/hyperion-hot-reload/index-probe.sh`: a dev-profile
      # `cargo build` with its own prefer-dynamic string and three rpaths into
      # `target/`. That proved the property for a build nothing shipped. These
      # are the same derivations the server and the rules dylib resolve, so a
      # PROBE_OK here is a statement about the artifacts that go to a host.
      indexProbe = {
        host =
          workspace.binaries.hot-reload-index-probe or (throw ''
            hot-reload packaging: the workspace has no binary `hot-reload-index-probe`.
            Got: ${lib.concatStringsSep ", " (lib.attrNames workspace.binaries)}
          '');
        module =
          workspace.libraries.hyperion_hot_reload_index_probe_module or (throw ''
            hot-reload packaging: the workspace has no library `hyperion_hot_reload_index_probe_module`.
            Got: ${lib.concatStringsSep ", " (lib.attrNames workspace.libraries)}
          '');
      };
    in
    {
      inherit
        hyperion-dylibs
        indexProbe
        reloadClient
        workspace
        ;
      # Keyed by event name so a consumer names the game rather than a path.
      events = lib.listToAttrs (map (event: lib.nameValuePair event.name (forEvent event)) events);
    };
in
packagingFor workspaceArgs.src
// {
  # The source split, as a function of an arbitrary source tree, so a check can
  # instantiate this packaging over a perturbed one and compare. See
  # `checks.hot-reload-source-split`.
  inherit packagingFor;
}
