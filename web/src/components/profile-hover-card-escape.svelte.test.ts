// Esc closes a card opened by the pointer, wherever focus is, and nothing else sees that Esc
// (WCAG 1.4.13, review 2026-11-14).
import { expect, it, vi } from "vitest";

vi.stubGlobal("getComputedStyle", () => ({
  getPropertyValue: (name: string) => (name.endsWith("open-delay") ? "450ms" : name.endsWith("close-delay") ? "200ms" : ""),
  fontSize: "16px",
}));
vi.stubGlobal("document", { documentElement: {} });

const windowListeners = new Map<string, (event: unknown) => void>();
vi.stubGlobal("window", {
  addEventListener: (type: string, listener: (event: unknown) => void) => windowListeners.set(type, listener),
  removeEventListener: (type: string) => windowListeners.delete(type),
});

function fakeElement() {
  const listeners = new Map<string, (event: unknown) => void>();
  return {
    isConnected: true,
    addEventListener: (type: string, listener: (event: unknown) => void) => listeners.set(type, listener),
    removeEventListener: (type: string) => listeners.delete(type),
    fire: (type: string, event: unknown) => listeners.get(type)?.(event),
  };
}

const user = { id: "1", username: "ana", display_name: "Ana", avatar: null, banner: null };

it("closes a card the pointer opened on Esc, and keeps the key from the side sheet or a drawer", async () => {
  vi.useFakeTimers();
  const surfaces = await import("./profile-surfaces.svelte");
  const row = fakeElement();
  surfaces.profileTrigger(user as never, { placement: "beside" })(row as never);
  row.fire("pointerenter", { pointerType: "mouse" });
  vi.advanceTimersByTime(450);
  expect(surfaces.shownProfileHoverCard()).not.toBeNull();

  const escape = { key: "Escape", preventDefault: vi.fn(), stopPropagation: vi.fn() };
  windowListeners.get("keydown")?.(escape);
  expect(surfaces.shownProfileHoverCard()).toBeNull();
  expect(escape.preventDefault).toHaveBeenCalled();
  expect(escape.stopPropagation).toHaveBeenCalled();
  // Closed, the card no longer takes Esc from anyone.
  expect(windowListeners.has("keydown")).toBe(false);
});
