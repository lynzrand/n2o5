use super::model::{Error, Pos};

/// A very barebones tokenizer for ninja build files
#[derive(Debug, PartialEq, Eq, Clone, Copy, logos::Logos)]
#[logos(error(LexError, LexError::from_lexer))]
pub(super) enum Token<'s> {
    /// A line feed followed by indentation spaces of the next line
    #[regex(r"\r?\n[ \t]+")]
    IndentedLineFeed,

    /// A line feed without indentation
    #[regex(r"\r?\n")]
    LineFeed,

    /// Spaces
    #[regex(r"[ \t]+", |lex| lex.slice())]
    Spaces(&'s str),

    // Ninja word elements -- these should be concatenated to make a word
    /// `$` followed by one of ` $:` ==>> An escaped character
    #[regex(r"\$[\$ \t:]", |lex| (lex.slice().chars().nth(1).unwrap()))]
    Escaped(char),

    /// `$ident` ==>> A variable replacement
    #[regex(r"\$[a-zA-Z_][a-zA-Z0-9_]*", |lex| (&lex.slice()[1..]))]
    Variable(&'s str),

    /// `${ident}` ==>> Also a variable replacement
    #[regex(r"\$\{[^\}\s\$]*\}", |lex| (&lex.slice()[2..lex.slice().len()-1]))]
    BracedVariable(&'s str),

    // Words and punctuations
    /// A colon for separating build outputs from inputs
    #[token(":")]
    Colon,

    /// A single pipe for separating normal inputs from implicit inputs
    #[token("|")]
    Pipe,

    /// A double pipe for separating implicit inputs from order-only inputs
    #[token("||")]
    TwoPipe,

    /// A equal sign for variable assignment
    #[token("=")]
    Equal,

    /// A word segment without escaping
    #[regex(r"[^=\s\$:|]+")]
    Word(&'s str),

    // Skipped tokens, kept for location tracking. They are skipped during
    // internal `next`.
    #[regex(r"#.*(?:\n)")]
    Comment,
    #[regex(r"\$\r?\n")]
    LineContinuation,
}

impl<'s> Token<'s> {
    pub(super) fn can_start_word(&self) -> bool {
        matches!(
            self,
            Token::Word(_) | Token::Escaped(_) | Token::Variable(_) | Token::BracedVariable(_)
        )
    }

    fn is_skipped(&self) -> bool {
        matches!(self, Token::Comment | Token::LineContinuation)
    }
}

pub(super) struct Lexer<'s> {
    inner: logos::Lexer<'s, Token<'s>>,
    /// The starting position of the token just returned by `next_inner`
    pos: Pos,
    /// The ending position of the token just returned by `next_inner`
    pos_end: Pos,

    // Manual peeking
    peeked: Option<Token<'s>>,
    peeked_pos: Option<Pos>,
}

fn line_col_offset(t: &str) -> (usize, usize) {
    let mut lines = 0;
    let mut last_line = "";
    for l in t.split('\n') {
        lines += 1;
        last_line = l;
    }
    lines -= 1;
    (lines, last_line.len())
}

impl<'s> Lexer<'s> {
    pub(super) fn new(s: &'s <Token<'s> as logos::Logos<'s>>::Source) -> Self {
        let inner = logos::Lexer::new(s);
        // Initialize extras with starting cursor position
        Self {
            inner,
            pos: Pos::new(0, 0),
            pos_end: Pos::new(0, 0),
            peeked: None,
            peeked_pos: None,
        }
    }

    fn next_inner(&mut self) -> Option<Result<Token<'s>, Error>> {
        loop {
            let next = self.inner.next()?;
            self.pos = self.pos_end;

            // Move position to the end of the matched slice
            let slice = self.inner.slice();
            let (line_offset, col_offset) = line_col_offset(slice);
            let mut new_pos_end = self.pos_end;
            if line_offset > 0 {
                new_pos_end.line += line_offset;
                new_pos_end.column = col_offset;
            } else {
                new_pos_end.column += col_offset;
            }
            self.pos_end = new_pos_end;

            // Return
            match next {
                Ok(tok) if tok.is_skipped() => continue,
                _ => {
                    return Some(next.map_err(|e| match e {
                        LexError::Unknown => Error::UnknownLexing(self.pos),
                        LexError::UnrecognizedToken => Error::UnrecognizedToken(self.pos),
                    }));
                }
            }
        }
    }

    pub(super) fn peek(&mut self) -> Result<Option<Token<'s>>, Error> {
        if let Some(tok) = self.peeked {
            Ok(Some(tok))
        } else {
            let next = self.next_inner().transpose()?;
            if next.is_some() {
                self.peeked_pos = Some(self.pos);
            } else {
                self.peeked_pos = None;
            }
            self.peeked = next;
            Ok(next)
        }
    }

    pub(super) fn peeked_pos(&self) -> Option<Pos> {
        self.peeked_pos
    }

    pub(super) fn cursor_pos(&self) -> Pos {
        self.pos
    }

    pub(super) fn expect(&mut self, expected: Token<'s>) -> Result<(), Error> {
        let next = self.next().ok_or(Error::UnexpectedEof(format!(
            "expecting token {expected:?}"
        )))??;

        if next == expected {
            Ok(())
        } else {
            Err(Error::UnexpectedToken(
                format!("{next:?}, expected {expected:?}"),
                self.pos,
            ))
        }
    }

    pub(super) fn unexpected<T>(&mut self, desc: &str) -> Result<T, Error> {
        let next = self.next().ok_or(Error::UnexpectedEof(
            "expecting some token, got end of file".into(),
        ))??;
        Err(Error::UnexpectedToken(
            format!("{next:?}, {}", desc),
            self.pos,
        ))
    }

    pub(super) fn skip_spaces(&mut self) {
        while let Some(Token::Spaces(_)) = self.peek().ok().flatten() {
            self.next();
        }
    }

    /// Consume one or more consecutive line breaks and return whether
    /// the next logical line is indented.
    ///
    /// Semantics:
    /// - Consumes any sequence of:
    ///     * Token::IndentedLineFeed (newline + indentation)
    ///     * Token::LineFeed (bare newline)
    /// - Returns:
    ///     * true  if the last consumed item was IndentedLineFeed
    ///     * false if no newline was consumed or the last consumed was bare LineFeed
    pub(super) fn eat_newlines(&mut self) -> bool {
        let mut indented = false;
        loop {
            match self.peek().ok().flatten() {
                Some(Token::IndentedLineFeed) => {
                    let _ = self.next(); // consume newline + indentation
                    indented = true;
                }
                Some(Token::LineFeed) => {
                    let _ = self.next(); // consume bare newline
                    indented = false;
                }
                None => return false, // EOF is always non-indented
                _ => break,
            }
        }
        indented
    }
}

impl<'s> Iterator for Lexer<'s> {
    type Item = Result<Token<'s>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(tok) = self.peeked.take() {
            let _pos = self.peeked_pos.take();
            Some(Ok(tok))
        } else {
            self.next_inner()
        }
    }
}

/// A lightweight error type for lexing errors
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(super) enum LexError {
    #[default]
    Unknown,
    UnrecognizedToken,
}

impl LexError {
    fn from_lexer<'a>(_lexer: &mut logos::Lexer<'a, Token<'a>>) -> Self {
        Self::UnrecognizedToken
    }
}
