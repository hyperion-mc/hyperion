# Which flake checks CI enforces, and the named exceptions.
#
# Enforcement is subtractive: every attribute of `checks` is built and must
# pass, except the names listed in `excluded`. A check added tomorrow is
# enforced the day it lands rather than the day someone remembers to add it
# here, which is the failure this file exists to prevent. See ENG-10817: the
# job that would have built these sat behind `continue-on-error: true`, so
# every gate landed in that window was believed to be running when it was not.
#
# An exclusion is not a comment. `nix run .#flake-gate` builds each excluded
# check as well and requires it to STILL FAIL. The day one starts passing, the
# gate goes red and names the entry to delete, so an exception cannot outlive
# the defect it was written for.
{
  lib,
  writeShellApplication,
  system,
  # Every name in `checks`, rather than the derivations: the gate shells out to
  # `nix build .#checks.<system>.<name>` and needs no more than the name.
  names,
}:
let
  # Each entry is the evidence that set it, not a justification. A reason
  # nobody can check is what ENG-10817 was about.
  excluded = {
    differential-traces = ''
      The committed golden traces disagree with the vanilla server the pinned
      jar runs. Measured 2026-07-28 on ubuntu-latest, GitHub Actions run
      30341066882: the snowball trajectory differs from tick 0 onward, so the
      recording predates a physics change rather than the server having
      regressed. Re-record with `nix run .#record-differential-traces`, check
      the diff is the physics change you expect, and delete this entry.
    '';
  };

  excludedNames = lib.attrNames excluded;

  # A renamed or deleted check must not leave a dead exclusion behind, reading
  # as though it exempts something while exempting nothing.
  stale = lib.subtractLists names excludedNames;

  enforced = lib.subtractLists excludedNames names;

  quote = xs: lib.concatMapStringsSep " " lib.escapeShellArg xs;

  gate = writeShellApplication {
    name = "flake-gate";
    text = ''
      flake="''${1:-.}"

      # cargoUnit content-addresses every crate unit, so a store that cannot
      # realise a content-addressed derivation cannot build most of this flake.
      # Checked here rather than assumed because the failure it produces
      # otherwise is `store path '...bedwars-0.1.0.drv' does not exist`, which
      # points nowhere near the cause and cost ENG-10817 months of a green-
      # looking CI. Note the client setting is not enough: the DAEMON's
      # /etc/nix/nix.conf is what decides, so `extra-experimental-features` in
      # a flake's nixConfig or in NIX_CONFIG reads as accepted and still fails.
      # `$out` below is the builder's variable, not this shell's, so the
      # single quotes are the point.
      # shellcheck disable=SC2016
      if ! nix build --no-link --quiet --expr 'derivation {
             name = "ca-derivations-probe";
             system = "${system}";
             builder = "/bin/sh";
             args = [ "-c" "echo probe > $out" ];
             __contentAddressed = true;
             outputHashMode = "recursive";
             outputHashAlgo = "sha256";
           }'; then
        {
          echo "this nix store cannot build content-addressed derivations."
          echo "add to the DAEMON config (/etc/nix/nix.conf, then restart it):"
          echo "  extra-experimental-features = ca-derivations"
          echo "on GitHub Actions that is the nix-installer-action 'extra-conf' input."
        } >&2
        exit 1
      fi

      enforced=(${quote enforced})
      excluded=(${quote excludedNames})

      build() {
        nix build --accept-flake-config --no-link --print-build-logs \
          "$flake#checks.${system}.$1"
      }

      status=0
      failed=()

      # Every failure is reported rather than the first one only, so a single
      # push can fix all of them.
      for name in "''${enforced[@]}"; do
        if build "$name"; then
          echo "ok        $name"
        else
          echo "FAILED    $name"
          failed+=("$name")
          status=1
        fi
      done

      for name in "''${excluded[@]}"; do
        if build "$name"; then
          echo "STALE     $name"
          {
            echo "checks.${system}.$name is excluded from CI but now builds."
            echo "Delete it from nix/ci/flake-gate.nix; CI enforces it after that."
          } >&2
          status=1
        else
          echo "excluded  $name (still failing, as nix/ci/flake-gate.nix records)"
        fi
      done

      if [ "''${#failed[@]}" -gt 0 ]; then
        {
          echo ""
          echo "enforced checks that failed: ''${failed[*]}"
          echo "reproduce one with: nix build .#checks.${system}.<name> -L"
        } >&2
      fi

      exit "$status"
    '';
  };
in
lib.throwIf (stale != [ ]) ''
  nix/ci/flake-gate.nix excludes checks that do not exist: ${lib.concatStringsSep ", " stale}
  Delete them, or spell them the way `checks.${system}` does.
''
  gate
