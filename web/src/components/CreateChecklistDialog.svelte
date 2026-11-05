<script lang="ts">
  import { tick } from "svelte";

  import { strings } from "../i18n";
  import {
    MAXIMUM_CHECKLIST_ITEMS,
    MAXIMUM_CHECKLIST_ITEM_LENGTH,
    MAXIMUM_CHECKLIST_TITLE_LENGTH,
    type NewChecklist,
  } from "../lib/protocol";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import ModalDialog from "./ModalDialog.svelte";
  import { characterCount } from "./poll-form";
  import {
    type ChecklistDraft,
    canAddChecklistRow,
    checklistDraftProblems,
    emptyChecklistDraft,
    hasChecklistDraftProblems,
    newChecklistOf,
  } from "./tracker-form";

  // "Create a checklist" (docs/design/web-checklists-and-progress.md, ADR-0047), opened from the
  // "/" menu: a title and up to 25 items, laid out as "Create a poll". A too long text is said as
  // it is typed; a missing title or item only once "Send checklist" is pressed. Sending goes
  // through the outbox, so the dialog closes at once and a failure shows on the checklist.
  interface Props {
    onsend: (checklist: NewChecklist) => void;
    onclose: () => void;
    fallbackFocus: () => void;
  }

  let { onsend, onclose, fallbackFocus }: Props = $props();

  /** Past this many characters of an item, its counter shows (the title's always does). */
  const ITEM_COUNTER_THRESHOLD = MAXIMUM_CHECKLIST_ITEM_LENGTH - 20;
  const FIELD_CLASS =
    "flex h-(--mw-component-text-field-height) min-w-0 items-center gap-xs rounded-(--mw-component-text-field-radius) border-(length:--mw-component-text-field-border-width) border-(--mw-component-text-field-border) bg-(--mw-component-text-field-background) px-(--mw-component-text-field-padding-inline) focus-within:focus-ring has-[input[aria-invalid=true]]:border-(--mw-component-dialog-error-text)";
  const INPUT_CLASS =
    "h-full min-w-0 flex-1 bg-transparent type-body-lg text-(--mw-component-text-field-text) outline-none placeholder:text-(--mw-component-text-field-placeholder)";

  const id = $props.id();
  const draft = $state<ChecklistDraft>(emptyChecklistDraft());
  /** Keys for the rows, so a removed row takes its own field and focus with it. */
  let itemKeys = $state(draft.items.map((_, index) => index));
  let nextItemKey = draft.items.length;
  let hasTriedSending = $state(false);
  let itemList = $state<HTMLElement | null>(null);

  const problems = $derived(checklistDraftProblems(draft));
  const titleMessage = $derived(
    problems.title === "tooLong"
      ? strings.polls.textTooLong(MAXIMUM_CHECKLIST_TITLE_LENGTH)
      : problems.title === "empty" && hasTriedSending
        ? strings.checklists.titleEmpty
        : "",
  );
  const itemsMessage = $derived(problems.hasNoItem && hasTriedSending ? strings.checklists.tooFewItems : "");

  function itemInputs(): HTMLInputElement[] {
    return [...(itemList?.querySelectorAll<HTMLInputElement>("input") ?? [])];
  }

  async function addRow() {
    if (!canAddChecklistRow(draft)) {
      return;
    }
    draft.items.push("");
    itemKeys.push(nextItemKey);
    nextItemKey += 1;
    await tick();
    itemInputs().at(-1)?.focus();
  }

  async function removeRow(index: number) {
    if (draft.items.length <= 1) {
      return;
    }
    draft.items.splice(index, 1);
    itemKeys.splice(index, 1);
    await tick();
    const inputs = itemInputs();
    (inputs[index] ?? inputs[index - 1])?.focus();
  }

  /** Enter goes to the next row, adding one after the last; never sends. */
  function handleItemKeydown(event: KeyboardEvent, index: number) {
    if (event.key !== "Enter" || event.isComposing) {
      return;
    }
    event.preventDefault();
    const next = itemInputs()[index + 1];
    if (next) {
      next.focus();
    } else {
      void addRow();
    }
  }

  async function send() {
    if (hasChecklistDraftProblems(problems)) {
      hasTriedSending = true;
      await tick();
      if (problems.title !== null) {
        document.getElementById(`${id}-title`)?.focus();
      } else {
        const inputs = itemInputs();
        const withProblem = problems.itemsTooLong.findIndex(Boolean);
        (inputs[withProblem >= 0 ? withProblem : 0] ?? inputs[0])?.focus();
      }
      return;
    }
    onsend(newChecklistOf($state.snapshot(draft)));
  }
</script>

{#snippet counter(fieldId: string, used: number, maximum: number)}
  <span
    id="{fieldId}-count"
    class={[
      "shrink-0 type-body-sm tabular-nums",
      used > maximum ? "text-(--mw-component-dialog-error-text)" : "text-(--mw-component-text-field-helper-text)",
    ]}
    ><span aria-hidden="true">{strings.polls.charactersUsed(used, maximum)}</span><span class="sr-only"
      >{strings.polls.charactersUsedSpoken(used, maximum)}</span
    ></span
  >
{/snippet}

{#snippet fieldMessage(fieldId: string, message: string)}
  <!-- Kept in the page, so a message that appears is announced. -->
  <p id="{fieldId}-message" role="status" class="type-body-sm text-(--mw-component-dialog-error-text) empty:hidden">{message}</p>
{/snippet}

<ModalDialog
  title={strings.checklists.create}
  subtitle={strings.checklists.createSubtitle}
  submitLabel={strings.checklists.send}
  {onclose}
  onsubmit={() => void send()}
  {fallbackFocus}
>
  <div class="flex flex-col gap-(--mw-component-text-field-label-gap)">
    <div class="flex items-baseline justify-between gap-xs">
      <label for="{id}-title" class="type-category text-(--mw-component-text-field-label-text)">{strings.checklists.title}</label>
      {@render counter(`${id}-title`, characterCount(draft.title), MAXIMUM_CHECKLIST_TITLE_LENGTH)}
    </div>
    <div class={FIELD_CLASS}>
      <input
        id="{id}-title"
        bind:value={draft.title}
        class={INPUT_CLASS}
        placeholder={strings.checklists.titlePlaceholder}
        autocomplete="off"
        enterkeyhint="next"
        aria-invalid={titleMessage !== "" || undefined}
        aria-describedby="{id}-title-count {id}-title-message"
        data-initial-focus=""
        onkeydown={(event) => {
          if (event.key === "Enter" && !event.isComposing) {
            event.preventDefault();
            itemInputs()[0]?.focus();
          }
        }}
      />
    </div>
    {@render fieldMessage(`${id}-title`, titleMessage)}
  </div>

  <fieldset class="flex min-w-0 flex-col gap-(--mw-component-text-field-label-gap)">
    <legend class="mb-(--mw-component-text-field-label-gap) type-category text-(--mw-component-text-field-label-text)">
      {strings.checklists.items}
    </legend>
    <ol bind:this={itemList} class="flex flex-col gap-(--mw-component-text-field-helper-gap)">
      {#each draft.items as item, index (itemKeys[index])}
        {@const fieldId = `${id}-item-${itemKeys[index]}`}
        {@const length = characterCount(item)}
        {@const message = problems.itemsTooLong[index] ? strings.polls.textTooLong(MAXIMUM_CHECKLIST_ITEM_LENGTH) : ""}
        <li class="flex flex-col gap-3xs">
          <div class="flex items-center gap-2xs">
            <div class={[FIELD_CLASS, "flex-1"]}>
              <input
                id={fieldId}
                bind:value={draft.items[index]}
                class={INPUT_CLASS}
                placeholder={strings.checklists.item(index + 1)}
                aria-label={strings.checklists.item(index + 1)}
                autocomplete="off"
                enterkeyhint="next"
                aria-invalid={message !== "" || undefined}
                aria-describedby="{fieldId}-message{length > ITEM_COUNTER_THRESHOLD ? ` ${fieldId}-count` : ''}"
                onkeydown={(event) => handleItemKeydown(event, index)}
              />
              {#if length > ITEM_COUNTER_THRESHOLD}
                {@render counter(fieldId, length, MAXIMUM_CHECKLIST_ITEM_LENGTH)}
              {/if}
            </div>
            {#if draft.items.length > 1}
              <IconButton
                icon="x"
                label={strings.checklists.removeItem(index + 1)}
                class="size-(--mw-size-hit-web-min) text-(--mw-component-text-field-icon) hover:bg-state-hover hover:text-(--mw-component-text-field-icon-hover) focus-visible:focus-ring"
                iconClass="size-(--mw-component-text-field-icon-size)"
                onclick={() => void removeRow(index)}
              />
            {/if}
          </div>
          {@render fieldMessage(fieldId, message)}
        </li>
      {/each}
    </ol>
    {@render fieldMessage(`${id}-items`, itemsMessage)}
    <div class="flex flex-wrap items-center gap-x-sm gap-y-2xs">
      {#if canAddChecklistRow(draft)}
        <div class="-ms-(--mw-component-button-padding-inline)">
          <Button variant="ghost" onclick={() => void addRow()}>
            <Icon name="plus" class="me-2xs size-(--mw-component-text-field-icon-size)" />
            {strings.checklists.addItem}
          </Button>
        </div>
      {/if}
      <p class="type-body-sm text-(--mw-component-text-field-helper-text)" aria-live="polite">
        {strings.checklists.itemsLeft(MAXIMUM_CHECKLIST_ITEMS - draft.items.length)}
      </p>
    </div>
  </fieldset>
</ModalDialog>
