<script lang="ts">
  import { strings } from "../i18n";
  import type { ConnectionStatus } from "../lib/client.svelte";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import { notificationSettings, updateNotificationSettings } from "./notification-preferences.svelte";
  import { askNotificationPermission, notificationSupport } from "./notifier.svelte";

  // The one-time invitation to turn on desktop notifications (docs/design/web-notifications.md):
  // shown once connected, while the browser was never asked and the reader never answered it.
  // The browser's prompt only ever opens from "Turn on notifications", never by itself.
  interface Props {
    status: ConnectionStatus;
    /** Phones and narrow windows: one 48 px row with short words, the sentence kept as its name. */
    isCompact?: boolean;
  }

  let { status, isCompact = false }: Props = $props();

  let announcement = $state("");
  const isShown = $derived(
    status === "connected" && notificationSupport() === "default" && !notificationSettings().hasAnsweredPermissionBar,
  );

  async function turnOn() {
    const answer = await askNotificationPermission();
    updateNotificationSettings((settings) => ({
      ...settings,
      hasAnsweredPermissionBar: true,
      areDesktopNotificationsEnabled: answer === "granted" || settings.areDesktopNotificationsEnabled,
    }));
    // A prompt closed without an answer ("default") blocked nothing: the settings switch asks again.
    announcement =
      answer === "granted"
        ? strings.notifications.turnedOn
        : answer === "denied"
          ? strings.notifications.refused
          : strings.notifications.askLater;
  }

  function dismiss() {
    updateNotificationSettings((settings) => ({ ...settings, hasAnsweredPermissionBar: true }));
  }
</script>

<!-- Always in the page: the result is read out after the bar has gone. -->
<p class="sr-only" role="status">{announcement}</p>
{#if isShown}
  <section
    aria-label={isCompact ? strings.notifications.barText : strings.notifications.title}
    class={[
      "flex items-center border-s-(length:--mw-component-notice-bar-invite-bar-width) border-(--mw-component-notice-bar-invite-bar) bg-(--mw-component-notice-bar-invite-background) text-(--mw-component-notice-bar-invite-text)",
      isCompact
        ? "gap-xs ps-(--mw-component-notice-bar-padding-inline) pe-2xs py-3xs"
        : "flex-wrap justify-end gap-x-md gap-y-2xs px-(--mw-component-notice-bar-padding-inline) py-(--mw-component-notice-bar-padding-block)",
    ]}
  >
    <p class="flex min-w-0 flex-[1_1_auto] items-center gap-(--mw-component-notice-bar-icon-gap) type-body">
      <Icon name="bell" class="size-(--mw-component-notice-bar-icon-size) shrink-0" />
      <!-- Hidden from screen readers when short: the section's name already says the sentence. -->
      <span class="min-w-0" aria-hidden={isCompact || undefined}
        >{isCompact ? strings.notifications.barTextShort : strings.notifications.barText}</span
      >
    </p>
    <div class="flex shrink-0 items-center gap-2xs">
      <Button variant="primary" aria-label={isCompact ? strings.notifications.turnOn : undefined} onclick={() => void turnOn()}
        >{isCompact ? strings.notifications.turnOnShort : strings.notifications.turnOn}</Button
      >
      <IconButton
        icon="x"
        label={strings.notifications.notNow}
        class="size-(--mw-size-hit-web-min) rounded-full hover:bg-state-hover"
        iconClass="size-(--mw-component-notice-bar-icon-size)"
        onclick={dismiss}
      />
    </div>
  </section>
{/if}
