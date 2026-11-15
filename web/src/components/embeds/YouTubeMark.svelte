<script lang="ts" module>
  import youTubeIcon from "../../../../design/brands/youtube-icon.svg?raw";

  // The mark's geometry, read from the repository's file (design/brands), its single source for the
  // clients that draw it.
  const viewBox = /viewBox="([^"]+)"/u.exec(youTubeIcon)?.[1] ?? "";
  const pathData = /\sd="([^"]+)"/u.exec(youTubeIcon)?.[1] ?? "";
</script>

<script lang="ts">
  // YouTube's own icon, unaltered, white on the dark stage (design/brands/README.md): its branding
  // rules want it next to the content it attributes and linking to that content on YouTube.
  interface Props {
    href: string;
    /** The link's tooltip, "Ouvrir sur YouTube : {titre}". */
    label: string;
  }

  let { href, label }: Props = $props();
</script>

<!-- The title already opens the video for keyboards and screen readers; the mark repeats it for the pointer. -->
<a
  {href}
  target="_blank"
  rel="noopener noreferrer"
  tabindex="-1"
  aria-hidden="true"
  title={label}
  class="-m-2xs grid shrink-0 place-items-center p-2xs text-(--mw-component-link-preview-brand-mark-color) pointer-coarse:-m-sm pointer-coarse:p-sm"
>
  <svg {viewBox} fill="currentColor" class="h-(--mw-component-link-preview-brand-mark-height) w-auto"
    ><path fill-rule="evenodd" d={pathData} /></svg
  >
</a>
