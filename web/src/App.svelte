<script lang="ts">
  import { onMount } from "svelte";

  import Shell from "./components/Shell.svelte";
  import SignInScreen from "./components/SignInScreen.svelte";
  import { MellowClient } from "./lib/client.svelte";

  const client = new MellowClient();
  // Until the instance says whether the session cookie is still valid, nothing is shown, so the
  // sign-in card does not flash before the shell.
  let isResumingSession = $state(true);

  onMount(() => {
    void client.resumeStoredSession().finally(() => (isResumingSession = false));
  });
</script>

{#if isResumingSession}
  <div class="min-h-dvh bg-(--mw-component-auth-card-backdrop)"></div>
{:else if client.status === "signed-out"}
  <SignInScreen {client} />
{:else}
  <Shell {client} />
{/if}
