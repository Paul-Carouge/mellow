<script lang="ts">
  import { tick } from "svelte";

  import { ApiError, type ProfileImageKind } from "../lib/api";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import { MAXIMUM_PROFILE_IMAGE_BYTES, PREVIEWABLE_IMAGE_TYPES, profileImageFileProblem } from "../lib/protocol";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";
  import ProfileCard from "./ProfileCard.svelte";

  // User settings, Profile (DESIGN.md §5.9, ADR-0021): Discord's Profiles page, the avatar and
  // banner editors beside a live preview of the profile card (under them on narrow pages). The
  // instance resizes and compresses a picture before answering, a second or two for a large GIF,
  // so the button says so meanwhile; one change runs at a time.
  interface Props {
    client: MellowClient;
  }

  let { client }: Props = $props();

  const MAXIMUM_SIZE_LABEL = `${MAXIMUM_PROFILE_IMAGE_BYTES / (1024 * 1024)} MB`;
  const TOO_LARGE_REASON = `the picture is larger than ${MAXIMUM_SIZE_LABEL}. Choose a smaller one.`;
  const WORDS: Record<ProfileImageKind, { noun: string; title: string; emptyText: string; savedText: string; removedText: string }> = {
    avatar: {
      noun: "avatar",
      title: "Avatar",
      emptyText: "Your initial shows until you add a picture.",
      savedText: "Avatar saved.",
      removedText: "Avatar removed.",
    },
    banner: {
      noun: "banner",
      title: "Profile banner",
      emptyText: "A plain strip shows until you add a picture.",
      savedText: "Banner saved.",
      removedText: "Banner removed.",
    },
  };

  const id = $props.id();
  const user = $derived(client.user);

  let busy = $state<{ kind: ProfileImageKind; action: "upload" | "remove" } | null>(null);
  let errorMessage = $state<{ kind: ProfileImageKind; text: string } | null>(null);
  let notice = $state<{ kind: ProfileImageKind; text: string } | null>(null);
  const fileInputs: Partial<Record<ProfileImageKind, HTMLInputElement>> = $state({});

  /** Says what to do about each refusal, in the words of this page rather than the generic ones. */
  function describeFailure(error: unknown, summary: string): string {
    if (error instanceof ApiError && error.code === "VALIDATION_FAILED") {
      return `${summary}: the picture couldn't be read, or it's over 8192 pixels a side or 600 frames. Choose another one.`;
    }
    if (error instanceof ApiError && error.code === "FILE_TOO_LARGE") {
      return `${summary}: ${TOO_LARGE_REASON}`;
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
      const verb = action === "upload" ? "change" : "remove";
      errorMessage = { kind, text: describeFailure(error, `Couldn't ${verb} the ${WORDS[kind].noun}`) };
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
    const summary = `Couldn't change the ${WORDS[kind].noun}`;
    if (problem === "not-a-picture") {
      notice = null;
      errorMessage = { kind, text: `${summary}: choose a PNG, JPEG, GIF or WebP picture.` };
      return;
    }
    if (problem === "too-large") {
      notice = null;
      errorMessage = { kind, text: `${summary}: ${TOO_LARGE_REASON}` };
      return;
    }
    void run(kind, "upload", () => client.setProfileImage(kind, file), WORDS[kind].savedText);
  }

  async function remove(kind: ProfileImageKind) {
    await run(kind, "remove", () => client.removeProfileImage(kind), WORDS[kind].removedText);
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
      accept={PREVIEWABLE_IMAGE_TYPES.join(",")}
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
        {isThisBusy && busy?.action === "upload" ? `Uploading ${words.noun}…` : `Change ${words.noun}`}
      </Button>
      {#if hasPicture}
        <Button isDisabled={busy !== null} onclick={() => void remove(kind)}>
          {isThisBusy && busy?.action === "remove" ? `Removing ${words.noun}…` : `Remove ${words.noun}`}
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

<h2 class="type-heading-lg text-(--mw-component-settings-page-title-text)">Profile</h2>
<p id="{id}-formats" class="mt-(--mw-component-settings-section-label-gap) type-body text-(--mw-component-settings-body-text)">
  Pictures can be PNG, JPEG, GIF or WebP, up to {MAXIMUM_SIZE_LABEL}. Animated GIFs stay animated.
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
      <h3 id="{id}-preview-title" class="type-category text-(--mw-component-settings-section-label-text)">Preview</h3>
      <div
        aria-busy={busy !== null}
        class="w-full max-w-(--mw-component-profile-popover-width) overflow-hidden rounded-(--mw-component-profile-popover-radius) border-(length:--mw-component-profile-popover-border-width) border-(--mw-component-profile-popover-border) bg-(--mw-component-profile-popover-background)"
      >
        <ProfileCard {user} nameElement="p" />
      </div>
    </section>
  {/if}
</div>

<!-- Exists from the start, so its first change is announced. -->
<p role="status" class="sr-only">{notice?.text ?? (busy?.action === "upload" ? `Uploading ${WORDS[busy.kind].noun}…` : "")}</p>
