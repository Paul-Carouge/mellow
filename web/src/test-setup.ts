// Tests read the interface in English whatever the machine's language: `i18n` chooses it from
// `navigator.languages` when first imported, so it is imported only after the stub. As in
// `main.ts`, its catalogue is loaded before anything reads it.
import { vi } from "vitest";

import { memoryStorage } from "./test-memory-storage";

vi.stubGlobal("navigator", { languages: ["en-GB"] });
const { loadStrings } = await import("./i18n");
await loadStrings();

// Node has its own `localStorage`, which warns and stays empty unless given a file to keep. Tests
// get the empty one a new browser profile has; defined directly rather than stubbed, so a test's
// `vi.unstubAllGlobals()` brings this one back and not Node's.
Object.defineProperty(globalThis, "localStorage", { value: memoryStorage(), configurable: true, writable: true });
