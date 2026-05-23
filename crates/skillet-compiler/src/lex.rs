use std::{ops::Range};


pub struct Lexer<'a> {
    pub src: &'a str,
    pub pos: usize,
}

#[derive(Debug, PartialEq)]
pub enum TokenKind {
    BodyText,
    Tick,
    TickDouble,
    DoubleColon,

    FragmentOpen,
    FragmentClose,

    RefValue,
    RefReference,
    RefSKill,
    RefAgent,
    RefCmd,
}

#[derive(Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub range: Range<u32>,
}

impl Token {
    pub fn new(kind: TokenKind, start: u32, end: u32) -> Self {
        Token {
            kind,
            range: Range {
                start,
                end
            }
        }
    }
}

const AGENT_REF: &'static [u8] = b"agent::";
const SKILL_REF: &'static [u8] = b"skill::";
const CMD_REF: &'static [u8]= b"cmd::";
const REFERENCE_REF: &'static [u8] = b"ref::";

impl <'a> Lexer<'a> {
    fn ref_kind_at(src_bytes: &[u8], start: usize) -> Option<(TokenKind, usize)> {
        match () {
            _ if src_bytes[start..].starts_with(REFERENCE_REF) => {
                Some((TokenKind::RefReference, REFERENCE_REF.len()))
            }
            _ if src_bytes[start..].starts_with(AGENT_REF) => {
                Some((TokenKind::RefAgent, AGENT_REF.len()))
            }
            _ if src_bytes[start..].starts_with(CMD_REF) => {
                Some((TokenKind::RefCmd, CMD_REF.len()))
            }
            _ if src_bytes[start..].starts_with(SKILL_REF) => {
                Some((TokenKind::RefSKill, SKILL_REF.len()))
            }
            _ => None,
        }
    }

    fn tick_prefixed_ref_token(&mut self, start_pos: u32) -> Option<Token> {
        let src_bytes = self.src.as_bytes();
        if start_pos == 0 || src_bytes[(start_pos - 1) as usize] != b'`' {
            return None;
        }

        let start = start_pos as usize;
        let (kind, prefix_len) = Self::ref_kind_at(src_bytes, start)?;
        self.pos += prefix_len - 3;

        Some(Token {
            kind,
            range: Range {
                start: start_pos,
                end: self.pos as u32,
            },
        })
    }

    pub fn new(src: &'a str) -> Self {
        Self {
            src,
            pos: 0
        }
    }

    pub fn next(&mut self) -> Option<u8> {
        if self.pos >= self.src.len() {
            return None;
        }

        let c = self.src.as_bytes()[self.pos];
        self.pos += 1;

        Some(c)
    }

    pub fn peek(&self, offset: usize) -> Option<u8> {
        self.src.as_bytes().get(self.pos + offset).copied()
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens: Vec<Token> = Vec::new();

        while let Some(c) = self.next() {
            let start_pos = (self.pos - 1) as u32;

            let token: Token = match c {
                b'a' | b's' | b'r' | b'c' => {
                    self.tick_prefixed_ref_token(start_pos)
                        .unwrap_or_else(|| self.make_body(start_pos))
                },
                b'`' => {
                    let p1 = self.peek(0);
                    let p2 = self.peek(1);

                    // Code fence
                    if p1 == Some(b'`') && p2 == Some(b'`') {
                        self.pos += 2;
                        self.make_body(start_pos)
                    }
                    // Double tick
                    else if p1 == Some(b'`') {
                        self.pos += 1;

                        tokens.push(
                            Token::new(
                                TokenKind::TickDouble, 
                                start_pos, 
                                self.pos as u32)
                        );

                        self.make_body(self.pos as u32)
                    }
                    else {
                        Token::new(TokenKind::Tick, start_pos, self.pos as u32)
                    }
                }
                b'{' => {
                    let p1 = self.peek(0);
                    if p1 != Some(b'>') {
                        self.make_body(start_pos)
                    }
                    else {
                        self.pos += 1;

                        tokens.push(Token {
                            kind: TokenKind::FragmentOpen,
                            range: Range {
                                start: start_pos,
                                end: self.pos as u32
                            }
                        });

                        self.make_fragment_id(self.pos as u32)
                    }
                },
                b'<' => {
                    let p1 = self.peek(0);
                    if p1 == Some(b'}') {
                        self.pos += 1;
                        tokens.push(
                            Token::new(
                                TokenKind::FragmentClose, 
                                start_pos, 
                                self.pos as u32
                        ))
                    };

                    continue;
                },
                b':' => {
                    let p1 = self.peek(0);
                    if p1 == Some(b':')  {
                        self.pos += 1;

                        tokens.push(Token {
                            kind: TokenKind::DoubleColon,
                            range: Range {
                                start: start_pos,
                                end: self.pos as u32
                            }
                        });

                        self.make_ref_value((self.pos) as u32)
                    }
                    else {
                        self.make_body(start_pos)
                    }
                },
                _ => self.make_body(start_pos)
            };

            tokens.push(token);
        }

        return tokens;
    }

    fn make_fragment_id(&mut self, start_pos: u32) -> Token {
        while let Some(c) = self.next() {
            match c {
                b'<' => {
                    self.pos -= 1;
                    break;
                },
                // Skip on all characters for now
                _ => ()
           } 
        }

        Token {
            kind: TokenKind::RefValue,
            range: Range {
                start: start_pos,
                end: self.pos as u32
            }
        }
    }

    /// TODO: Make this parse urls, paths, or values directly
    fn make_ref_value(&mut self, start_pos: u32) -> Token {
        while let Some(c) = self.next() {
            match c {
                b'`' => {
                    self.pos -= 1;
                    break;
                },
                // Skip on all characters for now
                _ => ()
           } 
        }

        Token {
            kind: TokenKind::RefValue,
            range: Range {
                start: start_pos,
                end: self.pos as u32
            }
        }
    }

    fn make_body(&mut self, start_pos: u32) -> Token {
        while let Some(c) = self.next() {
            match c {
                b'`' => {
                    let p1 = self.peek(0);
                    let p2 = self.peek(1);

                    if p1 == Some(b'`') && p2 == Some(b'`') {
                        self.pos += 2;
                    }
                    else {
                        self.pos -= 1;
                        break;
                    }
                },
                b'{' => {
                    self.pos -= 1;
                    break;
                },
                _ => ()
           } 
        }

        Token {
            kind: TokenKind::BodyText,
            range: Range {
                start: start_pos,
                end: self.pos as u32
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn slice<'a>(src: &'a str, token: &Token) -> &'a str {
       &src[token.range.start as usize..token.range.end as usize]
    }

    #[test]
    fn test_simple() {
        let s = "`ref::foo/bar.md` some text {> footer <}";
        let mut  l = Lexer::new(s);

        let tokens = l.tokenize();

        assert_eq!(tokens.len(), 9);
    }

    #[test]
    fn test_fragment_tokens() {
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
    fn test_fragment_with_surrounding_body_text() {
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
   fn test_tick_range_is_one_byte() {
       // A single backtick should have a range of length 1, not 0
       let s = "`";
       let mut l = Lexer::new(s);
       let tokens = l.tokenize();
       assert_eq!(tokens.len(), 1);
       assert_eq!(tokens[0].kind, TokenKind::Tick);
       assert_eq!(tokens[0].range, 0..1);
   }

   #[test]
   fn test_code_fence_is_single_body_text() {
       // Triple backtick fence including delimiters becomes one BodyText
       let s = "```rust\nlet x = 1;\n```";
       let mut l = Lexer::new(s);
       let tokens = l.tokenize();
       assert_eq!(tokens.len(), 1);
       assert_eq!(tokens[0].kind, TokenKind::BodyText);
       assert_eq!(slice(s, &tokens[0]), s); // full source, delimiters included
   }

    #[test]
   fn test_ref_without_ticks_is_body_text() {
       let s = "skill::my-agent";
       let mut l = Lexer::new(s);
       let tokens = l.tokenize();
       assert_eq!(tokens[0].kind, TokenKind::BodyText);
       assert_eq!(slice(s, &tokens[0]), "skill::my-agent");
   }


    #[test]
   fn test_escaped_ref_is_body_text() {
       let s = "``skill::my-agent``";
       let mut l = Lexer::new(s);
       let tokens = l.tokenize();
       assert_eq!(tokens[0].kind, TokenKind::TickDouble);
       assert_eq!(tokens[1].kind, TokenKind::BodyText);
       assert_eq!(slice(s, &tokens[1]), "skill::my-agent");
       assert_eq!(tokens[2].kind, TokenKind::TickDouble);
   }

   #[test]
   fn test_skill_ref() {
       let s = "`skill::my-agent`";
       let mut l = Lexer::new(s);
       let tokens = l.tokenize();
       assert_eq!(tokens[0].kind, TokenKind::Tick);
       assert_eq!(tokens[1].kind, TokenKind::RefSKill);
       assert_eq!(slice(s, &tokens[1]), "skill");
       assert_eq!(tokens[2].kind, TokenKind::DoubleColon);
       assert_eq!(tokens[3].kind, TokenKind::RefValue);
       assert_eq!(slice(s, &tokens[3]), "my-agent");
       assert_eq!(tokens[4].kind, TokenKind::Tick);
   }

   #[test]
   fn test_agent_ref() {
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
   fn test_cmd_ref() {
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
   fn test_reference_ref() {
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
   fn test_unknown_keyword_is_body_text() {
       // "foo::bar" is not a known ref type — entire thing is BodyText
       let s = "foo::bar";
       let mut l = Lexer::new(s);
       let tokens = l.tokenize();
       assert_eq!(tokens.len(), 1);
       assert_eq!(tokens[0].kind, TokenKind::BodyText);
   }

   #[test]
   fn test_empty_input() {
       let s = "";
       let mut l = Lexer::new(s);
       let tokens = l.tokenize();
       assert_eq!(tokens.len(), 0);
   }
}
