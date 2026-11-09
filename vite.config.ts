import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
// @ts-expect-error node builtins; this project has no @types/node
import { readFileSync } from "node:fs";
// @ts-expect-error node builtins; this project has no @types/node
import { resolve } from "node:path";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// Phone web app (`vite build --mode web` → dist-web/): iOS home-screen tags,
// no Google Fonts request (the UI uses the system font). The desktop
// index.html is left exactly as it is.
//
// No web manifest on purpose: a manifest start_url would drop the `#pair=`
// fragment an iOS home-screen app must read its pairing code from (its
// storage is separate from Safari's).
function webAppHtml(): Plugin {
  const tags = [
    '<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover" />',
    '<meta name="apple-mobile-web-app-capable" content="yes" />',
    '<meta name="mobile-web-app-capable" content="yes" />',
    '<meta name="apple-mobile-web-app-status-bar-style" content="black-translucent" />',
    '<meta name="apple-mobile-web-app-title" content="Tokens" />',
    '<meta name="application-name" content="Tokens" />',
    '<meta name="theme-color" media="(prefers-color-scheme: light)" content="#9A5644" />',
    '<meta name="theme-color" media="(prefers-color-scheme: dark)" content="#24171A" />',
    '<meta name="color-scheme" content="light dark" />',
    '<meta name="format-detection" content="telephone=no" />',
    '<meta name="referrer" content="no-referrer" />',
    '<link rel="apple-touch-icon" href="apple-touch-icon.png" />',
    '<link rel="icon" type="image/png" sizes="192x192" href="icon-192.png" />',
  ].join("\n    ");
  return {
    name: "token-counter-web-html",
    transformIndexHtml(html) {
      return html
        .replace(/<link rel="icon"[^>]*\/>\s*/, "")
        .replace(/<meta name="viewport"[^>]*\/>/, tags)
        .replace(/\s*<link rel="preconnect"[^>]*\/>/g, "")
        .replace(/\s*<link\s+href="https:\/\/fonts\.googleapis\.com[^>]*\/>/, "")
        .replace("<title>Token Counter</title>", "<title>Tokens</title>");
    },
  };
}

// Offline launch for the phone web app: the snapshot itself is kept in
// localStorage, but the page has to load without a network too. Navigations
// are network-first (so a new deploy shows up at once; 4 s timeout, then the
// cached page), same-origin files cache-first. Cross-origin requests (the
// sync server) are never touched. Emitted only by `vite build --mode web`.
const SW_SOURCE = `
const CACHE = 'tokens-__VERSION__';
const PRECACHE = __PRECACHE__;
const scoped = (p) => new URL(p, self.registration.scope).href;

self.addEventListener('install', (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(PRECACHE.map(scoped))).then(() => self.skipWaiting()));
});

self.addEventListener('activate', (e) => {
  e.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(keys.filter((k) => k.startsWith('tokens-') && k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  );
});

function fromNetwork(req, key) {
  return fetch(req).then((res) => {
    if (res.ok) {
      const copy = res.clone();
      caches.open(CACHE).then((c) => c.put(key || req, copy));
    }
    return res;
  });
}

self.addEventListener('fetch', (e) => {
  const req = e.request;
  if (req.method !== 'GET') return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;
  if (req.mode === 'navigate') {
    const page = scoped('./');
    const net = fromNetwork(req, page);
    const timeout = new Promise((resolve) => setTimeout(resolve, 4000));
    e.respondWith(
      Promise.race([net.catch(() => null), timeout])
        .then((res) => res || caches.match(page))
        .then((res) => res || net),
    );
    return;
  }
  e.respondWith(caches.match(req).then((hit) => hit || fromNetwork(req)));
});
`;

function fnv1a(text: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h.toString(16).padStart(8, "0");
}

function webServiceWorker(): Plugin {
  let publicDir = "";
  return {
    name: "token-counter-web-sw",
    apply: "build",
    enforce: "post",
    configResolved(config) {
      publicDir = config.publicDir;
    },
    generateBundle(_options, bundle) {
      const files = Object.keys(bundle).filter((f) => !f.endsWith(".map") && f !== "index.html");
      const icons = ["apple-touch-icon.png", "icon-192.png"];
      const precache = ["./", ...files.sort(), ...icons];
      // Bundle file names carry a content hash, but the icons keep fixed names
      // and same-origin files are served cache-first, so their bytes must move
      // the version too or a new icon never replaces the cached one.
      const iconBytes = icons.map((f) => readFileSync(resolve(publicDir, f)).toString("base64"));
      const version = fnv1a([...precache, ...iconBytes].join("\n"));
      this.emitFile({
        type: "asset",
        fileName: "sw.js",
        source: SW_SOURCE.replace("__VERSION__", version).replace("__PRECACHE__", JSON.stringify(precache)),
      });
    },
  };
}

// https://vite.dev/config/
export default defineConfig(async ({ mode }) => {
  const web = mode === "web";
  return {
    plugins: web ? [react(), webAppHtml(), webServiceWorker()] : [react()],

    // The web app is static files that may live under any path (relative
    // asset URLs); its own public/ holds the icons.
    ...(web
      ? {
          base: "./",
          publicDir: "web/public",
          build: { outDir: "dist-web", emptyOutDir: true },
        }
      : {}),

    // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
    //
    // 1. prevent Vite from obscuring rust errors
    clearScreen: false,
    // 2. tauri expects a fixed port, fail if that port is not available
    server: {
      port: 1420,
      strictPort: true,
      host: host || false,
      hmr: host
        ? {
            protocol: "ws",
            host,
            port: 1421,
          }
        : undefined,
      watch: {
        // 3. tell Vite to ignore watching `src-tauri`
        ignored: ["**/src-tauri/**"],
      },
    },
  };
});
