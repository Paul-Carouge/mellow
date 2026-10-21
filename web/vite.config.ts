import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";

// `npm run dev` proxies the API and the gateway to a local instance, so the browser sees one
// origin, as in production where the server binary serves this client itself (ADR-0019).
const DEVELOPMENT_INSTANCE = process.env.MELLOW_DEV_INSTANCE ?? "http://127.0.0.1:18080";

export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  server: {
    port: 5173,
    strictPort: true,
    // The instance accepts the session cookie only from its own origin, so the proxy presents
    // requests as coming from it.
    proxy: {
      "/api": { target: DEVELOPMENT_INSTANCE, changeOrigin: true, headers: { origin: DEVELOPMENT_INSTANCE } },
      "/gateway": {
        target: DEVELOPMENT_INSTANCE,
        changeOrigin: true,
        ws: true,
        headers: { origin: DEVELOPMENT_INSTANCE },
      },
    },
    // Figtree and the Phosphor icons are bundled with the desktop client; the web client reads the same files.
    fs: { allow: [".", "../crates/mellow-desktop/fonts", "../crates/mellow-desktop/icons"] },
  },
  build: { target: "es2022", sourcemap: false },
});
