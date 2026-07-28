# The gate over the text path: styling is a field, never characters in a
# string.
#
# The type is what makes the sidebar bug unrepresentable. Every text-carrying
# method on `smash::server::Server` takes a `Component`, so a colour cannot
# cross the seam as markup inside a `&str`, which is exactly how `[green]`
# reached a real client. Nothing here restates that; the compiler already does.
#
# What a type cannot reach is the one legacy spelling the client still
# understands *inside* component text: a section sign followed by a format
# character, which the pre-1.16 formatter picks out of any literal. It renders,
# so it looks like it works. It says nothing outside the sixteen named colours,
# it is invisible to every assertion that reads `Component::plain`, and it is
# the same mistake in a different alphabet. There is no type that forbids a
# character in a string, so this is the gate.
{ pkgs, sources }:
let
  # The one file in scope that still writes them. `tell` in smash's command
  # module goes straight to `hyperion::net::agnostic::chat` and never crosses
  # the `Server` seam, so the type does not reach it. Named rather than
  # pattern-matched, so a new file cannot quietly join it. ENG-10796 tracks
  # this one and the rest of the workspace.
  exempt = "command.rs";
in
pkgs.runCommand "smash-text-no-legacy-formatting" { } ''
  # Built with printf rather than written literally, so widening this gate to
  # cover nix files could never trip it over its own source.
  sign=$(printf '\302\247')

  grep -rlF "$sign" --include='*.rs' \
    ${sources.smashSource} ${sources.protoSource} > hits.txt || true

  grep -v "/${exempt}$" hits.txt > offenders.txt || true
  if [ -s offenders.txt ]; then
    echo "FAIL: legacy section-sign formatting codes in the text path." >&2
    sed \
      -e 's#^${sources.smashSource}#events/smash/src#' \
      -e 's#^${sources.protoSource}#crates/hyperion-minecraft-proto/src#' \
      offenders.txt >&2
    echo >&2
    echo "A colour is a field on a component, not characters in a literal." >&2
    echo "Reach for Component::color(NamedColor::..) or .bold(), which the" >&2
    echo "client renders through the component codec rather than through a" >&2
    echo "formatter kept alive for 1.8 servers." >&2
    exit 1
  fi

  # The exemption has to still be earned. Without this it outlives the debt
  # and quietly re-permits the file the day somebody cleans it up.
  if ! grep -q "/${exempt}$" hits.txt; then
    echo "FAIL: ${exempt} no longer has legacy formatting codes, so the" >&2
    echo "exemption in nix/text.nix is stale. Delete it, and close" >&2
    echo "ENG-10796 if nothing else is left." >&2
    exit 1
  fi

  touch "$out"
''
