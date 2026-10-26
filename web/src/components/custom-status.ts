import { MAXIMUM_CUSTOM_STATUS_LENGTH } from "../lib/protocol";

/**
 * How many characters the custom status may still take, negative when it is too long. Counted as
 * the instance does: Unicode scalar values of the trimmed text, so an emoji is not two.
 */
export function customStatusCharactersLeft(draft: string): number {
  return MAXIMUM_CUSTOM_STATUS_LENGTH - [...draft.trim()].length;
}
