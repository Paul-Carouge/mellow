//! `cargo xtask i18n`: generates the desktop client's string catalogue from the web client's
//! (`web/src/i18n/en.ts` and `fr.ts`), so both clients say the same thing (ADR-0051). English is
//! the reference: a key missing in French, a French entry of another kind or with other
//! parameters stops generation. The TypeScript is read by a purpose-built parser over the
//! catalogues' actual shape, and compiled into typed Rust accessors.

mod constants;
mod expressions;
mod lexer;
mod naming;
mod parser;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use anyhow::{Context as _, Result, anyhow, bail, ensure};

use self::constants::{ConstantValue, Constants, fold};
use self::expressions::{
    FunctionCompiler, ModuleContext, Registry, Scope, ValueType, Variable, compile_helper,
};
use self::naming::{choice_enum_name, pascal_case, rust_string_literal, snake_case_identifier};
use self::parser::{Arrow, Expression, Function, FunctionBody, parse_expression, parse_module};

pub const I18N_SOURCE_DIRECTORY: &str = "web/src/i18n";
pub const CATALOGUE_OUTPUT_PATH: &str = "crates/mellow-desktop/src/i18n/catalogue.rs";

/// The two lines of `plural.ts` the runtime's plural rules reproduce (CLDR's, through
/// `Intl.PluralRules`): if they change, the Rust rules must be checked again.
const EXPECTED_PLURAL_RULES: [&str; 2] = [
    "new Intl.PluralRules(language)",
    "rules.select(count) === \"one\" ? forms.one : forms.other",
];
/// Keys the root `Strings` type uses for its own methods.
const RESERVED_ROOT_KEYS: [&str; 2] = ["new", "language"];
const ARM_INDENT: &str = "            ";
const BLOCK_INDENT: &str = "                ";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CatalogueLanguage {
    English,
    French,
}

impl CatalogueLanguage {
    const ALL: [Self; 2] = [Self::English, Self::French];

    fn tag(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::French => "fr",
        }
    }

    fn from_tag(tag: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|language| language.tag() == tag)
            .ok_or_else(|| anyhow!("unsupported language tag `{tag}`"))
    }

    fn rust_variant(self) -> &'static str {
        match self {
            Self::English => "Language::English",
            Self::French => "Language::French",
        }
    }

    fn file_name(self) -> String {
        format!("{}.ts", self.tag())
    }
}

/// One catalogue file, read: its constants folded, its helper functions, its catalogue object.
struct LoadedModule {
    constants: Constants,
    helpers: BTreeMap<String, Function>,
    catalogue: Expression,
}

/// The generated Rust. `read_source` gives a file's text from its path relative to the i18n
/// directory (`en.ts`, `../lib/protocol.ts`), so tests can use sources in memory.
pub fn generate_catalogue(read_source: &dyn Fn(&str) -> Result<String>) -> Result<String> {
    let english = load_module(CatalogueLanguage::English, read_source)?;
    let french = load_module(CatalogueLanguage::French, read_source)?;
    Emitter::emit(&english, &french)
}

fn load_module(
    language: CatalogueLanguage,
    read_source: &dyn Fn(&str) -> Result<String>,
) -> Result<LoadedModule> {
    let file_name = language.file_name();
    let source = read_source(&file_name)?;
    let module = lexer::tokenize(&source)
        .and_then(parse_module)
        .with_context(|| format!("cannot read {file_name}"))?;

    let mut constants = Constants::new();
    for import in &module.imports {
        if import.path == "./plural" {
            ensure!(
                import.names == ["pluralChooser"],
                "{file_name}: only `pluralChooser` comes from ./plural"
            );
            check_plural_rules(&read_source("plural.ts")?)?;
            continue;
        }
        let path = format!("{}.ts", import.path);
        let imported_source = read_source(&path)?;
        for name in &import.names {
            constants.insert(
                name.clone(),
                imported_constant(&imported_source, name)
                    .with_context(|| format!("{file_name}: cannot import `{name}` from {path}"))?,
            );
        }
    }

    let mut catalogue = None;
    let mut has_plural = false;
    let mut has_count = false;
    for (name, value) in module.constants {
        match name.as_str() {
            "plural" => {
                check_plural_constant(&value, language)
                    .with_context(|| format!("{file_name}: `plural`"))?;
                has_plural = true;
            }
            "count" => {
                check_count_constant(&value, language)
                    .with_context(|| format!("{file_name}: `count`"))?;
                has_count = true;
            }
            _ if name == language.tag() => catalogue = Some(value),
            _ => {
                let folded = fold(&value, &constants).ok_or_else(|| {
                    anyhow!("{file_name}: the constant `{name}` is not a constant value")
                })?;
                constants.insert(name, folded);
            }
        }
    }
    ensure!(
        has_plural && has_count,
        "{file_name} must define `plural` and `count`, which entries call"
    );
    let mut helpers = BTreeMap::new();
    for function in module.functions {
        ensure!(
            !constants.contains_key(&function.name),
            "{file_name}: `{}` is both a constant and a function",
            function.name
        );
        helpers.insert(function.name.clone(), function);
    }
    Ok(LoadedModule {
        constants,
        helpers,
        catalogue: catalogue
            .ok_or_else(|| anyhow!("{file_name} exports no `{}` catalogue", language.tag()))?,
    })
}

fn check_plural_rules(plural_source: &str) -> Result<()> {
    for expected in EXPECTED_PLURAL_RULES {
        ensure!(
            plural_source.contains(expected),
            "plural.ts no longer contains `{expected}`: check the desktop's plural rules \
             (crates/mellow-desktop/src/i18n/numbers.rs) against it, then update the generator"
        );
    }
    Ok(())
}

/// `const plural = pluralChooser("en");`
fn check_plural_constant(value: &Expression, language: CatalogueLanguage) -> Result<()> {
    match value {
        Expression::Call { callee, arguments }
            if **callee == Expression::Identifier("pluralChooser".to_owned())
                && *arguments == [Expression::String(language.tag().to_owned())] =>
        {
            Ok(())
        }
        _ => bail!("expected `pluralChooser(\"{}\")`", language.tag()),
    }
}

/// `const count = (value: number) => value.toLocaleString("en");`
fn check_count_constant(value: &Expression, language: CatalogueLanguage) -> Result<()> {
    if let Expression::Arrow(arrow) = value
        && let [parameter] = arrow.parameters.as_slice()
        && let FunctionBody::Expression(body) = &arrow.body
        && let Expression::Call { callee, arguments } = body.as_ref()
        && let Expression::Member { object, property } = callee.as_ref()
        && **object == Expression::Identifier(parameter.name.clone())
        && property == "toLocaleString"
        && *arguments == [Expression::String(language.tag().to_owned())]
    {
        return Ok(());
    }
    bail!(
        "expected `(value: number) => value.toLocaleString(\"{}\")`",
        language.tag()
    )
}

/// The value of `export const NAME = …;` in a file the lexer does not otherwise read.
fn imported_constant(source: &str, name: &str) -> Result<ConstantValue> {
    let needle = format!("export const {name} = ");
    let start = source
        .find(&needle)
        .ok_or_else(|| anyhow!("no `{needle}…;`"))?;
    let line = source[..start].matches('\n').count() + 1;
    let tokens = lexer::tokenize_statement(&source[start + needle.len()..], line)?;
    fold(&parse_expression(tokens)?, &Constants::new())
        .ok_or_else(|| anyhow!("its value is not constant"))
}

/// A catalogue value, normalised: constants are folded and imported objects expanded.
enum Node<'a> {
    Text(String),
    Function(&'a Arrow),
    Object(Vec<(String, Node<'a>)>),
    List(Vec<Node<'a>>),
    /// Text by a key known at run time; each language has its own keys.
    Record(Vec<(String, String)>),
}

impl Node<'_> {
    fn kind(&self) -> &'static str {
        match self {
            Self::Text(_) => "text",
            Self::Function(_) => "a function",
            Self::Object(_) => "an object",
            Self::List(_) => "a list",
            Self::Record(_) => "a record",
        }
    }
}

fn node<'a>(expression: &'a Expression, constants: &Constants, path: &str) -> Result<Node<'a>> {
    match expression {
        Expression::Arrow(arrow) => Ok(Node::Function(arrow)),
        Expression::Object(properties) => {
            let mut entries = Vec::new();
            for property in properties {
                let child_path = format!("{path}.{}", property.key);
                let child = node(&property.value, constants, &child_path)
                    .with_context(|| format!("line {}", property.line))?;
                entries.push((property.key.clone(), child));
            }
            Ok(Node::Object(entries))
        }
        Expression::TextRecord(object) => match node(object, constants, path)? {
            Node::Object(entries) => entries
                .into_iter()
                .map(|(key, value)| match value {
                    Node::Text(text) => Ok((key, text)),
                    other => bail!("`{path}.{key}` is {}, records hold text", other.kind()),
                })
                .collect::<Result<_>>()
                .map(Node::Record),
            other => bail!("`{path}` is {}, not a record", other.kind()),
        },
        Expression::Array(elements) => elements
            .iter()
            .map(|element| match element {
                parser::ArrayElement::Item(item) => node(item, constants, path),
                parser::ArrayElement::Spread(_) => bail!("`{path}`: unsupported spread"),
            })
            .collect::<Result<_>>()
            .map(Node::List),
        _ => match fold(expression, constants) {
            Some(constant) => node_from_constant(constant, path),
            None => bail!("`{path}` is not a constant, a function, an object or a list"),
        },
    }
}

fn node_from_constant(constant: ConstantValue, path: &str) -> Result<Node<'static>> {
    match constant {
        ConstantValue::Text(text) => Ok(Node::Text(text)),
        ConstantValue::Number(_) => bail!("`{path}` is a number; catalogue entries are text"),
        ConstantValue::Object(fields) => fields
            .into_iter()
            .map(|(key, value)| {
                let child = node_from_constant(value, &format!("{path}.{key}"))?;
                Ok((key, child))
            })
            .collect::<Result<_>>()
            .map(Node::Object),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FieldKind {
    Text,
    /// Text some elements leave out.
    OptionalText,
    /// The name of the entry struct of the nested list.
    List(String),
}

/// The fields of the elements of every list sharing one key, in both languages.
#[derive(Debug, Default)]
struct EntryShape {
    /// Name, kind, and how many elements have it.
    fields: Vec<(String, FieldKind, usize)>,
    element_count: usize,
}

fn entry_struct_name(key: &str) -> String {
    format!("{}Entry", pascal_case(key))
}

/// Gathers the element shape of every list, before any literal is written: lists sharing a key
/// (`channels`) share one struct, whose field is optional when some element leaves it out.
fn collect_entry_shapes(
    key: &str,
    node: &Node<'_>,
    shapes: &mut BTreeMap<String, EntryShape>,
) -> Result<()> {
    match node {
        Node::Object(entries) => {
            for (child_key, child) in entries {
                collect_entry_shapes(child_key, child, shapes)?;
            }
        }
        Node::List(elements) => {
            let shape = shapes.entry(entry_struct_name(key)).or_default();
            for element in elements {
                let Node::Object(fields) = element else {
                    bail!("the `{key}` list must hold objects");
                };
                shape.element_count += 1;
                for (field, value) in fields {
                    let kind = match value {
                        Node::Text(_) => FieldKind::Text,
                        Node::List(_) => FieldKind::List(entry_struct_name(field)),
                        other => {
                            bail!("`{key}` elements hold text and lists, not {}", other.kind())
                        }
                    };
                    match shape.fields.iter_mut().find(|(name, _, _)| name == field) {
                        Some((_, existing, occurrences)) => {
                            ensure!(*existing == kind, "`{key}.{field}` changes kind");
                            *occurrences += 1;
                        }
                        None => shape.fields.push((field.clone(), kind, 1)),
                    }
                }
            }
            for element in elements {
                collect_entry_shapes(key, element, shapes)?;
            }
        }
        Node::Text(_) | Node::Function(_) | Node::Record(_) => {}
    }
    Ok(())
}

fn final_entry_structs(
    shapes: BTreeMap<String, EntryShape>,
) -> Result<BTreeMap<String, Vec<(String, FieldKind)>>> {
    shapes
        .into_iter()
        .map(|(name, shape)| {
            let fields = shape
                .fields
                .into_iter()
                .map(|(field, kind, occurrences)| {
                    let kind = match kind {
                        FieldKind::Text if occurrences < shape.element_count => {
                            FieldKind::OptionalText
                        }
                        FieldKind::List(_) if occurrences < shape.element_count => {
                            bail!("`{name}.{field}`: a list cannot be left out")
                        }
                        kind => kind,
                    };
                    Ok((field, kind))
                })
                .collect::<Result<_>>()?;
            Ok((name, fields))
        })
        .collect()
}

struct Emitter<'a> {
    english: &'a LoadedModule,
    french: &'a LoadedModule,
    registry: Registry,
    /// Accessor structs, in emission order.
    accessor_structs: Vec<String>,
    struct_names: BTreeSet<String>,
    entry_structs: BTreeMap<String, Vec<(String, FieldKind)>>,
}

impl<'a> Emitter<'a> {
    fn emit(english: &'a LoadedModule, french: &'a LoadedModule) -> Result<String> {
        let english_root = node(&english.catalogue, &english.constants, "en")?;
        let french_root = node(&french.catalogue, &french.constants, "fr")?;
        let (Node::Object(english_entries), Node::Object(french_entries)) =
            (english_root, french_root)
        else {
            bail!("each catalogue must be an object");
        };
        for (key, _) in &english_entries {
            ensure!(
                !RESERVED_ROOT_KEYS.contains(&key.as_str()),
                "the top-level key `{key}` is reserved by the generated `Strings`"
            );
        }
        let mut shapes = BTreeMap::new();
        for (key, entry) in english_entries.iter().chain(&french_entries) {
            collect_entry_shapes(key, entry, &mut shapes)?;
        }
        let mut emitter = Self {
            english,
            french,
            registry: Registry::default(),
            accessor_structs: Vec::new(),
            struct_names: BTreeSet::new(),
            entry_structs: final_entry_structs(shapes)?,
        };
        emitter.object(&[], &english_entries, &french_entries)?;
        let helpers = emitter.helpers()?;
        emitter.file(&helpers)
    }

    fn module_context(&self, language: CatalogueLanguage) -> ModuleContext<'a> {
        let module = match language {
            CatalogueLanguage::English => self.english,
            CatalogueLanguage::French => self.french,
        };
        ModuleContext {
            language,
            constants: &module.constants,
            helpers: &module.helpers,
        }
    }

    fn struct_name(path: &[String]) -> String {
        if path.is_empty() {
            "Strings".to_owned()
        } else {
            format!("{}Strings", pascal_case(&path.join(".")))
        }
    }

    fn object(
        &mut self,
        path: &[String],
        english_entries: &[(String, Node<'_>)],
        french_entries: &[(String, Node<'_>)],
    ) -> Result<()> {
        let dotted = if path.is_empty() {
            "the catalogue".to_owned()
        } else {
            format!("`{}`", path.join("."))
        };
        let english_keys: BTreeSet<&String> = english_entries.iter().map(|(key, _)| key).collect();
        let extra: Vec<&String> = french_entries
            .iter()
            .map(|(key, _)| key)
            .filter(|key| !english_keys.contains(key))
            .collect();
        ensure!(
            extra.is_empty(),
            "fr.ts has keys that en.ts lacks in {dotted}: {extra:?}"
        );

        let struct_name = Self::struct_name(path);
        ensure!(
            self.struct_names.insert(struct_name.clone()),
            "two catalogue objects both become `{struct_name}`"
        );
        let slot = self.accessor_structs.len();
        self.accessor_structs.push(String::new());

        let mut methods = Vec::new();
        let mut method_names = BTreeSet::new();
        for (key, english) in english_entries {
            let french = french_entries
                .iter()
                .find_map(|(french_key, french)| (french_key == key).then_some(french))
                .ok_or_else(|| anyhow!("fr.ts is missing `{}`", dotted_path(path, key)))?;
            let method_name = snake_case_identifier(key);
            ensure!(
                method_names.insert(method_name.clone()),
                "two keys of {dotted} both become `{method_name}`"
            );
            let child_path: Vec<String> = path.iter().cloned().chain([key.clone()]).collect();
            let method = match (english, french) {
                (Node::Text(english), Node::Text(french)) => {
                    text_method(&method_name, english, french)
                }
                (Node::Function(english), Node::Function(french)) => self
                    .function_method(&method_name, english, french)
                    .with_context(|| format!("in `{}`", child_path.join(".")))?,
                (Node::Object(english), Node::Object(french)) => {
                    let child_struct = Self::struct_name(&child_path);
                    self.object(&child_path, english, french)?;
                    format!(
                        "    pub fn {method_name}(self) -> {child_struct} {{\n        {child_struct} {{ language: self.language }}\n    }}\n"
                    )
                }
                (Node::Record(english), Node::Record(french)) => {
                    record_method(&method_name, english, french)?
                }
                // fr.ts is typed by en.ts: its plain object is the same record.
                (Node::Record(english), Node::Object(french)) => {
                    let french = french
                        .iter()
                        .map(|(french_key, value)| match value {
                            Node::Text(text) => Ok((french_key.clone(), text.clone())),
                            other => bail!(
                                "`{}.{french_key}` is {}, records hold text",
                                child_path.join("."),
                                other.kind()
                            ),
                        })
                        .collect::<Result<Vec<_>>>()?;
                    record_method(&method_name, english, &french)?
                }
                (Node::List(english), Node::List(french)) => self
                    .list_method(&method_name, key, english, french)
                    .with_context(|| format!("in `{}`", child_path.join(".")))?,
                (english, french) => bail!(
                    "`{}` is {} in en.ts but {} in fr.ts",
                    child_path.join("."),
                    english.kind(),
                    french.kind()
                ),
            };
            methods.push(method);
        }

        let code = accessor_struct(&struct_name, path.is_empty(), &methods)?;
        if let Some(slot) = self.accessor_structs.get_mut(slot) {
            *slot = code;
        }
        Ok(())
    }

    fn function_method(
        &mut self,
        method_name: &str,
        english: &Arrow,
        french: &Arrow,
    ) -> Result<String> {
        ensure!(
            english.parameters.len() == french.parameters.len(),
            "fr.ts takes {} parameters, en.ts {}",
            french.parameters.len(),
            english.parameters.len()
        );
        let mut parameter_types = Vec::new();
        let mut rust_names = Vec::new();
        for (english_parameter, french_parameter) in
            english.parameters.iter().zip(&french.parameters)
        {
            let annotation = english_parameter
                .annotation
                .as_ref()
                .ok_or_else(|| anyhow!("en.ts must type `{}`", english_parameter.name))?;
            if let Some(french_annotation) = &french_parameter.annotation {
                ensure!(
                    french_annotation == annotation,
                    "fr.ts types `{}` differently",
                    french_parameter.name
                );
            }
            let value_type = ValueType::from_annotation(annotation);
            if let ValueType::Choice(literals) = &value_type {
                self.registry.choices.insert(literals.clone());
            }
            parameter_types.push(value_type);
            rust_names.push(snake_case_identifier(&english_parameter.name));
        }

        let mut arms = Vec::new();
        let mut used_parameters = BTreeSet::new();
        for (language, arrow) in [
            (CatalogueLanguage::English, english),
            (CatalogueLanguage::French, french),
        ] {
            let mut scope = Scope::new();
            for (index, parameter) in arrow.parameters.iter().enumerate() {
                let (Some(rust_name), Some(value_type)) =
                    (rust_names.get(index), parameter_types.get(index))
                else {
                    bail!("parameter {index} has no English counterpart");
                };
                scope.insert(
                    parameter.name.clone(),
                    Variable {
                        rust_name: rust_name.clone(),
                        value_type: value_type.clone(),
                        is_owned: false,
                        parameter_index: Some(index),
                    },
                );
            }
            let context = self.module_context(language);
            let mut compiler = FunctionCompiler::new(&context, &mut self.registry);
            let arm = match &arrow.body {
                FunctionBody::Expression(body) => compiler
                    .compile_text_body(body, &scope)
                    .map(|code| format!("{ARM_INDENT}{} => {code},\n", language.rust_variant())),
                FunctionBody::Block(statements) => compiler
                    .compile_block_body(statements, &scope, BLOCK_INDENT)
                    .map(|code| {
                        format!(
                            "{ARM_INDENT}{} => {{\n{code}{ARM_INDENT}}}\n",
                            language.rust_variant()
                        )
                    }),
            }
            .with_context(|| language.file_name())?;
            used_parameters.extend(compiler.used_parameters.iter().copied());
            arms.push(arm);
        }

        let (parameters, conversions) =
            signature_and_conversions(&rust_names, &parameter_types, &used_parameters)?;
        Ok(format!(
            "    pub fn {method_name}({}) -> String {{\n{conversions}        match self.language {{\n{}        }}\n    }}\n",
            parameters.join(", "),
            arms.concat()
        ))
    }

    fn list_method(
        &self,
        method_name: &str,
        key: &str,
        english: &[Node<'_>],
        french: &[Node<'_>],
    ) -> Result<String> {
        let entry_struct = entry_struct_name(key);
        let literal = |elements: &[Node<'_>]| -> Result<String> {
            let entries = elements
                .iter()
                .map(|element| entry_literal(&entry_struct, element, &self.entry_structs))
                .collect::<Result<Vec<_>>>()?;
            Ok(format!("&[{}]", entries.join(", ")))
        };
        Ok(format!(
            "    pub fn {method_name}(self) -> &'static [{entry_struct}] {{\n        match self.language {{\n{ARM_INDENT}Language::English => {},\n{ARM_INDENT}Language::French => {},\n        }}\n    }}\n",
            literal(english)?,
            literal(french)?
        ))
    }

    fn helpers(&mut self) -> Result<String> {
        let mut emitted = BTreeSet::new();
        let mut code = String::new();
        loop {
            let pending: Vec<(CatalogueLanguage, String)> = self
                .registry
                .called_helpers
                .difference(&emitted)
                .cloned()
                .collect();
            if pending.is_empty() {
                return Ok(code);
            }
            for (language, name) in pending {
                let context = self.module_context(language);
                let function = context
                    .helpers
                    .get(&name)
                    .ok_or_else(|| anyhow!("unknown helper `{name}`"))?;
                code.push('\n');
                code.push_str(
                    &compile_helper(function, &context, &mut self.registry)
                        .with_context(|| language.file_name())?,
                );
                emitted.insert((language, name));
            }
        }
    }

    fn file(&self, helpers: &str) -> Result<String> {
        let mut file = String::from(
            "// @generated by `cargo xtask i18n` from web/src/i18n/en.ts and fr.ts. Do not edit by hand.\n\
             //! Every string of the web client's catalogues, in English and French: one accessor per\n\
             //! key, taking typed parameters where the web's entry is a function.\n\
             #![allow(\n\
             \x20   clippy::cast_precision_loss,\n\
             \x20   clippy::collapsible_else_if,\n\
             \x20   clippy::float_cmp,\n\
             \x20   clippy::if_not_else,\n\
             \x20   clippy::map_unwrap_or,\n\
             \x20   clippy::match_same_arms,\n\
             \x20   clippy::must_use_candidate,\n\
             \x20   clippy::too_many_lines,\n\
             \x20   clippy::unreadable_literal,\n\
             \x20   clippy::vec_init_then_push,\n\
             \x20   reason = \"generated from the web catalogues: numbers keep JavaScript's f64 meaning, each \
             accessor mirrors one entry (even when both languages say the same), and branches keep the \
             TypeScript's order\"\n\
             )]\n\nuse super::Language;\n",
        );
        for code in &self.accessor_structs {
            file.push('\n');
            file.push_str(code);
        }
        for (name, fields) in &self.entry_structs {
            writeln!(
                file,
                "\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub struct {name} {{"
            )?;
            for (field, kind) in fields {
                let field_type = match kind {
                    FieldKind::Text => "&'static str".to_owned(),
                    FieldKind::OptionalText => "Option<&'static str>".to_owned(),
                    FieldKind::List(entry) => format!("&'static [{entry}]"),
                };
                writeln!(
                    file,
                    "    pub {}: {field_type},",
                    snake_case_identifier(field)
                )?;
            }
            file.push_str("}\n");
        }
        for literals in &self.registry.choices {
            let name = choice_enum_name(literals);
            let variants: Vec<String> = literals
                .iter()
                .map(|literal| pascal_case(literal))
                .collect();
            writeln!(
                file,
                "\n/// The web catalogue's union `{}`.\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub enum {name} {{",
                literals
                    .iter()
                    .map(|literal| format!("\"{literal}\""))
                    .collect::<Vec<_>>()
                    .join(" | ")
            )?;
            for variant in &variants {
                writeln!(file, "    {variant},")?;
            }
            writeln!(
                file,
                "}}\n\nimpl std::fmt::Display for {name} {{\n    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n        formatter.write_str(match self {{"
            )?;
            for (variant, literal) in variants.iter().zip(literals) {
                writeln!(
                    file,
                    "            Self::{variant} => {},",
                    rust_string_literal(literal)
                )?;
            }
            file.push_str("        })\n    }\n}\n");
        }
        file.push_str(helpers);
        Ok(file)
    }
}

/// An accessor struct: one method per key of a catalogue object.
fn accessor_struct(struct_name: &str, is_root: bool, methods: &[String]) -> Result<String> {
    let mut code = String::new();
    if is_root {
        code.push_str(
            "/// Every string of the interface, in one language: `Strings::new(language)`, then one\n\
             /// accessor per catalogue key, as in the web client (`strings.common().cancel()`).\n\
             #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]\n",
        );
    } else {
        code.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n");
    }
    writeln!(
        code,
        "pub struct {struct_name} {{\n    language: Language,\n}}\n\nimpl {struct_name} {{"
    )?;
    if is_root {
        code.push_str(
            "    pub const fn new(language: Language) -> Self {\n        Self { language }\n    }\n\n\
             \x20   pub const fn language(self) -> Language {\n        self.language\n    }\n\n",
        );
    }
    code.push_str(&methods.join("\n"));
    code.push_str("}\n");
    Ok(code)
}

/// The accessor's parameters (unused ones prefixed with `_`) and the conversion of its numbers
/// to `f64`, which keeps JavaScript's meaning inside the body.
fn signature_and_conversions(
    rust_names: &[String],
    parameter_types: &[ValueType],
    used_parameters: &BTreeSet<usize>,
) -> Result<(Vec<String>, String)> {
    let mut parameters = vec!["self".to_owned()];
    let mut conversions = String::new();
    for (index, (rust_name, value_type)) in rust_names.iter().zip(parameter_types).enumerate() {
        let is_used = used_parameters.contains(&index);
        let declared_name = if is_used {
            rust_name.clone()
        } else {
            format!("_{}", rust_name.trim_start_matches("r#"))
        };
        parameters.push(format!(
            "{declared_name}: {}",
            value_type.rust_parameter_type(true)
        ));
        // Numbers keep JavaScript's meaning (`f64`) inside the body.
        match (value_type, is_used) {
            (ValueType::Number, true) => {
                writeln!(conversions, "        let {rust_name} = {rust_name} as f64;")?;
            }
            (ValueType::OptionalNumber, true) => writeln!(
                conversions,
                "        let {rust_name} = {rust_name}.map(|{rust_name}| {rust_name} as f64);"
            )?,
            _ => {}
        }
    }
    Ok((parameters, conversions))
}

fn dotted_path(path: &[String], key: &str) -> String {
    path.iter()
        .map(String::as_str)
        .chain([key])
        .collect::<Vec<_>>()
        .join(".")
}

fn text_method(method_name: &str, english: &str, french: &str) -> String {
    let body = if english == french {
        format!("        {}\n", rust_string_literal(english))
    } else {
        format!(
            "        match self.language {{\n{ARM_INDENT}Language::English => {},\n{ARM_INDENT}Language::French => {},\n        }}\n",
            rust_string_literal(english),
            rust_string_literal(french)
        )
    };
    format!("    pub fn {method_name}(self) -> &'static str {{\n{body}    }}\n")
}

fn record_method(
    method_name: &str,
    english: &[(String, String)],
    french: &[(String, String)],
) -> Result<String> {
    let arms = |entries: &[(String, String)]| -> Result<String> {
        let mut code = String::from("match key {\n");
        for (key, text) in entries {
            writeln!(
                code,
                "{BLOCK_INDENT}{} => Some({}),",
                rust_string_literal(key),
                rust_string_literal(text)
            )?;
        }
        write!(code, "{BLOCK_INDENT}_ => None,\n{ARM_INDENT}}}")?;
        Ok(code)
    };
    Ok(format!(
        "    pub fn {method_name}(self, key: &str) -> Option<&'static str> {{\n        match self.language {{\n{ARM_INDENT}Language::English => {},\n{ARM_INDENT}Language::French => {},\n        }}\n    }}\n",
        arms(english)?,
        arms(french)?
    ))
}

fn entry_literal(
    struct_name: &str,
    element: &Node<'_>,
    entry_structs: &BTreeMap<String, Vec<(String, FieldKind)>>,
) -> Result<String> {
    let Node::Object(values) = element else {
        bail!("lists must hold objects");
    };
    let fields = entry_structs
        .get(struct_name)
        .ok_or_else(|| anyhow!("unknown entry struct `{struct_name}`"))?;
    let mut parts = Vec::new();
    for (field, kind) in fields {
        let value = values
            .iter()
            .find_map(|(name, value)| (name == field).then_some(value));
        let code = match (kind, value) {
            (FieldKind::Text, Some(Node::Text(text))) => rust_string_literal(text),
            (FieldKind::OptionalText, Some(Node::Text(text))) => {
                format!("Some({})", rust_string_literal(text))
            }
            (FieldKind::OptionalText, None) => "None".to_owned(),
            (FieldKind::List(entry), Some(Node::List(items))) => format!(
                "&[{}]",
                items
                    .iter()
                    .map(|item| entry_literal(entry, item, entry_structs))
                    .collect::<Result<Vec<_>>>()?
                    .join(", ")
            ),
            _ => bail!("`{field}` changes kind between elements"),
        };
        parts.push(format!("{}: {code}", snake_case_identifier(field)));
    }
    Ok(format!("{struct_name} {{ {} }}", parts.join(", ")))
}

#[cfg(test)]
mod tests;
