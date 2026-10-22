<script lang="ts">
  import { strings } from "../i18n";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";

  // What a settings list shows before it has rows: static skeleton rows while it loads (no
  // shimmer, so nothing moves under reduced motion either), or what failed with "Try again".
  interface Props {
    status: "loading" | "failed";
    /** "Couldn't load the bans." */
    failureText: string;
    onretry: () => void;
  }

  let { status, failureText, onretry }: Props = $props();

  const SKELETON_TEXT_WIDTHS = ["w-2/5", "w-1/3", "w-1/2", "w-1/4"];
</script>

{#if status === "loading"}
  <div role="status" aria-label={strings.serverSettings.loading}>
    {#each SKELETON_TEXT_WIDTHS as width (width)}
      <div
        aria-hidden="true"
        class="flex min-h-(--mw-component-server-settings-row-min-height) items-center gap-(--mw-component-server-settings-row-gap) border-b border-(--mw-component-server-settings-row-divider)"
      >
        <span class="size-(--mw-component-server-settings-avatar-size) shrink-0 rounded-full bg-(--mw-component-server-settings-skeleton-fill)"></span>
        <span class={["h-(--mw-component-server-settings-skeleton-bar-height) rounded-full bg-(--mw-component-server-settings-skeleton-fill)", width]}></span>
      </div>
    {/each}
  </div>
{:else}
  <div role="alert" class="flex flex-col items-start gap-(--mw-component-settings-action-gap)">
    <p class="flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-settings-error-text)">
      <Icon name="warning-circle" class="size-(--mw-component-settings-error-icon-size) shrink-0" />
      <span>{failureText}</span>
    </p>
    <Button onclick={onretry}>{strings.common.tryAgain}</Button>
  </div>
{/if}
