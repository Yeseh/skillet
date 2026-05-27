use std::ops::Range;

/// Tokenizer for `.pan` inline syntax.
pub struct Lexer<'a> {
    src: &'a str,
    pos: usize,
}

/// Token categories emitted by [`Lexer`].
#[derive(Debug, PartialEq, Eq)]
pub enum TokenKind {
    /// Plain body text.
    BodyText,
    /// A single backtick.
    Tick,
    /// A double-backtick escape delimiter.
    TickDouble,
    /// The `::` separator in typed refs.
    DoubleColon,
    /// A `{>` fragment opener.
    FragmentOpen,
    /// A `<}` fragment closer.
    FragmentClose,
    /// The value portion of a ref or fragment.
    RefValue,
    /// A `ref::` prefix.
    RefReference,
    /// A `skill::` prefix.
    RefSkill,
    /// An `agent::` prefix.
    RefAgent,
    /// A `cmd::` prefix.
    RefCmd,
    /// A `path::` prefix.
    RefPath,
    /// A `url::` prefix.
    RefUrl,
    /// A `var::` prefix.
    RefVar,
    /// An `env::` prefix.
    RefEnv,
}

/// A single lexical token and its byte range within the source.
#[derive(Debug, PartialEq, Eq)]
pub struct Token {
    /// Token category.
    pub kind: TokenKind,
    /// Byte range covered by the token.
    pub range: Range<u32>,
}

const AGENT_REF: &[u8] = b"agent::";
const SKILL_REF: &[u8] = b"skill::";
const CMD_REF: &[u8] = b"cmd::";
const REFERENCE_REF: &[u8] = b"ref::";
const PATH_REF: &[u8] = b"path::";
const URL_REF: &[u8] = b"url::";
const VAR_REF: &[u8] = b"var::";
const ENV_REF: &[u8] = b"env::";

impl<'a> Lexer<'a> {
    fn ref_kind_at(src_bytes: &[u8], start: usize) -> Option<(TokenKind, usize)> {
        let slice = &src_bytes[start..];
        if slice.starts_with(REFERENCE_REF) {
            Some((TokenKind::RefReference, REFERENCE_REF.len() - 2))
        } else if slice.starts_with(AGENT_REF) {
            Some((TokenKind::RefAgent, AGENT_REF.len() - 2))
        } else if slice.starts_with(CMD_REF) {
            Some((TokenKind::RefCmd, CMD_REF.len() - 2))
        } else if slice.starts_with(SKILL_REF) {
            Some((TokenKind::RefSkill, SKILL_REF.len() - 2))
        } else if slice.starts_with(PATH_REF) {
            Some((TokenKind::RefPath, PATH_REF.len() - 2))
        } else if slice.starts_with(URL_REF) {
            Some((TokenKind::RefUrl, URL_REF.len() - 2))
        } else if slice.starts_with(VAR_REF) {
            Some((TokenKind::RefVar, VAR_REF.len() - 2))
        } else if slice.starts_with(ENV_REF) {
            Some((TokenKind::RefEnv, ENV_REF.len() - 2))
        } else {
            None
        }
    }

    fn tick_prefixed_ref_token(&mut self, start_pos: u32) -> Option<Token> {
        let src_bytes = self.src.as_bytes();
        if start_pos == 0 || src_bytes[(start_pos - 1) as usize] != b'`' {
            return None;
        }

        let start = start_pos as usize;
        let (kind, keyword_len) = Self::ref_kind_at(src_bytes, start)?;
        self.pos += keyword_len - 1;

        Some(self.make_token(kind, start_pos))
    }

    /// Creates a new lexer over `src`.
    #[must_use]
    pub fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    fn make_token(&self, kind: TokenKind, start: u32) -> Token {
        Token {
            kind,
            range: start..self.pos as u32,
        }
    }

    fn next(&mut self) -> Option<u8> {
        if self.pos >= self.src.len() {
            return None;
        }

        let c = self.src.as_bytes()[self.pos];
        self.pos += 1;

        Some(c)
    }

    fn peek(&self, offset: usize) -> Option<u8> {
        self.src.as_bytes().get(self.pos + offset).copied()
    }

    /// Tokenizes the full source string.
    #[must_use]
    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens: Vec<Token> = Vec::new();

        while let Some(c) = self.next() {
            let start_pos = (self.pos - 1) as u32;

            let token: Option<Token> = match c {
                b'a' | b'c' | b'e' | b'p' | b'r' | b's' | b'u' | b'v' => Some(
                    self.tick_prefixed_ref_token(start_pos)
                        .unwrap_or_else(|| self.make_body(start_pos)),
                ),
                b'`' => {
                    let p1 = self.peek(0);
                    let p2 = self.peek(1);

                    if p1 == Some(b'`') && p2 == Some(b'`') {
                        self.pos += 2;
                        Some(self.make_body(start_pos))
                    } else if p1 == Some(b'`') {
                        self.pos += 1;
                        tokens.push(self.make_token(TokenKind::TickDouble, start_pos));
                        Some(self.make_body(self.pos as u32))
                    } else {
                        Some(self.make_token(TokenKind::Tick, start_pos))
                    }
                }
                b'{' => {
                    if self.peek(0) != Some(b'>') {
                        Some(self.make_body(start_pos))
                    } else {
                        self.pos += 1;
                        tokens.push(self.make_token(TokenKind::FragmentOpen, start_pos));
                        Some(self.make_fragment_id(self.pos as u32))
                    }
                }
                b'<' => {
                    if self.peek(0) == Some(b'}') {
                        self.pos += 1;
                        Some(self.make_token(TokenKind::FragmentClose, start_pos))
                    } else {
                        None
                    }
                }
                b':' => {
                    if self.peek(0) == Some(b':') {
                        self.pos += 1;
                        tokens.push(self.make_token(TokenKind::DoubleColon, start_pos));
                        Some(self.make_ref_value(self.pos as u32))
                    } else {
                        Some(self.make_body(start_pos))
                    }
                }
                _ => Some(self.make_body(start_pos)),
            };

            if let Some(t) = token {
                tokens.push(t);
            }
        }

        tokens
    }

    fn make_fragment_id(&mut self, start_pos: u32) -> Token {
        while let Some(c) = self.next() {
            if c == b'<' {
                self.pos -= 1;
                break;
            }
        }
        self.make_token(TokenKind::RefValue, start_pos)
    }

    fn make_ref_value(&mut self, start_pos: u32) -> Token {
        while let Some(c) = self.next() {
            if c == b'`' {
                self.pos -= 1;
                break;
            }
        }
        self.make_token(TokenKind::RefValue, start_pos)
    }

    fn make_body(&mut self, start_pos: u32) -> Token {
        while let Some(c) = self.next() {
            match c {
                b'`' => {
                    let p1 = self.peek(0);
                    let p2 = self.peek(1);

                    if p1 == Some(b'`') && p2 == Some(b'`') {
                        self.pos += 2;
                    } else {
                        self.pos -= 1;
                        break;
                    }
                }
                b'{' => {
                    self.pos -= 1;
                    break;
                }
                _ => {}
            }
        }
        self.make_token(TokenKind::BodyText, start_pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slice<'a>(src: &'a str, token: &Token) -> &'a str {
        &src[token.range.start as usize..token.range.end as usize]
    }

    #[test]
    fn tokenizes_simple_ref_and_fragment_sequence() {
        let s = "`ref::foo/bar.md` some text {> footer <}";
        let mut l = Lexer::new(s);

        let tokens = l.tokenize();

        assert_eq!(tokens.len(), 9);
    }

    #[test]
    fn tokenizes_fragment_tokens() {
        let s = "{> footer <}";
        let mut l = Lexer::new(s);
        let tokens = l.tokenize();

        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].kind, TokenKind::FragmentOpen);
        assert_eq!(slice(s, &tokens[0]), "{>");
        assert_eq!(tokens[1].kind, TokenKind::RefValue);
        assert_eq!(slice(s, &tokens[1]), " footer ");
        assert_eq!(tokens[2].kind, TokenKind::FragmentClose);
        assert_eq!(slice(s, &tokens[2]), "<}");
    }

    #[test]
    fn tokenizes_fragment_with_surrounding_body_text() {
        let s = "prefix {>frag-id<} suffix";
        let mut l = Lexer::new(s);
        let tokens = l.tokenize();

        assert_eq!(tokens.len(), 5);
        assert_eq!(tokens[0].kind, TokenKind::BodyText);
        assert_eq!(slice(s, &tokens[0]), "prefix ");
        assert_eq!(tokens[1].kind, TokenKind::FragmentOpen);
        assert_eq!(slice(s, &tokens[1]), "{>");
        assert_eq!(tokens[2].kind, TokenKind::RefValue);
        assert_eq!(slice(s, &tokens[2]), "frag-id");
        assert_eq!(tokens[3].kind, TokenKind::FragmentClose);
        assert_eq!(slice(s, &tokens[3]), "<}");
        assert_eq!(tokens[4].kind, TokenKind::BodyText);
        assert_eq!(slice(s, &tokens[4]), " suffix");
    }

    #[test]
    fn tick_range_is_one_byte() {
        let s = "`";
        let mut l = Lexer::new(s);
        let tokens = l.tokenize();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenKind::Tick);
        assert_eq!(tokens[0].range, 0..1);
    }

    #[test]
    fn code_fence_is_single_body_text() {
        let s = "```rust\nlet x = 1;\n```";
        let mut l = Lexer::new(s);
        let tokens = l.tokenize();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenKind::BodyText);
        assert_eq!(slice(s, &tokens[0]), s);
    }

    #[test]
    fn ref_without_ticks_is_body_text() {
        let s = "skill::my-agent";
        let mut l = Lexer::new(s);
        let tokens = l.tokenize();
        assert_eq!(tokens[0].kind, TokenKind::BodyText);
        assert_eq!(slice(s, &tokens[0]), "skill::my-agent");
    }

    #[test]
    fn escaped_ref_is_body_text() {
        let s = "``skill::my-agent``";
        let mut l = Lexer::new(s);
        let tokens = l.tokenize();
        assert_eq!(tokens[0].kind, TokenKind::TickDouble);
        assert_eq!(tokens[1].kind, TokenKind::BodyText);
        assert_eq!(slice(s, &tokens[1]), "skill::my-agent");
        assert_eq!(tokens[2].kind, TokenKind::TickDouble);
    }

    #[test]
    fn tokenizes_skill_ref() {
        let s = "`skill::my-agent`";
        let mut l = Lexer::new(s);
        let tokens = l.tokenize();
        assert_eq!(tokens[0].kind, TokenKind::Tick);
        assert_eq!(tokens[1].kind, TokenKind::RefSkill);
        assert_eq!(slice(s, &tokens[1]), "skill");
        assert_eq!(tokens[2].kind, TokenKind::DoubleColon);
        assert_eq!(tokens[3].kind, TokenKind::RefValue);
        assert_eq!(slice(s, &tokens[3]), "my-agent");
        assert_eq!(tokens[4].kind, TokenKind::Tick);
    }

    #[test]
    fn tokenizes_agent_ref() {
        let s = "`agent::my-agent`";
        let mut l = Lexer::new(s);
        let tokens = l.tokenize();
        assert_eq!(tokens[0].kind, TokenKind::Tick);
        assert_eq!(tokens[1].kind, TokenKind::RefAgent);
        assert_eq!(slice(s, &tokens[1]), "agent");
        assert_eq!(tokens[2].kind, TokenKind::DoubleColon);
        assert_eq!(tokens[3].kind, TokenKind::RefValue);
        assert_eq!(slice(s, &tokens[3]), "my-agent");
        assert_eq!(tokens[4].kind, TokenKind::Tick);
    }

    #[test]
    fn tokenizes_cmd_ref() {
        let s = "`cmd::git push`";
        let mut l = Lexer::new(s);
        let tokens = l.tokenize();
        assert_eq!(tokens[0].kind, TokenKind::Tick);
        assert_eq!(tokens[1].kind, TokenKind::RefCmd);
        assert_eq!(slice(s, &tokens[1]), "cmd");
        assert_eq!(tokens[2].kind, TokenKind::DoubleColon);
        assert_eq!(tokens[3].kind, TokenKind::RefValue);
        assert_eq!(slice(s, &tokens[3]), "git push");
        assert_eq!(tokens[4].kind, TokenKind::Tick);
    }

    #[test]
    fn tokenizes_reference_ref() {
        let s = "`ref::./references/test.md`";
        let mut l = Lexer::new(s);
        let tokens = l.tokenize();
        assert_eq!(tokens[0].kind, TokenKind::Tick);
        assert_eq!(tokens[1].kind, TokenKind::RefReference);
        assert_eq!(slice(s, &tokens[1]), "ref");
        assert_eq!(tokens[2].kind, TokenKind::DoubleColon);
        assert_eq!(tokens[3].kind, TokenKind::RefValue);
        assert_eq!(slice(s, &tokens[3]), "./references/test.md");
        assert_eq!(tokens[4].kind, TokenKind::Tick);
    }

    #[test]
    fn unknown_keyword_is_body_text() {
        let s = "foo::bar";
        let mut l = Lexer::new(s);
        let tokens = l.tokenize();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenKind::BodyText);
    }

    #[test]
    fn tokenizes_path_ref() {
        let s = "`path::some/file.md`";
        let mut l = Lexer::new(s);
        let tokens = l.tokenize();
        assert_eq!(tokens[0].kind, TokenKind::Tick);
        assert_eq!(tokens[1].kind, TokenKind::RefPath);
        assert_eq!(slice(s, &tokens[1]), "path");
        assert_eq!(tokens[2].kind, TokenKind::DoubleColon);
        assert_eq!(tokens[3].kind, TokenKind::RefValue);
        assert_eq!(slice(s, &tokens[3]), "some/file.md");
        assert_eq!(tokens[4].kind, TokenKind::Tick);
    }

    #[test]
    fn tokenizes_url_ref() {
        let s = "`url::https://example.com`";
        let mut l = Lexer::new(s);
        let tokens = l.tokenize();
        assert_eq!(tokens[0].kind, TokenKind::Tick);
        assert_eq!(tokens[1].kind, TokenKind::RefUrl);
        assert_eq!(slice(s, &tokens[1]), "url");
        assert_eq!(tokens[2].kind, TokenKind::DoubleColon);
        assert_eq!(tokens[3].kind, TokenKind::RefValue);
        assert_eq!(slice(s, &tokens[3]), "https://example.com");
        assert_eq!(tokens[4].kind, TokenKind::Tick);
    }

    #[test]
    fn empty_input_produces_no_tokens() {
        let s = "";
        let mut l = Lexer::new(s);
        let tokens = l.tokenize();
        assert_eq!(tokens.len(), 0);
    }
}
