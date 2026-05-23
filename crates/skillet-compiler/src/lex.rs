use std::{ops::Range, os::raw};


pub struct Lexer<'a> {
    pub src: &'a str,
    pub pos: usize,
    pub escaped: bool
}

#[derive(Debug)]
pub enum TokenKind {
    Invalid,
    BodyText,
    Tick,
    TickDouble,
    DoubleColon,
    ReferenceRef,
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

const AGENT_REF: &'static [u8] = b"agent::";
const SKILL_REF: &'static [u8] = b"skill::";
const CMD_REF: &'static [u8]= b"cmd::";
const REFERENCE_REF: &'static [u8] = b"ref::";

impl <'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Self {
            src,
            pos: 0,
            escaped: false
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
                b'`' => {
                    let p1 = self.peek(0);
                    let p2 = self.peek(1);

                    // Code fence
                    if p1 == Some(b'`') && p2 == Some(b'`') {
                        self.make_body(start_pos)
                    }
                    // Double tick
                    else if p1 == Some(b'`') {
                        self.pos += 1;

                        let token = Token {
                            kind: TokenKind::TickDouble,
                            range: Range { 
                                start: start_pos, 
                                end: self.pos as u32 
                            }
                        };

                        token
                    }
                    else {
                        Token {
                            kind: TokenKind::Tick,
                            range: Range { 
                                start: start_pos, 
                                end: start_pos 
                            }
                        }
                    }
                }
                b'{' => {
                    let p1 = self.peek(0);
                    if p1 != Some(b'>') {
                        self.make_body(start_pos)
                    }
                    else {
                        self.pos += 1;
                        Token {
                            kind: TokenKind::FragmentOpen,
                            range: Range {
                                start: start_pos,
                                end: self.pos as u32
                            }
                        }
                    }
                },
                b':' => {
                    let p1 = self.peek(0);
                    if p1 == Some(b':')  {
                        self.pos += 1;
                        Token {
                            kind: TokenKind::DoubleColon,
                            range: Range {
                                start: start_pos,
                                end: self.pos as u32
                            }
                        }
                    }
                    else {
                        self.make_body(start_pos)
                    }
                },
                _ => { 
                    if self.src.as_bytes()[start_pos as usize..].starts_with(REFERENCE_REF) {
                        self.pos += REFERENCE_REF.len() - 2;
                        Token {
                            kind: TokenKind::RefReference,
                            range: Range {
                                start: start_pos,
                                end: self.pos as u32
                            }
                        }
                    }
                    else if self.src.as_bytes()[start_pos as usize..].starts_with(AGENT_REF) {
                        self.pos += AGENT_REF.len() - 2;
                        Token {
                            kind: TokenKind::RefAgent,
                            range: Range {
                                start: start_pos,
                                end: self.pos as u32
                            }
                        }
                    }
                    else if self.src.as_bytes()[start_pos as usize..].starts_with(CMD_REF) {
                        self.pos += CMD_REF.len() - 2;
                        Token {
                            kind: TokenKind::RefCmd,
                            range: Range {
                                start: start_pos,
                                end: self.pos as u32
                            }
                        }
                    }
                    else if self.src.as_bytes()[start_pos as usize..].starts_with(SKILL_REF) {
                        self.pos += SKILL_REF.len() -2;
                        Token {
                            kind: TokenKind::RefSKill,
                            range: Range {
                                start: start_pos,
                                end: self.pos as u32
                            }
                        }
                    }
                    else {
                        self.make_body(start_pos)
                    }
                }
            };

            tokens.push(token);
        }

        return tokens;
    }

    fn make_body(&mut self, start_pos: u32) -> Token {
        while let Some(c) = self.next() {
            match c {
                b'`' | b'{' => {
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
