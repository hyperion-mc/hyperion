# Minecraft protocol data pipeline.
#
# Everything downstream of the pinned server jar is a sandboxed derivation, so a
# protocol bump is a one-line change to nix/minecraft-version.json plus a
# rebuild. The version is the only knob.
{ pkgs }:

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

  serverJar = pkgs.fetchurl {
    inherit (pin.server) url;
    hash = pin.server.sha256;
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
      nativeBuildInputs = [ jdk pkgs.cfr pkgs.unzip ];
      meta = {
        description = "Decompiled net.minecraft.network sources for Minecraft ${pin.id}";
        license = lib.licenses.unfree;
      };
    }
    ''
      mkdir -p work && cd work

      # The published jar is a bundler; the real server is nested inside it.
      unzip -q ${serverJar} 'META-INF/versions/*/server-*.jar' -d bundle
      inner=$(find bundle -name 'server-*.jar' | head -n1)
      mkdir -p classes && (cd classes && unzip -q "$inner")

      mkdir -p $out
      # Only the networking tree is decompiled. Decompiling all 7445 classes
      # costs minutes and adds nothing the extractor reads.
      (cd classes && cfr --outputdir $out --silent true --comments false \
        $(find net/minecraft/network -name '*.class' | sort))
    '';

  extractor = pkgs.writers.writePython3Bin "extract-minecraft-protocol" { libraries = [ ]; }
    (builtins.readFile ./extract-protocol.py);

  codegen = pkgs.writers.writePython3Bin "generate-minecraft-proto" { libraries = [ ]; }
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
      nativeBuildInputs = [ codegen ];
      meta.description = "Generated Rust protocol tables for Minecraft ${pin.id}";
    }
    ''
      mkdir -p $out
      generate-minecraft-proto --protocol ${protocolJson}/protocol.json --out $out
    '';

  # Re-resolves Mojang's manifest and rewrites the pin. Impure by nature, so it
  # is an app rather than a derivation.
  updateScript = pkgs.writeShellApplication {
    name = "update-minecraft-data";
    runtimeInputs = [ pkgs.curl pkgs.jq pkgs.nix ];
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
      sri=$(nix store prefetch-file --json --hash-type sha256 "$jar" | jq -r '.hash')

      # The protocol number lives inside the jar, so it is read rather than
      # guessed from the version string.
      tmp=$(mktemp -d)
      trap 'rm -rf "$tmp"' EXIT
      store=$(nix store prefetch-file --json --hash-type sha256 "$jar" | jq -r '.storePath')
      ${lib.getExe' pkgs.unzip "unzip"} -p "$store" version.json > "$tmp/version.json"

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

  # Copies generated Rust into the crate. The output is committed so that a
  # plain `cargo build` works without nix; the check below keeps the two honest.
  syncScript = pkgs.writeShellApplication {
    name = "sync-minecraft-proto";
    runtimeInputs = [ ];
    text = ''
      root=$(git rev-parse --show-toplevel)
      dest="$root/crates/hyperion-minecraft-proto/src/generated"
      rm -rf "$dest"
      mkdir -p "$dest"
      cp -r ${generatedRust}/. "$dest/"
      chmod -R u+w "$dest"
      echo "synced $(find "$dest" -type f | wc -l | tr -d ' ') files into $dest" >&2
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
    ;
  inherit pin;
}
