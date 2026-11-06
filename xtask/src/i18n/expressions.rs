//! Compiles the catalogues' functions (arrows and helper functions) into Rust expressions with
//! JavaScript's meaning: numbers are `f64`, text truthiness is "not empty", `null` and `undefined`
//! are `None`, and TypeScript's narrowing (`x !== null ? x : …`) becomes `if let`. Each value has
//! a type, so a body mixing types the generator does not know fails with a message, never with
//! wrong Rust.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use anyhow::{Result, anyhow, bail, ensure};

use super::CatalogueLanguage;
use super::constants::{ConstantValue, Constants, fold, fold_to_text, javascript_number_to_string};
use super::naming::{choice_enum_name, pascal_case, rust_string_literal, snake_case_identifier};
use super::parser::{
    ArrayElement, BinaryOperator, Binding, Expression, Function, Statement, TypeAnnotation,
    UnaryOperator,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueType {
    Text,
    Number,
    Boolean,
    OptionalText,
    OptionalNumber,
    TextList(ListShape),
    /// A union of string literals, as a Rust enum.
    Choice(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListShape {
    /// `Vec<String>` built by the body, rather than the caller's `&[&str]`.
    pub has_owned_elements: bool,
    /// The code names a place (`people`, `places[..n]`) that must be borrowed to pass as a slice.
    pub needs_borrow_to_pass: bool,
}

const PARAMETER_LIST: ListShape = ListShape {
    has_owned_elements: false,
    needs_borrow_to_pass: false,
};
const BUILT_LIST: ListShape = ListShape {
    has_owned_elements: true,
    needs_borrow_to_pass: true,
};

impl ValueType {
    pub fn from_annotation(annotation: &TypeAnnotation) -> Self {
        match annotation {
            TypeAnnotation::String => Self::Text,
            TypeAnnotation::Number => Self::Number,
            TypeAnnotation::Boolean => Self::Boolean,
            TypeAnnotation::NullableString => Self::OptionalText,
            TypeAnnotation::NullableNumber => Self::OptionalNumber,
            TypeAnnotation::StringList => Self::TextList(PARAMETER_LIST),
            TypeAnnotation::StringUnion(literals) => Self::Choice(literals.clone()),
        }
    }

    /// The Rust type of a parameter: catalogue accessors take integers (counts, sizes) where
    /// helpers, called with already converted values, take `f64`.
    pub fn rust_parameter_type(&self, numbers_are_integers: bool) -> String {
        let number = if numbers_are_integers { "i64" } else { "f64" };
        match self {
            Self::Text => "&str".to_owned(),
            Self::Number => number.to_owned(),
            Self::Boolean => "bool".to_owned(),
            Self::OptionalText => "Option<&str>".to_owned(),
            Self::OptionalNumber => format!("Option<{number}>"),
            Self::TextList(_) => "&[&str]".to_owned(),
            Self::Choice(literals) => choice_enum_name(literals),
        }
    }
}

/// How tightly a piece of Rust binds, to add exactly the parentheses Rust needs (rustc warns on
/// the others).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Precedence {
    /// `if … else …`, a block, a `let` chain.
    Lowest,
    Or,
    And,
    Comparison,
    Additive,
    Multiplicative,
    Cast,
    Unary,
    Postfix,
}

#[derive(Debug, Clone)]
struct Compiled {
    code: String,
    value_type: ValueType,
    /// For text: a `String` rather than a `&str`.
    is_owned: bool,
    precedence: Precedence,
}

impl Compiled {
    fn new(code: String, value_type: ValueType, precedence: Precedence) -> Self {
        Self {
            code,
            value_type,
            is_owned: false,
            precedence,
        }
    }

    fn owned_text(code: String, precedence: Precedence) -> Self {
        Self {
            code,
            value_type: ValueType::Text,
            is_owned: true,
            precedence,
        }
    }

    fn wrapped(&self, minimum: Precedence) -> String {
        if self.precedence < minimum {
            format!("({})", self.code)
        } else {
            self.code.clone()
        }
    }

    /// The text as a `String`.
    fn into_owned_code(self) -> String {
        if self.is_owned {
            self.code
        } else if self.code == "\"\"" {
            "String::new()".to_owned()
        } else {
            format!("{}.to_owned()", self.wrapped(Precedence::Postfix))
        }
    }

    /// The text as a `&str` argument.
    fn borrowed_code(&self) -> String {
        if self.is_owned {
            format!("&{}", self.wrapped(Precedence::Unary))
        } else {
            self.code.clone()
        }
    }

    fn is_plain_identifier(&self) -> bool {
        !self.code.is_empty()
            && self
                .code
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
    }
}

#[derive(Debug, Clone)]
pub struct Variable {
    pub rust_name: String,
    pub value_type: ValueType,
    pub is_owned: bool,
    /// For a catalogue accessor's parameter: its position, to tell which ones a body uses.
    pub parameter_index: Option<usize>,
}

pub type Scope = BTreeMap<String, Variable>;

/// A compiled condition. A `let` in it (narrowing) only works at the top of an `if`.
struct Condition {
    code: String,
    precedence: Precedence,
    has_let: bool,
    /// `x === null` compiled as `let Some(x) = x`: the branches swap.
    is_inverted: bool,
    narrowed_when_true: Vec<(String, Variable)>,
    narrowed_when_false: Vec<(String, Variable)>,
}

impl Condition {
    fn plain(code: String, precedence: Precedence) -> Self {
        Self {
            code,
            precedence,
            has_let: false,
            is_inverted: false,
            narrowed_when_true: Vec::new(),
            narrowed_when_false: Vec::new(),
        }
    }

    /// A `let` is never parenthesised: it only chains with `&&`.
    fn wrapped(&self, minimum: Precedence) -> String {
        if self.precedence < minimum && !self.has_let {
            format!("({})", self.code)
        } else {
            self.code.clone()
        }
    }
}

/// What the compiled code refers to outside itself.
#[derive(Debug, Default)]
pub struct Registry {
    /// Unions of string literals, each becoming one enum.
    pub choices: BTreeSet<Vec<String>>,
    /// Helper functions called, by language.
    pub called_helpers: BTreeSet<(CatalogueLanguage, String)>,
}

/// One language's module, as the compiler needs it.
pub struct ModuleContext<'a> {
    pub language: CatalogueLanguage,
    pub constants: &'a Constants,
    pub helpers: &'a BTreeMap<String, Function>,
}

pub struct FunctionCompiler<'a> {
    module: &'a ModuleContext<'a>,
    registry: &'a mut Registry,
    pub used_parameters: BTreeSet<usize>,
}

/// The Rust name of a helper function: the language is in the TypeScript name already.
pub fn helper_rust_name(name: &str) -> String {
    snake_case_identifier(name)
}

impl<'a> FunctionCompiler<'a> {
    pub fn new(module: &'a ModuleContext<'a>, registry: &'a mut Registry) -> Self {
        Self {
            module,
            registry,
            used_parameters: BTreeSet::new(),
        }
    }

    fn language_code(&self) -> &'static str {
        self.module.language.rust_variant()
    }

    /// An expression body returning text: the code of a `String`.
    pub fn compile_text_body(&mut self, expression: &Expression, scope: &Scope) -> Result<String> {
        let compiled = self.compile(expression, scope)?;
        ensure!(
            compiled.value_type == ValueType::Text,
            "the function returns {:?}, not text",
            compiled.value_type
        );
        Ok(compiled.into_owned_code())
    }

    /// A block body: its statements, ending with the returned `String` as the tail.
    pub fn compile_block_body(
        &mut self,
        statements: &[Statement],
        scope: &Scope,
        indent: &str,
    ) -> Result<String> {
        ensure!(
            matches!(statements.last(), Some(Statement::Return(_))),
            "a function body must end with `return`"
        );
        let mut mutable_names = BTreeSet::new();
        collect_pushed_names(statements, &mut mutable_names);
        self.compile_statements(statements, scope.clone(), indent, true, &mutable_names)
    }

    fn compile_statements(
        &mut self,
        statements: &[Statement],
        mut scope: Scope,
        indent: &str,
        is_function_end: bool,
        mutable_names: &BTreeSet<String>,
    ) -> Result<String> {
        let mut code = String::new();
        for (index, statement) in statements.iter().enumerate() {
            let is_last = is_function_end && index + 1 == statements.len();
            match statement {
                Statement::Const {
                    binding: Binding::Name(name),
                    value,
                } => {
                    let compiled = if is_list_expression(value) {
                        self.compile_list(value, &scope)?
                    } else {
                        self.compile(value, &scope)?
                    };
                    let rust_name = snake_case_identifier(name);
                    let mutability = if mutable_names.contains(name) {
                        "mut "
                    } else {
                        ""
                    };
                    writeln!(
                        code,
                        "{indent}let {mutability}{rust_name} = {};",
                        compiled.code
                    )?;
                    scope.insert(
                        name.clone(),
                        Variable {
                            rust_name,
                            value_type: compiled.value_type,
                            is_owned: compiled.is_owned,
                            parameter_index: None,
                        },
                    );
                }
                Statement::Const {
                    binding: Binding::ArrayPattern(names),
                    value,
                } => {
                    let list = self.compile_list(value, &scope)?;
                    for (position, name) in names.iter().enumerate() {
                        let rust_name = snake_case_identifier(name);
                        writeln!(
                            code,
                            "{indent}let {rust_name} = {};",
                            element_at(&list, position)?
                        )?;
                        scope.insert(
                            name.clone(),
                            Variable {
                                rust_name,
                                value_type: ValueType::OptionalText,
                                is_owned: false,
                                parameter_index: None,
                            },
                        );
                    }
                }
                Statement::If { condition, then } => {
                    let then_returns = matches!(then.last(), Some(Statement::Return(_)));
                    let compiled = self.condition(condition, &scope, true, then_returns)?;
                    let inner_indent = format!("{indent}    ");
                    if compiled.is_inverted {
                        // `if (x === undefined) { return …; }`: x is narrowed after the block.
                        let body = self.compile_statements(
                            then,
                            with_narrowing(&scope, &compiled.narrowed_when_true),
                            &inner_indent,
                            false,
                            mutable_names,
                        )?;
                        writeln!(code, "{indent}{} else {{\n{body}{indent}}};", compiled.code)?;
                        scope = with_narrowing(&scope, &compiled.narrowed_when_false);
                    } else {
                        let body = self.compile_statements(
                            then,
                            with_narrowing(&scope, &compiled.narrowed_when_true),
                            &inner_indent,
                            false,
                            mutable_names,
                        )?;
                        writeln!(code, "{indent}if {} {{\n{body}{indent}}}", compiled.code)?;
                    }
                }
                Statement::Return(value) => {
                    let returned = self.compile_text_body(value, &scope)?;
                    if is_last {
                        writeln!(code, "{indent}{returned}")?;
                    } else {
                        writeln!(code, "{indent}return {returned};")?;
                    }
                }
                Statement::Expression(expression) => {
                    writeln!(
                        code,
                        "{indent}{};",
                        self.push_statement(expression, &scope)?
                    )?;
                }
            }
        }
        Ok(code)
    }

    /// `list.push(text)`, the one statement besides declarations, `if` and `return`.
    fn push_statement(&mut self, expression: &Expression, scope: &Scope) -> Result<String> {
        let Expression::Call { callee, arguments } = expression else {
            bail!("only `list.push(text);` is supported as a statement");
        };
        let Expression::Member { object, property } = callee.as_ref() else {
            bail!("only `list.push(text);` is supported as a statement");
        };
        let (Expression::Identifier(name), "push", [argument]) =
            (object.as_ref(), property.as_str(), arguments.as_slice())
        else {
            bail!("only `list.push(text);` is supported as a statement");
        };
        let list = self.variable(name, scope)?;
        ensure!(
            matches!(list.value_type, ValueType::TextList(shape) if shape.has_owned_elements),
            "`{name}.push` needs a list built in the function"
        );
        let pushed = self.text(argument, scope)?;
        Ok(format!("{}.push({})", list.code, pushed.into_owned_code()))
    }

    fn variable(&mut self, name: &str, scope: &Scope) -> Result<Compiled> {
        let variable = scope
            .get(name)
            .ok_or_else(|| anyhow!("unknown name `{name}`"))?;
        if let Some(index) = variable.parameter_index {
            self.used_parameters.insert(index);
        }
        Ok(Compiled {
            code: variable.rust_name.clone(),
            value_type: variable.value_type.clone(),
            is_owned: variable.is_owned,
            precedence: Precedence::Postfix,
        })
    }

    fn compile(&mut self, expression: &Expression, scope: &Scope) -> Result<Compiled> {
        if let Some(constant) = self.constant(expression, scope) {
            return constant_code(&constant);
        }
        match expression {
            Expression::String(_) | Expression::Number(_) => unreachable_constant(expression),
            Expression::Template {
                quasis,
                expressions,
            } => self.template(quasis, expressions, scope),
            Expression::Identifier(name) => self.variable(name, scope),
            Expression::Conditional {
                condition,
                when_true,
                when_false,
            } => {
                let condition = self.condition(condition, scope, true, true)?;
                let when_true = self.compile(
                    when_true,
                    &with_narrowing(scope, &condition.narrowed_when_true),
                )?;
                let when_false = self.compile(
                    when_false,
                    &with_narrowing(scope, &condition.narrowed_when_false),
                )?;
                let (when_true, when_false) = unify(when_true, when_false)?;
                let (first, second) = if condition.is_inverted {
                    (when_false, when_true)
                } else {
                    (when_true, when_false)
                };
                Ok(Compiled {
                    code: format!(
                        "if {} {{ {} }} else {{ {} }}",
                        condition.code, first.code, second.code
                    ),
                    value_type: first.value_type,
                    is_owned: first.is_owned,
                    precedence: Precedence::Lowest,
                })
            }
            Expression::Binary {
                operator,
                left,
                right,
            } => self.binary(*operator, left, right, scope, expression),
            Expression::Unary {
                operator: UnaryOperator::Negate,
                operand,
            } => {
                let operand = self.number(operand, scope)?;
                Ok(Compiled::new(
                    format!("-{}", operand.wrapped(Precedence::Unary)),
                    ValueType::Number,
                    Precedence::Unary,
                ))
            }
            Expression::Unary {
                operator: UnaryOperator::Not,
                ..
            } => self.boolean(expression, scope),
            Expression::Call { callee, arguments } => self.call(callee, arguments, scope),
            Expression::Member { object, property } if property == "length" => {
                let list = self.compile_list(object, scope)?;
                Ok(Compiled::new(
                    format!("{}.len() as f64", list.wrapped(Precedence::Postfix)),
                    ValueType::Number,
                    Precedence::Cast,
                ))
            }
            Expression::Index { object, index } => self.index(object, index, scope),
            Expression::Array(_) => self.compile_list(expression, scope),
            _ => bail!("unsupported expression {}", short_description(expression)),
        }
    }

    /// `expression`'s value when it depends on no parameter or local.
    fn constant(&self, expression: &Expression, scope: &Scope) -> Option<ConstantValue> {
        if mentions_any(expression, scope) {
            return None;
        }
        fold(expression, self.module.constants)
    }

    fn number(&mut self, expression: &Expression, scope: &Scope) -> Result<Compiled> {
        let compiled = self.compile(expression, scope)?;
        ensure!(
            compiled.value_type == ValueType::Number,
            "{} is not a number",
            short_description(expression)
        );
        Ok(compiled)
    }

    fn text(&mut self, expression: &Expression, scope: &Scope) -> Result<Compiled> {
        let compiled = self.compile(expression, scope)?;
        ensure!(
            compiled.value_type == ValueType::Text,
            "{} is not text",
            short_description(expression)
        );
        Ok(compiled)
    }

    fn boolean(&mut self, expression: &Expression, scope: &Scope) -> Result<Compiled> {
        let condition = self.condition(expression, scope, false, false)?;
        Ok(Compiled::new(
            condition.code,
            ValueType::Boolean,
            condition.precedence,
        ))
    }

    fn template(
        &mut self,
        quasis: &[String],
        expressions: &[Expression],
        scope: &Scope,
    ) -> Result<Compiled> {
        if let ([first, last], [only]) = (quasis, expressions)
            && first.is_empty()
            && last.is_empty()
        {
            let compiled = self.compile(only, scope)?;
            if compiled.value_type == ValueType::Text {
                return Ok(compiled);
            }
        }
        let mut format_string = String::new();
        // The same text without placeholders, used when every part is constant.
        let mut constant_text = String::new();
        let mut has_placeholders = false;
        let mut arguments = Vec::new();
        for (index, quasi) in quasis.iter().enumerate() {
            format_string.push_str(&escape_format_braces(quasi));
            constant_text.push_str(quasi);
            let Some(expression) = expressions.get(index) else {
                continue;
            };
            if !mentions_any(expression, scope)
                && let Some(text) = fold_to_text(expression, self.module.constants)
            {
                format_string.push_str(&escape_format_braces(&text));
                constant_text.push_str(&text);
                continue;
            }
            let compiled = self.compile(expression, scope)?;
            let argument = match &compiled.value_type {
                ValueType::Text | ValueType::Choice(_) => compiled,
                ValueType::Number => Compiled::owned_text(
                    format!("super::format_number_like_javascript({})", compiled.code),
                    Precedence::Postfix,
                ),
                ValueType::OptionalText => Compiled::new(
                    // Only lists' `at`, indexing and destructuring give optional text here.
                    format!(
                        "{}.unwrap_or(\"undefined\")",
                        compiled.wrapped(Precedence::Postfix)
                    ),
                    ValueType::Text,
                    Precedence::Postfix,
                ),
                other => bail!("cannot write {other:?} in a template string"),
            };
            has_placeholders = true;
            if argument.is_plain_identifier() && !argument.code.starts_with(char::is_numeric) {
                write!(format_string, "{{{}}}", argument.code)?;
            } else {
                format_string.push_str("{}");
                arguments.push(argument.code);
            }
        }
        if !has_placeholders {
            return Ok(Compiled::new(
                rust_string_literal(&constant_text),
                ValueType::Text,
                Precedence::Postfix,
            ));
        }
        let mut code = format!("format!({}", rust_string_literal(&format_string));
        for argument in arguments {
            code.push_str(", ");
            code.push_str(&argument);
        }
        code.push(')');
        Ok(Compiled::owned_text(code, Precedence::Postfix))
    }

    fn binary(
        &mut self,
        operator: BinaryOperator,
        left: &Expression,
        right: &Expression,
        scope: &Scope,
        whole: &Expression,
    ) -> Result<Compiled> {
        let arithmetic = match operator {
            BinaryOperator::Add => Some(("+", Precedence::Additive)),
            BinaryOperator::Subtract => Some(("-", Precedence::Additive)),
            BinaryOperator::Multiply => Some(("*", Precedence::Multiplicative)),
            BinaryOperator::Divide => Some(("/", Precedence::Multiplicative)),
            BinaryOperator::Remainder => Some(("%", Precedence::Multiplicative)),
            _ => None,
        };
        if let Some((symbol, precedence)) = arithmetic {
            let left = self.number(left, scope)?;
            let right = self.number(right, scope)?;
            let right_minimum = if precedence == Precedence::Additive {
                Precedence::Multiplicative
            } else {
                Precedence::Cast
            };
            return Ok(Compiled::new(
                format!(
                    "{} {symbol} {}",
                    left.wrapped(precedence),
                    right.wrapped(right_minimum)
                ),
                ValueType::Number,
                precedence,
            ));
        }
        match operator {
            BinaryOperator::NullishCoalescing => {
                let left = self.compile(left, scope)?;
                let right = self.compile(right, scope)?;
                match (&left.value_type, &right.value_type) {
                    (ValueType::OptionalText, ValueType::Text) if !right.is_owned => {
                        Ok(Compiled::new(
                            format!(
                                "{}.unwrap_or({})",
                                left.wrapped(Precedence::Postfix),
                                right.code
                            ),
                            ValueType::Text,
                            Precedence::Postfix,
                        ))
                    }
                    (ValueType::OptionalText, ValueType::Text) => Ok(Compiled::owned_text(
                        format!(
                            "{}.map_or_else(|| {}, str::to_owned)",
                            left.wrapped(Precedence::Postfix),
                            right.code
                        ),
                        Precedence::Postfix,
                    )),
                    (ValueType::OptionalNumber, ValueType::Number) => Ok(Compiled::new(
                        format!(
                            "{}.unwrap_or({})",
                            left.wrapped(Precedence::Postfix),
                            right.code
                        ),
                        ValueType::Number,
                        Precedence::Postfix,
                    )),
                    _ => bail!(
                        "unsupported `??` between {:?} and {:?}",
                        left.value_type,
                        right.value_type
                    ),
                }
            }
            BinaryOperator::Or => {
                let first = self.compile(left, scope)?;
                if first.value_type != ValueType::Text {
                    return self.boolean(whole, scope);
                }
                // `text || fallback`: the fallback when the text is empty.
                ensure!(
                    first.is_plain_identifier(),
                    "`||` on text needs a name on its left"
                );
                let fallback = self.text(right, scope)?;
                let check = format!("{}.is_empty()", first.code);
                let (fallback, first) = unify(fallback, first)?;
                Ok(Compiled {
                    code: format!(
                        "if {check} {{ {} }} else {{ {} }}",
                        fallback.code, first.code
                    ),
                    value_type: ValueType::Text,
                    is_owned: first.is_owned,
                    precedence: Precedence::Lowest,
                })
            }
            _ => self.boolean(whole, scope),
        }
    }

    fn call(
        &mut self,
        callee: &Expression,
        arguments: &[Expression],
        scope: &Scope,
    ) -> Result<Compiled> {
        let language = self.language_code();
        match callee {
            Expression::Identifier(name) if name == "count" && !scope.contains_key(name) => {
                let [value] = arguments else {
                    bail!("`count` takes one number");
                };
                let value = self.number(value, scope)?;
                Ok(Compiled::owned_text(
                    format!("super::format_count({language}, {})", value.code),
                    Precedence::Postfix,
                ))
            }
            Expression::Identifier(name) if name == "plural" && !scope.contains_key(name) => {
                let [count, Expression::Object(forms)] = arguments else {
                    bail!("`plural` takes a number and `{{ one, other }}`");
                };
                let form = |key: &str| {
                    forms
                        .iter()
                        .find(|property| property.key == key)
                        .map(|property| &property.value)
                        .ok_or_else(|| anyhow!("`plural` needs the `{key}` form"))
                };
                ensure!(forms.len() == 2, "`plural` takes exactly `one` and `other`");
                let count = self.number(count, scope)?;
                let one = self.text(form("one")?, scope)?;
                let other = self.text(form("other")?, scope)?;
                let (one, other) = unify(one, other)?;
                Ok(Compiled {
                    code: format!(
                        "if super::is_plural_one({language}, {}) {{ {} }} else {{ {} }}",
                        count.code, one.code, other.code
                    ),
                    value_type: ValueType::Text,
                    is_owned: one.is_owned,
                    precedence: Precedence::Lowest,
                })
            }
            Expression::Identifier(name) if self.module.helpers.contains_key(name) => {
                self.helper_call(name, arguments, scope)
            }
            Expression::Member { object, property } => {
                self.method_call(object, property, arguments, scope)
            }
            _ => bail!("unsupported call of {}", short_description(callee)),
        }
    }

    fn helper_call(
        &mut self,
        name: &str,
        arguments: &[Expression],
        scope: &Scope,
    ) -> Result<Compiled> {
        let helpers = self.module.helpers;
        let helper = helpers
            .get(name)
            .ok_or_else(|| anyhow!("unknown function `{name}`"))?;
        ensure!(
            helper.parameters.len() == arguments.len(),
            "`{name}` takes {} arguments",
            helper.parameters.len()
        );
        let mut codes = Vec::new();
        for (parameter, argument) in helper.parameters.iter().zip(arguments) {
            let annotation = parameter
                .annotation
                .as_ref()
                .ok_or_else(|| anyhow!("`{name}`'s parameters need types"))?;
            let expected = ValueType::from_annotation(annotation);
            let compiled = if matches!(expected, ValueType::TextList(_)) {
                self.compile_list(argument, scope)?
            } else {
                self.compile(argument, scope)?
            };
            let code = match (&expected, &compiled.value_type) {
                (ValueType::Text, ValueType::Text) => compiled.borrowed_code(),
                (ValueType::TextList(_), ValueType::TextList(shape)) => {
                    ensure!(
                        !shape.has_owned_elements,
                        "`{name}` takes the caller's list, not a built one"
                    );
                    if shape.needs_borrow_to_pass {
                        format!("&{}", compiled.wrapped(Precedence::Unary))
                    } else {
                        compiled.code
                    }
                }
                (expected, found) if expected == found => compiled.code,
                (expected, found) => {
                    bail!("`{name}` takes {expected:?}, not {found:?}")
                }
            };
            codes.push(code);
        }
        self.registry
            .called_helpers
            .insert((self.module.language, name.to_owned()));
        Ok(Compiled::owned_text(
            format!("{}({})", helper_rust_name(name), codes.join(", ")),
            Precedence::Postfix,
        ))
    }

    fn method_call(
        &mut self,
        object: &Expression,
        method: &str,
        arguments: &[Expression],
        scope: &Scope,
    ) -> Result<Compiled> {
        match (object, method, arguments) {
            (Expression::Identifier(math), "abs", [value])
                if math == "Math" && !scope.contains_key(math) =>
            {
                let value = self.number(value, scope)?;
                Ok(Compiled::new(
                    format!("{}.abs()", value.wrapped(Precedence::Postfix)),
                    ValueType::Number,
                    Precedence::Postfix,
                ))
            }
            (value, "toLocaleString", [Expression::String(tag), options @ ..]) => {
                let language = CatalogueLanguage::from_tag(tag)?.rust_variant();
                let value = self.number(value, scope)?;
                let code = match options {
                    [] => format!("super::format_count({language}, {})", value.code),
                    [Expression::Object(options)] => {
                        let digits = |key: &str| -> Result<usize> {
                            let digits = options
                                .iter()
                                .find(|property| property.key == key)
                                .and_then(|property| fold(&property.value, self.module.constants));
                            match digits {
                                Some(ConstantValue::Number(digits))
                                    if digits.fract() == 0.0 && (0.0..=20.0).contains(&digits) =>
                                {
                                    Ok(javascript_number_to_string(digits).parse()?)
                                }
                                _ => bail!("`toLocaleString` needs a constant `{key}`"),
                            }
                        };
                        ensure!(
                            options.len() == 2,
                            "`toLocaleString` supports only minimum and maximum fraction digits"
                        );
                        format!(
                            "super::format_number({language}, {}, {}, {})",
                            value.code,
                            digits("minimumFractionDigits")?,
                            digits("maximumFractionDigits")?
                        )
                    }
                    _ => bail!("unsupported `toLocaleString` options"),
                };
                Ok(Compiled::owned_text(code, Precedence::Postfix))
            }
            (list, "join", [Expression::String(separator)]) => {
                let list = self.compile_list(list, scope)?;
                Ok(Compiled::owned_text(
                    format!(
                        "{}.join({})",
                        list.wrapped(Precedence::Postfix),
                        rust_string_literal(separator)
                    ),
                    Precedence::Postfix,
                ))
            }
            (list, "at", [index]) if is_minus_one(index) => {
                let list = self.compile_list(list, scope)?;
                let element = if element_owned(&list) {
                    ".map(String::as_str)"
                } else {
                    ".copied()"
                };
                Ok(Compiled::new(
                    format!("{}.last(){element}", list.wrapped(Precedence::Postfix)),
                    ValueType::OptionalText,
                    Precedence::Postfix,
                ))
            }
            _ => bail!("unsupported method call `.{method}(…)`"),
        }
    }

    fn index(
        &mut self,
        object: &Expression,
        index: &Expression,
        scope: &Scope,
    ) -> Result<Compiled> {
        if let Expression::Identifier(name) = object
            && !scope.contains_key(name)
            && let Some(ConstantValue::Object(fields)) = self.module.constants.get(name)
        {
            // `UNITS[unit]`: one arm per literal of the union.
            let choice = self.compile(index, scope)?;
            let ValueType::Choice(literals) = &choice.value_type else {
                bail!("`{name}[…]` needs a union of string literals as its index");
            };
            let enum_name = choice_enum_name(literals);
            let mut arms = Vec::new();
            for literal in literals {
                let Some((_, ConstantValue::Text(text))) =
                    fields.iter().find(|(key, _)| key == literal)
                else {
                    bail!("`{name}` has no text for `{literal}`");
                };
                arms.push(format!(
                    "{enum_name}::{} => {}",
                    pascal_case(literal),
                    rust_string_literal(text)
                ));
            }
            return Ok(Compiled::new(
                format!("match {} {{ {} }}", choice.code, arms.join(", ")),
                ValueType::Text,
                Precedence::Lowest,
            ));
        }
        let list = self.compile_list(object, scope)?;
        let Some(ConstantValue::Number(position)) = fold(index, self.module.constants) else {
            bail!("lists are indexed by constant positions only");
        };
        ensure!(
            position.fract() == 0.0 && (0.0..1e6).contains(&position),
            "unsupported list position {position}"
        );
        let position: usize = javascript_number_to_string(position).parse()?;
        Ok(Compiled::new(
            element_at(&list, position)?,
            ValueType::OptionalText,
            Precedence::Postfix,
        ))
    }

    /// A list: a parameter or local, an array literal (built into a `Vec<String>`), its
    /// `.filter(Boolean)`, or `name.slice(0, -1)`.
    fn compile_list(&mut self, expression: &Expression, scope: &Scope) -> Result<Compiled> {
        match expression {
            Expression::Array(elements) => self.build_list(elements, false, scope),
            Expression::Call { callee, arguments } => match (callee.as_ref(), arguments.as_slice())
            {
                (Expression::Member { object, property }, [Expression::Identifier(boolean)])
                    if property == "filter" && boolean == "Boolean" =>
                {
                    let Expression::Array(elements) = object.as_ref() else {
                        bail!("`.filter(Boolean)` is supported on array literals only");
                    };
                    self.build_list(elements, true, scope)
                }
                (Expression::Member { object, property }, [Expression::Number(start), end])
                    if property == "slice" && *start == 0.0 && is_minus_one(end) =>
                {
                    let list = self.compile_list(object, scope)?;
                    ensure!(
                        list.is_plain_identifier(),
                        "`.slice(0, -1)` is supported on names only"
                    );
                    let ValueType::TextList(shape) = list.value_type else {
                        bail!("`.slice` needs a list");
                    };
                    Ok(Compiled::new(
                        format!("{0}[..{0}.len().saturating_sub(1)]", list.code),
                        ValueType::TextList(ListShape {
                            has_owned_elements: shape.has_owned_elements,
                            needs_borrow_to_pass: true,
                        }),
                        Precedence::Postfix,
                    ))
                }
                _ => bail!("unsupported list {}", short_description(expression)),
            },
            _ => {
                let compiled = self.compile(expression, scope)?;
                ensure!(
                    matches!(compiled.value_type, ValueType::TextList(_)),
                    "{} is not a list",
                    short_description(expression)
                );
                Ok(compiled)
            }
        }
    }

    fn build_list(
        &mut self,
        elements: &[ArrayElement],
        drops_empty_text: bool,
        scope: &Scope,
    ) -> Result<Compiled> {
        let mut code = String::from("{ let mut parts: Vec<String> = Vec::new(); ");
        self.push_elements(elements, drops_empty_text, scope, &mut code)?;
        code.push_str("parts }");
        Ok(Compiled::new(
            code,
            ValueType::TextList(BUILT_LIST),
            Precedence::Lowest,
        ))
    }

    fn push_elements(
        &mut self,
        elements: &[ArrayElement],
        drops_empty_text: bool,
        scope: &Scope,
        code: &mut String,
    ) -> Result<()> {
        for element in elements {
            match element {
                ArrayElement::Item(Expression::Binary {
                    operator: BinaryOperator::And,
                    left,
                    right,
                }) if drops_empty_text => {
                    // `condition && text`: `false` is dropped by `.filter(Boolean)`.
                    let condition = self.condition(left, scope, true, false)?;
                    write!(code, "if {} {{ ", condition.code)?;
                    self.push_elements(
                        &[ArrayElement::Item(right.as_ref().clone())],
                        drops_empty_text,
                        &with_narrowing(scope, &condition.narrowed_when_true),
                        code,
                    )?;
                    code.push_str("}\n");
                }
                ArrayElement::Item(Expression::String(text))
                    if drops_empty_text && text.is_empty() => {}
                ArrayElement::Item(item) => {
                    let text = self.text(item, scope)?;
                    let is_known_non_empty =
                        matches!(item, Expression::String(text) if !text.is_empty());
                    if drops_empty_text && !is_known_non_empty {
                        writeln!(
                            code,
                            "{{ let part = {}; if !part.is_empty() {{ parts.push({}); }} }}",
                            text.code,
                            Compiled {
                                code: "part".to_owned(),
                                precedence: Precedence::Postfix,
                                ..text.clone()
                            }
                            .into_owned_code()
                        )?;
                    } else {
                        write!(code, "parts.push({}); ", text.into_owned_code())?;
                    }
                }
                ArrayElement::Spread(Expression::Array(inner)) => {
                    self.push_elements(inner, drops_empty_text, scope, code)?;
                }
                ArrayElement::Spread(Expression::Conditional {
                    condition,
                    when_true,
                    when_false,
                }) => {
                    let (Expression::Array(when_true), Expression::Array(when_false)) =
                        (when_true.as_ref(), when_false.as_ref())
                    else {
                        bail!("a spread condition must choose between array literals");
                    };
                    let condition = self.condition(condition, scope, true, false)?;
                    write!(code, "if {} {{ ", condition.code)?;
                    self.push_elements(
                        when_true,
                        drops_empty_text,
                        &with_narrowing(scope, &condition.narrowed_when_true),
                        code,
                    )?;
                    code.push('}');
                    if !when_false.is_empty() {
                        ensure!(!condition.has_let, "unsupported narrowing in a spread");
                        code.push_str(" else { ");
                        self.push_elements(when_false, drops_empty_text, scope, code)?;
                        code.push('}');
                    }
                    code.push('\n');
                }
                ArrayElement::Spread(list) => {
                    let list = self.compile_list(list, scope)?;
                    let filter = if drops_empty_text {
                        ".filter(|part| !part.is_empty())"
                    } else {
                        ""
                    };
                    let elements = if element_owned(&list) {
                        ".cloned()"
                    } else {
                        ".copied().map(str::to_owned)"
                    };
                    write!(
                        code,
                        "parts.extend({}.iter(){filter}{elements}); ",
                        list.wrapped(Precedence::Postfix)
                    )?;
                }
            }
        }
        Ok(())
    }

    /// `===` and `!==`: against `null` or `undefined` (narrowing), between numbers, or between
    /// text or a union and a string literal.
    fn equality(
        &mut self,
        is_equal: bool,
        left: &Expression,
        right: &Expression,
        scope: &Scope,
        allows_let: bool,
        allows_inversion: bool,
    ) -> Result<Condition> {
        let absent_compared = match (left, right) {
            (value, Expression::Null | Expression::Undefined)
            | (Expression::Null | Expression::Undefined, value) => Some(value),
            _ => None,
        };
        if let Some(value) = absent_compared {
            return self.absence_check(value, is_equal, scope, allows_let, allows_inversion);
        }
        let left = self.compile(left, scope)?;
        let right_expression = right;
        let right = self.compile(right, scope)?;
        let (operator_code, negation) = if is_equal { ("==", "") } else { ("!=", "!") };
        let code = match (&left.value_type, &right.value_type, right_expression) {
            (ValueType::Number, ValueType::Number, _) => format!(
                "{} {operator_code} {}",
                left.wrapped(Precedence::Unary),
                right.wrapped(Precedence::Additive)
            ),
            (ValueType::Text, ValueType::Text, Expression::String(text)) if text.is_empty() => {
                return Ok(Condition::plain(
                    format!("{negation}{}.is_empty()", left.wrapped(Precedence::Postfix)),
                    if is_equal {
                        Precedence::Postfix
                    } else {
                        Precedence::Unary
                    },
                ));
            }
            (ValueType::Text, ValueType::Text, Expression::String(_)) => format!(
                "{} {operator_code} {}",
                left.wrapped(Precedence::Additive),
                right.code
            ),
            (ValueType::Choice(literals), ValueType::Text, Expression::String(text)) => {
                ensure!(
                    literals.contains(text),
                    "`{text}` is not one of {literals:?}"
                );
                format!(
                    "{} {operator_code} {}::{}",
                    left.code,
                    choice_enum_name(literals),
                    pascal_case(text)
                )
            }
            (left_type, right_type, _) => {
                bail!("cannot compare {left_type:?} with {right_type:?}")
            }
        };
        Ok(Condition::plain(code, Precedence::Comparison))
    }

    /// A condition, with TypeScript's narrowing when `allows_let` (it heads an `if`) and the
    /// `x === null` form when `allows_inversion` (its branches can swap).
    fn condition(
        &mut self,
        expression: &Expression,
        scope: &Scope,
        allows_let: bool,
        allows_inversion: bool,
    ) -> Result<Condition> {
        match expression {
            Expression::Binary {
                operator: operator @ (BinaryOperator::StrictEqual | BinaryOperator::StrictNotEqual),
                left,
                right,
            } => self.equality(
                *operator == BinaryOperator::StrictEqual,
                left,
                right,
                scope,
                allows_let,
                allows_inversion,
            ),
            Expression::Binary {
                operator:
                    operator @ (BinaryOperator::Less
                    | BinaryOperator::LessOrEqual
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterOrEqual),
                left,
                right,
            } => {
                let symbol = match operator {
                    BinaryOperator::Less => "<",
                    BinaryOperator::LessOrEqual => "<=",
                    BinaryOperator::Greater => ">",
                    _ => ">=",
                };
                let left = self.number(left, scope)?;
                let right = self.number(right, scope)?;
                Ok(Condition::plain(
                    // Above `as`: `x as f64 < y` would parse as generic arguments.
                    format!(
                        "{} {symbol} {}",
                        left.wrapped(Precedence::Unary),
                        right.wrapped(Precedence::Additive)
                    ),
                    Precedence::Comparison,
                ))
            }
            Expression::Binary {
                operator: BinaryOperator::And,
                left,
                right,
            } => {
                let left = self.condition(left, scope, allows_let, false)?;
                let right = self.condition(
                    right,
                    &with_narrowing(scope, &left.narrowed_when_true),
                    allows_let,
                    false,
                )?;
                let mut narrowed_when_true = left.narrowed_when_true.clone();
                narrowed_when_true.extend(right.narrowed_when_true.iter().cloned());
                Ok(Condition {
                    code: format!(
                        "{} && {}",
                        left.wrapped(Precedence::And),
                        right.wrapped(Precedence::Comparison)
                    ),
                    precedence: Precedence::And,
                    has_let: left.has_let || right.has_let,
                    is_inverted: false,
                    narrowed_when_true,
                    narrowed_when_false: Vec::new(),
                })
            }
            Expression::Binary {
                operator: BinaryOperator::Or,
                left,
                right,
            } => {
                let left = self.condition(left, scope, false, false)?;
                let right = self.condition(right, scope, false, false)?;
                Ok(Condition::plain(
                    format!(
                        "{} || {}",
                        left.wrapped(Precedence::Or),
                        right.wrapped(Precedence::And)
                    ),
                    Precedence::Or,
                ))
            }
            Expression::Unary {
                operator: UnaryOperator::Not,
                operand,
            } => {
                let operand = self.condition(operand, scope, false, false)?;
                Ok(Condition::plain(
                    format!("!{}", operand.wrapped(Precedence::Unary)),
                    Precedence::Unary,
                ))
            }
            _ => self.truthiness(expression, scope, allows_let),
        }
    }

    fn absence_check(
        &mut self,
        value: &Expression,
        is_equal: bool,
        scope: &Scope,
        allows_let: bool,
        allows_inversion: bool,
    ) -> Result<Condition> {
        let compiled = self.compile(value, scope)?;
        let narrowed_type = match compiled.value_type {
            ValueType::OptionalText => ValueType::Text,
            ValueType::OptionalNumber => ValueType::Number,
            ref other => bail!("{other:?} is never null or undefined"),
        };
        let can_narrow = allows_let && (allows_inversion || !is_equal);
        if let (Expression::Identifier(name), true) = (value, can_narrow) {
            let narrowed = vec![(
                name.clone(),
                Variable {
                    rust_name: compiled.code.clone(),
                    value_type: narrowed_type,
                    is_owned: false,
                    parameter_index: scope
                        .get(name)
                        .and_then(|variable| variable.parameter_index),
                },
            )];
            return Ok(Condition {
                code: format!("let Some({0}) = {0}", compiled.code),
                precedence: Precedence::Lowest,
                has_let: true,
                is_inverted: is_equal,
                narrowed_when_true: if is_equal {
                    Vec::new()
                } else {
                    narrowed.clone()
                },
                narrowed_when_false: if is_equal { narrowed } else { Vec::new() },
            });
        }
        let method = if is_equal { "is_none" } else { "is_some" };
        Ok(Condition::plain(
            format!("{}.{method}()", compiled.wrapped(Precedence::Postfix)),
            Precedence::Postfix,
        ))
    }

    fn truthiness(
        &mut self,
        expression: &Expression,
        scope: &Scope,
        allows_let: bool,
    ) -> Result<Condition> {
        let compiled = self.compile(expression, scope)?;
        match compiled.value_type {
            ValueType::Boolean => Ok(Condition::plain(compiled.code, compiled.precedence)),
            ValueType::Text => Ok(Condition::plain(
                format!("!{}.is_empty()", compiled.wrapped(Precedence::Postfix)),
                Precedence::Unary,
            )),
            ValueType::Number => Ok(Condition::plain(
                format!("{} != 0.0", compiled.wrapped(Precedence::Additive)),
                Precedence::Comparison,
            )),
            ValueType::OptionalText => {
                if let (Expression::Identifier(name), true) = (expression, allows_let) {
                    return Ok(Condition {
                        code: format!("let Some({0}) = {0} && !{0}.is_empty()", compiled.code),
                        precedence: Precedence::Lowest,
                        has_let: true,
                        is_inverted: false,
                        narrowed_when_true: vec![(
                            name.clone(),
                            Variable {
                                rust_name: compiled.code.clone(),
                                value_type: ValueType::Text,
                                is_owned: false,
                                parameter_index: scope
                                    .get(name)
                                    .and_then(|variable| variable.parameter_index),
                            },
                        )],
                        narrowed_when_false: Vec::new(),
                    });
                }
                Ok(Condition::plain(
                    format!(
                        "{}.is_some_and(|text| !text.is_empty())",
                        compiled.wrapped(Precedence::Postfix)
                    ),
                    Precedence::Postfix,
                ))
            }
            other => bail!("cannot use {other:?} as a condition"),
        }
    }
}

/// Compiles a helper function (`function placesInEnglish(…)`) into a private Rust function.
pub fn compile_helper(
    function: &Function,
    module: &ModuleContext<'_>,
    registry: &mut Registry,
) -> Result<String> {
    let mut scope = Scope::new();
    let mut parameters = Vec::new();
    for parameter in &function.parameters {
        let annotation = parameter
            .annotation
            .as_ref()
            .ok_or_else(|| anyhow!("`{}`'s parameters need types", function.name))?;
        let value_type = ValueType::from_annotation(annotation);
        if let ValueType::Choice(literals) = &value_type {
            registry.choices.insert(literals.clone());
        }
        let rust_name = snake_case_identifier(&parameter.name);
        parameters.push(format!(
            "{rust_name}: {}",
            value_type.rust_parameter_type(false)
        ));
        scope.insert(
            parameter.name.clone(),
            Variable {
                rust_name,
                value_type,
                is_owned: false,
                parameter_index: None,
            },
        );
    }
    let body = FunctionCompiler::new(module, registry)
        .compile_block_body(&function.body, &scope, "    ")
        .map_err(|error| anyhow!("in `{}`: {error:#}", function.name))?;
    Ok(format!(
        "fn {}({}) -> String {{\n{body}}}\n",
        helper_rust_name(&function.name),
        parameters.join(", ")
    ))
}

fn constant_code(constant: &ConstantValue) -> Result<Compiled> {
    match constant {
        ConstantValue::Text(text) => Ok(Compiled::new(
            rust_string_literal(text),
            ValueType::Text,
            Precedence::Postfix,
        )),
        ConstantValue::Number(number) => {
            ensure!(number.is_finite(), "unsupported number {number}");
            let code = format!("{number:?}");
            let precedence = if code.starts_with('-') {
                Precedence::Unary
            } else {
                Precedence::Postfix
            };
            Ok(Compiled::new(code, ValueType::Number, precedence))
        }
        ConstantValue::Object(_) => bail!("an object constant is not a value here"),
    }
}

fn unreachable_constant(expression: &Expression) -> Result<Compiled> {
    bail!(
        "{} should have folded into a constant",
        short_description(expression)
    )
}

/// Two branches of one value: the same type, and text owned on both sides when either is.
fn unify(first: Compiled, second: Compiled) -> Result<(Compiled, Compiled)> {
    ensure!(
        first.value_type == second.value_type,
        "the branches give {:?} and {:?}",
        first.value_type,
        second.value_type
    );
    if first.value_type != ValueType::Text || first.is_owned == second.is_owned {
        return Ok((first, second));
    }
    let owned = |compiled: Compiled| {
        if compiled.is_owned {
            compiled
        } else {
            Compiled::owned_text(compiled.into_owned_code(), Precedence::Postfix)
        }
    };
    Ok((owned(first), owned(second)))
}

fn element_owned(list: &Compiled) -> bool {
    matches!(list.value_type, ValueType::TextList(shape) if shape.has_owned_elements)
}

fn element_at(list: &Compiled, position: usize) -> Result<String> {
    ensure!(
        matches!(list.value_type, ValueType::TextList(_)),
        "only lists can be indexed"
    );
    let access = if position == 0 {
        "first()".to_owned()
    } else {
        format!("get({position})")
    };
    let element = if element_owned(list) {
        ".map(String::as_str)"
    } else {
        ".copied()"
    };
    Ok(format!(
        "{}.{access}{element}",
        list.wrapped(Precedence::Postfix)
    ))
}

fn with_narrowing(scope: &Scope, narrowed: &[(String, Variable)]) -> Scope {
    let mut narrowed_scope = scope.clone();
    for (name, variable) in narrowed {
        narrowed_scope.insert(name.clone(), variable.clone());
    }
    narrowed_scope
}

fn is_minus_one(expression: &Expression) -> bool {
    matches!(
        expression,
        Expression::Unary { operator: UnaryOperator::Negate, operand }
            if matches!(operand.as_ref(), Expression::Number(one) if one.to_bits() == 1.0_f64.to_bits())
    )
}

fn is_list_expression(expression: &Expression) -> bool {
    matches!(expression, Expression::Array(_))
}

fn escape_format_braces(text: &str) -> String {
    text.replace('{', "{{").replace('}', "}}")
}

/// Names that `list.push(…)` changes, declared `let mut`.
fn collect_pushed_names(statements: &[Statement], names: &mut BTreeSet<String>) {
    for statement in statements {
        match statement {
            Statement::Expression(Expression::Call { callee, .. }) => {
                if let Expression::Member { object, property } = callee.as_ref()
                    && let Expression::Identifier(name) = object.as_ref()
                    && property == "push"
                {
                    names.insert(name.clone());
                }
            }
            Statement::If { then, .. } => collect_pushed_names(then, names),
            _ => {}
        }
    }
}

/// Whether `expression` reads a name of `scope`: then it is not a constant.
fn mentions_any(expression: &Expression, scope: &Scope) -> bool {
    let mentions = |expression: &Expression| mentions_any(expression, scope);
    match expression {
        Expression::Identifier(name) => scope.contains_key(name),
        Expression::String(_)
        | Expression::Number(_)
        | Expression::Null
        | Expression::Undefined
        | Expression::Arrow(_) => false,
        Expression::Template { expressions, .. } => expressions.iter().any(mentions),
        Expression::Object(properties) => {
            properties.iter().any(|property| mentions(&property.value))
        }
        Expression::Array(elements) => elements.iter().any(|element| match element {
            ArrayElement::Item(item) | ArrayElement::Spread(item) => mentions(item),
        }),
        Expression::Unary { operand, .. } => mentions(operand),
        Expression::Binary { left, right, .. } => mentions(left) || mentions(right),
        Expression::Conditional {
            condition,
            when_true,
            when_false,
        } => mentions(condition) || mentions(when_true) || mentions(when_false),
        Expression::Call { callee, arguments } => {
            mentions(callee) || arguments.iter().any(mentions)
        }
        Expression::Member { object, .. } | Expression::TextRecord(object) => mentions(object),
        Expression::Index { object, index } => mentions(object) || mentions(index),
    }
}

fn short_description(expression: &Expression) -> String {
    match expression {
        Expression::Identifier(name) => format!("`{name}`"),
        Expression::Member { property, .. } => format!("`….{property}`"),
        Expression::Call { callee, .. } => format!("a call of {}", short_description(callee)),
        Expression::String(_) | Expression::Template { .. } => "a string".to_owned(),
        Expression::Number(number) => format!("`{number}`"),
        Expression::Arrow(_) => "a nested function".to_owned(),
        Expression::Object(_) => "an object".to_owned(),
        Expression::Array(_) => "an array".to_owned(),
        Expression::Null => "`null`".to_owned(),
        Expression::Undefined => "`undefined`".to_owned(),
        Expression::Unary { .. } => "a unary operation".to_owned(),
        Expression::Binary { operator, .. } => format!("a `{operator:?}` operation"),
        Expression::Conditional { .. } => "a conditional".to_owned(),
        Expression::Index { .. } => "an index".to_owned(),
        Expression::TextRecord(_) => "a record".to_owned(),
    }
}
