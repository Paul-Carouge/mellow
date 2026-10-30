<script lang="ts">
  import { onMount } from "svelte";

  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { Guild, GuildInvite } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import { displayName, fullTimestampLabel } from "./format";
  import Icon from "./Icon.svelte";
  import ModalDialog from "./ModalDialog.svelte";
  import SettingsListStatus from "./SettingsListStatus.svelte";

  // Invites (DESIGN.md §5.8): the invites that still work, newest first, with their code, who
  // created them, their uses and when they expire. Revoking asks first (§8). No gateway event
  // tells about invites, so the list is read when the page opens and after each change.
  interface Props {
    client: MellowClient;
    guild: Guild;
  }

  let { client, guild }: Props = $props();

  const id = $props.id();
  let invites = $state<GuildInvite[] | null>(null);
  let hasFailed = $state(false);
  let isCreating = $state(false);
  let errorMessage = $state("");
  let notice = $state("");
  let revoking = $state<GuildInvite | null>(null);
  let isRevoking = $state(false);
  let revokeError = $state("");

  async function load() {
    hasFailed = false;
    try {
      invites = await client.rest.listInvites(guild.id);
    } catch (error) {
      console.warn("loading the invites failed", error);
      invites = null;
      hasFailed = true;
    }
  }

  onMount(() => {
    void load();
  });

  async function create() {
    if (isCreating) {
      return;
    }
    isCreating = true;
    errorMessage = "";
    try {
      const { code } = await client.createInvite(guild.id);
      notice = strings.serverSettings.invites.created(code);
      await load();
    } catch (error) {
      errorMessage = describeError(error, strings.serverSettings.invites.createFailed);
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
    <Button id="{id}-create" variant="primary" isDisabled={isCreating} onclick={() => void create()}>
      {isCreating ? strings.serverSettings.invites.creating : strings.serverSettings.invites.createInvite}
    </Button>
  </div>
  <div role="alert" class="contents">
    {#if errorMessage}
      <p class="flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-settings-error-text)">
        <Icon name="warning-circle" class="size-(--mw-component-settings-error-icon-size) shrink-0" />
        <span>{errorMessage}</span>
      </p>
    {/if}
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
              {strings.serverSettings.invites.uses(invite.uses, invite.max_uses)}
              <span aria-hidden="true">·</span>
              {invite.expires_at_unix_milliseconds === null
                ? strings.serverSettings.invites.neverExpires
                : strings.serverSettings.invites.expires(fullTimestampLabel(invite.expires_at_unix_milliseconds))}
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
