use smallvec::SmallVec;
use std::{borrow::Cow, collections::HashMap};

/// Errors during parsing of Ninja files.
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

    #[error("Unexpected indentation at top level")]
    UnexpectedIndentation,
}

/// Dependency processing type for the `deps` rule variable
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DepsType {
    Gcc,
    Msvc,
}

/// A segment of [`Expandable`]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Segment<'s> {
    Regular(Cow<'s, str>),
    Var(&'s str),
}

/// A string that may contain `$`-replacements, which is handled lazily
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Expandable<'s>(pub SmallVec<[Segment<'s>; 1]>);

impl<'s> Expandable<'s> {
    /// Expand the variable using the provided scope
    pub fn expand(&self, scope: &ExpansionScope<'_, 's>) -> Cow<'s, str> {
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

pub type Scope<'s> = HashMap<&'s str, Cow<'s, str>>;
pub type RuleScope<'s> = HashMap<&'s str, Expandable<'s>>;

/// Corresponding to a ninja `rule` block
#[derive(Debug, Clone)]
pub struct Rule<'s> {
    pub vars: RuleScope<'s>,
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

/// The scope required for expanding variables in a build statement
#[derive(Debug, Copy, Clone)]
pub struct ExpansionScope<'r, 's> {
    pub in_files: &'r [Cow<'s, str>],
    pub out_files: &'r [Cow<'s, str>],
    pub global_scope: &'r Scope<'s>,
    pub build_scope: &'r Scope<'s>,
    pub rule: &'r Rule<'s>,
}

impl<'r, 's> ExpansionScope<'r, 's> {
    pub fn get(&self, variable: &str) -> Option<Cow<'s, str>> {
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

/// A `build` statement, expanded
#[derive(Debug, Clone)]
pub struct Build<'s> {
    pub inputs: Vec<Cow<'s, str>>,
    pub implicit_inputs: Vec<Cow<'s, str>>,
    pub order_only_inputs: Vec<Cow<'s, str>>,
    pub outputs: Vec<Cow<'s, str>>,

    /// The command line to run (required)
    pub command: Cow<'s, str>,
    /// Path to an optional Makefile that contains extra implicit dependencies
    pub depfile: Option<Cow<'s, str>>,
    /// Special dependency processing type
    pub deps: Option<DepsType>,
    /// String which should be stripped from msvc's /showIncludes output
    pub msvc_deps_prefix: Option<Cow<'s, str>>,
    /// A short description of the command
    pub description: Option<Cow<'s, str>>,
    /// Dynamically discovered dependency information file
    pub dyndep: Option<Cow<'s, str>>,
    /// Specifies that this rule is used to re-invoke the generator program
    pub generator: bool,
    /// Causes Ninja to re-stat the command's outputs after execution
    pub restat: bool,
    /// Response file path
    pub rspfile: Option<Cow<'s, str>>,
    /// Response file content
    pub rspfile_content: Option<Cow<'s, str>>,
}

/// A complete parsed Ninja file.
///
/// Most values are borrowed from the original string when possible using the `'s` lifetime.
#[derive(Debug, Clone)]
pub struct NinjaFile<'s> {
    pub global_scope: Scope<'s>,
    pub rules: HashMap<&'s str, Rule<'s>>,
    pub builds: Vec<Build<'s>>,
}
