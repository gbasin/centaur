# Artifact App Builder

Use this skill when the user asks for an Atrium artifact, applet, dashboard, demo,
interactive report, visualization, calculator, game, or other browser-runnable
work product.

## Goal

Create a useful static artifact that Atrium can capture and preview. Favor
business value and a working artifact over a perfect production architecture.

## Output Contract

- Prefer a single self-contained HTML file for small and medium artifacts.
- Put app artifacts in `shared/apps/<slug>/index.html` when `shared/` exists.
- If `shared/` does not exist, use `/home/agent/workspace/apps/<slug>/index.html`
  or `/home/agent/apps/<slug>/index.html`, whichever exists in the sandbox.
- Keep CSS and JavaScript inline for simple apps so capture and preview are easy.
- Use ordinary browser APIs and plain JavaScript when that is enough.
- For React or TypeScript apps, build to static HTML/JS and present the built
  `index.html`. Do not present `node_modules`, source maps, caches, or lockfiles.
- Keep generated artifacts reasonably small. Large bundles may be captured as
  metadata only and may not preview.

## Presentation

After creating the artifact, present it explicitly:

```bash
atrium-present shared/apps/<slug>/index.html --renderer html-app --title "Short Title"
```

Use `--description` for a short explanation when useful.

For React source-only experiments, present the source with:

```bash
atrium-present shared/apps/<slug>/App.jsx --renderer react-jsx --title "Short Title"
```

Prefer `html-app` unless the user specifically asks to inspect React source.

## Runtime Assumptions

Atrium artifact app previews are browser-only static apps:

- Do not require a backend server.
- Do not require API keys in the browser.
- Do not require localStorage or sessionStorage.
- Do not depend on external network requests unless the user explicitly asks for
  a quick prototype and accepts that it may not work in a locked-down preview.
- Embed sample data or already-approved fetched data directly into the artifact.

## Smoke Test

Before presenting, do at least one lightweight check:

- open the file with a local static server when available;
- run the build command for project-based apps;
- or inspect the HTML for obvious syntax/path mistakes.

If a check fails and you cannot fix it quickly, present the best available
artifact and explain the limitation.
