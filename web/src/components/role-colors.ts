// Role colours on names (DESIGN.md §2.3): the hue an admin picked, with its OKLCH lightness moved
// until it reaches 4.5:1 on the surface it is drawn on, separately in each theme.

import type { Guild, Role, Snowflake } from "../lib/protocol";

const MINIMUM_NAME_CONTRAST = 4.5;
const LIGHTNESS_STEP = 0.005;
/** Below this relative luminance a surface counts as dark: names get lighter on it, not darker. */
const DARK_SURFACE_LUMINANCE = 0.18;

type Rgb = readonly [number, number, number];

/** The colour of the member's highest role that has one (Discord's rule), or null. */
export function highestColouredRole(guild: Guild, roleIds: readonly Snowflake[]): Role | null {
  return (
    guild.roles
      .filter((role) => role.color !== 0 && roleIds.includes(role.id))
      .sort((left, right) => right.position - left.position)[0] ?? null
  );
}

/** `#RRGGBB` for the role colour (`0xRRGGBB`), adjusted for 4.5:1 on `surfaceHex`. */
export function readableRoleColor(color: number, surfaceHex: string): string {
  const surface = parseHexColor(surfaceHex);
  const picked: Rgb = [((color >> 16) & 0xff) / 255, ((color >> 8) & 0xff) / 255, (color & 0xff) / 255];
  if (!surface || contrastRatio(picked, surface) >= MINIMUM_NAME_CONTRAST) {
    return toHex(picked);
  }
  const direction = relativeLuminance(surface) < DARK_SURFACE_LUMINANCE ? 1 : -1;
  const [lightness, a, b] = toOklab(picked);
  for (let next = lightness; next >= 0 && next <= 1; next += direction * LIGHTNESS_STEP) {
    // Checked after rounding to 8 bits, since that is the colour drawn.
    const [red, green, blue] = fromOklab(next, a, b);
    const candidate: Rgb = [toByte(red) / 255, toByte(green) / 255, toByte(blue) / 255];
    if (contrastRatio(candidate, surface) >= MINIMUM_NAME_CONTRAST) {
      return toHex(candidate);
    }
  }
  // Out of gamut before reaching the ratio: the surface's own text colour family always does.
  return direction > 0 ? "#FFFFFF" : "#000000";
}

export function contrastRatio(left: Rgb, right: Rgb): number {
  const [lighter, darker] = [relativeLuminance(left), relativeLuminance(right)].sort((x, y) => y - x) as [number, number];
  return (lighter + 0.05) / (darker + 0.05);
}

function parseHexColor(hex: string): Rgb | null {
  const match = /^\s*#([0-9a-f]{6})(?:[0-9a-f]{2})?\s*$/iu.exec(hex);
  if (!match?.[1]) {
    return null;
  }
  const value = Number.parseInt(match[1], 16);
  return [((value >> 16) & 0xff) / 255, ((value >> 8) & 0xff) / 255, (value & 0xff) / 255];
}

function toHex(rgb: Rgb): string {
  return `#${rgb.map((channel) => toByte(channel).toString(16).padStart(2, "0")).join("").toUpperCase()}`;
}

const clamp = (value: number) => Math.min(Math.max(value, 0), 1);
const toByte = (channel: number) => Math.round(clamp(channel) * 255);
const toLinear = (channel: number) => (channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4);
const fromLinear = (channel: number) =>
  clamp(channel) <= 0.0031308 ? 12.92 * clamp(channel) : 1.055 * clamp(channel) ** (1 / 2.4) - 0.055;

function relativeLuminance(rgb: Rgb): number {
  const [red, green, blue] = rgb.map(toLinear) as [number, number, number];
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

// Björn Ottosson's OKLab matrices.
function toOklab(rgb: Rgb): Rgb {
  const [red, green, blue] = rgb.map(toLinear) as [number, number, number];
  const l = Math.cbrt(0.4122214708 * red + 0.5363325363 * green + 0.0514459929 * blue);
  const m = Math.cbrt(0.2119034982 * red + 0.6806995451 * green + 0.1073969566 * blue);
  const s = Math.cbrt(0.0883024619 * red + 0.2817188376 * green + 0.6299787005 * blue);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

function fromOklab(lightness: number, a: number, b: number): Rgb {
  const l = (lightness + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (lightness - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (lightness - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    fromLinear(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
    fromLinear(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
    fromLinear(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s),
  ];
}
