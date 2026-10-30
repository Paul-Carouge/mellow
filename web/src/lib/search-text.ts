// Text as searches compare it: what people type rarely has the name's capitals or accents.

/** Lowercase, without accents, "œ" as "oe": "Général", "general" and "GÉNÉRAL" read the same. */
export function foldForSearch(text: string): string {
  return text
    .toLocaleLowerCase("fr")
    .replaceAll("œ", "oe")
    .normalize("NFD")
    .replace(/\p{Diacritic}/gu, "");
}
