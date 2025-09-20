use std::{borrow::Cow, collections::HashMap, iter::Peekable};

use arcstr::{ArcStr, Substr};
use smallvec::SmallVec;

/// A very barebones tokenizer for ninja build files
#[derive(Debug, PartialEq, Eq, Clone, Copy, logos::Logos)]
#[logos(skip(r"#.*(?:\n)"))] // Skip comments
#[logos(skip(r"\$\r?\n"))] // Skip line continuations
#[logos(extras = (usize,usize))]
#[logos(error(LexError, LexError::from_lexer))]
enum Token<'s> {
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
    fn can_start_word(&self) -> bool {
        matches!(
            self,
            Token::Word(_) | Token::Escaped(_) | Token::Variable(_) | Token::BracedVariable(_)
        )
    }
}

struct Lexer<'s> {
    inner: logos::Lexer<'s, Token<'s>>,
    peeked: Option<Token<'s>>,
    peeked_pos: Option<(usize, usize)>,
}

impl<'s> Lexer<'s> {
    fn new(s: &'s <Token<'s> as logos::Logos<'s>>::Source) -> Self {
        Self {
            inner: logos::Lexer::new(s),
            peeked: None,
            peeked_pos: None,
        }
    }

    fn peek(&mut self) -> Result<Option<Token<'s>>, Error> {
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

    fn peeked_pos(&self) -> Option<(usize, usize)> {
        self.peeked_pos
    }

    fn peek_is(&mut self, expected: Token<'s>) -> Result<bool, Error> {
        Ok(self.peek()? == Some(expected))
    }

    fn eat_if(&mut self, expected: Token<'s>) -> Result<bool, Error> {
        if self.peek_is(expected)? {
            let _ = self.next();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn expect(&mut self, expected: Token<'s>) -> Result<(), Error> {
        let next = self.next().ok_or(Error::UnexpectedEof)??;

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

    fn unexpected<T>(&mut self) -> Result<T, Error> {
        let next = self.next().ok_or(Error::UnexpectedEof)??;
        Err(Error::UnexpectedToken(
            format!("{next:?}"),
            self.inner.extras.0,
            self.inner.extras.1,
        ))
    }

    fn skip_spaces(&mut self) {
        while let Some(Token::Spaces(_)) = self.peek().ok().flatten() {
            self.next();
        }
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
enum LexError {
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
            LexError::Unknown => Self::UnknownLexError,
        }
    }
}

#[derive(Clone, PartialEq, Debug, thiserror::Error)]
pub enum Error {
    #[error("Unrecognized token at {0}:{1}")]
    UnrecognizedToken(usize, usize),

    #[error("Unknown variable {0}")]
    UnknownVariable(String),

    #[error("Missing required rule variable {0}")]
    MissingRuleVariable(String),

    #[error("Invalid deps type {0} (expected: gcc|msvc)")]
    InvalidDepsType(String),

    #[error("Unexpected token {0:?} at {1}:{2}")]
    UnexpectedToken(String, usize, usize),

    #[error("Unexpected end of file")]
    UnexpectedEof,

    #[error("An unknown error occurred during lexing")]
    UnknownLexError,
}

/// Dependency processing type for the `deps` rule variable
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum DepsType {
    Gcc,
    Msvc,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Segment<'s> {
    Regular(Cow<'s, str>),
    Var(&'s str),
}

/// A string that may contain `$`-replacements, which is handled lazily
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Expandable<'s>(pub SmallVec<[Segment<'s>; 1]>);

impl<'s> Expandable<'s> {
    fn expand(&self, scope: &ExpansionScope<'_, 's>) -> Cow<'s, str> {
        let mut res = Cow::Borrowed("");
        for seg in &self.0 {
            match seg {
                Segment::Regular(s) => {
                    if res.is_empty() {
                        res = s.clone();
                    } else {
                        res.to_mut().push_str(s);
                    }
                }
                Segment::Var(name) => {
                    let value = scope.get(name);
                    match value {
                        None => {}
                        Some(Cow::Borrowed(v)) => {
                            if res.is_empty() {
                                res = Cow::Borrowed(v);
                            } else {
                                res.to_mut().push_str(v);
                            }
                        }
                        Some(Cow::Owned(v)) => {
                            if res.is_empty() {
                                res = Cow::Owned(v);
                            } else {
                                res.to_mut().push_str(&v);
                            }
                        }
                    }
                }
            }
        }
        res
    }
}

type Scope<'s> = HashMap<&'s str, Cow<'s, str>>;
type RuleScope<'s> = HashMap<&'s str, Expandable<'s>>;

/// Corresponding to a ninja `rule` block
#[derive(Debug, Clone)]
struct Rule<'s> {
    vars: RuleScope<'s>,
}

/*
    Ninja documentation:
    https://ninja-build.org/manual.html#ref_scope

    Variable declarations indented in a build block are scoped to the build
    block. The full lookup order for a variable expanded in a build block (or
    the rule it uses) is:

    - Special built-in variables ($in, $out).
    - Build-level variables from the build block.
    - Rule-level variables from the rule block (i.e. $command). (Note from
        the above discussion on expansion that these are expanded "late",
        and may make use of in-scope bindings like $in.)
    - File-level variables from the file that the build line was in.
    - Variables from the file that included that file using the subninja keyword.
*/
struct ExpansionScope<'r, 's> {
    in_files: &'r [Cow<'s, str>],
    out_files: &'r [Cow<'s, str>],
    global_scope: &'r Scope<'s>,
    build_scope: &'r Scope<'s>,
    rule: &'r Rule<'s>,
}

impl<'r, 's> ExpansionScope<'r, 's> {
    fn get(&self, variable: &str) -> Option<Cow<'s, str>> {
        // 1. special built-in variables
        if variable == "in" {
            return Some(
                shlex::try_join(self.in_files.iter().map(|s| s.as_ref()))
                    .unwrap()
                    .into(),
            );
        }
        if variable == "out" {
            return Some(
                shlex::try_join(self.out_files.iter().map(|s| s.as_ref()))
                    .unwrap()
                    .into(),
            );
        }

        // 2. Build-level variables
        if let Some(v) = self.build_scope.get(variable) {
            return Some(v.clone());
        }

        // 3. Rule-level variables (may expand recursively)
        if let Some(v) = self.rule.vars.get(variable) {
            return Some(v.expand(self));
        }

        // 4. Global scope
        if let Some(v) = self.global_scope.get(variable) {
            return Some(v.clone());
        }

        // Not found
        None
    }
}

/// A build statement
struct Build<'s> {
    inputs: Vec<Cow<'s, str>>,
    implicit_inputs: Vec<Cow<'s, str>>,
    order_only_inputs: Vec<Cow<'s, str>>,
    outputs: Vec<Cow<'s, str>>,

    /// The command line to run (required)
    command: Cow<'s, str>,
    /// Path to an optional Makefile that contains extra implicit dependencies
    depfile: Option<Cow<'s, str>>,
    /// Special dependency processing type
    deps: Option<DepsType>,
    /// String which should be stripped from msvc's /showIncludes output
    msvc_deps_prefix: Option<Cow<'s, str>>,
    /// A short description of the command
    description: Option<Cow<'s, str>>,
    /// Dynamically discovered dependency information file
    dyndep: Option<Cow<'s, str>>,
    /// Specifies that this rule is used to re-invoke the generator program
    generator: bool,
    /// Causes Ninja to re-stat the command's outputs after execution
    restat: bool,
    /// Response file path
    rspfile: Option<Cow<'s, str>>,
    /// Response file content
    rspfile_content: Option<Cow<'s, str>>,
}

struct NinjaFile<'s> {
    global_scope: Scope<'s>,
    rules: HashMap<&'s str, Rule<'s>>,
    builds: Vec<Build<'s>>,
}

pub fn parse<'s>(s: &'s str) -> Result<NinjaFile<'s>, Error> {
    use Token::*;
    let mut lexer = Lexer::new(s);
    let mut global_scope = Scope::new();
    let mut rules = HashMap::new();
    let mut builds = Vec::new();

    loop {
        let Some(next) = lexer.peek()? else {
            break;
        };

        match next {
            Word("build") => {
                let build = parse_build(&mut lexer, &global_scope, &rules)?;
                builds.push(build);
            }
            Word("rule") => {
                let (name, rule) = parse_rule(&mut lexer)?;
                if rules.insert(name, rule).is_some() {
                    let peek_pos = lexer.peeked_pos().unwrap();
                    return Err(Error::UnexpectedToken(
                        format!("redefinition of rule {name}"),
                        peek_pos.0,
                        peek_pos.1,
                    ));
                }
            }
            Word(_) => {
                let (k, v) = parse_variable_assignment(&mut lexer, &[&global_scope])?;
                global_scope.insert(k, v);
                // TODO: check top-level vars `builddir` and `ninja_required_version`
            }
            _ => lexer.unexpected()?,
        }
    }

    Ok(NinjaFile {
        global_scope,
        rules,
        builds,
    })
}

fn parse_rule<'s>(lexer: &mut Lexer<'s>) -> Result<(&'s str, Rule<'s>), Error> {
    // rule
    let _ = lexer.next().ok_or(Error::UnexpectedEof)??;
    lexer.skip_spaces();

    // <name>
    let name_tok = lexer.next().ok_or(Error::UnexpectedEof)??;
    let Token::Word(name) = name_tok else {
        lexer.unexpected()?
    };
    lexer.skip_spaces();
    lexer.expect(Token::LineFeed)?;

    let mut scope = HashMap::new();

    // Loop while we have indented string
    while matches!(lexer.peek()?, Some(Token::Spaces(_))) {
        lexer.skip_spaces();
        let (k, v) = parse_variable_assignment_no_expand(lexer)?;
        scope.insert(k, v);
        lexer.eat_if(Token::LineFeed)?;
    }

    let rule = Rule { vars: scope };
    Ok((name, rule))
}

fn parse_build<'s>(
    lexer: &mut Lexer<'s>,
    global_scope: &Scope<'s>,
    rules: &HashMap<&'s str, Rule<'s>>,
) -> Result<Build<'s>, Error> {
    let mut scope = Scope::new();

    // build
    let _ = lexer.next().ok_or(Error::UnexpectedEof)??;
    lexer.skip_spaces();

    // <outputs>
    let mut outputs = Vec::new();
    loop {
        match lexer.peek()?.ok_or(Error::UnexpectedEof)? {
            tok if tok.can_start_word() => {
                let output = parse_expand_word(lexer, &[global_scope], true)?;
                outputs.push(output);
                lexer.skip_spaces();
            }
            Token::Colon => break,
            _ => lexer.unexpected()?,
        }
    }

    lexer.expect(Token::Colon)?;
    lexer.skip_spaces();

    // <rule>
    let rule_tok = lexer.next().ok_or(Error::UnexpectedEof)??;
    let Token::Word(rule_name) = rule_tok else {
        lexer.unexpected()?
    };
    let rule = rules
        .get(rule_name)
        .ok_or(Error::UnknownVariable(rule_name.to_string()))?;
    lexer.skip_spaces();

    // <inputs> | <implicit_inputs> || <order_only_inputs>

    let mut inputs = Vec::new();
    let mut implicit_inputs = Vec::new();
    let mut order_only_inputs = Vec::new();
    while lexer.peek()?.is_some_and(|t| t.can_start_word()) {
        let input = parse_expand_word(lexer, &[global_scope], true)?;
        inputs.push(input);
        lexer.skip_spaces();
    }
    if lexer.peek()? == Some(Token::Pipe) {
        let _ = lexer.next(); // consume the pipe
        lexer.skip_spaces();
        while lexer.peek()?.is_some_and(|t| t.can_start_word()) {
            let input = parse_expand_word(lexer, &[global_scope], true)?;
            implicit_inputs.push(input);
            lexer.skip_spaces();
        }
    }
    if lexer.peek()? == Some(Token::TwoPipe) {
        let _ = lexer.next(); // consume the two-pipe
        lexer.skip_spaces();
        while lexer.peek()?.is_some_and(|t| t.can_start_word()) {
            let input = parse_expand_word(lexer, &[global_scope], true)?;
            order_only_inputs.push(input);
            lexer.skip_spaces();
        }
    }

    // LF, prepare to parse indented variables
    if lexer.peek()? == Some(Token::LineFeed) {
        let _ = lexer.next();
    } else {
        lexer.unexpected()?
    }

    // Vars
    while matches!(lexer.peek()?, Some(Token::Spaces(_))) {
        let _ = lexer.next();
        let (k, v) = parse_variable_assignment_no_expand(lexer)?;
        let exp_scope = ExpansionScope {
            in_files: &inputs,
            out_files: &outputs,
            global_scope,
            build_scope: &scope,
            rule,
        };
        let v = v.expand(&exp_scope);
        scope.insert(k, v);
        lexer.eat_if(Token::LineFeed)?;
    }

    let exp_scope = ExpansionScope {
        in_files: &inputs,
        out_files: &outputs,
        global_scope,
        build_scope: &scope,
        rule,
    };
    expand_build(rule, &exp_scope, &implicit_inputs, &order_only_inputs)
}

fn expand_build<'s>(
    _rule: &Rule<'s>,
    exp_scope: &ExpansionScope<'_, 's>,
    implicit_input: &[Cow<'s, str>],
    order_only_input: &[Cow<'s, str>],
) -> Result<Build<'s>, Error> {
    // Required: command
    let Some(command) = exp_scope.get("command") else {
        return Err(Error::MissingRuleVariable("command".to_string()));
    };

    // Optional simple string fields
    let depfile = exp_scope.get("depfile");
    let msvc_deps_prefix = exp_scope.get("msvc_deps_prefix");
    let description = exp_scope.get("description");
    let dyndep = exp_scope.get("dyndep");
    let rspfile = exp_scope.get("rspfile");
    let rspfile_content = exp_scope.get("rspfile_content");

    // Optional enum field: deps
    let deps = match exp_scope.get("deps") {
        Some(v) => match v.as_ref() {
            "gcc" => Some(DepsType::Gcc),
            "msvc" => Some(DepsType::Msvc),
            other => return Err(Error::InvalidDepsType(other.to_string())),
        },
        None => None,
    };

    // Boolean flags: generator and restat (truthy if not "" and not "0")
    let is_truthy = |v: &str| !v.is_empty() && v != "0";
    let generator = exp_scope
        .get("generator")
        .map(|v| is_truthy(v.as_ref()))
        .unwrap_or(false);
    let restat = exp_scope
        .get("restat")
        .map(|v| is_truthy(v.as_ref()))
        .unwrap_or(false);

    // Assemble Build
    Ok(Build {
        inputs: exp_scope.in_files.to_vec(),
        implicit_inputs: implicit_input.to_vec(),
        order_only_inputs: order_only_input.to_vec(),
        outputs: exp_scope.out_files.to_vec(),

        command,
        depfile,
        deps,
        msvc_deps_prefix,
        description,
        dyndep,
        generator,
        restat,
        rspfile,
        rspfile_content,
    })
}

/// Parse a variable assignment, and expand immediately. Does not support
fn parse_variable_assignment<'s>(
    lexer: &mut Lexer<'s>,
    scopes: &[&Scope<'s>],
) -> Result<(&'s str, Cow<'s, str>), Error> {
    let name = lexer.next().ok_or(Error::UnexpectedEof)??;
    let Token::Word(name) = name else {
        return Err(Error::UnexpectedToken(
            format!("{name:?}"),
            lexer.inner.extras.0,
            lexer.inner.extras.1,
        ));
    };

    lexer.skip_spaces();
    lexer.expect(Token::Equal)?;
    lexer.skip_spaces();

    let value = parse_expand_word(lexer, scopes, false)?;

    lexer.skip_spaces();

    Ok((name, value))
}

fn parse_variable_assignment_no_expand<'s>(
    lexer: &mut Lexer<'s>,
) -> Result<(&'s str, Expandable<'s>), Error> {
    let name = lexer.next().ok_or(Error::UnexpectedEof)??;
    let Token::Word(name) = name else {
        return Err(Error::UnexpectedToken(
            format!("{name:?}"),
            lexer.inner.extras.0,
            lexer.inner.extras.1,
        ));
    };

    lexer.skip_spaces();
    lexer.expect(Token::Equal)?;
    lexer.skip_spaces();

    let value = parse_noexpand_word(lexer)?;

    lexer.skip_spaces();

    Ok((name, value))
}

/// Parse and expand a word. Returns if the next token is not a word token,
/// and the corresponding token will be left in the [`Peekable::next()`] on return.
fn parse_expand_word<'s>(
    lexer: &mut Lexer<'s>,
    scope: &[&Scope<'s>],
    no_space: bool,
) -> Result<Cow<'s, str>, Error> {
    let mut res: std::borrow::Cow<'s, str> = std::borrow::Cow::Borrowed("");
    loop {
        match lexer.peek()?.ok_or(Error::UnexpectedEof)? {
            Token::Word(w) => {
                if res.is_empty() {
                    res = Cow::Borrowed(w);
                } else {
                    res.to_mut().push_str(w);
                }
            }
            Token::Spaces(w) if !no_space => {
                if res.is_empty() {
                    res = Cow::Borrowed(w);
                } else {
                    res.to_mut().push_str(w);
                }
            }
            Token::Escaped(c) => {
                res.to_mut().push(c);
            }
            Token::Variable(name) | Token::BracedVariable(name) => {
                let var_value = scope
                    .iter()
                    .find_map(|x| x.get(name))
                    .ok_or(Error::UnknownVariable(name.to_string()))?;
                res.to_mut().push_str(var_value);
            }
            _ => break,
        }
        lexer.next(); // consume the token just processed
    }
    Ok(res)
}

/// Parse word but don't expand variables. The rest of the semantics the same
/// as [`parse_expand_word`].
fn parse_noexpand_word<'s>(lexer: &mut Lexer<'s>) -> Result<Expandable<'s>, Error> {
    let mut res = SmallVec::new();
    let mut acc: Option<Cow<'_, str>> = None;
    loop {
        match lexer.peek()?.ok_or(Error::UnexpectedEof)? {
            Token::Word(w) | Token::Spaces(w) => {
                if let Some(acc) = acc.as_mut() {
                    acc.to_mut().push_str(w);
                } else {
                    acc = Some(Cow::Borrowed(w));
                }
            }
            Token::Escaped(c) => {
                if let Some(acc) = acc.as_mut() {
                    acc.to_mut().push(c);
                } else {
                    acc = Some(Cow::Owned(c.to_string()));
                }
            }
            Token::Variable(name) | Token::BracedVariable(name) => {
                if let Some(acc) = acc.take() {
                    res.push(Segment::Regular(acc));
                }
                res.push(Segment::Var(name));
            }
            _ => break,
        }
        lexer.next(); // consume the token just processed
    }
    if let Some(acc) = acc {
        res.push(Segment::Regular(acc));
    }
    Ok(Expandable(res))
}
