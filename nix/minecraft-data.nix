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

      # An element codec that throws now kills the dump, so the only way a
      # registry goes missing is `RegistryDataLoader.SYNCHRONIZED_REGISTRIES`
      # naming one this build does not carry. The three a client cannot render
      # without are checked by name, because a short dump is otherwise a
      # perfectly well-formed one.
      for required in dimension_type worldgen.biome chat_type; do
        if [ ! -s "$out/minecraft.$required.nbt" ]; then
          echo "registry dump is missing minecraft.$required" >&2
          exit 1
        fi
      done
    '';

  # The whole tag map, as `ClientboundUpdateTagsPacket` puts it on the wire.
  #
  # Tags are not datapack content the way registry elements are: a client keeps
  # the ones it loaded from its own packs only until a server tells it
  # otherwise, and a registry element naming a tag the server never sent fails
  # to parse. So this is not an optimisation, it is what makes a join work.
  tagContents = pkgs.runCommand "minecraft-tag-contents-${pin.id}"
    {
      nativeBuildInputs = [ vanillaEncoder ];
      meta = {
        description = "Network tag map for Minecraft ${pin.id}";
        license = lib.licenses.unfree; # Mojang EULA; derived from the server jar.
      };
    }
    ''
      export HOME="$PWD/home" && mkdir -p "$HOME"
      mkdir -p $out
      minecraft-encode tags $out

      # The three registries whose tags the synchronised registry elements
      # name. A dump missing one is well formed and still disconnects every
      # real client at `finish_configuration`, so it is checked by name.
      for required in item block entity_type; do
        if [ ! -s "$out/minecraft.$required.bin" ]; then
          echo "tag dump is missing minecraft.$required" >&2
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

  tagDataCodegen = pkgs.writers.writePython3Bin "generate-minecraft-tag-data" pythonWriterOptions
    (builtins.readFile ./generate-tag-data.py);

  blockStateCodegen = pkgs.writers.writePython3Bin "generate-minecraft-block-states" pythonWriterOptions
    (builtins.readFile ./generate-block-states.py);

  coverageChecker = pkgs.writers.writePython3Bin "check-minecraft-proto-coverage" pythonWriterOptions
    (builtins.readFile ./check-proto-coverage.py);

  toolPathChecker = pkgs.writers.writePython3Bin "check-tool-paths" pythonWriterOptions
    (builtins.readFile ./check-tool-paths.py);

  literalChecker = pkgs.writers.writePython3Bin "check-minecraft-literals" pythonWriterOptions
    (builtins.readFile ./check-minecraft-literals.py);

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

  # Same shape as generatedRegistryData: the id lists ride along as binary
  # files next to the Rust that `include_bytes!`es them.
  #
  # The repo's own rustfmt and rustfmt.toml, not nixpkgs' defaults, because the
  # committed copy is what `cargo fmt --check` sees: formatting it any other way
  # makes `fmt` rewrite a file nobody touched and the staleness check below then
  # calls it stale.
  generatedTagData = pkgs.runCommand "hyperion-minecraft-tag-data-${pin.id}"
    {
      nativeBuildInputs = [ tagDataCodegen rustfmt ];
      meta.description = "Generated Rust tag map for Minecraft ${pin.id}";
    }
    ''
      mkdir -p $out
      generate-minecraft-tag-data \
        --dump ${tagContents} \
        --version ${pin.id} \
        --out $out
      find $out -name '*.rs' -exec \
        rustfmt --edition 2024 --config-path ${../rustfmt.toml} {} +
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
    runtimeInputs = [ pkgs.coreutils pkgs.findutils pkgs.git coverageChecker ];
    text = ''
      root=$(git rev-parse --show-toplevel)
      crate="$root/crates/hyperion-minecraft-proto"

      install -m 644 ${protocolJson}/protocol.json "$crate/protocol.json"

      # The coverage baseline rides along, so tightening it is the same command
      # as regenerating what it describes and the two cannot fall out of step.
      check-minecraft-proto-coverage \
        --protocol "$crate/protocol.json" \
        --baseline "$root/nix/proto-coverage-baseline.json" \
        --write

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

  syncTagDataScript = pkgs.writeShellApplication {
    name = "sync-minecraft-tag-data";
    runtimeInputs = [ pkgs.coreutils pkgs.findutils pkgs.git ];
    text = ''
      root=$(git rev-parse --show-toplevel)
      dest="$root/crates/hyperion-minecraft-proto/src/tag_data"
      rm -rf "$dest"
      mkdir -p "$dest"
      cp -r ${generatedTagData}/. "$dest/"
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

  # Rewrites the raw-literal baseline. The same command tightens it after a
  # migration and records a deliberate new one, so the two cannot drift.
  syncLiteralsScript = pkgs.writeShellApplication {
    name = "sync-minecraft-literals";
    runtimeInputs = [ pkgs.git literalChecker ];
    text = ''
      root=$(git rev-parse --show-toplevel)
      check-minecraft-literals \
        --root "$root" \
        --baseline "$root/nix/minecraft-literal-baseline.json" \
        --write
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

  # Loads the registries with only the shipped tags bound, the way a joining
  # client does, using Mojang's own loader and codecs. A tag set too small for
  # a real client to join fails here rather than on a player's screen.
  #
  # Sending fewer tags than vanilla is the tempting optimisation and this is
  # what makes it safe to try: trim the dump and this check names the element
  # that stops parsing.
  #
  # The empty case runs too. A check nobody has watched fail is not a check,
  # and this one would pass just as happily if `verify-tags` had quietly
  # stopped loading anything.
  tagsLoadForClient = pkgs.runCommand "check-minecraft-tags-load"
    {
      nativeBuildInputs = [ vanillaEncoder ];
    }
    ''
      export HOME="$PWD/home" && mkdir -p "$HOME"

      echo "loading the registries with the shipped tags bound" >&2
      minecraft-encode verify-tags ${tagContents}

      echo "loading them again with no tags bound, which must fail" >&2
      mkdir -p empty && echo '[]' > empty/index.json
      if minecraft-encode verify-tags empty > empty.log 2>&1; then
        echo "the registries loaded with no tags bound, so this check proves nothing" >&2
        exit 1
      fi
      if ! grep -q "Missing tag" empty.log; then
        echo "the empty load failed for some reason other than a missing tag:" >&2
        cat empty.log >&2
        exit 1
      fi
      grep -m1 "Missing tag" empty.log >&2

      touch $out
    '';

  tagDataUpToDate = pkgs.runCommand "check-minecraft-tag-data"
    { }
    ''
      committed=${../crates/hyperion-minecraft-proto/src/tag_data}
      if diff -r "$committed" ${generatedTagData} > diff.txt 2>&1; then
        touch $out
      else
        echo "committed tag data is stale; run: nix run .#sync-minecraft-tag-data" >&2
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

  # The coverage gap, held against a committed baseline.
  #
  # `complete: false` in protocol.json means a packet has to be hand-written or
  # simply does not exist, and until this check nothing anywhere said how many
  # there were. Sixty-five had accumulated unnoticed, one of which
  # (`minecraft:interact`) meant no entity interaction of any kind worked until
  # somebody found it by hand.
  #
  # A ratchet rather than a target of zero: several of the remaining causes are
  # real modelling work. It fails in both directions, because a baseline nobody
  # tightens stops bounding anything, and both directions are fixed by
  # `nix run .#sync-minecraft-proto`.
  coverageRatchet = pkgs.runCommand "check-minecraft-proto-coverage"
    {
      nativeBuildInputs = [ coverageChecker ];
    }
    ''
      check-minecraft-proto-coverage \
        --protocol ${protocolJson}/protocol.json \
        --baseline ${../nix/proto-coverage-baseline.json}
      touch $out
    '';

  # A script naming a repository path that does not exist.
  #
  # Two scripted clients read `src/generated/registry.rs` with a regex. When
  # that file became a directory, both kept building -- a path in a Python
  # string is invisible to cargo and to every Rust grep -- and failed twenty
  # minutes into CI inside four end-to-end gates, on a `FileNotFoundError`.
  # A third named `src/entity_type.rs` and would have failed the same way one
  # merge later.
  #
  # The real fix was to stop reaching into another component's source: all
  # three read `protocol.json` now, which is data with a shape rather than a
  # file with a location. This is the cheap half, and it is what makes the
  # class fail in seconds instead of in a gate.
  toolPaths = pkgs.runCommand "check-tool-paths"
    {
      nativeBuildInputs = [ toolPathChecker pkgs.git ];
    }
    ''
      cp -r ${toolPathSource} source
      chmod -R u+w source
      cd source
      git init -q .
      git add -A
      check-tool-paths --root .
      touch $out
    '';

  # The checker reads scripts and stats paths, so it needs the tree's shape as
  # well as the scripts themselves. `fileFilter` on the whole root would drag
  # in every Rust file's contents; this takes names cheaply by taking the
  # directories the anchors name.
  toolPathSource = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../tools
      ../nix
      ../crates
      ../events
      ../docs
    ];
  };

  # A raw `"minecraft:..."` in Rust is an unproven claim, and this is what
  # stops the class from growing back.
  #
  # Every static registry is a closed enum now, so a name written as a string
  # is a name nothing checks: a typo or a Mojang rename becomes a `None` at run
  # time rather than a compile error. The checker walks each file the way Rust
  # lexes it -- a regex over lines cannot tell `//! minecraft:pig` from
  # `"minecraft:pig"`, and this repo's doc comments name registry entries
  # constantly.
  #
  # A ratchet in both directions, like the coverage one. A new literal is a
  # regression; a literal that has gone means the baseline has stopped bounding
  # anything and should be tightened with `nix run .#sync-minecraft-literals`.
  #
  # The source is filtered to what the checker reads. Without that, every Rust
  # edit anywhere in the tree rebuilds this.
  literalRatchet = pkgs.runCommand "check-minecraft-literals"
    {
      nativeBuildInputs = [ literalChecker pkgs.git ];
    }
    ''
      cp -r ${literalSource} source
      chmod -R u+w source
      cd source
      git init -q .
      git add -A
      check-minecraft-literals \
        --root . \
        --baseline ${../nix/minecraft-literal-baseline.json}
      touch $out
    '';

  # `git ls-files` is how the checker finds its inputs, so the copy it runs
  # against has to be a git tree. Only Rust sources and the checker's own
  # baseline matter, so nothing else is copied in.
  literalSource = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.fileFilter (file: file.hasExt "rs") ../.;
  };

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
    jdk
    serverJar
    serverClasspath
    generatedData
    decompiledSources
    protocolJson
    generatedRust
    vanillaEncoder
    encoderFixtures
    registryContents
    tagContents
    generatedRegistryData
    generatedTagData
    generatedBlockStates
    extractor
    codegen
    registryCodegen
    tagDataCodegen
    blockStateCodegen
    updateScript
    syncScript
    syncRegistryDataScript
    syncTagDataScript
    syncBlockStatesScript
    coverageChecker
    coverageRatchet
    toolPathChecker
    toolPaths
    literalChecker
    literalRatchet
    syncLiteralsScript
    generatedUpToDate
    registryDataUpToDate
    tagDataUpToDate
    tagsLoadForClient
    blockStatesUpToDate
    fixturesUpToDate
    protocolJsonUpToDate
    ;
  inherit pin;
}
