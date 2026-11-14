// A row removed while its card waits must not leave a card stuck at the window's corner (review, 2026-11-14).
import { expect, it, vi } from "vitest";

vi.stubGlobal("getComputedStyle", () => ({
  getPropertyValue: (name: string) => (name.endsWith("open-delay") ? "450ms" : name.endsWith("close-delay") ? "200ms" : ""),
  fontSize: "16px",
}));
vi.stubGlobal("document", { documentElement: {} });

function fakeElement() {
  const listeners = new Map<string, (event: unknown) => void>();
  return {
    isConnected: true,
    addEventListener: (type: string, listener: (event: unknown) => void) => listeners.set(type, listener),
    removeEventListener: (type: string) => listeners.delete(type),
    fire: (type: string, event: unknown) => listeners.get(type)?.(event),
  };
}

const user = { id: "1", username: "ana", display_name: null, avatar: null, banner: null };

it("a row removed while its card waits to open does not open a card for it", async () => {
  vi.useFakeTimers();
  const surfaces = await import("./profile-surfaces.svelte");
  const row = fakeElement();
  const detach = surfaces.profileTrigger(user as never, { placement: "beside" })(row as never) as () => void;
  row.fire("pointerenter", { pointerType: "mouse" });
  vi.advanceTimersByTime(100);
  // Presence change regroups the member list: the keyed row is destroyed, no pointerleave.
  row.isConnected = false;
  detach();
  await Promise.resolve();
  vi.advanceTimersByTime(450);
  const shown = surfaces.shownProfileHoverCard();
  expect(shown === null ? null : { anchorConnected: (shown.anchor as unknown as { isConnected: boolean }).isConnected }).toBeNull();
});
