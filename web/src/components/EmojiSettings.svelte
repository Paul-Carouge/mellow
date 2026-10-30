<script lang="ts">
  import { onDestroy, tick } from "svelte";
  import { SvelteMap } from "svelte/reactivity";

  import { strings } from "../i18n";
  import { ApiError } from "../lib/api";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import {
    type CustomEmoji,
    type Guild,
    MAXIMUM_EMOJIS_PER_GUILD_AND_KIND,
    MAXIMUM_EMOJI_NAME_LENGTH,
    PREVIEWABLE_IMAGE_TYPES,
    type Snowflake,
  } from "../lib/protocol";
  import Button from "./Button.svelte";
  import CustomEmojiImage from "./CustomEmojiImage.svelte";
  import {
    emojiCounts,
    emojiFileProblem,
    emojiNameFromFileName,
    emojiRenameProblem,
    emojiUploadFailureReason,
  } from "./custom-emoji";
  import { humanFileSize } from "./format";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import ModalDialog from "./ModalDialog.svelte";
  import { focusAfterRemoval } from "./server-settings";

  // Server settings → Emoji (DESIGN.md §5.8, §6.58, ADR-0028). Pictures upload as soon as they are
  // chosen, one after another, named after their file; each is renamed in place (Enter or leaving
  // the field saves, Esc puts the name back). Still and animated emoji are listed apart, each kind
  // with its own count, since each has its own room of 50. The list follows the gateway.
  interface Props {
    client: MellowClient;
    guild: Guild;
  }

  let { client, guild }: Props = $props();

  interface Upload {
    key: number;
    file: File;
    name: string;
    /** Null for a file that is not a picture: nothing to preview. */
    previewUrl: string | null;
    status: "waiting" | "adding" | "failed";
    error: string;
    /** False when the file itself is wrong: trying again would fail the same way. */
    canRetry: boolean;
  }

  const id = $props.id();
  const emojis = $derived(guild.emojis ?? []);
  const counts = $derived(emojiCounts(emojis));
  const isStillFull = $derived(counts.still >= MAXIMUM_EMOJIS_PER_GUILD_AND_KIND);
  const isAnimatedFull = $derived(counts.animated >= MAXIMUM_EMOJIS_PER_GUILD_AND_KIND);
  const kinds = $derived([
    { key: "still", title: strings.serverSettings.emoji.still, used: counts.still, isFull: isStillFull, empty: strings.serverSettings.emoji.noStill, list: emojis.filter((emoji) => !emoji.is_animated) },
    { key: "animated", title: strings.serverSettings.emoji.animated, used: counts.animated, isFull: isAnimatedFull, empty: strings.serverSettings.emoji.noAnimated, list: emojis.filter((emoji) => emoji.is_animated) },
  ]);

  let fileInput = $state<HTMLInputElement | null>(null);
  let uploads = $state<Upload[]>([]);
  let nextUploadKey = 0;
  let isUploading = false;
  let notice = $state("");
  const renameErrors = new SvelteMap<Snowflake, string>();
  /** The name asked for each emoji, until the gateway's list shows it or the request fails. */
  const requestedNames = new Map<Snowflake, string>();
  let deleting = $state<CustomEmoji | null>(null);
  let isDeleting = $state(false);
  let deleteError = $state("");
  /** The next row's name field once an emoji is deleted; null for the upload button. */
  let focusAfterDeleteId: string | null = null;

  function revokePreview(upload: Upload) {
    if (upload.previewUrl) {
      URL.revokeObjectURL(upload.previewUrl);
    }
  }

  onDestroy(() => uploads.forEach(revokePreview));

  function nameFieldId(emojiId: Snowflake): string {
    return `${id}-name-${emojiId}`;
  }

  function failureText(file: File, reason: string): string {
    return strings.errors.withReason(strings.serverSettings.emoji.addFailed(file.name), reason);
  }

  /** Each chosen file waits its turn, or fails at once when its problem is known before sending. */
  function addFiles(files: readonly File[]) {
    const takenNames = [...emojis.map((emoji) => emoji.name), ...uploads.map((upload) => upload.name)];
    for (const file of files) {
      const name = emojiNameFromFileName(file.name, takenNames);
      takenNames.push(name);
      const problem = emojiFileProblem(file, counts);
      const reasons = strings.serverSettings.emoji;
      const error =
        problem === "not-a-picture"
          ? reasons.notAPicture
          : problem === "too-large"
            ? reasons.tooLarge(humanFileSize(file.size))
            : problem === "no-room"
              ? reasons.noRoom
              : "";
      uploads.push({
        key: nextUploadKey++,
        file,
        name,
        previewUrl: problem === "not-a-picture" ? null : URL.createObjectURL(file),
        status: problem ? "failed" : "waiting",
        error: problem ? failureText(file, error) : "",
        canRetry: problem === null || problem === "no-room",
      });
    }
    void uploadWaiting();
  }

  /**
   * One at a time, so a refusal for too many pictures (the budget is shared with avatars and
   * icons) stops the rest: they wait with "Try again" rather than all failing the same way.
   */
  async function uploadWaiting() {
    if (isUploading) {
      return;
    }
    isUploading = true;
    try {
      for (let upload = uploads.find((candidate) => candidate.status === "waiting"); upload; upload = uploads.find((candidate) => candidate.status === "waiting")) {
        upload.status = "adding";
        try {
          const created = await client.createEmoji(guild.id, upload.name, upload.file);
          removeUpload(upload.key);
          notice = strings.serverSettings.emoji.added(created.name);
        } catch (error) {
          upload.status = "failed";
          upload.error = failureText(upload.file, emojiUploadFailureReason(error, upload.file));
          notice = upload.error;
          if (error instanceof ApiError && error.code === "RATE_LIMITED") {
            for (const waiting of uploads.filter((candidate) => candidate.status === "waiting")) {
              waiting.status = "failed";
              waiting.error = failureText(waiting.file, strings.serverSettings.emoji.tooMany);
            }
          }
        }
      }
    } finally {
      isUploading = false;
    }
  }

  function removeUpload(key: number) {
    const upload = uploads.find((candidate) => candidate.key === key);
    if (upload) {
      revokePreview(upload);
    }
    uploads = uploads.filter((candidate) => candidate.key !== key);
  }

  async function dismissUpload(key: number) {
    const rowIds = uploads.map((upload) => `${id}-upload-${upload.key}`);
    const next = focusAfterRemoval(rowIds, `${id}-upload-${key}`);
    removeUpload(key);
    await tick();
    (next ? document.getElementById(next) : document.getElementById(`${id}-upload`))?.focus();
  }

  function retryUpload(upload: Upload) {
    // Named again: an emoji added meanwhile may have taken the name.
    upload.name = emojiNameFromFileName(upload.file.name, [
      ...emojis.map((emoji) => emoji.name),
      ...uploads.filter((candidate) => candidate !== upload).map((candidate) => candidate.name),
    ]);
    upload.status = "waiting";
    upload.error = "";
    void uploadWaiting();
  }

  async function rename(emoji: CustomEmoji, field: HTMLInputElement) {
    const name = field.value.trim();
    if (requestedNames.get(emoji.id) === emoji.name) {
      // The gateway's list brought it: nothing is waiting any more.
      requestedNames.delete(emoji.id);
    }
    if (name === emoji.name) {
      renameErrors.delete(emoji.id);
      return;
    }
    // Enter, then leaving the field, would ask twice while the gateway's list is on its way.
    if (requestedNames.get(emoji.id) === name) {
      return;
    }
    const problem = emojiRenameProblem(name, emoji.id, emojis);
    if (problem) {
      const reason = problem === "taken" ? strings.serverSettings.emoji.nameTaken : strings.serverSettings.emoji.nameInvalid;
      renameErrors.set(emoji.id, reason);
      return;
    }
    renameErrors.delete(emoji.id);
    requestedNames.set(emoji.id, name);
    try {
      // The field keeps the typed name; the gateway's list brings the same one back.
      await client.renameEmoji(guild.id, emoji.id, name);
    } catch (error) {
      requestedNames.delete(emoji.id);
      renameErrors.set(emoji.id, describeError(error, strings.serverSettings.emoji.renameFailed));
    }
  }

  function handleNameKeydown(event: KeyboardEvent & { currentTarget: HTMLInputElement }, emoji: CustomEmoji) {
    if (event.key === "Enter") {
      event.preventDefault();
      void rename(emoji, event.currentTarget);
    } else if (event.key === "Escape" && (event.currentTarget.value !== emoji.name || renameErrors.has(emoji.id))) {
      // Esc first puts the name back; a second Esc closes the settings.
      event.preventDefault();
      event.stopPropagation();
      event.currentTarget.value = emoji.name;
      renameErrors.delete(emoji.id);
    }
  }

  async function confirmDelete() {
    const emoji = deleting;
    if (!emoji || isDeleting) {
      return;
    }
    isDeleting = true;
    deleteError = "";
    // In the order shown: still ones, then animated ones.
    const shownIds = kinds.flatMap((kind) => kind.list.map((candidate) => nameFieldId(candidate.id)));
    try {
      await client.deleteEmoji(guild.id, emoji.id);
      // The dialog's fallback focus takes it there, its opener (the trash button) being gone.
      focusAfterDeleteId = focusAfterRemoval(shownIds, nameFieldId(emoji.id));
      deleting = null;
      renameErrors.delete(emoji.id);
      notice = strings.serverSettings.emoji.deleted(emoji.name);
    } catch (error) {
      deleteError = describeError(error, strings.serverSettings.emoji.deleteFailed);
    } finally {
      isDeleting = false;
    }
  }
</script>

{#snippet picture(emoji: CustomEmoji)}
  <span
    class="grid size-(--mw-component-custom-emoji-settings-picture-size) shrink-0 place-items-center rounded-(--mw-component-custom-emoji-settings-picture-radius) bg-(--mw-component-custom-emoji-settings-picture-background) p-(--mw-component-custom-emoji-settings-picture-padding)"
  >
    <CustomEmojiImage {emoji} class="size-full" isDecorative />
  </span>
{/snippet}

<div class="flex flex-col gap-(--mw-component-server-settings-toolbar-gap)">
  <div class="flex flex-col gap-(--mw-component-server-settings-text-gap)">
    <p class="type-body-lg text-(--mw-component-settings-body-text)">{strings.serverSettings.emoji.intro}</p>
    <p id="{id}-rules" class="type-body-sm text-(--mw-component-server-settings-secondary-text)">
      {strings.serverSettings.emoji.rules}
    </p>
  </div>

  <div class="flex flex-col items-start gap-(--mw-component-server-settings-text-gap)">
    <Button
      id="{id}-upload"
      variant="primary"
      isDisabled={isStillFull && isAnimatedFull}
      aria-describedby={isStillFull && isAnimatedFull ? `${id}-full` : `${id}-rules`}
      onclick={() => fileInput?.click()}
    >
      {strings.serverSettings.emoji.upload}
    </Button>
    {#if isStillFull && isAnimatedFull}
      <p id="{id}-full" class="type-body-sm text-(--mw-component-server-settings-secondary-text)">
        {strings.serverSettings.emoji.bothFull}
      </p>
    {/if}
    <!-- Opened by the button only; the button is the control people reach. -->
    <input
      bind:this={fileInput}
      type="file"
      multiple
      accept={PREVIEWABLE_IMAGE_TYPES.join(",")}
      hidden
      tabindex="-1"
      aria-hidden="true"
      onchange={(event) => {
        addFiles(Array.from(event.currentTarget.files ?? []));
        // Choosing the same file again must fire change again.
        event.currentTarget.value = "";
      }}
    />
  </div>

  {#if uploads.length > 0}
    <ul aria-label={strings.serverSettings.emoji.upload}>
      {#each uploads as upload (upload.key)}
        <li
          class="flex min-h-(--mw-component-server-settings-row-min-height) items-center gap-(--mw-component-server-settings-row-gap) border-b border-(--mw-component-server-settings-row-divider) py-(--mw-component-server-settings-row-padding-block)"
        >
          <span
            class={[
              "grid size-(--mw-component-custom-emoji-settings-picture-size) shrink-0 place-items-center rounded-(--mw-component-custom-emoji-settings-picture-radius) bg-(--mw-component-custom-emoji-settings-picture-background) p-(--mw-component-custom-emoji-settings-picture-padding)",
              upload.status !== "failed" && "opacity-(--mw-component-composer-tray-item-uploading-opacity)",
            ]}
          >
            {#if upload.previewUrl}
              <img src={upload.previewUrl} alt="" class="size-full object-contain" />
            {:else}
              <Icon name="file" class="size-full text-(--mw-component-server-settings-secondary-text)" />
            {/if}
          </span>
          <!-- Inset like the name fields below, so names line up. -->
          <div class="flex min-w-0 flex-1 flex-col gap-(--mw-component-server-settings-text-gap) px-(--mw-component-text-field-padding-inline)">
            <p class="truncate type-body-lg text-(--mw-component-server-settings-code-text)">{upload.name}</p>
            {#if upload.status === "failed"}
              <p class="type-body-sm text-(--mw-component-settings-error-text)">{upload.error}</p>
            {:else}
              <p class="type-body-sm text-(--mw-component-server-settings-secondary-text)">
                {upload.status === "adding" ? strings.serverSettings.emoji.adding : strings.serverSettings.emoji.waiting}
              </p>
            {/if}
          </div>
          {#if upload.status === "failed"}
            {#if upload.canRetry}
              <Button aria-label={strings.serverSettings.emoji.retry(upload.file.name)} onclick={() => retryUpload(upload)}
                >{strings.common.tryAgain}</Button
              >
            {/if}
            <IconButton
              id="{id}-upload-{upload.key}"
              icon="x"
              label={strings.serverSettings.emoji.dismiss(upload.file.name)}
              class="size-(--mw-size-hit-web-min) text-(--mw-component-server-settings-secondary-text) hover:bg-state-hover hover:text-text-normal"
              iconClass="size-(--mw-component-context-menu-item-icon-size)"
              onclick={() => void dismissUpload(upload.key)}
            />
          {/if}
        </li>
      {/each}
    </ul>
  {/if}

  {#if emojis.length === 0}
    <p class="type-body-lg text-(--mw-component-settings-body-text)">{strings.serverSettings.emoji.empty}</p>
  {:else}
    {#each kinds as kind (kind.key)}
      <section aria-labelledby="{id}-{kind.key}">
        <h3
          class="flex items-baseline justify-between gap-sm border-b border-(--mw-component-server-settings-row-divider) pb-2xs type-category text-(--mw-component-settings-nav-section-text)"
        >
          <span id="{id}-{kind.key}">{kind.title}</span>
          <span
            class={[
              "tabular-nums",
              kind.isFull && "text-(--mw-component-custom-emoji-settings-count-full-text)",
            ]}
          >
            <span aria-hidden="true">{strings.serverSettings.emoji.count(kind.used, MAXIMUM_EMOJIS_PER_GUILD_AND_KIND)}</span>
            <span class="sr-only">{strings.serverSettings.emoji.countLabel(kind.used, MAXIMUM_EMOJIS_PER_GUILD_AND_KIND)}</span>
            {#if kind.isFull}· {strings.serverSettings.emoji.full}{/if}
          </span>
        </h3>
        {#if kind.list.length === 0}
          <p class="py-(--mw-component-server-settings-row-padding-block) type-body text-(--mw-component-server-settings-secondary-text)">
            {kind.empty}
          </p>
        {:else}
          <ul aria-labelledby="{id}-{kind.key}">
            {#each kind.list as emoji (emoji.id)}
              {@const error = renameErrors.get(emoji.id)}
              <li
                class="flex min-h-(--mw-component-server-settings-row-min-height) items-center gap-(--mw-component-server-settings-row-gap) border-b border-(--mw-component-server-settings-row-divider) py-(--mw-component-server-settings-row-padding-block)"
              >
                {@render picture(emoji)}
                <div class="flex min-w-0 flex-1 flex-col gap-(--mw-component-server-settings-text-gap)">
                  <label
                    class="flex h-(--mw-component-text-field-height) max-w-(--mw-component-custom-emoji-settings-name-max-width) items-center rounded-(--mw-component-text-field-radius) border-(length:--mw-component-text-field-border-width) border-transparent px-(--mw-component-text-field-padding-inline) type-body-lg text-(--mw-component-server-settings-code-text) focus-within:focus-ring hover:border-(--mw-component-custom-emoji-settings-name-border-hover) has-[input[aria-invalid=true]]:border-(--mw-component-settings-error-text)"
                  >
                    <span class="sr-only">{strings.serverSettings.emoji.nameLabel(emoji.name)}</span>
                    <input
                      id={nameFieldId(emoji.id)}
                      value={emoji.name}
                      maxlength={MAXIMUM_EMOJI_NAME_LENGTH}
                      autocomplete="off"
                      spellcheck="false"
                      aria-invalid={error ? "true" : undefined}
                      aria-describedby={error ? `${nameFieldId(emoji.id)}-error` : undefined}
                      class="min-w-0 flex-1 bg-transparent outline-none"
                      onkeydown={(event) => handleNameKeydown(event, emoji)}
                      onblur={(event) => void rename(emoji, event.currentTarget)}
                    />
                  </label>
                  {#if error}
                    <p id="{nameFieldId(emoji.id)}-error" role="alert" class="type-body-sm text-(--mw-component-settings-error-text)">{error}</p>
                  {/if}
                </div>
                <IconButton
                  icon="trash"
                  label={strings.serverSettings.emoji.deleteLabel(emoji.name)}
                  aria-haspopup="dialog"
                  class="size-(--mw-size-hit-web-min) text-(--mw-component-server-settings-secondary-text) hover:bg-state-hover hover:text-text-danger"
                  iconClass="size-(--mw-component-context-menu-item-icon-size)"
                  onclick={() => {
                    deleteError = "";
                    focusAfterDeleteId = null;
                    deleting = emoji;
                  }}
                />
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {/each}
  {/if}
  <p role="status" class="sr-only">{notice}</p>
</div>

{#if deleting}
  {@const emoji = deleting}
  <ModalDialog
    title={strings.serverSettings.emoji.deleteTitle(emoji.name)}
    subtitle={strings.serverSettings.emoji.deleteBody}
    errorMessage={deleteError}
    isBusy={isDeleting}
    submitLabel={isDeleting ? strings.serverSettings.emoji.deleting : strings.serverSettings.emoji.deleteSubmit}
    isSubmitDanger
    onclose={() => {
      deleting = null;
      deleteError = "";
    }}
    onsubmit={() => void confirmDelete()}
    fallbackFocus={() => document.getElementById(focusAfterDeleteId ?? `${id}-upload`)?.focus()}
  />
{/if}
