//! Version constraint expressions.
//!
//! Constraints combine semver comparators with `&` (and) and `|` (or), where
//! `&` binds tighter than `|`, and `()` groups sub-expressions. Examples:
//!
//! ```text
//! 1.2.3
//! >=1.2.3 & <2.0.0
//! >=1.2.3 & <2.0.0 | >=3.0.0
//! (>=1.2.3 & <2.0.0) | >=3.0.0
//! ```
//!
//! Whitespace is insignificant; two comparators are only combined when an
//! explicit `&`/`|` separates them. A comparator's version must be a full
//! `major.minor.patch` semver.

use anyhow::{Result, anyhow, bail};
use semver::{Version, VersionReq};

#[derive(Debug, Clone)]
pub enum VersionConstraint {
    Comparator(VersionReq),
    All(Vec<VersionConstraint>),
    Any(Vec<VersionConstraint>),
}

impl VersionConstraint {
    pub fn parse(input: &str) -> Result<Self> {
        let tokens = tokenize(input)?;
        if tokens.is_empty() {
            bail!("invalid version constraint: {input:?} is empty");
        }
        let mut parser = Parser { tokens, pos: 0 };
        let expression = parser.parse_or()?;
        if parser.pos != parser.tokens.len() {
            bail!("invalid version constraint {input:?}: expected '&', '|' or end of input");
        }
        Ok(expression)
    }

    pub fn matches(&self, version: &Version) -> bool {
        match self {
            Self::Comparator(req) => req.matches(version),
            Self::All(items) => items.iter().all(|item| item.matches(version)),
            Self::Any(items) => items.iter().any(|item| item.matches(version)),
        }
    }
}

/// Validate a version constraint string, ignoring the parsed result.
pub fn validate_version_constraint(input: &str) -> Result<()> {
    VersionConstraint::parse(input).map(|_| ())
}

#[derive(Debug, Clone)]
enum Token {
    And,
    Or,
    Open,
    Close,
    Comparator(VersionReq),
}

fn tokenize(input: &str) -> Result<Vec<Token>> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < chars.len() {
        let ch = chars[index];
        match ch {
            ch if ch.is_whitespace() => index += 1,
            '&' => {
                tokens.push(Token::And);
                index += 1;
            }
            '|' => {
                tokens.push(Token::Or);
                index += 1;
            }
            '(' => {
                tokens.push(Token::Open);
                index += 1;
            }
            ')' => {
                tokens.push(Token::Close);
                index += 1;
            }
            '<' | '>' | '=' | '^' | '~' | '0'..='9' => {
                let operator_start = index;
                while index < chars.len() && matches!(chars[index], '<' | '>' | '=' | '^' | '~') {
                    index += 1;
                }
                let operator: String = chars[operator_start..index].iter().collect();

                let version_start = index;
                while index < chars.len()
                    && (chars[index].is_ascii_alphanumeric()
                        || matches!(chars[index], '.' | '-' | '+'))
                {
                    index += 1;
                }
                let version: String = chars[version_start..index].iter().collect();
                if version.is_empty() {
                    bail!(
                        "invalid version constraint {input:?}: expected a version after {operator:?}"
                    );
                }
                tokens.push(Token::Comparator(build_comparator(&operator, &version)?));
            }
            other => bail!("invalid version constraint {input:?}: unexpected character {other:?}"),
        }
    }

    Ok(tokens)
}

fn build_comparator(operator: &str, version: &str) -> Result<VersionReq> {
    // Require a full major.minor.patch version; semver's own ranges would
    // otherwise accept partial versions such as `>=1.2`.
    Version::parse(version).map_err(|err| anyhow!("invalid version {version:?}: {err}"))?;

    let normalized = match operator {
        // A bare version (or `==`) means an exact match.
        "" | "==" | "=" => format!("={version}"),
        ">=" | "<=" | ">" | "<" | "^" | "~" => format!("{operator}{version}"),
        other => bail!("invalid version operator {other:?}"),
    };

    VersionReq::parse(&normalized)
        .map_err(|err| anyhow!("invalid version constraint {normalized:?}: {err}"))
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn parse_or(&mut self) -> Result<VersionConstraint> {
        let mut items = vec![self.parse_and()?];
        while matches!(self.peek(), Some(Token::Or)) {
            self.pos += 1;
            items.push(self.parse_and()?);
        }
        Ok(collapse(items, VersionConstraint::Any))
    }

    fn parse_and(&mut self) -> Result<VersionConstraint> {
        let mut items = vec![self.parse_term()?];
        while matches!(self.peek(), Some(Token::And)) {
            self.pos += 1;
            items.push(self.parse_term()?);
        }
        Ok(collapse(items, VersionConstraint::All))
    }

    fn parse_term(&mut self) -> Result<VersionConstraint> {
        match self.peek() {
            Some(Token::Open) => {
                self.pos += 1;
                let inner = self.parse_or()?;
                if !matches!(self.peek(), Some(Token::Close)) {
                    bail!("invalid version constraint: expected ')'");
                }
                self.pos += 1;
                Ok(inner)
            }
            Some(Token::Comparator(req)) => {
                let req = req.clone();
                self.pos += 1;
                Ok(VersionConstraint::Comparator(req))
            }
            _ => bail!("invalid version constraint: expected a version comparator or '('"),
        }
    }
}

fn collapse(
    mut items: Vec<VersionConstraint>,
    wrap: fn(Vec<VersionConstraint>) -> VersionConstraint,
) -> VersionConstraint {
    if items.len() == 1 {
        items.pop().expect("len checked")
    } else {
        wrap(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(constraint: &str, version: &str) -> bool {
        VersionConstraint::parse(constraint)
            .unwrap()
            .matches(&Version::parse(version).unwrap())
    }

    #[test]
    fn accepts_valid_constraints() {
        for constraint in [
            "1.2.3",
            "=1.2.3",
            "==1.2.3",
            ">=1.2.3",
            ">=1.2.3 & <2.0.0",
            "^1.2.3",
            "~1.2.3",
            ">=1.2.3 & <2.0.0 | >=3.0.0",
            "(>=1.2.3 & <2.0.0) | >=3.0.0",
            "  >=1.2.3  ",
            "1.2.3-alpha.1",
        ] {
            validate_version_constraint(constraint)
                .unwrap_or_else(|err| panic!("{constraint:?} should be valid: {err}"));
        }
    }

    #[test]
    fn rejects_invalid_constraints() {
        for constraint in [
            "",
            ">=1.2",             // partial version
            ">=1.2.0 <2.0.0",    // whitespace is no longer an implicit AND
            ">=1.2.3 |",         // trailing operator
            "& 1.2.3",           // leading operator
            "(>=1.2.3 & <2.0.0", // unbalanced parenthesis
            "1.2.3 & ",          // dangling and
            ">>1.2.3",           // invalid operator
        ] {
            assert!(
                validate_version_constraint(constraint).is_err(),
                "{constraint:?} should be rejected"
            );
        }
    }

    #[test]
    fn evaluates_and_or_with_precedence() {
        // `&` binds tighter than `|`: (>=1.2.3 & <2.0.0) OR (>=3.0.0)
        let constraint = ">=1.2.3 & <2.0.0 | >=3.0.0";
        assert!(matches(constraint, "1.5.0"));
        assert!(!matches(constraint, "2.5.0"));
        assert!(matches(constraint, "3.1.0"));

        assert!(matches("1.2.3", "1.2.3"));
        assert!(!matches("1.2.3", "1.2.4"));
        assert!(matches("^1.2.3", "1.9.0"));
        assert!(!matches("^1.2.3", "2.0.0"));
    }
}
