// Two-factor sign-in on the web (ADR-0049): what the Security page and the sign-in card build from
// the instance's answers, kept out of the components so it can be tested.

/** How apps name the account: the instance's host keeps two Mellow accounts apart. */
function issuer(host: string): string {
  return `Mellow (${host})`;
}

/** The `otpauth://` URI authenticator apps read from the QR code (Key Uri Format). */
export function otpauthUri(secret: string, username: string, host: string): string {
  const label = `${encodeURIComponent(issuer(host))}:${encodeURIComponent(username)}`;
  const parameters = new URLSearchParams({
    secret,
    issuer: issuer(host),
    algorithm: "SHA1",
    digits: "6",
    period: "30",
  });
  // `URLSearchParams` writes spaces as "+", which some apps keep as is.
  return `otpauth://totp/${label}?${parameters.toString().replaceAll("+", "%20")}`;
}

/** The secret in groups of four, as people type it into an app by hand. */
export function groupedSecret(secret: string): string {
  return secret.match(/.{1,4}/g)?.join(" ") ?? "";
}

/** The text file a person keeps their recovery codes in. */
export function recoveryCodesFileText(heading: string, codes: readonly string[]): string {
  return `${heading}\n\n${codes.join("\n")}\n`;
}

/** Six digits, as the code field accepts them (spaces from a paste are dropped). */
export function isSixDigitCode(typed: string): boolean {
  return /^\d{6}$/.test(typed.replaceAll(" ", ""));
}

/** Sixteen letters and digits of a recovery code, dashes and spaces ignored. */
export function isRecoveryCode(typed: string): boolean {
  return /^[a-z2-7]{16}$/i.test(typed.replaceAll(/[\s-]/g, ""));
}
