// Two-factor sign-in on the web (ADR-0049): what the Security page and the sign-in card build from
// the instance's answers, kept out of the components so it can be tested.

/** Wrong codes older than this no longer warn on the Security page. */
const RECENT_WRONG_CODE_MILLISECONDS = 30 * 24 * 60 * 60 * 1000;

/** How apps name the account: the instance's host name keeps two Mellow accounts apart. */
function issuer(hostname: string): string {
  return `Mellow (${hostname})`;
}

/** The `otpauth://` URI authenticator apps read from the QR code (Key Uri Format). Takes the host
 * name without the port: apps split the label at its first colon, encoded or not. */
export function otpauthUri(secret: string, username: string, hostname: string): string {
  const label = `${encodeURIComponent(issuer(hostname))}:${encodeURIComponent(username)}`;
  const parameters = new URLSearchParams({
    secret,
    issuer: issuer(hostname),
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

/** Whether a wrong code was typed for the account lately: someone else may know the password. */
export function isWrongCodeRecent(lastFailedAtUnixMilliseconds: number | null, nowUnixMilliseconds: number): boolean {
  return lastFailedAtUnixMilliseconds !== null && nowUnixMilliseconds - lastFailedAtUnixMilliseconds < RECENT_WRONG_CODE_MILLISECONDS;
}

/** Six digits, as the code field accepts them (spaces from a paste are dropped). */
export function isSixDigitCode(typed: string): boolean {
  return /^\d{6}$/.test(typed.replaceAll(" ", ""));
}

/** Sixteen letters and digits of a recovery code, dashes and spaces ignored. */
export function isRecoveryCode(typed: string): boolean {
  return /^[a-z2-7]{16}$/i.test(typed.replaceAll(/[\s-]/g, ""));
}
