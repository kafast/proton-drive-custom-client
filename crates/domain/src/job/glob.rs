//! Einfache Dateimuster für Dateinamen: `*`, `?` und Zeichenklassen `[abc]`, `[a-z]`, `[!x]`.
//!
//! Muster gelten nur für einen Namen (ohne `/`), nicht für ganze Pfade.

use crate::DomainError;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Literal(char),
    /// `?`: genau ein beliebiges Zeichen.
    AnyChar,
    /// `*`: beliebig viele Zeichen.
    AnyRun,
    /// `[...]`: ein Zeichen aus (oder bei `negated` nicht aus) den Bereichen.
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

/// Ein geprüftes Dateimuster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobPattern {
    source: String,
    tokens: Vec<Token>,
}

impl GlobPattern {
    /// Liest ein Muster wie `*.tar` oder `backup-202?-*.tar`.
    ///
    /// # Errors
    /// [`DomainError::InvalidGlob`] bei leerem Muster, `/` oder offener Klammer.
    pub fn parse(pattern: &str) -> Result<Self, DomainError> {
        let invalid = |reason| DomainError::InvalidGlob {
            pattern: pattern.to_owned(),
            reason,
        };
        if pattern.is_empty() {
            return Err(invalid("leeres Muster"));
        }
        if pattern.contains('/') {
            return Err(invalid("Muster gelten nur für Dateinamen, ohne /"));
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
                        let start = chars.next().ok_or_else(|| invalid("[ ohne ]"))?;
                        // Ein `]` direkt am Anfang zählt als Zeichen.
                        if start == ']' && !ranges.is_empty() {
                            break;
                        }
                        let end = if chars.next_if_eq(&'-').is_some() {
                            match chars.next() {
                                Some(']') | None => return Err(invalid("Bereich ohne Ende")),
                                Some(end) => end,
                            }
                        } else {
                            start
                        };
                        if start > end {
                            return Err(invalid("Bereich ist rückwärts"));
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

    /// Das Muster, wie es angegeben wurde.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.source
    }

    /// `true`, wenn `name` zum Muster passt.
    #[must_use]
    pub fn matches(&self, name: &str) -> bool {
        let text: Vec<char> = name.chars().collect();
        let (mut t, mut p) = (0, 0);
        // Merkt sich das letzte `*` und wie weit es schon geschluckt hat, um
        // bei einem Fehlschlag ein Zeichen mehr zu schlucken (Backtracking).
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
