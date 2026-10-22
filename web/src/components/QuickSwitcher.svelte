<script lang="ts">
  import { onMount, tick } from "svelte";

  import { type MellowClient, describeError } from "../lib/client.svelte";
  import { Permission, hasPermission } from "../lib/protocol";
  import Icon from "./Icon.svelte";
  import { type QuickSwitcherItem, quickSwitcherResults } from "./quick-switcher";

  // The quick switcher (DESIGN.md §5.10): one field, results grouped by kind, arrows and Enter.
  // The field is a combobox owning a listbox; the active option is announced through
  // aria-activedescendant, so focus never leaves the field. A native modal <dialog>.
  interface Props {
    client: MellowClient;
    onclose: () => void;
    /** After a result was opened: the conversation takes focus instead of the opener. */
    onnavigated: () => void;
  }

  let { client, onclose, onnavigated }: Props = $props();

  const id = $props.id();
  let dialog: HTMLDialogElement;
  let input: HTMLInputElement;
  let query = $state("");
  let activeIndex = $state(0);
  let errorMessage = $state("");
  let hasNavigated = false;

  const groups = $derived(
    quickSwitcherResults(query, {
      guilds: client.guilds,
      conversations: client.conversations,
      knownPeople: [...client.membersByGuild.values()].flat().map((member) => member.user),
      currentUserId: client.user?.id ?? null,
      hasUnread: (channelId) => client.hasUnread(channelId),
      canSendIn: (channelId) => {
        const permissions = client.myChannelPermissions(channelId);
        return permissions === null || hasPermission(permissions, Permission.SEND_MESSAGES);
      },
    }),
  );
  const items = $derived(groups.flatMap((group) => group.items));
  // Each option's position in the whole list, computed once per result set.
  const numberedGroups = $derived.by(() => {
    let offset = 0;
    return groups.map((group) => {
      const numbered = group.items.map((item, position) => ({ item, index: offset + position }));
      offset += group.items.length;
      return { label: group.label, items: numbered };
    });
  });
  const activeOptionIndex = $derived(Math.min(activeIndex, items.length - 1));
  const activeItem = $derived(items[activeOptionIndex] ?? null);

  onMount(() => {
    const opener =
      document.activeElement instanceof HTMLElement && document.activeElement !== document.body
        ? document.activeElement
        : null;
    dialog.showModal();
    input.focus();
    return () => {
      dialog.close();
      if (hasNavigated) {
        void tick().then(onnavigated);
      } else if (opener?.isConnected) {
        opener.focus();
      }
    };
  });

  async function open(item: QuickSwitcherItem) {
    const target = item.target;
    errorMessage = "";
    try {
      if (target.kind === "channel") {
        client.selectGuild(target.guildId, target.channelId);
      } else if (target.kind === "server") {
        client.selectGuild(target.guildId, null);
      } else if (target.conversationId) {
        client.selectGuild(null, target.conversationId);
      } else {
        await client.openDirectMessage(target.userId);
      }
    } catch (error) {
      errorMessage = describeError(error, `Couldn't open the conversation with ${item.label}`);
      return;
    }
    hasNavigated = true;
    onclose();
  }

  function handleKeydown(event: KeyboardEvent) {
    if (items.length === 0) {
      return;
    }
    const index = Math.min(activeIndex, items.length - 1);
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      activeIndex = (index + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
      void tick().then(() => document.getElementById(`${id}-option-${activeIndex}`)?.scrollIntoView({ block: "nearest" }));
    } else if (event.key === "Enter" && activeItem) {
      event.preventDefault();
      void open(activeItem);
    }
  }
</script>

<dialog
  bind:this={dialog}
  aria-label="Go to"
  class="quick-switcher mx-auto mt-[calc(var(--mw-component-quick-switcher-offset-top)*100vh)] w-[min(var(--mw-component-quick-switcher-width),calc(100vw_-_2*var(--mw-component-dialog-margin)))] max-w-none rounded-(--mw-component-dialog-radius) bg-(--mw-component-dialog-background) p-md text-(--mw-component-dialog-body-text) shadow-high backdrop:bg-(--mw-component-dialog-scrim) dark:[outline:1px_solid_var(--mw-color-border-divider)]"
  oncancel={(event) => {
    event.preventDefault();
    onclose();
  }}
  onclick={(event) => {
    if (event.target === dialog) {
      onclose();
    }
  }}
>
  <input
    bind:this={input}
    bind:value={query}
    role="combobox"
    aria-expanded="true"
    aria-controls="{id}-results"
    aria-autocomplete="list"
    aria-activedescendant={activeItem ? `${id}-option-${activeOptionIndex}` : undefined}
    aria-label="Where would you like to go?"
    placeholder="Where would you like to go?"
    autocomplete="off"
    spellcheck="false"
    class="h-(--mw-component-quick-switcher-input-height) w-full rounded-md bg-surface-input px-md type-heading-lg text-text-heading outline-none placeholder:text-text-muted focus-visible:focus-ring"
    oninput={() => (activeIndex = 0)}
    onkeydown={handleKeydown}
  />
  <!-- A listbox of groups of options, each group named by its heading; it scrolls past a share of
       the window height. -->
  <div id="{id}-results" role="listbox" aria-label="Results" class="mt-sm max-h-[calc(var(--mw-component-quick-switcher-results-max-height)*100vh)] overflow-y-auto">
    {#each numberedGroups as group, groupIndex (group.label)}
      <div role="group" aria-labelledby="{id}-group-{groupIndex}">
        <div id="{id}-group-{groupIndex}" role="presentation" class="px-xs pt-xs pb-2xs type-category text-text-muted">
          {group.label}
        </div>
        {#each group.items as { item, index } (item.key)}
          {@const isActive = index === activeOptionIndex}
          <!-- Pointer only: the keyboard goes through the field's arrows and Enter. -->
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            id="{id}-option-{index}"
            role="option"
            tabindex="-1"
            aria-selected={isActive}
            class={[
              "flex h-(--mw-component-quick-switcher-option-height) cursor-pointer items-center gap-xs rounded-sm px-xs pointer-coarse:h-(--mw-size-hit-web-min)",
              isActive && "bg-state-selected text-text-heading",
            ]}
            onclick={() => void open(item)}
            onpointermove={() => (activeIndex = index)}
          >
            <Icon name={item.icon} class="size-(--mw-component-channel-row-icon-size) text-text-muted" />
            <span class="min-w-0 truncate">{item.label}</span>
            {#if item.hasUnread}
              <span
                aria-hidden="true"
                class="size-(--mw-component-channel-row-unread-pill-width) shrink-0 rounded-full bg-unread-pill"
              ></span>
              <span class="sr-only">, unread</span>
            {/if}
            <span class="ms-auto shrink-0 type-body-sm text-text-muted">{item.where}</span>
          </div>
        {/each}
      </div>
    {/each}
  </div>
  <!-- Outside the listbox, and announced: the count, or why there is nothing to choose. -->
  <div role="status" class="contents">
    {#if items.length === 0}
      <p class="px-xs pt-xs pb-2xs type-body text-text-muted">
        No results for “{query.trim()}”. Check the spelling or search another server.
      </p>
    {:else}
      <p class="sr-only">{items.length} {items.length === 1 ? "result" : "results"}</p>
    {/if}
  </div>
  <div role="alert" class="contents">
    {#if errorMessage}
      <p class="mt-sm type-body text-(--mw-component-dialog-error-text)">{errorMessage}</p>
    {/if}
  </div>
  <p class="mt-sm type-body-sm text-text-muted">
    Arrows to choose, Enter to open, Esc to close. Prefixes: @ people, # channels, * servers.
  </p>
</dialog>

<style>
  .quick-switcher[open] {
    /* The settle duration, which reduced motion sets to 0 ms (the dialog token is not). */
    animation: switcher-in var(--mw-motion-duration-settle) var(--mw-motion-easing-settle) both;
  }
  .quick-switcher[open]::backdrop {
    animation: mw-fade-in var(--mw-motion-duration-base) var(--mw-motion-easing-standard) both;
  }
  @keyframes switcher-in {
    from {
      opacity: 0;
      transform: scale(var(--mw-component-dialog-enter-scale));
    }
  }
</style>
