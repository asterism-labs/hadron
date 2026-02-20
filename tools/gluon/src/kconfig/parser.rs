//! Recursive descent parser for Kconfig files.
//!
//! Produces an AST ([`KconfigFile`]) from a token stream. The parser
//! follows `source` directives to load sub-files, but does not resolve
//! them — that is handled by [`super::load_kconfig`].

use crate::model::Binding;

use super::ast::*;
use super::lexer::{Token, TokenKind};

/// Parser state: a cursor over the token stream.
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    /// Create a new parser from a token stream.
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    /// Parse the entire token stream into a [`KconfigFile`].
    pub fn parse(&mut self) -> Result<KconfigFile, String> {
        self.skip_newlines();
        let mut items = Vec::new();
        while !self.at_eof() {
            let item = self.parse_item(None)?;
            if let Some(it) = item {
                items.push(it);
            }
            self.skip_newlines();
        }
        Ok(KconfigFile { items })
    }

    /// Parse a single top-level item.
    fn parse_item(&mut self, menu_title: Option<&str>) -> Result<Option<KconfigItem>, String> {
        self.skip_newlines();
        if self.at_eof() {
            return Ok(None);
        }

        match self.peek_kind() {
            TokenKind::Config => Ok(Some(self.parse_config(menu_title)?)),
            TokenKind::Menu => Ok(Some(self.parse_menu()?)),
            TokenKind::Source => Ok(Some(self.parse_source()?)),
            TokenKind::EndMenu => Ok(None), // handled by parse_menu
            _ => {
                let tok = self.peek();
                Err(format!("{}: unexpected token {:?}", tok.span, tok.kind))
            }
        }
    }

    /// Parse a `config NAME` block.
    fn parse_config(&mut self, menu_title: Option<&str>) -> Result<KconfigItem, String> {
        self.expect(TokenKind::Config)?;
        let name = self.expect_ident()?;
        self.expect_newline()?;

        let mut block = ConfigBlock {
            name,
            ty: None,
            prompt: None,
            default: None,
            depends_on: None,
            selects: Vec::new(),
            range: None,
            bindings: Vec::new(),
            help: menu_title.map(|_| String::new()), // placeholder
        };

        // Parse indented properties until next config/menu/source/endmenu/eof
        loop {
            self.skip_newlines();
            if self.at_eof() {
                break;
            }
            match self.peek_kind() {
                TokenKind::Config
                | TokenKind::Menu
                | TokenKind::EndMenu
                | TokenKind::Source => break,

                TokenKind::Bool => {
                    self.advance();
                    let prompt = self.try_string();
                    block.ty = Some(TypeDecl {
                        kind: TypeKind::Bool,
                        variants: Vec::new(),
                        prompt: prompt.clone(),
                    });
                    if prompt.is_some() {
                        block.prompt = prompt;
                    }
                    self.expect_newline()?;
                }
                TokenKind::U32 => {
                    self.advance();
                    let prompt = self.try_string();
                    block.ty = Some(TypeDecl {
                        kind: TypeKind::U32,
                        variants: Vec::new(),
                        prompt: prompt.clone(),
                    });
                    if prompt.is_some() {
                        block.prompt = prompt;
                    }
                    self.expect_newline()?;
                }
                TokenKind::U64 => {
                    self.advance();
                    let prompt = self.try_string();
                    block.ty = Some(TypeDecl {
                        kind: TypeKind::U64,
                        variants: Vec::new(),
                        prompt: prompt.clone(),
                    });
                    if prompt.is_some() {
                        block.prompt = prompt;
                    }
                    self.expect_newline()?;
                }
                TokenKind::Str => {
                    self.advance();
                    let prompt = self.try_string();
                    block.ty = Some(TypeDecl {
                        kind: TypeKind::Str,
                        variants: Vec::new(),
                        prompt: prompt.clone(),
                    });
                    if prompt.is_some() {
                        block.prompt = prompt;
                    }
                    self.expect_newline()?;
                }
                TokenKind::Choice => {
                    self.advance();
                    let prompt = self.try_string();
                    let variants = self.parse_bracket_list()?;
                    block.ty = Some(TypeDecl {
                        kind: TypeKind::Choice,
                        variants,
                        prompt: prompt.clone(),
                    });
                    if prompt.is_some() {
                        block.prompt = prompt;
                    }
                    self.expect_newline()?;
                }
                TokenKind::Default => {
                    self.advance();
                    block.default = Some(self.parse_default_value()?);
                    self.expect_newline()?;
                }
                TokenKind::DependsOn => {
                    self.advance();
                    block.depends_on = Some(self.parse_depends_expr()?);
                    self.expect_newline()?;
                }
                TokenKind::Select => {
                    self.advance();
                    block.selects.push(self.expect_ident()?);
                    self.expect_newline()?;
                }
                TokenKind::Range => {
                    self.advance();
                    let min = self.expect_integer()?;
                    let max = self.expect_integer()?;
                    block.range = Some((min, max));
                    self.expect_newline()?;
                }
                TokenKind::Binding => {
                    self.advance();
                    let binding = self.parse_binding()?;
                    block.bindings.push(binding);
                    self.expect_newline()?;
                }
                TokenKind::Help => {
                    self.advance();
                    let help_text = self.try_string().unwrap_or_default();
                    block.help = Some(help_text);
                    self.expect_newline()?;
                }
                TokenKind::Prompt => {
                    self.advance();
                    block.prompt = self.try_string();
                    self.expect_newline()?;
                }
                _ => break,
            }
        }

        // If no explicit help but inside a menu, use menu title for grouping
        // (help field is not needed for menu grouping — that's done via the
        // menu_title parameter passed to the converter).
        let _ = menu_title;

        Ok(KconfigItem::Config(block))
    }

    /// Parse a `menu "title"` ... `endmenu` block.
    fn parse_menu(&mut self) -> Result<KconfigItem, String> {
        self.expect(TokenKind::Menu)?;
        let title = self.expect_string()?;
        self.expect_newline()?;

        let mut items = Vec::new();
        loop {
            self.skip_newlines();
            if self.at_eof() {
                break;
            }
            if self.peek_kind() == TokenKind::EndMenu {
                self.advance();
                // Consume optional newline after endmenu
                if self.peek_kind() == TokenKind::Newline {
                    self.advance();
                }
                break;
            }
            let item = self.parse_item(Some(&title))?;
            if let Some(it) = item {
                items.push(it);
            }
        }

        Ok(KconfigItem::Menu(MenuBlock { title, items }))
    }

    /// Parse a `source "path"` directive.
    fn parse_source(&mut self) -> Result<KconfigItem, String> {
        self.expect(TokenKind::Source)?;
        let path = self.expect_string()?;
        self.expect_newline()?;
        Ok(KconfigItem::Source(path))
    }

    /// Parse a default value: `y`, `n`, integer, or quoted string.
    fn parse_default_value(&mut self) -> Result<DefaultValue, String> {
        match self.peek_kind() {
            TokenKind::Yes => {
                self.advance();
                Ok(DefaultValue::Bool(true))
            }
            TokenKind::No => {
                self.advance();
                Ok(DefaultValue::Bool(false))
            }
            TokenKind::Integer(v) => {
                let val = v;
                self.advance();
                Ok(DefaultValue::Integer(val))
            }
            TokenKind::String(ref s) => {
                let val = s.clone();
                self.advance();
                Ok(DefaultValue::Str(val))
            }
            _ => {
                let tok = self.peek();
                Err(format!("{}: expected default value, got {:?}", tok.span, tok.kind))
            }
        }
    }

    /// Parse a dependency expression: `SYMBOL`, `A && B`, `A || B`, `!A`, `(expr)`.
    fn parse_depends_expr(&mut self) -> Result<DependsExpr, String> {
        self.parse_or_expr()
    }

    fn parse_or_expr(&mut self) -> Result<DependsExpr, String> {
        let mut left = self.parse_and_expr()?;
        while self.peek_kind() == TokenKind::Or {
            self.advance();
            let right = self.parse_and_expr()?;
            left = DependsExpr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and_expr(&mut self) -> Result<DependsExpr, String> {
        let mut left = self.parse_unary_expr()?;
        while self.peek_kind() == TokenKind::And {
            self.advance();
            let right = self.parse_unary_expr()?;
            left = DependsExpr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_unary_expr(&mut self) -> Result<DependsExpr, String> {
        if self.peek_kind() == TokenKind::Not {
            self.advance();
            let inner = self.parse_unary_expr()?;
            return Ok(DependsExpr::Not(Box::new(inner)));
        }
        self.parse_primary_expr()
    }

    fn parse_primary_expr(&mut self) -> Result<DependsExpr, String> {
        if self.peek_kind() == TokenKind::LParen {
            self.advance();
            let expr = self.parse_depends_expr()?;
            self.expect(TokenKind::RParen)?;
            return Ok(expr);
        }
        let name = self.expect_ident()?;
        Ok(DependsExpr::Symbol(name))
    }

    /// Parse a binding type: `cfg`, `cfg cumulative`, `const`, `build`.
    fn parse_binding(&mut self) -> Result<Binding, String> {
        let name = self.expect_ident()?;
        match name.as_str() {
            "cfg" => {
                // Check for "cumulative" modifier
                if self.peek_kind() == TokenKind::Ident("cumulative".to_string()) {
                    self.advance();
                    Ok(Binding::CfgCumulative)
                } else {
                    Ok(Binding::Cfg)
                }
            }
            "const" => Ok(Binding::Const),
            "build" => Ok(Binding::Build),
            _ => {
                Err(format!("unknown binding type '{name}', expected cfg, const, or build"))
            }
        }
    }

    /// Parse a bracket-delimited list: `[a, b, c]`.
    fn parse_bracket_list(&mut self) -> Result<Vec<String>, String> {
        if self.peek_kind() != TokenKind::LBracket {
            return Ok(Vec::new());
        }
        self.advance(); // consume [

        let mut items = Vec::new();
        loop {
            if self.peek_kind() == TokenKind::RBracket {
                self.advance();
                break;
            }
            // Accept both quoted strings and identifiers
            let item = match self.peek_kind() {
                TokenKind::String(ref s) => {
                    let val = s.clone();
                    self.advance();
                    val
                }
                TokenKind::Ident(ref s) => {
                    let val = s.clone();
                    self.advance();
                    val
                }
                _ => {
                    let tok = self.peek();
                    return Err(format!("{}: expected string or identifier in list, got {:?}", tok.span, tok.kind));
                }
            };
            items.push(item);

            if self.peek_kind() == TokenKind::Comma {
                self.advance();
            }
        }
        Ok(items)
    }

    // ---- Helpers ----

    fn peek(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn peek_kind(&self) -> TokenKind {
        self.peek().kind.clone()
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos.min(self.tokens.len() - 1)];
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn at_eof(&self) -> bool {
        self.pos >= self.tokens.len() || self.tokens[self.pos].kind == TokenKind::Eof
    }

    fn skip_newlines(&mut self) {
        while !self.at_eof() && self.peek_kind() == TokenKind::Newline {
            self.advance();
        }
    }

    fn expect(&mut self, expected: TokenKind) -> Result<(), String> {
        let tok = self.peek().clone();
        if std::mem::discriminant(&tok.kind) == std::mem::discriminant(&expected) {
            self.advance();
            Ok(())
        } else {
            Err(format!("{}: expected {:?}, got {:?}", tok.span, expected, tok.kind))
        }
    }

    fn expect_ident(&mut self) -> Result<String, String> {
        let tok = self.peek().clone();
        match &tok.kind {
            TokenKind::Ident(s) => {
                let val = s.clone();
                self.advance();
                Ok(val)
            }
            _ => Err(format!("{}: expected identifier, got {:?}", tok.span, tok.kind)),
        }
    }

    fn expect_string(&mut self) -> Result<String, String> {
        let tok = self.peek().clone();
        match &tok.kind {
            TokenKind::String(s) => {
                let val = s.clone();
                self.advance();
                Ok(val)
            }
            _ => Err(format!("{}: expected string, got {:?}", tok.span, tok.kind)),
        }
    }

    fn try_string(&mut self) -> Option<String> {
        if let TokenKind::String(ref s) = self.peek_kind() {
            let val = s.clone();
            self.advance();
            Some(val)
        } else {
            None
        }
    }

    fn expect_integer(&mut self) -> Result<u64, String> {
        let tok = self.peek().clone();
        match &tok.kind {
            TokenKind::Integer(v) => {
                let val = *v;
                self.advance();
                Ok(val)
            }
            _ => Err(format!("{}: expected integer, got {:?}", tok.span, tok.kind)),
        }
    }

    fn expect_newline(&mut self) -> Result<(), String> {
        // Allow EOF as implicit newline
        if self.at_eof() {
            return Ok(());
        }
        match self.peek_kind() {
            TokenKind::Newline | TokenKind::Eof => {
                self.advance();
                Ok(())
            }
            _ => {
                let tok = self.peek();
                Err(format!("{}: expected newline, got {:?}", tok.span, tok.kind))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::lexer;
    use std::path::PathBuf;

    fn parse_str(src: &str) -> KconfigFile {
        let tokens = lexer::tokenize(src, PathBuf::from("test")).unwrap();
        let mut parser = Parser::new(tokens);
        parser.parse().unwrap()
    }

    #[test]
    fn parse_simple_bool() {
        let file = parse_str(r#"
config SMP
    bool "Enable SMP"
    default y
    depends on ACPI
    select APIC
    binding cfg
"#);
        assert_eq!(file.items.len(), 1);
        if let KconfigItem::Config(ref block) = file.items[0] {
            assert_eq!(block.name, "SMP");
            assert_eq!(block.ty.as_ref().unwrap().kind, TypeKind::Bool);
            assert!(matches!(block.default, Some(DefaultValue::Bool(true))));
            assert_eq!(block.selects, vec!["APIC"]);
            assert_eq!(block.bindings, vec![Binding::Cfg]);
        } else {
            panic!("expected Config item");
        }
    }

    #[test]
    fn parse_menu_with_configs() {
        let file = parse_str(r#"
menu "SMP"

config SMP
    bool "Enable SMP"
    default y

config MAX_CPUS
    u32 "Max CPUs"
    default 256
    range 1 256

endmenu
"#);
        assert_eq!(file.items.len(), 1);
        if let KconfigItem::Menu(ref menu) = file.items[0] {
            assert_eq!(menu.title, "SMP");
            assert_eq!(menu.items.len(), 2);
        } else {
            panic!("expected Menu item");
        }
    }

    #[test]
    fn parse_source() {
        let file = parse_str(r#"source "kernel/hadron-kernel/Kconfig""#);
        assert_eq!(file.items.len(), 1);
        assert!(matches!(&file.items[0], KconfigItem::Source(p) if p == "kernel/hadron-kernel/Kconfig"));
    }

    #[test]
    fn parse_choice_type() {
        let file = parse_str(r#"
config LOG_LEVEL
    choice "Kernel log level" [error, warn, info, debug, trace]
    default "debug"
    binding cfg cumulative
    binding const
"#);
        if let KconfigItem::Config(ref block) = file.items[0] {
            assert_eq!(block.name, "LOG_LEVEL");
            let ty = block.ty.as_ref().unwrap();
            assert_eq!(ty.kind, TypeKind::Choice);
            assert_eq!(ty.variants, vec!["error", "warn", "info", "debug", "trace"]);
            assert!(matches!(&block.default, Some(DefaultValue::Str(s)) if s == "debug"));
            assert_eq!(block.bindings, vec![Binding::CfgCumulative, Binding::Const]);
        } else {
            panic!("expected Config item");
        }
    }
}
