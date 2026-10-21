// Reads design tokens (`--mw-*` in lib/tokens.css) where script needs a number: media queries,
// timers and scroll distances cannot take `var()`.

export function parseRemLength(value: string): number {
  const match = /^\s*(-?\d+(?:\.\d+)?)rem\s*$/u.exec(value);
  if (!match?.[1]) {
    throw new Error(`expected a rem length, got "${value}"`);
  }
  return Number(match[1]);
}

export function parseDurationInMilliseconds(value: string): number {
  const match = /^\s*(\d+(?:\.\d+)?)(ms|s)\s*$/u.exec(value);
  if (!match?.[1]) {
    throw new Error(`expected a duration, got "${value}"`);
  }
  return Number(match[1]) * (match[2] === "s" ? 1_000 : 1);
}

function tokenValue(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(`--mw-${name}`);
}

/** Media query matching windows at least as wide as the given panes side by side (DESIGN.md §4.2). */
export function minimumWidthQueryForPanes(paneWidthTokens: readonly string[]): string {
  const totalRem = paneWidthTokens.reduce((total, name) => total + parseRemLength(tokenValue(name)), 0);
  return `(min-width: ${totalRem}rem)`;
}

export function lengthTokenInPixels(name: string): number {
  const rootFontSize = Number.parseFloat(getComputedStyle(document.documentElement).fontSize);
  return parseRemLength(tokenValue(name)) * rootFontSize;
}

/** Reduced motion already maps these tokens to 0 ms. */
export function durationTokenInMilliseconds(name: string): number {
  return parseDurationInMilliseconds(tokenValue(name));
}
