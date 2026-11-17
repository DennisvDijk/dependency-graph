//! Terraform files read with a real HCL parser.
//!
//! The collectors ask three things of a file: which blocks it declares, what
//! their attributes say, and what their nested blocks hold. Answering them
//! from the syntax tree instead of from lines of text means braces in strings,
//! heredocs, multi-line lists and comments cannot throw the reading off.

use std::collections::BTreeMap;

use hcl_edit::Span;
use hcl_edit::expr::{Expression, ObjectKey};
use hcl_edit::structure::{BlockLabel, Body as Edit};

/// One attribute value: its source text and, when it is a plain string, the
/// string itself.
#[derive(Debug, Clone, Default)]
struct Value {
    text: String,
    literal: Option<String>,
    /// For an object (`tags = { Name = "x" }`), its keys with plain-string values.
    pairs: BTreeMap<String, String>,
}

/// A block body: attributes, nested blocks and the source text of the block.
#[derive(Debug, Clone, Default)]
pub struct Body {
    /// The source text of the block, header included, for scans by name.
    pub raw: String,
    attrs: BTreeMap<String, Value>,
    blocks: Vec<Block>,
}

/// A block such as `resource "aws_sqs_queue" "jobs" { ... }`.
#[derive(Debug, Clone)]
pub struct Block {
    /// `resource`, `data`, `module`, `locals`, a nested block's name, ...
    pub kind: String,
    /// The quoted labels, in order.
    pub labels: Vec<String>,
    /// The attributes and nested blocks.
    pub body: Body,
}

/// Parse Terraform text. `None` when it is not valid HCL.
#[must_use]
pub fn parse(text: &str) -> Option<Vec<Block>> {
    let body = hcl_edit::parser::parse_body(text).ok()?;
    Some(blocks_of(&body, text))
}

/// The attributes of the file itself, which `parse` leaves out.
#[must_use]
pub fn is_valid(text: &str) -> bool {
    hcl_edit::parser::parse_body(text).is_ok()
}

fn blocks_of(body: &Edit, text: &str) -> Vec<Block> {
    body.blocks()
        .map(|block| Block {
            kind: block.ident.as_str().to_owned(),
            labels: block
                .labels
                .iter()
                .map(|label| match label {
                    BlockLabel::Ident(ident) => ident.as_str().to_owned(),
                    BlockLabel::String(text) => text.as_str().to_owned(),
                })
                .collect(),
            body: body_of(
                &block.body,
                block.span().and_then(|span| text.get(span)),
                text,
            ),
        })
        .collect()
}

fn body_of(body: &Edit, raw: Option<&str>, text: &str) -> Body {
    Body {
        raw: raw.unwrap_or_default().to_owned(),
        attrs: body
            .attributes()
            .map(|attr| (attr.key.as_str().to_owned(), value_of(&attr.value)))
            .collect(),
        blocks: blocks_of(body, text),
    }
}

fn value_of(expression: &Expression) -> Value {
    let text = expression.to_string().trim().to_owned();
    let literal = match expression {
        Expression::String(value) => Some(value.as_str().to_owned()),
        Expression::StringTemplate(_) => text
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .map(str::to_owned),
        _ => None,
    };
    let mut pairs = BTreeMap::new();
    if let Expression::Object(object) = expression {
        for (key, value) in object.iter() {
            let key = match key {
                ObjectKey::Ident(ident) => ident.as_str().to_owned(),
                ObjectKey::Expression(Expression::String(key)) => key.as_str().to_owned(),
                ObjectKey::Expression(_) => continue,
            };
            if let Expression::String(value) = value.expr() {
                pairs.insert(key, value.as_str().to_owned());
            }
        }
    }
    Value {
        text,
        literal,
        pairs,
    }
}

impl Body {
    /// The source text of the attribute's value, or an empty string.
    #[must_use]
    pub fn value(&self, key: &str) -> String {
        self.attrs
            .get(key)
            .map(|value| value.text.clone())
            .unwrap_or_default()
    }

    /// The attribute's value when it is a non-empty string.
    #[must_use]
    pub fn string(&self, key: &str) -> Option<String> {
        self.attrs
            .get(key)?
            .literal
            .clone()
            .filter(|text| !text.is_empty())
    }

    /// The nested blocks called `name`.
    #[must_use]
    pub fn nested(&self, name: &str) -> Vec<&Body> {
        self.blocks
            .iter()
            .filter(|block| block.kind == name)
            .map(|block| &block.body)
            .collect()
    }

    /// The `tags` object, with the keys whose values are plain strings.
    #[must_use]
    pub fn tags(&self) -> BTreeMap<String, String> {
        self.attrs
            .get("tags")
            .map(|value| value.pairs.clone())
            .unwrap_or_default()
    }

    /// Every plain-string attribute, by name.
    #[must_use]
    pub fn strings(&self) -> BTreeMap<String, String> {
        self.attrs
            .iter()
            .filter_map(|(key, value)| Some((key.clone(), value.literal.clone()?)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"
# a comment with a brace {
resource "aws_sqs_queue" "jobs" {
  name = "billing-${var.env}.fifo"
  tags = { Name = "jobs", "team" = "pay" }
  redrive = <<EOT
{ "not": "a block" }
EOT
  ingress {
    security_groups = [
      aws_security_group.a.id,
      aws_security_group.b.id,
    ]
  }
}
variable "env" {
  default = "prod"
}
"#;

    #[test]
    fn blocks_attributes_and_nesting_are_read_from_the_syntax() {
        let blocks = parse(FILE).unwrap();
        assert_eq!(blocks.len(), 2);
        let queue = &blocks[0];
        assert_eq!(
            (queue.kind.as_str(), queue.labels.as_slice()),
            (
                "resource",
                &["aws_sqs_queue".to_owned(), "jobs".to_owned()][..]
            )
        );
        assert_eq!(
            queue.body.string("name").as_deref(),
            Some("billing-${var.env}.fifo")
        );
        assert_eq!(queue.body.tags()["team"], "pay");
        let ingress = queue.body.nested("ingress");
        assert_eq!(ingress.len(), 1);
        assert!(
            ingress[0]
                .value("security_groups")
                .contains("aws_security_group.b.id")
        );
        assert_eq!(blocks[1].body.string("default").as_deref(), Some("prod"));
    }

    #[test]
    fn invalid_hcl_is_reported_and_not_guessed() {
        assert!(parse("resource \"a\" \"b\" {").is_none());
        assert!(!is_valid("resource \"a\" \"b\" {"));
        assert!(is_valid(FILE));
    }
}
