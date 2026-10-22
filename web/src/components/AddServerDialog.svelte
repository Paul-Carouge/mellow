<script lang="ts">
  import { tick } from "svelte";

  import { strings } from "../i18n";
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
  // Plain: read by the dialog when it closes, after this component stopped updating.
  let hasSucceeded = false;
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
  errorMessage={error && error.isJoin === isJoining ? error.text : ""}
  {isBusy}
  submitLabel={isJoining
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
    <TextField
      id="create-server-name"
      label={strings.addServer.serverName}
      autocomplete="off"
      readonly={isBusy}
      data-initial-focus=""
      bind:value={serverName}
      bind:element={nameField}
    />
  {/if}
  <p class="flex flex-wrap items-center justify-center gap-x-(--mw-space-2xs) type-body text-(--mw-component-dialog-hint-text)">
    {isJoining ? strings.addServer.hasNoInvite : strings.addServer.hasInvite}
    <button
      type="button"
      class="min-h-(--mw-size-hit-web-min) font-medium text-(--mw-component-dialog-mode-link-text) hover:underline aria-disabled:opacity-(--mw-component-button-disabled-opacity)"
      aria-disabled={isBusy || undefined}
      onclick={switchPage}>{isJoining ? strings.addServer.createTitle : strings.addServer.joinTitle}</button
    >
  </p>
</ModalDialog>
