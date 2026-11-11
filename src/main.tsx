import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { IS_WEB_BUILD } from "./env";

async function boot() {
  // The menu bar panel is the same app in a compact window (see tray.rs).
  if (new URLSearchParams(window.location.search).get("view") === "tray") {
    document.documentElement.classList.add("tc-tray");
  }
  if (IS_WEB_BUILD) {
    document.documentElement.classList.add("tc-web");
    // Offline launch (see vite.config.ts); needs HTTPS (or localhost).
    if (import.meta.env.PROD && "serviceWorker" in navigator) {
      navigator.serviceWorker.register("./sw.js").catch(() => {});
    }
  }
  // Dev only (stripped from builds): ?mock=off|primary|viewer runs the
  // desktop UI in a plain browser against a fake Tauri backend.
  if (import.meta.env.DEV && !IS_WEB_BUILD) {
    const mock = new URLSearchParams(window.location.search).get("mock");
    if (mock) await (await import("./dev/tauriMock")).installTauriMock(mock);
  }
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

void boot();
