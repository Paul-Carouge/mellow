use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Result, anyhow};

use super::*;

const PLURAL_SOURCE: &str = r#"export function pluralChooser(language: string): (count: number, forms: { one: string; other: string }) => string {
  const rules = new Intl.PluralRules(language);
  return (count, forms) => (rules.select(count) === "one" ? forms.one : forms.other);
}"#;

const PROTOCOL_SOURCE: &str = "const UNRELATED = /regex/;\nexport const MAXIMUM_THINGS = 4 * 2;\n";

const ENGLISH_SOURCE: &str = r#"
import type { ApiError } from "../lib/api";
import { MAXIMUM_THINGS } from "../lib/protocol";
import { pluralChooser } from "./plural";

const plural = pluralChooser("en");
const count = (value: number) => value.toLocaleString("en");

function namesInEnglish(names: readonly string[]): string {
  return names.length <= 1 ? (names[0] ?? "") : `${names.slice(0, -1).join(", ")} and ${names.at(-1) ?? ""}`;
}

export const en = {
  common: {
    cancel: "Cancel",
    limit: `At most ${MAXIMUM_THINGS} things`,
    withMentions: (name: string, mentions: number) =>
      `${name}, ${plural(mentions, { one: `${count(mentions)} mention`, other: `${count(mentions)} mentions` })}`,
    status: (status: string | null) => (status ? `Status: ${status}` : "No status"),
    names: (names: readonly string[]) => namesInEnglish(names),
    unit: (unit: "KB" | "MB") => `${unit}`,
  },
  tips: [{ example: "a", meaning: "b" }, { example: "c" }],
  keys: { ShiftLeft: "Left Shift" } as Record<string, string>,
  move: "Move",
};

export type Catalogue = typeof en;
"#;

const FRENCH_SOURCE: &str = r#"
import { MAXIMUM_THINGS } from "../lib/protocol";
import type { Catalogue } from "./en";
import { pluralChooser } from "./plural";

const NO_BREAK_SPACE = "\u00a0";
const plural = pluralChooser("fr");
const count = (value: number) => value.toLocaleString("fr");

export const fr: Catalogue = {
  common: {
    cancel: "Annuler",
    limit: `Au plus ${MAXIMUM_THINGS}${NO_BREAK_SPACE}choses`,
    withMentions: (name, mentions) =>
      `${name}, ${plural(mentions, { one: `${count(mentions)} mention`, other: `${count(mentions)} mentions` })}`,
    status: (status) => (status ? `Statut : ${status}` : "Aucun statut"),
    names: (people) => people.join(" et "),
    unit: (unit) => (unit === "KB" ? "Ko" : "Mo"),
  },
  tips: [{ example: "a", meaning: "b" }],
  keys: { ShiftLeft: "Maj gauche" },
  move: "Déplacer",
};
"#;

fn sources(replacements: &[(&str, &str, &str)]) -> BTreeMap<String, String> {
    let mut sources = BTreeMap::from([
        ("en.ts".to_owned(), ENGLISH_SOURCE.to_owned()),
        ("fr.ts".to_owned(), FRENCH_SOURCE.to_owned()),
        ("plural.ts".to_owned(), PLURAL_SOURCE.to_owned()),
        ("../lib/protocol.ts".to_owned(), PROTOCOL_SOURCE.to_owned()),
    ]);
    for (file, from, to) in replacements {
        let source = sources.get_mut(*file).unwrap();
        assert!(source.contains(from), "{from} is not in {file}");
        *source = source.replace(from, to);
    }
    sources
}

fn generate(replacements: &[(&str, &str, &str)]) -> Result<String> {
    let sources = sources(replacements);
    generate_catalogue(&|path: &str| {
        sources
            .get(path)
            .cloned()
            .ok_or_else(|| anyhow!("no {path}"))
    })
}

fn error(replacements: &[(&str, &str, &str)]) -> String {
    format!("{:#}", generate(replacements).unwrap_err())
}

#[test]
fn generates_one_typed_accessor_per_key() {
    let generated = generate(&[]).unwrap();
    for expected in [
        "pub struct Strings {",
        "pub fn common(self) -> CommonStrings {",
        "Language::English => \"Cancel\",\n            Language::French => \"Annuler\",",
        // Constants fold, the no-break space is escaped to stay visible.
        "\"At most 8 things\"",
        "\"Au plus 8\\u{a0}choses\"",
        "pub fn with_mentions(self, name: &str, mentions: i64) -> String {",
        "let mentions = mentions as f64;",
        "super::is_plural_one(Language::French, mentions)",
        "pub fn status(self, status: Option<&str>) -> String {",
        "if let Some(status) = status && !status.is_empty()",
        "fn names_in_english(names: &[&str]) -> String {",
        "pub fn unit(self, unit: KbOrMb) -> String {",
        "if unit == KbOrMb::Kb",
        "pub enum KbOrMb {",
        "Self::Kb => \"KB\",",
        "pub fn tips(self) -> &'static [TipsEntry] {",
        "pub meaning: Option<&'static str>,",
        "TipsEntry { example: \"c\", meaning: None }",
        "pub fn keys(self, key: &str) -> Option<&'static str> {",
        "\"ShiftLeft\" => Some(\"Maj gauche\"),",
        "pub fn r#move(self) -> &'static str {",
    ] {
        assert!(
            generated.contains(expected),
            "missing {expected:?} in\n{generated}"
        );
    }
}

#[test]
fn only_emits_the_helpers_an_entry_calls() {
    let generated = generate(&[(
        "en.ts",
        "names: (names: readonly string[]) => namesInEnglish(names),",
        "names: (names: readonly string[]) => names.join(\", \"),",
    )])
    .unwrap();
    assert!(!generated.contains("fn names_in_english"));
}

#[test]
fn rejects_a_key_missing_in_french() {
    let message = error(&[("fr.ts", "    cancel: \"Annuler\",\n", "")]);
    assert!(
        message.contains("fr.ts is missing `common.cancel`"),
        "{message}"
    );
}

#[test]
fn rejects_a_key_english_does_not_have() {
    let message = error(&[(
        "fr.ts",
        "    cancel: \"Annuler\",",
        "cancel: \"Annuler\", extra: \"x\",",
    )]);
    assert!(message.contains("keys that en.ts lacks"), "{message}");
}

#[test]
fn rejects_an_entry_of_another_kind_or_signature() {
    let message = error(&[("fr.ts", "move: \"Déplacer\"", "move: () => \"Déplacer\"")]);
    assert!(
        message.contains("is text in en.ts but a function in fr.ts"),
        "{message}"
    );
    let message = error(&[("fr.ts", "unit: (unit) =>", "unit: (unit, extra) =>")]);
    assert!(message.contains("takes 2 parameters"), "{message}");
}

#[test]
fn rejects_shapes_it_does_not_support() {
    for (from, to) in [
        ("cancel: \"Annuler\"", "cancel: new Date()"),
        ("(status ? `Statut", "(status == \"\" ? `Statut"),
        (
            "people.join(\" et \")",
            "people.map((person) => person).join(\" et \")",
        ),
        ("cancel: \"Annuler\"", "cancel: 1"),
    ] {
        assert!(
            generate(&[("fr.ts", from, to)]).is_err(),
            "{to} was accepted"
        );
    }
}

#[test]
fn stops_when_the_plural_rules_change() {
    let message = error(&[("plural.ts", "=== \"one\"", "=== \"few\"")]);
    assert!(
        message.contains("plural.ts no longer contains"),
        "{message}"
    );
    let message = error(&[("fr.ts", "pluralChooser(\"fr\")", "pluralChooser(\"en\")")]);
    assert!(message.contains("`plural`"), "{message}");
}

#[test]
fn reads_the_real_catalogues() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(I18N_SOURCE_DIRECTORY);
    let generated =
        generate_catalogue(&|path: &str| Ok(std::fs::read_to_string(directory.join(path))?))
            .unwrap();
    assert!(generated.contains("pub fn reacted_with(self, includes_you: bool, names: &[&str]"));
}
