<script lang="ts">
  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { DirectMessageChannel, PublicUser } from "../lib/protocol";
  import { displayName } from "./format";
  import { conversationDisplayName, nextOwnerAfterLeaving, otherParticipants } from "./group-conversations";
  import ModalDialog from "./ModalDialog.svelte";

  // "Leave group" and the owner's "Remove from group" (DESIGN.md §5.6, ADR-0026), the destructive
  // pattern of DESIGN.md §8: Cancel takes focus. Leaving says who will own the group next, or that
  // the group goes with its last person.
  interface Props {
    client: MellowClient;
    group: DirectMessageChannel;
    /** `null` leaves; a person is removed by the owner. */
    person: PublicUser | null;
    onclose: () => void;
    /** After the instance agreed: the result's sentence for a live region (removals only). */
    ondone: (announcement: string) => void;
    fallbackFocus: () => void;
  }

  let { client, group, person, onclose, ondone, fallbackFocus }: Props = $props();

  const currentUserId = $derived(client.user?.id ?? null);
  const groupName = $derived(conversationDisplayName(group, currentUserId));
  const personName = $derived(person ? displayName(person) : "");
  const leaveBody = $derived.by(() => {
    if (otherParticipants(group, currentUserId).length === 0) {
      return strings.groups.leaveBodyAsLast;
    }
    const nextOwner = nextOwnerAfterLeaving(group, currentUserId);
    return nextOwner ? strings.groups.leaveBodyAsOwner(displayName(nextOwner)) : strings.groups.leaveBody;
  });
  let isBusy = $state(false);
  // Plain: read by the dialog when it closes, after this component stopped updating.
  let hasSucceeded = false;
  let errorMessage = $state("");

  async function confirm() {
    isBusy = true;
    errorMessage = "";
    try {
      if (person) {
        await client.removeGroupParticipant(group.id, person.id);
        ondone(strings.groups.removed(personName));
      } else {
        // The client drops the group and goes back to Friends; the shell announces it.
        await client.leaveGroupConversation(group.id);
      }
      hasSucceeded = true;
      onclose();
    } catch (error) {
      errorMessage = describeError(
        error,
        person ? strings.groups.removeFailed(personName) : strings.groups.leaveFailed,
      );
    } finally {
      isBusy = false;
    }
  }
</script>

<ModalDialog
  shouldReturnFocusToOpener={() => !hasSucceeded}
  title={person ? strings.groups.removeTitle(personName) : strings.groups.leaveTitle(groupName)}
  subtitle={person ? strings.groups.removeBody(personName) : leaveBody}
  {errorMessage}
  {isBusy}
  submitLabel={isBusy
    ? person
      ? strings.groups.removing
      : strings.groups.leaving
    : person
      ? strings.groups.removeSubmit(personName)
      : strings.groups.leave}
  isSubmitDanger
  {onclose}
  onsubmit={confirm}
  {fallbackFocus}
/>
