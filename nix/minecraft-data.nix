# Minecraft protocol data pipeline.
#
# Everything downstream of the pinned server jar is a sandboxed derivation, so a
# protocol bump is a one-line change to nix/minecraft-version.json plus a
# rebuild. The version is the only knob.
#
# `rustfmt` is the repo's pinned toolchain rather than nixpkgs' own: rustfmt.toml
# turns on unstable options, which only the nightly binary accepts.
{ pkgs, rustfmt }:

let
  inherit (pkgs) lib;

  pin = lib.importJSON ./minecraft-version.json;

  # 26.x refuses to start on anything older; the version pin records the
  # requirement so a bump that needs a newer JDK fails at eval, not at runtime.
  jdk =
    let
      attr = "jdk${toString pin.javaMajor}";
    in
    pkgs.${attr} or (throw "nixpkgs has no ${attr}, required by Minecraft ${pin.id}");

  # The jar is the one unfree thing in the tree, so it is named to match the
  # flake's allowUnfreePredicate ("minecraft-" prefixed) and carries the
  # licence itself. Without both, the policy that is supposed to gate Mojang's
  # EULA never actually looks at the file it exists for.
  serverJar = pkgs.fetchurl {
    name = "minecraft-server-${pin.id}.jar";
    inherit (pin.server) url;
    hash = pin.server.sha256;
    meta = {
      description = "Mojang's Minecraft ${pin.id} server jar";
      license = lib.licenses.unfree;
    };
  };

  # Mojang's own data generator. It needs no mappings and never did, which is
  # why it is the one extraction path unaffected by 26.1 dropping obfuscation.
  generatedData = pkgs.runCommand "minecraft-generated-data-${pin.id}"
    {
      nativeBuildInputs = [ jdk ];
      passthru = { inherit (pin) id protocolVersion; };
      meta = {
        description = "Vanilla data generator reports for Minecraft ${pin.id}";
        license = lib.licenses.unfree; # Mojang EULA; derived from the server jar.
      };
    }
    ''
      mkdir -p work && cd work

      # The generator resolves a writable user home and a timezone; without both
      # it aborts inside the sandbox rather than emitting a diagnostic.
      export HOME="$PWD/home"
      export TZ=UTC
      mkdir -p "$HOME"

      java -DbundlerMainClass=net.minecraft.data.Main \
        -jar ${serverJar} --all

      mkdir -p $out
      cp -r generated/reports $out/reports
      cp -r generated/data $out/data

      # version.json rides along so downstream steps read the protocol number
      # from the jar itself rather than trusting the pin.
      ${lib.getExe' pkgs.unzip "unzip"} -p ${serverJar} version.json > $out/version.json
    '';

  # Decompiled sources are the only place packet wire layouts exist: the
  # generator's packets.json carries ids and names but no field information.
  decompiledSources = pkgs.runCommand "minecraft-decompiled-${pin.id}"
    {
      # No jdk: nixpkgs wraps cfr with its own runtime, so pulling a second
      # one into the closure buys nothing.
      nativeBuildInputs = [ pkgs.cfr pkgs.unzip ];
      meta = {
        description = "Decompiled packet and codec sources for Minecraft ${pin.id}";
        license = lib.licenses.unfree;
      };
    }
    ''
      mkdir -p work && cd work

      # The published jar is a bundler; the real server is nested inside it.
      unzip -q ${serverJar} 'META-INF/versions/*/server-*.jar' -d bundle
      inner=$(find "$PWD/bundle" -name 'server-*.jar' | head -n1)
      if [ -z "$inner" ]; then
        echo "no inner server jar found in the bundle" >&2
        exit 1
      fi
      mkdir -p classes && (cd classes && unzip -q "$inner")
      cd classes

      # Which classes to decompile is decided from the jar rather than from a
      # list, so that a version bump cannot silently stop covering something.
      # Three clauses, each for a reason:
      #
      #   net/minecraft/network        every packet class lives here
      #   any class naming StreamCodec every codec definition a packet layout
      #                                reaches, wherever it lives: BlockPos in
      #                                core, ItemStack in world/item, the 111
      #                                data component types in
      #                                world/item/component
      #   net/minecraft/core/registries  the registry key table, which is what
      #                                turns a registry id into a name
      #   any enum a packet references  `writeEnum` sends an ordinal, so the
      #                                declaration order of the constants is
      #                                the discriminant table; enums like
      #                                RecipeBookType carry no StreamCodec of
      #                                their own and would otherwise be missed
      #
      # This is about a sixth of the jar's 7434 classes and costs ten seconds;
      # decompiling everything is minutes for sources nothing reads.
      #
      # An enum is recognised by `java/lang/Enum` in its constant pool, which
      # is in the class file for every enum and for nothing else.
      grep -rhoa 'net/minecraft/[A-Za-z0-9/$]*' --include='*.class' net/minecraft/network \
        | sed 's/$/.class/' | sort -u > referenced.txt
      : > enums.txt
      while read -r candidate; do
        if [ -e "$candidate" ] && grep -qa 'java/lang/Enum' "$candidate"; then
          printf '%s\n' "$candidate" >> enums.txt
        fi
      done < referenced.txt
      echo "  of which $(wc -l < enums.txt) are enums a packet names" >&2

      { find net/minecraft/network net/minecraft/core/registries -name '*.class'
        grep -rl StreamCodec --include='*.class' net/minecraft
        cat enums.txt
      } | sed 's/\$[^/]*\.class$/.class/' | sort -u > outers.txt

      # Inner classes are decompiled into the file of their outermost class, so
      # each selected class has to bring its siblings along or cfr writes a file
      # missing the nested codec the extractor came for. The glob is anchored on
      # '$' so that picking Foo.class does not also drag in FooBar.class.
      while read -r outer; do
        printf '%s\n' "$outer"
        for nested in "''${outer%.class}"\$*.class; do
          # An `if` rather than `[ -e ] &&`, whose non-zero status on the last
          # iteration would become the loop's and trip pipefail.
          if [ -e "$nested" ]; then
            printf '%s\n' "$nested"
          fi
        done
      done < outers.txt | sort -u > selected.txt

      echo "decompiling $(wc -l < selected.txt) of $(find net/minecraft -name '*.class' | wc -l) classes" >&2

      mkdir -p $out
      cfr --outputdir $out --silent true --comments false $(cat selected.txt)
    '';

  # writePython3Bin runs flake8 over each script, which is what keeps them
  # honest without a separate lint step. Three checks are turned off, each
  # because it argues with something the code does on purpose:
  #
  #   E501  both scripts are code emitters, and the long lines are format
  #         strings whose layout mirrors the Rust they produce; reflowing them
  #         makes the generated output harder to read, not the script
  #   E203  slice bounds are written `text[a : b]` with the spaces PEP 8 asks
  #         for around a colon in a complex slice, which E203 rejects and
  #         every current formatter emits
  #   W503  PEP 8 recommends breaking *before* a binary operator, which is
  #         what W503 flags; its opposite W504 is the one to keep
  pythonWriterOptions = {
    libraries = [ ];
    flakeIgnore = [ "E501" "E203" "W503" ];
  };

  extractor = pkgs.writers.writePython3Bin "extract-minecraft-protocol" pythonWriterOptions
    (builtins.readFile ./extract-protocol.py);

  codegen = pkgs.writers.writePython3Bin "generate-minecraft-proto" pythonWriterOptions
    (builtins.readFile ./generate-rust.py);

  protocolJson = pkgs.runCommand "minecraft-protocol-${pin.id}.json"
    {
      nativeBuildInputs = [ extractor ];
      meta.description = "Extracted protocol description for Minecraft ${pin.id}";
    }
    ''
      mkdir -p $out
      extract-minecraft-protocol \
        --generated ${generatedData} \
        --decompiled ${decompiledSources} \
        --version-json ${generatedData}/version.json \
        --out $out/protocol.json
    '';

  generatedRust = pkgs.runCommand "hyperion-minecraft-proto-generated-${pin.id}"
    {
      # rustfmt runs here rather than on the committed copy: formatting the
      # copy alone would make it differ from what the generator emits, and the
      # staleness check compares the two.
      #
      # It needs the repo's own rustfmt.toml. Without --config-path it would
      # find no configuration in the sandbox and settle on defaults, so the
      # committed tables would disagree with `cargo fmt` -- which rewrites them
      # on the next run and fails `fmt --check` on a tree nobody has touched.
      nativeBuildInputs = [ codegen rustfmt ];
      meta.description = "Generated Rust protocol tables for Minecraft ${pin.id}";
    }
    ''
      mkdir -p $out
      generate-minecraft-proto --protocol ${protocolJson}/protocol.json --out $out
      find $out -name '*.rs' -exec \
        rustfmt --edition 2024 --config-path ${../rustfmt.toml} {} +
    '';

  # Re-resolves Mojang's manifest and rewrites the pin. Impure by nature, so it
  # is an app rather than a derivation.
  updateScript = pkgs.writeShellApplication {
    name = "update-minecraft-data";
    # git resolves the repository root. writeShellApplication only prepends
    # these to PATH, so an undeclared tool silently falls through to whatever
    # the caller happens to have.
    runtimeInputs = [ pkgs.curl pkgs.git pkgs.jq pkgs.nix pkgs.unzip ];
    text = ''
      manifest=${lib.escapeShellArg pin.manifestUrl}
      target="''${1:-}"

      root=$(git rev-parse --show-toplevel)
      out="$root/nix/minecraft-version.json"

      meta=$(curl -sSf "$manifest")
      if [ -z "$target" ]; then
        target=$(printf '%s' "$meta" | jq -r '.latest.release')
      fi
      echo "resolving Minecraft $target" >&2

      url=$(printf '%s' "$meta" | jq -r --arg v "$target" '.versions[] | select(.id == $v) | .url')
      if [ -z "$url" ] || [ "$url" = "null" ]; then
        echo "no such version: $target" >&2
        exit 1
      fi

      version=$(curl -sSf "$url")
      jar=$(printf '%s' "$version" | jq -r '.downloads.server.url')
      sha1=$(printf '%s' "$version" | jq -r '.downloads.server.sha1')
      size=$(printf '%s' "$version" | jq -r '.downloads.server.size')
      java=$(printf '%s' "$version" | jq -r '.javaVersion.majorVersion')
      released=$(printf '%s' "$version" | jq -r '.releaseTime')
      kind=$(printf '%s' "$version" | jq -r '.type')

      echo "fetching $jar" >&2
      prefetch=$(nix store prefetch-file --json --hash-type sha256 "$jar")
      sri=$(printf '%s' "$prefetch" | jq -r '.hash')
      store=$(printf '%s' "$prefetch" | jq -r '.storePath')

      # The protocol number lives inside the jar, so it is read rather than
      # inferred from the version string, which has no stable relationship to it.
      tmp=$(mktemp -d)
      trap 'rm -rf "$tmp"' EXIT
      unzip -p "$store" version.json > "$tmp/version.json"

      jq -n \
        --arg id "$target" \
        --arg type "$kind" \
        --arg released "$released" \
        --arg manifest "$manifest" \
        --arg versionMeta "$url" \
        --arg jar "$jar" \
        --arg sha1 "$sha1" \
        --arg sri "$sri" \
        --argjson size "$size" \
        --argjson java "$java" \
        --slurpfile inner "$tmp/version.json" \
        '{
          id: $id,
          type: $type,
          releaseTime: $released,
          protocolVersion: $inner[0].protocol_version,
          worldVersion: $inner[0].world_version,
          packVersion: {
            resourceMajor: $inner[0].pack_version.resource_major,
            resourceMinor: $inner[0].pack_version.resource_minor,
            dataMajor: $inner[0].pack_version.data_major,
            dataMinor: $inner[0].pack_version.data_minor
          },
          javaMajor: $java,
          manifestUrl: $manifest,
          versionMetaUrl: $versionMeta,
          server: { url: $jar, sha1: $sha1, sha256: $sri, size: $size }
        }' > "$out"

      echo "wrote $out" >&2
      cat "$out" >&2
    '';
  };

  # Copies the extraction into the crate: protocol.json, which build.rs turns
  # into packet structs, and the tables that stay committed Rust. Both are in
  # the tree so that a plain `cargo build` works without nix; the checks below
  # are what make the committed copies trustworthy rather than merely present.
  syncScript = pkgs.writeShellApplication {
    name = "sync-minecraft-proto";
    runtimeInputs = [ pkgs.coreutils pkgs.findutils pkgs.git ];
    text = ''
      root=$(git rev-parse --show-toplevel)
      crate="$root/crates/hyperion-minecraft-proto"

      install -m 644 ${protocolJson}/protocol.json "$crate/protocol.json"

      dest="$crate/src/generated"
      rm -rf "$dest"
      mkdir -p "$dest"
      cp -r ${generatedRust}/. "$dest/"
      chmod -R u+w "$dest"

      echo "synced protocol.json and $(find "$dest" -type f | wc -l | tr -d ' ') tables into $crate" >&2
    '';
  };

  # Fails if the committed generated sources drift from what the pipeline
  # produces, which is the only thing making the committed copy trustworthy.
  generatedUpToDate = pkgs.runCommand "check-minecraft-proto-generated"
    { }
    ''
      committed=${../crates/hyperion-minecraft-proto/src/generated}
      if diff -r "$committed" ${generatedRust} > diff.txt 2>&1; then
        touch $out
      else
        echo "committed generated sources are stale; run: nix run .#sync-minecraft-proto" >&2
        cat diff.txt >&2
        exit 1
      fi
    '';

  # protocol.json is the input build.rs reads, so a stale copy is a stale
  # packet struct in every build that does not go through nix. Guarding it is
  # what lets the structs live in OUT_DIR instead of in the tree.
  protocolJsonUpToDate = pkgs.runCommand "check-minecraft-protocol-json"
    { }
    ''
      committed=${../crates/hyperion-minecraft-proto/protocol.json}
      if diff -u "$committed" ${protocolJson}/protocol.json > diff.txt 2>&1; then
        touch $out
      else
        echo "committed protocol.json is stale; run: nix run .#sync-minecraft-proto" >&2
        head -n 200 diff.txt >&2
        exit 1
      fi
    '';
in
{
  inherit
    serverJar
    generatedData
    decompiledSources
    protocolJson
    generatedRust
    extractor
    codegen
    updateScript
    syncScript
    generatedUpToDate
    protocolJsonUpToDate
    ;
  inherit pin;
}
