<script lang="ts">
  import { onMount } from "svelte";

  import Button from "./components/Button.svelte";
  import Shell from "./components/Shell.svelte";
  import SignInScreen from "./components/SignInScreen.svelte";
  import { strings } from "./i18n";
  import { MellowClient } from "./lib/client.svelte";

  const client = new MellowClient();
  // Until the instance says whether the session cookie is still valid, nothing is shown, so the
  // sign-in card does not flash before the shell.
  let isResumingSession = $state(true);

  function resume() {
    isResumingSession = true;
    void client.resumeStoredSession().finally(() => (isResumingSession = false));
  }

  onMount(() => {
    resume();
    // Messages that arrived while the tab was hidden are read once it shows again.
    const noteVisibility = () => {
      if (document.visibilityState === "visible") {
        client.noteTabVisible();
      } else {
        client.noteTabHidden();
      }
    };
    document.addEventListener("visibilitychange", noteVisibility);
    return () => document.removeEventListener("visibilitychange", noteVisibility);
  });
</script>

{#if isResumingSession}
  <div class="min-h-dvh bg-(--mw-component-auth-card-backdrop)"></div>
{:else if client.status === "unreachable"}
  <main class="grid min-h-dvh place-items-center bg-(--mw-component-auth-card-backdrop) p-md">
    <div role="alert" class="flex max-w-md flex-col items-center gap-md text-center">
      <p class="type-heading-md text-text-heading">{strings.app.unreachableTitle}</p>
      <p class="type-body text-text-muted">{strings.app.unreachableBody}</p>
      <Button variant="primary" onclick={resume}>{strings.common.tryAgain}</Button>
    </div>
  </main>
{:else if client.status === "signed-out"}
  <SignInScreen {client} />
{:else}
  <Shell {client} />
{/if}
