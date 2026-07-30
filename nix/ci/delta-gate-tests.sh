#!/usr/bin/env bash
# Fixture tests for nix/ci/delta-gate.sh. No network, no nix, no clock beyond
# `date`, so `nix build .#checks.<system>.delta-gate` runs the whole table.
#
# Every row of the verdict table gets a case, and so does every guard. A guard
# nobody has watched fail is not a guard: the cases below deliberately include
# the inverse of each one (immunity granted AND immunity forfeited, cap blocks
# AND cap yields to a shrinking set), because the failure mode of this file is
# a test that passes for the wrong reason.
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Invoked through `bash` rather than executed: a nix build sandbox has no
# /usr/bin/env, so the shebang would exit 126 and every case would fail for
# a reason that has nothing to do with the verdict.
dg() { bash "${here}/delta-gate.sh" "$@"; }
work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

pass=0
fail=0

# `check <name> <expected> <actual>`
check() {
  if [[ "$2" == "$3" ]]; then
    printf 'ok    %s\n' "$1"
    pass=$((pass + 1))
  else
    printf 'FAIL  %s\n        expected: %s\n        actual:   %s\n' "$1" "$2" "$3"
    fail=$((fail + 1))
  fi
}

# The library must PARSE before anything else is asserted. Twice now an
# apostrophe inside a jq program has ended the enclosing shell string, and the
# symptom was sixty failing cases rather than one syntax error. One line here
# turns that back into one line of output.
if ! bash -n "${here}/delta-gate.sh" 2>"${work}/parse.err"; then
  printf 'FAIL  delta-gate.sh does not parse\n'
  cat "${work}/parse.err"
  printf '\nhint: an apostrophe inside a single-quoted jq program ends the string.\n'
  exit 1
fi
printf 'ok    delta-gate.sh parses\n'

now="$(date +%s)"

# results <file> <runId> <evalFailed> <attr:outcome:drv> ...
# `outcome` is flake-gate.nix's own vocabulary: pass or fail.
results() {
  local out=$1 run=$2 evalFailed=$3
  shift 3
  local checks="[]" entry
  for entry in "$@"; do
    IFS=: read -r a o d <<<"${entry}"
    checks="$(jq -c --arg a "$a" --arg o "$o" --arg d "$d" \
      '. + [{attr: $a, outcome: $o, drvPath: (if $d == "" then null else $d end)}]' <<<"${checks}")"
  done
  jq -n --arg run "${run}" --argjson ef "${evalFailed}" --argjson checks "${checks}" \
    --argjson epoch "${now}" \
    '{schemaVersion: 1, system: "x86_64-linux", commit: "deadbeef", runId: $run,
      recordedAt: "2026-07-29T00:00:00Z", recordedAtEpoch: $epoch,
      evalFailed: $ef, checks: $checks}' >"${out}"
}

verdict_field() { jq -r "$2" "$1"; }

# --- row 1: fails here, fails on the base -> excused, gate passes -------------
results "${work}/pr.json"   pr   false "smash-e2e:fail:/nix/store/A.drv" "ok-check:pass:/nix/store/B.drv"
results "${work}/base.json" base false "smash-e2e:fail:/nix/store/A.drv" "ok-check:pass:/nix/store/B.drv"
dg verdict "${work}/pr.json" "${work}/base.json" "" "" "" >"${work}/v.json"
check "row1 fails-both: gate passes"   true    "$(verdict_field "${work}/v.json" .gatePasses)"
check "row1 fails-both: excused names it" "smash-e2e" "$(verdict_field "${work}/v.json" '.excused[0].attr')"
check "row1 fails-both: identical drv recorded" "identical-drv" "$(verdict_field "${work}/v.json" '.excused[0].evidence')"
check "row1 fails-both: nothing blocked" 0 "$(verdict_field "${work}/v.json" '.blocked | length')"

# --- row 2: fails here, passes on the base, DIFFERENT drv -> BLOCK ------------
# The regression case. `minecraft-literals` on 2026-07-29 was exactly this: the
# derivation moved and the outcome flipped, and it must not be excused.
results "${work}/pr.json"   pr   false "minecraft-literals:fail:/nix/store/NEW.drv"
results "${work}/base.json" base false "minecraft-literals:pass:/nix/store/OLD.drv"
dg verdict "${work}/pr.json" "${work}/base.json" "" "" "" >"${work}/v.json"
check "row2 new-failure: gate BLOCKS" false "$(verdict_field "${work}/v.json" .gatePasses)"
check "row2 new-failure: names the check" "minecraft-literals" "$(verdict_field "${work}/v.json" '.blocked[0].attr')"
check "row2 new-failure: verdict is block" "block" "$(verdict_field "${work}/v.json" .verdict)"
check "row2 new-failure: not excused as unstable" 0 "$(verdict_field "${work}/v.json" '.unstable | length')"

# --- row 2b: same, but the drv is IDENTICAL -> nondeterminism, not a block ----
# `differential-traces` on 2026-07-29: byte-identical derivation, opposite
# outcomes, eighteen minutes apart.
results "${work}/pr.json"   pr   false "differential-traces:fail:/nix/store/SAME.drv"
results "${work}/base.json" base false "differential-traces:pass:/nix/store/SAME.drv"
dg verdict "${work}/pr.json" "${work}/base.json" "" "" "" >"${work}/v.json"
check "row2b same-drv flip: gate passes" true "$(verdict_field "${work}/v.json" .gatePasses)"
check "row2b same-drv flip: reported unstable" "differential-traces" "$(verdict_field "${work}/v.json" '.unstable[0].attr')"
check "row2b same-drv flip: evidence names the proof" "identical-drv-passed-on-base" \
  "$(verdict_field "${work}/v.json" '.unstable[0].evidence')"
check "row2b same-drv flip: nothing blocked" 0 "$(verdict_field "${work}/v.json" '.blocked | length')"

# --- guard: immunity forfeited -> the SAME input now blocks -------------------
# The inverse of row2b, on byte-identical fixtures. If this does not flip, the
# forfeiture is decorative.
dg verdict "${work}/pr.json" "${work}/base.json" "" "" "touches nix/ci/flake-gate.nix" >"${work}/v.json"
check "forfeit: same-drv flip now BLOCKS" false "$(verdict_field "${work}/v.json" .gatePasses)"
check "forfeit: reason is carried" "touches nix/ci/flake-gate.nix" \
  "$(verdict_field "${work}/v.json" .forfeitReason)"
check "forfeit: no longer excused as unstable" 0 "$(verdict_field "${work}/v.json" '.unstable | length')"

# --- a changed check set withdraws the identical-derivation excuse ------------
# The gap this closes: a pull request that ADDS a check adds contention under a
# concurrent gate, which can time out an e2e check whose own derivation did not
# move. Without this, that reads as a coin flip and is waved through.
results "${work}/pr.json"   pr   false "proxy:fail:/nix/store/P.drv" "brand-new-check:pass:/nix/store/X.drv"
results "${work}/base.json" base false "proxy:pass:/nix/store/P.drv"
dg verdict "${work}/pr.json" "${work}/base.json" "" "" "" >"${work}/v.json"
check "check-set: adding a check withdraws same-drv immunity" false "$(verdict_field "${work}/v.json" .gatePasses)"
check "check-set: the change is reported" true "$(verdict_field "${work}/v.json" .checkSetChanged)"
check "check-set: the new failure is named" "proxy" "$(verdict_field "${work}/v.json" '.blocked[0].attr')"

# ... and with the check set UNCHANGED the identical fixtures are excused, or
# the check-set test above is passing for some other reason.
results "${work}/pr.json"   pr   false "proxy:fail:/nix/store/P.drv"
dg verdict "${work}/pr.json" "${work}/base.json" "" "" "" >"${work}/v.json"
check "check-set: unchanged set still grants same-drv immunity" true "$(verdict_field "${work}/v.json" .gatePasses)"

# A check PROVEN nondeterministic keeps its excuse even when the set changed:
# that is a claim about the check, not about this run.
results "${work}/f-ok.json"  f1 false "flaky:pass:/nix/store/F.drv"
results "${work}/f-bad.json" f2 false "flaky:fail:/nix/store/F.drv"
dg fold ""                 "${work}/f-ok.json"  >"${work}/k1.json"
dg fold "${work}/k1.json"  "${work}/f-bad.json" >"${work}/krec.json"
results "${work}/pr.json"   pr   false "flaky:fail:/nix/store/G.drv" "brand-new-check:pass:/nix/store/X.drv"
results "${work}/base.json" base false "flaky:pass:/nix/store/G.drv"
dg verdict "${work}/pr.json" "${work}/base.json" "${work}/krec.json" "" "" >"${work}/v.json"
check "check-set: a proven coin flip keeps its excuse" true "$(verdict_field "${work}/v.json" .gatePasses)"
check "check-set: and the evidence is the record, not the drv" "proven-nondeterministic" \
  "$(verdict_field "${work}/v.json" '.unstable[0].evidence')"

# --- row 3: passes here, fails on the base -> fixed, and it is CREDITED -------
results "${work}/pr.json"   pr   false "completions-e2e:pass:/nix/store/N.drv" "other:pass:/nix/store/B.drv"
results "${work}/base.json" base false "completions-e2e:fail:/nix/store/O.drv" "other:pass:/nix/store/B.drv"
dg verdict "${work}/pr.json" "${work}/base.json" "" "" "" >"${work}/v.json"
check "row3 repaired: gate passes" true "$(verdict_field "${work}/v.json" .gatePasses)"
check "row3 repaired: named in fixed" "completions-e2e" "$(verdict_field "${work}/v.json" '.fixed[0].attr')"
check "row3 repaired: not called removed" 0 "$(verdict_field "${work}/v.json" '.removed | length')"

# --- a DELETED check is not a repaired check ---------------------------------
# The check set moved between 60 and 67 entries over eighteen runs, so a base
# failure that is simply absent here is a real case and must not read as repair.
results "${work}/pr.json"   pr   false "other:pass:/nix/store/B.drv"
results "${work}/base.json" base false "deleted-check:fail:/nix/store/O.drv" "other:pass:/nix/store/B.drv"
dg verdict "${work}/pr.json" "${work}/base.json" "" "" "" >"${work}/v.json"
check "removed: not credited as a repair" 0 "$(verdict_field "${work}/v.json" '.fixed | length')"
check "removed: reported as removed" "deleted-check" "$(verdict_field "${work}/v.json" '.removed[0].attr')"
check "removed: gate passes" true "$(verdict_field "${work}/v.json" .gatePasses)"

# --- row 4: nothing fails anywhere -> absolute green -------------------------
results "${work}/pr.json"   pr   false "a:pass:/nix/store/A.drv"
results "${work}/base.json" base false "a:pass:/nix/store/A.drv"
dg verdict "${work}/pr.json" "${work}/base.json" "" "" "" >"${work}/v.json"
check "row4 all-green: gate passes" true "$(verdict_field "${work}/v.json" .gatePasses)"
check "row4 all-green: verdict is pass" "pass" "$(verdict_field "${work}/v.json" .verdict)"

# --- history-proven instability, when the pull request MOVED the drv ---------
# The case the within-verdict proof cannot reach: a pull request touching a core
# crate moves every downstream hash, so the base and PR derivations differ and
# only the record can speak.
results "${work}/r-ok.json"   r1 false "smash-hud-e2e:pass:/nix/store/H.drv"
results "${work}/r-bad.json"  r2 false "smash-hud-e2e:fail:/nix/store/H.drv"
dg fold ""                  "${work}/r-ok.json"  >"${work}/i1.json"
dg fold "${work}/i1.json"   "${work}/r-bad.json" >"${work}/inst.json"
check "fold: the check is proven unstable" "smash-hud-e2e" "$(dg unstable "${work}/inst.json")"

results "${work}/pr.json"   pr   false "smash-hud-e2e:fail:/nix/store/MOVED.drv"
results "${work}/base.json" base false "smash-hud-e2e:pass:/nix/store/OTHER.drv"
dg verdict "${work}/pr.json" "${work}/base.json" "${work}/inst.json" "" "" >"${work}/v.json"
check "history: moved drv, still not blamed" true "$(verdict_field "${work}/v.json" .gatePasses)"
check "history: evidence is the record" "proven-nondeterministic" \
  "$(verdict_field "${work}/v.json" '.unstable[0].evidence')"

# ... and the same fixtures with NO record must block, or the record is not
# what is doing the work.
dg verdict "${work}/pr.json" "${work}/base.json" "" "" "" >"${work}/v.json"
check "history: without the record it BLOCKS" false "$(verdict_field "${work}/v.json" .gatePasses)"

# --- a deterministic check is never promoted by the fold ---------------------
# Two runs, two different derivations, one ok and one fail. That is a
# regression, not a coin flip, and the record must not confuse them.
results "${work}/d-ok.json"  d1 false "deterministic:pass:/nix/store/D1.drv"
results "${work}/d-bad.json" d2 false "deterministic:fail:/nix/store/D2.drv"
dg fold ""                   "${work}/d-ok.json"  >"${work}/j1.json"
dg fold "${work}/j1.json"    "${work}/d-bad.json" >"${work}/j2.json"
check "fold: different drvs prove nothing" "" "$(dg unstable "${work}/j2.json")"

# --- the cap -----------------------------------------------------------------
seven=(); for n in 1 2 3 4 5 6 7; do seven+=("c${n}:fail:/nix/store/C${n}.drv"); done
results "${work}/base.json" base false "${seven[@]}"
results "${work}/pr.json"   pr   false "${seven[@]}"
dg verdict "${work}/pr.json" "${work}/base.json" "" "" "" >"${work}/v.json"
check "cap: seven on the base BLOCKS a no-op PR" false "$(verdict_field "${work}/v.json" .gatePasses)"
check "cap: verdict says cap" "cap" "$(verdict_field "${work}/v.json" .verdict)"

# The escape hatch. Without it, tripping the cap would block the fixes that are
# the only way to untrip it.
results "${work}/pr.json" pr false "c1:pass:/nix/store/C1.drv" "c2:fail:/nix/store/C2.drv" \
  "c3:fail:/nix/store/C3.drv" "c4:fail:/nix/store/C4.drv" "c5:fail:/nix/store/C5.drv" \
  "c6:fail:/nix/store/C6.drv" "c7:fail:/nix/store/C7.drv"
dg verdict "${work}/pr.json" "${work}/base.json" "" "" "" >"${work}/v.json"
check "cap: a PR that shrinks the set still lands" true "$(verdict_field "${work}/v.json" .gatePasses)"
check "cap: the shrink is recorded" true "$(verdict_field "${work}/v.json" .shrinksFailingSet)"

# --- an evaluation failure is never excused ----------------------------------
results "${work}/pr.json"   pr   true  "a:fail:"
results "${work}/base.json" base false "a:fail:/nix/store/A.drv"
dg verdict "${work}/pr.json" "${work}/base.json" "" "" "" >"${work}/v.json"
check "eval failure: BLOCKS even though the base fails too" false "$(verdict_field "${work}/v.json" .gatePasses)"
check "eval failure: verdict says so" "eval-failed" "$(verdict_field "${work}/v.json" .verdict)"

# --- no baseline: fail closed, exactly the old behaviour ---------------------
results "${work}/pr.json" pr false "a:fail:/nix/store/A.drv"
dg verdict "${work}/pr.json" "" "" "no baseline has been published yet" "" >"${work}/v.json"
check "no baseline: nothing is excused" false "$(verdict_field "${work}/v.json" .gatePasses)"
check "no baseline: the reason is carried to the reader" "no baseline has been published yet" \
  "$(verdict_field "${work}/v.json" .baseline.rejectedBecause)"

# --- baseline admission fails closed on every malformed shape ----------------
good="$(jq -n --argjson e "${now}" '{schemaVersion:1, system:"x86_64-linux", evalFailed:false,
  recordedAtEpoch:$e, checks:[{attr:"a",outcome:"ok",drvPath:"/nix/store/A.drv"}]}')"
admit() { printf '%s' "$1" >"${work}/b.json"; dg admissible "${work}/b.json" x86_64-linux 48 2>/dev/null; }

admit "${good}"; check "admit: a good baseline is admitted" 0 "$?"
admit "$(jq '.schemaVersion = 99' <<<"${good}")"; check "admit: wrong schema refused" 1 "$?"
admit "$(jq '.system = "aarch64-darwin"' <<<"${good}")"; check "admit: wrong system refused" 1 "$?"
admit "$(jq '.evalFailed = true' <<<"${good}")"; check "admit: eval-failed baseline refused" 1 "$?"
admit "$(jq '.checks = []' <<<"${good}")"; check "admit: empty check set refused" 1 "$?"
admit "$(jq --argjson e "$((now - 60 * 3600))" '.recordedAtEpoch = $e' <<<"${good}")"
check "admit: a 60h baseline refused at a 48h limit" 1 "$?"
admit "not json at all"; check "admit: garbage refused" 1 "$?"
admit ""; check "admit: an empty document refused" 1 "$?"

# --- the forfeiture path recognises what it claims to --------------------
printf 'crates/hyperion/src/lib.rs\n' >"${work}/files.txt"
check "forfeit-reason: an ordinary source change forfeits nothing" "" \
  "$(dg forfeit-reason "${work}/files.txt")"
printf 'crates/hyperion/src/lib.rs\n.github/workflows/ci.yml\n' >"${work}/files.txt"
check "forfeit-reason: a workflow change forfeits, naming the path" ".github/workflows/ci.yml" \
  "$(dg forfeit-reason "${work}/files.txt" | grep -o '\.github/workflows/ci\.yml')"
printf 'nix/ci/flake-gate.nix\n' >"${work}/files.txt"
check "forfeit-reason: a gate change forfeits" "nix/ci/flake-gate.nix" \
  "$(dg forfeit-reason "${work}/files.txt" | grep -o 'nix/ci/flake-gate.nix')"

# --- the summary renders for every verdict, and says which meaning is in force
for v in pass block cap excused; do
  case "${v}" in
  pass)    results "${work}/pr.json" pr false "a:pass:/nix/store/A.drv"
           results "${work}/base.json" base false "a:pass:/nix/store/A.drv" ;;
  block)   results "${work}/pr.json" pr false "a:fail:/nix/store/N.drv"
           results "${work}/base.json" base false "a:pass:/nix/store/O.drv" ;;
  cap)     results "${work}/base.json" base false "${seven[@]}"
           results "${work}/pr.json" pr false "${seven[@]}" ;;
  excused) results "${work}/pr.json" pr false "a:fail:/nix/store/A.drv"
           results "${work}/base.json" base false "a:fail:/nix/store/A.drv" ;;
  esac
  dg verdict "${work}/pr.json" "${work}/base.json" "" "" "" >"${work}/v.json"
  dg summary "${work}/v.json" >"${work}/s.md" 2>"${work}/s.err"
  rc=$?
  check "summary(${v}): renders" 0 "${rc}"
  check "summary(${v}): is not empty" true "$([[ -s ${work}/s.md ]] && echo true || echo false)"
  check "summary(${v}): states the enforcement status" true \
    "$(grep -q 'not a required status check' "${work}/s.md" && echo true || echo false)"
done

printf '\n%d passed, %d failed\n' "${pass}" "${fail}"
[[ ${fail} -eq 0 ]]
