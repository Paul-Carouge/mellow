<script lang="ts">
  import { tick } from "svelte";

  import { strings } from "../i18n";
  import type { IconCrop } from "../lib/api";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import {
    MAXIMUM_PROFILE_IMAGE_BYTES,
    MAXIMUM_SERVER_NAME_LENGTH,
    PREVIEWABLE_IMAGE_TYPES,
    profileImageFileProblem,
  } from "../lib/protocol";
  import Button from "./Button.svelte";
  import ChoiceOption from "./ChoiceOption.svelte";
  import { humanFileSize, serverInitials } from "./format";
  import Icon from "./Icon.svelte";
  import { centredSquare, placementInPercent, stillPictureOf } from "./icon-crop";
  import IconCropDialog from "./IconCropDialog.svelte";
  import ModalDialog from "./ModalDialog.svelte";
  import ServerPreview from "./ServerPreview.svelte";
  import { SERVER_TEMPLATE_IDS, serverTemplates, templateChannelNames } from "./server-templates";
  import TextField from "./TextField.svelte";

  // Discord's "Create a server" and "Join a server" pages in one dialog (DESIGN.md §5.2, §8). A
  // new server gets an icon (the crop dialog of §6.54), a name and a starter template in the
  // interface's language, with a live preview of the navigator beside the form, or under it on a
  // phone. The icon is sent once the server exists; if it is refused, the server stays and the
  // dialog says where to add it.
  interface Props {
    client: MellowClient;
    onclose: () => void;
    fallbackFocus: () => void;
  }

  let { client, onclose, fallbackFocus }: Props = $props();

  const id = $props.id();
  const TOO_LARGE_REASON = strings.profile.tooLarge(humanFileSize(MAXIMUM_PROFILE_IMAGE_BYTES));
  const templates = serverTemplates();

  let isJoining = $state(false);
  let serverName = $state("");
  let inviteCode = $state("");
  let templateId = $state<string>(SERVER_TEMPLATE_IDS[0] ?? "blank");
  let isBusy = $state(false);
  // Plain: read by the dialog when it closes, after this component stopped updating.
  let hasSucceeded = false;
  // An error belongs to the page it was raised on: switching pages hides it.
  let error = $state<{ isJoin: boolean; text: string } | null>(null);
  /** The server exists but its icon was refused: only Close is left. */
  let isCreatedWithoutIcon = $state(false);
  let nameField = $state<HTMLInputElement | null>(null);
  let codeField = $state<HTMLInputElement | null>(null);
  let fileInput = $state<HTMLInputElement | null>(null);
  /** The picture being cropped, before it becomes the icon. */
  let fileToCrop = $state<File | null>(null);
  let icon = $state.raw<{
    file: File;
    crop: IconCrop | null;
    preview: { url: string; placement: { left: number; top: number; width: number } };
  } | null>(null);

  const template = $derived(templates.find((candidate) => candidate.id === templateId) ?? templates[0]);
  const canSubmit = $derived(!isBusy && (isJoining ? inviteCode.trim() : serverName.trim()) !== "");

  // The preview's object URL goes with the icon it shows.
  $effect(() => {
    const shown = icon;
    return () => {
      if (shown) {
        URL.revokeObjectURL(shown.preview.url);
      }
    };
  });

  function chooseFile(input: HTMLInputElement) {
    const file = input.files?.[0];
    // Cleared, so choosing the same file again (after an error) still fires `change`.
    input.value = "";
    if (!file) {
      return;
    }
    const problem = profileImageFileProblem(file);
    if (problem !== null) {
      const reason = problem === "too-large" ? TOO_LARGE_REASON : strings.profile.notAPicture;
      error = { isJoin: false, text: strings.errors.withReason(strings.iconEditor.changeFailed, reason) };
      return;
    }
    error = null;
    fileToCrop = file;
  }

  async function keepCroppedIcon(file: File, crop: IconCrop | null) {
    fileToCrop = null;
    try {
      const still = await stillPictureOf(file);
      icon = {
        file,
        crop,
        preview: { url: still.url, placement: placementInPercent(crop ?? centredSquare(still.picture), still.picture) },
      };
    } catch {
      error = { isJoin: false, text: strings.errors.withReason(strings.iconEditor.changeFailed, strings.profile.unreadable) };
    }
  }

  async function submit() {
    if (isCreatedWithoutIcon) {
      return;
    }
    const isJoin = isJoining;
    const name = serverName.trim();
    const chosenIcon = icon;
    isBusy = true;
    error = null;
    try {
      if (isJoin) {
        await client.joinServer(inviteCode.trim());
      } else {
        const guild = await client.createServer(name, template?.categories);
        if (chosenIcon) {
          try {
            await client.setServerIcon(guild.id, chosenIcon.file, chosenIcon.crop);
          } catch (iconError) {
            hasSucceeded = true;
            isCreatedWithoutIcon = true;
            error = {
              isJoin: false,
              text: `${describeError(iconError, strings.addServer.iconFailed(name))} ${strings.addServer.iconLater}`,
            };
            return;
          }
        }
      }
      hasSucceeded = true;
      onclose();
    } catch (failure) {
      error = { isJoin, text: describeError(failure, isJoin ? strings.addServer.joinFailed : strings.addServer.createFailed) };
    } finally {
      isBusy = false;
    }
  }

  async function switchPage() {
    if (isBusy) {
      return;
    }
    isJoining = !isJoining;
    await tick();
    (isJoining ? codeField : nameField)?.focus();
  }
</script>

<ModalDialog
  shouldReturnFocusToOpener={() => !hasSucceeded}
  title={isJoining ? strings.addServer.joinTitle : strings.addServer.createTitle}
  subtitle={isJoining ? strings.addServer.joinSubtitle : strings.addServer.createSubtitle}
  isHeadingCentered
  isWide={!isJoining}
  errorMessage={error && error.isJoin === isJoining ? error.text : ""}
  {isBusy}
  cancelLabel={isCreatedWithoutIcon ? strings.common.close : strings.common.cancel}
  submitLabel={isCreatedWithoutIcon
    ? ""
    : isJoining
      ? isBusy
        ? strings.addServer.joining
        : strings.addServer.join
      : isBusy
        ? strings.addServer.creating
        : strings.addServer.create}
  isSubmitEnabled={canSubmit}
  {onclose}
  onsubmit={submit}
  {fallbackFocus}
>
  {#if isJoining}
    <TextField
      id="join-server-invite-code"
      label={strings.addServer.inviteCode}
      helper={strings.addServer.inviteCodeHelper}
      autocomplete="off"
      readonly={isBusy}
      data-initial-focus=""
      bind:value={inviteCode}
      bind:element={codeField}
    />
  {:else}
    <!-- Side by side while both fit; the preview wraps under the form on a phone. -->
    <div class="flex flex-wrap gap-(--mw-component-dialog-section-gap)">
      <fieldset
        class="flex min-w-0 flex-[999_1_var(--mw-component-server-preview-width)] flex-col gap-(--mw-component-dialog-section-gap)"
        disabled={isBusy || isCreatedWithoutIcon}
      >
        <div class="flex items-end gap-(--mw-component-dialog-section-gap)">
          <!-- The icon as the strip will show it; a pointer shortcut to "Upload icon" below, which
               is the control for keyboards and screen readers. -->
          <button
            type="button"
            tabindex="-1"
            aria-hidden="true"
            class="relative grid size-(--mw-component-icon-picker-preview-size) shrink-0 place-items-center overflow-hidden rounded-full bg-(--mw-component-icon-picker-preview-background) type-heading-lg text-(--mw-component-icon-picker-preview-text) transition-[border-radius] duration-(--mw-motion-duration-settle) ease-(--mw-motion-easing-settle) hover:rounded-(--mw-component-rail-icon-radius-active)"
            onclick={() => fileInput?.click()}
          >
            {#if icon}
              <img
                src={icon.preview.url}
                alt=""
                class="absolute max-w-none"
                style:left="{icon.preview.placement.left}%"
                style:top="{icon.preview.placement.top}%"
                style:width="{icon.preview.placement.width}%"
              />
            {:else if serverName.trim()}
              {serverInitials(serverName)}
            {:else}
              <Icon name="plus" class="size-(--mw-component-choice-icon-size)" />
            {/if}
          </button>
          <div class="min-w-0 flex-1">
            <TextField
              id="create-server-name"
              label={strings.addServer.serverName}
              maxlength={MAXIMUM_SERVER_NAME_LENGTH}
              autocomplete="off"
              readonly={isBusy}
              data-initial-focus=""
              bind:value={serverName}
              bind:element={nameField}
            />
          </div>
        </div>
        <div class="-mt-xs flex flex-wrap items-center gap-x-(--mw-component-dialog-button-gap) gap-y-3xs">
          <input
            bind:this={fileInput}
            type="file"
            accept={PREVIEWABLE_IMAGE_TYPES.join(",")}
            tabindex="-1"
            aria-hidden="true"
            class="sr-only"
            onchange={(event) => chooseFile(event.currentTarget)}
          />
          <Button
            id="{id}-icon-upload"
            variant="ghost"
            aria-describedby={icon ? undefined : `${id}-icon-note`}
            onclick={() => fileInput?.click()}>{strings.iconEditor.upload}</Button
          >
          {#if icon}
            <Button
              variant="ghost"
              onclick={() => {
                icon = null;
                // The button goes with the icon: Upload takes the focus.
                document.getElementById(`${id}-icon-upload`)?.focus();
              }}>{strings.iconEditor.remove}</Button>
          {:else}
            <p id="{id}-icon-note" class="w-full type-body-sm text-(--mw-component-dialog-hint-text)">
              {strings.addServer.iconOptional}
            </p>
          {/if}
        </div>

        <fieldset class="flex flex-col gap-(--mw-component-choice-list-gap)">
          <legend class="mb-(--mw-component-text-field-label-gap) type-category text-(--mw-component-text-field-label-text)">
            {strings.addServer.startWith}
          </legend>
          {#each templates as candidate (candidate.id)}
            <ChoiceOption
              name="{id}-template"
              value={candidate.id}
              bind:group={templateId}
              label={candidate.name}
              description={templateChannelNames(candidate)}
            />
          {/each}
        </fieldset>
      </fieldset>

      <div class="flex min-w-0 flex-[1_1_var(--mw-component-server-preview-width)] flex-col gap-(--mw-component-text-field-label-gap)">
        <p class="type-category text-(--mw-component-text-field-label-text)">{strings.addServer.preview}</p>
        <ServerPreview name={serverName} icon={icon?.preview ?? null} categories={template?.categories ?? []} />
      </div>
    </div>
  {/if}
  {#if !isCreatedWithoutIcon}
    <p class="flex flex-wrap items-center justify-center gap-x-(--mw-space-2xs) type-body text-(--mw-component-dialog-hint-text)">
      {isJoining ? strings.addServer.hasNoInvite : strings.addServer.hasInvite}
      <button
        type="button"
        class="min-h-(--mw-size-hit-web-min) font-medium text-(--mw-component-dialog-mode-link-text) hover:underline aria-disabled:opacity-(--mw-component-button-disabled-opacity)"
        aria-disabled={isBusy || undefined}
        onclick={switchPage}>{isJoining ? strings.addServer.createTitle : strings.addServer.joinTitle}</button
      >
    </p>
  {/if}
</ModalDialog>

{#if fileToCrop}
  {@const file = fileToCrop}
  <IconCropDialog
    {file}
    kind="server"
    name={serverName.trim() || strings.addServer.previewPlaceholder}
    isBusy={false}
    errorMessage=""
    onclose={() => (fileToCrop = null)}
    onsave={(crop) => void keepCroppedIcon(file, crop)}
  />
{/if}
