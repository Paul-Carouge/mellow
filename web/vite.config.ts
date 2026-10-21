import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";

// `npm run dev` proxies the API and the gateway to a local instance, so the browser sees one
// origin, as in production where the server binary serves this client itself (ADR-0019).
const DEVELOPMENT_INSTANCE = process.env.MELLOW_DEV_INSTANCE ?? "http://127.0.0.1:18080";
const DEVELOPMENT_PORT = 5173;
const DEVELOPMENT_ORIGINS = new Set([
  `http://127.0.0.1:${DEVELOPMENT_PORT}`,
  `http://localhost:${DEVELOPMENT_PORT}`,
]);

/** Only this dev server's own pages are presented as the instance's origin; other sites keep theirs. */
function presentDevelopmentPagesAsTheInstance(request: { getHeader(name: string): unknown; setHeader(name: string, value: string): void }): void {
  const origin = request.getHeader("origin");
  if (typeof origin === "string" && DEVELOPMENT_ORIGINS.has(origin)) {
    request.setHeader("origin", DEVELOPMENT_INSTANCE);
  }
}

export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  server: {
    port: DEVELOPMENT_PORT,
    strictPort: true,
    // The instance accepts the session cookie only from its own origin, so the proxy presents
    // requests as coming from it.
    proxy: {
      "/api": {
        target: DEVELOPMENT_INSTANCE,
        changeOrigin: true,
        configure: (proxy) => proxy.on("proxyReq", presentDevelopmentPagesAsTheInstance),
      },
      "/gateway": {
        target: DEVELOPMENT_INSTANCE,
        changeOrigin: true,
        ws: true,
        configure: (proxy) => proxy.on("proxyReqWs", presentDevelopmentPagesAsTheInstance),
      },
    },
    // Figtree and the Phosphor icons are bundled with the desktop client; the web client reads the same files.
    fs: { allow: [".", "../crates/mellow-desktop/fonts", "../crates/mellow-desktop/icons"] },
  },
  build: { target: "es2022", sourcemap: false },
});
