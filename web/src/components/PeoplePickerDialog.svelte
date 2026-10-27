<script lang="ts">
  import { strings } from "../i18n";
  import { ApiError } from "../lib/api";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import {
    type DirectMessageChannel,
    MAXIMUM_GROUP_CONVERSATION_NAME_LENGTH,
    type PublicUser,
    type Snowflake,
  } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import { displayName } from "./format";
  import Icon from "./Icon.svelte";
  import { conversationDisplayName, friendsMatching, participantCount, pickerRoom } from "./group-conversations";
  import ModalDialog from "./ModalDialog.svelte";
  import TextField from "./TextField.svelte";

  // "New message" and "Add friends" (DESIGN.md §5.6, ADR-0026): your friends with a search, the
  // ones picked as chips in the field, how many more fit, then one action. One friend opens (or
  // finds) your conversation with them; several make a new group, with an optional name. In a
  // group, the friends already in it are left out and the picked ones are added.
  interface Props {
    client: MellowClient;
    /** The group to add to; `null` starts a conversation. */
    group: DirectMessageChannel | null;
    onclose: () => void;
    /** "Add friend" from the empty state: the Friends space's Add friend tab. */
    onaddfriend: () => void;
    /** After people joined the group: their names, for the shell's live region. */
    onadded: (names: string[]) => void;
    fallbackFocus: () => void;
  }

  let { client, group, onclose, onaddfriend, onadded, fallbackFocus }: Props = $props();

  const id = $props.id();
  let query = $state("");
  let groupName = $state("");
  let pickedIds = $state<Snowflake[]>([]);
  let isBusy = $state(false);
  let errorMessage = $state("");
  let searchField = $state<HTMLInputElement | null>(null);
  // Plain: read by the dialog when it closes, after this component stopped updating.
  let hasSucceeded = false;

  const currentUserId = $derived(client.user?.id ?? null);
  const friends = $derived(client.relationshipsOfKind("friend").map((relationship) => relationship.user));
  const alreadyInIds = $derived(new Set(group?.recipients.map((recipient) => recipient.id) ?? []));
  const candidates = $derived(friends.filter((friend) => !alreadyInIds.has(friend.id)));
  // Someone who stopped being a friend, or joined meanwhile, leaves the selection by themselves.
  const picked = $derived(
    pickedIds.flatMap((userId) => candidates.find((candidate) => candidate.id === userId) ?? []),
  );
  const shown = $derived(friendsMatching(friends, query, alreadyInIds));
  const room = $derived(pickerRoom(group ? participantCount(group, currentUserId) : 1, picked.length));
  const isCreatingGroup = $derived(group === null && picked.length >= 2);

  const title = $derived(
    group ? strings.groups.addTitle(conversationDisplayName(group, currentUserId)) : strings.groups.newMessage,
  );
  const submitLabel = $derived.by(() => {
    if (group) {
      return isBusy ? strings.groups.adding : strings.groups.addToGroup;
    }
    if (isCreatingGroup) {
      return isBusy ? strings.groups.creating : strings.groups.createGroup;
    }
    return isBusy ? strings.members.opening : strings.groups.message;
  });

  function isPicked(userId: Snowflake): boolean {
    return picked.some((person) => person.id === userId);
  }

  function togglePick(person: PublicUser, isChecked: boolean) {
    pickedIds = isChecked ? [...pickedIds, person.id] : pickedIds.filter((userId) => userId !== person.id);
  }

  /**
   * Enter after a search picks the first friend it found and clears it, so names can be typed one
   * after the other; with nothing typed, Enter does the dialog's action. Backspace in an empty
   * search takes back the last friend picked, as in Discord's field.
   */
  function pickFromSearch(event: KeyboardEvent) {
    if (event.key === "Enter" && query.trim() !== "") {
      event.preventDefault();
      const first = shown.find((person) => !isPicked(person.id));
      if (first && room > 0) {
        togglePick(first, true);
        query = "";
      }
      return;
    }
    const last = picked.at(-1);
    if (event.key === "Backspace" && query === "" && last) {
      event.preventDefault();
      togglePick(last, false);
    }
  }

  function unpick(person: PublicUser) {
    pickedIds = pickedIds.filter((userId) => userId !== person.id);
    // The chip pressed is gone: the field keeps the keyboard in the picker.
    searchField?.focus();
  }

  function errorSentence(error: unknown, summary: string, reasonByCode: Partial<Record<ApiError["code"], string>>) {
    const reason = error instanceof ApiError ? reasonByCode[error.code] : undefined;
    return reason ? strings.errors.withReason(summary, reason) : describeError(error, summary);
  }

  async function submit() {
    if (picked.length === 0) {
      return;
    }
    isBusy = true;
    errorMessage = "";
    try {
      if (group) {
        await addToGroup(group);
      } else {
        await startConversation();
      }
    } finally {
      isBusy = false;
    }
  }

  async function startConversation() {
    try {
      await client.startConversation(
        picked.map((person) => person.id),
        isCreatingGroup ? groupName : "",
      );
    } catch (error) {
      errorMessage = isCreatingGroup
        ? errorSentence(error, strings.groups.createFailed, { NOT_FOUND: strings.groups.cannotJoin })
        : describeError(error, strings.members.openConversationFailed);
      return;
    }
    hasSucceeded = true;
    onclose();
  }

  /** One request per person, in the order picked; on a refusal the rest stay picked. */
  async function addToGroup(target: DirectMessageChannel) {
    const added: string[] = [];
    for (const person of picked) {
      try {
        await client.addGroupParticipant(target.id, person.id);
        added.push(displayName(person));
      } catch (error) {
        const refusal = errorSentence(error, strings.groups.addFailed(displayName(person)), {
          NOT_FOUND: strings.groups.addNotFound,
          LIMIT_REACHED: strings.groups.addFull,
        });
        // The page behind the dialog is inert: who joined is said in the dialog's own alert.
        errorMessage = added.length > 0 ? `${strings.groups.added(added)} ${refusal}` : refusal;
        return;
      }
    }
    onadded(added);
    hasSucceeded = true;
    onclose();
  }
</script>

<ModalDialog
  {title}
  subtitle={group ? strings.groups.addSubtitle : strings.groups.pickerSubtitle}
  {errorMessage}
  {isBusy}
  submitLabel={candidates.length === 0 ? "" : submitLabel}
  isSubmitEnabled={picked.length > 0}
  shouldReturnFocusToOpener={() => !hasSucceeded}
  {onclose}
  onsubmit={() => void submit()}
  {fallbackFocus}
>
  {#if friends.length === 0}
    <!-- Nobody to pick yet: the way to get someone. -->
    <div class="flex flex-col items-start gap-sm">
      <p class="type-body-lg text-text-secondary">{strings.groups.noFriends}</p>
      <Button variant="primary" data-initial-focus="" onclick={onaddfriend}>{strings.groups.addFriend}</Button>
    </div>
  {:else if candidates.length === 0}
    <p class="type-body-lg text-text-secondary">{strings.groups.everyoneIsIn}</p>
  {:else}
    <div class="flex flex-col gap-(--mw-component-text-field-label-gap)">
      <label for="{id}-search" class="type-category text-(--mw-component-text-field-label-text)">
        {strings.groups.search}
      </label>
      <!-- Discord's field: the friends picked sit in it as chips, before what is typed. -->
      <div
        class="flex min-h-(--mw-component-text-field-height) flex-wrap items-center gap-(--mw-component-people-picker-chip-gap) rounded-(--mw-component-text-field-radius) border-(length:--mw-component-text-field-border-width) border-(--mw-component-text-field-border) bg-(--mw-component-text-field-background) px-(--mw-component-text-field-padding-inline) py-2xs focus-within:focus-ring"
      >
        {#if picked.length > 0}
          <ul aria-label={strings.groups.picked} class="contents">
            {#each picked as person (person.id)}
              <li class="contents">
                <button
                  type="button"
                  aria-label={strings.groups.unpick(displayName(person))}
                  class="flex h-(--mw-component-people-picker-chip-height) max-w-full pointer-coarse:h-(--mw-size-hit-web-min) items-center gap-2xs rounded-(--mw-component-people-picker-chip-radius) bg-(--mw-component-people-picker-chip-background) ps-(--mw-component-people-picker-chip-padding-inline) pe-2xs type-body text-(--mw-component-people-picker-chip-text) focus-visible:focus-ring"
                  onclick={() => unpick(person)}
                >
                  <Avatar user={person} sizeToken="--mw-component-people-picker-chip-avatar-size" />
                  <span class="truncate">{displayName(person)}</span>
                  <Icon name="x" class="size-(--mw-component-text-field-icon-size) text-(--mw-component-people-picker-chip-icon)" />
                </button>
              </li>
            {/each}
          </ul>
        {/if}
        <input
          bind:this={searchField}
          bind:value={query}
          id="{id}-search"
          type="search"
          autocomplete="off"
          data-initial-focus=""
          placeholder={strings.groups.searchPlaceholder}
          aria-describedby="{id}-room"
          class="h-(--mw-component-people-picker-chip-height) min-w-(--mw-component-people-picker-search-min-width) flex-1 bg-transparent type-body-lg text-(--mw-component-text-field-text) outline-none placeholder:text-(--mw-component-text-field-placeholder)"
          onkeydown={pickFromSearch}
        />
      </div>
      <p id="{id}-room" aria-live="polite" class="type-body text-(--mw-component-people-picker-counter-text)">
        {strings.groups.roomLeft(room)}
      </p>
    </div>

    <div
      id="{id}-list"
      class="-mx-(--mw-component-people-picker-row-padding-inline) max-h-(--mw-component-people-picker-list-max-height) overflow-y-auto [scrollbar-width:thin]"
    >
      {#if shown.length === 0}
        <p class="px-(--mw-component-people-picker-row-padding-inline) type-body text-text-muted">
          {strings.groups.noMatch(query.trim())}
        </p>
      {:else}
        <ul>
          {#each shown as person (person.id)}
            {@const isChecked = isPicked(person.id)}
            {@const isFull = room === 0 && !isChecked}
            <li class="mt-(--mw-component-people-picker-row-gap)">
              <label
                class={[
                  "flex min-h-(--mw-component-people-picker-row-height) items-center gap-sm rounded-(--mw-component-people-picker-row-radius) px-(--mw-component-people-picker-row-padding-inline) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard)",
                  isFull ? "opacity-(--mw-component-button-disabled-opacity)" : "cursor-pointer hover:bg-(--mw-component-people-picker-row-background-hover)",
                ]}
              >
                <Avatar user={person} sizeToken="--mw-component-people-picker-avatar-size" />
                <span class="flex min-w-0 flex-1 flex-col">
                  <span class="truncate type-member-name text-(--mw-component-people-picker-name-text)">{displayName(person)}</span>
                  <span class="truncate type-body-sm text-(--mw-component-people-picker-username-text)">{person.username}</span>
                </span>
                <!-- Native, so Space, the screen reader's "checked" and forced colours come free. -->
                <input
                  type="checkbox"
                  checked={isChecked}
                  disabled={isFull}
                  aria-describedby={isFull ? `${id}-room` : undefined}
                  class="size-(--mw-component-people-picker-checkbox-size) shrink-0 accent-(--mw-component-people-picker-checkbox-color)"
                  onchange={(event) => togglePick(person, event.currentTarget.checked)}
                />
              </label>
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    {#if isCreatingGroup}
      <TextField
        id="{id}-name"
        label={strings.groups.groupName}
        bind:value={groupName}
        helper={strings.groups.groupNameHelper}
        placeholder={strings.groups.participantNames(picked.map(displayName))}
        maxlength={MAXIMUM_GROUP_CONVERSATION_NAME_LENGTH}
        autocomplete="off"
      />
    {/if}
  {/if}
</ModalDialog>
