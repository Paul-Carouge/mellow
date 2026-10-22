/** The singular or plural form of `count` under the language's own rules: French counts 0 as singular. */
export function pluralChooser(language: string): (count: number, forms: { one: string; other: string }) => string {
  const rules = new Intl.PluralRules(language);
  return (count, forms) => (rules.select(count) === "one" ? forms.one : forms.other);
}
