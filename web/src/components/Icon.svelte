<script lang="ts" module>
  // The desktop client's Phosphor set (DESIGN.md §2.9). Each file uses `fill="currentColor"`, so
  // the icon takes the text colour of its parent, which comes from the tokens.
  const ICON_SOURCES = import.meta.glob<string>("../../../crates/mellow-desktop/icons/*.svg", {
    query: "?raw",
    import: "default",
    eager: true,
  });

  export type IconName =
    | "arrow-bend-up-left"
    | "at"
    | "caret-down"
    | "caret-right"
    | "caret-up"
    | "chat-circle"
    | "check"
    | "clock"
    | "copy"
    | "dots-three"
    | "download-simple"
    | "eye"
    | "eye-slash"
    | "file"
    | "file-arrow-up"
    | "gear-six"
    | "hash"
    | "headphones"
    | "list"
    | "lock"
    | "magnifying-glass"
    | "microphone"
    | "minus"
    | "paper-plane-right"
    | "pencil-simple"
    | "plus"
    | "plus-circle"
    | "prohibit"
    | "push-pin"
    | "push-pin-slash"
    | "sign-out"
    | "smiley"
    | "trash"
    | "user-minus"
    | "user-plus"
    | "users"
    | "warning-circle"
    | "wifi-slash"
    | "x"
    | "x-circle";

  function iconSource(name: IconName): string {
    const source = ICON_SOURCES[`../../../crates/mellow-desktop/icons/${name}.svg`];
    if (source === undefined) {
      throw new Error(`icon ${name} is missing from crates/mellow-desktop/icons`);
    }
    return source;
  }
</script>

<script lang="ts">
  interface Props {
    name: IconName;
    /** Size and colour utilities; the icon is decorative, its control carries the label. */
    class?: string;
  }

  let { name, class: className = "" }: Props = $props();
</script>

<!-- Static SVG files from the repository, never user content. -->
<span aria-hidden="true" class={["inline-flex shrink-0 [&>svg]:size-full", className]}>{@html iconSource(name)}</span>
