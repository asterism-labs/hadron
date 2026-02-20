//! Tokenizer for Kconfig files.
//!
//! Produces a stream of [`Token`]s from Kconfig source text. Handles
//! keywords, quoted strings, numbers (decimal and hex), identifiers,
//! and significant newlines.

use std::path::PathBuf;

/// A token with source location.
#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// Source location for error reporting.
#[derive(Debug, Clone)]
pub struct Span {
    pub file: PathBuf,
    pub line: usize,
    pub col: usize,
}

impl std::fmt::Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}", self.file.display(), self.line, self.col)
    }
}

/// Token variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    // Keywords
    Config,
    Menu,
    EndMenu,
    Source,
    Bool,
    U32,
    U64,
    Str,
    Choice,
    Default,
    DependsOn,
    Select,
    Range,
    Binding,
    Help,
    Prompt,

    // Literals
    /// Quoted string content (quotes stripped).
    String(String),
    /// Integer literal (decimal or hex).
    Integer(u64),
    /// `y` boolean true.
    Yes,
    /// `n` boolean false.
    No,
    /// An identifier (config name, binding type, etc.).
    Ident(String),

    // Operators (for depends expressions)
    /// `&&`
    And,
    /// `||`
    Or,
    /// `!`
    Not,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `[`
    LBracket,
    /// `]`
    RBracket,
    /// `,`
    Comma,

    // Structure
    Newline,
    Eof,
}

/// Tokenize Kconfig source text.
pub fn tokenize(source: &str, file: PathBuf) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = source.char_indices().peekable();
    let mut line = 1usize;
    let mut line_start = 0usize;

    while let Some(&(pos, ch)) = chars.peek() {
        let col = pos - line_start + 1;

        match ch {
            // Comments: skip to end of line
            '#' => {
                while let Some(&(_, c)) = chars.peek() {
                    if c == '\n' {
                        break;
                    }
                    chars.next();
                }
            }

            // Newline
            '\n' => {
                // Collapse consecutive newlines
                if tokens.last().map_or(true, |t: &Token| t.kind != TokenKind::Newline) {
                    tokens.push(Token {
                        kind: TokenKind::Newline,
                        span: Span { file: file.clone(), line, col },
                    });
                }
                chars.next();
                line += 1;
                line_start = pos + 1;
            }

            // Whitespace (non-newline)
            ' ' | '\t' | '\r' => {
                chars.next();
            }

            // Quoted string
            '"' => {
                chars.next(); // consume opening quote
                let mut s = String::new();
                loop {
                    match chars.next() {
                        Some((_, '"')) => break,
                        Some((_, '\\')) => {
                            if let Some((_, escaped)) = chars.next() {
                                match escaped {
                                    'n' => s.push('\n'),
                                    't' => s.push('\t'),
                                    '\\' => s.push('\\'),
                                    '"' => s.push('"'),
                                    _ => {
                                        s.push('\\');
                                        s.push(escaped);
                                    }
                                }
                            }
                        }
                        Some((_, c)) => s.push(c),
                        None => return Err(format!("{}:{}:{}: unterminated string", file.display(), line, col)),
                    }
                }
                tokens.push(Token {
                    kind: TokenKind::String(s),
                    span: Span { file: file.clone(), line, col },
                });
            }

            // Operators and brackets
            '(' => {
                tokens.push(Token { kind: TokenKind::LParen, span: Span { file: file.clone(), line, col } });
                chars.next();
            }
            ')' => {
                tokens.push(Token { kind: TokenKind::RParen, span: Span { file: file.clone(), line, col } });
                chars.next();
            }
            '[' => {
                tokens.push(Token { kind: TokenKind::LBracket, span: Span { file: file.clone(), line, col } });
                chars.next();
            }
            ']' => {
                tokens.push(Token { kind: TokenKind::RBracket, span: Span { file: file.clone(), line, col } });
                chars.next();
            }
            ',' => {
                tokens.push(Token { kind: TokenKind::Comma, span: Span { file: file.clone(), line, col } });
                chars.next();
            }
            '!' => {
                tokens.push(Token { kind: TokenKind::Not, span: Span { file: file.clone(), line, col } });
                chars.next();
            }
            '&' => {
                chars.next();
                if let Some(&(_, '&')) = chars.peek() {
                    chars.next();
                }
                tokens.push(Token { kind: TokenKind::And, span: Span { file: file.clone(), line, col } });
            }
            '|' => {
                chars.next();
                if let Some(&(_, '|')) = chars.peek() {
                    chars.next();
                }
                tokens.push(Token { kind: TokenKind::Or, span: Span { file: file.clone(), line, col } });
            }

            // Numbers (decimal or hex)
            '0'..='9' => {
                let start = pos;
                let mut is_hex = false;

                chars.next();
                if ch == '0' {
                    if let Some(&(_, 'x' | 'X')) = chars.peek() {
                        chars.next();
                        is_hex = true;
                    }
                }

                while let Some(&(_, c)) = chars.peek() {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        chars.next();
                    } else {
                        break;
                    }
                }

                let end = chars.peek().map_or(source.len(), |&(p, _)| p);
                let raw = &source[start..end];
                let clean = raw.replace('_', "");

                let value = if is_hex {
                    let hex_part = clean.strip_prefix("0x").or_else(|| clean.strip_prefix("0X")).unwrap_or(&clean);
                    u64::from_str_radix(hex_part, 16)
                        .map_err(|_| format!("{}:{}:{}: invalid hex literal '{raw}'", file.display(), line, col))?
                } else {
                    clean.parse::<u64>()
                        .map_err(|_| format!("{}:{}:{}: invalid integer literal '{raw}'", file.display(), line, col))?
                };

                tokens.push(Token {
                    kind: TokenKind::Integer(value),
                    span: Span { file: file.clone(), line, col },
                });
            }

            // Identifiers and keywords
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = pos;
                chars.next();
                while let Some(&(_, c)) = chars.peek() {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        chars.next();
                    } else {
                        break;
                    }
                }
                let end = chars.peek().map_or(source.len(), |&(p, _)| p);
                let word = &source[start..end];

                let kind = match word {
                    "config" => TokenKind::Config,
                    "menu" => TokenKind::Menu,
                    "endmenu" => TokenKind::EndMenu,
                    "source" => TokenKind::Source,
                    "bool" => TokenKind::Bool,
                    "u32" => TokenKind::U32,
                    "u64" => TokenKind::U64,
                    "str" => TokenKind::Str,
                    "choice" => TokenKind::Choice,
                    "default" => TokenKind::Default,
                    "depends" => {
                        // Peek for "on" keyword
                        // Save position and check
                        let saved: Vec<_> = chars.clone().take(10).collect();
                        let remaining: String = saved.iter().map(|(_, c)| c).collect();
                        let trimmed = remaining.trim_start();
                        if trimmed.starts_with("on") && trimmed[2..].starts_with(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
                            // Consume whitespace + "on"
                            while let Some(&(_, c)) = chars.peek() {
                                if c == ' ' || c == '\t' {
                                    chars.next();
                                } else {
                                    break;
                                }
                            }
                            // Consume "on"
                            if let Some(&(_, 'o')) = chars.peek() {
                                chars.next();
                                if let Some(&(_, 'n')) = chars.peek() {
                                    chars.next();
                                }
                            }
                            TokenKind::DependsOn
                        } else {
                            TokenKind::Ident(word.to_string())
                        }
                    }
                    "select" => TokenKind::Select,
                    "range" => TokenKind::Range,
                    "binding" => TokenKind::Binding,
                    "help" => TokenKind::Help,
                    "prompt" => TokenKind::Prompt,
                    "y" => TokenKind::Yes,
                    "n" => TokenKind::No,
                    "on" => TokenKind::Ident(word.to_string()),
                    "cumulative" | "cfg" | "const" | "build" => TokenKind::Ident(word.to_string()),
                    _ => TokenKind::Ident(word.to_string()),
                };

                tokens.push(Token {
                    kind,
                    span: Span { file: file.clone(), line, col },
                });
            }

            _ => {
                return Err(format!("{}:{}:{}: unexpected character '{ch}'", file.display(), line, col));
            }
        }
    }

    // Ensure we end with a newline before EOF for uniform parsing
    if tokens.last().map_or(true, |t| t.kind != TokenKind::Newline) {
        tokens.push(Token {
            kind: TokenKind::Newline,
            span: Span { file: file.clone(), line, col: 1 },
        });
    }

    tokens.push(Token {
        kind: TokenKind::Eof,
        span: Span { file, line, col: 1 },
    });

    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_config_block() {
        let src = r#"
config SMP
    bool "Enable SMP"
    default y
    depends on ACPI
    select APIC
    binding cfg
"#;
        let tokens = tokenize(src, PathBuf::from("test")).unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();

        assert!(kinds.contains(&&TokenKind::Config));
        assert!(kinds.contains(&&TokenKind::Ident("SMP".to_string())));
        assert!(kinds.contains(&&TokenKind::Bool));
        assert!(kinds.contains(&&TokenKind::String("Enable SMP".to_string())));
        assert!(kinds.contains(&&TokenKind::Default));
        assert!(kinds.contains(&&TokenKind::Yes));
        assert!(kinds.contains(&&TokenKind::DependsOn));
        assert!(kinds.contains(&&TokenKind::Ident("ACPI".to_string())));
        assert!(kinds.contains(&&TokenKind::Select));
        assert!(kinds.contains(&&TokenKind::Ident("APIC".to_string())));
        assert!(kinds.contains(&&TokenKind::Binding));
        assert!(kinds.contains(&&TokenKind::Ident("cfg".to_string())));
    }

    #[test]
    fn tokenize_hex_number() {
        let tokens = tokenize("default 0x10_0000", PathBuf::from("test")).unwrap();
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Integer(0x10_0000)));
    }

    #[test]
    fn tokenize_menu() {
        let src = "menu \"SMP\"\nendmenu";
        let tokens = tokenize(src, PathBuf::from("test")).unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert!(kinds.contains(&&TokenKind::Menu));
        assert!(kinds.contains(&&TokenKind::String("SMP".to_string())));
        assert!(kinds.contains(&&TokenKind::EndMenu));
    }
}
