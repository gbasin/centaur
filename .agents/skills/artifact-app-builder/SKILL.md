---
name: artifact-app-builder
description: "Build a browser-runnable work product (app, applet, dashboard, demo, interactive report, visualization, calculator, game) that Atrium presents and previews. Use when the user wants something they can open and interact with, not just a file."
---

# Artifact App Builder

Create a useful static artifact that Atrium captures, presents, and previews. Favor
business value and a working result over a perfect production architecture.

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

## Output contract

- Prefer a single self-contained `index.html` for small/medium artifacts.
- Keep CSS and JavaScript inline so capture and preview are simple.
- Use ordinary browser APIs and plain JavaScript when that's enough.
- For React/TypeScript apps, build to static HTML/JS and put the built `index.html`
  under `shared/apps/<slug>/`. Do not ship `node_modules`, source maps, or lockfiles.
- Keep artifacts reasonably small — very large bundles may be captured as metadata
  only and won't preview.

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
