<script lang="ts">
  import { tick } from "svelte";

  import { type MellowClient, describeError } from "../lib/client.svelte";
  import ModalDialog from "./ModalDialog.svelte";
  import TextField from "./TextField.svelte";

  // Discord's "Create a server" and "Join a server" pages in one dialog (DESIGN.md §5.2, §8).
  interface Props {
    client: MellowClient;
    onclose: () => void;
    fallbackFocus: () => void;
  }

  let { client, onclose, fallbackFocus }: Props = $props();

  let isJoining = $state(false);
  let serverName = $state("");
  let inviteCode = $state("");
  let isBusy = $state(false);
  // An error belongs to the page it was raised on: switching pages hides it.
  let error = $state<{ isJoin: boolean; text: string } | null>(null);
  let nameField = $state<HTMLInputElement | null>(null);
  let codeField = $state<HTMLInputElement | null>(null);

  const canSubmit = $derived(!isBusy && (isJoining ? inviteCode.trim() : serverName.trim()) !== "");

  async function submit() {
    const isJoin = isJoining;
    isBusy = true;
    error = null;
    try {
      if (isJoin) {
        await client.joinServer(inviteCode.trim());
      } else {
        await client.createServer(serverName.trim());
      }
      onclose();
    } catch (failure) {
      error = { isJoin, text: describeError(failure, isJoin ? "Couldn't join the server" : "Couldn't create the server") };
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
  title={isJoining ? "Join a server" : "Create a server"}
  subtitle={isJoining
    ? "Enter an invite to join a server someone already runs."
    : "Give your server a name. You can add channels and invite people next."}
  isHeadingCentered
  errorMessage={error && error.isJoin === isJoining ? error.text : ""}
  {isBusy}
  submitLabel={isJoining ? (isBusy ? "Joining…" : "Join") : isBusy ? "Creating…" : "Create"}
  isSubmitEnabled={canSubmit}
  {onclose}
  onsubmit={submit}
  {fallbackFocus}
>
  {#if isJoining}
    <TextField
      id="join-server-invite-code"
      label="Invite code"
      helper="Paste the invite code someone gave you."
      autocomplete="off"
      readonly={isBusy}
      data-initial-focus=""
      bind:value={inviteCode}
      bind:element={codeField}
    />
  {:else}
    <TextField
      id="create-server-name"
      label="Server name"
      autocomplete="off"
      readonly={isBusy}
      data-initial-focus=""
      bind:value={serverName}
      bind:element={nameField}
    />
  {/if}
  <p class="flex flex-wrap items-center justify-center gap-x-(--mw-space-2xs) type-body text-(--mw-component-dialog-hint-text)">
    {isJoining ? "Don't have an invite?" : "Have an invite already?"}
    <button
      type="button"
      class="min-h-(--mw-size-hit-web-min) font-medium text-(--mw-component-dialog-mode-link-text) hover:underline aria-disabled:opacity-(--mw-component-button-disabled-opacity)"
      aria-disabled={isBusy || undefined}
      onclick={switchPage}>{isJoining ? "Create a server" : "Join a server"}</button
    >
  </p>
</ModalDialog>
