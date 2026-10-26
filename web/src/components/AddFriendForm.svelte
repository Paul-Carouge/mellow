<script lang="ts">
  import { strings } from "../i18n";
  import { ApiError } from "../lib/api";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import Button from "./Button.svelte";
  import { displayName } from "./format";
  import { friendRequestOutcome } from "./friends";
  import Icon from "./Icon.svelte";

  // "Add friend" (DESIGN.md §5.6.1, ADR-0025): one username field with the request button inside
  // it, as on Discord; the answer is said under it in words and read out. A missing account,
  // yourself and someone who blocked you read the same, so the form tells nothing about blocks.
  interface Props {
    client: MellowClient;
  }

  let { client }: Props = $props();

  const id = $props.id();
  let username = $state("");
  let isSending = $state(false);
  let result = $state<{ isSuccess: boolean; text: string } | null>(null);
  let field = $state<HTMLInputElement | null>(null);

  const canSend = $derived(username.trim() !== "" && !isSending);

  export function focusField(): void {
    field?.focus();
  }

  async function send() {
    if (!canSend) {
      return;
    }
    isSending = true;
    result = null;
    const kindsBefore = new Map([...client.relationshipByUserId].map(([userId, relationship]) => [userId, relationship.kind]));
    try {
      const relationship = await client.sendFriendRequest(username);
      const person = displayName(relationship.user);
      const outcome = friendRequestOutcome(kindsBefore.get(relationship.user.id), relationship.kind);
      result = {
        isSuccess: true,
        text:
          outcome === "accepted"
            ? strings.friends.acceptedTheirs(person)
            : outcome === "already-friends"
              ? strings.friends.alreadyFriends(person)
              : outcome === "already-sent"
                ? strings.friends.alreadySent(person)
                : strings.friends.sent(person),
      };
      username = "";
    } catch (error) {
      result = { isSuccess: false, text: refusalText(error) };
    } finally {
      isSending = false;
    }
    field?.focus();
  }

  function refusalText(error: unknown): string {
    if (error instanceof ApiError) {
      switch (error.code) {
        case "NOT_FOUND":
          return strings.friends.notFound;
        case "RATE_LIMITED":
          return strings.friends.rateLimited;
        case "LIMIT_REACHED":
          return strings.friends.limitReached;
      }
    }
    return describeError(error, strings.friends.sendFailed);
  }
</script>

<form
  class="flex max-w-(--mw-component-friends-content-max-width) flex-col gap-xs"
  aria-labelledby="{id}-title"
  onsubmit={(event) => {
    event.preventDefault();
    void send();
  }}
>
  <h2 id="{id}-title" class="type-heading-lg text-text-heading">{strings.friends.addTitle}</h2>
  <p id="{id}-hint" class="type-body text-text-secondary">{strings.friends.addSubtitle(window.location.host)}</p>
  <!-- The button sits inside the field, as on Discord; on a phone it wraps under the name. -->
  <div
    class="mt-xs flex flex-wrap items-center gap-xs rounded-(--mw-component-text-field-radius) border-(length:--mw-component-text-field-border-width) border-(--mw-component-text-field-border) bg-(--mw-component-text-field-background) p-2xs ps-(--mw-component-text-field-padding-inline) focus-within:focus-ring"
  >
    <label for="{id}-username" class="sr-only">{strings.friends.username}</label>
    <input
      bind:this={field}
      bind:value={username}
      id="{id}-username"
      type="text"
      autocomplete="off"
      autocapitalize="none"
      spellcheck="false"
      placeholder={strings.friends.usernamePlaceholder}
      aria-describedby="{id}-hint {id}-result"
      aria-invalid={result !== null && !result.isSuccess}
      readonly={isSending}
      class="h-(--mw-component-button-height-large) min-w-[min(100%,var(--mw-component-friends-username-min-width))] grow-[999] bg-transparent type-body-lg text-(--mw-component-text-field-text) outline-none placeholder:text-(--mw-component-text-field-placeholder)"
      oninput={() => (result = null)}
    />
    <!-- Beside the field it keeps its width; alone on its row (phones), it spans it. -->
    <div class="flex grow [&>button]:grow">
      <Button type="submit" variant="primary" isDisabled={!canSend}>
        {isSending ? strings.friends.sending : strings.friends.send}
      </Button>
    </div>
  </div>
  <!-- Polite: the answer is read once it arrives, whichever it is. -->
  <p id="{id}-result" aria-live="polite" class="min-h-[calc(var(--mw-type-body-font-size)*var(--mw-type-body-line-height))]">
    {#if result}
      <span
        class={[
          "flex items-start gap-2xs type-body",
          result.isSuccess
            ? "text-(--mw-component-friends-result-positive-text)"
            : "text-(--mw-component-friends-result-error-text)",
        ]}
      >
        <Icon
          name={result.isSuccess ? "check" : "warning-circle"}
          class="mt-px size-(--mw-component-friends-result-icon-size) shrink-0"
        />
        <span>{result.text}</span>
      </span>
    {/if}
  </p>
</form>
