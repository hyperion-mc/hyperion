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

  # The results file is JSON assembled with printf rather than jq, so a name
  # has to be a string that survives being dropped between two quotes. Every
  # attribute in `checks` is a plain identifier today, and this is what stops
  # the day one is not from producing a results file that parses as something
  # else, or does not parse at all.
  unquotable = lib.filter (name: builtins.match "[A-Za-z0-9._-]+" name == null) names;

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

      # Every check's derivation path, in one flake load.
      #
      # A derivation path is known at evaluation time even where an output path
      # is not, so this is the one identity the gate can state for every check
      # including the deferred ones. It goes in the results file below, where a
      # consumer comparing a pull request against main uses it to tell a flaky
      # check from a broken one: the same derivation with two different
      # outcomes is proof the change did not cause the failure.
      #
      # One `--apply` over the whole set rather than one `nix eval` per name.
      # Both would work while the flake reference is locked and the evaluation
      # cache is warm -- a per-name `nix eval` of an already-built attribute
      # measured 0.26 s -- but that is a property of the checkout being clean,
      # and 67 full evaluations is what it costs when it is not. One load costs
      # one load either way.
      drv_map="$(mktemp -t flake-gate-drvs.XXXXXX)"
      trap 'rm -f "$drv_map"' EXIT
      nix eval --accept-flake-config --raw "$flake#checks.${system}" --apply '
        checks:
        builtins.concatStringsSep "\n" (
          builtins.attrValues (builtins.mapAttrs (name: check: name + " " + check.drvPath) checks)
        )
      ' > "$drv_map"

      declare -A drv_path=()
      # `--raw` writes no trailing newline, so the last pair arrives with `read`
      # already at end of file and would be dropped by the loop condition alone.
      while read -r name path || [ -n "$name" ]; do
        [ -n "$name" ] || continue
        drv_path["$name"]="$path"
      done < "$drv_map"

      # A name the gate was built with that evaluation does not produce would
      # otherwise report as a failed check, which is a true statement about
      # nothing. Say which name instead.
      unevaluated=()
      for name in "''${enforced[@]}" "''${excluded[@]}"; do
        [ -n "''${drv_path[$name]:-}" ] || unevaluated+=("$name")
      done
      if [ "''${#unevaluated[@]}" -gt 0 ]; then
        {
          echo "checks.${system} did not evaluate: ''${unevaluated[*]}"
          echo "the gate was built against a different set of names than the flake has."
        } >&2
        exit 1
      fi

      # One `nix build` for the whole set, rather than one per name.
      #
      # The checks ran strictly one after another and nothing about a check
      # needs the machine to itself, so all the serialisation bought was wall
      # clock. Measured on GitHub Actions run 30500640846: 67 checks, 2102
      # seconds, of which `smash-e2e` was 639 and `bedwars-bow-e2e` was 546.
      # The other 65 queued behind those two, and 55 of the 67 were under 20
      # seconds each.
      #
      # Concurrency is nix's own `max-jobs` and is deliberately not pinned
      # here. `nix-installer-action` writes `max-jobs = auto`, so on the hosted
      # runner it is the core count -- four, which the hud gate reads back off
      # the same machine as `CPU 2% of 400%`. A number written into this script
      # would be that machine's number on everybody else's. It also does not
      # raise the ceiling: one `nix build` of one e2e check already scheduled
      # its crate units four wide, so four is what the compile phase ran at
      # before this. What changes is that whole checks overlap, and an e2e gate
      # spends its time waiting on a server to boot rather than on a core.
      #
      # `--keep-going` is load bearing. Without it the first failed check
      # abandons the rest, and every name after it would report as failed
      # having never been attempted -- the opposite of the complete report the
      # loops below exist to produce.
      #
      # The cost, named: one evaluation covers every check, so an evaluation
      # error anywhere in `checks` now stops the gate at the `nix eval` above
      # with nix's own message, rather than failing one name and passing the
      # other 66. The report is not printed at all in that case, which is the
      # honest answer: nothing was measured.
      installables=()
      for name in "''${enforced[@]}" "''${excluded[@]}"; do
        installables+=("$flake#checks.${system}.$name")
      done

      # `--print-build-logs` prefixes every line with the derivation that
      # emitted it, which is what keeps interleaved logs attributable. The
      # prefix is the derivation name and not the check name, so `smash-e2e`
      # reads as `hyperion-smash-e2e`.
      #
      # The exit status is discarded on purpose: it says only that something
      # failed, and which checks failed is the question the loops below answer
      # per name. It is also the expected status whenever `excluded` is
      # non-empty.
      nix build --accept-flake-config --no-link --print-build-logs --keep-going \
        "''${installables[@]}" || true

      # Did this check pass? Asked of the store, never by building it again.
      #
      # `--max-jobs 0` forbids starting a local build, so this exits 0 exactly
      # when the output is already realised and fails otherwise; `--builders ""`
      # closes the same door for a contributor whose nix.conf names a remote
      # builder. A plain `nix build` per name would instead rebuild every check
      # that just failed, because a failed derivation is not in the store, and
      # the slowest thing in the job would be paid for twice.
      #
      # Asked of nix rather than by comparing `.outPath` against the store,
      # because cargoUnit content-addresses every crate unit and a derivation
      # downstream of a content-addressed one has a deferred output path.
      # `nix eval .#checks.${system}.smash-e2e.outPath` answers with a
      # placeholder rather than a store path for 14 of the 67 checks, and those
      # 14 are exactly the e2e gates. Given the derivation, nix resolves it
      # against the realisations it recorded and answers exactly.
      #
      # By derivation path rather than by flake attribute, so this loads no
      # flake at all: 67 of these cost nothing whatever state the evaluation
      # cache is in.
      #
      # Output is dropped because the only thing on the failing path is nix
      # explaining that it will not build with max-jobs 0, which is the answer
      # rather than a problem. The build above is where a check says why it
      # failed.
      realised() {
        nix build --no-link --max-jobs 0 --builders "" "$1^*" > /dev/null 2>&1
      }

      # A machine-readable verdict beside the human one, one line per check.
      #
      # It exists so that a consumer comparing this run against another can
      # difference the set of failing checks without scraping the log, which is
      # the only other place the answer lives. The name is the attribute, so a
      # reader never has to map `hyperion-smash-e2e` back to `smash-e2e`.
      #
      # No `seconds` field, and the reason is the change above rather than an
      # oversight: with every check realised by one concurrent `nix build`,
      # a per-check wall time is not a thing that exists. `smash-e2e` and
      # `bedwars-bow-e2e` overlap, both overlap the compile they depend on, and
      # any number attributed to one of them would be a share of a wall clock
      # they did not have to themselves. Recovering real per-check durations
      # means splitting the realisation into a dependency phase and a leaf
      # phase and timing the leaves, which is a different change with its own
      # failure modes. The gate's total is printed below and in the job log.
      #
      # Built with printf rather than jq: an attribute name is checked at
      # evaluation time (see `unquotable`) to need no JSON escaping, and a
      # store path cannot contain a quote or a backslash.
      results="''${FLAKE_GATE_RESULTS:-flake-gate-results.jsonl}"
      : > "$results"
      record() {
        printf '{"attr":"%s","outcome":"%s","drvPath":"%s","seconds":null}\n' \
          "$1" "$2" "$3" >> "$results"
      }

      status=0
      failed=()

      # Every failure is reported rather than the first one only, so a single
      # push can fix all of them.
      for name in "''${enforced[@]}"; do
        if realised "''${drv_path[$name]}"; then
          echo "ok        $name"
          record "$name" pass "''${drv_path[$name]}"
        else
          echo "FAILED    $name"
          record "$name" fail "''${drv_path[$name]}"
          failed+=("$name")
          status=1
        fi
      done

      # An excluded check is recorded by what it did, not by how the gate
      # treats it. A consumer differencing two runs wants the outcome; whether
      # this file forgives it is this file's business and changes under them.
      for name in "''${excluded[@]}"; do
        if realised "''${drv_path[$name]}"; then
          echo "STALE     $name"
          record "$name" pass "''${drv_path[$name]}"
          {
            echo "checks.${system}.$name is excluded from CI but now builds."
            echo "nix/ci/flake-gate.nix excluded it on this evidence:"
            reason "$name"
            echo "Delete the entry; CI enforces the check from then on."
          } >&2
          status=1
        else
          echo "excluded  $name (still failing, on the evidence recorded for it)"
          record "$name" fail "''${drv_path[$name]}"
          reason "$name"
        fi
      done

      echo ""
      echo "$results holds one json line per check: attr, outcome, drvPath."
      # The gate's own wall clock, so a run says what it cost without anyone
      # having to subtract two timestamps out of the job log. This is the
      # number the change to one concurrent build was made to move.
      echo "gate: ''${#enforced[@]} enforced and ''${#excluded[@]} excluded checks in ''${SECONDS}s"

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
  (lib.throwIf (unquotable != [ ]) ''
    nix/ci/flake-gate.nix writes each check's verdict into a json results file
    with printf, which holds only for names that need no json escaping. These
    do: ${lib.concatStringsSep ", " unquotable}
    Rename them, or teach the gate to escape a name before it writes one.
  '' gate)
