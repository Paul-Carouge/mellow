<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";

  import { formattingLocale, strings } from "../../i18n";
  import type { LinkEmbed, LinkPreview, Snowflake } from "../../lib/protocol";
  import { mediumDateLabel, mediumDateLabelWithoutRepeatedYear } from "../format";
  import Icon from "../Icon.svelte";
  import { retryImageOnceThenMarkFailed } from "../image-retry";
  import { compactCount, issueStateBadge, type IssueStateTone, languageColorOf, MAXIMUM_SHOWN_TOPICS } from "./provider-cards";

  // A GitHub repository banner, or an issue or pull request with its state (docs/design/web-provider-embeds.md §4.3):
  // the facts as text instead of GitHub's social picture, states in Mellow's colours with a word and a glyph each.
  interface Props {
    preview: LinkPreview;
    embed: Extract<LinkEmbed, { kind: "github_repository" | "github_issue" | "github_pull_request" }>;
    /** The owner's avatar as the instance stored it, or null. */
    imageUrl: string | null;
    /** The page's colour for the pill, or null for the token's. */
    barColor: string | null;
  }

  let { preview, embed, imageUrl, barColor }: Props = $props();

  const BADGE_TONE_CLASSES: Record<IssueStateTone, string> = {
    open: "bg-(--mw-component-link-preview-state-open-background) text-(--mw-component-link-preview-state-open-text)",
    draft:
      "border-(length:--mw-component-link-preview-border-width) border-(--mw-component-link-preview-state-draft-border) text-(--mw-component-link-preview-state-draft-text)",
    merged: "bg-(--mw-component-link-preview-state-merged-background) text-(--mw-component-link-preview-state-merged-text)",
    completed: "bg-(--mw-component-link-preview-state-completed-background) text-(--mw-component-link-preview-state-completed-text)",
    closed: "bg-(--mw-component-link-preview-state-closed-background) text-(--mw-component-link-preview-state-closed-text)",
  };

  const retriedImageIds = new SvelteSet<Snowflake>();
  const failedImageIds = new SvelteSet<Snowflake>();
  const hasAvatar = $derived(imageUrl !== null && !failedImageIds.has(preview.id));
</script>

{#snippet stat(icon: "star" | "git-fork" | "scales", shown: string, spoken: string)}
  <span class="inline-flex items-center gap-2xs" title={spoken}>
    <Icon name={icon} class="size-(--mw-component-link-preview-stat-icon-size) text-(--mw-component-link-preview-stat-icon)" />
    <span aria-hidden="true">{shown}</span>
    <span class="sr-only">{spoken}</span>
  </span>
{/snippet}

<div
  role="group"
  aria-label={preview.title ?? preview.url}
  class="flex w-full max-w-(--mw-component-link-preview-max-width) gap-(--mw-component-link-preview-gap) rounded-(--mw-component-link-preview-radius) border-(length:--mw-component-link-preview-border-width) border-(--mw-component-link-preview-border) bg-(--mw-component-link-preview-background) p-(--mw-component-link-preview-padding)"
>
  <!-- The page's colour is decoration only; without one, the quote bar's dusk. -->
  <span
    aria-hidden="true"
    style:background-color={barColor}
    class="w-(--mw-component-link-preview-bar-width) shrink-0 self-stretch rounded-full bg-(--mw-component-link-preview-bar-fallback)"
  ></span>
  <div class="flex min-w-0 flex-1 flex-col gap-(--mw-component-link-preview-text-gap)">
    {#if embed.kind === "github_repository"}
      {@const languageColor = languageColorOf(embed)}
      {#if preview.site_name}
        <p class="type-body-sm break-words text-(--mw-component-link-preview-site-text)">{preview.site_name}</p>
      {/if}
      <div class="flex items-center gap-(--mw-component-link-preview-gap)">
        {#if hasAvatar && imageUrl !== null}
          {#key retriedImageIds.has(preview.id)}
            <img
              src={imageUrl}
              alt=""
              width={preview.image?.width}
              height={preview.image?.height}
              loading="lazy"
              decoding="async"
              class="size-(--mw-component-link-preview-owner-avatar-size) shrink-0 rounded-(--mw-component-link-preview-picture-radius) bg-(--mw-component-link-preview-picture-placeholder) object-cover"
              onerror={() => retryImageOnceThenMarkFailed(preview.id, retriedImageIds, failedImageIds)}
            />
          {/key}
        {:else}
          <span
            aria-hidden="true"
            class="grid size-(--mw-component-link-preview-owner-avatar-size) shrink-0 place-items-center rounded-(--mw-component-link-preview-picture-radius) bg-(--mw-component-link-preview-picture-placeholder) type-body-lg font-semibold text-(--mw-component-link-preview-site-text)"
            >{embed.owner.slice(0, 1).toUpperCase()}</span
          >
        {/if}
        <div class="flex min-w-0 flex-wrap items-center gap-x-(--mw-component-link-preview-gap) gap-y-3xs">
          <a
            href={preview.url}
            target="_blank"
            rel="noopener noreferrer"
            class="inline-flex min-h-(--mw-size-hit-desktop-min) min-w-0 flex-wrap items-center gap-x-2xs rounded-xs type-body-lg break-words text-(--mw-component-link-preview-title-text) hover:underline focus-visible:focus-ring pointer-coarse:min-h-(--mw-size-hit-web-min)"
          >
            <span class="min-w-0">{embed.owner} /</span>
            <span class="min-w-0 font-semibold">{embed.name}</span>
          </a>
          {#if embed.is_archived}
            <span
              class="inline-flex h-(--mw-component-link-preview-state-badge-height) items-center rounded-(--mw-component-link-preview-state-badge-radius) px-xs type-body-sm font-semibold {BADGE_TONE_CLASSES.draft}"
              >{strings.providerCards.archived}</span
            >
          {/if}
        </div>
      </div>
      {#if preview.description}
        <p
          class="line-clamp-(--mw-component-link-preview-description-lines) type-body break-words text-(--mw-component-link-preview-description-text)"
        >
          {preview.description}
        </p>
      {/if}
      {#if embed.topics.length > 0}
        <ul aria-label={strings.providerCards.topics} class="flex flex-wrap gap-2xs">
          {#each embed.topics.slice(0, MAXIMUM_SHOWN_TOPICS) as topic (topic)}
            <li
              class="rounded-(--mw-component-link-preview-topic-radius) bg-(--mw-component-link-preview-topic-background) px-(--mw-component-link-preview-topic-padding-inline) py-(--mw-component-link-preview-topic-padding-block) type-body-sm break-all text-(--mw-component-link-preview-topic-text)"
            >
              {topic}
            </li>
          {/each}
        </ul>
      {/if}
      <div
        class="flex flex-wrap items-center gap-x-(--mw-component-link-preview-stat-gap) gap-y-3xs type-body-sm tabular-nums text-(--mw-component-link-preview-stat-text)"
      >
        {#if embed.language}
          <span class="inline-flex items-center gap-2xs">
            <span
              aria-hidden="true"
              style:background-color={languageColor}
              class="size-(--mw-component-link-preview-language-dot-size) shrink-0 rounded-full bg-(--mw-component-link-preview-language-dot-fallback) ring-1 ring-(--mw-component-link-preview-language-dot-ring)"
            ></span>
            <span aria-hidden="true">{embed.language}</span>
            <span class="sr-only">{strings.providerCards.language(embed.language)}</span>
          </span>
        {/if}
        {@render stat("star", compactCount(embed.stars, formattingLocale), strings.providerCards.stars(embed.stars))}
        {@render stat("git-fork", compactCount(embed.forks, formattingLocale), strings.providerCards.forks(embed.forks))}
        {#if embed.licence_spdx}
          {@render stat("scales", embed.licence_spdx, strings.providerCards.licence(embed.licence_spdx))}
        {/if}
      </div>
    {:else}
      {@const badge = issueStateBadge(embed.state, embed.kind === "github_pull_request")}
      <p class="flex flex-wrap gap-x-xs type-body-sm break-words text-(--mw-component-link-preview-site-text)">
        {#if preview.site_name}<span>{preview.site_name}</span>{/if}
        <span>{embed.owner}/{embed.repository}</span>
      </p>
      <div class="flex flex-wrap items-center gap-x-xs gap-y-3xs">
        <span
          class="inline-flex h-(--mw-component-link-preview-state-badge-height) items-center gap-2xs rounded-(--mw-component-link-preview-state-badge-radius) px-xs type-body-sm font-semibold {BADGE_TONE_CLASSES[
            badge.tone
          ]}"
        >
          <Icon name={badge.icon} class="size-(--mw-component-link-preview-state-badge-icon-size)" />
          {strings.providerCards.issueStates[embed.state]}
        </span>
        <span class="type-body-sm tabular-nums text-(--mw-component-link-preview-site-text)">#{embed.number}</span>
      </div>
      <a
        href={preview.url}
        target="_blank"
        rel="noopener noreferrer"
        class="inline-flex min-h-(--mw-size-hit-desktop-min) items-center self-start rounded-xs type-body-lg font-semibold break-words text-(--mw-component-link-preview-title-text) hover:underline focus-visible:focus-ring pointer-coarse:min-h-(--mw-size-hit-web-min)"
        >{preview.title ?? `${embed.owner}/${embed.repository}#${embed.number}`}</a
      >
      {#if preview.description}
        <p
          class="line-clamp-(--mw-component-link-preview-description-lines) type-body break-words text-(--mw-component-link-preview-description-text)"
        >
          {preview.description}
        </p>
      {/if}
      <p
        class="flex flex-wrap items-center gap-x-(--mw-component-link-preview-stat-gap) gap-y-3xs type-body-sm tabular-nums text-(--mw-component-link-preview-stat-text)"
      >
        {#if embed.opened_at_unix_milliseconds !== undefined && embed.author !== ""}
          <span>{strings.providerCards.openedBy(embed.author, mediumDateLabel(embed.opened_at_unix_milliseconds))}</span>
        {/if}
        {#if embed.merged_at_unix_milliseconds !== undefined}
          <!-- The opening date just before shows the year (§4.3), so the line stays one line. -->
          {@const shownOpenedAt = embed.author === "" ? undefined : embed.opened_at_unix_milliseconds}
          <span>{strings.providerCards.mergedOn(mediumDateLabelWithoutRepeatedYear(embed.merged_at_unix_milliseconds, shownOpenedAt))}</span>
        {/if}
        <span class="inline-flex items-center gap-2xs">
          <Icon name="chat-circle" class="size-(--mw-component-link-preview-stat-icon-size) text-(--mw-component-link-preview-stat-icon)" />
          <span aria-hidden="true">{embed.comments}</span>
          <span class="sr-only">{strings.providerCards.comments(embed.comments)}</span>
        </span>
      </p>
    {/if}
  </div>
</div>
