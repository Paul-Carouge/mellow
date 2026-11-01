/// <reference types="vitest/config" />
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { brotliCompressSync, constants as zlibConstants, gzipSync } from "node:zlib";
import { defineConfig, type Plugin } from "vite";

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

// The server binary embeds `dist` and serves these siblings by content negotiation (ADR-0019), so
// it never compresses at runtime. PNG is already compressed.
const PRECOMPRESSED_EXTENSIONS = [".js", ".css", ".html", ".svg", ".json", ".webmanifest", ".ttf", ".wasm"];
/** A sibling is kept only when it saves more than this share of the original's bytes. */
const MAXIMUM_PRECOMPRESSED_SIZE_RATIO = 0.9;
const GZIP_MAXIMUM_LEVEL = 9;
/** The only face the first screen renders; the italic loads when a message needs it. */
const PRELOADED_FONT_SOURCE_NAME = "Figtree[wght].ttf";

function listFilesRecursively(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? listFilesRecursively(path) : [path];
  });
}

/** Writes `name.br` and `name.gz` next to each compressible file of the build. */
function precompressBuildOutput(): Plugin {
  let outputDirectory = "";
  return {
    name: "mellow-precompress-build-output",
    apply: "build",
    configResolved(config) {
      outputDirectory = join(config.root, config.build.outDir);
    },
    closeBundle() {
      const compressibleFiles = listFilesRecursively(outputDirectory).filter((path) =>
        PRECOMPRESSED_EXTENSIONS.some((extension) => path.endsWith(extension)),
      );
      for (const path of compressibleFiles) {
        const original = readFileSync(path);
        const compressedSiblings: [string, Buffer][] = [
          [
            ".br",
            brotliCompressSync(original, {
              params: {
                [zlibConstants.BROTLI_PARAM_QUALITY]: zlibConstants.BROTLI_MAX_QUALITY,
                [zlibConstants.BROTLI_PARAM_SIZE_HINT]: original.length,
              },
            }),
          ],
          [".gz", gzipSync(original, { level: GZIP_MAXIMUM_LEVEL })],
        ];
        for (const [suffix, compressed] of compressedSiblings) {
          if (compressed.length < original.length * MAXIMUM_PRECOMPRESSED_SIZE_RATIO) {
            writeFileSync(path + suffix, compressed);
          }
        }
      }
    },
  };
}

/** The font is referenced only from the stylesheet; preloading it starts its download with the CSS. */
function preloadUprightFont(): Plugin {
  return {
    name: "mellow-preload-upright-font",
    apply: "build",
    transformIndexHtml: {
      order: "post",
      handler(_html, context) {
        const font = Object.values(context.bundle ?? {}).find(
          (output) =>
            output.type === "asset" &&
            output.originalFileNames.some((name) => name.endsWith(PRELOADED_FONT_SOURCE_NAME)),
        );
        if (font === undefined) {
          throw new Error(`the build has no ${PRELOADED_FONT_SOURCE_NAME} to preload`);
        }
        return [
          {
            tag: "link",
            attrs: { rel: "preload", as: "font", type: "font/ttf", href: `/${font.fileName}`, crossorigin: true },
            injectTo: "head",
          },
        ];
      },
    },
  };
}

export default defineConfig({
  plugins: [tailwindcss(), svelte(), preloadUprightFont(), precompressBuildOutput()],
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
  test: { setupFiles: ["src/test-setup.ts"] },
});
