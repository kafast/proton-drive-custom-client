//! Simple file patterns for file names: `*`, `?` and character classes `[abc]`, `[a-z]`, `[!x]`.
//!
//! Patterns apply to a single name only (without `/`), not to whole paths.

use crate::DomainError;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Literal(char),
    /// `?`: exactly one arbitrary character.
    AnyChar,
    /// `*`: any number of characters.
    AnyRun,
    /// `[...]`: one character from (or with `negated` not from) the ranges.
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

/// A validated file pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobPattern {
    source: String,
    tokens: Vec<Token>,
}

impl GlobPattern {
    /// Reads a pattern such as `*.tar` or `backup-202?-*.tar`.
    ///
    /// # Errors
    /// [`DomainError::InvalidGlob`] for an empty pattern, `/` or an unclosed bracket.
    pub fn parse(pattern: &str) -> Result<Self, DomainError> {
        let invalid = |reason| DomainError::InvalidGlob {
            pattern: pattern.to_owned(),
            reason,
        };
        if pattern.is_empty() {
            return Err(invalid("empty pattern"));
        }
        if pattern.contains('/') {
            return Err(invalid("patterns apply to file names only, without /"));
        }
        let mut tokens = Vec::new();
        let mut chars = pattern.chars().peekable();
        while let Some(c) = chars.next() {
            tokens.push(match c {
                '*' => Token::AnyRun,
                '?' => Token::AnyChar,
                '[' => {
                    let negated = chars.next_if(|c| *c == '!' || *c == '^').is_some();
                    let mut ranges = Vec::new();
                    loop {
                        let start = chars.next().ok_or_else(|| invalid("[ without ]"))?;
                        // A `]` right at the start counts as a character.
                        if start == ']' && !ranges.is_empty() {
                            break;
                        }
                        let end = if chars.next_if_eq(&'-').is_some() {
                            match chars.next() {
                                Some(']') | None => return Err(invalid("range without end")),
                                Some(end) => end,
                            }
                        } else {
                            start
                        };
                        if start > end {
                            return Err(invalid("range is backwards"));
                        }
                        ranges.push((start, end));
                    }
                    Token::Class { negated, ranges }
                }
                other => Token::Literal(other),
            });
        }
        Ok(Self {
            source: pattern.to_owned(),
            tokens,
        })
    }

    /// The pattern as it was given.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.source
    }

    /// `true` if `name` matches the pattern.
    #[must_use]
    pub fn matches(&self, name: &str) -> bool {
        let text: Vec<char> = name.chars().collect();
        let (mut t, mut p) = (0, 0);
        // Remembers the last `*` and how far it has already consumed, in order to
        // consume one more character on a mismatch (backtracking).
        let mut star: Option<(usize, usize)> = None;
        while t < text.len() {
            match self.tokens.get(p) {
                Some(Token::AnyRun) => {
                    star = Some((p, t));
                    p += 1;
                }
                Some(token) if single_matches(token, text[t]) => {
                    t += 1;
                    p += 1;
                }
                _ => match star {
                    Some((star_p, star_t)) => {
                        star = Some((star_p, star_t + 1));
                        p = star_p + 1;
                        t = star_t + 1;
                    }
                    None => return false,
                },
            }
        }
        self.tokens[p..].iter().all(|token| *token == Token::AnyRun)
    }
}

fn single_matches(token: &Token, c: char) -> bool {
    match token {
        Token::Literal(expected) => *expected == c,
        Token::AnyChar => true,
        Token::Class { negated, ranges } => {
            ranges
                .iter()
                .any(|(start, end)| (*start..=*end).contains(&c))
                != *negated
        }
        Token::AnyRun => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glob(pattern: &str) -> GlobPattern {
        GlobPattern::parse(pattern).unwrap()
    }

    #[test]
    fn stars_and_question_marks() {
        let tar = glob("*.tar");
        assert!(tar.matches("backup.tar"));
        assert!(tar.matches(".tar"));
        assert!(!tar.matches("backup.tar.gz"));
        assert!(glob("*").matches(""));
        assert!(glob("a*b*c").matches("aXbYbZc"));
        assert!(!glob("a*b*c").matches("aXbY"));
        assert!(glob("backup-202?-*").matches("backup-2026-10.tar"));
        assert!(!glob("?").matches(""));
    }

    #[test]
    fn character_classes() {
        assert!(glob("[abc].txt").matches("b.txt"));
        assert!(!glob("[abc].txt").matches("d.txt"));
        assert!(glob("file[0-9]").matches("file7"));
        assert!(glob("[!0-9]*").matches("x1"));
        assert!(!glob("[!0-9]*").matches("1x"));
        assert!(glob("[]]").matches("]"));
    }

    #[test]
    fn unicode_names() {
        assert!(glob("Übersicht-*.pdf").matches("Übersicht-März.pdf"));
        assert!(glob("?.txt").matches("ä.txt"));
    }

    #[test]
    fn invalid_patterns() {
        for bad in ["", "a/b", "[abc", "[a-]", "[z-a]"] {
            assert!(GlobPattern::parse(bad).is_err(), "{bad:?}");
        }
        assert_eq!(glob("*.tar").as_str(), "*.tar");
    }
}
