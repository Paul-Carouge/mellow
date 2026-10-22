// Tests read the interface in English whatever the machine's language: `i18n` chooses it from
// `navigator.languages` when first imported, which happens after this file runs.
import { vi } from "vitest";

vi.stubGlobal("navigator", { languages: ["en-GB"] });
