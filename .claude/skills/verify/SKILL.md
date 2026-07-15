---
name: verify
description: Build, launch, and drive stvorhy-vocalboard (Tauri + SolidJS v2 beta frontend) to verify frontend changes at runtime
---

# Verifying stvorhy-vocalboard

Tauri 2 app with a SolidJS **v2 beta** frontend (bun + vite). Frontend changes can be verified without compiling the Rust shell.

## Build / launch

- `bun install` — deps (bun.lock is the lockfile)
- `bun run build` — production build to `dist/` (vite)
- `bun run dev` — vite dev server on **port 1420, strictPort** (same path `tauri dev` uses). Kill anything on 1420 first.
- Full desktop shell needs `bun run tauri dev` (Rust toolchain) — only needed for IPC/window changes, not frontend logic.

## Drive the frontend without Tauri

The built bundle runs in happy-dom; stub the IPC before importing it:

```js
window.__TAURI_INTERNALS__ = {
  metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  transformCallback: (cb) => { const id = Math.floor(Math.random() * 1e9); window[`_${id}`] = cb; return id; },
  invoke: async (cmd, args) => { /* per-command stub, e.g. greet -> `Hello, ${args.name}!...` */ },
};
document.body.innerHTML = '<div id="root"></div>';
await import(pathToFileURL("dist/assets/index-<hash>.js").href); // hash from dist/assets/
```

Then dispatch real DOM events (`change`, `submit`) and assert on the DOM. A worked example lives in a past session's scratchpad `domtest/run.js`.

## Gotchas

- **Solid v2 auto-batches signal writes**: dispatching two events synchronously (e.g. `change` then `submit` with no `await` between) makes the second handler read the *stale* signal value. Always `await new Promise(r => setTimeout(r))` between dispatched events — this matches real browser timing.
- Dev-transform check: `curl http://localhost:1420/src/App.jsx` returns the compiled module — a 500 here means the new `@dom-expressions/jsx-compiler` rejected the JSX.
- Solid v2 packages are exact-pinned; `^2.0.0-beta.x` ranges would pull `2.0.0-experimental.*` (semver sorts experimental after beta). Don't loosen them.
