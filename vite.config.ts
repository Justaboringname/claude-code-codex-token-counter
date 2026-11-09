import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";

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
    '<meta name="theme-color" media="(prefers-color-scheme: light)" content="#C9775A" />',
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

// https://vite.dev/config/
export default defineConfig(async ({ mode }) => {
  const web = mode === "web";
  return {
    plugins: web ? [react(), webAppHtml()] : [react()],

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
