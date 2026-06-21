#!/usr/bin/env bash
# Rebuild the atrium/integration deploy line from atrium-integration.manifest.
#
#   origin/main (or pinned tag)  +  each topic branch, merged in order.
#
# Idempotent and destructive by design: it force-resets atrium/integration to
# `base` and re-merges every topic. git rerere replays prior conflict
# resolutions, so a clean rebuild needs no manual work. See ATRIUM_FORK.md.
set -euo pipefail

cd "$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
MANIFEST="atrium-integration.manifest"
BRANCH="atrium/integration"

git config rerere.enabled true
git config rerere.autoupdate true

base="$(awk -F'=' '/^[[:space:]]*base[[:space:]]*=/{gsub(/[[:space:]]/,"",$2);print $2;exit}' "$MANIFEST")"
[ -n "$base" ] || { echo "no 'base =' in $MANIFEST" >&2; exit 1; }

# Topic branches = non-comment, non-blank, non-'base=' lines; first field only.
mapfile -t topics < <(grep -vE '^[[:space:]]*(#|base[[:space:]]*=|$)' "$MANIFEST" | awk '{print $1}')

echo "==> fetching upstream + fork"
git fetch origin --quiet
git fetch fork   --quiet

echo "==> resetting $BRANCH to $base"
git switch -C "$BRANCH" "$base"

for topic in "${topics[@]}"; do
  echo "==> merging $topic"
  if ! git merge --no-ff "$topic" -m "Merge $topic into $BRANCH"; then
    if git ls-files -u | grep -q .; then
      echo "!! unresolved conflicts merging $topic — resolve, 'git add', 'git commit', then re-run." >&2
      exit 1
    fi
    # rerere resolved everything (autoupdate staged it) — finalize the merge.
    git commit --no-edit
  fi
done

echo "==> done. $BRANCH = $base + ${#topics[@]} topic(s):"
git log --oneline "$base..$BRANCH" | sed 's/^/    /'
echo
echo "Next: build the image  (just build-one api-rs && just build-one sandbox && just deploy)"
echo "Required env: ARTIFACT_CAPTURE_API_KEY, CENTAUR_API_URL"
