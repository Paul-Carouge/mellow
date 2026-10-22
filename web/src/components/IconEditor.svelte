<script lang="ts">
  import { tick } from "svelte";

  import { strings } from "../i18n";
  import { ApiError, type IconCrop } from "../lib/api";
  import { describeError } from "../lib/client.svelte";
  import { prefersReducedMotion } from "../lib/motion.svelte";
  import {
    MAXIMUM_PROFILE_IMAGE_BYTES,
    PREVIEWABLE_IMAGE_TYPES,
    type ProfileImage,
    iconUrl,
    profileImageFileProblem,
  } from "../lib/protocol";
  import Button from "./Button.svelte";
  import { humanFileSize, serverInitials } from "./format";
  import Icon from "./Icon.svelte";
  import IconCropDialog from "./IconCropDialog.svelte";

  // A server's or channel's icon on its Overview page (DESIGN.md §6.54): the current one, "Upload
  // icon" opening the crop dialog once a picture is chosen, and "Remove icon". Saved at once, apart
  // from the page's unsaved-changes bar, as avatars are. The instance crops and compresses; the
  // gateway brings the new icon to everyone, this page included.
  interface Props {
    kind: "server" | "channel";
    icon: ProfileImage | null | undefined;
    /** The server's or channel's name: initials and the dialog's samples. */
    name: string;
    onupload: (file: File, crop: IconCrop | null) => Promise<void>;
    onremove: () => Promise<void>;
  }

  let { kind, icon, name, onupload, onremove }: Props = $props();

  const MAXIMUM_SIZE_LABEL = humanFileSize(MAXIMUM_PROFILE_IMAGE_BYTES);
  const TOO_LARGE_REASON = strings.profile.tooLarge(MAXIMUM_SIZE_LABEL);

  const id = $props.id();
  const words = $derived(strings.iconEditor[kind]);
  let fileInput = $state<HTMLInputElement | null>(null);
  let chosenFile = $state<File | null>(null);
  let busy = $state<"upload" | "remove" | null>(null);
  let errorMessage = $state("");
  let dialogError = $state("");
  let notice = $state("");
  let failedIconId = $state<string | null>(null);

  const shownIcon = $derived(icon && icon.id !== failedIconId ? icon : null);

  /** Says what to do about each refusal, in this page's words rather than the generic ones. */
  function describeFailure(error: unknown, summary: string): string {
    if (error instanceof ApiError && error.code === "VALIDATION_FAILED") {
      return strings.errors.withReason(summary, strings.profile.unreadable);
    }
    if (error instanceof ApiError && error.code === "FILE_TOO_LARGE") {
      return strings.errors.withReason(summary, TOO_LARGE_REASON);
    }
    return describeError(error, summary);
  }

  function choose(input: HTMLInputElement) {
    const file = input.files?.[0];
    // Cleared, so choosing the same file again (after an error) still fires `change`.
    input.value = "";
    if (!file) {
      return;
    }
    notice = "";
    const problem = profileImageFileProblem(file);
    if (problem !== null) {
      const reason = problem === "too-large" ? TOO_LARGE_REASON : strings.profile.notAPicture;
      errorMessage = strings.errors.withReason(strings.iconEditor.changeFailed, reason);
      return;
    }
    errorMessage = "";
    dialogError = "";
    chosenFile = file;
  }

  async function upload(file: File, crop: IconCrop | null) {
    if (busy) {
      return;
    }
    busy = "upload";
    dialogError = "";
    try {
      await onupload(file, crop);
      chosenFile = null;
      notice = strings.iconEditor.saved;
    } catch (error) {
      // The dialog stays open with its square, so trying again needs no new pick.
      dialogError = describeFailure(error, strings.iconEditor.changeFailed);
    } finally {
      busy = null;
    }
  }

  async function remove() {
    if (busy) {
      return;
    }
    busy = "remove";
    errorMessage = "";
    notice = "";
    try {
      await onremove();
      notice = strings.iconEditor.removed;
      // "Remove icon" goes with the icon: focus stays in the editor.
      await tick();
      document.getElementById(`${id}-upload`)?.focus();
    } catch (error) {
      errorMessage = describeError(error, strings.iconEditor.removeFailed);
    } finally {
      busy = null;
    }
  }
</script>

<section aria-labelledby="{id}-title" aria-busy={busy !== null} class="flex flex-wrap items-start gap-(--mw-component-settings-action-gap)">
  <span
    aria-hidden="true"
    class={[
      "grid size-(--mw-component-icon-picker-preview-size) shrink-0 place-items-center overflow-hidden bg-(--mw-component-icon-picker-preview-background) text-(--mw-component-icon-picker-preview-text)",
      kind === "server" ? "rounded-full" : "rounded-(--mw-component-icon-picker-preview-radius-channel)",
    ]}
  >
    {#if shownIcon}
      {@const iconId = shownIcon.id}
      <!-- The preview plays an animated icon, as the profile card does, except under reduced motion. -->
      <img
        src={iconUrl(shownIcon, !prefersReducedMotion.current)}
        alt=""
        decoding="async"
        class="size-full object-cover"
        onerror={() => (failedIconId = iconId)}
      />
    {:else if kind === "server"}
      <span class="type-heading-lg">{serverInitials(name)}</span>
    {:else}
      <Icon name="hash" class="size-1/2" />
    {/if}
  </span>

  <div class="flex max-w-(--mw-component-dialog-width) min-w-0 flex-[1_1_var(--mw-component-roles-list-width)] flex-col gap-(--mw-component-settings-section-label-gap)">
    <h3 id="{id}-title" class="type-category text-(--mw-component-settings-section-label-text)">{words.title}</h3>
    <p id="{id}-note" class="type-body text-(--mw-component-settings-body-text)">
      {shownIcon ? words.note : words.emptyText}
      {strings.iconEditor.formats(MAXIMUM_SIZE_LABEL)}
    </p>
    <input
      bind:this={fileInput}
      type="file"
      accept={PREVIEWABLE_IMAGE_TYPES.join(",")}
      tabindex="-1"
      aria-hidden="true"
      class="sr-only"
      onchange={(event) => choose(event.currentTarget)}
    />
    <div class="mt-(--mw-component-settings-section-label-gap) flex flex-wrap gap-(--mw-component-settings-account-button-gap)">
      <Button id="{id}-upload" variant="primary" aria-describedby="{id}-note" isDisabled={busy !== null} onclick={() => fileInput?.click()}>
        {busy === "upload" ? strings.iconEditor.uploading : strings.iconEditor.upload}
      </Button>
      {#if icon}
        <Button isDisabled={busy !== null} onclick={() => void remove()}>
          {busy === "remove" ? strings.iconEditor.removing : strings.iconEditor.remove}
        </Button>
      {/if}
    </div>
    <div role="alert" class="contents">
      {#if errorMessage}
        <p class="flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-settings-error-text)">
          <Icon name="warning-circle" class="size-(--mw-component-settings-error-icon-size) shrink-0 text-(--mw-component-settings-error-icon)" />
          <span>{errorMessage}</span>
        </p>
      {/if}
    </div>
    {#if notice}
      <!-- Announced through the status region below, so it is not read twice. -->
      <p aria-hidden="true" class="flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-settings-notice-text)">
        <Icon name="check" class="size-(--mw-component-settings-error-icon-size) shrink-0" />
        <span>{notice}</span>
      </p>
    {/if}
    <p role="status" class="sr-only">{notice}</p>
  </div>
</section>

{#if chosenFile}
  <IconCropDialog
    file={chosenFile}
    {kind}
    {name}
    isBusy={busy === "upload"}
    errorMessage={dialogError}
    onclose={() => {
      chosenFile = null;
      dialogError = "";
    }}
    onsave={(crop) => {
      if (chosenFile) {
        void upload(chosenFile, crop);
      }
    }}
  />
{/if}
