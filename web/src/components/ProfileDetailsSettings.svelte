<script lang="ts">
  import { onMount, tick } from "svelte";

  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import {
    MAXIMUM_ABOUT_ME_LENGTH,
    MAXIMUM_PROFILE_LINK_LABEL_LENGTH,
    MAXIMUM_PROFILE_LINKS,
    type ProfileLink,
  } from "../lib/protocol";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import {
    EMPTY_PROFILE_DRAFT,
    type ProfileDraft,
    canSaveProfileDraft,
    characterCount,
    isAboutMeTooLong,
    linkProblem,
    profileDraftOf,
    profileRequestOf,
  } from "./profile-details-form";
  import TextField from "./TextField.svelte";

  // User settings, Profile: "About me" and links (ADR-0050, DESIGN.md §5.9), under the picture
  // editors. Saved with its own button, since a text is typed in several goes; the preview card
  // beside shows the draft (`draft`, bound by the page). Links' site icons are fetched by the
  // instance after a save, so a new link shows the link glyph until then.
  interface Props {
    client: MellowClient;
    /** The draft as typed, for the preview. */
    draft: ProfileDraft;
    /** The links as saved, whose icons the preview reuses. */
    savedLinks: readonly ProfileLink[];
  }

  let { client, draft = $bindable(), savedLinks = $bindable() }: Props = $props();

  const id = $props.id();
  let phase = $state<"loading" | "ready" | "load-failed">("loading");
  let isSaving = $state(false);
  let errorMessage = $state("");
  let notice = $state("");
  let saved = $state<ProfileDraft>(EMPTY_PROFILE_DRAFT);

  const aboutMeCount = $derived(characterCount(draft.aboutMe.trim()));
  const isTooLong = $derived(isAboutMeTooLong(draft));
  const hasChanges = $derived(JSON.stringify(profileRequestOf(draft)) !== JSON.stringify(profileRequestOf(saved)));
  const canSave = $derived(phase === "ready" && hasChanges && canSaveProfileDraft(draft) && !isSaving);

  async function load() {
    const userId = client.user?.id;
    if (!userId) {
      return;
    }
    phase = "loading";
    try {
      const profile = await client.loadUserProfile(userId);
      saved = profileDraftOf(profile);
      draft = saved;
      savedLinks = profile.links;
      phase = "ready";
    } catch (error) {
      console.warn("loading the profile details failed", error);
      phase = "load-failed";
    }
  }

  onMount(() => {
    void load();
  });

  function changeLink(index: number, change: Partial<{ url: string; label: string }>) {
    draft = { ...draft, links: draft.links.map((link, position) => (position === index ? { ...link, ...change } : link)) };
    notice = "";
  }

  async function addLink() {
    draft = { ...draft, links: [...draft.links, { url: "", label: "" }] };
    await tick();
    document.getElementById(`${id}-link-${draft.links.length - 1}-url`)?.focus();
  }

  async function removeLink(index: number) {
    draft = { ...draft, links: draft.links.filter((_, position) => position !== index) };
    await tick();
    // Focus stays in the list: the row now in its place, else the one above, else "Add a link".
    const next = document.getElementById(`${id}-link-${Math.min(index, draft.links.length - 1)}-url`);
    (next ?? document.getElementById(`${id}-add-link`))?.focus();
  }

  async function save() {
    if (!canSave) {
      return;
    }
    isSaving = true;
    errorMessage = "";
    notice = "";
    try {
      const profile = await client.updateMyProfile(profileRequestOf(draft));
      saved = profileDraftOf(profile);
      draft = saved;
      savedLinks = profile.links;
      notice = strings.profileDetails.saved;
    } catch (error) {
      errorMessage = describeError(error, strings.profileDetails.saveFailed);
    } finally {
      isSaving = false;
    }
  }

  const problemText = { "not-https": strings.profileDetails.notHttps, "too-long": strings.profileDetails.tooLong } as const;
</script>

<section aria-labelledby="{id}-title" aria-busy={phase === "loading" || isSaving} class="flex flex-col gap-(--mw-component-settings-section-label-gap)">
  <h3 id="{id}-title" class="type-category text-(--mw-component-settings-section-label-text)">{strings.profileDetails.editorTitle}</h3>
  <p class="type-body text-(--mw-component-settings-body-text)">{strings.profileDetails.editorIntro}</p>

  {#if phase === "loading"}
    <p role="status" class="type-body text-(--mw-component-settings-body-text)">{strings.profileDetails.loading}</p>
  {:else if phase === "load-failed"}
    <div role="alert" class="flex flex-col items-start gap-(--mw-component-settings-section-label-gap)">
      <p class="flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-settings-error-text)">
        <Icon name="warning-circle" class="size-(--mw-component-settings-error-icon-size) shrink-0 text-(--mw-component-settings-error-icon)" />
        <span>{strings.profileDetails.loadEditorFailed}</span>
      </p>
      <Button onclick={() => void load()}>{strings.common.tryAgain}</Button>
    </div>
  {:else}
    <div class="mt-(--mw-component-settings-section-label-gap) flex flex-col gap-(--mw-component-text-field-label-gap)">
      <label for="{id}-about" class="type-category text-(--mw-component-text-field-label-text)">{strings.profileDetails.aboutMe}</label>
      <textarea
        id="{id}-about"
        rows="4"
        value={draft.aboutMe}
        aria-describedby="{id}-about-helper {id}-about-count"
        aria-invalid={isTooLong ? true : undefined}
        class={[
          "min-h-[calc(4*var(--mw-component-text-field-height)/2)] resize-y rounded-(--mw-component-text-field-radius) border-(length:--mw-component-text-field-border-width) bg-(--mw-component-text-field-background) px-(--mw-component-text-field-padding-inline) py-xs type-body text-(--mw-component-text-field-text) focus-visible:focus-ring",
          isTooLong ? "border-text-danger" : "border-(--mw-component-text-field-border)",
        ]}
        oninput={(event) => {
          draft = { ...draft, aboutMe: event.currentTarget.value };
          notice = "";
        }}
      ></textarea>
      <div class="flex flex-wrap justify-between gap-xs type-body-sm">
        <p id="{id}-about-helper" class="text-(--mw-component-text-field-helper-text)">
          {strings.profileDetails.aboutMeHelper(MAXIMUM_ABOUT_ME_LENGTH)}
        </p>
        <p
          id="{id}-about-count"
          class={["tabular-nums", isTooLong ? "text-(--mw-component-settings-error-text)" : "text-(--mw-component-text-field-helper-text)"]}
        >
          {strings.profileDetails.remaining(MAXIMUM_ABOUT_ME_LENGTH - aboutMeCount)}
        </p>
      </div>
    </div>

    <fieldset class="mt-(--mw-component-settings-section-label-gap) flex min-w-0 flex-col gap-(--mw-component-settings-section-label-gap)">
      <legend class="type-category text-(--mw-component-text-field-label-text)">{strings.profileDetails.links}</legend>
      <p id="{id}-links-helper" class="type-body-sm text-(--mw-component-text-field-helper-text)">
        {strings.profileDetails.linksHelper(MAXIMUM_PROFILE_LINKS)}
      </p>
      {#each draft.links as link, index (index)}
        {@const problem = linkProblem(link)}
        <div class="flex flex-wrap items-end gap-xs pb-xs">
          <div class="min-w-0 flex-[2_1_10rem]">
            <TextField
              id="{id}-link-{index}-url"
              label={strings.profileDetails.linkAddress(index + 1)}
              value={link.url}
              type="url"
              inputmode="url"
              autocomplete="url"
              placeholder="https://"
              leadingIcon="link-simple"
              error={problem ? problemText[problem] : ""}
              oninput={(event) => changeLink(index, { url: event.currentTarget.value })}
            />
          </div>
          <div class="min-w-0 flex-[1_1_7rem]">
            <TextField
              id="{id}-link-{index}-label"
              label={strings.profileDetails.linkName(index + 1)}
              value={link.label}
              maxlength={MAXIMUM_PROFILE_LINK_LABEL_LENGTH}
              placeholder={strings.profileDetails.linkNamePlaceholder}
              autocomplete="off"
              oninput={(event) => changeLink(index, { label: event.currentTarget.value })}
            />
          </div>
          <IconButton
            icon="trash"
            label={strings.profileDetails.removeLink(index + 1)}
            class="size-(--mw-component-text-field-height) shrink-0 text-(--mw-component-text-field-icon) hover:bg-state-hover hover:text-text-normal"
            iconClass="size-(--mw-component-text-field-icon-size)"
            onclick={() => void removeLink(index)}
          />
        </div>
      {/each}
      {#if draft.links.length < MAXIMUM_PROFILE_LINKS}
        <div>
          <Button id="{id}-add-link" variant="ghost" onclick={() => void addLink()}>
            <Icon name="plus" class="me-2xs size-(--mw-component-text-field-icon-size)" />{strings.profileDetails.addLink}
          </Button>
        </div>
      {/if}
    </fieldset>

    <div class="flex flex-wrap items-center gap-(--mw-component-settings-account-button-gap)">
      <Button isDisabled={!canSave} onclick={() => void save()}>
        {isSaving ? strings.profileDetails.saving : strings.profileDetails.save}
      </Button>
    </div>
    <div role="alert" class="contents">
      {#if errorMessage}
        <p class="flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-settings-error-text)">
          <Icon name="warning-circle" class="size-(--mw-component-settings-error-icon-size) shrink-0 text-(--mw-component-settings-error-icon)" />
          <span>{errorMessage}</span>
        </p>
      {/if}
    </div>
    {#if notice}
      <p aria-hidden="true" class="flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-settings-notice-text)">
        <Icon name="check" class="size-(--mw-component-settings-error-icon-size) shrink-0" />
        <span>{notice}</span>
      </p>
    {/if}
  {/if}
  <p role="status" class="sr-only">{notice}</p>
</section>
