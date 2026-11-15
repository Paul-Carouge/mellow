<script lang="ts">
  import { tick } from "svelte";

  import { strings } from "../i18n";
  import { checklistProgress, progressPercent } from "../lib/checklists";
  import { type Checklist, MAXIMUM_CHECKLIST_ITEMS, MAXIMUM_CHECKLIST_ITEM_LENGTH, type Snowflake } from "../lib/protocol";
  import Button from "./Button.svelte";
  import { fullTimestampLabel, messageTimestampLabel } from "./format";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import ProgressMeter from "./ProgressMeter.svelte";
  import { lengthTokenInPixels } from "./tokens";
  import { checklistItemText } from "./tracker-form";

  // A checklist in a message (docs/design/web-checklists-and-progress.md, ADR-0047, redesigned
  // 2026-11-15): the poll's well, wider, a segmented level under the title (one segment per item),
  // then borderless rows that light up under the pointer, each a large box and its text, with who
  // ticked it and when. Its author and moderators switch it to editing: each item becomes a field,
  // with a Remove button, and a field at the end adds one.
  interface Props {
    checklist: Checklist;
    /** Still in the outbox: shown as it will arrive, nothing to press yet. */
    isPending?: boolean;
    canTick: boolean;
    /** Whether the reader may write here: says why the boxes are inert when they are. */
    canWrite?: boolean;
    canEdit: boolean;
    nameOfUser: (userId: Snowflake) => string;
    ontoggle?: (itemId: number) => void;
    /** Each resolves `true` once the instance took the change. */
    onadd?: (text: string) => Promise<boolean>;
    onrename?: (itemId: number, text: string) => Promise<boolean>;
    onremove?: (itemId: number) => Promise<boolean>;
  }

  let { checklist, isPending = false, canTick, canWrite = true, canEdit, nameOfUser, ontoggle, onadd, onrename, onremove }: Props =
    $props();

  const id = $props.id();
  // Phones: the card and its rows give their insets back to the text.
  let cardWidth: number | undefined = $state();
  const isCompact = $derived(cardWidth !== undefined && cardWidth < lengthTokenInPixels("component-checklist-compact-width"));
  const progress = $derived(checklistProgress(checklist));
  const isAllDone = $derived(progress.total > 0 && progress.done === progress.total);
  const canTickNow = $derived(canTick && !isPending);
  let isEditing = $state(false);
  let newItem = $state("");
  let newItemField = $state<HTMLInputElement | null>(null);
  let itemList = $state<HTMLUListElement | null>(null);

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

  /** Focus goes to the next item's field, else the previous one, else the new-item field, as in the dialog. */
  async function remove(itemId: number, index: number) {
    if (!(await onremove?.(itemId))) {
      return;
    }
    await tick();
    const fields = itemList?.querySelectorAll("input") ?? [];
    (fields[index] ?? fields[index - 1] ?? newItemField)?.focus();
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
  bind:clientWidth={cardWidth}
  aria-labelledby="{id}-title"
  class="mt-3xs flex w-full max-w-(--mw-component-checklist-max-width) flex-col gap-(--mw-component-checklist-gap) rounded-(--mw-component-poll-radius) border-(length:--mw-component-poll-border-width) border-(--mw-component-poll-border) bg-(--mw-component-poll-background) p-(--card-padding)"
  style:--card-padding="var(--mw-component-checklist-padding{isCompact ? '-compact' : ''})"
  style:--card-inset="var(--mw-component-checklist-row-padding-inline{isCompact ? '-compact' : ''})"
>
  <div class="flex flex-col gap-xs px-(--card-inset) pt-2xs">
    <h4 id="{id}-title" class="flex items-start gap-(--mw-component-checklist-header-gap) type-heading-md break-words text-(--mw-component-poll-question-text)">
      <Icon
        name="list-checks"
        class="mt-[calc((var(--mw-type-heading-md-font-size)*var(--mw-type-heading-md-line-height)-var(--mw-component-checklist-mark-size))/2)] size-(--mw-component-checklist-mark-size) shrink-0 text-(--mw-component-poll-hint-text)"
      />
      <span class="min-w-0">{checklist.title}</span>
    </h4>
    <ProgressMeter
      labelledBy="{id}-title"
      isSegmented
      value={progress.done}
      maximum={progress.total}
      percent={progressPercent(progress.done, progress.total)}
      figure={strings.checklists.done(progress.done, progress.total)}
      spoken={strings.checklists.doneSpoken(progress.done, progress.total)}
      figureOverride={isAllDone ? allDone : undefined}
    />
  </div>

  <ul bind:this={itemList} class="flex flex-col gap-(--mw-component-checklist-row-gap)" aria-labelledby="{id}-title">
    {#each checklist.items as item, index (item.id)}
      {@const isChecked = item.checked !== undefined}
      {#if isEditing}
        <li class="flex min-w-0 items-center gap-2xs">
          <input
            value={item.text}
            aria-label={strings.checklists.rename(item.text)}
            maxlength={MAXIMUM_CHECKLIST_ITEM_LENGTH}
            class="h-(--mw-component-text-field-height) min-w-0 flex-1 rounded-(--mw-component-checklist-row-radius) border-(length:--mw-component-text-field-border-width) border-(--mw-component-text-field-border) bg-(--mw-component-text-field-background) px-(--card-inset) type-message text-(--mw-component-checklist-item-text) outline-none focus-visible:focus-ring"
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
              class="size-(--mw-size-hit-web-min) text-(--mw-component-poll-hint-text) hover:bg-(--mw-component-checklist-row-background-hover) hover:text-(--mw-component-action-bar-icon-danger-hover) focus-visible:focus-ring"
              iconClass="size-(--mw-component-checklist-mark-glyph-size)"
              onclick={() => void remove(item.id, index)}
            />
          {/if}
        </li>
      {:else}
        <!-- The checkbox stretches over its row (its ::after), so a click on "who ticked it" ticks too,
             while that line stays outside the checkbox's name (WCAG 2.5.3). -->
        <li
          class={[
            "group/row relative flex min-h-(--mw-component-checklist-row-min-height) min-w-0 flex-col rounded-(--mw-component-checklist-row-radius) px-(--card-inset) py-(--mw-component-checklist-row-padding-block) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) has-focus-visible:focus-ring-inset",
            canTickNow && "hover:bg-(--mw-component-checklist-row-background-hover)",
          ]}
        >
          <button
            type="button"
            role="checkbox"
            aria-checked={isChecked}
            aria-disabled={!canTickNow || undefined}
            aria-labelledby="{id}-text-{item.id}"
            aria-describedby={item.checked ? `${id}-by-${item.id}` : undefined}
            class={[
              "flex min-w-0 items-start gap-(--mw-component-checklist-row-inner-gap) text-start outline-none after:absolute after:inset-0 after:rounded-(--mw-component-checklist-row-radius)",
              canTickNow ? "cursor-pointer" : "cursor-default",
            ]}
            onclick={() => canTickNow && ontoggle?.(item.id)}
          >
            <span
              aria-hidden="true"
              class={[
                "mark mt-[calc((var(--mw-type-message-font-size)*var(--mw-type-message-line-height)-var(--mw-component-checklist-mark-size))/2)] grid size-(--mw-component-checklist-mark-size) shrink-0 place-items-center rounded-(--mw-component-checklist-mark-radius) border-(length:--mw-component-checklist-mark-border-width) text-(--mw-component-checklist-mark-glyph)",
                isChecked && "is-checked border-(--mw-component-checklist-mark-checked) bg-(--mw-component-checklist-mark-checked)",
                !isChecked && canTickNow && "border-(--mw-component-checklist-mark-border) group-hover/row:border-(--mw-component-checklist-mark-border-hover)",
                !isChecked && !canTickNow && "border-(--mw-component-checklist-mark-border-inert)",
              ]}
            >
              <Icon name="check" class="glyph size-(--mw-component-checklist-mark-glyph-size)" />
            </span>
            <span
              id="{id}-text-{item.id}"
              class={[
                "min-w-0 flex-1 type-message break-words",
                isChecked
                  ? "text-(--mw-component-checklist-item-text-done) line-through decoration-(--mw-component-checklist-item-text-done)"
                  : "text-(--mw-component-checklist-item-text)",
              ]}>{item.text}</span
            >
          </button>
          {#if item.checked}
            {@const at = item.checked.at_unix_milliseconds}
            {@const name = nameOfUser(item.checked.user_id)}
            <span
              aria-hidden="true"
              class="ms-[calc(var(--mw-component-checklist-mark-size)+var(--mw-component-checklist-row-inner-gap))] type-body-sm text-(--mw-component-checklist-meta-text)"
            >
              <span class="font-medium text-(--mw-component-checklist-meta-name-text)">{name}</span>
              · <time datetime={new Date(at).toISOString()}>{messageTimestampLabel(at, new Date())}</time>
            </span>
            <span id="{id}-by-{item.id}" hidden>{strings.checklists.tickedBy(name, fullTimestampLabel(at))}</span>
          {/if}
        </li>
      {/if}
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
        class="h-(--mw-component-text-field-height) min-w-0 flex-1 rounded-(--mw-component-checklist-row-radius) border-(length:--mw-component-text-field-border-width) border-(--mw-component-text-field-border) bg-(--mw-component-text-field-background) px-(--card-inset) type-message text-(--mw-component-checklist-item-text) outline-none placeholder:text-(--mw-component-text-field-placeholder) focus-visible:focus-ring disabled:opacity-(--mw-component-button-disabled-opacity)"
      />
      <Button type="submit" isDisabled={checklist.items.length >= MAXIMUM_CHECKLIST_ITEMS}>{strings.checklists.addItem}</Button>
    </form>
    <p class="px-(--card-inset) type-body-sm text-(--mw-component-checklist-footer-text)" aria-live="polite">
      {checklist.items.length > 1
        ? strings.checklists.itemsLeft(MAXIMUM_CHECKLIST_ITEMS - checklist.items.length)
        : strings.checklists.lastItemStays}
    </p>
  {/if}

  {#if (canEdit || !canTick) && !isPending}
    <div class="flex flex-wrap items-center justify-between gap-x-sm gap-y-2xs">
      {#if !canTick}
        <p class="px-(--card-inset) type-body-sm text-(--mw-component-checklist-footer-text)">
          {canWrite ? strings.checklists.cannotTick : strings.checklists.readOnly}
        </p>
      {/if}
      {#if canEdit}
        <button
          type="button"
          class="inline-flex h-(--mw-component-button-height-small) items-center gap-2xs rounded-(--mw-component-checklist-row-radius) px-(--card-inset) type-body font-medium text-text-secondary transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:bg-(--mw-component-checklist-row-background-hover) hover:text-text-heading focus-visible:focus-ring pointer-coarse:h-(--mw-size-hit-web-min)"
          onclick={() => (isEditing ? (isEditing = false) : void startEditing())}
        >
          <Icon name={isEditing ? "check" : "pencil-simple"} class="size-(--mw-component-checklist-mark-glyph-size)" />
          {isEditing ? strings.checklists.doneEditing : strings.checklists.edit}
        </button>
      {/if}
    </div>
  {/if}
</section>

{#snippet allDone()}
  <span aria-hidden="true" class="inline-flex shrink-0 items-center gap-3xs type-body font-semibold text-(--mw-component-checklist-done-text)">
    <Icon name="check-circle" class="size-(--mw-component-checklist-mark-glyph-size)" />
    {strings.checklists.allDone}
  </span>
{/snippet}

<style>
  /* The check grows into a box that was just ticked; CSS transitions do not run on the first
     render, so a page of history appears in place. The duration tokens are 0 under reduced motion. */
  .mark {
    transition:
      background-color var(--mw-motion-duration-fast) var(--mw-motion-easing-standard),
      border-color var(--mw-motion-duration-fast) var(--mw-motion-easing-standard);
  }

  .mark :global(.glyph) {
    opacity: 0;
    transform: scale(0.5);
    transition:
      opacity var(--mw-motion-duration-fast) var(--mw-motion-easing-standard),
      transform var(--mw-motion-duration-settle) var(--mw-motion-easing-settle);
  }

  .mark.is-checked :global(.glyph) {
    opacity: 1;
    transform: none;
  }
</style>
