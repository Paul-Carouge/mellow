<script lang="ts">
  import { strings } from "../i18n";
  import { choosePictureAnimation, pictureAnimationChoice, prefersReducedMotion } from "../lib/motion.svelte";
  import { PICTURE_ANIMATION_CHOICES } from "../lib/picture-animation";
  import ChoiceOption from "./ChoiceOption.svelte";

  // User settings → Accessibility (docs/design/web-animation-setting.md): when animated pictures
  // and Melli play, kept in this browser. While the system asks to reduce motion, a sentence says
  // what that does to "Animate on hover", so a still avatar is never a mystery.

  const id = $props.id();
  const choice = {
    get value(): string {
      return pictureAnimationChoice();
    },
    set value(picked: string) {
      choosePictureAnimation(PICTURE_ANIMATION_CHOICES.find((known) => known === picked) ?? "on-hover");
    },
  };
</script>

<h2 class="type-heading-lg text-(--mw-component-settings-page-title-text)">{strings.settings.accessibility}</h2>
<p class="mt-(--mw-component-settings-page-title-gap) type-body-lg text-(--mw-component-settings-body-text)">
  {strings.accessibilitySettings.intro}
</p>

<fieldset class="mt-(--mw-component-settings-section-gap) flex flex-col gap-(--mw-component-choice-list-gap)">
  <legend class="mb-(--mw-component-settings-section-label-gap) type-category text-(--mw-component-settings-section-label-text)">
    {strings.accessibilitySettings.animatedPictures}
  </legend>
  <ChoiceOption
    name="{id}-pictures"
    value="on-hover"
    bind:group={choice.value}
    label={strings.accessibilitySettings.animateOnHover}
    description={strings.accessibilitySettings.animateOnHoverDescription}
  />
  <ChoiceOption
    name="{id}-pictures"
    value="always"
    bind:group={choice.value}
    label={strings.accessibilitySettings.alwaysAnimate}
    description={strings.accessibilitySettings.alwaysAnimateDescription}
  />
  {#if prefersReducedMotion.current}
    <p class="mt-xs type-body text-(--mw-component-text-field-helper-text)">
      {strings.accessibilitySettings.reducedMotionNote}
    </p>
  {/if}
</fieldset>
