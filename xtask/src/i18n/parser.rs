//! Parses the catalogues' tokens into a small syntax tree: the module's imports, constants, helper
//! functions and the exported catalogue object. The grammar is the subset the catalogues use;
//! every other construct is an error that names its line.

use anyhow::{Result, anyhow, bail};

use super::lexer::{Template, Token, TokenKind};

#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    String(String),
    Number(f64),
    Template {
        quasis: Vec<String>,
        expressions: Vec<Expression>,
    },
    Identifier(String),
    Null,
    Undefined,
    Arrow(Box<Arrow>),
    Object(Vec<Property>),
    Array(Vec<ArrayElement>),
    Unary {
        operator: UnaryOperator,
        operand: Box<Expression>,
    },
    Binary {
        operator: BinaryOperator,
        left: Box<Expression>,
        right: Box<Expression>,
    },
    Conditional {
        condition: Box<Expression>,
        when_true: Box<Expression>,
        when_false: Box<Expression>,
    },
    Call {
        callee: Box<Expression>,
        arguments: Vec<Expression>,
    },
    Member {
        object: Box<Expression>,
        property: String,
    },
    Index {
        object: Box<Expression>,
        index: Box<Expression>,
    },
    /// `{ … } as Record<string, string>`: text looked up by a key only known at run time.
    TextRecord(Box<Expression>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOperator {
    Not,
    Negate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOperator {
    Or,
    NullishCoalescing,
    And,
    StrictEqual,
    StrictNotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub key: String,
    pub value: Expression,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ArrayElement {
    Item(Expression),
    Spread(Expression),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arrow {
    pub parameters: Vec<Parameter>,
    pub body: FunctionBody,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    pub name: String,
    pub annotation: Option<TypeAnnotation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeAnnotation {
    String,
    Number,
    Boolean,
    NullableString,
    NullableNumber,
    StringList,
    /// `"a" | "b"`: the literals in their written order.
    StringUnion(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum FunctionBody {
    Expression(Box<Expression>),
    Block(Vec<Statement>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Const {
        binding: Binding,
        value: Expression,
    },
    If {
        condition: Expression,
        then: Vec<Statement>,
    },
    Return(Expression),
    Expression(Expression),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    Name(String),
    /// `const [first, second] = list;`
    ArrayPattern(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub names: Vec<String>,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: String,
    pub parameters: Vec<Parameter>,
    pub body: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Module {
    /// Value imports; `import type` brings nothing a catalogue's value can use.
    pub imports: Vec<Import>,
    /// Top-level `const`s, exported or not, in source order.
    pub constants: Vec<(String, Expression)>,
    pub functions: Vec<Function>,
}

pub fn parse_module(tokens: Vec<Token>) -> Result<Module> {
    let mut parser = Parser::new(tokens);
    let mut module = Module::default();
    while !parser.is_at_end() {
        if parser.eat_identifier("import") {
            let is_type_only = parser.eat_identifier("type");
            parser.expect("{")?;
            let names = parser.comma_separated_names("}")?;
            parser.expect_identifier("from")?;
            let path = parser.string()?;
            parser.expect(";")?;
            if !is_type_only {
                module.imports.push(Import { names, path });
            }
            continue;
        }
        let is_exported = parser.eat_identifier("export");
        if is_exported && parser.eat_identifier("type") {
            parser.skip_past_top_level(";")?;
        } else if parser.eat_identifier("const") {
            let name = parser.identifier()?;
            if parser.eat(":") {
                parser.skip_type_until(&["="])?;
            }
            parser.expect("=")?;
            let value = parser.expression()?;
            parser.expect(";")?;
            module.constants.push((name, value));
        } else if parser.eat_identifier("function") {
            let name = parser.identifier()?;
            let parameters = parser.parameters()?;
            if parser.eat(":") {
                parser.return_annotation()?;
            }
            let body = parser.block()?;
            module.functions.push(Function {
                name,
                parameters,
                body,
            });
        } else {
            return Err(parser.unexpected("an import, a constant or a function"));
        }
    }
    Ok(module)
}

/// One expression filling all of `tokens`.
pub fn parse_expression(tokens: Vec<Token>) -> Result<Expression> {
    let mut parser = Parser::new(tokens);
    let expression = parser.expression()?;
    if !parser.is_at_end() {
        return Err(parser.unexpected("the end of the expression"));
    }
    Ok(expression)
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            position: 0,
        }
    }

    fn is_at_end(&self) -> bool {
        self.position >= self.tokens.len()
    }

    fn peek_kind(&self, offset: usize) -> Option<&TokenKind> {
        self.tokens
            .get(self.position + offset)
            .map(|token| &token.kind)
    }

    fn is_punctuation(&self, punctuation: &str) -> bool {
        matches!(self.peek_kind(0), Some(TokenKind::Punctuation(found)) if *found == punctuation)
    }

    fn is_identifier(&self, name: &str) -> bool {
        matches!(self.peek_kind(0), Some(TokenKind::Identifier(found)) if found == name)
    }

    fn line(&self) -> usize {
        self.tokens
            .get(self.position)
            .or_else(|| self.tokens.last())
            .map_or(0, |token| token.line)
    }

    fn unexpected(&self, expected: &str) -> anyhow::Error {
        match self.peek_kind(0) {
            Some(found) => anyhow!(
                "line {}: expected {expected}, found {}",
                self.line(),
                describe(found)
            ),
            None => anyhow!("line {}: expected {expected}, found the end", self.line()),
        }
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        self.position += 1;
        token
    }

    fn eat(&mut self, punctuation: &str) -> bool {
        let is_there = self.is_punctuation(punctuation);
        if is_there {
            self.position += 1;
        }
        is_there
    }

    fn eat_identifier(&mut self, name: &str) -> bool {
        let is_there = self.is_identifier(name);
        if is_there {
            self.position += 1;
        }
        is_there
    }

    fn expect(&mut self, punctuation: &str) -> Result<()> {
        if self.eat(punctuation) {
            Ok(())
        } else {
            Err(self.unexpected(&format!("`{punctuation}`")))
        }
    }

    fn expect_identifier(&mut self, name: &str) -> Result<()> {
        if self.eat_identifier(name) {
            Ok(())
        } else {
            Err(self.unexpected(&format!("`{name}`")))
        }
    }

    fn identifier(&mut self) -> Result<String> {
        match self.peek_kind(0) {
            Some(TokenKind::Identifier(name)) => {
                let name = name.clone();
                self.position += 1;
                Ok(name)
            }
            _ => Err(self.unexpected("a name")),
        }
    }

    fn string(&mut self) -> Result<String> {
        match self.peek_kind(0) {
            Some(TokenKind::String(text)) => {
                let text = text.clone();
                self.position += 1;
                Ok(text)
            }
            _ => Err(self.unexpected("a string")),
        }
    }

    fn comma_separated_names(&mut self, closing: &str) -> Result<Vec<String>> {
        let mut names = Vec::new();
        while !self.eat(closing) {
            names.push(self.identifier()?);
            if !self.eat(",") {
                self.expect(closing)?;
                break;
            }
        }
        Ok(names)
    }

    /// Skips tokens up to and including `terminator` at bracket depth zero.
    fn skip_past_top_level(&mut self, terminator: &str) -> Result<()> {
        self.skip_type_until(&[terminator])?;
        self.expect(terminator)
    }

    /// Skips a type the catalogue does not need (`satisfies …`, `: Catalogue`) up to one of
    /// `terminators` at bracket depth zero, which is left in place.
    fn skip_type_until(&mut self, terminators: &[&str]) -> Result<()> {
        let mut depth = 0_usize;
        loop {
            match self.peek_kind(0) {
                None => return Err(self.unexpected(&format!("one of {terminators:?}"))),
                Some(TokenKind::Punctuation(punctuation)) => {
                    if depth == 0 && terminators.contains(punctuation) {
                        return Ok(());
                    }
                    match *punctuation {
                        "{" | "(" | "[" | "<" => depth += 1,
                        "}" | ")" | "]" | ">" => {
                            depth = depth.checked_sub(1).ok_or_else(|| {
                                self.unexpected(&format!("one of {terminators:?}"))
                            })?;
                        }
                        _ => {}
                    }
                }
                Some(_) => {}
            }
            self.position += 1;
        }
    }

    fn block(&mut self) -> Result<Vec<Statement>> {
        self.expect("{")?;
        let mut statements = Vec::new();
        while !self.eat("}") {
            statements.push(self.statement()?);
        }
        Ok(statements)
    }

    fn statement(&mut self) -> Result<Statement> {
        if self.eat_identifier("const") {
            let binding = if self.eat("[") {
                Binding::ArrayPattern(self.comma_separated_names("]")?)
            } else {
                Binding::Name(self.identifier()?)
            };
            self.expect("=")?;
            let value = self.expression()?;
            self.expect(";")?;
            return Ok(Statement::Const { binding, value });
        }
        if self.eat_identifier("if") {
            self.expect("(")?;
            let condition = self.expression()?;
            self.expect(")")?;
            let then = self.block()?;
            if self.is_identifier("else") {
                return Err(self.unexpected("no `else` (unsupported)"));
            }
            return Ok(Statement::If { condition, then });
        }
        if self.eat_identifier("return") {
            let value = self.expression()?;
            self.expect(";")?;
            return Ok(Statement::Return(value));
        }
        let expression = self.expression()?;
        self.expect(";")?;
        Ok(Statement::Expression(expression))
    }

    fn parameters(&mut self) -> Result<Vec<Parameter>> {
        self.expect("(")?;
        let mut parameters = Vec::new();
        while !self.eat(")") {
            let name = self.identifier()?;
            let annotation = if self.eat(":") {
                Some(self.type_annotation()?)
            } else {
                None
            };
            parameters.push(Parameter { name, annotation });
            if !self.eat(",") {
                self.expect(")")?;
                break;
            }
        }
        Ok(parameters)
    }

    fn return_annotation(&mut self) -> Result<()> {
        if self.eat_identifier("string") {
            Ok(())
        } else {
            Err(self.unexpected("`string` as the return type"))
        }
    }

    fn type_annotation(&mut self) -> Result<TypeAnnotation> {
        if self.eat_identifier("readonly") {
            self.expect_identifier("string")?;
            self.expect("[")?;
            self.expect("]")?;
            return Ok(TypeAnnotation::StringList);
        }
        if matches!(self.peek_kind(0), Some(TokenKind::String(_))) {
            let mut literals = vec![self.string()?];
            while self.eat("|") {
                literals.push(self.string()?);
            }
            return Ok(TypeAnnotation::StringUnion(literals));
        }
        let base = self.identifier()?;
        let is_nullable = if self.eat("|") {
            self.expect_identifier("null")?;
            true
        } else {
            false
        };
        match (base.as_str(), is_nullable) {
            ("string", false) => Ok(TypeAnnotation::String),
            ("string", true) => Ok(TypeAnnotation::NullableString),
            ("number", false) => Ok(TypeAnnotation::Number),
            ("number", true) => Ok(TypeAnnotation::NullableNumber),
            ("boolean", false) => Ok(TypeAnnotation::Boolean),
            _ => bail!("line {}: unsupported parameter type `{base}`", self.line()),
        }
    }

    fn expression(&mut self) -> Result<Expression> {
        let condition = self.binary(0)?;
        if !self.eat("?") {
            return Ok(condition);
        }
        let when_true = self.expression()?;
        self.expect(":")?;
        let when_false = self.expression()?;
        Ok(Expression::Conditional {
            condition: Box::new(condition),
            when_true: Box::new(when_true),
            when_false: Box::new(when_false),
        })
    }

    /// Binary operators by precedence climbing, loosest level first.
    fn binary(&mut self, level: usize) -> Result<Expression> {
        const LEVELS: [&[(&str, BinaryOperator)]; 6] = [
            &[
                ("||", BinaryOperator::Or),
                ("??", BinaryOperator::NullishCoalescing),
            ],
            &[("&&", BinaryOperator::And)],
            &[
                ("===", BinaryOperator::StrictEqual),
                ("!==", BinaryOperator::StrictNotEqual),
            ],
            &[
                ("<=", BinaryOperator::LessOrEqual),
                (">=", BinaryOperator::GreaterOrEqual),
                ("<", BinaryOperator::Less),
                (">", BinaryOperator::Greater),
            ],
            &[("+", BinaryOperator::Add), ("-", BinaryOperator::Subtract)],
            &[
                ("*", BinaryOperator::Multiply),
                ("/", BinaryOperator::Divide),
                ("%", BinaryOperator::Remainder),
            ],
        ];
        let Some(operators) = LEVELS.get(level) else {
            return self.unary();
        };
        let mut left = self.binary(level + 1)?;
        'operators: loop {
            for (punctuation, operator) in *operators {
                if self.eat(punctuation) {
                    let right = self.binary(level + 1)?;
                    left = Expression::Binary {
                        operator: *operator,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                    continue 'operators;
                }
            }
            if self.is_punctuation("==") || self.is_punctuation("!=") || self.is_punctuation("?.") {
                return Err(self.unexpected("`===`, `!==` or no optional chaining (unsupported)"));
            }
            return Ok(left);
        }
    }

    fn unary(&mut self) -> Result<Expression> {
        let operator = if self.eat("!") {
            UnaryOperator::Not
        } else if self.eat("-") {
            UnaryOperator::Negate
        } else {
            return self.postfix();
        };
        Ok(Expression::Unary {
            operator,
            operand: Box::new(self.unary()?),
        })
    }

    fn postfix(&mut self) -> Result<Expression> {
        let mut expression = self.primary()?;
        loop {
            if self.eat(".") {
                let property = self.identifier()?;
                expression = Expression::Member {
                    object: Box::new(expression),
                    property,
                };
            } else if self.eat("(") {
                let mut arguments = Vec::new();
                while !self.eat(")") {
                    arguments.push(self.expression()?);
                    if !self.eat(",") {
                        self.expect(")")?;
                        break;
                    }
                }
                expression = Expression::Call {
                    callee: Box::new(expression),
                    arguments,
                };
            } else if self.eat("[") {
                let index = self.expression()?;
                self.expect("]")?;
                expression = Expression::Index {
                    object: Box::new(expression),
                    index: Box::new(index),
                };
            } else if self.eat_identifier("satisfies") {
                // A type check only: the value is unchanged.
                self.skip_type_until(&[",", "}", ";", ")"])?;
            } else if self.eat_identifier("as") {
                if self.eat_identifier("Record") {
                    self.expect("<")?;
                    self.expect_identifier("string")?;
                    self.expect(",")?;
                    self.expect_identifier("string")?;
                    self.expect(">")?;
                    expression = Expression::TextRecord(Box::new(expression));
                } else {
                    self.expect_identifier("const")?;
                }
            } else {
                return Ok(expression);
            }
        }
    }

    fn primary(&mut self) -> Result<Expression> {
        if self.is_punctuation("(") {
            if let Some(arrow) = self.try_arrow()? {
                return Ok(arrow);
            }
            self.position += 1;
            let expression = self.expression()?;
            self.expect(")")?;
            return Ok(expression);
        }
        if self.is_punctuation("{") {
            return self.object();
        }
        if self.eat("[") {
            return self.array();
        }
        let line = self.line();
        let Some(token) = self.next() else {
            return Err(self.unexpected("an expression"));
        };
        match token.kind {
            TokenKind::String(text) => Ok(Expression::String(text)),
            TokenKind::Number(number) => Ok(Expression::Number(number)),
            TokenKind::Template(Template {
                quasis,
                expressions,
            }) => Ok(Expression::Template {
                quasis,
                expressions: expressions
                    .into_iter()
                    .map(parse_expression)
                    .collect::<Result<_>>()?,
            }),
            TokenKind::Identifier(name) => match name.as_str() {
                "null" => Ok(Expression::Null),
                "undefined" => Ok(Expression::Undefined),
                "true" | "false" | "function" | "new" | "this" | "typeof" => {
                    bail!("line {line}: `{name}` is not supported in a catalogue value")
                }
                _ if self.is_punctuation("=>") => {
                    self.position += 1;
                    let body = self.arrow_body()?;
                    Ok(Expression::Arrow(Box::new(Arrow {
                        parameters: vec![Parameter {
                            name,
                            annotation: None,
                        }],
                        body,
                    })))
                }
                _ => Ok(Expression::Identifier(name)),
            },
            TokenKind::Punctuation(punctuation) => {
                self.position -= 1;
                Err(self.unexpected(&format!("an expression, not `{punctuation}`")))
            }
        }
    }

    /// An arrow function starting at `(`, or `None` (and nothing consumed) for a parenthesised
    /// expression.
    fn try_arrow(&mut self) -> Result<Option<Expression>> {
        let start = self.position;
        let Ok(parameters) = self.parameters() else {
            self.position = start;
            return Ok(None);
        };
        if self.eat(":") {
            self.return_annotation()?;
        }
        if !self.eat("=>") {
            self.position = start;
            return Ok(None);
        }
        let body = self.arrow_body()?;
        Ok(Some(Expression::Arrow(Box::new(Arrow {
            parameters,
            body,
        }))))
    }

    fn arrow_body(&mut self) -> Result<FunctionBody> {
        if self.is_punctuation("{") {
            Ok(FunctionBody::Block(self.block()?))
        } else {
            Ok(FunctionBody::Expression(Box::new(self.expression()?)))
        }
    }

    fn object(&mut self) -> Result<Expression> {
        self.expect("{")?;
        let mut properties = Vec::new();
        while !self.eat("}") {
            let line = self.line();
            let Some(TokenKind::Identifier(key) | TokenKind::String(key)) =
                self.peek_kind(0).cloned()
            else {
                return Err(self.unexpected("a property name"));
            };
            self.position += 1;
            self.expect(":")?;
            let value = self.expression()?;
            properties.push(Property { key, value, line });
            if !self.eat(",") {
                self.expect("}")?;
                break;
            }
        }
        Ok(Expression::Object(properties))
    }

    fn array(&mut self) -> Result<Expression> {
        let mut elements = Vec::new();
        while !self.eat("]") {
            if self.eat("...") {
                elements.push(ArrayElement::Spread(self.expression()?));
            } else {
                elements.push(ArrayElement::Item(self.expression()?));
            }
            if !self.eat(",") {
                self.expect("]")?;
                break;
            }
        }
        Ok(Expression::Array(elements))
    }
}

fn describe(kind: &TokenKind) -> String {
    match kind {
        TokenKind::Identifier(name) => format!("`{name}`"),
        TokenKind::String(_) => "a string".to_owned(),
        TokenKind::Number(number) => format!("`{number}`"),
        TokenKind::Template(_) => "a template string".to_owned(),
        TokenKind::Punctuation(punctuation) => format!("`{punctuation}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::lexer::tokenize;

    fn expression(source: &str) -> Expression {
        parse_expression(tokenize(source).unwrap()).unwrap()
    }

    #[test]
    fn binds_operators_by_their_precedence() {
        let Expression::Binary {
            operator: BinaryOperator::Or,
            right,
            ..
        } = expression("a > 0 || b !== null && c")
        else {
            panic!("`||` is not the root");
        };
        assert!(matches!(
            *right,
            Expression::Binary {
                operator: BinaryOperator::And,
                ..
            }
        ));
    }

    #[test]
    fn reads_arrows_with_and_without_annotations() {
        let Expression::Arrow(arrow) = expression("(a: string | null, b: \"x\" | \"y\") => a")
        else {
            panic!("not an arrow");
        };
        assert_eq!(
            arrow.parameters[1].annotation,
            Some(TypeAnnotation::StringUnion(vec![
                "x".to_owned(),
                "y".to_owned()
            ]))
        );
        assert!(matches!(expression("(a, b) => a"), Expression::Arrow(_)));
        assert!(matches!(expression("(a)"), Expression::Identifier(_)));
    }

    #[test]
    fn reads_a_module_and_skips_type_only_parts() {
        let module = parse_module(
            tokenize(
                "import type { A } from \"./a\";\nimport { B } from \"./b\";\n\
                 const c = 1;\nfunction f(x: number): string { return `${x}`; }\n\
                 export const en = { k: \"v\" satisfies string, o: { as: 1 } as const };\n\
                 export type Catalogue = typeof en;",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(module.imports.len(), 1);
        assert_eq!(module.constants.len(), 2);
        assert_eq!(module.functions[0].name, "f");
    }

    #[test]
    fn rejects_unsupported_constructs() {
        for source in ["a == b", "a?.b", "true", "new Date()"] {
            assert!(
                parse_expression(tokenize(source).unwrap()).is_err(),
                "{source} was accepted"
            );
        }
        assert!(parse_module(tokenize("let a = 1;").unwrap()).is_err());
    }
}
