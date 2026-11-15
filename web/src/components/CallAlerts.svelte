<script lang="ts">
  import type { MellowClient } from "../lib/client.svelte";
  import { alertCallsWhileMounted } from "./call-alerts.svelte";

  // The sounds and the browser notification of private calls (ADR-0062), in a chunk of their own:
  // the shell loads it only while a call rings the reader or holds them.
  interface Props {
    client: MellowClient;
  }

  let { client }: Props = $props();

  alertCallsWhileMounted(() => client);
</script>
