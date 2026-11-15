<script lang="ts">
  import { type CallMessageSummary, callMessageSummary } from "../lib/calls";
  import type { MessageCall, Snowflake } from "../lib/protocol";
  import { callMessageLine } from "./calls-view";
  import { unixMillisecondsOfSnowflake } from "./format";
  import Icon, { type IconName } from "./Icon.svelte";

  // A call's message (DESIGN.md §5.6.3, ADR-0062): its caller and time as any message, then a disc
  // with the call's sign and one line: "Appel en cours" (apricot, it is happening), "Appel · 12 min"
  // (quiet, it is history), "Appel sans réponse" (your own call nobody took, quiet), "Appel manqué" (red, as phones say it, never by colour alone: its own
  // sign and words). The banner over the conversation is where one joins.
  interface Props {
    call: MessageCall;
    messageId: Snowflake;
    readerId: Snowflake;
  }

  let { call, messageId, readerId }: Props = $props();

  const summary = $derived(callMessageSummary(call, unixMillisecondsOfSnowflake(messageId), readerId));
  const LOOKS: Record<CallMessageSummary["kind"], { icon: IconName; disc: string; tone: string }> = {
    ongoing: { icon: "phone-call", disc: "bg-(--mw-component-call-message-ongoing-disc)", tone: "text-(--mw-component-call-message-ongoing-icon)" },
    ended: { icon: "phone", disc: "bg-(--mw-component-call-message-ended-disc)", tone: "text-(--mw-component-call-message-ended-icon)" },
    unanswered: { icon: "phone-x", disc: "bg-(--mw-component-call-message-ended-disc)", tone: "text-(--mw-component-call-message-ended-icon)" },
    missed: { icon: "phone-x", disc: "bg-(--mw-component-call-message-missed-disc)", tone: "text-(--mw-component-call-message-missed-icon)" },
  };
  const look = $derived(LOOKS[summary.kind]);
</script>

<p class="flex min-h-(--mw-component-call-message-disc-size) items-center gap-(--mw-component-call-message-gap) type-message text-(--mw-component-call-message-text)">
  <span class="grid size-(--mw-component-call-message-disc-size) shrink-0 place-items-center rounded-full {look.disc}">
    <Icon name={look.icon} class="size-(--mw-component-call-message-icon-size) {look.tone}" />
  </span>
  <span class={["min-w-0", summary.kind === "ongoing" && "font-medium"]}>{callMessageLine(summary)}</span>
</p>
