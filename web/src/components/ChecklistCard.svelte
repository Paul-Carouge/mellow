<script lang="ts">
  import { tick } from "svelte";

  import { strings } from "../i18n";
  import { checklistProgress, progressPercent } from "../lib/checklists";
  import { type Checklist, MAXIMUM_CHECKLIST_ITEMS, MAXIMUM_CHECKLIST_ITEM_LENGTH, type Snowflake } from "../lib/protocol";
  import { fullTimestampLabel } from "./format";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import ProgressMeter from "./ProgressMeter.svelte";
  import { checklistItemText } from "./tracker-form";

  // A checklist in a message (docs/design/web-checklists-and-progress.md, ADR-0047): the poll
  // card's frame and tokens, a "3/5" level under the title, then one checkbox row per item with
  // who ticked it and when. Its author and moderators switch it to editing: each item becomes a
  // field, with a Remove button, and a field at the end adds one.
  interface Props {
    checklist: Checklist;
    /** Still in the outbox: shown as it will arrive, nothing to press yet. */
    isPending?: boolean;
    canTick: boolean;
    canEdit: boolean;
    nameOfUser: (userId: Snowflake) => string;
    ontoggle?: (itemId: number) => void;
    /** Each resolves `true` once the instance took the change. */
    onadd?: (text: string) => Promise<boolean>;
    onrename?: (itemId: number, text: string) => Promise<boolean>;
    onremove?: (itemId: number) => void;
  }

  let { checklist, isPending = false, canTick, canEdit, nameOfUser, ontoggle, onadd, onrename, onremove }: Props =
    $props();

  const id = $props.id();
  const progress = $derived(checklistProgress(checklist));
  const canTickNow = $derived(canTick && !isPending);
  let isEditing = $state(false);
  let newItem = $state("");
  let newItemField = $state<HTMLInputElement | null>(null);

  async function startEditing() {
    isEditing = true;
    await tick();
    newItemField?.focus();
  }

  async function add() {
    const text = checklistItemText(newItem);
    if (text !== null && (await onadd?.(text))) {
      newItem = "";
    }
  }

  /** On Enter or leaving the field: a blank or unchanged text keeps the item as it is. */
  function rename(itemId: number, field: HTMLInputElement, current: string) {
    const text = checklistItemText(field.value);
    if (text === null || text === current) {
      field.value = current;
      return;
    }
    void onrename?.(itemId, text).then((isRenamed) => {
      if (!isRenamed) {
        field.value = current;
      }
    });
  }
</script>

<section
  aria-labelledby="{id}-title"
  class="mt-3xs flex w-full max-w-(--mw-component-poll-max-width) flex-col gap-(--mw-component-poll-gap) rounded-(--mw-component-poll-radius) border-(length:--mw-component-poll-border-width) border-(--mw-component-poll-border) bg-(--mw-component-poll-background) p-(--mw-component-poll-padding)"
>
  <div class="flex flex-col gap-(--mw-component-poll-header-gap)">
    <h4 id="{id}-title" class="flex items-center gap-2xs type-heading-md break-words text-(--mw-component-poll-question-text)">
      <Icon name="list-checks" class="size-(--mw-component-poll-mark-size) shrink-0 text-(--mw-component-poll-hint-text)" />
      <span class="min-w-0">{checklist.title}</span>
    </h4>
    <ProgressMeter
      labelledBy="{id}-title"
      value={progress.done}
      maximum={progress.total}
      percent={progressPercent(progress.done, progress.total)}
      figure={strings.checklists.done(progress.done, progress.total)}
      spoken={strings.checklists.doneSpoken(progress.done, progress.total)}
    />
  </div>

  <ul class="flex flex-col gap-(--mw-component-poll-answer-gap)" aria-labelledby="{id}-title">
    {#each checklist.items as item (item.id)}
      {@const isChecked = item.checked !== undefined}
      <li
        class={[
          "flex min-h-(--mw-size-hit-web-min) items-stretch overflow-hidden rounded-(--mw-component-poll-answer-radius) border-(length:--mw-component-poll-answer-border-width) bg-(--mw-component-poll-answer-background) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard)",
          isChecked ? "border-(--mw-component-poll-answer-border-mine)" : "border-(--mw-component-poll-answer-border)",
        ]}
      >
        {#if isEditing}
          <input
            value={item.text}
            aria-label={strings.checklists.rename(item.text)}
            maxlength={MAXIMUM_CHECKLIST_ITEM_LENGTH}
            class="min-w-0 flex-1 bg-transparent px-(--mw-component-poll-answer-padding-inline) type-message text-(--mw-component-poll-answer-text) outline-none focus-visible:focus-ring-inset"
            onkeydown={(event) => {
              if (event.key === "Enter" && !event.isComposing) {
                event.preventDefault();
                rename(item.id, event.currentTarget, item.text);
              }
            }}
            onblur={(event) => rename(item.id, event.currentTarget, item.text)}
          />
          {#if checklist.items.length > 1}
            <IconButton
              icon="trash"
              label={strings.checklists.remove(item.text)}
              class="size-(--mw-size-hit-web-min) shrink-0 text-(--mw-component-poll-hint-text) hover:bg-state-hover hover:text-(--mw-component-action-bar-icon-danger-hover) focus-visible:focus-ring-inset"
              iconClass="size-(--mw-component-poll-mark-glyph-size)"
              onclick={() => onremove?.(item.id)}
            />
          {/if}
        {:else}
          <button
            type="button"
            role="checkbox"
            aria-checked={isChecked}
            aria-disabled={!canTickNow || undefined}
            aria-describedby={item.checked ? `${id}-by-${item.id}` : undefined}
            class={[
              "flex min-w-0 flex-1 items-center gap-(--mw-component-poll-answer-inner-gap) px-(--mw-component-poll-answer-padding-inline) py-(--mw-component-poll-answer-padding-block) text-start focus-visible:focus-ring-inset",
              canTickNow ? "cursor-pointer hover:bg-state-hover" : "cursor-default",
            ]}
            onclick={() => canTickNow && ontoggle?.(item.id)}
          >
            <span
              aria-hidden="true"
              class={[
                "grid size-(--mw-component-poll-mark-size) shrink-0 place-items-center rounded-(--mw-component-poll-mark-radius-checkbox)",
                isChecked
                  ? "bg-(--mw-component-poll-mark-checked) text-(--mw-component-poll-mark-glyph)"
                  : "border-(length:--mw-component-poll-mark-border-width) border-(--mw-component-poll-mark-border)",
              ]}
            >
              {#if isChecked}
                <Icon name="check" class="size-(--mw-component-poll-mark-glyph-size)" />
              {/if}
            </span>
            <span class="flex min-w-0 flex-col">
              <span class={["type-message break-words text-(--mw-component-poll-answer-text)", isChecked && "line-through"]}
                >{item.text}</span
              >
              {#if item.checked}
                {@const at = item.checked.at_unix_milliseconds}
                <span id="{id}-by-{item.id}" class="type-body-sm text-(--mw-component-poll-count-text)">
                  <time datetime={new Date(at).toISOString()}
                    >{strings.checklists.tickedBy(nameOfUser(item.checked.user_id), fullTimestampLabel(at))}</time
                  >
                </span>
              {/if}
            </span>
          </button>
        {/if}
      </li>
    {/each}
  </ul>

  {#if isEditing}
    <form
      class="flex items-center gap-2xs"
      onsubmit={(event) => {
        event.preventDefault();
        void add();
      }}
    >
      <input
        bind:this={newItemField}
        bind:value={newItem}
        aria-label={strings.checklists.newItem}
        maxlength={MAXIMUM_CHECKLIST_ITEM_LENGTH}
        placeholder={strings.checklists.newItem}
        disabled={checklist.items.length >= MAXIMUM_CHECKLIST_ITEMS}
        autocomplete="off"
        class="h-(--mw-size-hit-web-min) min-w-0 flex-1 rounded-(--mw-component-poll-answer-radius) border-(length:--mw-component-poll-answer-border-width) border-(--mw-component-poll-answer-border) bg-(--mw-component-poll-answer-background) px-(--mw-component-poll-answer-padding-inline) type-message text-(--mw-component-poll-answer-text) outline-none placeholder:text-(--mw-component-poll-hint-text) focus-visible:focus-ring"
      />
      <button
        type="submit"
        disabled={checklist.items.length >= MAXIMUM_CHECKLIST_ITEMS}
        class="inline-flex min-h-(--mw-size-hit-web-min) items-center rounded-xs px-2xs type-body-sm font-semibold text-text-link hover:underline focus-visible:focus-ring disabled:text-text-muted disabled:no-underline"
        >{strings.checklists.addItem}</button
      >
    </form>
    <p class="type-body-sm text-(--mw-component-poll-footer-text)" aria-live="polite">
      {checklist.items.length > 1
        ? strings.checklists.itemsLeft(MAXIMUM_CHECKLIST_ITEMS - checklist.items.length)
        : strings.checklists.lastItemStays}
    </p>
  {/if}

  {#if canEdit && !isPending}
    <div class="flex">
      <button
        type="button"
        class="inline-flex min-h-(--mw-size-hit-desktop-min) items-center gap-3xs rounded-xs type-body-sm font-semibold text-text-link hover:underline focus-visible:focus-ring pointer-coarse:min-h-(--mw-size-hit-web-min)"
        onclick={() => (isEditing ? (isEditing = false) : void startEditing())}
      >
        {#if !isEditing}<Icon name="pencil-simple" class="size-(--mw-component-poll-mark-glyph-size)" />{/if}
        {isEditing ? strings.checklists.doneEditing : strings.checklists.edit}
      </button>
    </div>
  {/if}
</section>
