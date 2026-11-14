<script lang="ts">
  import { onMount, tick } from "svelte";
  import { MediaQuery } from "svelte/reactivity";

  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { PublicUser } from "../lib/protocol";
  import Button from "./Button.svelte";
  import { buttonVariantClasses } from "./button-variants";
  import { dayDividerLabel, displayName } from "./format";
  import IconButton from "./IconButton.svelte";
  import PersonContextMenu from "./PersonContextMenu.svelte";
  import ProfileCard from "./ProfileCard.svelte";
  import ProfileDetails from "./ProfileDetails.svelte";
  import { profileFactsOf } from "./profile-facts";
  import { type ProfileModalRequest, readRecentUserProfile, returnFocusWithoutHoverCard } from "./profile-surfaces.svelte";
  import RoleChips from "./RoleChips.svelte";
  import { minimumWidthQueryForPanes } from "./tokens";
  import { createUserProfileLoader } from "./user-profile-loader.svelte";

  // The profile modal (DESIGN.md §5.7.1): the person's own card on the left, as they framed it in
  // Profile settings, beside what you share with them; a bottom sheet on phones. A native modal
  // <dialog>: the page behind is inert, so focus stays inside, and it goes back to the name or row
  // that opened it.
  interface Props {
    client: MellowClient;
    request: ProfileModalRequest;
    onclose: () => void;
    /** After "Send message" opened the conversation: the shell focuses its composer. */
    onopenedconversation: () => void;
    /** Your own profile's "Edit profile": User settings on Profile. */
    oneditprofile: () => void;
  }

  let { client, request, onclose, onopenedconversation, oneditprofile }: Props = $props();

  const id = $props.id();
  let dialog: HTMLDialogElement;
  let moreButton = $state<HTMLButtonElement | null>(null);
  let personMenu = $state<PersonContextMenu | null>(null);

  // The card and the details side by side need the modal's width and its margins; below, a sheet.
  const isWide = new MediaQuery(
    minimumWidthQueryForPanes(["component-profile-modal-width", "component-profile-modal-margin", "component-profile-modal-margin"]),
  );

  const userId = $derived(request.user.id);
  const isSelf = $derived(userId === client.user?.id);
  const guild = $derived(client.selectedGuild);
  const facts = $derived(profileFactsOf(userId, guild, guild ? client.membersByGuild.get(guild.id) : undefined));
  const relationship = $derived(client.relationshipByUserId.get(userId) ?? null);
  // The freshest copy we hold: the member list and relationships follow USER_UPDATE.
  const user = $derived.by((): PublicUser => {
    const me = client.user;
    if (isSelf && me) {
      return { id: me.id, username: me.username, display_name: me.display_name, avatar: me.avatar, banner: me.banner };
    }
    return facts.member?.user ?? relationship?.user ?? request.user;
  });
  const person = $derived(displayName(user));
  const presence = $derived(client.presenceOf(userId));
  const customStatus = $derived(client.customStatusOf(userId));

  const details = createUserProfileLoader((profileUserId) =>
    readRecentUserProfile((readUserId) => client.loadUserProfile(readUserId), profileUserId, client.user?.id ?? null),
  );
  $effect(() => {
    details.show(userId);
  });
  const loadedDetails = $derived(details.current.kind === "loaded" && details.current.userId === userId ? details.current.profile : null);
  const hasNoDetails = $derived(
    loadedDetails !== null && loadedDetails.about_me.trim() === "" && loadedDetails.links.length === 0,
  );

  let isOpening = $state(false);
  let isChangingRelationship = $state(false);
  let errorMessage = $state("");
  let announcement = $state("");

  async function sendMessage() {
    isOpening = true;
    errorMessage = "";
    try {
      await client.openDirectMessage(userId);
      onopenedconversation();
    } catch (error) {
      errorMessage = describeError(error, strings.members.openConversationFailed);
    } finally {
      isOpening = false;
    }
  }

  type ProfileAction = "send-message" | "unblock" | "more";

  /** The button pressed is replaced once the relationship changes: focus goes to `next`. */
  async function focusAfterChange(next: ProfileAction) {
    await tick();
    if (dialog.contains(document.activeElement) && document.activeElement !== dialog) {
      return;
    }
    (dialog.querySelector<HTMLElement>(`[data-profile-action="${next}"]`) ?? moreButton)?.focus();
  }

  async function changeRelationship(change: () => Promise<unknown>, done: string, failed: string, next: ProfileAction) {
    isChangingRelationship = true;
    errorMessage = "";
    try {
      await change();
      announcement = done;
      await focusAfterChange(next);
    } catch (error) {
      errorMessage = describeError(error, failed);
    } finally {
      isChangingRelationship = false;
    }
  }

  function openMoreMenu(button: HTMLButtonElement) {
    const bounds = button.getBoundingClientRect();
    personMenu?.openFor(user, { kind: "anchor", bounds, align: "end" }, button);
  }

  onMount(() => {
    const opener = request.opener;
    // Opened from the keyboard, focus goes back with its ring; after a click, without it.
    const wasOpenedFromKeyboard = opener?.matches(":focus-visible") ?? false;
    dialog.showModal();
    dialog.querySelector<HTMLElement>("[data-initial-focus]")?.focus();
    return () => {
      dialog.close();
      if (opener?.isConnected) {
        // `focusVisible` (HTML FocusOptions) is not in TypeScript's DOM types yet.
        const options: FocusOptions & { focusVisible: boolean } = { focusVisible: wasOpenedFromKeyboard };
        returnFocusWithoutHoverCard(opener, options);
      }
    };
  });

  const moreButtonClass = `size-(--mw-component-button-height-large) rounded-(--mw-component-button-radius) ${buttonVariantClasses("secondary", false)}`;
  const sectionClass = "flex flex-col gap-(--mw-component-profile-modal-section-label-gap)";
  const sectionLabelClass = "type-category text-(--mw-component-profile-popover-section-label-text)";
</script>

{#snippet dateFigure(label: string, unixMilliseconds: number)}
  <div class="flex flex-col">
    <dt class="type-body-sm text-(--mw-component-profile-modal-date-label-text)">{label}</dt>
    <dd class="type-body text-(--mw-component-profile-modal-date-text)">
      <time datetime={new Date(unixMilliseconds).toISOString()}>{dayDividerLabel(unixMilliseconds)}</time>
    </dd>
  </div>
{/snippet}

<dialog
  bind:this={dialog}
  aria-labelledby="{id}-name"
  class={[
    "profile-modal max-w-none overflow-hidden p-0 text-(--mw-component-profile-popover-about-text) shadow-(--mw-component-profile-modal-shadow)",
    isWide.current
      ? "profile-modal-centred m-auto w-(--mw-component-profile-modal-width) rounded-(--mw-component-profile-modal-radius) bg-(--mw-component-profile-modal-details-background) dark:[outline:var(--mw-component-profile-modal-border-width)_solid_var(--mw-component-profile-modal-border)]"
      : "profile-modal-sheet inset-x-0 top-auto bottom-0 m-0 h-[calc(100dvh_-_var(--mw-component-profile-modal-sheet-top-gap))] max-h-none w-full rounded-t-(--mw-component-profile-modal-sheet-radius) bg-(--mw-component-profile-modal-details-background)",
  ]}
  oncancel={(event) => {
    event.preventDefault();
    onclose();
  }}
  onclick={(event) => {
    // A click on the ::backdrop targets the dialog itself; the card fills the dialog's box.
    if (event.target === dialog) {
      onclose();
    }
  }}
>
  <div
    class={[
      "flex",
      isWide.current
        ? "max-h-[calc(100dvh_-_2*var(--mw-component-profile-modal-margin))] min-h-[min(var(--mw-component-profile-modal-min-height),calc(100dvh_-_2*var(--mw-component-profile-modal-margin)))]"
        : "h-full flex-col overflow-y-auto overscroll-contain pb-[env(safe-area-inset-bottom)] [scrollbar-width:thin]",
    ]}
  >
    <div
      class={[
        "flex shrink-0 flex-col bg-(--mw-component-profile-modal-background)",
        isWide.current && "w-(--mw-component-profile-modal-identity-width) overflow-y-auto [scrollbar-width:thin]",
      ]}
    >
      <ProfileCard
        {user}
        nameElement="h2"
        nameId="{id}-name"
        {presence}
        roleIcon={facts.roleIcon}
        isBannerProportional={!isWide.current}
        surfaceToken="--mw-component-profile-modal-background"
      >
        <!-- The avatar already says it to screen readers. -->
        <p aria-hidden="true" class="type-body-sm text-(--mw-component-profile-modal-presence-text)">{strings.presence[presence]}</p>
        {#if customStatus}
          <p
            class="w-fit max-w-full rounded-(--mw-component-profile-modal-status-radius) bg-(--mw-component-profile-modal-status-background) px-(--mw-component-profile-modal-status-padding-inline) py-(--mw-component-profile-modal-status-padding-block) type-body break-words text-(--mw-component-profile-modal-status-text)"
          >
            <span class="sr-only">{strings.customStatus.field}, </span>{customStatus}
          </p>
        {/if}

        <div class="mt-xs flex flex-col gap-(--mw-component-profile-modal-action-gap)">
          <div class="flex gap-(--mw-component-profile-modal-action-gap) [&>button:first-child]:flex-1">
            {#if isSelf}
              <Button data-initial-focus onclick={oneditprofile}>{strings.profileCard.editProfile}</Button>
            {:else if relationship?.kind === "blocked"}
              <!-- No conversation while blocked: unblocking comes first. -->
              <Button
                data-initial-focus
                data-profile-action="unblock"
                isDisabled={isChangingRelationship}
                onclick={() =>
                  void changeRelationship(
                    () => client.removeRelationship(userId),
                    strings.friends.unblocked(person),
                    strings.friends.unblockFailed(person),
                    "send-message",
                  )}>{strings.friends.unblock}</Button
              >
            {:else}
              <Button
                data-initial-focus
                data-profile-action="send-message"
                variant="primary"
                isDisabled={isOpening}
                onclick={() => void sendMessage()}>{isOpening ? strings.members.opening : strings.members.sendMessage}</Button
              >
            {/if}
            <IconButton
              bind:element={moreButton}
              data-profile-action="more"
              aria-haspopup="menu"
              icon="dots-three"
              label={strings.profileCard.moreActionsFor(person)}
              class={moreButtonClass}
              iconClass="size-(--mw-component-chat-header-icon-size)"
              onclick={(event) => openMoreMenu(event.currentTarget)}
            />
          </div>
          {#if !isSelf && relationship?.kind !== "blocked" && relationship?.kind !== "friend"}
            {#if relationship === null}
              <Button
                isFullWidth
                isDisabled={isChangingRelationship}
                onclick={() =>
                  void changeRelationship(
                    () => client.sendFriendRequest({ username: user.username }),
                    strings.friends.sent(person),
                    strings.members.friendRequestFailed,
                    "more",
                  )}>{strings.members.addFriend}</Button
              >
            {:else if relationship.kind === "incoming_request"}
              <Button
                isFullWidth
                isDisabled={isChangingRelationship}
                onclick={() =>
                  void changeRelationship(
                    () => client.acceptFriendRequest(userId),
                    strings.friends.accepted(person),
                    strings.members.acceptFailed,
                    "more",
                  )}>{strings.members.acceptRequest}</Button
              >
            {:else if relationship.kind === "outgoing_request"}
              <Button isFullWidth isDisabled>{strings.members.requestSent}</Button>
            {/if}
          {/if}
        </div>
        <p class="sr-only" aria-live="polite">{announcement}</p>
        <div role="alert" class="contents">
          {#if errorMessage}
            <p class="type-body text-(--mw-component-profile-popover-error-text)">{errorMessage}</p>
          {/if}
        </div>
      </ProfileCard>
    </div>

    <div
      class={[
        "flex min-w-0 flex-1 flex-col gap-(--mw-component-profile-modal-section-gap) p-(--mw-component-profile-modal-details-padding)",
        isWide.current && "overflow-y-auto [scrollbar-width:thin]",
      ]}
    >
      {#if loadedDetails}
        <ProfileDetails
          aboutMe={loadedDetails.about_me}
          links={loadedDetails.links}
          id="{id}-details"
          headingElement="h3"
          {sectionClass}
        />
        {#if isSelf && hasNoDetails}
          <p class="type-body text-(--mw-component-profile-modal-date-label-text)">{strings.profileCard.noDetailsYet}</p>
        {/if}
      {:else if details.current.kind === "failed"}
        <p class="type-body-sm text-(--mw-component-profile-modal-date-label-text)">{strings.profileDetails.loadFailed}</p>
      {/if}

      {#if guild && facts.member && (facts.isOwner || facts.member.role_ids.length > 0)}
        <section aria-labelledby="{id}-roles" class={sectionClass}>
          <h3 id="{id}-roles" class={sectionLabelClass}>{strings.profileCard.roles}</h3>
          <RoleChips
            {guild}
            roleIds={facts.member.role_ids}
            isOwner={facts.isOwner}
            label={strings.profileCard.rolesOf(person)}
            areIconsShown
          />
        </section>
      {/if}

      <section aria-labelledby="{id}-since" class={sectionClass}>
        <h3 id="{id}-since" class={sectionLabelClass}>{strings.profileCard.memberSince}</h3>
        <dl class="flex flex-wrap gap-x-(--mw-component-profile-modal-dates-gap) gap-y-xs">
          {@render dateFigure(strings.profileCard.mellow, facts.joinedMellowAtUnixMilliseconds)}
          {#if guild && facts.joinedServerAtUnixMilliseconds !== null}
            {@render dateFigure(guild.name, facts.joinedServerAtUnixMilliseconds)}
          {/if}
        </dl>
      </section>
    </div>
  </div>

  <IconButton
    icon="x"
    label={strings.common.close}
    class={[
      "absolute top-(--mw-component-profile-modal-close-inset) right-(--mw-component-profile-modal-close-inset)",
      isWide.current
        ? "size-(--mw-component-profile-modal-close-size) text-(--mw-component-profile-modal-close-icon) hover:bg-(--mw-component-profile-modal-close-background-hover) hover:text-(--mw-component-profile-modal-close-icon-hover)"
        : "size-(--mw-size-hit-web-min) bg-(--mw-component-profile-modal-close-on-picture-background) text-(--mw-component-profile-modal-close-on-picture-icon)",
    ].join(" ")}
    iconClass="size-(--mw-component-user-panel-button-icon-size)"
    onclick={onclose}
  />

  <PersonContextMenu
    bind:this={personMenu}
    {client}
    isInProfile
    {onopenedconversation}
    fallbackFocus={() => moreButton?.focus()}
  />
</dialog>

<style>
  /* The page behind dims and blurs a little (DESIGN.md §5.7.1); the blur's radius never animates,
     only the scrim's opacity. Reduced motion or transparency, and browsers without
     backdrop-filter, keep the plain scrim of every dialog. */
  .profile-modal::backdrop {
    background-color: var(--mw-component-profile-modal-scrim);
    backdrop-filter: blur(var(--mw-component-profile-modal-scrim-blur));
    animation: mw-fade-in var(--mw-motion-duration-base) var(--mw-motion-easing-standard) both;
  }
  @media (prefers-reduced-motion: reduce), (prefers-reduced-transparency: reduce) {
    .profile-modal::backdrop {
      backdrop-filter: none;
    }
  }
  .profile-modal-centred[open] {
    animation: profile-modal-in var(--mw-motion-duration-settle) var(--mw-motion-easing-settle) both;
  }
  .profile-modal-sheet[open] {
    animation: profile-sheet-in var(--mw-motion-duration-slow) var(--mw-motion-easing-enter) both;
  }
  @keyframes profile-modal-in {
    from {
      opacity: 0;
      transform: scale(var(--mw-component-profile-modal-enter-scale));
    }
  }
  @keyframes profile-sheet-in {
    from {
      transform: translateY(100%);
    }
  }
</style>
