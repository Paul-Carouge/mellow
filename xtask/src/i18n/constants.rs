//! Values known when the catalogue is generated: imported limits, the catalogues' own constants
//! and every entry without parameters, folded into plain text as JavaScript would compute it.

use std::collections::BTreeMap;

use super::parser::{BinaryOperator, Expression, UnaryOperator};

#[derive(Debug, Clone, PartialEq)]
pub enum ConstantValue {
    Number(f64),
    Text(String),
    /// Keys in source order.
    Object(Vec<(String, ConstantValue)>),
}

pub type Constants = BTreeMap<String, ConstantValue>;

/// The value of `expression` if it depends on constants only, `None` otherwise.
pub fn fold(expression: &Expression, constants: &Constants) -> Option<ConstantValue> {
    match expression {
        Expression::String(text) => Some(ConstantValue::Text(text.clone())),
        Expression::Number(number) => Some(ConstantValue::Number(*number)),
        Expression::Template {
            quasis,
            expressions,
        } => {
            let mut text = String::new();
            for (index, quasi) in quasis.iter().enumerate() {
                text.push_str(quasi);
                if let Some(expression) = expressions.get(index) {
                    text.push_str(&fold_to_text(expression, constants)?);
                }
            }
            Some(ConstantValue::Text(text))
        }
        Expression::Identifier(name) => constants.get(name).cloned(),
        Expression::Object(properties) => properties
            .iter()
            .map(|property| Some((property.key.clone(), fold(&property.value, constants)?)))
            .collect::<Option<_>>()
            .map(ConstantValue::Object),
        Expression::Member { object, property } => match fold(object, constants)? {
            ConstantValue::Object(fields) => fields
                .into_iter()
                .find_map(|(key, value)| (key == *property).then_some(value)),
            _ => None,
        },
        Expression::Unary {
            operator: UnaryOperator::Negate,
            operand,
        } => match fold(operand, constants)? {
            ConstantValue::Number(number) => Some(ConstantValue::Number(-number)),
            _ => None,
        },
        Expression::Binary {
            operator,
            left,
            right,
        } => fold_binary(*operator, fold(left, constants)?, fold(right, constants)?),
        _ => None,
    }
}

/// What `${expression}` writes in a template, if it is constant.
pub fn fold_to_text(expression: &Expression, constants: &Constants) -> Option<String> {
    match fold(expression, constants)? {
        ConstantValue::Text(text) => Some(text),
        ConstantValue::Number(number) => Some(javascript_number_to_string(number)),
        ConstantValue::Object(_) => None,
    }
}

fn fold_binary(
    operator: BinaryOperator,
    left: ConstantValue,
    right: ConstantValue,
) -> Option<ConstantValue> {
    match (left, right) {
        (ConstantValue::Number(left), ConstantValue::Number(right)) => {
            let number = match operator {
                BinaryOperator::Add => left + right,
                BinaryOperator::Subtract => left - right,
                BinaryOperator::Multiply => left * right,
                BinaryOperator::Divide => left / right,
                BinaryOperator::Remainder => left % right,
                _ => return None,
            };
            Some(ConstantValue::Number(number))
        }
        (ConstantValue::Text(left), ConstantValue::Text(right))
            if operator == BinaryOperator::Add =>
        {
            Some(ConstantValue::Text(left + &right))
        }
        _ => None,
    }
}

/// JavaScript's `Number.prototype.toString()`: what a number becomes inside a template string.
pub fn javascript_number_to_string(number: f64) -> String {
    /// From this magnitude JavaScript switches to exponent notation.
    const LARGEST_PLAIN_NUMBER: f64 = 1e21;
    const SMALLEST_PLAIN_NUMBER: f64 = 1e-6;
    if number.is_nan() {
        return "NaN".to_owned();
    }
    if number.is_infinite() {
        return if number > 0.0 {
            "Infinity"
        } else {
            "-Infinity"
        }
        .to_owned();
    }
    if number == 0.0 {
        // -0 too.
        return "0".to_owned();
    }
    let magnitude = number.abs();
    if (SMALLEST_PLAIN_NUMBER..LARGEST_PLAIN_NUMBER).contains(&magnitude) {
        // Rust's shortest round-trip digits are JavaScript's in this range.
        return format!("{number}");
    }
    let exponent_form = format!("{number:e}");
    match exponent_form.split_once('e') {
        Some((mantissa, exponent)) if !exponent.starts_with('-') => {
            format!("{mantissa}e+{exponent}")
        }
        _ => exponent_form,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::lexer::tokenize;
    use crate::i18n::parser::parse_expression;

    fn folded(source: &str, constants: &Constants) -> Option<ConstantValue> {
        fold(
            &parse_expression(tokenize(source).unwrap()).unwrap(),
            constants,
        )
    }

    #[test]
    fn folds_arithmetic_and_templates_like_javascript() {
        let constants =
            Constants::from([("LIMIT".to_owned(), ConstantValue::Number(256.0 * 1024.0))]);
        assert_eq!(
            folded("`${LIMIT / 1024} KB, ${-0.5}`", &constants),
            Some(ConstantValue::Text("256 KB, -0.5".to_owned()))
        );
        assert_eq!(
            folded("{ a: { b: \"c\" } }.a.b", &constants),
            Some(ConstantValue::Text("c".to_owned()))
        );
    }

    #[test]
    fn leaves_parameters_unfolded() {
        assert_eq!(folded("`${name}`", &Constants::new()), None);
    }

    #[test]
    fn writes_numbers_as_javascript_does() {
        assert_eq!(javascript_number_to_string(3.0), "3");
        assert_eq!(javascript_number_to_string(-0.0), "0");
        assert_eq!(
            javascript_number_to_string(0.1 + 0.2),
            "0.30000000000000004"
        );
        assert_eq!(javascript_number_to_string(1e21), "1e+21");
        assert_eq!(javascript_number_to_string(1.5e-7), "1.5e-7");
        assert_eq!(javascript_number_to_string(f64::NAN), "NaN");
    }
}
