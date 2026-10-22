<script lang="ts">
  /** Usernames are ASCII, so the browser's UTF-16 count matches the server's (account rules). */
  const MAXIMUM_USERNAME_LENGTH = 32;
  import { tick } from "svelte";

  import { ApiError } from "../lib/api";
  import { strings } from "../i18n";
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
      // No login was typed here: only the current password can be wrong.
      errorMessage =
        error instanceof ApiError && error.code === "INVALID_CREDENTIALS"
          ? strings.errors.withReason(failureSummary, strings.account.wrongCurrentPassword)
          : describeError(error, failureSummary);
    } finally {
      isBusy = false;
    }
  }

  const saveDisplayName = () =>
    save(
      "display-name",
      () => client.updateDisplayName(displayNameDraft),
      strings.account.displayNameFailed,
      strings.account.displayNameSaved,
    );
  const saveUsername = () =>
    save(
      "username",
      () => client.updateUsername(usernameDraft, currentPasswordDraft),
      strings.account.usernameFailed,
      strings.account.usernameSaved,
    );
  const changePassword = () =>
    save(
      "password",
      () => client.changePassword(currentPasswordDraft, newPasswordDraft),
      strings.account.passwordFailed,
      strings.account.passwordChanged,
    );
</script>

{#snippet fieldValue(label: string, value: string)}
  <div class="flex min-w-0 flex-1 flex-col gap-(--mw-component-settings-account-value-gap)">
    <h3 class="type-category text-(--mw-component-settings-section-label-text)">{label}</h3>
    <p class="type-body-lg break-words text-(--mw-component-settings-account-value-text)">{value}</p>
  </div>
{/snippet}

{#snippet editableField(form: AccountForm, label: string, editLabel: string, value: string)}
  <div class="flex flex-col gap-(--mw-component-settings-section-label-gap)">
    <div class="flex items-center gap-(--mw-component-settings-account-button-gap)">
      {@render fieldValue(label, value)}
      <Button id={openerId(form)} aria-label={editLabel} isDisabled={isBusy} onclick={() => void open(form)}
        >{strings.common.edit}</Button
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

<h2 class="type-heading-lg text-(--mw-component-settings-page-title-text)">{strings.settings.myAccount}</h2>

<div
  class="mt-(--mw-component-settings-page-title-gap) flex flex-col gap-(--mw-component-settings-account-row-gap) rounded-(--mw-component-settings-account-card-radius) bg-(--mw-component-settings-account-card-background) p-(--mw-component-settings-account-card-padding)"
>
  {#if openForm === "display-name"}
    <AccountEditForm
      label={strings.account.changeDisplayName}
      saveLabel={strings.common.save}
      busyLabel={strings.common.saving}
      isSaveEnabled={canSaveDisplayName}
      {isBusy}
      {errorMessage}
      onsave={() => void saveDisplayName()}
      oncancel={() => void close()}
    >
      <TextField
        id="{id}-display-name"
        label={strings.account.displayName}
        helper={strings.account.displayNameHelper}
        autocomplete="nickname"
        readonly={isBusy}
        bind:value={displayNameDraft}
        bind:element={firstField}
      />
    </AccountEditForm>
  {:else}
    {@render editableField("display-name", strings.account.displayName, strings.account.editDisplayName, user?.display_name ?? "")}
  {/if}

  {#if openForm === "username"}
    <AccountEditForm
      label={strings.account.changeUsername}
      saveLabel={strings.common.save}
      busyLabel={strings.common.saving}
      isSaveEnabled={canSaveUsername}
      {isBusy}
      {errorMessage}
      onsave={() => void saveUsername()}
      oncancel={() => void close()}
    >
      <TextField
        id="{id}-username"
        label={strings.account.username}
        helper={strings.signIn.usernameHelper}
        autocomplete="username"
        autocapitalize="none"
        spellcheck={false}
        maxlength={MAXIMUM_USERNAME_LENGTH}
        readonly={isBusy}
        bind:value={usernameDraft}
        bind:element={firstField}
      />
      <TextField
        id="{id}-username-password"
        label={strings.account.currentPassword}
        isPassword
        autocomplete="current-password"
        readonly={isBusy}
        bind:value={currentPasswordDraft}
      />
    </AccountEditForm>
  {:else}
    {@render editableField("username", strings.account.username, strings.account.editUsername, `@${user?.username ?? ""}`)}
  {/if}

  {@render fieldValue(strings.account.email, user?.email ?? "")}
</div>

<h3 class="mt-(--mw-component-settings-section-gap) type-category text-(--mw-component-settings-section-label-text)">
  {strings.account.password}
</h3>
<div class="mt-(--mw-component-settings-section-label-gap)">
  {#if openForm === "password"}
    <div
      class="rounded-(--mw-component-settings-account-card-radius) bg-(--mw-component-settings-account-card-background) p-(--mw-component-settings-account-card-padding)"
    >
      <AccountEditForm
        label={strings.account.changePassword}
        saveLabel={strings.account.changePassword}
        busyLabel={strings.account.changingPassword}
        isSaveEnabled={canChangePassword}
        {isBusy}
        {errorMessage}
        onsave={() => void changePassword()}
        oncancel={() => void close()}
      >
        <TextField
          id="{id}-current-password"
          label={strings.account.currentPassword}
          isPassword
          autocomplete="current-password"
          readonly={isBusy}
          bind:value={currentPasswordDraft}
          bind:element={firstField}
        />
        <TextField
          id="{id}-new-password"
          label={strings.account.newPassword}
          helper={strings.signIn.passwordLengthHelper(MINIMUM_PASSWORD_LENGTH)}
          isPassword
          autocomplete="new-password"
          readonly={isBusy}
          bind:value={newPasswordDraft}
        />
        <!-- Says why Save stays disabled while the two differ. -->
        <TextField
          id="{id}-confirm-password"
          label={strings.account.confirmNewPassword}
          helper={confirmPasswordDraft !== "" && !doesConfirmationMatch ? strings.account.passwordsDiffer : ""}
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
        >{strings.account.changePassword}</Button
      >
      {@render savedNotice("password")}
    </div>
  {/if}
</div>

<!-- Exists from the start, so its first change is announced. -->
<p role="status" class="sr-only">{notice?.text ?? ""}</p>
