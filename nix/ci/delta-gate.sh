#!/usr/bin/env bash
# Differential CI gating: judge a pull request on the DELTA against the base
# rather than on absolute green.
#
# | check fails on the PR | fails on the base too | verdict                   |
# | --------------------- | --------------------- | ------------------------- |
# | yes                   | yes                   | pass, not this PR's fault |
# | yes                   | no                    | BLOCK, this PR's fault    |
# | no                    | yes                   | pass, this PR fixed it    |
# | no                    | no                    | pass                      |
#
# WHY THIS EXISTS. On 2026-07-29 two correct pull requests each displayed
# exactly the failures the other one fixes. 1080 fixed `completions-e2e` and
# `minecraft-literals` and its CI reported `bedwars-dev-boot-e2e`,
# `smash-dev-boot-e2e` and `smash-e2e`; 1081 fixed the two boot gates and its CI
# reported the other three. Neither could ever be green on its own, so neither
# could land, so the failures accumulated. An absolute verdict cannot express
# "better than before", which is the only question a reviewer actually has.
#
# The design is ix's, from `Differential CI verdict (ix#9053)`. It is vendored
# rather than imported because indexable-inc/ix is INTERNAL and this repository
# is PUBLIC, so a flake input pointing at it would break every outside
# contributor. This is a second copy and it will drift; when it does, ix's
# scripts/ci/delta-gate.sh is the elder.
#
# Two things here are NOT from ix, because ix does not have them.
#
#   The whole flakiness half. ix's gate calls a coin flip a regression and
#   blocks. That is survivable there and is not survivable here: three checks in
#   this suite are measured coin flips, 61% of runs hit at least one, and a
#   differential gate that falsely accuses is dead within a week.
#
#   Reporting `removed` apart from `fixed`. A check a pull request DELETED is
#   absent from its failing set for a reason that is not repair, and calling it
#   repaired credits the wrong thing. The check set here moved between 60 and 67
#   entries over eighteen consecutive runs, so this is not hypothetical.
#
# THE IDENTITY IS THE CHECK ATTRIBUTE, NOT THE DERIVATION HASH. A pull request
# that touches a shared crate moves every downstream drv hash, so a hash
# comparison would call all of them new and block on failures the PR did not
# cause. The hash answers a different and narrower question, below.
set -euo pipefail

readonly DG_SCHEMA_VERSION=1

# The results vocabulary is nix/ci/flake-gate.nix's: `outcome` is "pass" or
# "fail". Everything here tests for "fail" and treats anything else as passing,
# so a third spelling arriving one day reads as a pass rather than silently
# turning every check red.

# The most failing checks a base may carry before this gate refuses a pull
# request regardless of its delta.
#
# A differential gate makes red survivable, and survivable is comfortable. This
# is the number that stops "no worse than before" from being a licence for
# "before" to grow without limit. Six was chosen against a measured base of four
# deterministic failures, so there is room for a genuine two-cause day and not
# room for a habit.
#
# EDIT THIS DOWNWARD ONLY, and never from an observed count. A suite carrying
# proven coin flips will eventually have a lucky night, and a cap that lowered
# itself onto that night's number would freeze the repository on the next
# ordinary day. That is the failure mode this whole file exists to prevent, and
# self-inflicting it would be worse than not having a cap.
#
# A pull request that STRICTLY SHRINKS the failing set is exempt. Without that,
# tripping the cap would block the fixes that are the only way to untrip it,
# which is the original deadlock wearing a cap.
readonly DG_BASELINE_CAP=6

# How long an observation stays evidence about a check's determinism.
readonly DG_INSTABILITY_WINDOW_DAYS=30

# Bound on the per-derivation recency map, oldest dropped first.
readonly DG_INSTABILITY_MAX_ENTRIES=5000

# How many recent runs the measured flake rate is taken over. Exit criterion 1
# for making this verdict a required status context asks for 20.
readonly DG_FLAKE_RATE_RUNS=20

# Diagnostic sink. Defaults to stderr so a local run says what it is doing.
dg_log() { printf 'delta-gate: %s\n' "$*" >>"${DG_LOG_SINK:-/dev/stderr}"; }

# --- what the derivation hash is for ----------------------------------------
#
# A derivation is the complete specification of a build, and its path is a
# Merkle root over every input derivation. So an unchanged drvPath means an
# unchanged build closure, and:
#
#     SAME DERIVATION, DIFFERENT OUTCOME, IS A PROOF OF NONDETERMINISM.
#
# Not evidence, not a heuristic. If a check's drvPath is identical on the base
# and on the pull request, and it passed there and fails here, then the diff did
# not cause the failure. Something outside the derivation did: a timeout against
# a slower runner, an OOM, a sandbox leak. Which one does not matter for the
# verdict. The verdict-relevant claim is "this pull request did not cause it",
# and that claim is airtight.
#
# This is worth having because a retry cannot do the job. For a check that fails
# 30% of the time, two failures in a row happen 9% of the time, so a retry can
# acquit and can never convict. Any gate whose flake answer is a retry blocks
# honest work 9% of the time. A proof has no such rate.
#
# Measured here, on this repository, on 2026-07-29:
#
#   run 30428441399 (34eb46c, 06:31Z)  differential-traces  ok
#   run 30429463435 (dd8311f, 06:49Z)  differential-traces  FAILED
#   both on /nix/store/fczpzka3v7c9npf4a3i730ahkncla3f5-check-hyperion-differential-traces.drv
#
# Byte-identical derivation, opposite outcomes, eighteen minutes apart. The
# failure was `IllegalStateException: chunks were still not entity ticking after
# 600 ticks`, a wall-clock timeout inside the sandbox. Nothing in the derivation
# moved; the runner was slower.
#
# And the counter-example that fixes the resolution. `minecraft-literals`
# transitioned contiguously from ok to FAILED across runs 30417510835 and
# 30418671535, and each run builds two derivations sharing the name
# `check-minecraft-literals`:
#
#   ok    d48rbprp0l1q7ndhz0klp5qws89wbbg1 , 9md01i6lzfz65fxjk4ql9jkaykk9krsr
#   fail  d48rbprp0l1q7ndhz0klp5qws89wbbg1 , 3rwh12gki71qk23xczyjzdp8jcyjgjdf
#
# `d48rbprp` is common to both. Comparing "a derivation named after the check"
# and happening to pick that one would have excused a genuine regression as a
# flake. The one nix named in `Cannot build` is `3rwh12gk`, which is new.
#
# The resolution is to compare THE CHECK ATTRIBUTE'S OWN drvPath, from
# `nix eval`, and nothing else. Because a drv path is a Merkle root, the
# attribute's own path moves whenever anything it depends on moves, so it is
# both the coarsest comparison that is sound and the only one worth making.
# `d48rbprp` staying fixed while `3rwh12gk` moved is itself the proof that
# `d48rbprp` is not an ancestor of the failing derivation.

# --- guard: when the derivation does not describe the run --------------------

# Paths whose contents the derivation does not capture but the run depends on.
# A pull request touching any of them FORFEITS drv-identity immunity, because
# for such a change "identical derivation" no longer implies "this PR is not the
# cause".
#
# Three known members, and the general form matters more than the list because
# the list will grow:
#
#   ENVIRONMENT. `.github/workflows/**` picks the runner and the timeouts;
#   flake.nix's `nixConfig` picks the substituters. Same derivation, different
#   world, and the pull request is why.
#
#   SCHEDULING and CONCURRENCY. These e2e checks start real servers on real
#   ports. `nix/ci/flake-gate.nix` decides whether checks run one at a time or
#   all at once, and the e2e port offsets decide whether two of them collide.
#   A change there can break a check whose derivation is byte identical, and the
#   cause genuinely is the change. The branch that makes this gate concurrent is
#   exactly such a pull request and is the first one this will meet.
#
# The general form, for whoever adds the fourth: a pull request forfeits
# immunity when it changes anything the derivation does not capture but the run
# depends on.
dg_immunity_forfeit_reason() {
  local changed_files=$1
  [[ -s ${changed_files} ]] || return 0
  local hit
  hit="$(grep -aE '^(\.github/workflows/|nix/ci/flake-gate\.nix$|nix/ci/delta-gate\.sh$)' \
    "${changed_files}" || true)"
  if [[ -z ${hit} ]]; then
    hit="$(grep -aE 'e2e[Oo]ffsets|ports-distinct' "${changed_files}" || true)"
  fi
  [[ -z ${hit} ]] && return 0
  printf 'this pull request changes %s, which the derivation does not capture but the run depends on' \
    "$(tr '\n' ' ' <<<"${hit}" | sed 's/ $//')"
}

# --- instability: which checks are proven nondeterministic -------------------

# Fold one run's results into the instability record, and promote any check
# attribute whose derivation has now been seen both passing and failing.
#
#   $1 the existing record, or "" on the first run
#   $2 this run's results document
#
# Keyed on drvPath for the PROOF and promoted onto the ATTRIBUTE for the
# CONCLUSION, and those are two different things on purpose. The proof has to be
# per-derivation, because that is the only comparison that means anything. The
# conclusion has to be per-attribute, because nondeterminism is a property of
# how a check is written and it survives a change to that check's inputs.
# Without the promotion the record would go silent for exactly the pull requests
# that move a drv hash, which is most of them.
#
# The cost of the promotion, stated plainly: one infrastructure failure (an OOM,
# a bad NAR) against an otherwise deterministic check marks that check unstable
# for the length of the window, and the gate stops enforcing it. That is a real
# hole. The mitigations are visibility rather than cleverness: the window
# expires, every grant of immunity prints the two run ids that bought it, and
# the standing issue names every unstable check with the date it was proven.
dg_merge_instability() {
  local record=${1:-} results=$2
  local record_arg=/dev/null
  [[ -n ${record} && -s ${record} ]] && record_arg=${record}

  jq -n \
    --slurpfile old "${record_arg}" \
    --slurpfile run "${results}" \
    --argjson windowDays "${DG_INSTABILITY_WINDOW_DAYS}" \
    --argjson maxEntries "${DG_INSTABILITY_MAX_ENTRIES}" \
    --argjson rateRuns "${DG_FLAKE_RATE_RUNS}" \
    --argjson nowEpoch "$(date +%s)" '
    ($old[0] // {}) as $o
    | ($run[0] // {}) as $r
    | ($nowEpoch - ($windowDays * 86400)) as $cutoff
    | ($r.runId // "unknown") as $runId
    | (($o.seen // {}) | with_entries(select(.value.lastEpoch >= $cutoff))) as $aged
    | reduce (($r.checks // [])[] | select(.drvPath != null and .drvPath != "")) as $c
        ($aged;
          .[$c.drvPath] as $p
          | .[$c.drvPath] = {
              attr: $c.attr,
              ok:   (($p.ok   // false) or ($c.outcome != "fail")),
              fail: (($p.fail // false) or ($c.outcome == "fail")),
              okRunId:   (if $c.outcome != "fail" then $runId else ($p.okRunId   // null) end),
              failRunId: (if $c.outcome == "fail" then $runId else ($p.failRunId // null) end),
              lastEpoch: $nowEpoch
            })
    # Oldest-first eviction, so reaching the bound on a busy day cannot silently
    # discard the disagreement that matters.
    | ([to_entries[]] | sort_by(-.value.lastEpoch) | .[0:$maxEntries] | from_entries) as $seen
    | [$seen | to_entries[] | select(.value.ok and .value.fail)
       | {attr: .value.attr, drvPath: .key, okRunId: .value.okRunId,
          failRunId: .value.failRunId, provenAtEpoch: .value.lastEpoch}] as $fresh
    # One row per run, so the flake rate is measured rather than assumed. Which
    # attrs failed is recorded raw; which of them count as unstable is decided
    # at read time against the CURRENT proof set, so a check proven flaky
    # tomorrow is credited in the runs already recorded.
    | ([{runId: $runId, epoch: $nowEpoch,
         failed: [($r.checks // [])[] | select(.outcome == "fail") | .attr] | sort}]
       + [($o.runs // [])[] | select(.epoch >= $cutoff)]
       | sort_by(-.epoch) | .[0:($rateRuns * 3)]) as $runs
    | {
        schemaVersion: '"${DG_SCHEMA_VERSION}"',
        windowDays: $windowDays,
        updatedAt: ($r.recordedAt // "unknown"),
        updatedAtEpoch: $nowEpoch,
        runsFolded: (($o.runsFolded // 0) + 1),
        seen: $seen,
        runs: $runs,
        # A proof keeps its ORIGINAL date through later folds, so the standing
        # issue can show how long a check has gone unenforced rather than
        # resetting the clock every night.
        proven: ([($o.proven // [])[] | select(.provenAtEpoch >= $cutoff)] + $fresh
                 | group_by(.attr) | map(sort_by(.provenAtEpoch) | .[0]) | sort_by(.attr))
      }
  '
}

# The check attributes proven nondeterministic, one per line.
dg_unstable_attrs() {
  local record=${1:-}
  [[ -n ${record} && -s ${record} ]] || return 0
  jq -r '(.proven // [])[] | .attr' "${record}" 2>/dev/null | sort -u
}

# The measured share of recent runs in which at least one PROVEN-UNSTABLE check
# failed, plus the sample size. This is exit criterion 1 for turning the verdict
# into a required status context: 0.61 measured over the 18 main runs of
# 2026-07-28/29, and the criterion is 0.05.
#
# Reported as a document rather than a bare number because a rate without its
# sample size is not a measurement, and because the criterion needs both.
dg_flake_rate() {
  local record=${1:-}
  local arg=/dev/null
  [[ -n ${record} && -s ${record} ]] && arg=${record}
  jq -n --slurpfile r "${arg}" --argjson rateRuns "${DG_FLAKE_RATE_RUNS}" '
    ($r[0] // {}) as $rec
    | [($rec.proven // [])[] | .attr] as $unstable
    | ([($rec.runs // [])[]] | sort_by(-.epoch) | .[0:$rateRuns]) as $runs
    | ($runs | length) as $n
    | ([$runs[] | select([.failed[] | select(IN($unstable[]))] | length > 0)] | length) as $hit
    | {
        runs: $n,
        runsWithAFlake: $hit,
        rate: (if $n == 0 then null else (($hit / $n) * 1000 | round) / 1000 end),
        unstableChecks: ($unstable | sort),
        criterion: 0.05,
        meetsCriterion: (if $n < $rateRuns then false
                         else (($hit / $n) <= 0.05) end),
        why: (if $n < $rateRuns
              then "only \($n) of the \($rateRuns) runs the criterion asks for have been folded"
              else "measured over the last \($n) runs" end)
      }
  '
}

# --- baseline admission ------------------------------------------------------

# Structural admission of a published baseline: schema, system, freshness.
# ANCESTRY IS NOT CHECKED HERE, because it needs the API; the caller does it.
# Every path fails closed. A baseline nobody can read must excuse nothing, which
# is exactly how this gate behaved before the feature existed.
dg_baseline_admissible() {
  local baseline=$1 system=$2 max_age_hours=${3:-48}
  local schema doc_system recorded_at age_hours

  if [[ ! -s ${baseline} ]]; then
    dg_log "baseline ${baseline} is missing or empty"
    return 1
  fi
  if ! jq -e 'type == "object"' "${baseline}" >/dev/null 2>&1; then
    dg_log "baseline ${baseline} is not a JSON object"
    return 1
  fi
  schema="$(jq -r '.schemaVersion // "missing"' "${baseline}")"
  if [[ ${schema} != "${DG_SCHEMA_VERSION}" ]]; then
    dg_log "baseline schemaVersion is ${schema}, this gate speaks ${DG_SCHEMA_VERSION}"
    return 1
  fi
  doc_system="$(jq -r '.system // ""' "${baseline}")"
  if [[ ${doc_system} != "${system}" ]]; then
    dg_log "baseline is for system '${doc_system}', this run is '${system}'"
    return 1
  fi
  # A run whose evaluation failed produced no per-check identity at all, so it
  # says nothing about which checks fail and must never excuse anything.
  if [[ "$(jq -r '.evalFailed // false' "${baseline}")" == "true" ]]; then
    dg_log "the baseline run could not evaluate the flake, so it names no checks"
    return 1
  fi
  if [[ "$(jq -r '(.checks // []) | length' "${baseline}")" == "0" ]]; then
    dg_log "the baseline names no checks at all"
    return 1
  fi
  recorded_at="$(jq -r '.recordedAtEpoch // empty' "${baseline}")"
  if [[ -z ${recorded_at} || ! ${recorded_at} =~ ^[0-9]+$ ]]; then
    dg_log "baseline carries no usable recordedAtEpoch"
    return 1
  fi
  age_hours=$((($(date +%s) - recorded_at) / 3600))
  if ((age_hours > max_age_hours)); then
    # Not pedantry. The ancestry check cannot catch a check that main FIXED
    # after the baseline was taken; only freshness can. Without a limit this
    # gate would excuse a pull request for re-breaking something main repaired.
    dg_log "baseline is ${age_hours}h old (limit ${max_age_hours}h), too stale to describe the base"
    return 1
  fi
  return 0
}

# --- the verdict -------------------------------------------------------------

#   $1 this run's results document
#   $2 the baseline document, or "" for "no usable baseline"
#   $3 the instability record, or "" for "nothing is proven unstable yet"
#   $4 why the baseline was rejected, rendered when $2 is absent
#   $5 why drv-identity immunity is forfeited, or "" when it is not
#
# `gatePasses` is the single field the gate keys off.
dg_verdict() {
  local pr_doc=$1 baseline_doc=${2:-} instability=${3:-} reject=${4:-} forfeit=${5:-}
  local base_arg=/dev/null inst_arg=/dev/null have_baseline=false
  if [[ -n ${baseline_doc} && -s ${baseline_doc} ]]; then
    base_arg=${baseline_doc}
    have_baseline=true
  fi
  [[ -n ${instability} && -s ${instability} ]] && inst_arg=${instability}

  jq -n \
    --slurpfile pr "${pr_doc}" \
    --slurpfile base "${base_arg}" \
    --slurpfile inst "${inst_arg}" \
    --argjson haveBaseline "${have_baseline}" \
    --argjson cap "${DG_BASELINE_CAP}" \
    --arg reject "${reject}" \
    --arg forfeit "${forfeit}" '
    ($pr[0] // {}) as $p
    | (if $haveBaseline then ($base[0] // {}) else null end) as $b
    | ($inst[0] // {}) as $i
    | ($forfeit | length > 0) as $forfeited
    | (($p.checks // [])) as $prAll
    | (if $b == null then [] else ($b.checks // []) end) as $baseAll
    | [$prAll[]   | select(.outcome == "fail")] as $prFail
    | [$baseAll[] | select(.outcome == "fail")] as $baseFail
    | ($prAll   | map(.attr)) as $prAttrs
    | ($prFail  | map(.attr)) as $prFailAttrs
    | ($baseFail | map(.attr)) as $baseFailAttrs
    | [($i.proven // [])[] | .attr] as $unstableAttrs
    | ($i.proven // []) as $proofs

    | [$prFail[] | select(.attr | IN($baseFailAttrs[]))] as $excusedRaw
    | [$prFail[] | select((.attr | IN($baseFailAttrs[])) | not)] as $newFail

    # A failure the base does not have is this pull request s to answer for,
    # UNLESS the derivation proves it could not be. Two proofs, both exact.
    # A pull request that ADDS OR REMOVES a check changes what else is running
    # beside every other check, and the derivation of those others does not
    # record that. Under a concurrent gate that is contention, and contention is
    # how an e2e check that starts a real server on a real port times out
    # without anything it depends on having moved. So a changed check set
    # withdraws the same-derivation excuse.
    #
    # Detected from the two documents rather than from a path list, which is
    # both more general and impossible to forget: any edit that changes the set
    # shows up here whatever file it lived in.
    #
    # It withdraws the SAME-DRV excuse only, not the proven-nondeterministic
    # one. Those claim different things. Same-drv claims "everything else about
    # this run is equal", which a changed check set contradicts outright.
    # Proven-nondeterministic claims "this check is a coin flip", which stays
    # true under contention: blocking on it would accuse falsely at the base rate
    # that check already has, which is the thing this gate exists not to do.
    #
    # NOTE, and this has now cost two debugging rounds: an apostrophe anywhere in
    # a jq program below ENDS THE SHELL STRING. Keep these comments free of them.
    | (($prAttrs | sort) != ($baseAll | map(.attr) | sort)) as $checkSetChanged
    | [$newFail[]
       | . as $c
       | ($baseAll[] | select(.attr == $c.attr)) as $m
       | (($c.drvPath != null) and ($c.drvPath != "")
          and ($m.drvPath == $c.drvPath) and ($checkSetChanged | not)) as $sameDrv
       | ($c.attr | IN($unstableAttrs[])) as $proven
       | select(($forfeited | not) and ($sameDrv or $proven))
       | {attr: $c.attr, drvPath: $c.drvPath,
          evidence: (if $sameDrv then "identical-drv-passed-on-base" else "proven-nondeterministic" end),
          proof: (if $sameDrv
                  then {baseRunId: ($b.runId // null), prRunId: ($p.runId // null), drvPath: $c.drvPath}
                  else (($proofs[] | select(.attr == $c.attr))
                        | {okRunId, failRunId, drvPath, provenAtEpoch}) end)}]
      as $unstable
    | ($unstable | map(.attr)) as $unstableHere
    | [$newFail[] | select((.attr | IN($unstableHere[])) | not)] as $blocked

    # A check the pull request DELETED is not a check it repaired.
    | [$baseFail[] | select((.attr | IN($prFailAttrs[])) | not)
                   | select(.attr | IN($prAttrs[]))] as $fixed
    | [$baseFail[] | select((.attr | IN($prAttrs[])) | not)] as $removed

    | ($baseFail | length) as $baseCount
    | ($prFail | length) as $prCount
    | ($prCount < $baseCount) as $shrinks
    | (($baseCount > $cap) and ($shrinks | not)) as $capBlocks
    | ($p.evalFailed == true) as $evalFailed

    | {
        schemaVersion: '"${DG_SCHEMA_VERSION}"',
        system: ($p.system // null),
        prCommit: ($p.commit // null),
        prRunId: ($p.runId // null),
        evalFailed: $evalFailed,
        immunityForfeited: $forfeited,
        forfeitReason: (if $forfeited then $forfeit else null end),
        cap: $cap,
        checkSetChanged: $checkSetChanged,
        baselineFailingCount: $baseCount,
        prFailingCount: $prCount,
        shrinksFailingSet: $shrinks,
        capExceeded: ($baseCount > $cap),
        capBlocks: $capBlocks,
        baseline: (if $b == null
                   then {accepted: false, rejectedBecause: $reject}
                   else {accepted: true, commit: ($b.commit // null), runId: ($b.runId // null),
                         recordedAt: ($b.recordedAt // null), failingCount: $baseCount}
                   end),
        instability: {runsFolded: ($i.runsFolded // 0), provenUnstable: ($unstableAttrs | sort)},
        excused: [$excusedRaw[] | . as $e | ($baseFail[] | select(.attr == $e.attr)) as $m
                  | {attr: $e.attr, prDrvPath: ($e.drvPath // null), baseDrvPath: ($m.drvPath // null),
                     evidence: (if ($e.drvPath != null) and ($e.drvPath == $m.drvPath)
                                then "identical-drv" else "same-attr" end)}] | sort_by(.attr),
        unstable: ($unstable | sort_by(.attr)),
        blocked: [$blocked[] | {attr, drvPath: (.drvPath // null)}] | sort_by(.attr),
        fixed:   [$fixed[]   | {attr, baseDrvPath: (.drvPath // null)}] | sort_by(.attr),
        removed: [$removed[] | {attr}] | sort_by(.attr)
      }
    | .gatePasses = (((.blocked | length) == 0) and (.capBlocks | not) and (.evalFailed | not))
    | .verdict = (if .evalFailed then "eval-failed"
                  elif (.blocked | length) > 0 then "block"
                  elif .capBlocks then "cap"
                  elif (.excused | length) > 0 or (.unstable | length) > 0 then "excused"
                  else "pass" end)
    | .reason = (if .evalFailed
                 then "the flake did not evaluate, so no check has an identity and nothing can be compared"
                 elif (.blocked | length) > 0
                 then "this pull request introduces \(.blocked | length) failure(s) that do not fail on the base"
                 elif .capBlocks
                 then "the base carries \(.baselineFailingCount) failing checks, over the cap of \(.cap), and this pull request does not shrink the set"
                 elif (.unstable | length) > 0 and (.excused | length) > 0
                 then "\(.excused | length) failure(s) also fail on the base and \(.unstable | length) are proven nondeterministic, so this pull request makes nothing worse"
                 elif (.unstable | length) > 0
                 then "\(.unstable | length) failure(s) are proven nondeterministic and not attributable to this pull request"
                 elif (.excused | length) > 0
                 then "\(.excused | length) failure(s) also fail on the base, so this pull request makes nothing worse"
                 elif (.fixed | length) > 0
                 then "no failures here, and this pull request repairs \(.fixed | length) check(s) that fail on the base"
                 else "no failures" end)
  '
}

# --- the summary -------------------------------------------------------------

# Render the verdict as markdown for $GITHUB_STEP_SUMMARY and the job log.
#
# This is not decoration. Differential gating changes what a green tick MEANS,
# from "this is shippable" to "this is no worse than the base", and a reader of
# a tick cannot be expected to know which one is in force. So the first line
# always states it, on every run, including the boring ones.
dg_render_summary() {
  local verdict_doc=$1
  jq -r '
    def drv($d): if $d == null then "-" else "`\($d)`" end;
    ["## Flake gate: differential verdict", ""]
    + (if .verdict == "eval-failed" then
         ["**BLOCKED: the flake did not evaluate.**", "",
          "No check has an identity when evaluation fails, so there is nothing to compare",
          "and nothing can be excused. The evaluation error is above this summary in the",
          "job log; it names the offending attribute.", ""]
       elif .verdict == "block" then
         ["**BLOCKED:** \(.reason).", "",
          "A failure that is not present on the base belongs to this pull request, red base",
          "or not. Excusing it is how one red cause becomes six, which is the state this",
          "gate was written to end, so a red base is not a licence to add to it.", ""]
       elif .verdict == "cap" then
         ["**BLOCKED by the cap:** \(.reason).", "",
          "This is not about your change. The base has accumulated more failures than the",
          "cap allows, and the only pull requests that can land until it is back under the",
          "cap are ones that shrink it. Differential gating makes red survivable; the cap is",
          "what stops survivable becoming permanent.", ""]
       elif .verdict == "excused" then
         ["**Green here means \"no worse than the base\", NOT absolutely green.**",
          "\(.reason).", ""]
       elif (.fixed | length) > 0 then
         ["**Green, and better than the base:** \(.reason).", ""]
       else
         ["**Green means absolutely green:** \(.reason).", ""]
       end)
    + ["- failing checks: base \(.baselineFailingCount), here \(.prFailingCount), net \(if .prFailingCount > .baselineFailingCount then "+" else "" end)\(.prFailingCount - .baselineFailingCount)  (cap \(.cap))"]
    + (if .baseline.accepted then
         ["- baseline: `\(.baseline.commit // "?")` (run \(.baseline.runId // "?"), recorded \(.baseline.recordedAt // "?"))"]
       else
         ["- baseline: **none accepted**, so nothing was excused against a base and this gate behaved exactly as it did before the differential verdict existed. Reason: \(.baseline.rejectedBecause // "not published")"]
       end)
    + ["- instability record: \(.instability.runsFolded) run(s) folded, proven nondeterministic: \(if (.instability.provenUnstable | length) == 0 then "none" else (.instability.provenUnstable | join(", ")) end)"]
    + (if .checkSetChanged and (.baseline.accepted) then
         ["- **the check set changed**, so the identical-derivation excuse is withdrawn: adding or removing a check changes what runs beside every other check, and their derivations do not record that. A check proven nondeterministic is still excused, because that is a claim about the check rather than about this run."]
       else [] end)
    + (if .immunityForfeited then
         ["- **drv-identity immunity forfeited**: \(.forfeitReason). A pull request that changes what the derivation does not capture but the run depends on (environment, scheduling, concurrency) is judged strictly, because for such a change an identical derivation no longer means an unrelated failure."]
       else [] end)
    + (if (.blocked | length) > 0 then
         ["", "### Blocking: \(.blocked | length) failure(s) not present on the base", "",
          "| check | derivation |", "| --- | --- |"]
         + [.blocked[] | "| `\(.attr)` | \(drv(.drvPath)) |"]
         + ["", "Reproduce one with `nix build .#checks.\(.system // "x86_64-linux").<name> -L`."]
       else [] end)
    + (if (.unstable | length) > 0 then
         ["", "### Not attributable to this pull request: \(.unstable | length) nondeterministic failure(s)", "",
          "`identical-drv-passed-on-base` means the base and this run built the BYTE-IDENTICAL",
          "derivation and disagreed about the result, which is a proof that the diff did not",
          "cause it. `proven-nondeterministic` means some derivation of this check has been",
          "recorded both passing and failing, so the check is a coin flip whatever it is",
          "built from.", "",
          "**These checks are not gating anything.** That is the deliberate trade, and it is",
          "not free: a real break in one of them would also pass here. They are named in the",
          "standing issue with the date they were proven, and the point of the record is that",
          "the list should be getting shorter.", "",
          "| check | evidence | proof |", "| --- | --- | --- |"]
         + [.unstable[] | "| `\(.attr)` | \(.evidence) | ok in run \(.proof.okRunId // .proof.baseRunId // "?"), failed in run \(.proof.failRunId // .proof.prRunId // "?") |"]
       else [] end)
    + (if (.excused | length) > 0 then
         ["", "### Excused: \(.excused | length) failure(s) that already fail on the base", "",
          "Not this pull request'"'"'s fault. `identical-drv` means the base and this run failed",
          "on the byte-identical derivation, so it is literally the same build; `same-attr`",
          "means the hash moved because this pull request touches something the check depends",
          "on, and the CHECK ATTRIBUTE is the identity.", "",
          "| check | evidence | derivation |", "| --- | --- | --- |"]
         + [.excused[] | "| `\(.attr)` | \(.evidence) | \(drv(.prDrvPath)) |"]
       else [] end)
    + (if (.fixed | length) > 0 then
         ["", "### Repaired by this pull request: \(.fixed | length) check(s)", "",
          "These fail on the base and pass here.", "",
          "| check | derivation on the base |", "| --- | --- |"]
         + [.fixed[] | "| `\(.attr)` | \(drv(.baseDrvPath)) |"]
       else [] end)
    + (if (.removed | length) > 0 then
         ["", "### Removed by this pull request: \(.removed | length) check(s)", "",
          "These fail on the base and no longer exist here. Deleting a check is not repairing",
          "it, so they are reported apart from the repairs above.", "",
          "| check |", "| --- |"]
         + [.removed[] | "| `\(.attr)` |"]
       else [] end)
    + ["",
       "---",
       "",
       "This verdict is **not a required status check**. Ruleset 566717 on `main` carries no",
       "`required_status_checks` rule at all, so nothing here blocks a merge; one approving",
       "review is the only gate. Making it binding is one `required_status_checks` entry",
       "naming this job, and it should not be added until: the flake rate is at or under 5%",
       "over 20 consecutive runs (measured 61% on 2026-07-29), the instability record holds",
       "at least 30 runs, at most 1 in 20 pull request runs is blocked by a verdict a re-run",
       "then clears, and the p95 gate wall time is under 40 minutes against the merge queue'"'"'s",
       "60 minute `check_response_timeout_minutes`."]
    | .[]
  ' "${verdict_doc}"
}

# --- CLI ---------------------------------------------------------------------

dg_usage() {
  cat >&2 <<'EOF'
usage:
  delta-gate.sh admissible <baseline.json> <system> [max-age-hours]
  delta-gate.sh verdict <results.json> [baseline.json] [instability.json] [reject-reason] [forfeit-reason]
  delta-gate.sh summary <verdict.json>
  delta-gate.sh fold <instability.json|""> <results.json>
  delta-gate.sh unstable <instability.json>
  delta-gate.sh flake-rate <instability.json>
  delta-gate.sh forfeit-reason <changed-files.txt>
EOF
  exit 64
}

if [[ ${BASH_SOURCE[0]} == "${0}" ]]; then
  case "${1:-}" in
  admissible)
    [[ $# -ge 3 ]] || dg_usage
    dg_baseline_admissible "$2" "$3" "${4:-48}"
    ;;
  verdict)
    [[ $# -ge 2 ]] || dg_usage
    dg_verdict "$2" "${3:-}" "${4:-}" "${5:-}" "${6:-}"
    ;;
  summary)
    [[ $# -eq 2 ]] || dg_usage
    dg_render_summary "$2"
    ;;
  fold)
    [[ $# -eq 3 ]] || dg_usage
    dg_merge_instability "$2" "$3"
    ;;
  unstable)
    [[ $# -eq 2 ]] || dg_usage
    dg_unstable_attrs "$2"
    ;;
  flake-rate)
    [[ $# -eq 2 ]] || dg_usage
    dg_flake_rate "$2"
    ;;
  forfeit-reason)
    [[ $# -eq 2 ]] || dg_usage
    dg_immunity_forfeit_reason "$2"
    ;;
  *) dg_usage ;;
  esac
fi
