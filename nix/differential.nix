# Records vanilla's answer for every committed differential scenario.
#
# Three outputs, in the pattern the rest of `nix/minecraft-data.nix` already
# uses for generated data:
#
#   recordedTraces     the derivation that runs the real server
#   syncScript         `nix run .#record-differential-traces`, which copies the
#                      recording over the committed copy
#   tracesUpToDate     a `nix flake check` gate that fails when the two differ,
#                      so a jar bump cannot leave the golden data behind
#
# The committed traces are what `cargo test` reads, so the everyday loop needs
# no Java, no network and no server. `docs/differential-testing.md` explains
# the scenario format and what the comparison does and does not prove.
{ pkgs, jdk, serverClasspath, pin }:

let
  inherit (pkgs) lib;

  scenarios = ../crates/hyperion/tests/differential/scenarios;
  committed = ../crates/hyperion/tests/differential/traces;

  # Seeds the recording is replayed under. The first is the one written into
  # the scenario files; the rest exist to make the reproducibility claim
  # checkable rather than asserted. Vanilla derives the level's random source
  # from the world seed, so a scenario that reaches it produces a different
  # trace here and fails the build. See the note in VanillaTrace.java.
  seeds = [ "4242" "1" "8675309" ];

  recorder = pkgs.runCommand "minecraft-trace-recorder-${pin.id}"
    {
      nativeBuildInputs = [ jdk pkgs.makeWrapper ];
      meta = {
        description = "Headless Minecraft ${pin.id} server that records entity state per tick";
        license = lib.licenses.unfree; # Mojang EULA; runs the server jar.
        mainProgram = "minecraft-trace";
      };
    }
    ''
      mkdir -p $out/share/java $out/bin
      classpath=$(cat ${serverClasspath}/classpath)

      # javac insists a public class live in a file named after it, and a store
      # path is prefixed with its hash, so the source is copied first.
      cp ${./java/VanillaTrace.java} VanillaTrace.java

      # -nowarn because the server jar carries no -parameters metadata and
      # javac otherwise emits a page of notes about it.
      javac -nowarn -cp "$classpath" -d $out/share/java VanillaTrace.java

      makeWrapper ${lib.getExe' jdk "java"} $out/bin/minecraft-trace \
        --add-flags "-cp $out/share/java:$classpath VanillaTrace"
    '';

  # Named to match the flake's allowUnfreePredicate, which gates the Mojang
  # EULA on a "minecraft-" prefix. A derivation that runs the jar and is not
  # named for it slips past the one check that exists for that licence.
  recordedTraces = pkgs.runCommand "minecraft-differential-traces-${pin.id}"
    {
      nativeBuildInputs = [ recorder pkgs.jq ];
      meta = {
        description = "Per-tick entity state from Minecraft ${pin.id} for every committed scenario";
        license = lib.licenses.unfree; # Mojang EULA; derived from the server jar.
      };
    }
    ''
      # The server resolves a writable home and a timezone during startup and
      # aborts inside the sandbox without both.
      export HOME="$PWD/home" && mkdir -p "$HOME"
      export TZ=UTC

      mkdir -p $out runs

      shopt -s nullglob
      found=0
      for scenario in ${scenarios}/*.json; do
        found=$((found + 1))
        name=$(jq -r .name "$scenario")
        base=$(basename "$scenario" .json)
        if [ "$name" != "$base" ]; then
          echo "$scenario: name is \"$name\" but the file is called \"$base\"" >&2
          exit 1
        fi

        # Every seed, and then a byte comparison. A scenario whose recorded
        # numbers depend on the level's random source disagrees here, which is
        # the only thing standing between this pipeline and a golden file full
        # of noise that no test could ever be trusted to check.
        for seed in ${lib.concatStringsSep " " seeds}; do
          minecraft-trace "$scenario" "runs/$name.$seed.json" "$seed"

          # The seed is recorded in the trace, so it is stripped before the
          # comparison: everything else must be identical.
          jq 'del(.seed)' "runs/$name.$seed.json" > "runs/$name.$seed.stripped"
        done

        first=""
        for seed in ${lib.concatStringsSep " " seeds}; do
          if [ -z "$first" ]; then
            first="$seed"
            continue
          fi
          if ! diff -u "runs/$name.$first.stripped" "runs/$name.$seed.stripped" > seed-diff.txt; then
            echo "$name is not reproducible: seeds $first and $seed disagree" >&2
            echo "the scenario reaches the level's random source, so it cannot be a golden trace" >&2
            head -n 40 seed-diff.txt >&2
            exit 1
          fi
        done

        cp "runs/$name.${lib.head seeds}.json" "$out/$name.json"
      done

      if [ "$found" -eq 0 ]; then
        echo "no scenarios found in ${scenarios}" >&2
        exit 1
      fi
      echo "recorded $found scenarios under ${toString (builtins.length seeds)} seeds each" >&2
    '';

  syncScript = pkgs.writeShellApplication {
    name = "record-differential-traces";
    runtimeInputs = [ pkgs.coreutils pkgs.findutils pkgs.git ];
    text = ''
      root=$(git rev-parse --show-toplevel)
      dest="$root/crates/hyperion/tests/differential/traces"
      rm -rf "$dest"
      mkdir -p "$dest"
      cp -r ${recordedTraces}/. "$dest/"
      chmod -R u+w "$dest"
      echo "recorded $(find "$dest" -type f | wc -l | tr -d ' ') traces into $dest" >&2
    '';
  };

  tracesUpToDate = pkgs.runCommand "check-hyperion-differential-traces"
    { }
    ''
      if diff -r ${committed} ${recordedTraces} > diff.txt 2>&1; then
        touch $out
      else
        echo "committed differential traces no longer match Minecraft ${pin.id};" >&2
        echo "re-record them with: nix run .#record-differential-traces" >&2
        head -n 60 diff.txt >&2
        exit 1
      fi
    '';
in
{
  inherit recorder recordedTraces syncScript tracesUpToDate;
}
