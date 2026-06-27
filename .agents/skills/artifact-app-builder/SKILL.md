---
name: artifact-app-builder
description: "Build a browser-runnable work product (app, applet, dashboard, demo, interactive report, visualization, calculator, game) that Atrium presents and previews. Use when the user wants something they can open and interact with, not just a file."
---

# Artifact App Builder

Create a useful static artifact that Atrium captures, presents, and previews. Favor
business value and a working result over a perfect production architecture.

## When to use this skill

Use this skill even if the user does **not** mention Atrium, artifacts, `shared/apps`,
preview mode, metadata, or this skill by name. Ordinary requests like these should
produce an Atrium-presented app:

- "Make me an incident command center for SaaS outages."
- "Build a sales pipeline dashboard with sample data."
- "Create an interactive ROI calculator."
- "Make a small game / simulator / visual report I can try."

If the user asks for something interactive, browser-runnable, app-like, dashboard-like,
or demo-like, infer the Atrium app contract yourself. Do not ask the user for the
output path unless the requested app name or scope is genuinely ambiguous.

For a simple request, choose:

- a slug from the request, e.g. `incident-command-center`, `sales-pipeline-dashboard`;
- output path `shared/apps/<slug>/index.html`;
- metadata path `shared/apps/<slug>/atrium.app.json`;
- embedded sample data when the user has not supplied real data;
- `index.html?preview=1` preview mode;
- an `atrium.app.json` that includes title, description, entrypoint, renderer,
  preview URL, preview sizing, and isolated state policy.

## How presentation works (no command needed)

Presentation is **automatic**: build the app in the right place and it shows up for
the human as a "Presented app" with a Preview. There is no command to run and no
file to POST — putting the app at the path below *is* presenting it.

- Put the app at `shared/apps/<slug>/index.html` (`<slug>` is `[a-z0-9][a-z0-9_-]*`).
- It auto-surfaces once captured. The human previews it in a sandboxed iframe and can
  Publish it to a durable, launchable version.

Before writing, probe the exact target directory:

```sh
mkdir -p shared/apps/<slug> && test -w shared/apps/<slug>
```

If the target is not writable, stop and report the exact permission problem. Do **not**
use `sudo`, rename/replace `shared`, write to `shared.root-owned`, or create a
lookalike path elsewhere; Atrium will not present those files as the requested app.

## Optional metadata

By default the tile's title is the `<slug>` and the renderer is inferred from the
entry file. To customize, drop a sibling `shared/apps/<slug>/atrium.app.json`:

```json
{ "title": "Weather Dashboard", "description": "Live 7-day forecast", "renderer": "html-app" }
```

All fields optional. `entry` may point at a non-default file (e.g. `"App.jsx"` with
`"renderer": "react-jsx"`), but prefer a built `index.html` for real apps.

For user-facing app requests, metadata is effectively required because it teaches
Atrium how to render the inline preview. Prefer this shape:

```json
{
  "title": "Incident Command Center",
  "description": "Interactive SaaS outage dashboard with severity and status filters.",
  "entrypoint": "index.html",
  "renderer": "html-app",
  "preview": {
    "enabled": true,
    "url": "index.html?preview=1",
    "defaultSize": "card",
    "sizes": [
      { "id": "compact", "minWidth": 280, "height": 220 },
      { "id": "card", "minWidth": 420, "height": 320 },
      { "id": "wide", "minWidth": 640, "height": 440 },
      { "id": "expanded", "minWidth": 640, "height": 720 }
    ]
  },
  "state": {
    "mode": "isolated"
  }
}
```

The app should read both query parameters:

- `preview=1`: render the compact embedded preview surface.
- `previewSize=<id>`: adapt density/layout for `compact`, `card`, `wide`, or
  `expanded` when practical. If unsupported, ignore it gracefully.

## Output contract

- Prefer a single self-contained `index.html` for small/medium artifacts.
- Keep CSS and JavaScript inline so capture and preview are simple.
- Use ordinary browser APIs and plain JavaScript when that's enough.
- For React/TypeScript apps, build to static HTML/JS and put the built `index.html`
  under `shared/apps/<slug>/`. Do not ship `node_modules`, source maps, or lockfiles.
- Keep artifacts reasonably small — very large bundles may be captured as metadata
  only and won't preview.
- Do not wait for the user to specify the app path. Pick a safe slug and create the
  standard `shared/apps/<slug>/` files.

## Preview mode design

When the app supports `index.html?preview=1`, treat that as an embedded Atrium
card surface, not a miniature standalone landing page.

- Keep a subtly distinct app background so the generated app does not look like
  native Atrium UI, but keep outer padding tight: roughly 8-12px.
- Avoid large centered stage wrappers, showcase cards, heavy shadows, and thick
  decorative borders in preview mode.
- Prefer a full-width compact layout that uses most of the iframe area.
- Repeated records can use small cards or rows, but do not wrap the entire
  preview in another large card inside Atrium's card.
- Minimize empty margins and decorative chrome. Put useful interactive content
  near the top of the preview.
- Full mode may be more spacious and app-like; preview mode should be dense,
  scannable, and sized to render well in an Atrium thread without inner scrollbars
  for ordinary content.
- If `previewSize=expanded`, keep the same embedded style but allow more vertical
  detail so Atrium's expanded preview can avoid inner scrollbars.

## Runtime assumptions (the preview is a locked-down static browser sandbox)

- No backend server, no server-side rendering.
- No API keys in the browser.
- No `localStorage` / `sessionStorage`.
- Avoid external network requests; **embed sample or already-fetched data** directly in
  the artifact. (If the user explicitly wants a quick prototype that calls out, warn
  that it may not work in the locked-down preview.)
- CSS from a CDN may be blocked — prefer inline styles or a small bundled stylesheet
  over relying on a CDN `<script>`.

## Smoke test before you finish

Do at least one lightweight check:

- open the file with a local static server when available;
- run the build command for project-based apps;
- or inspect the HTML for obvious syntax / path mistakes.

If a check fails and you can't fix it quickly, leave the best working version in place
and tell the user the limitation.
