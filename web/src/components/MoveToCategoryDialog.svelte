<script lang="ts">
  import { untrack } from "svelte";

  import { strings } from "../i18n";
  import type { ChannelSection } from "../lib/channels";
  import type { Channel, Snowflake } from "../lib/protocol";
  import ChoiceOption from "./ChoiceOption.svelte";
  import ModalDialog from "./ModalDialog.svelte";

  // "Move to category…" (DESIGN.md §5.3): the single-pointer and keyboard way to drag a channel
  // into another category, or out of any. It lands at the end of the one chosen.
  interface Props {
    channel: Channel;
    /** The sidebar's sections as shown, a pending order included. */
    sections: readonly ChannelSection[];
    /** Resolves to an error to show, or null once moved. */
    onmove: (categoryId: Snowflake | null) => Promise<string | null>;
    onclose: () => void;
    fallbackFocus: () => void;
  }

  let { channel, sections, onmove, onclose, fallbackFocus }: Props = $props();

  /** The radios' value for "No category". */
  const NO_CATEGORY = "";
  const id = $props.id();
  const currentCategoryId = untrack(
    () => sections.find((section) => section.channels.some((candidate) => candidate.id === channel.id))?.category?.id ?? null,
  );
  let chosen = $state(currentCategoryId ?? NO_CATEGORY);
  let isBusy = $state(false);
  let errorMessage = $state("");
  // Plain: read by the dialog when it closes, after this component stopped updating.
  let hasSucceeded = false;

  const categories = $derived(sections.flatMap((section) => (section.category ? [section.category] : [])));

  async function move() {
    isBusy = true;
    errorMessage = "";
    const problem = await onmove(chosen === NO_CATEGORY ? null : chosen);
    isBusy = false;
    if (problem === null) {
      hasSucceeded = true;
      onclose();
    } else {
      errorMessage = problem;
    }
  }
</script>

<ModalDialog
  shouldReturnFocusToOpener={() => !hasSucceeded}
  title={strings.channelOrder.moveTitle(channel.name)}
  subtitle={strings.channelOrder.moveSubtitle}
  {errorMessage}
  {isBusy}
  submitLabel={isBusy ? strings.channelOrder.moving : strings.channelOrder.move}
  isSubmitEnabled={chosen !== (currentCategoryId ?? NO_CATEGORY)}
  {onclose}
  onsubmit={() => void move()}
  {fallbackFocus}
>
  <fieldset class="flex flex-col gap-(--mw-component-choice-list-gap)" disabled={isBusy}>
    <legend class="mb-(--mw-component-text-field-label-gap) type-category text-(--mw-component-text-field-label-text)">
      {strings.channelOrder.destination}
    </legend>
    <ChoiceOption
      name="{id}-category"
      value={NO_CATEGORY}
      bind:group={chosen}
      label={strings.channelOrder.noCategory}
      description={currentCategoryId === null ? strings.channelOrder.currentPlace : ""}
      isInitialFocus={currentCategoryId === null}
    />
    {#each categories as category (category.id)}
      <ChoiceOption
        name="{id}-category"
        value={category.id}
        bind:group={chosen}
        label={category.name}
        description={category.id === currentCategoryId ? strings.channelOrder.currentPlace : ""}
        isInitialFocus={category.id === currentCategoryId}
      />
    {/each}
  </fieldset>
</ModalDialog>
