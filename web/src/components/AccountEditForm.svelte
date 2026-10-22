<script lang="ts">
  import type { Snippet } from "svelte";

  import { strings } from "../i18n";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";

  // The form an Edit button opens in place (DESIGN.md §5.9, My account): its fields, the error it
  // raised, then Cancel and Save. Enter in a field saves once Save is enabled.
  interface Props {
    /** Names the form for screen readers, e.g. "Change username". */
    label: string;
    saveLabel: string;
    /** Replaces the save label while busy (DESIGN.md §6.1, loading state). */
    busyLabel: string;
    isSaveEnabled: boolean;
    isBusy: boolean;
    /** Empty hides it. */
    errorMessage: string;
    onsave: () => void;
    oncancel: () => void;
    children: Snippet;
  }

  let { label, saveLabel, busyLabel, isSaveEnabled, isBusy, errorMessage, onsave, oncancel, children }: Props =
    $props();
</script>

<form
  aria-label={label}
  aria-busy={isBusy}
  class="flex flex-col gap-(--mw-component-text-field-field-gap)"
  onsubmit={(event) => {
    event.preventDefault();
    if (!isBusy && isSaveEnabled) {
      onsave();
    }
  }}
>
  {@render children()}

  <!-- Kept in the form, so the error is announced as soon as it appears. -->
  <div role="alert" class="contents">
    {#if errorMessage}
      <p class="flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-settings-error-text)">
        <Icon
          name="warning-circle"
          class="size-(--mw-component-settings-error-icon-size) text-(--mw-component-settings-error-icon)"
        />
        <span>{errorMessage}</span>
      </p>
    {/if}
  </div>

  <div class="flex flex-wrap justify-end gap-(--mw-component-settings-account-button-gap)">
    <Button isDisabled={isBusy} onclick={oncancel}>{strings.common.cancel}</Button>
    <Button type="submit" variant="primary" isDisabled={!isSaveEnabled || isBusy}>{isBusy ? busyLabel : saveLabel}</Button>
  </div>
</form>
