<script lang="ts">
  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { PublicUser } from "../lib/protocol";
  import { displayName } from "./format";
  import ModalDialog from "./ModalDialog.svelte";

  // "Remove friend" and "Block", the destructive pattern of DESIGN.md §8: Cancel takes focus.
  // Blocking says what it does, and that the other person is never told (ADR-0025).
  interface Props {
    client: MellowClient;
    action: "remove" | "block";
    user: PublicUser;
    onclose: () => void;
    /** After the instance agreed: the result's sentence, for a live region. */
    ondone: (announcement: string) => void;
    fallbackFocus: () => void;
  }

  let { client, action, user, onclose, ondone, fallbackFocus }: Props = $props();

  const person = $derived(displayName(user));
  let isBusy = $state(false);
  // Plain: read by the dialog when it closes, after this component stopped updating.
  let hasSucceeded = false;
  let errorMessage = $state("");

  async function confirm() {
    isBusy = true;
    errorMessage = "";
    try {
      if (action === "block") {
        await client.blockUser(user.id);
      } else {
        await client.removeRelationship(user.id);
      }
      hasSucceeded = true;
      ondone(action === "block" ? strings.friends.blocked(person) : strings.friends.removed(person));
      onclose();
    } catch (error) {
      errorMessage = describeError(
        error,
        action === "block" ? strings.friends.blockFailed(person) : strings.friends.removeFailed(person),
      );
    } finally {
      isBusy = false;
    }
  }
</script>

<ModalDialog
  shouldReturnFocusToOpener={() => !hasSucceeded}
  title={action === "block" ? strings.friends.blockTitle(person) : strings.friends.removeTitle(person)}
  subtitle={action === "block" ? strings.friends.blockBody(person) : strings.friends.removeBody(person)}
  {errorMessage}
  {isBusy}
  submitLabel={isBusy
    ? action === "block"
      ? strings.friends.blocking
      : strings.friends.removing
    : action === "block"
      ? strings.friends.blockSubmit(person)
      : strings.friends.removeFriend}
  isSubmitDanger
  {onclose}
  onsubmit={confirm}
  {fallbackFocus}
/>
