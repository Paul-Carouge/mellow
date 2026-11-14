<script lang="ts">
  import { strings } from "../i18n";
  import { type ProfileLink, linkIconUrl } from "../lib/protocol";
  import Icon from "./Icon.svelte";
  import { parseInlineMarkdown, safeHttpUrl } from "./markdown";
  import MessageMarkdown from "./MessageMarkdown.svelte";

  // A profile's "About me" and links (ADR-0050, DESIGN.md §5.7), under the name in the profile
  // card. Each link shows its site's icon, stored by the instance (browsers never ask the site),
  // or a link glyph until it is fetched; its label, or its host, names it.
  interface Props {
    aboutMe: string;
    links: readonly ProfileLink[];
    /** Unique per page: ties each heading to its section. */
    id: string;
    /** Each section's layout and spacing; the profile modal's details column spaces them itself. */
    sectionClass?: string;
    /** Under the card's name: h4 below the settings preview's h3, h3 below the modal's h2. */
    headingElement?: "h3" | "h4";
  }

  let { aboutMe, links, id, headingElement = "h4", sectionClass = "mt-(--mw-component-profile-popover-section-gap) flex flex-col gap-2xs" }: Props = $props();

  const aboutBlocks = $derived(parseInlineMarkdown(aboutMe));
  const shownLinks = $derived(
    links.flatMap((link) => {
      const url = safeHttpUrl(link.url);
      if (url === null) {
        return [];
      }
      const host = new URL(url).host;
      return [{ url, host, label: link.label?.trim() || null, iconId: link.icon_id ?? null }];
    }),
  );
  let failedIconIds = $state<ReadonlySet<string>>(new Set());
</script>

{#if aboutBlocks.length > 0}
  <section aria-labelledby="{id}-about" class={sectionClass}>
    <svelte:element this={headingElement} id="{id}-about" class="type-category text-(--mw-component-profile-popover-section-label-text)">
      {strings.profileDetails.aboutMe}
    </svelte:element>
    <div class="type-body break-words text-(--mw-component-profile-popover-about-text)">
      <MessageMarkdown blocks={aboutBlocks} />
    </div>
  </section>
{/if}
{#if shownLinks.length > 0}
  <section aria-labelledby="{id}-links" class={sectionClass}>
    <svelte:element this={headingElement} id="{id}-links" class="type-category text-(--mw-component-profile-popover-section-label-text)">
      {strings.profileDetails.links}
    </svelte:element>
    <ul class="flex flex-col">
      {#each shownLinks as link, index (index)}
        <li>
          <a
            href={link.url}
            target="_blank"
            rel="noopener noreferrer nofollow ugc"
            title={link.url}
            class="group flex min-h-(--mw-component-profile-popover-link-row-height) w-fit max-w-full items-center gap-(--mw-component-profile-popover-link-gap) rounded-xs type-body text-(--mw-component-profile-popover-link-text) focus-visible:focus-ring pointer-coarse:min-h-(--mw-size-hit-web-min)"
          >
            {#if link.iconId && !failedIconIds.has(link.iconId)}
              {@const iconId = link.iconId}
              <!-- On a light plate: many sites draw dark marks on transparency. -->
              <span
                class="grid size-(--mw-component-profile-popover-link-icon-plate-size) shrink-0 place-items-center rounded-(--mw-component-profile-popover-link-icon-radius) bg-(--mw-component-profile-popover-link-icon-plate)"
              >
                <img
                  src={linkIconUrl(iconId)}
                  alt=""
                  loading="lazy"
                  decoding="async"
                  draggable="false"
                  width="64"
                  height="64"
                  class="size-(--mw-component-profile-popover-link-icon-size) object-contain"
                  onerror={() => (failedIconIds = new Set([...failedIconIds, iconId]))}
                />
              </span>
            {:else}
              <span class="grid size-(--mw-component-profile-popover-link-icon-plate-size) shrink-0 place-items-center">
                <Icon
                  name="link-simple"
                  class="size-(--mw-component-profile-popover-link-icon-size) text-(--mw-component-profile-popover-link-icon-fallback)"
                />
              </span>
            {/if}
            <span class="min-w-0 truncate group-hover:underline">
              {link.label ? strings.profileDetails.linkLabel(link.label, link.host) : link.host}
            </span>
            <span class="sr-only">({strings.profileDetails.opensInNewTab})</span>
          </a>
        </li>
      {/each}
    </ul>
  </section>
{/if}
