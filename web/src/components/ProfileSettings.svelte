<script lang="ts">
  import { tick } from "svelte";

  import { strings } from "../i18n";
  import { ApiError, type PictureCrop, type ProfileImageKind } from "../lib/api";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import { MAXIMUM_PROFILE_IMAGE_BYTES, PICTURE_UPLOAD_TYPES, profileImageFileProblem } from "../lib/protocol";
  import Button from "./Button.svelte";
  import { humanFileSize } from "./format";
  import Icon from "./Icon.svelte";
  import PictureCropDialog from "./PictureCropDialog.svelte";
  import ProfileCard from "./ProfileCard.svelte";

  // User settings, Profile (DESIGN.md §5.9, ADR-0021): Discord's Profiles page, the avatar and
  // banner editors beside a live preview of the profile card (under them on narrow pages). A
  // chosen picture opens the crop dialog (§6.54): a circle for the avatar, the card's band for the
  // banner. The instance crops, resizes and compresses before answering, a second or two for a
  // large GIF, so the buttons say so meanwhile; one change runs at a time.
  interface Props {
    client: MellowClient;
  }

  let { client }: Props = $props();

  const MAXIMUM_SIZE_LABEL = humanFileSize(MAXIMUM_PROFILE_IMAGE_BYTES);
  const TOO_LARGE_REASON = strings.profile.tooLarge(MAXIMUM_SIZE_LABEL);
  const WORDS = { avatar: strings.profile.avatar, banner: strings.profile.banner } satisfies Record<ProfileImageKind, unknown>;

  const id = $props.id();
  const user = $derived(client.user);

  let busy = $state<{ kind: ProfileImageKind; action: "upload" | "remove" } | null>(null);
  let errorMessage = $state<{ kind: ProfileImageKind; text: string } | null>(null);
  let notice = $state<{ kind: ProfileImageKind; text: string } | null>(null);
  let chosen = $state<{ kind: ProfileImageKind; file: File } | null>(null);
  let dialogError = $state("");
  const fileInputs: Partial<Record<ProfileImageKind, HTMLInputElement>> = $state({});

  /** Says what to do about each refusal, in the words of this page rather than the generic ones. */
  function describeFailure(error: unknown, summary: string): string {
    if (error instanceof ApiError && error.code === "VALIDATION_FAILED") {
      return strings.errors.withReason(summary, strings.profile.unreadable);
    }
    if (error instanceof ApiError && error.code === "FILE_TOO_LARGE") {
      return strings.errors.withReason(summary, TOO_LARGE_REASON);
    }
    return describeError(error, summary);
  }

  async function run(kind: ProfileImageKind, action: "upload" | "remove", change: () => Promise<void>, doneText: string) {
    if (busy) {
      return;
    }
    busy = { kind, action };
    errorMessage = null;
    notice = null;
    try {
      await change();
      notice = { kind, text: doneText };
    } catch (error) {
      const summary = action === "upload" ? WORDS[kind].changeFailed : WORDS[kind].removeFailed;
      errorMessage = { kind, text: describeFailure(error, summary) };
    } finally {
      busy = null;
    }
  }

  function upload(kind: ProfileImageKind, input: HTMLInputElement) {
    const file = input.files?.[0];
    // Cleared, so choosing the same file again (after an error) still fires `change`.
    input.value = "";
    if (!file) {
      return;
    }
    const problem = profileImageFileProblem(file);
    const summary = WORDS[kind].changeFailed;
    if (problem === "not-a-picture") {
      notice = null;
      errorMessage = { kind, text: strings.errors.withReason(summary, strings.profile.notAPicture) };
      return;
    }
    if (problem === "too-large") {
      notice = null;
      errorMessage = { kind, text: strings.errors.withReason(summary, TOO_LARGE_REASON) };
      return;
    }
    errorMessage = null;
    dialogError = "";
    chosen = { kind, file };
  }

  async function uploadCropped(kind: ProfileImageKind, file: File, crop: PictureCrop | null) {
    if (busy) {
      return;
    }
    busy = { kind, action: "upload" };
    dialogError = "";
    notice = null;
    try {
      await client.setProfileImage(kind, file, crop);
      chosen = null;
      notice = { kind, text: WORDS[kind].saved };
    } catch (error) {
      // The dialog stays open with its crop, so trying again needs no new pick.
      dialogError = describeFailure(error, WORDS[kind].changeFailed);
    } finally {
      busy = null;
    }
  }

  async function remove(kind: ProfileImageKind) {
    await run(kind, "remove", () => client.removeProfileImage(kind), WORDS[kind].removed);
    // The Remove button is gone once it worked: focus stays in the editor rather than on the page.
    await tick();
    if (!user?.[kind]) {
      document.getElementById(changeButtonId(kind))?.focus();
    }
  }

  function changeButtonId(kind: ProfileImageKind): string {
    return `${id}-${kind}-change`;
  }
</script>

{#snippet editor(kind: ProfileImageKind)}
  {@const words = WORDS[kind]}
  {@const hasPicture = Boolean(user?.[kind])}
  {@const isThisBusy = busy?.kind === kind}
  <section aria-labelledby="{id}-{kind}-title" aria-busy={isThisBusy} class="flex flex-col gap-(--mw-component-settings-section-label-gap)">
    <h3 id="{id}-{kind}-title" class="type-category text-(--mw-component-settings-section-label-text)">{words.title}</h3>
    {#if !hasPicture}
      <p class="type-body text-(--mw-component-settings-body-text)">{words.emptyText}</p>
    {/if}
    <input
      bind:this={fileInputs[kind]}
      type="file"
      accept={PICTURE_UPLOAD_TYPES.join(",")}
      tabindex="-1"
      aria-hidden="true"
      class="sr-only"
      onchange={(event) => upload(kind, event.currentTarget)}
    />
    <div class="mt-(--mw-component-settings-section-label-gap) flex flex-wrap gap-(--mw-component-settings-account-button-gap)">
      <Button
        id={changeButtonId(kind)}
        variant="primary"
        aria-describedby="{id}-formats"
        isDisabled={busy !== null}
        onclick={() => fileInputs[kind]?.click()}
      >
        {isThisBusy && busy?.action === "upload" ? words.uploading : words.change}
      </Button>
      {#if hasPicture}
        <Button isDisabled={busy !== null} onclick={() => void remove(kind)}>
          {isThisBusy && busy?.action === "remove" ? words.removing : words.remove}
        </Button>
      {/if}
    </div>
    <div role="alert" class="contents">
      {#if errorMessage?.kind === kind}
        <p class="flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-settings-error-text)">
          <Icon
            name="warning-circle"
            class="size-(--mw-component-settings-error-icon-size) shrink-0 text-(--mw-component-settings-error-icon)"
          />
          <span>{errorMessage.text}</span>
        </p>
      {/if}
    </div>
    {#if notice?.kind === kind}
      <!-- Announced through the status region below, so it is not read twice. -->
      <p aria-hidden="true" class="flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-settings-notice-text)">
        <Icon name="check" class="size-(--mw-component-settings-error-icon-size) shrink-0" />
        <span>{notice.text}</span>
      </p>
    {/if}
  </section>
{/snippet}

<h2 class="type-heading-lg text-(--mw-component-settings-page-title-text)">{strings.settings.profile}</h2>
<p id="{id}-formats" class="mt-(--mw-component-settings-section-label-gap) type-body text-(--mw-component-settings-body-text)">
  {strings.profile.formats(MAXIMUM_SIZE_LABEL)}
</p>

<!-- Both columns start at the card's width, so the preview wraps under the editors when the page
     cannot hold the two. -->
<div class="mt-(--mw-component-settings-page-title-gap) flex flex-wrap gap-(--mw-component-settings-section-gap)">
  <div class="flex min-w-0 flex-[1_1_var(--mw-component-profile-popover-width)] flex-col gap-(--mw-component-settings-section-gap)">
    {@render editor("avatar")}
    {@render editor("banner")}
  </div>
  {#if user}
    <section aria-labelledby="{id}-preview-title" class="flex flex-[0_1_var(--mw-component-profile-popover-width)] flex-col gap-(--mw-component-settings-section-label-gap)">
      <h3 id="{id}-preview-title" class="type-category text-(--mw-component-settings-section-label-text)">{strings.profile.preview}</h3>
      <div
        aria-busy={busy !== null}
        class="w-full max-w-(--mw-component-profile-popover-width) overflow-hidden rounded-(--mw-component-profile-popover-radius) border-(length:--mw-component-profile-popover-border-width) border-(--mw-component-profile-popover-border) bg-(--mw-component-profile-popover-background)"
      >
        <ProfileCard {user} nameElement="p" />
      </div>
    </section>
  {/if}
</div>

{#if chosen}
  {@const { kind, file } = chosen}
  <PictureCropDialog
    {file}
    {kind}
    user={user ?? undefined}
    isBusy={busy?.action === "upload"}
    errorMessage={dialogError}
    onclose={() => {
      chosen = null;
      dialogError = "";
    }}
    onsave={(crop) => void uploadCropped(kind, file, crop)}
  />
{/if}

<!-- Exists from the start, so its first change is announced. -->
<p role="status" class="sr-only">{notice?.text ?? (busy?.action === "upload" ? WORDS[busy.kind].uploading : "")}</p>
