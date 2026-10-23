<script lang="ts">
  import { mentionBadgeText } from "./format";

  // The red mention count (DESIGN.md §2.3, §4.2): never animated. Hidden from screen readers,
  // because the row or the icon it sits on says "3 mentions" in its own name.
  interface Props {
    mentionCount: number;
    /** On an icon: cut out of this surface token, at the icon's bottom trailing corner. */
    ringColorToken?: string;
    class?: string;
  }

  let { mentionCount, ringColorToken, class: className = "" }: Props = $props();
</script>

<span
  aria-hidden="true"
  class={[
    "grid h-(--mw-component-badge-height) min-w-(--mw-component-badge-min-width) shrink-0 place-items-center rounded-(--mw-component-badge-radius) bg-(--mw-component-badge-background) px-(--mw-component-badge-padding-inline) type-badge text-(--mw-component-badge-text) tabular-nums",
    ringColorToken && "shadow-[0_0_0_var(--mw-component-badge-ring-width)_var(--badge-ring)]",
    className,
  ]}
  style:--badge-ring={ringColorToken ? `var(${ringColorToken})` : undefined}>{mentionBadgeText(mentionCount)}</span
>
