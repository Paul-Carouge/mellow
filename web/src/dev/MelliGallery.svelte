<script lang="ts">
  import Melli from "../components/Melli.svelte";
  import type { MelliState } from "../components/melli";

  // Development only (web/dev/melli.html): every Melli state side by side, at the sizes it ships
  // at, to tweak them against DESIGN.md §2.10.
  const STATE_USES: Record<MelliState, string> = {
    idle: "Home button, nothing to report. Also the logo and the favicon.",
    connecting: "Home button while the client connects to the instance.",
    "connection-lost": "Home button while the connection is lost and the client reconnects.",
    notification: "Home button while a direct message is unread.",
    "incoming-call": "Not wired yet: someone is calling.",
    "in-call": "Not wired yet: you are in a call.",
    "update-available": "Not wired yet: a new version is ready to install.",
    sleeping: "Not wired yet: you are away or idle.",
  };
  const states = Object.keys(STATE_USES) as MelliState[];

  let theme = $state<"system" | "dark" | "light">("system");
  let isStill = $state(false);

  $effect(() => {
    if (theme === "system") {
      delete document.documentElement.dataset["theme"];
    } else {
      document.documentElement.dataset["theme"] = theme;
    }
  });
</script>

<main class="gallery min-h-dvh bg-surface-chat px-lg py-xl text-text-normal">
  <header class="mx-auto flex max-w-6xl flex-wrap items-end justify-between gap-md">
    <div>
      <h1 class="type-heading-xl text-text-heading">Melli</h1>
      <p class="type-body text-text-muted">Every state at preview size and at Home button size, on the rail at rest and on apricot when selected. The tab shows the favicon.</p>
    </div>
    <div class="flex flex-wrap items-center gap-lg type-body">
      <fieldset class="flex items-center gap-sm">
        <legend class="sr-only">Theme</legend>
        {#each ["system", "dark", "light"] as const as option (option)}
          <label class="flex min-h-(--mw-size-hit-web-min) items-center gap-2xs">
            <input type="radio" name="theme" value={option} bind:group={theme} />
            {option === "system" ? "Match system" : option === "dark" ? "Dark" : "Light"}
          </label>
        {/each}
      </fieldset>
      <label class="flex min-h-(--mw-size-hit-web-min) items-center gap-2xs">
        <input type="checkbox" bind:checked={isStill} />
        Still poses (as with reduced motion)
      </label>
    </div>
  </header>

  <ul class="mx-auto mt-xl grid max-w-6xl gap-lg [grid-template-columns:repeat(auto-fill,minmax(var(--gallery-card-min-width),1fr))]">
    {#each states as state (state)}
      <li class="flex flex-col gap-md rounded-lg bg-surface-sidebar p-md">
        <Melli {state} {isStill} sizeToken="--gallery-melli-preview-size" />
        <div class="flex items-center gap-md rounded-(--mw-component-rail-icon-radius-active) bg-(--mw-component-rail-background) p-sm">
          <span class="grid size-(--mw-component-rail-item-size) place-items-center">
            <Melli {state} {isStill} sizeToken="--mw-component-rail-item-size" />
          </span>
          <span class="grid size-(--mw-component-rail-item-size) place-items-center overflow-hidden rounded-(--mw-component-rail-icon-radius-active) bg-(--mw-component-rail-icon-background-active) [--melli-mark:var(--mw-color-mascot-ink)]">
            <Melli {state} {isStill} sizeToken="--mw-component-rail-item-size" />
          </span>
        </div>
        <div>
          <h2 class="type-heading-md text-text-heading"><code>{state}</code></h2>
          <p class="type-body-sm text-text-muted">{STATE_USES[state]}</p>
        </div>
      </li>
    {/each}
  </ul>
</main>

<style>
  /* Derived from the rail icon so the page holds no raw sizes: 192 and 240 px at 48 px. */
  .gallery {
    --gallery-melli-preview-size: calc(var(--mw-component-rail-item-size) * 4);
    --gallery-card-min-width: calc(var(--mw-component-rail-item-size) * 5);
  }
</style>
