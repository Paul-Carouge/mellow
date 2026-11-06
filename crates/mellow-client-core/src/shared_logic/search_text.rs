//! Text as searches compare it (`web/src/lib/search-text.ts`): what people type rarely has the
//! name's capitals or accents.

use unicode_normalization::UnicodeNormalization;

use super::unicode_classes::is_diacritic;

/// Lowercase, without accents, "œ" as "oe": "Général", "general" and "GÉNÉRAL" read the same.
#[must_use]
pub fn fold_for_search(text: &str) -> String {
    text.to_lowercase()
        .replace('œ', "oe")
        .nfd()
        .filter(|character| !is_diacritic(*character))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_case_accents_and_the_oe_ligature_as_the_web_does() {
        assert_eq!(fold_for_search("GÉNÉRAL"), "general");
        assert_eq!(fold_for_search("Œuvre ÉTÉ"), "oeuvre ete");
        // `\p{Diacritic}` also holds spacing marks such as "^" and "`".
        assert_eq!(fold_for_search("a^b ¨ ´ `x"), "ab   x");
    }
}
