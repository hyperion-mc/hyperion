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
  # Empty, and that is the interesting part. ENG-10817 exempted the whole set
  # on the belief that most of it could not pass. Measured per check on
  # ubuntu-latest instead of as one all-or-nothing job (run 30341722090, 41
  # checks), 38 passed and the 3 that failed were the sandboxed e2e gates,
  # failing on a missing trust store rather than on the runner. nix/e2e.nix
  # names the CA bundle now, so nothing is left to exempt.
  #
  # `differential-traces` is the one worth remembering: it failed on run
  # 30341066882 and then passed four times (30341722090, plus three
  # independent repeats in 30342250503). The only difference was whether the
  # daemon could realise a content-addressed derivation, so it was the store
  # and not the check, and it is enforced.
  #
  # An entry here is `name = <the evidence>`, never a justification. The gate
  # prints that text, builds the check anyway, and fails if it passes, so an
  # exception cannot outlive the defect it was written for. Anything you cannot
  # write as an observation carrying a date and a run id does not belong here.
  # Fix the check or delete it instead.
  excluded = { };

  excludedNames = lib.attrNames excluded;

  # A renamed or deleted check must not leave a dead exclusion behind, reading
  # as though it exempts something while exempting nothing.
  stale = lib.subtractLists names excludedNames;

  enforced = lib.subtractLists excludedNames names;

  quote = xs: lib.concatMapStringsSep " " lib.escapeShellArg xs;

  # The recorded evidence is printed by the gate, not left in this file for
  # someone to go and read. An exception nobody sees is an exception nobody
  # revisits.
  reasonArms = lib.concatStrings (
    lib.mapAttrsToList (
      name: why: "${lib.escapeShellArg name}) printf '%s' ${lib.escapeShellArg why} ;;\n    "
    ) excluded
  );

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

      reason() {
        case "$1" in
          ${reasonArms}esac
      }

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
            echo "nix/ci/flake-gate.nix excluded it on this evidence:"
            reason "$name"
            echo "Delete the entry; CI enforces the check from then on."
          } >&2
          status=1
        else
          echo "excluded  $name (still failing, on the evidence recorded for it)"
          reason "$name"
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
