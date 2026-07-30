# Which flake checks CI enforces, and how a pull request is judged against them.
#
# Enforcement is subtractive: every attribute of `checks` is built and must
# pass. A check added tomorrow is enforced the day it lands rather than the day
# someone remembers to add it here, which is the failure this file exists to
# prevent. See ENG-10817: the job that would have built these sat behind
# `continue-on-error: true`, so every gate landed in that window was believed to
# be running when it was not.
#
# THERE IS NO EXCLUSION LIST ANY MORE, and that is deliberate. This file used to
# carry an `excluded` attrset of known-broken checks with a recorded reason, and
# rebuilt each one to require that it STILL FAIL. The idea was right and the
# storage was wrong: a list in the repository is refreshed by human attention,
# so it goes stale on a schedule nobody sets, and every repair is a merge
# conflict against every other repair in flight.
#
# "This check is known broken" now lives in the baseline, which main publishes
# on every run and which therefore cannot go stale, and a reason belongs in the
# standing issue where a reader will actually meet it. If you find yourself
# wanting an exception list back, what you want is either the baseline or, for a
# check that genuinely cannot run in CI on this system, leaving the attribute
# out of `checks` in flake.nix. That is a truer home for "this cannot run here"
# than a CI-side exception.
#
# The verdict is DIFFERENTIAL: a pull request is judged on whether it makes the
# failing set worse, not on whether the set is empty. Two correct pull requests
# on 2026-07-29 each displayed exactly the failures the other one repairs, so
# neither could be green and neither could land. The rule, the soundness
# conditions and the reason the check attribute rather than the derivation hash
# is the identity all live in nix/ci/delta-gate.sh, whose pure half
# nix/ci/delta-gate-tests.sh exercises without a network or a nix store.
#
# ---------------------------------------------------------------------------
# THIS VERDICT BLOCKS NOTHING, AND WHAT WOULD HAVE TO BE TRUE BEFORE IT DOES
# ---------------------------------------------------------------------------
#
# Ruleset 566717 on `main` carries no `required_status_checks` rule at all, it
# is the only ruleset, and `branches/main/protection` returns 404. So one
# approving review is the whole gate, red pipeline or no pipeline (ENG-10827).
# Every trigger is `workflow_dispatch` besides, as of #1088.
#
# Making this binding is ONE `required_status_checks` entry naming the `Flake`
# job. It is written down here rather than only in the run summary because the
# summary is only read when the pipeline runs, and the pipeline is manual, so
# the header is the only place a person actually meets this.
#
# Four things should be true first. None of them was on 2026-07-30, and each is
# a measurement rather than a judgement:
#
#   1. FLAKE RATE at or under 5% over 20 consecutive runs. Measured 0.611 over
#      the 18 main runs of 2026-07-28/29: three checks fail at random
#      (smash-hud-e2e 44%, differential-traces 39%, bedwars-bow-e2e 11%) and 11
#      of 18 runs hit at least one, so a correct change needed 2.57 attempts.
#      5% is not arbitrary: the merge_queue rule sets `grouping_strategy:
#      ALLGREEN` with `max_entries_to_build: 5`, so a flake ejects a batch of up
#      to five pull requests rather than costing one author a re-run, and the
#      bisection retries carry the same rate. `delta-gate.sh flake-rate` reports
#      this against the instability record.
#
#   2. AT LEAST 30 RUNS in the instability record. P(a check flaking at rate q
#      is proven within n same-derivation samples) is 1 - q^n - (1-q)^n. At
#      n=18 an 11% flake is still 12% likely to be unproven; n=30 puts anything
#      at or above 10% over 97% likely to be proven. Below 5%, 30 is not enough
#      and no claim should be made that it is.
#
#   3. AT MOST 1 IN 20 pull request runs blocked by a verdict that a re-run then
#      clears, over at least 20 pull request runs. This is the only one of the
#      four that measures the whole system rather than a part, and it can only
#      be taken while the verdict is reporting rather than blocking.
#
#   4. p95 WALL TIME UNDER 40 MINUTES, measured over runs in which every check
#      reached a verdict. Stated that way deliberately: the observed 31 to 41
#      minute range is over RED runs only and no green run of this gate has ever
#      been observed, so the green cost is unknown in both magnitude and
#      direction. Satisfying this may need the concurrent gate first, simply to
#      have green runs to measure. The ceiling is the merge queue's
#      `check_response_timeout_minutes: 60`, which is also why `timeout-minutes`
#      in ci.yml must stay strictly below it.
#
# The measurement, the method and the design in full are ENG-11433.
{
  lib,
  writeShellApplication,
  system,
  bash,
  jq,
  # Every name in `checks`, rather than the derivations: the gate shells out to
  # `nix build .#checks.<system>.<name>` and needs no more than the name.
  names,
}:
let
  # The results file is JSON assembled with printf rather than jq, so a name
  # has to be a string that survives being dropped between two quotes. Every
  # attribute in `checks` is a plain identifier today, and this is what stops
  # the day one is not from producing a results file that parses as something
  # else, or does not parse at all.
  unquotable = lib.filter (name: builtins.match "[A-Za-z0-9._-]+" name == null) names;

  quote = xs: lib.concatMapStringsSep " " lib.escapeShellArg xs;

  deltaGate = ./delta-gate.sh;

  gate = writeShellApplication {
    name = "flake-gate";
    runtimeInputs = [
      bash
      jq
    ];
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

      enforced=(${quote names})

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
      for name in "''${enforced[@]}"; do
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
      for name in "''${enforced[@]}"; do
        installables+=("$flake#checks.${system}.$name")
      done

      # `--print-build-logs` prefixes every line with the derivation that
      # emitted it, which is what keeps interleaved logs attributable. The
      # prefix is the derivation name and not the check name, so `smash-e2e`
      # reads as `hyperion-smash-e2e`.
      #
      # The exit status is discarded on purpose: it says only that something
      # failed, and which checks failed is the question the loops below answer
      # per name.
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
      # Where this run's documents land. CI points HYPERION_GATE_OUT_DIR at a
      # directory it uploads; a local run gets a temporary one and the same
      # files. FLAKE_GATE_RESULTS still overrides the results path alone, so
      # anything already pointing at it keeps working.
      out="''${HYPERION_GATE_OUT_DIR:-$(mktemp -d -t flake-gate.XXXXXX)}"
      mkdir -p "$out"
      results="''${FLAKE_GATE_RESULTS:-$out/results.jsonl}"
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

      echo ""
      echo "$results holds one json line per check: attr, outcome, drvPath."
      # The gate's own wall clock, so a run says what it cost without anyone
      # having to subtract two timestamps out of the job log. This is the
      # number the change to one concurrent build was made to move.
      echo "gate: ''${#enforced[@]} checks in ''${SECONDS}s"

      if [ "''${#failed[@]}" -gt 0 ]; then
        {
          echo ""
          echo "checks that failed: ''${failed[*]}"
          echo "reproduce one with: nix build .#checks.${system}.<name> -L"
        } >&2
      fi

      # ---- the differential verdict -------------------------------------
      #
      # Everything below reads the results file and nothing else, so how the
      # checks were built is not its business. That is the seam: the loop above
      # went from one `nix build` per name to one concurrent build for the whole
      # set without any of this noticing.

      jq -s \
        --arg system "${system}" \
        --arg commit "''${GITHUB_SHA:-$(git -C "$flake" rev-parse HEAD 2>/dev/null || echo unknown)}" \
        --arg runId "''${GITHUB_RUN_ID:-local}" \
        --arg event "''${GITHUB_EVENT_NAME:-local}" \
        --arg recordedAt "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" \
        --argjson recordedAtEpoch "$(date +%s)" \
        '{schemaVersion: 1, system: $system, commit: $commit, runId: $runId,
          event: $event, recordedAt: $recordedAt, recordedAtEpoch: $recordedAtEpoch,
          evalFailed: false, checks: (. | sort_by(.attr))}' \
        "$results" >"$out/results.json"

      # Folded on every run including a green one. A run that agrees with the
      # last is not a wasted sample: the record needs agreements for the
      # eventual disagreement to mean anything.
      ${bash}/bin/bash ${deltaGate} fold \
        "''${HYPERION_GATE_INSTABILITY:-}" "$out/results.json" >"$out/instability.json"
      ${bash}/bin/bash ${deltaGate} flake-rate "$out/instability.json" >"$out/flake-rate.json"

      # A baseline is admitted or it is not, and there is no middle. When it is
      # not, nothing is excused and the gate behaves exactly as it did before
      # any of this existed, which is the fail-closed direction.
      baseline=""
      reject="no baseline was supplied to this run"
      if [ -n "''${HYPERION_GATE_BASELINE:-}" ] && [ -s "''${HYPERION_GATE_BASELINE}" ]; then
        if ${bash}/bin/bash ${deltaGate} admissible "''${HYPERION_GATE_BASELINE}" \
          "${system}" "''${HYPERION_GATE_BASELINE_MAX_AGE_HOURS:-48}"; then
          baseline="''${HYPERION_GATE_BASELINE}"
          reject=""
        else
          reject="the published baseline was not admissible; see the delta-gate lines above"
        fi
      fi

      forfeit=""
      if [ -n "''${HYPERION_GATE_CHANGED_FILES:-}" ] && [ -s "''${HYPERION_GATE_CHANGED_FILES}" ]; then
        forfeit="$(${bash}/bin/bash ${deltaGate} forfeit-reason "''${HYPERION_GATE_CHANGED_FILES}")"
      fi

      ${bash}/bin/bash ${deltaGate} verdict \
        "$out/results.json" "$baseline" "$out/instability.json" "$reject" "$forfeit" \
        >"$out/verdict.json"
      ${bash}/bin/bash ${deltaGate} summary "$out/verdict.json" >"$out/summary.md"

      printf '::group::Differential CI verdict\n'
      cat "$out/summary.md"
      printf '::endgroup::\n'
      if [ -n "''${GITHUB_STEP_SUMMARY:-}" ]; then
        printf '\n' >>"$GITHUB_STEP_SUMMARY"
        cat "$out/summary.md" >>"$GITHUB_STEP_SUMMARY"
      fi

      # The verdict decides, not the raw status. A run whose every failure also
      # fails on the base has made nothing worse, and saying otherwise is the
      # deadlock this exists to end. With no baseline the two agree, so a local
      # `nix run .#flake-gate` behaves exactly as it always has.
      if [ "$(jq -r '.gatePasses' "$out/verdict.json")" = "true" ]; then
        exit 0
      fi
      [ "$status" -eq 0 ] && status=1
      exit "$status"
    '';
  };
in
lib.throwIf (unquotable != [ ]) ''
  nix/ci/flake-gate.nix writes each check's verdict into a json results file
  with printf, which holds only for names that need no json escaping. These
  do: ${lib.concatStringsSep ", " unquotable}
  Rename them, or teach the gate to escape a name before it writes one.
'' gate
