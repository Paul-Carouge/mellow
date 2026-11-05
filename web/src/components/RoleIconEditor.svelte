<script lang="ts">
  import { strings } from "../i18n";
  import { ApiError } from "../lib/api";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import { type CustomEmoji, type Guild, MAXIMUM_ROLE_ICON_UPLOAD_BYTES, PICTURE_UPLOAD_TYPES, type Role } from "../lib/protocol";
  import { reactionKey } from "../lib/reactions";
  import Button from "./Button.svelte";
  import EmojiPicker from "./EmojiPicker.svelte";
  import Icon from "./Icon.svelte";
  import RoleIcon from "./RoleIcon.svelte";
  import { hasRoleIcon } from "./role-icons";

  // Server settings, Roles, Display: the role's icon (ADR-0050, DESIGN.md §5.8). A picture (its
  // centred square, animated GIFs kept animated, like emoji) or a Unicode emoji, one replacing the
  // other. Saved at once, as on Discord's role editor; the new icon comes back with the gateway's
  // role list, like every role edit.
  interface Props {
    client: MellowClient;
    guild: Guild;
    role: Role;
    isEditable: boolean;
    isCompact: boolean;
  }

  let { client, guild, role, isEditable, isCompact }: Props = $props();

  const id = $props.id();
  const MAXIMUM_KILOBYTES = MAXIMUM_ROLE_ICON_UPLOAD_BYTES / 1024;
  let busy = $state<"picture" | "emoji" | "remove" | null>(null);
  let errorMessage = $state("");
  let notice = $state("");
  let fileInput = $state<HTMLInputElement | null>(null);
  let emojiButton = $state<HTMLElement | null>(null);
  let isPickerOpen = $state(false);
  const hasIcon = $derived(hasRoleIcon(role));

  async function change(
    kind: "picture" | "emoji" | "remove",
    icon: Parameters<MellowClient["setRoleIcon"]>[2],
  ) {
    if (busy) {
      return;
    }
    busy = kind;
    errorMessage = "";
    notice = "";
    try {
      await client.setRoleIcon(guild.id, role.id, icon);
      notice = kind === "remove" ? strings.roleIcons.removed : strings.roleIcons.saved;
    } catch (error) {
      const summary = kind === "remove" ? strings.roleIcons.removeFailed : strings.roleIcons.changeFailed;
      if (error instanceof ApiError && error.code === "FILE_TOO_LARGE") {
        errorMessage = strings.errors.withReason(summary, strings.roleIcons.tooLarge(MAXIMUM_KILOBYTES));
      } else if (error instanceof ApiError && error.code === "VALIDATION_FAILED" && kind === "picture") {
        errorMessage = strings.errors.withReason(summary, strings.roleIcons.unreadable);
      } else {
        errorMessage = describeError(error, summary);
      }
    } finally {
      busy = null;
    }
  }

  function upload(input: HTMLInputElement) {
    const file = input.files?.[0];
    // Cleared, so choosing the same file again (after an error) still fires `change`.
    input.value = "";
    if (!file) {
      return;
    }
    if (file.type !== "" && !PICTURE_UPLOAD_TYPES.includes(file.type)) {
      errorMessage = strings.errors.withReason(strings.roleIcons.changeFailed, strings.profile.notAPicture);
      return;
    }
    if (file.size > MAXIMUM_ROLE_ICON_UPLOAD_BYTES) {
      errorMessage = strings.errors.withReason(strings.roleIcons.changeFailed, strings.roleIcons.tooLarge(MAXIMUM_KILOBYTES));
      return;
    }
    void change("picture", { kind: "picture", file });
  }

  function pick(emoji: string | CustomEmoji) {
    isPickerOpen = false;
    emojiButton?.focus();
    // Server emoji are offered by recent picks only; a role icon is a Unicode emoji.
    if (typeof emoji === "string") {
      void change("emoji", { kind: "emoji", emoji: reactionKey(emoji) });
    }
  }
</script>

<section aria-labelledby="{id}-title" aria-busy={busy !== null} class="flex flex-col gap-(--mw-component-settings-section-label-gap)">
  <h4 id="{id}-title" class="type-category text-(--mw-component-settings-section-label-text)">{strings.roleIcons.title}</h4>
  <p id="{id}-intro" class="type-body text-(--mw-component-roles-permission-description-text)">{strings.roleIcons.intro}</p>
  <div class="flex flex-wrap items-center gap-(--mw-component-role-icon-editor-gap)">
    <div
      role="img"
      aria-label={hasIcon ? strings.roleIcons.currentIcon(role.name) : strings.roleIcons.none}
      class={[
        "grid size-(--mw-component-role-icon-editor-tile-size) shrink-0 place-items-center rounded-(--mw-component-role-icon-editor-tile-radius) bg-(--mw-component-role-icon-editor-tile-background)",
        hasIcon
          ? "border-(length:--mw-component-text-field-border-width) border-(--mw-component-role-icon-editor-tile-border)"
          : "border-(length:--mw-component-text-field-border-width) border-dashed border-(--mw-component-role-icon-editor-empty-icon)",
      ]}
    >
      {#if hasIcon}
        <RoleIcon {role} sizeToken="--mw-component-role-icon-editor-preview-size" isDecorative />
      {:else}
        <Icon name="plus" class="size-(--mw-component-text-field-icon-size) text-(--mw-component-role-icon-editor-empty-icon)" />
      {/if}
    </div>
    <input
      bind:this={fileInput}
      type="file"
      accept={PICTURE_UPLOAD_TYPES.join(",")}
      tabindex="-1"
      aria-hidden="true"
      class="sr-only"
      onchange={(event) => upload(event.currentTarget)}
    />
    <div class="flex flex-wrap gap-(--mw-component-settings-account-button-gap)">
      <Button aria-describedby="{id}-formats" isDisabled={!isEditable || busy !== null} onclick={() => fileInput?.click()}>
        {busy === "picture" ? strings.roleIcons.uploading : strings.roleIcons.uploadPicture}
      </Button>
      <Button
        id="{id}-emoji"
        aria-haspopup="dialog"
        aria-expanded={isPickerOpen}
        isDisabled={!isEditable || busy !== null}
        onclick={() => {
          emojiButton = document.getElementById(`${id}-emoji`);
          isPickerOpen = true;
        }}
      >
        <Icon name="smiley" class="me-2xs size-(--mw-component-text-field-icon-size)" />
        {busy === "emoji" ? strings.roleIcons.uploading : strings.roleIcons.chooseEmoji}
      </Button>
      {#if hasIcon}
        <Button variant="ghost" isDisabled={!isEditable || busy !== null} onclick={() => void change("remove", { kind: "none" })}>
          {busy === "remove" ? strings.roleIcons.removing : strings.roleIcons.remove}
        </Button>
      {/if}
    </div>
  </div>
  <p id="{id}-formats" class="type-body-sm text-(--mw-component-roles-note-text)">{strings.roleIcons.formats(MAXIMUM_KILOBYTES)}</p>
  <div role="alert" class="contents">
    {#if errorMessage}
      <p class="flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-roles-error-text)">
        <Icon name="warning-circle" class="size-(--mw-component-settings-error-icon-size) shrink-0" />
        <span>{errorMessage}</span>
      </p>
    {/if}
  </div>
  <p role="status" class="sr-only">{notice}</p>
</section>

{#if isPickerOpen && emojiButton}
  <EmojiPicker
    anchor={emojiButton}
    {isCompact}
    server={null}
    onpick={pick}
    onclose={(shouldReturnFocus) => {
      isPickerOpen = false;
      if (shouldReturnFocus) {
        emojiButton?.focus();
      }
    }}
  />
{/if}
