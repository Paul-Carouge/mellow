<script lang="ts">
  import { onMount, tick } from "svelte";

  import { strings } from "../i18n";
  import { ApiError } from "../lib/api";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { TwoFactorStatus } from "../lib/protocol";
  import { QR_QUIET_ZONE_MODULES, encodeQrCode, qrCodeSvgPath } from "../lib/qr-code";
  import AccountEditForm from "./AccountEditForm.svelte";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";
  import TextField from "./TextField.svelte";
  import { groupedSecret, isSixDigitCode, otpauthUri, recoveryCodesFileText } from "./two-factor";

  // User settings, Security (ADR-0049): two-factor sign-in in the account card's look. Off, a
  // short why and one button; turning it on asks for the password, then shows the QR code (drawn
  // here, the secret never leaves the browser) and the code that confirms it, then the recovery
  // codes, once. On, the codes left, new codes and Turn off, each behind the password and a code.
  interface Props {
    client: MellowClient;
  }

  let { client }: Props = $props();

  type Form = "password" | "scan" | "disable" | "regenerate";

  const id = $props.id();
  const host = window.location.host;
  let status = $state<TwoFactorStatus | null>(null);
  let hasLoadFailed = $state(false);
  let openForm = $state<Form | null>(null);
  let isBusy = $state(false);
  let errorMessage = $state("");
  let notice = $state("");
  let passwordDraft = $state("");
  let codeDraft = $state("");
  let secret = $state("");
  let recoveryCodes = $state<readonly string[] | null>(null);
  let copyNotice = $state("");
  let firstField = $state<HTMLInputElement | null>(null);
  let codesHeading = $state<HTMLElement | null>(null);

  const qrMatrix = $derived(secret ? encodeQrCode(otpauthUri(secret, client.user?.username ?? "", host)) : null);
  const qrViewBox = $derived(qrMatrix ? qrMatrix.length + 2 * QR_QUIET_ZONE_MODULES : 0);

  async function load() {
    hasLoadFailed = false;
    try {
      status = await client.rest.twoFactorStatus();
    } catch {
      hasLoadFailed = true;
    }
  }

  onMount(() => void load());

  function openerId(form: Form): string {
    return `${id}-open-${form}`;
  }

  /** Typed passwords and codes are not kept once their form closes. */
  function clearDrafts() {
    passwordDraft = "";
    codeDraft = "";
  }

  async function open(form: Form) {
    clearDrafts();
    errorMessage = "";
    notice = "";
    openForm = form;
    await tick();
    firstField?.focus();
  }

  async function close() {
    const closedForm = openForm;
    openForm = null;
    errorMessage = "";
    secret = "";
    copyNotice = "";
    clearDrafts();
    await tick();
    document.getElementById(openerId(closedForm === "scan" ? "password" : (closedForm ?? "password")))?.focus();
  }

  /** Esc closes the open form before the settings, and never skips the recovery codes. */
  export function closeOpenForm(): boolean {
    if (recoveryCodes !== null) {
      return true;
    }
    if (openForm === null) {
      return false;
    }
    if (!isBusy) {
      void close();
    }
    return true;
  }

  function failureText(error: unknown, summary: string): string {
    return error instanceof ApiError && error.code === "INVALID_CREDENTIALS"
      ? strings.errors.withReason(summary, strings.account.wrongCurrentPassword)
      : describeError(error, summary);
  }

  async function run(work: () => Promise<void>, failureSummary: string) {
    if (isBusy) {
      return;
    }
    isBusy = true;
    errorMessage = "";
    try {
      await work();
    } catch (error) {
      errorMessage = failureText(error, failureSummary);
    } finally {
      isBusy = false;
    }
  }

  async function showRecoveryCodes(codes: readonly string[]) {
    openForm = null;
    secret = "";
    clearDrafts();
    recoveryCodes = codes;
    await tick();
    codesHeading?.focus();
  }

  const startEnrolment = () =>
    run(async () => {
      const enrolment = await client.rest.startTwoFactorEnrolment(passwordDraft);
      passwordDraft = "";
      secret = enrolment.secret;
      openForm = "scan";
      await tick();
      firstField?.focus();
    }, strings.security.startFailed);

  const enable = () =>
    run(async () => {
      const answer = await client.rest.enableTwoFactor(codeDraft);
      status = { is_enabled: true, recovery_codes_remaining: answer.recovery_codes.length };
      notice = strings.security.turnedOn;
      await showRecoveryCodes(answer.recovery_codes);
    }, strings.security.enableFailed);

  const regenerate = () =>
    run(async () => {
      const answer = await client.rest.regenerateRecoveryCodes(passwordDraft, codeDraft);
      status = { is_enabled: true, recovery_codes_remaining: answer.recovery_codes.length };
      notice = strings.security.newCodesReady;
      await showRecoveryCodes(answer.recovery_codes);
    }, strings.security.regenerateFailed);

  const disable = () =>
    run(async () => {
      await client.rest.disableTwoFactor(passwordDraft, codeDraft);
      status = { is_enabled: false, recovery_codes_remaining: 0 };
      notice = strings.security.turnedOff;
      openForm = null;
      clearDrafts();
      await tick();
      document.getElementById(openerId("password"))?.focus();
    }, strings.security.disableFailed);

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      copyNotice = strings.security.copied;
    } catch {
      copyNotice = strings.security.copyFailed;
    }
  }

  function codesFile(): string {
    return recoveryCodesFileText(
      strings.security.codesFileHeading(client.user?.username ?? "", host),
      recoveryCodes ?? [],
    );
  }

  function download() {
    const url = URL.createObjectURL(new Blob([codesFile()], { type: "text/plain;charset=utf-8" }));
    const link = document.createElement("a");
    link.href = url;
    link.download = strings.security.codesFileName;
    link.click();
    URL.revokeObjectURL(url);
  }

  async function finishWithCodes() {
    recoveryCodes = null;
    copyNotice = "";
    await tick();
    document.getElementById(openerId("regenerate"))?.focus();
  }
</script>

{#snippet codeField(fieldId: string, isFirst: boolean)}
  <TextField
    id={fieldId}
    label={strings.security.codeFromApp}
    helper={openForm === "scan" ? "" : strings.security.codeHelper}
    autocomplete="one-time-code"
    autocapitalize="none"
    spellcheck={false}
    isMonospace
    readonly={isBusy}
    bind:value={codeDraft}
    bind:element={() => (isFirst ? firstField : null), (element) => {
      if (isFirst) {
        firstField = element;
      }
    }}
  />
{/snippet}

{#snippet passwordField(fieldId: string, helper: string)}
  <TextField
    id={fieldId}
    label={strings.account.currentPassword}
    {helper}
    isPassword
    autocomplete="current-password"
    readonly={isBusy}
    bind:value={passwordDraft}
    bind:element={firstField}
  />
{/snippet}

<h2 class="type-heading-lg text-(--mw-component-settings-page-title-text)">{strings.security.title}</h2>

<h3 class="mt-(--mw-component-settings-page-title-gap) type-category text-(--mw-component-settings-section-label-text)">
  {strings.security.twoFactorHeading}
</h3>
<div
  class="mt-(--mw-component-settings-section-label-gap) flex flex-col gap-(--mw-component-settings-action-gap) rounded-(--mw-component-settings-account-card-radius) bg-(--mw-component-settings-account-card-background) p-(--mw-component-settings-account-card-padding)"
>
  {#if hasLoadFailed}
    <p role="alert" class="flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-settings-error-text)">
      <Icon name="warning-circle" class="size-(--mw-component-settings-error-icon-size) text-(--mw-component-settings-error-icon)" />
      <span>{strings.security.loadFailed}</span>
    </p>
    <div><Button onclick={() => void load()}>{strings.security.tryAgain}</Button></div>
  {:else if status === null}
    <p aria-busy="true" class="type-body-lg text-(--mw-component-settings-body-text)">{strings.security.loading}</p>
  {:else if recoveryCodes !== null}
    <h4 bind:this={codesHeading} tabindex="-1" class="type-heading-md text-(--mw-component-settings-page-title-text) outline-none">
      {strings.security.codesTitle}
    </h4>
    <p class="type-body-lg text-(--mw-component-settings-body-text)">{strings.security.codesIntro}</p>
    <ol
      aria-label={strings.security.codesListLabel}
      class="grid grid-cols-1 gap-x-lg gap-y-2xs sm:grid-cols-2 type-code text-(--mw-component-settings-two-factor-code-text)"
    >
      {#each recoveryCodes as code (code)}
        <li class="select-all">{code}</li>
      {/each}
    </ol>
    <div class="flex flex-wrap items-center gap-(--mw-component-settings-account-button-gap)">
      <Button onclick={download}><Icon name="download-simple" class="me-2xs size-(--mw-component-settings-error-icon-size)" />{strings.security.download}</Button>
      <Button onclick={() => void copy((recoveryCodes ?? []).join("\n"))}
        ><Icon name="copy" class="me-2xs size-(--mw-component-settings-error-icon-size)" />{strings.security.copyAll}</Button
      >
      <Button variant="primary" onclick={() => void finishWithCodes()}>{strings.security.savedThem}</Button>
    </div>
    <p role="status" class="type-body text-(--mw-component-settings-notice-text)">{copyNotice}</p>
  {:else if openForm === "password"}
    <AccountEditForm
      label={strings.security.passwordStepTitle}
      saveLabel={strings.security.continue}
      busyLabel={strings.security.continuing}
      isSaveEnabled={passwordDraft !== ""}
      {isBusy}
      {errorMessage}
      onsave={() => void startEnrolment()}
      oncancel={() => void close()}
    >
      {@render passwordField(`${id}-enrol-password`, strings.security.passwordStepHelper)}
    </AccountEditForm>
  {:else if openForm === "scan"}
    <h4 class="type-heading-md text-(--mw-component-settings-page-title-text)">{strings.security.scanTitle}</h4>
    <p class="type-body-lg text-(--mw-component-settings-body-text)">{strings.security.scanStep}</p>
    {#if qrMatrix}
      <svg
        role="img"
        aria-label={strings.security.qrCodeLabel}
        viewBox="0 0 {qrViewBox} {qrViewBox}"
        shape-rendering="crispEdges"
        class="size-(--mw-component-settings-two-factor-qr-size) rounded-(--mw-component-settings-two-factor-qr-radius) bg-(--mw-component-settings-two-factor-qr-background) fill-(--mw-component-settings-two-factor-qr-module)"
      >
        <path d={qrCodeSvgPath(qrMatrix)} />
      </svg>
    {/if}
    <div class="flex flex-col gap-(--mw-component-settings-section-label-gap)">
      <p id="{id}-manual-key-label" class="type-category text-(--mw-component-settings-section-label-text)">
        {strings.security.manualKeyLabel}
      </p>
      <div class="flex flex-wrap items-center gap-(--mw-component-settings-account-button-gap)">
        <code
          aria-labelledby="{id}-manual-key-label"
          class="select-all break-all type-code text-(--mw-component-settings-two-factor-code-text)">{groupedSecret(secret)}</code
        >
        <Button onclick={() => void copy(secret)}>{strings.security.copyKey}</Button>
      </div>
      <p role="status" class="type-body-sm text-(--mw-component-settings-notice-text)">{copyNotice}</p>
    </div>
    <p class="type-body-lg text-(--mw-component-settings-body-text)">{strings.security.confirmStep}</p>
    <AccountEditForm
      label={strings.security.scanTitle}
      saveLabel={strings.security.turnOnButton}
      busyLabel={strings.security.turningOn}
      isSaveEnabled={isSixDigitCode(codeDraft)}
      {isBusy}
      {errorMessage}
      onsave={() => void enable()}
      oncancel={() => void close()}
    >
      {@render codeField(`${id}-enable-code`, true)}
    </AccountEditForm>
  {:else if openForm === "disable" || openForm === "regenerate"}
    {@const isDisabling = openForm === "disable"}
    <AccountEditForm
      label={isDisabling ? strings.security.turnOffTitle : strings.security.newRecoveryCodes}
      saveLabel={isDisabling ? strings.security.turnOff : strings.security.newRecoveryCodes}
      busyLabel={isDisabling ? strings.security.turningOff : strings.security.gettingRecoveryCodes}
      isSaveEnabled={passwordDraft !== "" && codeDraft.trim() !== ""}
      {isBusy}
      {errorMessage}
      onsave={() => void (isDisabling ? disable() : regenerate())}
      oncancel={() => void close()}
    >
      {@render passwordField(`${id}-prove-password`, isDisabling ? "" : strings.security.newRecoveryCodesHelper)}
      {@render codeField(`${id}-prove-code`, false)}
    </AccountEditForm>
  {:else if status.is_enabled}
    <p class="flex gap-(--mw-component-settings-error-icon-gap) type-body-lg text-(--mw-component-settings-body-text)">
      <Icon name="lock" class="size-(--mw-component-settings-error-icon-size) shrink-0 text-(--mw-component-settings-notice-text)" />
      <span>{strings.security.onStatus}</span>
    </p>
    <p class={["type-body", status.recovery_codes_remaining === 0 ? "text-(--mw-component-settings-error-text)" : "text-(--mw-component-settings-body-text)"]}>
      {status.recovery_codes_remaining === 0
        ? strings.security.noRecoveryCodesLeft
        : strings.security.recoveryCodesLeft(status.recovery_codes_remaining)}
    </p>
    <div class="flex flex-wrap gap-(--mw-component-settings-account-button-gap)">
      <Button id={openerId("regenerate")} onclick={() => void open("regenerate")}>{strings.security.newRecoveryCodes}</Button>
      <Button id={openerId("disable")} variant="danger" onclick={() => void open("disable")}>{strings.security.turnOff}</Button>
    </div>
  {:else}
    <p class="type-body-lg text-(--mw-component-settings-body-text)">{strings.security.offIntro}</p>
    <div>
      <Button id={openerId("password")} variant="primary" onclick={() => void open("password")}>{strings.security.turnOn}</Button>
    </div>
  {/if}
</div>

<!-- Exists from the start, so its first change is announced. -->
<p role="status" class="sr-only">{notice}</p>
