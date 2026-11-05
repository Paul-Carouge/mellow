<script lang="ts">
  import { onMount } from "svelte";

  import { strings } from "../i18n";
  import { POLL_VOTERS_PAGE_SIZE } from "../lib/api";
  import type { PublicUser, Snowflake } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import { displayName } from "./format";
  import ModalDialog from "./ModalDialog.svelte";

  // Who voted for one answer of a poll (DESIGN.md §6.83, docs/design/web-polls.md), as "who
  // reacted" lists people: avatar and name, a page at a time, "Show more" while pages come full.
  interface Props {
    answerText: string;
    /** The voters after `after`, by ascending id. */
    loadVoters: (after?: Snowflake) => Promise<PublicUser[]>;
    onclose: () => void;
    fallbackFocus: () => void;
  }

  let { answerText, loadVoters, onclose, fallbackFocus }: Props = $props();

  let voters = $state<PublicUser[]>([]);
  let loadState = $state<"loading" | "loaded" | "failed">("loading");
  let hasMore = $state(false);
  let isLoadingMore = $state(false);
  let errorMessage = $state("");

  onMount(() => {
    // The dialog closing before the answer comes leaves nothing to fill.
    let isCurrent = true;
    loadVoters().then(
      (page) => {
        if (isCurrent) {
          voters = page;
          hasMore = page.length === POLL_VOTERS_PAGE_SIZE;
          loadState = "loaded";
        }
      },
      (error: unknown) => {
        console.warn("loading poll voters failed", error);
        if (isCurrent) {
          loadState = "failed";
        }
      },
    );
    return () => {
      isCurrent = false;
    };
  });

  async function loadMore() {
    const last = voters.at(-1);
    if (!last || isLoadingMore) {
      return;
    }
    isLoadingMore = true;
    errorMessage = "";
    try {
      const page = await loadVoters(last.id);
      const known = new Set(voters.map((voter) => voter.id));
      voters = [...voters, ...page.filter((voter) => !known.has(voter.id))];
      hasMore = page.length === POLL_VOTERS_PAGE_SIZE;
    } catch (error) {
      console.warn("loading more poll voters failed", error);
      errorMessage = strings.polls.moreVotersFailed;
    } finally {
      isLoadingMore = false;
    }
  }
</script>

<ModalDialog title={strings.polls.votersTitle(answerText)} cancelLabel={strings.common.close} {errorMessage} {onclose} {fallbackFocus}>
  {#if loadState === "loading"}
    <p role="status" class="type-body text-text-muted">{strings.polls.loadingVoters}</p>
  {:else if loadState === "failed"}
    <p role="alert" class="type-body text-text-muted">{strings.polls.votersFailed}</p>
  {:else if voters.length === 0}
    <p class="type-body text-text-muted">{strings.polls.noVoters}</p>
  {:else}
    <ul class="flex flex-col">
      {#each voters as voter (voter.id)}
        <li class="flex min-h-(--mw-component-member-row-height) items-center gap-sm pointer-coarse:min-h-(--mw-size-hit-web-min)">
          <Avatar user={voter} sizeToken="--mw-component-member-row-avatar-size" />
          <span class="min-w-0 flex-1 truncate type-member-name text-text-normal">{displayName(voter)}</span>
        </li>
      {/each}
    </ul>
    {#if hasMore}
      <div>
        <Button variant="secondary" isDisabled={isLoadingMore} onclick={() => void loadMore()}>{strings.polls.moreVoters}</Button>
      </div>
    {/if}
  {/if}
</ModalDialog>
