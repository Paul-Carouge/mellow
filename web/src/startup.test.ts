import { expect, it, vi } from "vitest";

// `main.ts` imports the app before its catalogue is loaded, so no module the app imports may read
// `strings` at its top level (it would throw on `undefined`).
it("evaluates every module the app starts with before the catalogue is loaded", async () => {
  vi.resetModules();
  const i18n = await import("./i18n");
  expect(i18n.strings).toBeUndefined();
  await expect(import("./App.svelte")).resolves.toBeDefined();
});
