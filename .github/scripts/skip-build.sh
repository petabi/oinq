#!/usr/bin/env bash
# Decide whether this CI run can skip the Rust checks.
#
# Prints cargo=true or cargo=false to "$GITHUB_OUTPUT". cargo=false only when
# every changed path matches a skip glob. Globs come from SKIP_PATHS when that
# is set, otherwise from .github/ci-skip-paths. A missing file runs the Rust
# checks. Anything this script cannot establish runs them too. A reusable
# workflow call always runs them: the caller passes CALL_REF, and this script
# does not trust the caller's event.

set -uo pipefail

out=${GITHUB_OUTPUT:-/dev/stdout}

run_rust() {
  local reason=${1-}
  echo "cargo=true" >> "$out"
  if [ -n "$reason" ]; then
    echo "Running the Rust checks ($reason)."
  else
    echo "Running the Rust checks."
  fi
  exit 0
}

skip_rust() {
  echo "cargo=false" >> "$out"
  echo "Every changed path matches a skip glob. Skipping the Rust checks."
  exit 0
}

[ -z "${CALL_REF:-}" ] || run_rust "reusable call"

event=${EVENT:-}
ref_type=${REF_TYPE:-}
ref_name=${REF_NAME:-}
default_branch=${DEFAULT_BRANCH:-}
pr_base=${PR_BASE:-}
pr_head=${PR_HEAD:-}
before=${BEFORE:-}
sha=${SHA:-}

is_sha() {
  case "$1" in
    *[!0-9a-f]*) return 1 ;;
  esac
  [ "${#1}" -eq 40 ] || [ "${#1}" -eq 64 ] || return 1
  case "$1" in
    *[!0]*) return 0 ;;
  esac
  return 1
}

have_commit() {
  git cat-file -e "$1^{commit}" 2>/dev/null
}

shallow=$(git rev-parse --is-shallow-repository 2>/dev/null) \
  || run_rust "not a git repository"
[ "$shallow" = false ] || run_rust "shallow checkout"

top=$(git rev-parse --show-toplevel 2>/dev/null) || run_rust "no work tree"
cd "$top" || run_rust "cannot enter $top"

base=
head=
case "$event" in
  pull_request)
    # Diff the pull request head against its merge-base with the base tip, so
    # commits already on the base are not this pull request's change.
    is_sha "$pr_base" || run_rust "no pull request base"
    is_sha "$pr_head" || run_rust "no pull request head"
    have_commit "$pr_base" || run_rust "base $pr_base is not in the checkout"
    have_commit "$pr_head" || run_rust "head $pr_head is not in the checkout"
    base=$(git merge-base "$pr_base" "$pr_head") || run_rust "no merge-base with the base branch"
    head=$pr_head
    ;;
  push)
    [ "$ref_type" = branch ] || run_rust "push of a ${ref_type:-ref}"
    is_sha "$sha" || run_rust "no head commit"
    have_commit "$sha" || run_rust "head $sha is not in the checkout"
    head=$sha
    if [ "$ref_name" = "$default_branch" ]; then
      is_sha "$before" || run_rust "push has no previous commit"
      have_commit "$before" || run_rust "previous commit $before is not in the checkout"
      base=$before
    else
      git fetch --no-tags origin \
        "refs/heads/${default_branch}:refs/remotes/origin/${default_branch}" \
        || run_rust "cannot fetch origin/$default_branch"
      base=$(git merge-base "refs/remotes/origin/${default_branch}" "$head") \
        || run_rust "no merge-base with origin/$default_branch"
    fi
    ;;
  workflow_dispatch)
    [ "$ref_type" = branch ] || run_rust "dispatch on a ${ref_type:-ref}"
    [ "$ref_name" != "$default_branch" ] || run_rust "dispatch on $default_branch"
    is_sha "$sha" || run_rust "no head commit"
    have_commit "$sha" || run_rust "head $sha is not in the checkout"
    head=$sha
    git fetch --no-tags origin \
      "refs/heads/${default_branch}:refs/remotes/origin/${default_branch}" \
      || run_rust "cannot fetch origin/$default_branch"
    base=$(git merge-base "refs/remotes/origin/${default_branch}" "$head") \
      || run_rust "no merge-base with origin/$default_branch"
    ;;
  *)
    run_rust "event ${event:-<none>}"
    ;;
esac

is_sha "$base" || run_rust "no base commit"
have_commit "$base" || run_rust "base $base is not in the checkout"

if [ -n "${SKIP_PATHS-}" ]; then
  paths=$SKIP_PATHS
elif [ -f .github/ci-skip-paths ]; then
  paths=$(cat .github/ci-skip-paths)
else
  paths=
fi

exclude=()
while IFS= read -r pattern || [ -n "$pattern" ]; do
  pattern=${pattern%$'\r'}
  pattern="${pattern#"${pattern%%[![:space:]]*}"}"
  pattern="${pattern%"${pattern##*[![:space:]]}"}"
  [ -n "$pattern" ] || continue
  case "$pattern" in
    '#'*) continue ;;
  esac
  exclude+=(":(glob,exclude)$pattern")
done <<EOF
$paths
EOF

diff_names() {
  git -c core.quotePath=false diff --no-ext-diff --no-renames --name-only "$base" "$head" "$@"
}

changed=$(diff_names) || run_rust "git diff failed"
[ -n "$changed" ] || run_rust "the change has no paths"

if [ "${#exclude[@]}" -eq 0 ]; then
  outside=$changed
else
  outside=$(diff_names -- . "${exclude[@]}") || run_rust "git diff failed"
fi

echo "Comparing $base..$head"
echo "Changed paths:"
printf '%s\n' "$changed" | sed 's/^/  /'
if [ -n "$outside" ]; then
  echo "Paths outside the skip list:"
  printf '%s\n' "$outside" | sed 's/^/  /'
  run_rust
fi

skip_rust
