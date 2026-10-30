<script lang="ts">
  import { strings } from "../i18n";
  import { playChimeSound, prepareChimeSound } from "./chime-sound";
  import { notificationSettings, updateNotificationSettings } from "./notification-preferences.svelte";
  import { askNotificationPermission, notificationSupport } from "./notifier.svelte";
  import Switch from "./Switch.svelte";

  // User settings → Notifications (docs/design/web-notifications.md): desktop notifications with
  // the browser's permission said in words, previews and the sound. Per-place choices are in the
  // menus; all of them stay in this browser.

  const id = $props.id();
  const settings = $derived(notificationSettings());
  const support = $derived(notificationSupport());
  const isDesktopLocked = $derived(support === "denied" || support === "unsupported");
  const desktopDescription = $derived(
    support === "unsupported"
      ? strings.notifications.desktopUnsupported
      : support === "denied"
        ? strings.notifications.desktopBlocked
        : support === "granted"
          ? strings.notifications.desktopAllowed
          : strings.notifications.desktopAsk,
  );

  async function changeDesktopNotifications(isOn: boolean) {
    // The browser asks from this click; turning off never asks anything.
    const answer = isOn && support === "default" ? await askNotificationPermission() : support;
    updateNotificationSettings((current) => ({
      ...current,
      hasAnsweredPermissionBar: true,
      areDesktopNotificationsEnabled: isOn && answer === "granted",
    }));
  }

  async function changeSound(isOn: boolean) {
    updateNotificationSettings((current) => ({ ...current, isSoundEnabled: isOn }));
    if (isOn) {
      // Turning it on plays it once, so the reader knows what they chose.
      await prepareChimeSound();
      playChimeSound();
    }
  }
</script>

{#snippet switchRow(key: string, label: string, description: string, isOn: boolean, isLocked: boolean, onchange: (isOn: boolean) => void)}
  <li class="flex items-center gap-md border-b border-border-divider py-sm">
    <div class="min-w-0 flex-1">
      <p id="{id}-{key}" class="type-channel-name text-text-heading">{label}</p>
      <p id="{id}-{key}-description" class="type-body text-text-muted">{description}</p>
    </div>
    <Switch {isOn} {isLocked} labelledBy="{id}-{key}" describedBy="{id}-{key}-description" {onchange} />
  </li>
{/snippet}

<h2 class="type-heading-lg text-(--mw-component-settings-page-title-text)">{strings.notifications.title}</h2>
<p class="mt-(--mw-component-settings-page-title-gap) type-body-lg text-(--mw-component-settings-body-text)">
  {strings.notifications.intro}
</p>
<ul class="mt-(--mw-component-settings-section-gap)">
  {@render switchRow(
    "desktop",
    strings.notifications.desktop,
    desktopDescription,
    settings.areDesktopNotificationsEnabled && support === "granted",
    isDesktopLocked,
    (isOn) => void changeDesktopNotifications(isOn),
  )}
  {@render switchRow(
    "previews",
    strings.notifications.previews,
    strings.notifications.previewsDescription,
    settings.shouldShowPreviews,
    false,
    (isOn) => updateNotificationSettings((current) => ({ ...current, shouldShowPreviews: isOn })),
  )}
  {@render switchRow(
    "sound",
    strings.notifications.sound,
    strings.notifications.soundDescription,
    settings.isSoundEnabled,
    false,
    (isOn) => void changeSound(isOn),
  )}
</ul>
