<script lang="ts">
  import { untrack } from "svelte";

  import { strings } from "../i18n";
  import {
    MUTE_DURATIONS,
    NOTIFY_LEVELS,
    type NotifyLevel,
    isMuteActive,
    withPlaceSettings,
  } from "../lib/notification-settings";
  import ChoiceOption from "./ChoiceOption.svelte";
  import { upcomingTimeLabel } from "./format";
  import ModalDialog from "./ModalDialog.svelte";
  import { notificationSettings, updateNotificationSettings } from "./notification-preferences.svelte";
  import {
    CURRENT_MUTE_CHOICE as CURRENT_MUTE,
    NOT_MUTED_CHOICE as NOT_MUTED,
    type NotificationTarget,
    SERVER_LEVEL_CHOICE as SERVER_LEVEL,
    defaultNotifyLevel,
    notificationTargetId,
    placeSettingsFromDialog,
    serverNotifyLevel,
  } from "./notifications";

  // "Mute…" and "Notification settings…" from a server's, a channel's or a conversation's menu
  // (docs/design/web-notifications.md): how long it stays quiet, and what it tells about. Nothing
  // changes until Save; the choices stay in this browser.
  interface Props {
    target: NotificationTarget;
    /** "Rust Lyon", "#general", "Weekend crew" or "Ana": what the title and the result name. */
    placeName: string;
    /** Which group takes focus first: the menu item that opened the dialog. */
    initialGroup: "mute" | "level";
    onclose: () => void;
    ondone: (sentence: string) => void;
  }

  let { target, placeName, initialGroup, onclose, ondone }: Props = $props();

  const id = $props.id();
  const now = Date.now();
  // A dialog is about one place for its whole life.
  const fixedTarget = untrack(() => target);
  const targetId = notificationTargetId(fixedTarget);
  const isServerChannel = fixedTarget.kind === "channel";
  const stored = untrack(() => notificationSettings().places[targetId] ?? {});
  const runningMuteEnd = isMuteActive(stored.mutedUntil, now) ? stored.mutedUntil : undefined;
  /** A timed mute still running is a row of its own, so saving does not restart its clock. */
  const timedMuteEnd = typeof runningMuteEnd === "number" ? runningMuteEnd : null;

  let chosenMute = $state<string>(
    runningMuteEnd === undefined ? NOT_MUTED : runningMuteEnd === "never" ? "until-turned-back-on" : CURRENT_MUTE,
  );
  /** What applies without a choice of its own: the server's for a channel, else the default. */
  const fallbackLevel: NotifyLevel = untrack(() =>
    fixedTarget.kind === "channel"
      ? serverNotifyLevel(notificationSettings(), fixedTarget.guildId)
      : defaultNotifyLevel(fixedTarget.kind === "server" ? fixedTarget.guildId : null),
  );
  let chosenLevel = $state<string>(stored.level ?? (isServerChannel ? SERVER_LEVEL : fallbackLevel));

  function save() {
    const place = placeSettingsFromDialog({
      targetKind: fixedTarget.kind,
      chosenMute,
      chosenLevel,
      runningMuteEnd,
      fallbackLevel,
      now: Date.now(),
    });
    updateNotificationSettings((settings) => withPlaceSettings(settings, targetId, place));
    ondone(strings.notifications.saved(placeName));
    onclose();
  }
</script>

<ModalDialog
  title={strings.notifications.dialogTitle(placeName)}
  submitLabel={strings.common.save}
  {onclose}
  onsubmit={save}
>
  <div class="flex flex-col gap-(--mw-component-settings-section-gap)">
    <fieldset class="flex flex-col gap-(--mw-component-choice-list-gap)" aria-describedby="{id}-mute-description">
      <legend class="mb-(--mw-component-text-field-label-gap) type-category text-(--mw-component-text-field-label-text)">
        {strings.notifications.muteLegend}
      </legend>
      <p id="{id}-mute-description" class="mb-(--mw-component-text-field-label-gap) type-body-sm text-text-muted">
        {strings.notifications.muteDescription}
      </p>
      <ChoiceOption
        name="{id}-mute"
        value={NOT_MUTED}
        bind:group={chosenMute}
        label={strings.notifications.dontMute}
        isInitialFocus={initialGroup === "mute" && chosenMute === NOT_MUTED}
      />
      {#if timedMuteEnd !== null}
        <ChoiceOption
          name="{id}-mute"
          value={CURRENT_MUTE}
          bind:group={chosenMute}
          label={strings.notifications.mutedUntilTime(upcomingTimeLabel(timedMuteEnd, new Date(now)))}
          description={strings.notifications.mutedNow}
          isInitialFocus={initialGroup === "mute"}
        />
      {/if}
      {#each MUTE_DURATIONS as duration (duration)}
        <ChoiceOption
          name="{id}-mute"
          value={duration}
          bind:group={chosenMute}
          label={strings.notifications.muteFor[duration]}
          isInitialFocus={initialGroup === "mute" && duration === "until-turned-back-on" && runningMuteEnd === "never"}
        />
      {/each}
    </fieldset>

    <fieldset class="flex flex-col gap-(--mw-component-choice-list-gap)">
      <legend class="mb-(--mw-component-text-field-label-gap) type-category text-(--mw-component-text-field-label-text)">
        {strings.notifications.levelLegend}
      </legend>
      {#if isServerChannel}
        <ChoiceOption
          name="{id}-level"
          value={SERVER_LEVEL}
          bind:group={chosenLevel}
          label={strings.notifications.sameAsServer}
          description={strings.notifications.sameAsServerNow(strings.notifications.levels[fallbackLevel].label)}
          isInitialFocus={initialGroup === "level" && chosenLevel === SERVER_LEVEL}
        />
      {/if}
      {#each NOTIFY_LEVELS as level (level)}
        <ChoiceOption
          name="{id}-level"
          value={level}
          bind:group={chosenLevel}
          label={strings.notifications.levels[level].label}
          description={strings.notifications.levels[level].description}
          isInitialFocus={initialGroup === "level" && chosenLevel === level}
        />
      {/each}
    </fieldset>
  </div>
</ModalDialog>
