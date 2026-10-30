<script lang="ts">
  import { onMount, tick } from "svelte";

  import { strings } from "../i18n";
  import type { InviteLimits } from "../lib/api";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { Invite, Snowflake } from "../lib/protocol";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";
  import InviteLimitsFields from "./InviteLimitsFields.svelte";
  import { DEFAULT_INVITE_LIMITS, inviteCodeGroups, inviteLimitsSentence, spokenInviteCode } from "./invites";
  import ModalDialog from "./ModalDialog.svelte";
  import { durationTokenInMilliseconds } from "./tokens";

  // Discord's invite dialog, with a code instead of a link (DESIGN.md §5.3, §6.57, §8 "Invite
  // people"). Opening creates an invite with Discord's default limits (a week, no use limit); the
  // limits change only when "Create a new code" is pressed, so browsing the presets creates nothing.
  interface Props {
    client: MellowClient;
    guildId: Snowflake;
    serverName: string;
    onclose: () => void;
    fallbackFocus: () => void;
  }

  let { client, guildId, serverName, onclose, fallbackFocus }: Props = $props();

  const id = $props.id();
  let invite = $state<Invite | null>(null);
  /** The limits the shown code was created with, for the sentence under it. */
  let limitsOfCode = $state<InviteLimits>(DEFAULT_INVITE_LIMITS);
  /** What the selects say; applied by "Create a new code". */
  let chosenLimits = $state<InviteLimits>(DEFAULT_INVITE_LIMITS);
  let isLimitsPanelOpen = $state(false);
  let isCreating = $state(false);
  let errorMessage = $state("");
  let isCopied = $state(false);
  let announcement = $state("");
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;

  const codeGroups = $derived(invite ? inviteCodeGroups(invite.code) : []);

  async function createInvite(limits: InviteLimits) {
    if (isCreating) {
      return;
    }
    const isReplacingCode = invite !== null;
    isCreating = true;
    errorMessage = "";
    try {
      const created = await client.createInvite(guildId, limits);
      invite = created;
      limitsOfCode = limits;
      isCopied = false;
      if (isReplacingCode) {
        announcement = strings.invitePeople.newCodeCreated(spokenInviteCode(created.code));
      }
    } catch (error) {
      errorMessage = describeError(error, strings.invitePeople.failed);
    } finally {
      isCreating = false;
    }
  }

  onMount(() => {
    void createInvite(DEFAULT_INVITE_LIMITS);
    return () => clearTimeout(copiedTimer);
  });

  async function copy() {
    if (!invite) {
      return;
    }
    try {
      await navigator.clipboard.writeText(invite.code);
    } catch (error) {
      console.warn("copying the invite code failed", error);
      errorMessage = strings.invitePeople.copyFailed;
      return;
    }
    isCopied = true;
    // Emptied first, so a second copy is announced again.
    announcement = "";
    await tick();
    announcement = strings.invitePeople.codeCopied;
    clearTimeout(copiedTimer);
    // Reduced motion sets the token to 0 ms; "Copied" then stays rather than flashing away.
    const copiedLabelDuration = durationTokenInMilliseconds("component-dialog-copied-label-duration");
    if (copiedLabelDuration > 0) {
      copiedTimer = setTimeout(() => (isCopied = false), copiedLabelDuration);
    }
  }
</script>

<ModalDialog
  title={strings.invitePeople.title(serverName)}
  subtitle={strings.invitePeople.subtitle}
  cancelLabel={strings.common.close}
  {errorMessage}
  {onclose}
  {fallbackFocus}
>
  <div class="flex flex-col gap-(--mw-component-invite-stack-gap)">
    <div
      class="@container flex flex-col items-center justify-center rounded-(--mw-component-invite-code-radius) border-(length:--mw-component-invite-code-border-width) border-(--mw-component-invite-code-border) bg-(--mw-component-invite-code-background) px-(--mw-component-invite-code-padding-inline) py-(--mw-component-invite-code-padding-block) text-center"
    >
      {#if invite}
        <!-- Spelled out for screen readers; on screen the dashes step back so the groups read as
             words, and a click selects the whole code for copying by hand. -->
        <p class="sr-only">{strings.invitePeople.codeLabel(spokenInviteCode(invite.code))}</p>
        <p aria-hidden="true" class="invite-code type-invite-code text-(--mw-component-invite-code-text) select-all">
          {#each codeGroups as group, index (index)}{#if index > 0}<span class="text-(--mw-component-invite-code-dash-text)">-</span>{/if}{group}{/each}
        </p>
      {:else if errorMessage && !isCreating}
        <Button onclick={() => void createInvite(limitsOfCode)}>{strings.common.tryAgain}</Button>
      {:else}
        <!-- Inside the code's own line box, so the well keeps its height when the code arrives. -->
        <p class="invite-code type-invite-code"><span class="type-body text-(--mw-component-dialog-hint-text)">{strings.invitePeople.creating}</span></p>
      {/if}
    </div>
    <Button variant="primary" isFullWidth isDisabled={!invite} data-initial-focus="" onclick={() => void copy()}>
      {isCopied ? strings.invitePeople.copied : strings.invitePeople.copy}
    </Button>
    {#if invite}
      <p class="type-body-sm text-(--mw-component-invite-limits-text)">{inviteLimitsSentence(limitsOfCode)}</p>
    {/if}
  </div>

  <div class="flex flex-col gap-(--mw-component-invite-stack-gap)">
    <button
      type="button"
      aria-expanded={isLimitsPanelOpen}
      aria-controls="{id}-limits"
      class="-ms-(--mw-space-2xs) flex min-h-(--mw-size-hit-web-min) items-center gap-(--mw-space-2xs) self-start rounded-(--mw-radius-xs) px-(--mw-space-2xs) font-medium type-body text-(--mw-component-dialog-mode-link-text) hover:underline focus-visible:focus-ring"
      onclick={() => (isLimitsPanelOpen = !isLimitsPanelOpen)}
    >
      <Icon name={isLimitsPanelOpen ? "caret-down" : "caret-right"} class="size-(--mw-component-category-chevron-size)" />
      {strings.invitePeople.changeLimits}
    </button>
    <div id="{id}-limits" hidden={!isLimitsPanelOpen} class="flex flex-col gap-(--mw-component-dialog-section-gap)">
      <InviteLimitsFields bind:limits={chosenLimits} isDisabled={isCreating} />
      <div class="flex flex-wrap items-center justify-between gap-(--mw-component-dialog-button-gap)">
        <p class="type-body-sm text-(--mw-component-dialog-hint-text)">{strings.invitePeople.previousCodeStays}</p>
        <Button isDisabled={isCreating} onclick={() => void createInvite(chosenLimits)}>
          {isCreating && invite ? strings.invitePeople.creatingNewCode : strings.invitePeople.createNewCode}
        </Button>
      </div>
    </div>
  </div>
  <p class="sr-only" aria-live="polite">{announcement}</p>
</ModalDialog>

<style>
  /* One line always: on a phone the code shrinks to its well's width rather than breaking. */
  .invite-code {
    font-size: min(
      var(--mw-type-invite-code-font-size),
      calc(100cqi / var(--mw-component-invite-code-width-in-em))
    );
    white-space: nowrap;
  }
</style>
