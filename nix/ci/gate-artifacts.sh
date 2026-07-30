#!/usr/bin/env bash
# Read the two documents this gate carries between runs: the BASELINE (main's
# failing set, which a pull request is judged against) and the INSTABILITY
# RECORD (which checks have been proven nondeterministic). Both are Actions
# artifacts.
#
# THE IMPURE HALF ONLY. Every decision that can be made from files alone lives
# in nix/ci/delta-gate.sh, where the fixture suite can reach it. What is here is
# what needs the API: which artifact is newest, and whether its commit is
# actually an ancestor of this pull request's base.
#
# Artifacts rather than a second evaluation of the base tree. `main` already
# computes its failing set in order to build at all, so publishing that set
# makes a pull request's comparison ONE API read instead of a full rebuild of
# the base. Rebuilding the base per pull request would double a job measured at
# 31 to 41 minutes over the eighteen main runs of 2026-07-28/29, and there is no
# cache to make the second copy cheap: the flake names cache.ix.dev as a
# substituter but nothing in this repository pushes to it.
#
# Every path exits 0 and prints a reason. "No baseline" is a normal outcome that
# degrades the gate to absolute green, which is the fail-closed direction and is
# exactly how the gate behaved before any of this existed. It is not an error
# that should fail the job.
set -uo pipefail

say() { printf '%s\n' "$*"; }

# Download the newest unexpired artifact named $1 and extract member $2 to $3.
# Prints a reason and returns 1 when it cannot.
newest_artifact() {
  local name=$1 member=$2 dest=$3 artifacts id created zip
  for tool in gh jq unzip; do
    command -v "${tool}" >/dev/null 2>&1 || { say "${tool} is not on PATH"; return 1; }
  done
  [[ -n ${GH_TOKEN:-} && -n ${GITHUB_REPOSITORY:-} ]] || {
    say "GH_TOKEN or GITHUB_REPOSITORY is unset, so ${name} cannot be read"
    return 1
  }
  # ONE read of the artifact index, filtered server-side by name. Newest by
  # created_at taken explicitly rather than by trusting the endpoint order:
  # "newest wins" is what makes a main that has gone GREEN revoke every excuse,
  # because the newest baseline is then an empty failing set. Publishing on
  # green is not an optimisation to skip, it is the revocation mechanism.
  artifacts="$(gh api "repos/${GITHUB_REPOSITORY}/actions/artifacts?name=${name}&per_page=50" 2>/dev/null)" || {
    say "the artifacts API read failed"
    return 1
  }
  read -r id created <<<"$(jq -r '
    [.artifacts[]? | select(.expired != true)] | max_by(.created_at)
    | if . == null then "" else "\(.id) \(.created_at)" end' <<<"${artifacts}")"
  [[ -n ${id:-} ]] || { say "no unexpired '${name}' artifact has been published yet"; return 1; }

  zip="${dest}.zip"
  gh api "repos/${GITHUB_REPOSITORY}/actions/artifacts/${id}/zip" >"${zip}" 2>/dev/null || {
    say "artifact ${id} could not be downloaded"
    return 1
  }
  unzip -p "${zip}" "${member}" >"${dest}" 2>/dev/null || {
    rm -f "${zip}"
    say "artifact ${id} contains no ${member}"
    return 1
  }
  rm -f "${zip}"
  printf '%s %s' "${id}" "${created}"
  return 0
}

cmd_baseline() {
  local dest=${1:?usage: gate-artifacts.sh baseline <dest.json>}
  local meta id created commit status

  [[ ${GITHUB_EVENT_NAME:-} == "pull_request" ]] || {
    say "event '${GITHUB_EVENT_NAME:-local}' is not a pull request, so there is no base to be no-worse-than"
    return 0
  }
  # Without the base sha the ancestry check cannot run, and a baseline that
  # cannot be placed in this pull request's history must be refused rather than
  # trusted.
  [[ -n ${PR_BASE_SHA:-} ]] || {
    say "PR_BASE_SHA is unset, so the baseline cannot be placed in this pull request's history"
    return 0
  }
  meta="$(newest_artifact "${BASELINE_ARTIFACT:-main-check-failures}" results.json "${dest}")" || {
    say "${meta}"
    return 0
  }
  read -r id created <<<"${meta}"

  # ANCESTRY. A baseline describes this pull request's base only when its commit
  # is an ancestor of, or equal to, that base. `compare/A...B` answers `ahead`
  # when B is ahead of A, which is to say A is an ancestor of B, and `identical`
  # when they are the same commit. `behind` or `diverged` means the baseline
  # describes a main this pull request is not built on, and excusing against it
  # would excuse failures that may already have been repaired.
  commit="$(jq -r '.commit // ""' "${dest}" 2>/dev/null)"
  [[ -n ${commit} ]] || { rm -f "${dest}"; say "the baseline names no commit"; return 0; }
  status="$(gh api "repos/${GITHUB_REPOSITORY}/compare/${commit}...${PR_BASE_SHA}" --jq '.status' 2>/dev/null || true)"
  case "${status}" in
  identical | ahead) ;;
  *)
    rm -f "${dest}"
    say "baseline commit ${commit} is '${status:-unresolvable}' relative to this base ${PR_BASE_SHA}, so it does not describe this base"
    return 0
    ;;
  esac

  say "baseline accepted: ${commit} (artifact ${id}, created ${created}, $(
    jq -r '[(.checks // [])[] | select(.outcome == "fail")] | length' "${dest}"
  ) failing check(s))"
}

# The instability record is not about any particular commit, so it needs no
# ancestry check: a proof that some derivation of a check both passed and failed
# stays a proof whatever the tree has done since.
cmd_instability() {
  local dest=${1:?usage: gate-artifacts.sh instability <dest.json>}
  local meta
  meta="$(newest_artifact "${INSTABILITY_ARTIFACT:-gate-instability}" instability.json "${dest}")" || {
    say "${meta}; starting a fresh record"
    return 0
  }
  say "instability record loaded ($(jq -r '.runsFolded // 0' "${dest}") run(s) folded, $(
    jq -r '(.proven // []) | length' "${dest}"
  ) check(s) proven nondeterministic)"
}

case "${1:-}" in
baseline) shift; cmd_baseline "$@" ;;
instability) shift; cmd_instability "$@" ;;
*)
  cat >&2 <<'EOF'
usage:
  gate-artifacts.sh baseline <dest.json>
  gate-artifacts.sh instability <dest.json>
EOF
  exit 64
  ;;
esac
