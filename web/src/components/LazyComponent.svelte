<script lang="ts" generics="Module">
  import type { Snippet } from "svelte";

  import { strings } from "../i18n";
  import ModalDialog from "./ModalDialog.svelte";

  // A part of the client that is not needed at startup (the settings, some dialogs) lives in its
  // own chunk, downloaded when it first opens. A tab opened before an update asks for chunks the
  // instance no longer has: the import fails, and the reader is offered a reload instead of a
  // control that does nothing.
  interface Props {
    load: () => Promise<Module>;
    children: Snippet<[Module]>;
    /** Closes whatever opened this part, so it can be opened again. */
    onclose: () => void;
  }

  let { load, children, onclose }: Props = $props();

  // The browser keeps each imported module, so asking again never downloads a chunk twice.
  const loading = $derived(load());
</script>

{#await loading}
  <p
    role="status"
    class="fixed inset-x-0 top-(--mw-component-dialog-margin) z-50 mx-auto w-fit animate-[mw-fade-in_var(--mw-component-dialog-enter-duration)_var(--mw-component-dialog-enter-duration)_both] rounded-(--mw-component-dialog-radius) bg-(--mw-component-dialog-background) px-(--mw-component-dialog-padding) py-(--mw-component-notice-bar-padding-block) type-body text-(--mw-component-dialog-body-text) shadow-high"
  >
    {strings.app.loadingPart}
  </p>
{:then module}
  {@render children(module)}
{:catch}
  <ModalDialog
    title={strings.app.partUnavailableTitle}
    subtitle={strings.app.partUnavailableBody}
    submitLabel={strings.app.reload}
    {onclose}
    onsubmit={() => window.location.reload()}
  />
{/await}
