<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";

  import type { LinkPreview } from "../../lib/protocol";
  import { retryImageOnceThenMarkFailed } from "../image-retry";

  // A preview's picture as the instance stored it (ADR-0041): tried twice, then gone, and
  // `onFailed` lets the card show its placeholder or drop the picture's box.
  interface Props {
    preview: LinkPreview;
    src: string;
    alt: string;
    class: string;
    /** The box's CSS width before the picture loads, when the class does not set it. */
    width?: string;
    onFailed?: () => void;
  }

  let { preview, src, alt, class: className, width, onFailed }: Props = $props();

  const retriedIds = new SvelteSet<string>();
  const failedIds = new SvelteSet<string>();

  function retryOnceThenGiveUp() {
    retryImageOnceThenMarkFailed(preview.id, retriedIds, failedIds);
    if (failedIds.has(preview.id)) {
      onFailed?.();
    }
  }
</script>

{#if !failedIds.has(preview.id)}
  {#key retriedIds.has(preview.id)}
    <img
      {src}
      {alt}
      width={preview.image?.width}
      height={preview.image?.height}
      loading="lazy"
      decoding="async"
      style:width
      class={className}
      onerror={retryOnceThenGiveUp}
    />
  {/key}
{/if}
