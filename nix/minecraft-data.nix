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

  # The bundler jar is a launcher wrapped around the real server jar and its
  # dependencies. Both the harness below and anything else wanting to run
  # server code needs them unpacked with a classpath, so it happens once.
  serverClasspath = pkgs.runCommand "minecraft-server-classpath-${pin.id}"
    {
      nativeBuildInputs = [ pkgs.unzip ];
      meta = {
        description = "Unpacked Minecraft ${pin.id} server jar and its libraries";
        license = lib.licenses.unfree;
      };
    }
    ''
      mkdir -p $out
      unzip -q ${serverJar} -d $out

      # META-INF/classpath-joined lists the libraries in the order the bundler
      # loads them, semicolon separated and relative to META-INF. It has no
      # trailing newline, so the version jar is appended rather than echoed on
      # a line of its own.
      libs=$(tr ';' ':' < "$out/META-INF/classpath-joined" \
        | sed "s|libraries/|$out/META-INF/libraries/|g")
      version=$(find "$out/META-INF/versions" -name 'server-*.jar' | head -n1)
      if [ -z "$version" ]; then
        echo "no versioned server jar inside the bundle" >&2
        exit 1
      fi
      printf '%s:%s' "$libs" "$version" > $out/classpath
    '';

  # Prints bytes from Mojang's own encoders. Compiled here rather than by hand
  # so that every slice of the protocol work can check itself against the
  # server without first rebuilding a harness. See hyperion-mc/hyperion#970.
  vanillaEncoder = pkgs.runCommand "minecraft-vanilla-encoder-${pin.id}"
    {
      nativeBuildInputs = [ jdk pkgs.makeWrapper ];
      meta = {
        description = "Harness printing Minecraft ${pin.id} bytes from the server's own codecs";
        license = lib.licenses.unfree;
        # Without this `lib.getExe` guesses the derivation name, and
        # `nix run .#minecraft-encode` dies on a missing
        # `bin/minecraft-vanilla-encoder`.
        mainProgram = "minecraft-encode";
      };
    }
    ''
      mkdir -p $out/share/java $out/bin
      classpath=$(cat ${serverClasspath}/classpath)

      # javac insists a public class live in a file named after it, and a
      # store path is prefixed with its hash, so the source is copied first.
      cp ${./java/VanillaEncoder.java} VanillaEncoder.java

      # -nowarn because the server jar carries no -parameters metadata and
      # javac otherwise emits a page of notes about it.
      javac -nowarn -cp "$classpath" -d $out/share/java VanillaEncoder.java

      makeWrapper ${lib.getExe' jdk "java"} $out/bin/minecraft-encode \
        --add-flags "-cp $out/share/java:$classpath VanillaEncoder"
    '';

  # Named hex strings the Rust tests compare against. Regenerating them is a
  # rebuild rather than a manual run, so a protocol bump moves the fixtures
  # and the tests fail loudly instead of passing against stale bytes.
  encoderFixtures = pkgs.runCommand "minecraft-encoder-fixtures-${pin.id}"
    {
      nativeBuildInputs = [ vanillaEncoder ];
      meta.description = "Reference wire bytes from Minecraft ${pin.id}'s own encoders";
    }
    ''
      export HOME="$PWD/home" && mkdir -p "$HOME"
      mkdir -p $out
      minecraft-encode fixtures $out/fixtures.json
    '';

  # The contents of every synchronised registry, as network NBT.
  #
  # These are datapack-loaded, so `reports/registries.json` has the names and
  # nothing else; the values only exist once the game has built them. Running
  # `RegistrySynchronization`'s own encoding is the only way to get bytes a
  # client will accept.
  registryContents = pkgs.runCommand "minecraft-registry-contents-${pin.id}"
    {
      nativeBuildInputs = [ vanillaEncoder ];
      meta = {
        description = "Network NBT for Minecraft ${pin.id}'s synchronised registries";
        license = lib.licenses.unfree; # Mojang EULA; derived from the server jar.
      };
    }
    ''
      export HOME="$PWD/home" && mkdir -p "$HOME"
      mkdir -p $out
      minecraft-encode registries $out

      # A registry that fails to encode is reported rather than dropped, and
      # the three a client cannot render without must never be among them.
      for required in dimension_type worldgen.biome chat_type; do
        if [ ! -s "$out/minecraft.$required.nbt" ]; then
          echo "registry dump is missing minecraft.$required" >&2
          cat $out/skipped.json >&2
          exit 1
        fi
      done
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

  registryCodegen = pkgs.writers.writePython3Bin "generate-minecraft-registry-data" pythonWriterOptions
    (builtins.readFile ./generate-registry-data.py);

  blockStateCodegen = pkgs.writers.writePython3Bin "generate-minecraft-block-states" pythonWriterOptions
    (builtins.readFile ./generate-block-states.py);

  entityTypeCodegen = pkgs.writers.writePython3Bin "generate-minecraft-entity-types" pythonWriterOptions
    (builtins.readFile ./generate-entity-types.py);

  # The NBT blobs live next to the Rust that `include_bytes!`es them, so this
  # output is a whole directory rather than a single file.
  generatedRegistryData = pkgs.runCommand "hyperion-minecraft-registry-data-${pin.id}"
    {
      nativeBuildInputs = [ registryCodegen pkgs.rustfmt ];
      meta.description = "Generated Rust registry contents for Minecraft ${pin.id}";
    }
    ''
      mkdir -p $out
      generate-minecraft-registry-data \
        --dump ${registryContents} \
        --version ${pin.id} \
        --out $out
      find $out -name '*.rs' -exec rustfmt --edition 2024 {} +
    '';

  # blocks.json lists all 32366 states one by one; the generator collapses that
  # to one row per block, which is only sound because it re-proves the two facts
  # that make it sound on every run. A single file rather than a directory, so
  # the staleness check below is a plain diff.
  generatedBlockStates = pkgs.runCommand "hyperion-minecraft-block-states-${pin.id}"
    {
      nativeBuildInputs = [ blockStateCodegen rustfmt ];
      meta.description = "Generated Rust block state table for Minecraft ${pin.id}";
    }
    ''
      mkdir -p $out
      generate-minecraft-block-states \
        --blocks ${generatedData}/reports/blocks.json \
        --version ${pin.id} \
        --protocol ${toString pin.protocolVersion} \
        --out $out/block_state.rs
      rustfmt --edition 2024 --config-path ${../rustfmt.toml} $out/block_state.rs
    '';

  # Entity type ids come out of protocol.json's registries rather than out of a
  # second read of the jar, so this table and generated/registry.rs cannot
  # disagree about what `minecraft:entity_type` holds. A single file, so the
  # staleness check below is a plain diff.
  generatedEntityTypes = pkgs.runCommand "hyperion-minecraft-entity-types-${pin.id}"
    {
      nativeBuildInputs = [ entityTypeCodegen rustfmt ];
      meta.description = "Generated Rust entity type table for Minecraft ${pin.id}";
    }
    ''
      mkdir -p $out
      generate-minecraft-entity-types \
        --protocol ${protocolJson}/protocol.json \
        --out $out/entity_type.rs
      rustfmt --edition 2024 --config-path ${../rustfmt.toml} $out/entity_type.rs
    '';

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

  syncRegistryDataScript = pkgs.writeShellApplication {
    name = "sync-minecraft-registry-data";
    runtimeInputs = [ pkgs.coreutils pkgs.findutils pkgs.git ];
    text = ''
      root=$(git rev-parse --show-toplevel)
      dest="$root/crates/hyperion-minecraft-proto/src/registry_data"
      rm -rf "$dest"
      mkdir -p "$dest"
      cp -r ${generatedRegistryData}/. "$dest/"
      chmod -R u+w "$dest"
      echo "synced $(find "$dest" -type f | wc -l | tr -d ' ') files into $dest" >&2
    '';
  };

  syncBlockStatesScript = pkgs.writeShellApplication {
    name = "sync-minecraft-block-states";
    runtimeInputs = [ pkgs.coreutils pkgs.git ];
    text = ''
      root=$(git rev-parse --show-toplevel)
      dest="$root/crates/hyperion-minecraft-proto/src/block_state.rs"
      install -m 644 ${generatedBlockStates}/block_state.rs "$dest"
      echo "synced $dest" >&2
    '';
  };

  syncEntityTypesScript = pkgs.writeShellApplication {
    name = "sync-minecraft-entity-types";
    runtimeInputs = [ pkgs.coreutils pkgs.git ];
    text = ''
      root=$(git rev-parse --show-toplevel)
      dest="$root/crates/hyperion-minecraft-proto/src/entity_type.rs"
      install -m 644 ${generatedEntityTypes}/entity_type.rs "$dest"
      echo "synced $dest" >&2
    '';
  };

  # The fixtures are committed so `cargo test` runs without nix. That only
  # stays honest if something notices when the jar stops producing them.
  fixturesUpToDate = pkgs.runCommand "check-minecraft-encoder-fixtures"
    { }
    ''
      committed=${../crates/hyperion-minecraft-proto/tests/fixtures/vanilla.json}
      if diff "$committed" ${encoderFixtures}/fixtures.json > diff.txt 2>&1; then
        touch $out
      else
        echo "committed test fixtures are stale; regenerate with:" >&2
        echo "  nix run .#minecraft-encode -- fixtures \\" >&2
        echo "    crates/hyperion-minecraft-proto/tests/fixtures/vanilla.json" >&2
        cat diff.txt >&2
        exit 1
      fi
    '';

  registryDataUpToDate = pkgs.runCommand "check-minecraft-registry-data"
    { }
    ''
      committed=${../crates/hyperion-minecraft-proto/src/registry_data}
      if diff -r "$committed" ${generatedRegistryData} > diff.txt 2>&1; then
        touch $out
      else
        echo "committed registry data is stale; run: nix run .#sync-minecraft-registry-data" >&2
        cat diff.txt >&2
        exit 1
      fi
    '';

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

  blockStatesUpToDate = pkgs.runCommand "check-minecraft-block-states"
    { }
    ''
      committed=${../crates/hyperion-minecraft-proto/src/block_state.rs}
      if diff -u "$committed" ${generatedBlockStates}/block_state.rs > diff.txt 2>&1; then
        touch $out
      else
        echo "committed block state table is stale; run: nix run .#sync-minecraft-block-states" >&2
        head -n 200 diff.txt >&2
        exit 1
      fi
    '';

  entityTypesUpToDate = pkgs.runCommand "check-minecraft-entity-types"
    { }
    ''
      committed=${../crates/hyperion-minecraft-proto/src/entity_type.rs}
      if diff -u "$committed" ${generatedEntityTypes}/entity_type.rs > diff.txt 2>&1; then
        touch $out
      else
        echo "committed entity type table is stale; run: nix run .#sync-minecraft-entity-types" >&2
        head -n 200 diff.txt >&2
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
    serverClasspath
    generatedData
    decompiledSources
    protocolJson
    generatedRust
    vanillaEncoder
    encoderFixtures
    registryContents
    generatedRegistryData
    generatedBlockStates
    generatedEntityTypes
    extractor
    codegen
    registryCodegen
    blockStateCodegen
    entityTypeCodegen
    updateScript
    syncScript
    syncRegistryDataScript
    syncBlockStatesScript
    syncEntityTypesScript
    generatedUpToDate
    registryDataUpToDate
    blockStatesUpToDate
    entityTypesUpToDate
    fixturesUpToDate
    protocolJsonUpToDate
    ;
  inherit pin;
}
