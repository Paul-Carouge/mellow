<script lang="ts">
  // A card's title: the link to the page, and the keyboard's first stop in the card.
  interface Props {
    href: string;
    title: string;
  }

  let { href, title }: Props = $props();
</script>

<a
  {href}
  target="_blank"
  rel="noopener noreferrer"
  class="inline-flex min-h-(--mw-size-hit-desktop-min) items-center self-start rounded-xs type-body-lg font-semibold break-words text-(--mw-component-link-preview-title-text) hover:underline focus-visible:focus-ring pointer-coarse:min-h-(--mw-size-hit-web-min)"
  >{title}</a
>
