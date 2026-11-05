<script lang="ts">
  import { onDestroy, tick } from "svelte";

  import { strings } from "../i18n";
  import Button from "./Button.svelte";
  import { durationTokenInMilliseconds } from "./tokens";

  // The user's own friend code (ADR-0048), with a copy button: shown in My account and under "Add
  // friend", so it can be shared where friends are added. The code never changes.
  interface Props {
    code: string;
  }

  let { code }: Props = $props();

  const id = $props.id();
  let isCopied = $state(false);
  let announcement = $state("");
  let copyError = $state("");
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;

  onDestroy(() => clearTimeout(copiedTimer));

  async function copy() {
    copyError = "";
    try {
      await navigator.clipboard.writeText(code);
    } catch (error) {
      console.warn("copying the friend code failed", error);
      copyError = strings.friendCode.copyFailed;
      return;
    }
    isCopied = true;
    // Emptied first, so a second copy is announced again.
    announcement = "";
    await tick();
    announcement = strings.friendCode.copied;
    clearTimeout(copiedTimer);
    const copiedLabelDuration = durationTokenInMilliseconds("component-dialog-copied-label-duration");
    if (copiedLabelDuration > 0) {
      copiedTimer = setTimeout(() => (isCopied = false), copiedLabelDuration);
    }
  }
</script>

<div class="flex flex-col gap-(--mw-component-settings-section-label-gap)">
  <div class="flex items-center gap-(--mw-component-settings-account-button-gap)">
    <div class="flex min-w-0 flex-1 flex-col gap-(--mw-component-settings-account-value-gap)">
      <h3 id="{id}-label" class="type-category text-(--mw-component-settings-section-label-text)">{strings.friendCode.label}</h3>
      <!-- Selectable whole, for copying by hand. -->
      <p aria-labelledby="{id}-label" class="type-code select-all text-(--mw-component-settings-account-value-text)">{code}</p>
    </div>
    <Button aria-describedby="{id}-hint" onclick={() => void copy()}>
      {isCopied ? strings.friendCode.copiedShort : strings.friendCode.copy}
    </Button>
  </div>
  <p id="{id}-hint" class="type-body-sm text-text-secondary">{strings.friendCode.hint}</p>
  {#if copyError}
    <p role="alert" class="type-body-sm text-text-danger">{copyError}</p>
  {/if}
  <p class="sr-only" aria-live="polite">{announcement}</p>
</div>
