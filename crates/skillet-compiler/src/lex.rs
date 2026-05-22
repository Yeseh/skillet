use std::ops::Range;


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
    CmdRef,
    ReferenceRef,
    FragmentOpen,
    FragmentClose,
    RefValue,
    RefName
}

#[derive(Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub range: Range<u32>,
}

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
            let start_pos = self.pos as u32;

            match c {
                b'`' => {
                    let p1 = self.peek(1);
                    let p2 = self.peek(2);

                    /// Code fence
                    if p1 == Some(b'`') && p2 == Some(b'`') {
                        let token = self.make_body();
                        tokens.push(token);
                    }
                    /// Double tick
                    else if p1 == Some(b'`') {
                        self.pos += 1;

                        let token = Token {
                            kind: TokenKind::TickDouble,
                            range: Range { 
                                start: start_pos, 
                                end: self.pos as u32 
                            }
                        };

                        tokens.push(token);
                    }
                    /// Possible ref
                    else {
                        let token = Token {
                            kind: TokenKind::Tick,
                            range: Range { 
                                start: start_pos, 
                                end: start_pos 
                            }
                        };

                        tokens.push(token);

                        

                    }

                    token.kind = TokenKind::Tick;
                }
                b'{' => {
                    let p1 = self.peek(1);
                    if p1 != Some(b'{') {
                        token.Kind = TokenKind::BodyText;
                        continue;
                    }

                    let p2 = self.peek(2);
                    if p2 == Some(b'>') {
                        token.kind = TokenKind::FragmentOpen;
                        self.pos += 2;
                        token.range.end = self.pos as u32;
                    }
                },
                b'a'..b'z' => {
                    
                } 
                _ => ()
            }


            match c {
            }
        }

        return tokens;
    }

    fn make_body(&mut self) -> Token {
        let mut token = Token {
            kind: TokenKind::BodyText,
            range: Range {
                start: self.pos as u32,
                end: self.pos as u32
            }
        };

        while let Some(c) = self.next() {
            match c {
                b'`' | b'{' => {
                    token.range.end = self.pos as u32;
                    break;
                },
                _ => ()
           } 
        }

        token;
    }

    fn make_ref_type(&mut self) -> Token {
        let mut token = Token {
            kind: TokenKind::BodyText,
            range: Range {
                start: self.pos as u32,
                end: self.pos as u32
            }
        };

        while let Some(c) = self.next() {
            match c {
                b':' => {
                    let p1 = self.peek(1);

                    if let Some(p) = b':' {
                        token.range.end = self.pos;
                        break;
                    }

                },
                b'a'..b'z' => {

                },
                _ => ()
           } 
        }

        token
    }
}
