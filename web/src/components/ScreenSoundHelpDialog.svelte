<script lang="ts">
  import { onMount, tick } from "svelte";

  import { strings } from "../i18n";
  import { LINUX_SHARE_LOOPBACK_COMMAND, LINUX_SHARE_OUTPUT_COMMAND } from "../lib/screen-share";
  import Button from "./Button.svelte";
  import ModalDialog from "./ModalDialog.svelte";
  import { durationTokenInMilliseconds } from "./tokens";

  // Sharing sound on Linux from the browser (docs/design/web-stream-watching.md): the whole
  // computer through a monitor, or one app through a virtual output made once with two commands.
  // The steps are a real sequence, so they are numbered; each command sits in a well with its own
  // Copy button, which says "Copied" for a moment and is read out.
  interface Props {
    onclose: () => void;
  }

  let { onclose }: Props = $props();

  const text = strings.screenShare.soundHelp;
  let copiedStep = $state<number | null>(null);
  let errorMessage = $state("");
  let announcement = $state("");
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;

  onMount(() => () => clearTimeout(copiedTimer));

  async function copy(step: number, command: string) {
    errorMessage = "";
    try {
      await navigator.clipboard.writeText(command);
    } catch (error) {
      console.warn("copying a sound command failed", error);
      errorMessage = text.copyFailed;
      return;
    }
    copiedStep = step;
    // Emptied first, so a second copy is announced again.
    announcement = "";
    await tick();
    announcement = text.commandCopied;
    clearTimeout(copiedTimer);
    // Reduced motion sets the token to 0 ms; "Copied" then stays rather than flashing away.
    const copiedLabelDuration = durationTokenInMilliseconds("component-dialog-copied-label-duration");
    if (copiedLabelDuration > 0) {
      copiedTimer = setTimeout(() => (copiedStep = null), copiedLabelDuration);
    }
  }
</script>

{#snippet command(step: number, line: string)}
  <div
    class="mt-2xs flex items-center gap-xs rounded-(--mw-component-screen-sound-help-command-radius) border border-(--mw-component-screen-sound-help-command-border) bg-(--mw-component-screen-sound-help-command-background) p-(--mw-component-screen-sound-help-command-padding)"
  >
    <code class="min-w-0 flex-1 wrap-anywhere type-code text-(--mw-component-screen-sound-help-command-text)">{line}</code>
    <Button aria-label={text.copyCommand(step)} onclick={() => void copy(step, line)}>
      {copiedStep === step ? text.copied : text.copy}
    </Button>
  </div>
{/snippet}

<ModalDialog title={text.title} cancelLabel={text.close} {errorMessage} {onclose}>
  <section class="flex flex-col gap-2xs">
    <h3 class="type-heading-md text-(--mw-component-dialog-title-text)">{text.wholeComputerTitle}</h3>
    <p class="type-body">{text.wholeComputer}</p>
  </section>
  <section class="flex flex-col gap-2xs">
    <h3 class="type-heading-md text-(--mw-component-dialog-title-text)">{text.oneAppTitle}</h3>
    <ol class="flex list-decimal flex-col gap-xs ps-md type-body marker:tabular-nums">
      <li>
        {text.createOutput}
        {@render command(1, LINUX_SHARE_OUTPUT_COMMAND)}
      </li>
      <li>
        {text.keepHearing}
        {@render command(2, LINUX_SHARE_LOOPBACK_COMMAND)}
      </li>
      <li>{text.moveApp}</li>
      <li>{text.chooseMonitor}</li>
    </ol>
  </section>
  <p class="type-body-sm text-text-muted">{text.footnote}</p>
  <p class="sr-only" aria-live="polite">{announcement}</p>
</ModalDialog>
