use std::{iter::Peekable, ops::Range, slice::Iter};

use crate::{
    PanSource,
    lex::{Lexer, Token, TokenKind},
};

pub enum RefKind {
    Agent,
    Skill,
    Cmd,
    Path,
    Url,
    Reference,
    Var,
    Env,
}

pub enum ParseErrorKind {
    UnexpectedEOF,
    UnexpectedToken,
    ExpectedRefValueAfterDoubleColon,
    ExpectedRefValueAfterFragmentOpen,
    ExpectedBodyTextAfterEscape,
    UnclosedFragment,
    UnclosedEscape,
    UnclosedRef,
}

pub enum Node {
    Ref {
        kind: RefKind,
        value: String,
        source_range: Range<u32>,
    },
    Fragment {
        value: String,
        source_range: Range<u32>,
    },
    RefSuspect {
        source_range: Range<u32>,
    },
    EscapedBody {
        source_range: Range<u32>,
    },
    Body {
        source_range: Range<u32>,
    },
}

pub struct ParseError {
    pub range: Range<u32>,
    pub kind: ParseErrorKind,
}

pub struct PanParse<'a> {
    pub path: Option<std::path::PathBuf>,
    pub nodes: Vec<Node>,
    pub errors: Vec<ParseError>,
    src: &'a str,
}

impl<'a> PanParse<'a> {
    pub fn new(pan_source: &'a PanSource) -> Self {
        Self {
            path: pan_source.path.clone(),
            src: &pan_source.src,
            nodes: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn get_source_string(&self, token: &Token) -> String {
        self.src[token.range.start as usize..token.range.end as usize].to_string()
    }

    fn make_ref_node(
        &mut self,
        token_iter: &mut Peekable<Iter<'_, Token>>,
        start_offset: u32,
        kind: RefKind,
    ) {
        // Consume the ref-kind token (RefSkill, RefAgent, etc.) that was already peeked by the
        // caller. The iterator is pointing at it, not at DoubleColon yet.
        let rf = token_iter.next().unwrap();

        let peek_double_colon = token_iter.peek();
        if peek_double_colon.is_none_or(|t| t.kind != TokenKind::DoubleColon) {
            return;
        }

        let double_colon = token_iter.next().unwrap();

        // `skill:: might be intended to be a ref, should error
        let peek_value = token_iter.peek();
        if peek_value.is_none_or(|t| t.kind != TokenKind::RefValue) {
            self.errors.push(ParseError {
                kind: ParseErrorKind::ExpectedRefValueAfterDoubleColon,
                range: double_colon.range.clone(),
            });

            // Go to the next token??? This should recover, how? What is the error boundrary?
            // Find next backtick?
            Self::recover(token_iter);
            return;
        }

        // Missing backtick: `skill::my-skill some other body
        let ref_value = token_iter.next().unwrap();
        let peek_tick = token_iter.peek();
        if peek_tick.is_none_or(|t| t.kind != TokenKind::Tick) {
            self.errors.push(ParseError {
                kind: ParseErrorKind::UnclosedRef,
                range: start_offset..rf.range.end,
            });

            Self::recover(token_iter);
            return;
        }

        // TODO: Actually have the parse rules here for different formats

        let closing_tick = token_iter.next().unwrap();

        self.nodes.push(Node::Ref {
            kind,
            value: self.get_source_string(ref_value),
            source_range: start_offset..closing_tick.range.end,
        });
    }

    fn recover(iter: &mut Peekable<Iter<'_, Token>>) {
        while let Some(t) = iter.peek() {
            match t.kind {
                TokenKind::Tick => break,
                TokenKind::FragmentOpen => break,
                _ => {
                    _ = iter.next();
                }
            }
        }
    }

    pub fn parse(&mut self) {
        let mut lexer = Lexer::new(self.src);
        let tokens = lexer.tokenize();
        let mut token_iter = tokens.iter().peekable();

        while let Some(t) = token_iter.next() {
            match t.kind {
                // Emit body node
                TokenKind::BodyText => {
                    self.nodes.push(Node::Body {
                        source_range: t.range.clone(),
                    });
                }
                TokenKind::Tick => {
                    let peek = token_iter.peek();

                    match peek {
                        Some(p) => {
                            match p.kind {
                                TokenKind::RefSkill => self.make_ref_node(
                                    &mut token_iter,
                                    t.range.start,
                                    RefKind::Skill,
                                ),
                                TokenKind::RefReference => self.make_ref_node(
                                    &mut token_iter,
                                    t.range.start,
                                    RefKind::Reference,
                                ),
                                TokenKind::RefAgent => self.make_ref_node(
                                    &mut token_iter,
                                    t.range.start,
                                    RefKind::Agent,
                                ),
                                TokenKind::RefCmd => {
                                    self.make_ref_node(
                                        &mut token_iter,
                                        t.range.start,
                                        RefKind::Cmd,
                                    );
                                }
                                TokenKind::RefPath => self.make_ref_node(
                                    &mut token_iter,
                                    t.range.start,
                                    RefKind::Path,
                                ),
                                TokenKind::RefUrl => {
                                    self.make_ref_node(
                                        &mut token_iter,
                                        t.range.start,
                                        RefKind::Url,
                                    );
                                }
                                TokenKind::RefVar => {
                                    self.make_ref_node(
                                        &mut token_iter,
                                        t.range.start,
                                        RefKind::Var,
                                    );
                                }
                                TokenKind::RefEnv => {
                                    self.make_ref_node(
                                        &mut token_iter,
                                        t.range.start,
                                        RefKind::Env,
                                    );
                                }
                                TokenKind::BodyText => {
                                    let body = token_iter.next().unwrap();

                                    let peek_tick = token_iter.peek();
                                    if peek_tick.is_none_or(|t| t.kind != TokenKind::Tick) {
                                        // Treat this as body, normal markdown syntax
                                        self.nodes.push(Node::Body {
                                            source_range: t.range.start..body.range.end,
                                        });

                                        continue;
                                    }

                                    let closing_tick = token_iter.next().unwrap();
                                    self.nodes.push(Node::RefSuspect {
                                        source_range: Range {
                                            start: t.range.start,
                                            end: closing_tick.range.end,
                                        },
                                    });
                                }
                                // Should be unreachable if the rest of the logic is sound
                                _ => self.errors.push(ParseError {
                                    kind: ParseErrorKind::UnexpectedToken,
                                    range: t.range.start..p.range.end,
                                }),
                            }
                        }
                        // Is this a parse error? Might just be the last character someone used in a document
                        None => self.errors.push(ParseError {
                            range: t.range.clone(),
                            kind: ParseErrorKind::UnexpectedEOF,
                        }),
                    }
                }
                TokenKind::TickDouble => {
                    let peek_body = token_iter.peek();
                    if peek_body.is_none_or(|t| t.kind != TokenKind::BodyText) {
                        self.errors.push(ParseError {
                            kind: ParseErrorKind::ExpectedBodyTextAfterEscape,
                            range: t.range.clone(),
                        });

                        Self::recover(&mut token_iter);
                        continue;
                    }

                    // Consume body
                    _ = token_iter.next();

                    let peek_close = token_iter.peek();
                    if peek_close.is_none_or(|t| t.kind != TokenKind::TickDouble) {
                        self.errors.push(ParseError {
                            kind: ParseErrorKind::UnclosedEscape,
                            range: Range {
                                start: t.range.start,
                                end: t.range.end,
                            },
                        });

                        Self::recover(&mut token_iter);
                        continue;
                    }

                    let close = token_iter.next().unwrap();
                    self.nodes.push(Node::EscapedBody {
                        source_range: t.range.start..close.range.end,
                    });
                }
                TokenKind::FragmentOpen => {
                    let peek_value = token_iter.peek();
                    if peek_value.is_none_or(|t| t.kind != TokenKind::RefValue) {
                        self.errors.push(ParseError {
                            kind: ParseErrorKind::ExpectedRefValueAfterFragmentOpen,
                            range: t.range.clone(),
                        });

                        Self::recover(&mut token_iter);
                        continue;
                    }

                    let ref_value = token_iter.next().unwrap();

                    let peek_close = token_iter.peek();
                    if peek_close.is_none_or(|t| t.kind != TokenKind::FragmentClose) {
                        self.errors.push(ParseError {
                            kind: ParseErrorKind::UnclosedFragment,
                            range: t.range.clone(),
                        });

                        Self::recover(&mut token_iter);
                        continue;
                    }

                    let close = token_iter.next().unwrap();
                    self.nodes.push(Node::Fragment {
                        value: self.get_source_string(ref_value),
                        source_range: t.range.start..close.range.end,
                    });
                }
                // Should be unreachable if the rest of the logic is sound
                _ => self.errors.push(ParseError {
                    kind: ParseErrorKind::UnexpectedToken,
                    range: t.range.clone(),
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PanSource;

    struct ParseResult {
        nodes: Vec<Node>,
        errors: Vec<ParseError>,
    }

    fn parse_str(src: &str) -> ParseResult {
        let pan = PanSource::new(src.to_string(), None);
        let mut parsed = PanParse::new(&pan);
        parsed.parse();
        ParseResult {
            nodes: parsed.nodes,
            errors: parsed.errors,
        }
    }

    fn node_kind_name(node: &Node) -> &'static str {
        match node {
            Node::Ref { .. } => "Ref",
            Node::Fragment { .. } => "Fragment",
            Node::RefSuspect { .. } => "RefSuspect",
            Node::EscapedBody { .. } => "EscapedBody",
            Node::Body { .. } => "Body",
        }
    }

    // ── Body ─────────────────────────────────────────────────────────────

    #[test]
    fn plain_body_text_produces_body_node() {
        let result = parse_str("hello world");
        assert!(result.errors.is_empty(), "expected no errors");
        assert_eq!(result.nodes.len(), 1);
        assert!(
            matches!(&result.nodes[0], Node::Body { .. }),
            "expected Body"
        );
    }

    #[test]
    fn body_node_source_range_covers_full_text() {
        let src = "hello world";
        let result = parse_str(src);
        assert!(result.errors.is_empty());
        match &result.nodes[0] {
            Node::Body { source_range } => {
                assert_eq!(*source_range, 0..u32::try_from(src.len()).unwrap());
            }
            n => panic!("expected Body, got {}", node_kind_name(n)),
        }
    }

    #[test]
    fn empty_input_produces_no_nodes() {
        let result = parse_str("");
        assert!(result.errors.is_empty());
        assert!(result.nodes.is_empty());
    }

    // ── Fragment ─────────────────────────────────────────────────────────

    #[test]
    fn fragment_insertion_produces_single_fragment_node() {
        let src = "{>my-fragment<}";
        let result = parse_str(src);
        assert!(result.errors.is_empty(), "expected no parse errors");
        assert_eq!(result.nodes.len(), 1, "expected exactly one node");
        match &result.nodes[0] {
            Node::Fragment { value, .. } => assert_eq!(value, "my-fragment"),
            n => panic!("expected Fragment, got {}", node_kind_name(n)),
        }
    }

    #[test]
    fn fragment_source_range_covers_delimiters() {
        let src = "{>my-fragment<}";
        let result = parse_str(src);
        assert!(result.errors.is_empty());
        match &result.nodes[0] {
            Node::Fragment { source_range, .. } => {
                assert_eq!(*source_range, 0..u32::try_from(src.len()).unwrap());
            }
            n => panic!("expected Fragment, got {}", node_kind_name(n)),
        }
    }

    #[test]
    fn fragment_with_surrounding_body() {
        let result = parse_str("before {>my-frag<} after");
        assert!(result.errors.is_empty());
        // body + fragment + body (trailing body may or may not exist depending on lexer)
        assert!(
            result
                .nodes
                .iter()
                .any(|n| matches!(n, Node::Fragment { value, .. } if value == "my-frag"))
        );
    }

    #[test]
    fn unclosed_fragment_produces_error() {
        let result = parse_str("{>my-fragment");
        assert!(
            !result.errors.is_empty(),
            "expected an error for unclosed fragment"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e.kind, ParseErrorKind::UnclosedFragment))
        );
    }

    #[test]
    fn fragment_open_without_id_produces_error() {
        // "{>" immediately followed by close "<}" has an empty RefValue, which is still a RefValue token;
        // to trigger ExpectedRefValueAfterFragmentOpen we'd need a non-RefValue after {>.
        // The most direct way is a bare "{>" at end of input (no RefValue token at all).
        let result = parse_str("{>");
        assert!(!result.errors.is_empty(), "expected an error");
        assert!(result.errors.iter().any(|e| {
            matches!(
                e.kind,
                ParseErrorKind::ExpectedRefValueAfterFragmentOpen
                    | ParseErrorKind::UnclosedFragment
            )
        }));
    }

    // ── Escaped body (double-tick) ────────────────────────────────────────

    #[test]
    fn escaped_body_produces_escaped_body_node() {
        let result = parse_str("``verbatim``");
        assert!(result.errors.is_empty(), "expected no errors");
        assert!(
            result
                .nodes
                .iter()
                .any(|n| matches!(n, Node::EscapedBody { .. }))
        );
    }

    #[test]
    fn escaped_body_source_range_covers_both_delimiters() {
        let src = "``verbatim``";
        let result = parse_str(src);
        assert!(result.errors.is_empty());
        let escaped = result
            .nodes
            .iter()
            .find(|n| matches!(n, Node::EscapedBody { .. }));
        match escaped.expect("expected EscapedBody node") {
            Node::EscapedBody { source_range } => {
                assert_eq!(source_range.start, 0);
                assert_eq!(source_range.end, 12);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn unclosed_escaped_body_produces_error() {
        let result = parse_str("``unclosed");
        assert!(!result.errors.is_empty());
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e.kind, ParseErrorKind::UnclosedEscape))
        );
    }

    // ── RefSuspect ───────────────────────────────────────────────────────

    #[test]
    fn tick_wrapped_unknown_text_produces_ref_suspect() {
        let result = parse_str("`sometext`");
        assert!(result.errors.is_empty(), "expected no errors");
        assert!(
            result
                .nodes
                .iter()
                .any(|n| matches!(n, Node::RefSuspect { .. }))
        );
    }

    #[test]
    fn ref_suspect_source_range_covers_both_ticks() {
        let src = "`suspect`";
        let result = parse_str(src);
        assert!(result.errors.is_empty());
        let node = result
            .nodes
            .iter()
            .find(|n| matches!(n, Node::RefSuspect { .. }))
            .expect("expected RefSuspect node");
        match node {
            Node::RefSuspect { source_range } => {
                assert_eq!(source_range.start, 0);
                assert_eq!(source_range.end, u32::try_from(src.len()).unwrap());
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn tick_followed_by_body_without_closing_tick_is_body_node() {
        // `text — no closing tick. Parser treats this as normal markdown inline code → Body node.
        let result = parse_str("`text");
        assert!(
            result.errors.is_empty(),
            "expected no errors for unclosed markdown inline code"
        );
        assert!(result.nodes.iter().any(|n| matches!(n, Node::Body { .. })));
    }

    #[test]
    fn lone_trailing_tick_produces_unexpected_eof_error() {
        let result = parse_str("`");
        assert!(!result.errors.is_empty());
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e.kind, ParseErrorKind::UnexpectedEOF))
        );
    }

    // ── Typed refs ───────────────────────────────────────────────────────

    #[test]
    fn skill_ref_produces_ref_node_with_skill_kind() {
        let result = parse_str("`skill::my-skill`");
        assert!(
            result.errors.is_empty(),
            "expected no errors, got: {}",
            result.errors.len()
        );
        assert_eq!(result.nodes.len(), 1);
        match &result.nodes[0] {
            Node::Ref {
                kind: RefKind::Skill,
                value,
                ..
            } => assert_eq!(value, "my-skill"),
            n => panic!("expected Ref(Skill), got {}", node_kind_name(n)),
        }
    }

    #[test]
    fn agent_ref_produces_ref_node_with_agent_kind() {
        let result = parse_str("`agent::my-agent`");
        assert!(result.errors.is_empty(), "expected no errors");
        assert_eq!(result.nodes.len(), 1);
        match &result.nodes[0] {
            Node::Ref {
                kind: RefKind::Agent,
                value,
                ..
            } => assert_eq!(value, "my-agent"),
            n => panic!("expected Ref(Agent), got {}", node_kind_name(n)),
        }
    }

    #[test]
    fn cmd_ref_produces_ref_node_with_cmd_kind() {
        let result = parse_str("`cmd::git-status`");
        assert!(result.errors.is_empty(), "expected no errors");
        match &result.nodes[0] {
            Node::Ref {
                kind: RefKind::Cmd,
                value,
                ..
            } => assert_eq!(value, "git-status"),
            n => panic!("expected Ref(Cmd), got {}", node_kind_name(n)),
        }
    }

    #[test]
    fn path_ref_produces_ref_node_with_path_kind() {
        let result = parse_str("`path::src/main.rs`");
        assert!(result.errors.is_empty(), "expected no errors");
        match &result.nodes[0] {
            Node::Ref {
                kind: RefKind::Path,
                value,
                ..
            } => assert_eq!(value, "src/main.rs"),
            n => panic!("expected Ref(Path), got {}", node_kind_name(n)),
        }
    }

    #[test]
    fn url_ref_produces_ref_node_with_url_kind() {
        let result = parse_str("`url::https://example.com`");
        assert!(result.errors.is_empty(), "expected no errors");
        match &result.nodes[0] {
            Node::Ref {
                kind: RefKind::Url,
                value,
                ..
            } => assert_eq!(value, "https://example.com"),
            n => panic!("expected Ref(Url), got {}", node_kind_name(n)),
        }
    }

    #[test]
    fn reference_ref_produces_ref_node_with_reference_kind() {
        let result = parse_str("`ref::some-ref`");
        assert!(result.errors.is_empty(), "expected no errors");
        match &result.nodes[0] {
            Node::Ref {
                kind: RefKind::Reference,
                value,
                ..
            } => assert_eq!(value, "some-ref"),
            n => panic!("expected Ref(Reference), got {}", node_kind_name(n)),
        }
    }

    #[test]
    fn ref_node_source_range_covers_both_ticks() {
        let src = "`skill::my-skill`";
        let result = parse_str(src);
        assert!(result.errors.is_empty());
        match &result.nodes[0] {
            Node::Ref { source_range, .. } => {
                assert_eq!(source_range.start, 0);
                assert_eq!(source_range.end, u32::try_from(src.len()).unwrap());
            }
            n => panic!("expected Ref, got {}", node_kind_name(n)),
        }
    }

    #[test]
    fn unclosed_ref_produces_error() {
        let result = parse_str("`skill::my-skill");
        assert!(!result.errors.is_empty());
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e.kind, ParseErrorKind::UnclosedRef))
        );
    }

    // ── Mixed document ───────────────────────────────────────────────────

    #[test]
    fn mixed_document_produces_correct_node_count() {
        // body + skill ref + body + fragment
        let result = parse_str("Use `skill::my-skill` here. {>footer<}");
        assert!(result.errors.is_empty(), "expected no errors");
        assert!(result.nodes.iter().any(|n| matches!(
            n,
            Node::Ref {
                kind: RefKind::Skill,
                ..
            }
        )));
        assert!(
            result
                .nodes
                .iter()
                .any(|n| matches!(n, Node::Fragment { .. }))
        );
        assert!(result.nodes.iter().any(|n| matches!(n, Node::Body { .. })));
    }

    #[test]
    fn multiple_refs_of_different_kinds() {
        let result = parse_str("`skill::a` `agent::b`");
        assert!(result.errors.is_empty());
        assert!(result.nodes.iter().any(|n| matches!(
            n,
            Node::Ref {
                kind: RefKind::Skill,
                ..
            }
        )));
        assert!(result.nodes.iter().any(|n| matches!(
            n,
            Node::Ref {
                kind: RefKind::Agent,
                ..
            }
        )));
    }

    #[test]
    fn var_ref_produces_ref_node_with_var_kind() {
        let result = parse_str("`var::MY_VAR`");
        assert!(result.errors.is_empty(), "expected no errors");
        assert_eq!(result.nodes.len(), 1);
        match &result.nodes[0] {
            Node::Ref {
                kind: RefKind::Var,
                value,
                ..
            } => assert_eq!(value, "MY_VAR"),
            n => panic!("expected Ref(Var), got {}", node_kind_name(n)),
        }
    }

    #[test]
    fn env_ref_produces_ref_node_with_env_kind() {
        let result = parse_str("`env::DATABASE_URL`");
        assert!(result.errors.is_empty(), "expected no errors");
        assert_eq!(result.nodes.len(), 1);
        match &result.nodes[0] {
            Node::Ref {
                kind: RefKind::Env,
                value,
                ..
            } => assert_eq!(value, "DATABASE_URL"),
            n => panic!("expected Ref(Env), got {}", node_kind_name(n)),
        }
    }
}
