<script lang="ts">
  import { onMount } from "svelte";

  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { Snowflake } from "../lib/protocol";
  import Button from "./Button.svelte";
  import ModalDialog from "./ModalDialog.svelte";
  import TextField from "./TextField.svelte";
  import { durationTokenInMilliseconds } from "./tokens";

  // Discord's invite dialog, with a code instead of a link (DESIGN.md §5.3, §8 "Invite people").
  interface Props {
    client: MellowClient;
    guildId: Snowflake;
    serverName: string;
    onclose: () => void;
    fallbackFocus: () => void;
  }

  let { client, guildId, serverName, onclose, fallbackFocus }: Props = $props();

  let inviteCode = $state("");
  let errorMessage = $state("");
  let isCopied = $state(false);
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;

  onMount(() => {
    client
      .createInvite(guildId)
      .then((code) => (inviteCode = code))
      .catch((error: unknown) => (errorMessage = describeError(error, strings.invitePeople.failed)));
    return () => clearTimeout(copiedTimer);
  });

  async function copy() {
    if (!inviteCode) {
      return;
    }
    try {
      await navigator.clipboard.writeText(inviteCode);
    } catch (error) {
      console.warn("copying the invite code failed", error);
      errorMessage = strings.invitePeople.copyFailed;
      return;
    }
    isCopied = true;
    clearTimeout(copiedTimer);
    // Reduced motion sets the token to 0 ms; "Copied" then stays rather than flashing away.
    const copiedLabelDuration = durationTokenInMilliseconds("component-dialog-copied-label-duration");
    if (copiedLabelDuration > 0) {
      copiedTimer = setTimeout(() => (isCopied = false), copiedLabelDuration);
    }
  }
</script>

<ModalDialog title={strings.invitePeople.title(serverName)} cancelLabel={strings.common.close} {errorMessage} {onclose} {fallbackFocus}>
  <div class="flex flex-col gap-(--mw-component-text-field-helper-gap)">
    <div class="flex items-end gap-(--mw-component-dialog-button-gap)">
      <div class="min-w-0 flex-1">
        <TextField
          id="invite-code"
          label={strings.invitePeople.codeLabel}
          value={inviteCode}
          placeholder={errorMessage ? "" : strings.invitePeople.creating}
          readonly
          isMonospace
          data-initial-focus=""
          onfocus={(event) => event.currentTarget.select()}
        />
      </div>
      <div class="pb-[calc((var(--mw-component-text-field-height)_-_var(--mw-component-button-height-large))/2)]">
        <Button variant="primary" isDisabled={!inviteCode} onclick={copy}>{isCopied ? strings.invitePeople.copied : strings.invitePeople.copy}</Button>
      </div>
    </div>
    <p class="type-body-sm text-(--mw-component-dialog-hint-text)">{strings.invitePeople.neverExpires}</p>
  </div>
  <p class="sr-only" aria-live="polite">{isCopied ? strings.invitePeople.codeCopied : ""}</p>
</ModalDialog>
