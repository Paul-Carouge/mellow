<script lang="ts">
  import { onMount } from "svelte";

  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { InviteLimits } from "../lib/api";
  import type { Guild, GuildInvite } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import { displayName, fullTimestampLabel } from "./format";
  import InviteLimitsFields from "./InviteLimitsFields.svelte";
  import { DEFAULT_INVITE_LIMITS, expiryLabel } from "./invites";
  import ModalDialog from "./ModalDialog.svelte";
  import SettingsListStatus from "./SettingsListStatus.svelte";

  // Invites (DESIGN.md §5.8): the invites that still work, newest first, with their code, who
  // created them, their uses and when they expire. "Create invite" asks for the limits first, as
  // "Invite people" does; revoking asks first (§8). No gateway event tells about invites, so the
  // list is read when the page opens and after each change.
  interface Props {
    client: MellowClient;
    guild: Guild;
  }

  let { client, guild }: Props = $props();

  const id = $props.id();
  let invites = $state<GuildInvite[] | null>(null);
  let hasFailed = $state(false);
  const EXPIRY_REFRESH_MILLISECONDS = 60_000;
  let isCreateDialogOpen = $state(false);
  let limitsToCreate = $state<InviteLimits>(DEFAULT_INVITE_LIMITS);
  let isCreating = $state(false);
  let createError = $state("");
  /** "Expires in 3 days" counts from now, refreshed every minute while the page is open. */
  let nowUnixMilliseconds = $state(Date.now());
  let notice = $state("");
  let revoking = $state<GuildInvite | null>(null);
  let isRevoking = $state(false);
  let revokeError = $state("");

  async function load() {
    hasFailed = false;
    try {
      invites = await client.rest.listInvites(guild.id);
      nowUnixMilliseconds = Date.now();
    } catch (error) {
      console.warn("loading the invites failed", error);
      invites = null;
      hasFailed = true;
    }
  }

  onMount(() => {
    void load();
    const clock = setInterval(() => (nowUnixMilliseconds = Date.now()), EXPIRY_REFRESH_MILLISECONDS);
    return () => clearInterval(clock);
  });

  async function create() {
    if (isCreating) {
      return;
    }
    isCreating = true;
    createError = "";
    try {
      const { code } = await client.createInvite(guild.id, limitsToCreate);
      isCreateDialogOpen = false;
      notice = strings.serverSettings.invites.created(code);
      await load();
    } catch (error) {
      createError = describeError(error, strings.serverSettings.invites.createFailed);
    } finally {
      isCreating = false;
    }
  }

  async function revoke() {
    const invite = revoking;
    if (!invite) {
      return;
    }
    isRevoking = true;
    revokeError = "";
    try {
      await client.rest.revokeInvite(invite.code);
      invites = invites?.filter((candidate) => candidate.code !== invite.code) ?? null;
      revoking = null;
    } catch (error) {
      revokeError = describeError(error, strings.serverSettings.invites.revokeFailed);
    } finally {
      isRevoking = false;
    }
  }
</script>

<div class="flex flex-col gap-(--mw-component-server-settings-toolbar-gap)">
  <div>
    <Button
      id="{id}-create"
      variant="primary"
      aria-haspopup="dialog"
      onclick={() => {
        limitsToCreate = DEFAULT_INVITE_LIMITS;
        isCreateDialogOpen = true;
      }}
    >
      {strings.serverSettings.invites.createInvite}
    </Button>
  </div>

  {#if invites === null}
    <SettingsListStatus status={hasFailed ? "failed" : "loading"} failureText={strings.serverSettings.invites.loadFailed} onretry={() => void load()} />
  {:else if invites.length === 0}
    <p class="type-body-lg text-(--mw-component-settings-body-text)">{strings.serverSettings.invites.empty}</p>
  {:else}
    <ul aria-label={strings.serverSettings.sections.invites}>
      {#each invites as invite (invite.code)}
        <li
          class="flex min-h-(--mw-component-server-settings-row-min-height) items-center gap-(--mw-component-server-settings-row-gap) border-b border-(--mw-component-server-settings-row-divider) py-(--mw-component-server-settings-row-padding-block)"
        >
          <Avatar user={invite.creator} sizeToken="--mw-component-server-settings-avatar-size" />
          <div class="flex min-w-0 flex-1 flex-col gap-(--mw-component-server-settings-text-gap)">
            <p class="truncate font-mono type-body-lg text-(--mw-component-server-settings-code-text)">{invite.code}</p>
            <p class="type-body-sm text-(--mw-component-server-settings-secondary-text)">
              {strings.serverSettings.invites.createdBy(displayName(invite.creator))}
            </p>
            <p class="type-body-sm text-(--mw-component-server-settings-secondary-text)">
              <!-- Each part stays on one line; a narrow row breaks at the dot. -->
              <span class="whitespace-nowrap">{strings.serverSettings.invites.uses(invite.uses, invite.max_uses)}</span>
              <span aria-hidden="true">·</span>
              {#if invite.expires_at_unix_milliseconds === null}
                <span class="whitespace-nowrap">{strings.serverSettings.invites.neverExpires}</span>
              {:else}
                <time
                  class="whitespace-nowrap"
                  datetime={new Date(invite.expires_at_unix_milliseconds).toISOString()}
                  title={fullTimestampLabel(invite.expires_at_unix_milliseconds)}
                  >{expiryLabel(invite.expires_at_unix_milliseconds, nowUnixMilliseconds)}</time
                >
              {/if}
            </p>
          </div>
          <Button
            aria-label={strings.serverSettings.invites.revokeLabel(invite.code)}
            aria-haspopup="dialog"
            onclick={() => (revoking = invite)}
          >
            {strings.serverSettings.invites.revoke}
          </Button>
        </li>
      {/each}
    </ul>
  {/if}
  <p role="status" class="sr-only">{notice}</p>
</div>

{#if isCreateDialogOpen}
  <ModalDialog
    title={strings.serverSettings.invites.createTitle}
    errorMessage={createError}
    isBusy={isCreating}
    submitLabel={isCreating ? strings.serverSettings.invites.creating : strings.serverSettings.invites.createSubmit}
    onclose={() => {
      isCreateDialogOpen = false;
      createError = "";
    }}
    onsubmit={() => void create()}
    fallbackFocus={() => document.getElementById(`${id}-create`)?.focus()}
  >
    <InviteLimitsFields bind:limits={limitsToCreate} isDisabled={isCreating} />
  </ModalDialog>
{/if}

{#if revoking}
  <ModalDialog
    title={strings.serverSettings.invites.revokeTitle}
    subtitle={strings.serverSettings.invites.revokeBody}
    errorMessage={revokeError}
    isBusy={isRevoking}
    submitLabel={isRevoking ? strings.serverSettings.invites.revoking : strings.serverSettings.invites.revokeSubmit}
    isSubmitDanger
    onclose={() => {
      revoking = null;
      revokeError = "";
    }}
    onsubmit={() => void revoke()}
    fallbackFocus={() => document.getElementById(`${id}-create`)?.focus()}
  />
{/if}
