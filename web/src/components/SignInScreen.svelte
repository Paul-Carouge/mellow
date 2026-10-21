<script lang="ts">
  import { onMount, tick } from "svelte";

  import { type MellowClient, describeError } from "../lib/client.svelte";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";
  import TextField from "./TextField.svelte";

  // Discord's authentication card (DESIGN.md §5.1). No instance address: the web client talks to
  // the instance that served it, named in the subtitle.
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
  let form: HTMLFormElement;

  /** The first field the person still has to fill, or the first one. */
  function focusFirstFieldToFill() {
    const fields = [...form.querySelectorAll<HTMLInputElement>("input:not([type=hidden])")];
    (fields.find((field) => field.value === "") ?? fields[0])?.focus();
  }

  onMount(focusFirstFieldToFill);

  async function switchMode() {
    isRegistering = !isRegistering;
    errorMessage = "";
    await tick();
    focusFirstFieldToFill();
  }

  async function submit() {
    if (isBusy) {
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
      errorMessage = describeError(error, isRegistering ? "Couldn't create your account" : "Couldn't sign in");
      isBusy = false;
      await tick();
      focusFirstFieldToFill();
    }
  }
</script>

<main class="grid min-h-dvh place-items-center overflow-y-auto bg-(--mw-component-auth-card-backdrop) p-(--mw-component-auth-card-margin)">
  <form
    bind:this={form}
    class="flex w-full max-w-(--mw-component-auth-card-max-width) flex-col gap-(--mw-component-auth-card-section-gap) rounded-(--mw-component-auth-card-radius) bg-(--mw-component-auth-card-background) p-(--mw-component-auth-card-padding) shadow-high"
    aria-labelledby="sign-in-title"
    onsubmit={(event) => {
      event.preventDefault();
      void submit();
    }}
  >
    <div class="flex flex-col gap-(--mw-component-auth-card-subtitle-gap) text-center">
      <h1 id="sign-in-title" class="type-heading-xl text-(--mw-component-auth-card-title-text)">
        {isRegistering ? "Create an account" : "Welcome back"}
      </h1>
      <p class="type-body-lg break-words text-(--mw-component-auth-card-subtitle-text)">
        {isRegistering ? `Your account will live on ${instanceName}.` : `Sign in to ${instanceName}.`}
      </p>
    </div>

    <div class="flex flex-col gap-(--mw-component-text-field-field-gap)">
      {#if isRegistering}
        <TextField id="register-email" label="Email" type="email" autocomplete="email" required bind:value={email} />
        <TextField
          id="register-username"
          label="Username"
          autocomplete="username"
          autocapitalize="none"
          spellcheck={false}
          required
          helper="2 to 32 characters: lowercase letters, digits, dots and underscores."
          bind:value={username}
        />
        <TextField
          id="register-password"
          label="Password"
          isPassword
          autocomplete="new-password"
          required
          helper="At least 10 characters."
          bind:value={password}
        />
      {:else}
        <TextField
          id="sign-in-login"
          label="Username or email"
          autocomplete="username"
          autocapitalize="none"
          spellcheck={false}
          required
          bind:value={login}
        />
        <TextField
          id="sign-in-password"
          label="Password"
          isPassword
          autocomplete="current-password"
          required
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
          {isBusy ? "Creating account…" : "Create account"}
        {:else}
          {isBusy ? "Signing in…" : "Sign in"}
        {/if}
      </Button>
      <p class="flex flex-wrap items-center gap-x-2xs type-body text-(--mw-component-auth-card-hint-text)">
        {isRegistering ? "Already have an account?" : "Need an account?"}
        <button
          type="button"
          class="min-h-(--mw-size-hit-web-min) font-medium text-(--mw-component-auth-card-link-text) hover:underline"
          onclick={switchMode}>{isRegistering ? "Sign in" : "Register"}</button
        >
      </p>
    </div>
  </form>
</main>
