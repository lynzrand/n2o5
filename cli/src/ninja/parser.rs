use smallvec::SmallVec;
use std::path::Path;
use std::{borrow::Cow, sync::Arc};

use crate::ninja::model::ParseBuildResult;

use super::model::{
    Build, DepsType, Error, Expandable, ExpansionScope, NinjaFile, PhonyBuild, Rule, RuleScope,
    Scope,
};
use super::tokenizer::{Lexer, Token};

pub struct ParseSource {
    in_memory: bool,
    sources: elsa::FrozenVec<String>,
}

impl ParseSource {
    pub fn new(file: impl AsRef<Path>) -> Self {
        let file = file.as_ref();
        let content = std::fs::read_to_string(file).expect("failed to read ninja file");
        let sources = elsa::FrozenVec::new();
        sources.push(content);
        Self {
            in_memory: false,
            sources,
        }
    }

    #[allow(unused)] // mainly for testing
    pub fn new_in_memory(content: impl Into<String>) -> Self {
        let sources = elsa::FrozenVec::new();
        sources.push(content.into());
        Self {
            in_memory: true,
            sources,
        }
    }

    pub fn main_file(&self) -> &str {
        &self.sources[0]
    }

    pub fn add_file(&self, file: impl AsRef<Path>) -> &str {
        if self.in_memory {
            panic!("cannot include files in in-memory ParseSource");
        }
        let file = file.as_ref();
        let content = std::fs::read_to_string(file).unwrap_or_else(|e| {
            panic!("failed to read included ninja file {}: {e}", file.display())
        });
        self.sources.push_get(content)
    }
}

pub fn parse<'s>(source: &'s ParseSource, s: &'s str) -> Result<NinjaFile<'s>, Error> {
    let mut file = NinjaFile {
        global_scope: Default::default(),
        rules: Default::default(),
        builds: Default::default(),
        phony: Default::default(),
    };
    parse_inner(source, s, &mut file)?;
    Ok(file)
}

fn parse_inner<'s>(
    source: &'s ParseSource,
    s: &'s str,
    file: &mut NinjaFile<'s>,
) -> Result<(), Error> {
    use Token::*;
    let mut lexer = Lexer::new(s);

    loop {
        let indented = lexer.eat_newlines();
        if indented {
            return Err(Error::UnexpectedIndentation);
        }
        if matches!(lexer.peek()?, Some(Spaces(_))) {
            return Err(Error::UnexpectedIndentation);
        }
        let Some(next) = lexer.peek()? else {
            break;
        };

        match next {
            Word("build") => {
                let build = parse_build(&mut lexer, file)?;
                match build {
                    ParseBuildResult::Build(build) => file.builds.push(build),
                    ParseBuildResult::Phony(phony_build) => {
                        let ph = Arc::new(phony_build);
                        for t in &ph.targets {
                            file.phony.insert(t.clone(), Arc::clone(&ph));
                        }
                    }
                }
            }
            Word("rule") => {
                let (name, rule) = parse_rule(&mut lexer)?;
                if file.rules.insert(name, rule).is_some() {
                    let peek_pos = lexer.peeked_pos().unwrap();
                    return Err(Error::UnexpectedToken(
                        format!("redefinition of rule {name}"),
                        peek_pos.0,
                        peek_pos.1,
                    ));
                }
            }
            Word("include") => {
                // include <filename>
                let _ = lexer.next();
                lexer.skip_spaces();
                let filename = parse_expand_word(&mut lexer, &[&file.global_scope], true)?;
                lexer.skip_spaces();
                let file_contents = source.add_file(&*filename);
                parse_inner(source, file_contents, file)?;
            }
            Word("subninja") => {
                todo!("subninja directive not implemented")
            }
            Word("pool") => {
                todo!("pool directive not implemented")
            }
            Word(_) => {
                let (k, v) = parse_variable_assignment(&mut lexer, &[&file.global_scope])?;
                file.global_scope.insert(k, v);
                // TODO: check top-level vars `builddir` and `ninja_required_version`
            }
            _ => lexer.unexpected()?,
        }
    }

    Ok(())
}

fn parse_rule<'s>(lexer: &mut Lexer<'s>) -> Result<(&'s str, Rule<'s>), Error> {
    // rule
    let _ = lexer
        .next()
        .ok_or(Error::UnexpectedEof("parsing rule".into()))??;
    lexer.skip_spaces();

    // <name>
    let name_tok = lexer
        .next()
        .ok_or(Error::UnexpectedEof("parsing name of rule".into()))??;
    let Token::Word(name) = name_tok else {
        lexer.unexpected()?
    };
    lexer.skip_spaces();
    // Expect at least one newline, then consume all consecutive newlines and detect indentation
    match lexer.peek()? {
        Some(Token::LineFeed) | Some(Token::IndentedLineFeed) => {}
        _ => lexer.unexpected()?,
    }
    let mut indented = lexer.eat_newlines();

    let mut scope: RuleScope<'s> = Default::default();

    // Loop while the next logical line is indented
    while indented {
        let (k, v) = parse_variable_assignment_no_expand(lexer)?;
        scope.insert(k, v);
        indented = lexer.eat_newlines();
    }

    let rule = Rule { vars: scope };
    Ok((name, rule))
}

fn parse_build<'s>(
    lexer: &mut Lexer<'s>,
    file: &NinjaFile<'s>,
) -> Result<ParseBuildResult<'s>, Error> {
    let mut scope = Scope::new();

    // build
    let _ = lexer
        .next()
        .ok_or(Error::UnexpectedEof("parsing build".into()))??;
    lexer.skip_spaces();

    let io_expand_scope = &[&file.global_scope];

    // <outputs>
    let mut outputs = Vec::new();
    loop {
        match lexer.peek()?.ok_or(Error::UnexpectedEof(
            "parsing the outputs of a build".into(),
        ))? {
            tok if tok.can_start_word() => {
                let output = parse_expand_word(lexer, io_expand_scope, true)?;
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
    let rule_tok = lexer
        .next()
        .ok_or(Error::UnexpectedEof("parsing the name of a rule".into()))??;
    let Token::Word(rule_name) = rule_tok else {
        lexer.unexpected()?
    };
    let rule = if rule_name == "phony" {
        None
    } else {
        let rule = file
            .rules
            .get(rule_name)
            .ok_or(Error::UnknownVariable(rule_name.to_string()))?;
        Some(rule)
    };
    lexer.skip_spaces();

    // <inputs> | <implicit_inputs> || <order_only_inputs>

    let mut inputs = Vec::new();
    let mut implicit_inputs = Vec::new();
    let mut order_only_inputs = Vec::new();
    while lexer.peek()?.is_some_and(|t| t.can_start_word()) {
        let input = parse_expand_word(lexer, io_expand_scope, true)?;
        inputs.push(input);
        lexer.skip_spaces();
    }
    if lexer.peek()? == Some(Token::Pipe) {
        let _ = lexer.next(); // consume the pipe
        lexer.skip_spaces();
        while lexer.peek()?.is_some_and(|t| t.can_start_word()) {
            let input = parse_expand_word(lexer, io_expand_scope, true)?;
            implicit_inputs.push(input);
            lexer.skip_spaces();
        }
    }
    if lexer.peek()? == Some(Token::TwoPipe) {
        let _ = lexer.next(); // consume the two-pipe
        lexer.skip_spaces();
        while lexer.peek()?.is_some_and(|t| t.can_start_word()) {
            let input = parse_expand_word(lexer, io_expand_scope, true)?;
            order_only_inputs.push(input);
            lexer.skip_spaces();
        }
    }

    // LF(s), prepare to parse indented variables
    match lexer.peek()? {
        Some(Token::LineFeed) | Some(Token::IndentedLineFeed) => {}
        _ => lexer.unexpected()?,
    }
    let mut indented = lexer.eat_newlines();

    // Vars
    while indented {
        let (k, v) = parse_variable_assignment_no_expand(lexer)?;
        let exp_scope = ExpansionScope {
            in_files: &inputs,
            out_files: &outputs,
            file,
            build_scope: &scope,
            rule,
        };
        let v = v.expand(&exp_scope);
        scope.insert(k, v);
        indented = lexer.eat_newlines();
    }

    let exp_scope = ExpansionScope {
        in_files: &inputs,
        out_files: &outputs,
        file,
        build_scope: &scope,
        rule,
    };

    if rule_name == "phony" {
        let phony = expand_phony(&exp_scope, &order_only_inputs)?;
        Ok(ParseBuildResult::Phony(phony))
    } else {
        let build = expand_build(&exp_scope, &implicit_inputs, &order_only_inputs)?;
        Ok(ParseBuildResult::Build(build))
    }
}

fn expand_build<'s>(
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

fn expand_phony<'s>(
    exp_scope: &ExpansionScope<'_, 's>,
    order_only_input: &[Cow<'s, str>],
) -> Result<PhonyBuild<'s>, Error> {
    let description = exp_scope.get("description");
    Ok(PhonyBuild {
        targets: exp_scope.out_files.to_vec(),
        order_only_inputs: order_only_input.to_vec(),
        description,
    })
}

/// Parse a variable assignment, and expand immediately.
fn parse_variable_assignment<'s>(
    lexer: &mut Lexer<'s>,
    scopes: &[&Scope<'s>],
) -> Result<(&'s str, Cow<'s, str>), Error> {
    let name = lexer.next().ok_or(Error::UnexpectedEof(
        "parsing the name of an assignment".into(),
    ))??;
    let Token::Word(name) = name else {
        let (line, col) = lexer.cursor_pos();
        return Err(Error::UnexpectedToken(format!("{name:?}"), line, col));
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
    let name = lexer.next().ok_or(Error::UnexpectedEof(
        "parsing the name of an assignment".into(),
    ))??;
    let Token::Word(name) = name else {
        let (line, col) = lexer.cursor_pos();
        return Err(Error::UnexpectedToken(format!("{name:?}"), line, col));
    };

    lexer.skip_spaces();
    lexer.expect(Token::Equal)?;
    lexer.skip_spaces();

    let value = parse_noexpand_word(lexer)?;

    lexer.skip_spaces();

    Ok((name, value))
}

/// Parse and expand a word. Returns if the next token is not a word token,
/// and the corresponding token will be left in the Peekable::next() on return.
fn parse_expand_word<'s>(
    lexer: &mut Lexer<'s>,
    scope: &[&Scope<'s>],
    inline: bool,
) -> Result<Cow<'s, str>, Error> {
    let mut res: std::borrow::Cow<'s, str> = std::borrow::Cow::Borrowed("");
    loop {
        let Some(peek) = lexer.peek()? else { break };
        match peek {
            Token::Word(w) => {
                if res.is_empty() {
                    res = Cow::Borrowed(w);
                } else {
                    res.to_mut().push_str(w);
                }
            }
            Token::Spaces(w) if !inline => {
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
            Token::Colon if !inline => res.to_mut().push(':'),
            Token::Pipe if !inline => res.to_mut().push('|'),
            Token::TwoPipe if !inline => res.to_mut().push_str("||"),
            Token::Equal if !inline => res.to_mut().push('='),
            _ => break,
        }
        let _ = lexer.next(); // consume the token just processed
    }
    Ok(res)
}

/// Parse word but don't expand variables. The rest of the semantics the same
/// as parse_expand_word.
fn parse_noexpand_word<'s>(lexer: &mut Lexer<'s>) -> Result<Expandable<'s>, Error> {
    let mut res = SmallVec::new();
    let mut acc: Option<Cow<'_, str>> = None;
    loop {
        let Some(peek) = lexer.peek()? else { break };
        match peek {
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
                    res.push(super::model::Segment::Regular(acc));
                }
                res.push(super::model::Segment::Var(name));
            }
            Token::Colon => acc.get_or_insert_default().to_mut().push(':'),
            Token::Pipe => acc.get_or_insert_default().to_mut().push('|'),
            Token::TwoPipe => acc.get_or_insert_default().to_mut().push_str("||"),
            Token::Equal => acc.get_or_insert_default().to_mut().push('='),
            _ => break,
        }
        let _ = lexer.next(); // consume the token just processed
    }
    if let Some(acc) = acc {
        res.push(super::model::Segment::Regular(acc));
    }
    Ok(Expandable(res))
}
