<script lang="ts">
  import { onMount, tick } from "svelte";

  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import { MINIMUM_PASSWORD_LENGTH } from "../lib/protocol";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";
  import Melli from "./Melli.svelte";
  import { fieldErrorsBeforeSending } from "./sign-in-form";
  import TextField from "./TextField.svelte";

  // The authentication card (DESIGN.md §5.1), with Melli standing on its edge. No instance
  // address: the web client talks to the instance that served it, named in the title.
  interface Props {
    client: MellowClient;
  }

  let { client }: Props = $props();

  const instanceName = window.location.host;
  let isRegistering = $state(false);
  let login = $state("");
  let email = $state("");
  let username = $state("");
  let password = $state("");
  let isBusy = $state(false);
  let errorMessage = $state("");
  /** Field id to Mellow's own error: the form has `novalidate`, so no browser bubble. */
  let fieldErrors = $state(new Map<string, string>());
  let form: HTMLFormElement;
  let emailInput: HTMLInputElement | null = $state(null);

  // Melli's drawing (viewBox units, Melli.svelte): the cloud spans 164 of its 256 rows and ends at
  // row 206, its shadow at rows 215 to 229. The card's edge passes at row 210, between the two, so
  // the whole cloud stands above the card at the token's size and its shadow falls on the card.
  const MELLI_DRAWING_SIZE = 256;
  const MELLI_CLOUD_HEIGHT = 164;
  const CARD_EDGE_IN_MELLI_DRAWING = 210;

  /** The first field the person still has to fill, or the first one. */
  function focusFirstFieldToFill() {
    const fields = [...form.querySelectorAll<HTMLInputElement>("input:not([type=hidden])")];
    (fields.find((field) => field.value === "") ?? fields[0])?.focus();
  }

  onMount(focusFirstFieldToFill);

  async function switchMode() {
    isRegistering = !isRegistering;
    errorMessage = "";
    fieldErrors = new Map();
    await tick();
    focusFirstFieldToFill();
  }

  function forgetFieldError(id: string) {
    if (fieldErrors.has(id)) {
      fieldErrors = new Map([...fieldErrors].filter(([fieldId]) => fieldId !== id));
    }
  }

  async function submit() {
    if (isBusy) {
      return;
    }
    fieldErrors = fieldErrorsBeforeSending(
      isRegistering
        ? [
            {
              id: "register-email",
              value: email.trim(),
              emptyError: strings.signIn.emailMissing,
              isMalformed: emailInput?.validity.typeMismatch ?? false,
              malformedError: strings.signIn.emailMalformed,
            },
            { id: "register-username", value: username.trim(), emptyError: strings.signIn.usernameMissing },
            { id: "register-password", value: password, emptyError: strings.signIn.newPasswordMissing },
          ]
        : [
            { id: "sign-in-login", value: login.trim(), emptyError: strings.signIn.loginMissing },
            { id: "sign-in-password", value: password, emptyError: strings.signIn.passwordMissing },
          ],
    );
    const firstFieldToFix = fieldErrors.keys().next().value;
    if (firstFieldToFix !== undefined) {
      errorMessage = "";
      await tick();
      // Focus reads the field's label and its error (`aria-describedby`).
      document.getElementById(firstFieldToFix)?.focus();
      return;
    }
    isBusy = true;
    errorMessage = "";
    try {
      if (isRegistering) {
        await client.register(username.trim(), email.trim(), password);
      } else {
        await client.signIn(login.trim(), password);
      }
    } catch (error) {
      errorMessage = describeError(error, isRegistering ? strings.signIn.registerFailed : strings.signIn.signInFailed);
      isBusy = false;
      await tick();
      focusFirstFieldToFill();
    }
  }
</script>

<main class="grid min-h-dvh place-items-center overflow-y-auto bg-(--mw-component-auth-card-backdrop) p-(--mw-component-auth-card-margin)">
  <form
    bind:this={form}
    novalidate
    class="relative mt-(--sign-in-melli-above-card) flex w-full max-w-(--mw-component-auth-card-max-width) flex-col gap-(--mw-component-auth-card-section-gap) rounded-(--mw-component-auth-card-radius) bg-(--mw-component-auth-card-background) p-(--mw-component-auth-card-padding) shadow-high"
    style:--sign-in-melli-box="calc(var(--mw-component-auth-card-melli-size) * {MELLI_DRAWING_SIZE / MELLI_CLOUD_HEIGHT})"
    style:--sign-in-melli-above-card="calc(var(--sign-in-melli-box) * {CARD_EDGE_IN_MELLI_DRAWING / MELLI_DRAWING_SIZE})"
    aria-labelledby="sign-in-title"
    onsubmit={(event) => {
      event.preventDefault();
      void submit();
    }}
  >
    <!-- The screen's hero (DESIGN.md §2.10, §5.1): the cloud above the edge, its shadow on the card;
         it looks up at its rising dots while the form is sent. -->
    <span class="absolute top-0 left-1/2 size-(--sign-in-melli-box) -translate-x-1/2 -translate-y-(--sign-in-melli-above-card)">
      <Melli state={isBusy ? "connecting" : "idle"} sizeToken="--sign-in-melli-box" />
    </span>
    <div class="flex flex-col gap-(--mw-component-auth-card-subtitle-gap) text-center">
      <h1 id="sign-in-title" class="type-heading-xl break-words text-(--mw-component-auth-card-title-text)">
        {isRegistering ? strings.signIn.registerTitle(instanceName) : strings.signIn.signInTitle(instanceName)}
      </h1>
      {#if client.hasSessionEndedByItself && !isRegistering}
        <!-- An alert, so it is read when the card replaces the shell under the reader. -->
        <p role="alert" class="type-body-lg text-(--mw-component-auth-card-subtitle-text)">{strings.signIn.sessionEnded(instanceName)}</p>
      {:else}
        <p class="type-body-lg text-(--mw-component-auth-card-subtitle-text)">
          {isRegistering ? strings.signIn.registerSubtitle : strings.signIn.signInSubtitle}
        </p>
      {/if}
    </div>

    <div class="flex flex-col gap-(--mw-component-text-field-field-gap)">
      {#if isRegistering}
        <TextField
          id="register-email"
          label={strings.signIn.email}
          type="email"
          autocomplete="email"
          error={fieldErrors.get("register-email")}
          oninput={() => forgetFieldError("register-email")}
          bind:element={emailInput}
          bind:value={email}
        />
        <TextField
          id="register-username"
          label={strings.signIn.username}
          autocomplete="username"
          autocapitalize="none"
          spellcheck={false}
          helper={strings.signIn.usernameHelper}
          error={fieldErrors.get("register-username")}
          oninput={() => forgetFieldError("register-username")}
          bind:value={username}
        />
        <TextField
          id="register-password"
          label={strings.signIn.password}
          isPassword
          autocomplete="new-password"
          helper={strings.signIn.passwordLengthHelper(MINIMUM_PASSWORD_LENGTH)}
          error={fieldErrors.get("register-password")}
          oninput={() => forgetFieldError("register-password")}
          bind:value={password}
        />
      {:else}
        <TextField
          id="sign-in-login"
          label={strings.signIn.login}
          autocomplete="username"
          autocapitalize="none"
          spellcheck={false}
          error={fieldErrors.get("sign-in-login")}
          oninput={() => forgetFieldError("sign-in-login")}
          bind:value={login}
        />
        <TextField
          id="sign-in-password"
          label={strings.signIn.password}
          isPassword
          autocomplete="current-password"
          error={fieldErrors.get("sign-in-password")}
          oninput={() => forgetFieldError("sign-in-password")}
          bind:value={password}
        />
      {/if}
    </div>

    <div class="flex flex-col gap-xs">
      <!-- Present from the start, so the error is announced when it appears. -->
      <div role="alert" class="contents">
        {#if errorMessage}
          <p class="flex gap-xs type-body text-(--mw-component-auth-card-error-text)">
            <Icon name="warning-circle" class="size-(--mw-component-auth-card-error-icon-size)" />
            <span>{errorMessage}</span>
          </p>
        {/if}
      </div>
      <Button type="submit" variant="primary" isFullWidth isDisabled={isBusy}>
        {#if isRegistering}
          {isBusy ? strings.signIn.creatingAccount : strings.signIn.createAccount}
        {:else}
          {isBusy ? strings.signIn.signingIn : strings.signIn.signIn}
        {/if}
      </Button>
    </div>
    <p class="flex flex-wrap items-center justify-center gap-x-2xs type-body text-(--mw-component-auth-card-hint-text)">
      {isRegistering ? strings.signIn.hasAccount : strings.signIn.hasNoAccount}
      <button
        type="button"
        class="text-(--mw-component-auth-card-link-text) hover:underline pointer-coarse:min-h-(--mw-size-hit-web-min) pointer-coarse:min-w-(--mw-size-hit-web-min)"
        onclick={switchMode}>{isRegistering ? strings.signIn.signIn : strings.signIn.createAnAccount}</button
      >
    </p>
  </form>
</main>
