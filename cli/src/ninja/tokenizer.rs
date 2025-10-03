use super::model::Error;

/// A very barebones tokenizer for ninja build files
#[derive(Debug, PartialEq, Eq, Clone, Copy, logos::Logos)]
#[logos(skip(r"#.*(?:\n)"))] // Skip comments
#[logos(skip(r"\$\r?\n"))] // Skip line continuations
#[logos(extras = (usize,usize))]
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
}

impl<'s> Token<'s> {
    pub(super) fn can_start_word(&self) -> bool {
        matches!(
            self,
            Token::Word(_) | Token::Escaped(_) | Token::Variable(_) | Token::BracedVariable(_)
        )
    }
}

pub(super) struct Lexer<'s> {
    inner: logos::Lexer<'s, Token<'s>>,
    peeked: Option<Token<'s>>,
    peeked_pos: Option<(usize, usize)>,
}

impl<'s> Lexer<'s> {
    pub(super) fn new(s: &'s <Token<'s> as logos::Logos<'s>>::Source) -> Self {
        Self {
            inner: logos::Lexer::new(s),
            peeked: None,
            peeked_pos: None,
        }
    }

    pub(super) fn peek(&mut self) -> Result<Option<Token<'s>>, Error> {
        if let Some(tok) = self.peeked {
            Ok(Some(tok))
        } else {
            let next = self.inner.next().transpose()?;
            if next.is_some() {
                self.peeked_pos = Some(self.inner.extras);
            } else {
                self.peeked_pos = None;
            }
            self.peeked = next;
            Ok(next)
        }
    }

    pub(super) fn peeked_pos(&self) -> Option<(usize, usize)> {
        self.peeked_pos
    }

    pub(super) fn cursor_pos(&self) -> (usize, usize) {
        self.inner.extras
    }

    pub(super) fn expect(&mut self, expected: Token<'s>) -> Result<(), Error> {
        let next = self.next().ok_or(Error::UnexpectedEof(format!(
            "expecting token {expected:?}"
        )))??;

        if next == expected {
            Ok(())
        } else {
            Err(Error::UnexpectedToken(
                format!("{next:?}"),
                self.inner.extras.0,
                self.inner.extras.1,
            ))
        }
    }

    pub(super) fn unexpected<T>(&mut self) -> Result<T, Error> {
        let next = self.next().ok_or(Error::UnexpectedEof(
            "expecting some token, got end of file".into(),
        ))??;
        Err(Error::UnexpectedToken(
            format!("{next:?}"),
            self.inner.extras.0,
            self.inner.extras.1,
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
            self.inner.next().map(|x| x.map_err(Into::into))
        }
    }
}

/// A lightweight error type for lexing errors
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(super) enum LexError {
    #[default]
    Unknown,
    UnrecognizedToken(usize, usize),
}

impl LexError {
    fn from_lexer<'a>(lexer: &mut logos::Lexer<'a, Token<'a>>) -> Self {
        Self::UnrecognizedToken(lexer.extras.0, lexer.extras.1)
    }
}

impl From<LexError> for Error {
    fn from(err: LexError) -> Self {
        match err {
            LexError::UnrecognizedToken(line, col) => Self::UnrecognizedToken(line, col),
            LexError::Unknown => Self::UnknownLexing,
        }
    }
}
