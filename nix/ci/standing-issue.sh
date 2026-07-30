#!/usr/bin/env bash
# The standing issue: one issue in this repository, rewritten in place on every
# main run, that names what is currently failing and how long it has been.
#
# WHY A STANDING ISSUE AND NOT A NOTIFICATION. Differential gating makes red
# survivable, and survivable is comfortable. Three things push back, and this is
# the weakest of the three, deliberately:
#
#   The RATCHET, which is automatic and needs nobody. A pull request takes the
#   newest baseline, so the instant a check passes on main it can never be
#   excused again. Repairs are one-way and there is no list to forget.
#
#   The CAP in nix/ci/delta-gate.sh, which refuses everything but repairs once
#   the base carries more than six failures.
#
#   This issue, which is the only part that asks a person for anything.
#
# One issue rewritten in place, never one per red run, and it closes itself when
# main goes green. A stream of notifications is a stream people mute; a single
# document with a date on it is a thing somebody can be asked about.
#
# It lives in hyperion-mc/hyperion rather than in Linear because the repository
# is public and so is the failing set. An outside contributor who meets a red
# gate needs to be able to read why without an internal account. Reference the
# Linear issue from the body for internal tracking, not the other way round.
#
# DRY RUN IS THE DEFAULT. Set GATE_ISSUE_APPLY=1 to actually write. A script
# that files issues should be watched rendering its output before it is allowed
# to file anything, and the render is the whole of what it does.
set -euo pipefail

title=${GATE_ISSUE_TITLE:-"Flake gate: main is red"}
repo=${GITHUB_REPOSITORY:-hyperion-mc/hyperion}
results=${1:?usage: standing-issue.sh <results.json> [instability.json]}
record=${2:-}
apply=${GATE_ISSUE_APPLY:-0}

body="$(mktemp)"
trap 'rm -f "${body}"' EXIT

failing="$(jq -r '[(.checks // [])[] | select(.outcome == "fail")] | length' "${results}")"

# The existing issue, matched on an EXACT title. Search is fuzzy, and a near
# miss would open a second standing issue on every run, which is the notification
# stream this exists to avoid.
existing=""
if command -v gh >/dev/null 2>&1 && [[ -n ${GH_TOKEN:-} ]]; then
  existing="$(gh issue list --repo "${repo}" --state open --limit 100 \
    --search "\"${title}\" in:title" --json number,title \
    --jq "[.[] | select(.title == \"${title}\")] | first | .number // empty" 2>/dev/null || true)"
fi

if [[ ${failing} -eq 0 ]]; then
  if [[ -n ${existing} ]]; then
    printf 'main is green; closing #%s\n' "${existing}"
    [[ ${apply} == 1 ]] && gh issue close "${existing}" --repo "${repo}" \
      --comment "Every flake check passes on main as of \`$(jq -r '.commit // "?"' "${results}")\` (run $(jq -r '.runId // "?"' "${results}")). Closed by nix/ci/standing-issue.sh."
  else
    printf 'main is green and no standing issue is open\n'
  fi
  exit 0
fi

{
  printf '%s check(s) are failing on `main` as of `%s` ([run %s](https://github.com/%s/actions/runs/%s)).\n\n' \
    "${failing}" "$(jq -r '.commit // "?"' "${results}")" "$(jq -r '.runId // "?"' "${results}")" \
    "${repo}" "$(jq -r '.runId // "?"' "${results}")"
  printf 'Pull requests are judged on the DELTA against this set, so a red `main` does\n'
  printf 'not block anybody: a pull request passes when it makes the set no larger. That\n'
  printf 'is what stops one red cause becoming six, and it is also why this issue exists,\n'
  printf 'because nothing else here is inconvenienced by the set staying red.\n\n'
  printf 'Two things do push back. The set is a RATCHET: the moment a check passes on\n'
  printf '`main` it can never be excused again, so a repair cannot be undone by\n'
  printf 'inattention. And there is a CAP of 6 in `nix/ci/delta-gate.sh`: past it, the\n'
  printf 'only pull requests that can land are ones that shrink this list.\n\n'
  printf '### Failing\n\n| check |\n| --- |\n'
  jq -r '(.checks // [])[] | select(.outcome == "fail") | "| `\(.attr)` |"' "${results}"

  if [[ -n ${record} && -s ${record} ]]; then
    unstable="$(jq -r '(.proven // []) | length' "${record}")"
    if [[ ${unstable} -gt 0 ]]; then
      printf '\n### Proven nondeterministic, and therefore NOT GATING ANYTHING\n\n'
      printf 'Some derivation of each of these has been recorded both passing and failing,\n'
      printf 'which is a proof that the check is a coin flip rather than a suspicion. The\n'
      printf 'gate does not blame a pull request for them, which is the only way a\n'
      printf 'differential verdict can be trusted, and the cost is that a real break in one\n'
      printf 'of them would also go unnoticed. **This list should be getting shorter.**\n\n'
      printf '| check | proof |\n| --- | --- |\n'
      jq -r '(.proven // [])[] | "| `\(.attr)` | passed in run \(.okRunId // "?"), failed in run \(.failRunId // "?"), same derivation |"' "${record}"
      printf '\nMeasured flake rate: '
      jq -r 'if .rate == null then "not enough runs yet" else "\(.rate * 100 | round)% of the last \(.runs) runs hit at least one of these (criterion for making the verdict a required check: 5%)" end' \
        <(bash "$(dirname "${BASH_SOURCE[0]}")/delta-gate.sh" flake-rate "${record}")
      printf '\n'
    fi
  fi

  printf '\n---\n\nRewritten in place by every `main` run of `nix/ci/standing-issue.sh`, and closed\n'
  printf 'automatically when every check passes. There is deliberately no comment and no\n'
  printf 'new issue per failure: this is a status document, not a notification stream.\n'
} >"${body}"

if [[ ${apply} != 1 ]]; then
  printf '=== DRY RUN (set GATE_ISSUE_APPLY=1 to write) ===\n'
  printf 'would %s issue in %s titled: %s\n\n' \
    "$([[ -n ${existing} ]] && echo "update #${existing}" || echo "create an")" "${repo}" "${title}"
  cat "${body}"
  exit 0
fi

if [[ -n ${existing} ]]; then
  gh issue edit "${existing}" --repo "${repo}" --body-file "${body}"
  printf 'updated #%s\n' "${existing}"
else
  gh issue create --repo "${repo}" --title "${title}" --body-file "${body}"
fi
