# Atrium's Centaur fork — how we work here

This is a **fork** of `paradigmxyz/centaur` (`origin`) maintained by Atrium
(`fork` = `gbasin/centaur`). It carries Atrium's in-flight and fork-only changes
on top of upstream. Read this before branching, merging, or deploying.

> This file lives only on the integration line, never on the upstreamable topic
> branches — keep it that way so PRs to paradigm stay clean.

## Remotes
- `origin` → `github.com/paradigmxyz/centaur` — **upstream, source of truth.**
- `fork`   → `github.com/gbasin/centaur` — where our branches live.

## Branch model
Two different shapes, for two different jobs:

**Topic branches** — single-purpose, cut from `origin/main`, one feature each.
Each is labelled by intent (see the manifest):
- `upstream-pending` — will be PR'd to paradigm; deleted once it merges upstream.
- `fork-permanent`   — Atrium-only; never upstreamed; carried & rebased forever.
- `undecided`        — may go either way; revisit before it grows.

**`atrium/integration`** — the **deploy line**: `origin/main` + every topic, merged.
This is what Atrium's Centaur image is built from. It is **throwaway and
rebuildable** — never the source of truth, never commit features directly to it.

```
origin/main
  ├─ gb/session-cancel-api-rs      (upstream-pending, PR #616)   ┐
  ├─ gb/api-rs-hitl-relay          (upstream-pending, on cancel) ├─ merged ─▶ atrium/integration
  └─ gb/api-rs-artifact-capture    (undecided)                   ┘
```

## Rules for agents
1. **New change → new topic branch off `origin/main`.** Never branch off
   `atrium/integration`, never commit a feature straight onto it.
2. **Never put fork-process files on a topic branch** — not this file, not the
   manifest, not the rebuild script, not Atrium-only config. They'd leak into
   the upstream PR diff.
3. **Keep fork-only changes additive / flag-gated** where possible. Additive
   changes rebase cleanly across upstream churn; invasive edits cost you on
   every sync.
4. **Migrations:** paradigm (`origin`) uses sequential 4-digit versions
   (`0001`, `0002`, …). **Fork-permanent migrations MUST use the reserved
   `1000+` range** (e.g. `1000_artifact_blobs.sql`) so they always sort after
   paradigm's and never collide as upstream grows — a sequential fork migration
   gets steamrolled the moment paradigm ships a migration at that number (this
   bit us three times). `1000+` migrations only depend on early upstream tables
   (e.g. `session_executions`), so applying them last is fine. Upstream-pending
   topics still number sequentially from `origin/main`'s latest (they're going
   into paradigm's sequence). NOTE: renumbering a migration that a live DB has
   already applied requires reconciling that DB's `_sqlx_migrations` before the
   next deploy.
5. **`git rerere` is enabled** — your conflict resolutions are recorded and
   replayed on the next rebuild. Resolve carefully once.

## Rebuilding the integration line
```
./atrium-integration.sh            # fetch upstream + topics, rebuild, merge, report
```
Then build & deploy the image (see below). When a topic lands upstream, drop it
from `atrium-integration.manifest`; when all are upstream, delete the branch and
track `origin/main` directly.

## Building / deploying for Atrium
```
just build-one api-rs
just build-one sandbox
just deploy
```
**Required env for artifact capture:** `ARTIFACT_CAPTURE_API_KEY` (a dedicated,
narrowly-scoped key — do NOT reuse `CENTAUR_API_KEY`) and `CENTAUR_API_URL`.

## Pinning
Prefer tracking upstream **release tags** (`centaur-0.1.x`) as the manifest
`base` rather than chasing every `origin/main` commit — fewer rebuilds, more
stable deploys. Bump the base on your own schedule.
