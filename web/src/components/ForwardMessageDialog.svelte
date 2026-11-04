<script lang="ts">
  import { strings } from "../i18n";
  import { generateMessageNonce } from "../lib/api";
  import type { MellowClient } from "../lib/client.svelte";
  import { MAXIMUM_MESSAGE_LENGTH, type Message, type Snowflake } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import {
    type ForwardAttempts,
    type ForwardDestination,
    MAXIMUM_FORWARD_DESTINATIONS,
    destinationLabel,
    destinationsMatching,
    forwardDestinations,
    forwardToEach,
    inOpeningOrder,
  } from "./forward-messages";
  import GroupAvatar from "./GroupAvatar.svelte";
  import Icon from "./Icon.svelte";
  import ModalDialog from "./ModalDialog.svelte";
  import { messageAsPlainText } from "./role-mentions";
  import TextField from "./TextField.svelte";

  // "Forward message" (DESIGN.md §5.4, ADR-0039): the places the reader may post in, searchable,
  // up to five; the forward as it will arrive; an optional message sent after it in each place.
  // Each place gets its own requests: those that worked leave the selection, those that failed
  // stay picked with their reason, so trying again only sends what is missing.
  interface Props {
    client: MellowClient;
    message: Message;
    /** The chat has a composer to take focus after a forward; a read-only one gives it back to the opener. */
    canFocusComposer: boolean;
    onclose: () => void;
    /** After every place got the forward: "Forwarded to #general and Ana.", for the chat's notice. */
    onforwarded: (sentence: string) => void;
    fallbackFocus: () => void;
  }

  let { client, message, canFocusComposer, onclose, onforwarded, fallbackFocus }: Props = $props();

  const id = $props.id();
  let query = $state("");
  let note = $state("");
  let pickedIds = $state<Snowflake[]>([]);
  let isBusy = $state(false);
  let errorMessage = $state("");
  /** The places reached when others refused: said apart from the failures, which are red. */
  let partialSuccess = $state("");
  /** Why a place refused, shown on its row until the next try. */
  let refusalByChannelId = $state<ReadonlyMap<Snowflake, string>>(new Map());
  // Plain: read by the dialog when it closes, after this component stopped updating.
  let hasSucceeded = false;
  // Plain: carried from one try to the next, never shown.
  const attempts: ForwardAttempts = { noncesByChannelId: new Map(), forwardedChannelIds: new Set() };

  // Plain: taken from the first list, then only read.
  let orderAtOpening: Snowflake[] | null = null;
  const destinations = $derived.by(() => {
    const current = client.user
      ? forwardDestinations({
          guilds: client.guilds,
          conversations: client.conversations,
          currentUserId: client.user.id,
          isBlocked: (userId) => client.isBlocked(userId),
          hasAttachments: (message.attachments ?? []).length > 0,
        })
      : [];
    orderAtOpening ??= current.map((destination) => destination.channelId);
    return inOpeningOrder(current, orderAtOpening);
  });
  // A place that goes away meanwhile (left, hidden, blocked) leaves the selection by itself.
  const pickedDestinations = $derived(
    pickedIds.flatMap((channelId) => destinations.find((destination) => destination.channelId === channelId) ?? []),
  );
  const destinationsMatchingQuery = $derived(destinationsMatching(destinations, query));
  const remainingDestinationCount = $derived(MAXIMUM_FORWARD_DESTINATIONS - pickedDestinations.length);
  const attachmentCount = $derived((message.attachments ?? []).length);

  function isPicked(channelId: Snowflake): boolean {
    return pickedIds.includes(channelId);
  }

  function togglePick(destination: ForwardDestination, isChecked: boolean) {
    pickedIds = isChecked
      ? [...pickedIds, destination.channelId]
      : pickedIds.filter((channelId) => channelId !== destination.channelId);
  }

  /** Enter after a search picks the first place it found and clears it; with nothing typed, Enter forwards. */
  function pickFromSearch(event: KeyboardEvent) {
    if (event.key !== "Enter" || query.trim() === "") {
      return;
    }
    event.preventDefault();
    // The places being sent to stay as they are until the answers are in.
    if (isBusy) {
      return;
    }
    const first = destinationsMatchingQuery.find((destination) => !isPicked(destination.channelId));
    if (first && remainingDestinationCount > 0) {
      togglePick(first, true);
      query = "";
    }
  }

  async function submit() {
    const targets = pickedDestinations;
    if (targets.length === 0) {
      return;
    }
    isBusy = true;
    errorMessage = "";
    partialSuccess = "";
    const { reached, refused } = await forwardToEach(targets, note.trim(), attempts, {
      forward: (channelId, nonce) => client.forwardMessage(channelId, message, nonce),
      sendNote: (channelId, text, nonce) => client.sendMessageTo(channelId, text, nonce),
      createNonce: generateMessageNonce,
    });
    isBusy = false;
    const reachedLabels = reached.map(destinationLabel);
    if (refused.length === 0) {
      hasSucceeded = true;
      onforwarded(strings.forward.sent(reachedLabels));
      onclose();
      return;
    }
    // What worked leaves the selection, so trying again only sends to the places that refused.
    pickedIds = refused.map(({ destination }) => destination.channelId);
    refusalByChannelId = new Map(refused.map(({ destination, reason }) => [destination.channelId, reason]));
    partialSuccess = reachedLabels.length > 0 ? strings.forward.sent(reachedLabels) : "";
    errorMessage = refused.map(({ sentence }) => sentence).join(" ");
  }
</script>

<ModalDialog
  title={strings.forward.title}
  subtitle={strings.forward.subtitle(MAXIMUM_FORWARD_DESTINATIONS)}
  {errorMessage}
  {isBusy}
  submitLabel={destinations.length === 0 ? "" : isBusy ? strings.forward.submitting : strings.forward.submit}
  isSubmitEnabled={pickedDestinations.length > 0}
  shouldReturnFocusToOpener={() => !hasSucceeded || !canFocusComposer}
  {onclose}
  onsubmit={() => void submit()}
  {fallbackFocus}
>
  {#if destinations.length === 0}
    <p class="type-body-lg text-text-secondary">{strings.forward.noPlaces}</p>
  {:else}
    <div class="flex flex-col gap-(--mw-component-text-field-label-gap)">
      <label for="{id}-search" class="type-category text-(--mw-component-text-field-label-text)">
        {strings.forward.search}
      </label>
      <div
        class="flex min-h-(--mw-component-text-field-height) items-center gap-xs rounded-(--mw-component-text-field-radius) border-(length:--mw-component-text-field-border-width) border-(--mw-component-text-field-border) bg-(--mw-component-text-field-background) px-(--mw-component-text-field-padding-inline) focus-within:focus-ring"
      >
        <Icon name="magnifying-glass" class="size-(--mw-component-text-field-icon-size) text-(--mw-component-text-field-placeholder)" />
        <input
          bind:value={query}
          id="{id}-search"
          type="search"
          autocomplete="off"
          data-initial-focus=""
          placeholder={strings.forward.searchPlaceholder}
          aria-describedby="{id}-room"
          aria-controls="{id}-list"
          class="min-w-0 flex-1 bg-transparent type-body-lg text-(--mw-component-text-field-text) outline-none placeholder:text-(--mw-component-text-field-placeholder)"
          onkeydown={pickFromSearch}
        />
      </div>
      <p id="{id}-room" aria-live="polite" class="type-body text-(--mw-component-people-picker-counter-text)">
        {strings.forward.roomLeft(remainingDestinationCount)}
      </p>
    </div>

    <div
      id="{id}-list"
      class="-mx-(--mw-component-people-picker-row-padding-inline) max-h-(--mw-component-people-picker-list-max-height) overflow-y-auto [scrollbar-width:thin]"
    >
      {#if destinationsMatchingQuery.length === 0}
        <p class="px-(--mw-component-people-picker-row-padding-inline) type-body text-text-muted">
          {strings.forward.noMatch(query.trim())}
        </p>
      {:else}
        <ul aria-label={strings.forward.places}>
          {#each destinationsMatchingQuery as destination (destination.channelId)}
            {@const isChecked = isPicked(destination.channelId)}
            {@const isFull = remainingDestinationCount === 0 && !isChecked}
            {@const refusal = refusalByChannelId.get(destination.channelId)}
            <li class="mt-(--mw-component-people-picker-row-gap)">
              <label
                class={[
                  "flex min-h-(--mw-component-people-picker-row-height) items-center gap-sm rounded-(--mw-component-people-picker-row-radius) px-(--mw-component-people-picker-row-padding-inline) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard)",
                  isFull ? "opacity-(--mw-component-button-disabled-opacity)" : "cursor-pointer hover:bg-(--mw-component-people-picker-row-background-hover)",
                ]}
              >
                {#if destination.kind === "channel"}
                  <!-- A place, square like a server in the strip; people stay round. -->
                  <span
                    aria-hidden="true"
                    class="grid size-(--mw-component-people-picker-avatar-size) shrink-0 place-items-center rounded-md bg-neutral-fill text-text-on-neutral"
                  >
                    <Icon name="hash" class="size-(--mw-component-text-field-icon-size)" />
                  </span>
                {:else if destination.kind === "group"}
                  <GroupAvatar
                    people={destination.people}
                    sizeToken="--mw-component-people-picker-avatar-size"
                    ringColorToken="--mw-component-dialog-background"
                  />
                {:else if destination.people[0]}
                  <Avatar user={destination.people[0]} sizeToken="--mw-component-people-picker-avatar-size" />
                {/if}
                <span class="flex min-w-0 flex-1 flex-col">
                  <span class="truncate type-member-name text-(--mw-component-people-picker-name-text)">{destinationLabel(destination)}</span>
                  {#if refusal}
                    <span class="flex items-center gap-2xs type-body-sm text-(--mw-component-dialog-error-text)">
                      <Icon name="warning-circle" class="size-(--mw-component-message-status-icon-size)" />
                      <span class="min-w-0 first-letter:uppercase">{refusal}</span>
                    </span>
                  {:else}
                    <span class="truncate type-body-sm text-(--mw-component-people-picker-username-text)">{destination.where}</span>
                  {/if}
                </span>
                <!-- Native, so Space, the screen reader's "checked" and forced colours come free. -->
                <input
                  type="checkbox"
                  checked={isChecked}
                  disabled={isFull || isBusy}
                  aria-describedby={isFull ? `${id}-room` : undefined}
                  class="size-(--mw-component-people-picker-checkbox-size) shrink-0 accent-(--mw-component-people-picker-checkbox-color)"
                  onchange={(event) => togglePick(destination, event.currentTarget.checked)}
                />
              </label>
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    <!-- The forward as it will arrive: the same bar and mark as in the list, never the original's
         author (the forwarder sends it). -->
    <section aria-label={strings.forward.preview} class="flex flex-col gap-2xs border-l-(length:--mw-component-message-quote-bar-width) border-(--mw-component-message-quote-bar-color) pl-sm">
      <p class="flex items-center gap-2xs type-message-timestamp text-text-muted">
        <Icon name="arrow-bend-up-right" class="size-(--mw-component-message-status-icon-size)" />
        {strings.forward.forwarded}
      </p>
      {#if message.content.trim() !== ""}
        <p
          class="max-h-(--mw-component-dialog-preview-body-max-height) overflow-hidden type-message break-words whitespace-pre-wrap text-(--mw-component-message-body-text)"
        >
          {messageAsPlainText(message.content, client.knownRoles)}
        </p>
      {/if}
      {#if attachmentCount > 0}
        <p class="flex items-center gap-2xs type-body text-text-secondary">
          <Icon name="file" class="size-(--mw-component-message-status-icon-size)" />
          {strings.messages.attachmentCount(attachmentCount)}
        </p>
      {/if}
    </section>

    <TextField
      id="{id}-note"
      label={strings.forward.note}
      bind:value={note}
      helper={strings.forward.noteHelper}
      maxlength={MAXIMUM_MESSAGE_LENGTH}
      autocomplete="off"
      readonly={isBusy}
    />
  {/if}
  <div role="status" class="contents">
    {#if partialSuccess}
      <p class="flex gap-(--mw-component-dialog-error-gap) type-body text-text-secondary">
        <Icon name="check" class="size-(--mw-component-dialog-error-icon-size)" />
        <span>{partialSuccess}</span>
      </p>
    {/if}
  </div>
</ModalDialog>
