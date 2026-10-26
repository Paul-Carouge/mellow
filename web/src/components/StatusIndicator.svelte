<script lang="ts" module>
  import type { ChosenStatus, PresenceStatus } from "../lib/protocol";

  export type ShownStatus = PresenceStatus | ChosenStatus;

  const COLOR_TOKEN_BY_STATUS: Record<ShownStatus, string> = {
    online: "--mw-component-status-online",
    idle: "--mw-component-status-idle",
    do_not_disturb: "--mw-component-status-dnd",
    offline: "--mw-component-status-offline",
    invisible: "--mw-component-status-offline",
  };

  // On a 20-unit grid, as Discord draws them; the inner shapes are true holes (even-odd), so the
  // surface behind the indicator shows through whatever its colour.
  const DISC = "M10 0a10 10 0 1 1 0 20a10 10 0 1 1 0-20Z";
  const PATH_BY_STATUS: Record<ShownStatus, string> = {
    online: DISC,
    // The disc minus a 7.5-radius circle centred at (5, 5): the arcs meet where both circles cross.
    idle: "M10.607 .018A10 10 0 1 1 .018 10.607A7.5 7.5 0 0 0 10.607 .018Z",
    do_not_disturb: `${DISC}M5 7.5h10a2.5 2.5 0 0 1 0 5H5a2.5 2.5 0 0 1 0-5Z`,
    offline: `${DISC}M10 5a5 5 0 1 0 0 10a5 5 0 1 0 0-10Z`,
    invisible: `${DISC}M10 5a5 5 0 1 0 0 10a5 5 0 1 0 0-10Z`,
  };
</script>

<script lang="ts">
  // DESIGN.md §2.3 and §6.9: a shape per status (dot, crescent, barred disc, ring), so colour
  // never carries it alone. Invisible looks offline, as others see it. Decorative: the host names
  // the status in words.
  interface Props {
    status: ShownStatus;
    /** A `--mw-*` length token, for example "--mw-component-status-size-small". */
    sizeToken: string;
  }

  let { status, sizeToken }: Props = $props();
</script>

<svg
  aria-hidden="true"
  viewBox="0 0 20 20"
  class="block shrink-0"
  style:width="var({sizeToken})"
  style:height="var({sizeToken})"
  style:color="var({COLOR_TOKEN_BY_STATUS[status]})"
>
  <path fill="currentColor" fill-rule="evenodd" d={PATH_BY_STATUS[status]} />
</svg>
