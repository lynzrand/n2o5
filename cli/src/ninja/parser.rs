use smallvec::SmallVec;
use std::{borrow::Cow, collections::HashMap};

use super::model::{
    Build, DepsType, Error, Expandable, ExpansionScope, NinjaFile, Rule, RuleScope, Scope,
};
use super::tokenizer::{Lexer, Token};

pub fn parse<'s>(s: &'s str) -> Result<NinjaFile<'s>, Error> {
    use Token::*;
    let mut lexer = Lexer::new(s);
    let mut global_scope = Scope::new();
    let mut rules: HashMap<&'s str, Rule<'s>> = HashMap::new();
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

    let mut scope: RuleScope<'s> = HashMap::new();

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

/// Parse a variable assignment, and expand immediately.
fn parse_variable_assignment<'s>(
    lexer: &mut Lexer<'s>,
    scopes: &[&Scope<'s>],
) -> Result<(&'s str, Cow<'s, str>), Error> {
    let name = lexer.next().ok_or(Error::UnexpectedEof)??;
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
    let name = lexer.next().ok_or(Error::UnexpectedEof)??;
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
                    res.push(super::model::Segment::Regular(acc));
                }
                res.push(super::model::Segment::Var(name));
            }
            _ => break,
        }
        let _ = lexer.next(); // consume the token just processed
    }
    if let Some(acc) = acc {
        res.push(super::model::Segment::Regular(acc));
    }
    Ok(Expandable(res))
}
