<script lang="ts">
  import { tick } from "svelte";

  import { type MellowClient, describeError } from "../lib/client.svelte";
  import AccountEditForm from "./AccountEditForm.svelte";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";
  import TextField from "./TextField.svelte";

  // User settings, My account (DESIGN.md §5.9): Discord's account card with display name, username
  // and email, each editable field opening its form in place, then the password section. One form
  // is open at a time, so an error always sits under the form that raised it.
  interface Props {
    client: MellowClient;
  }

  let { client }: Props = $props();

  type AccountForm = "display-name" | "username" | "password";

  // Mirrors the server's rule (mellow-server domain/account_rules.rs); the server still decides.
  const MINIMUM_PASSWORD_LENGTH = 10;

  const id = $props.id();
  const user = $derived(client.user);

  let openForm = $state<AccountForm | null>(null);
  let isBusy = $state(false);
  let errorMessage = $state("");
  let notice = $state<{ form: AccountForm; text: string } | null>(null);
  let displayNameDraft = $state("");
  let usernameDraft = $state("");
  let currentPasswordDraft = $state("");
  let newPasswordDraft = $state("");
  let confirmPasswordDraft = $state("");
  let firstField = $state<HTMLInputElement | null>(null);

  const canSaveDisplayName = $derived(
    displayNameDraft.trim() !== "" && displayNameDraft.trim() !== user?.display_name,
  );
  const canSaveUsername = $derived(
    usernameDraft.trim() !== "" && usernameDraft.trim() !== user?.username && currentPasswordDraft !== "",
  );
  const doesConfirmationMatch = $derived(confirmPasswordDraft === newPasswordDraft);
  const canChangePassword = $derived(
    currentPasswordDraft !== "" &&
      [...newPasswordDraft].length >= MINIMUM_PASSWORD_LENGTH &&
      confirmPasswordDraft !== "" &&
      doesConfirmationMatch,
  );

  /** The button that opens a form, and takes focus back when it closes. */
  function openerId(form: AccountForm): string {
    return `${id}-open-${form}`;
  }

  function clearDrafts() {
    displayNameDraft = "";
    usernameDraft = "";
    currentPasswordDraft = "";
    newPasswordDraft = "";
    confirmPasswordDraft = "";
  }

  async function open(form: AccountForm) {
    clearDrafts();
    // Prefilled, as Discord does: the change is usually small.
    if (form === "display-name") {
      displayNameDraft = user?.display_name ?? "";
    } else if (form === "username") {
      usernameDraft = user?.username ?? "";
    }
    errorMessage = "";
    notice = null;
    openForm = form;
    await tick();
    firstField?.focus();
  }

  /** Typed passwords are not kept once their form closes. */
  async function close() {
    const closedForm = openForm;
    openForm = null;
    errorMessage = "";
    clearDrafts();
    await tick();
    if (closedForm) {
      document.getElementById(openerId(closedForm))?.focus();
    }
  }

  /** Esc closes the open form before the settings (true when there was one). */
  export function closeOpenForm(): boolean {
    if (openForm === null) {
      return false;
    }
    if (!isBusy) {
      void close();
    }
    return true;
  }

  async function save(form: AccountForm, run: () => Promise<void>, failureSummary: string, successText: string) {
    if (isBusy) {
      return;
    }
    isBusy = true;
    errorMessage = "";
    try {
      await run();
      notice = { form, text: successText };
      await close();
    } catch (error) {
      errorMessage = describeError(error, failureSummary);
    } finally {
      isBusy = false;
    }
  }

  const saveDisplayName = () =>
    save(
      "display-name",
      () => client.updateDisplayName(displayNameDraft),
      "Couldn't save the display name",
      "Display name saved.",
    );
  const saveUsername = () =>
    save(
      "username",
      () => client.updateUsername(usernameDraft, currentPasswordDraft),
      "Couldn't change the username",
      "Username saved.",
    );
  const changePassword = () =>
    save(
      "password",
      () => client.changePassword(currentPasswordDraft, newPasswordDraft),
      "Couldn't change the password",
      "Password changed. Your other devices are signed out.",
    );
</script>

{#snippet fieldValue(label: string, value: string)}
  <div class="flex min-w-0 flex-1 flex-col gap-(--mw-component-settings-account-value-gap)">
    <h3 class="type-category text-(--mw-component-settings-section-label-text)">{label}</h3>
    <p class="type-body-lg break-words text-(--mw-component-settings-account-value-text)">{value}</p>
  </div>
{/snippet}

{#snippet editableField(form: AccountForm, label: string, value: string)}
  <div class="flex flex-col gap-(--mw-component-settings-section-label-gap)">
    <div class="flex items-center gap-(--mw-component-settings-account-button-gap)">
      {@render fieldValue(label, value)}
      <Button id={openerId(form)} aria-label="Edit {label.toLowerCase()}" isDisabled={isBusy} onclick={() => void open(form)}
        >Edit</Button
      >
    </div>
    {@render savedNotice(form)}
  </div>
{/snippet}

{#snippet savedNotice(form: AccountForm)}
  {#if notice?.form === form}
    <!-- Announced through the status region below, so it is not read twice. -->
    <p
      aria-hidden="true"
      class="flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-settings-notice-text)"
    >
      <Icon name="check" class="size-(--mw-component-settings-error-icon-size)" />
      <span>{notice.text}</span>
    </p>
  {/if}
{/snippet}

<h2 class="type-heading-lg text-(--mw-component-settings-page-title-text)">My account</h2>

<div
  class="mt-(--mw-component-settings-page-title-gap) flex flex-col gap-(--mw-component-settings-account-row-gap) rounded-(--mw-component-settings-account-card-radius) bg-(--mw-component-settings-account-card-background) p-(--mw-component-settings-account-card-padding)"
>
  {#if openForm === "display-name"}
    <AccountEditForm
      label="Change display name"
      saveLabel="Save"
      busyLabel="Saving…"
      isSaveEnabled={canSaveDisplayName}
      {isBusy}
      {errorMessage}
      onsave={() => void saveDisplayName()}
      oncancel={() => void close()}
    >
      <TextField
        id="{id}-display-name"
        label="Display name"
        helper="Up to 32 characters. This is the name people see in chat."
        autocomplete="nickname"
        maxlength={32}
        readonly={isBusy}
        bind:value={displayNameDraft}
        bind:element={firstField}
      />
    </AccountEditForm>
  {:else}
    {@render editableField("display-name", "Display name", user?.display_name ?? "")}
  {/if}

  {#if openForm === "username"}
    <AccountEditForm
      label="Change username"
      saveLabel="Save"
      busyLabel="Saving…"
      isSaveEnabled={canSaveUsername}
      {isBusy}
      {errorMessage}
      onsave={() => void saveUsername()}
      oncancel={() => void close()}
    >
      <TextField
        id="{id}-username"
        label="Username"
        helper="2 to 32 characters: lowercase letters, digits, dots and underscores."
        autocomplete="username"
        autocapitalize="none"
        spellcheck={false}
        maxlength={32}
        readonly={isBusy}
        bind:value={usernameDraft}
        bind:element={firstField}
      />
      <TextField
        id="{id}-username-password"
        label="Current password"
        isPassword
        autocomplete="current-password"
        readonly={isBusy}
        bind:value={currentPasswordDraft}
      />
    </AccountEditForm>
  {:else}
    {@render editableField("username", "Username", `@${user?.username ?? ""}`)}
  {/if}

  {@render fieldValue("Email", user?.email ?? "")}
</div>

<h3 class="mt-(--mw-component-settings-section-gap) type-category text-(--mw-component-settings-section-label-text)">
  Password
</h3>
<div class="mt-(--mw-component-settings-section-label-gap)">
  {#if openForm === "password"}
    <div
      class="rounded-(--mw-component-settings-account-card-radius) bg-(--mw-component-settings-account-card-background) p-(--mw-component-settings-account-card-padding)"
    >
      <AccountEditForm
        label="Change password"
        saveLabel="Change password"
        busyLabel="Changing password…"
        isSaveEnabled={canChangePassword}
        {isBusy}
        {errorMessage}
        onsave={() => void changePassword()}
        oncancel={() => void close()}
      >
        <TextField
          id="{id}-current-password"
          label="Current password"
          isPassword
          autocomplete="current-password"
          readonly={isBusy}
          bind:value={currentPasswordDraft}
          bind:element={firstField}
        />
        <TextField
          id="{id}-new-password"
          label="New password"
          helper="At least {MINIMUM_PASSWORD_LENGTH} characters."
          isPassword
          autocomplete="new-password"
          readonly={isBusy}
          bind:value={newPasswordDraft}
        />
        <!-- Says why Save stays disabled while the two differ. -->
        <TextField
          id="{id}-confirm-password"
          label="Confirm new password"
          helper={confirmPasswordDraft !== "" && !doesConfirmationMatch ? "Doesn't match the new password." : ""}
          isPassword
          autocomplete="new-password"
          readonly={isBusy}
          bind:value={confirmPasswordDraft}
        />
      </AccountEditForm>
    </div>
  {:else}
    <div class="flex flex-col items-start gap-(--mw-component-settings-action-gap)">
      <Button id={openerId("password")} variant="primary" isDisabled={isBusy} onclick={() => void open("password")}
        >Change password</Button
      >
      {@render savedNotice("password")}
    </div>
  {/if}
</div>

<!-- Exists from the start, so its first change is announced. -->
<p role="status" class="sr-only">{notice?.text ?? ""}</p>
