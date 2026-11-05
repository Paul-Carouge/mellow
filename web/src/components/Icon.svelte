<script lang="ts" module>
  // The desktop client's Phosphor set (DESIGN.md §2.9). Each file uses `fill="currentColor"`, so
  // the icon takes the text colour of its parent, which comes from the tokens.
  const ICON_SOURCES = import.meta.glob<string>("../../../design/icons/*.svg", {
    query: "?raw",
    import: "default",
    eager: true,
  });

  export type IconName =
    | "airplane-tilt"
    | "arrow-bend-up-left"
    | "arrow-bend-up-right"
    | "arrow-square-out"
    | "at"
    | "bell"
    | "bell-slash"
    | "bowl-food"
    | "caret-down"
    | "caret-left"
    | "caret-right"
    | "caret-up"
    | "cell-signal-high"
    | "cell-signal-low"
    | "cell-signal-medium"
    | "chat-circle"
    | "check"
    | "clock"
    | "code"
    | "code-block"
    | "compass"
    | "copy"
    | "corners-in"
    | "corners-out"
    | "crown"
    | "dots-three"
    | "download-simple"
    | "eye"
    | "eye-slash"
    | "file"
    | "file-arrow-up"
    | "file-audio"
    | "flag"
    | "gif"
    | "gear-six"
    | "hash"
    | "headphones"
    | "heart"
    | "lightbulb"
    | "link-simple"
    | "list"
    | "list-bullets"
    | "list-numbers"
    | "lock"
    | "magnifying-glass"
    | "microphone"
    | "microphone-slash"
    | "minus"
    | "monitor-arrow-up"
    | "paper-plane-right"
    | "paw-print"
    | "pencil-simple"
    | "phone-disconnect"
    | "play"
    | "plus"
    | "plus-circle"
    | "prohibit"
    | "push-pin"
    | "push-pin-slash"
    | "quotes"
    | "shield-warning"
    | "sign-out"
    | "smiley"
    | "smiley-angry"
    | "smiley-meh"
    | "soccer-ball"
    | "speaker-high"
    | "speaker-slash"
    | "speaker-x"
    | "squares-four"
    | "star"
    | "star-outline"
    | "text-b"
    | "text-h-one"
    | "text-h-three"
    | "text-h-two"
    | "text-italic"
    | "text-strikethrough"
    | "text-underline"
    | "trash"
    | "user-focus"
    | "user-minus"
    | "user-plus"
    | "users"
    | "video-camera"
    | "video-camera-slash"
    | "warning-circle"
    | "wifi-slash"
    | "x"
    | "x-circle";

  function iconSource(name: IconName): string {
    const source = ICON_SOURCES[`../../../design/icons/${name}.svg`];
    if (source === undefined) {
      throw new Error(`icon ${name} is missing from design/icons`);
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
