import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "open-glass-ui/styles.css";
import "./styles/theme.css";
import "./styles/global.css";
import "./styles/liquid-surfaces.css";
import { App } from "./App";
import "./styles/refinement.css";
import "./styles/provider-refinement.css";
import "./styles/frosted-sample.css";
import { store, type AppState } from "./lib/store";

// Dev-server convenience: no Tauri IPC in a plain browser, so seed the store
// with representative mock data BEFORE first render to keep the UI browsable
// while iterating on visuals. `#sessions` / `#models` / `#settings` open
// straight on that page. Stripped from production builds (DEV guard +
// tree-shaking).
async function boot() {
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    const { mockState } = await import("./lib/devMock");
    const page = location.hash.slice(1) as AppState["page"];
    store.set(
      ["dashboard", "sessions", "models", "settings"].includes(page)
        ? { ...mockState, page }
        : mockState,
    );
  }
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}

boot();
