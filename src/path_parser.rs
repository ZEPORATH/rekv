use nom::{
    branch::alt,
    bytes::complete::{is_not, tag, tag_no_case, take_while1},
    character::complete::{char, digit1, multispace0},
    combinator::{map, map_res, opt, recognize, value},
    multi::many0,
    sequence::{delimited, pair, preceded, tuple},
    IResult,
};

use crate::constants::{
    JSON_FALSE, JSON_TRUE, OP_EQ, OP_EQ_DOUBLE, OP_GT, OP_GTE, OP_LT, OP_LTE, OP_NEQ,
    PATH_SEPARATOR_CHAR, PRIMARY_KEY_SHORTHAND_PREFIX, ROOT_PATH, WILDCARD_RECURSIVE,
    WILDCARD_SINGLE,
};

/// Comparison operator for predicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredicateOp {
    Eq,
    NotEq,
    Gt,
    Gte,
    Lt,
    Lte,
}

/// Value on the right side of a predicate.
#[derive(Debug, Clone, PartialEq)]
pub enum PredicateValue {
    String(String),
    Number(f64),
    Bool(bool),
}

/// A filter predicate like key = value.
#[derive(Debug, Clone, PartialEq)]
pub struct Predicate {
    pub key: String,
    pub op: PredicateOp,
    pub value: PredicateValue,
}

/// A single segment in a query path.
#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    /// Object field name.
    Key(String),
    /// Array index number.
    Index(usize),
    /// Primary key shorthand #ID.
    Id(String),
    /// Filter predicates in brackets.
    Predicates(Vec<Predicate>),
    /// Single-level wildcard *.
    Wildcard,
    /// Multi-level recursive wildcard **.
    RecursiveWildcard,
}

/// Parsed query path containing segments.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QueryPath {
    pub segments: Vec<Segment>,
}

impl QueryPath {
    /// Parse a query path string into a QueryPath.
    pub fn parse(input: &str) -> Result<Self, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() || trimmed == ROOT_PATH {
            return Ok(QueryPath { segments: vec![] });
        }

        match parse_query_path(trimmed) {
            Ok(("", path)) => Ok(path),
            Ok((remaining, _)) => Err(format!(
                "Failed to parse path completely, unparsed remainder: '{}'",
                remaining
            )),
            Err(e) => Err(format!("Parse error at: {}", e)),
        }
    }

    /// Check if path is root /.
    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }
}

// Check if character is valid for an identifier.
fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

// Parse identifier key name.
fn parse_identifier(input: &str) -> IResult<&str, String> {
    map(take_while1(is_ident_char), |s: &str| s.to_string())(input)
}

// Parse comparison operator.
fn parse_operator(input: &str) -> IResult<&str, PredicateOp> {
    delimited(
        multispace0,
        alt((
            value(PredicateOp::Gte, tag(OP_GTE)),
            value(PredicateOp::Lte, tag(OP_LTE)),
            value(PredicateOp::NotEq, tag(OP_NEQ)),
            value(PredicateOp::Eq, tag(OP_EQ_DOUBLE)),
            value(PredicateOp::Eq, tag(OP_EQ)),
            value(PredicateOp::Gt, tag(OP_GT)),
            value(PredicateOp::Lt, tag(OP_LT)),
        )),
        multispace0,
    )(input)
}

// Parse double-quoted string.
fn parse_double_quoted_string(input: &str) -> IResult<&str, String> {
    delimited(char('"'), map(is_not("\""), |s: &str| s.to_string()), char('"'))(input)
}

// Parse single-quoted string.
fn parse_single_quoted_string(input: &str) -> IResult<&str, String> {
    delimited(char('\''), map(is_not("'"), |s: &str| s.to_string()), char('\''))(input)
}

// Parse boolean true or false.
fn parse_bool(input: &str) -> IResult<&str, bool> {
    alt((
        value(true, tag_no_case(JSON_TRUE)),
        value(false, tag_no_case(JSON_FALSE)),
    ))(input)
}

// Parse number.
fn parse_number(input: &str) -> IResult<&str, f64> {
    map_res(
        recognize(tuple((
            opt(char('-')),
            digit1,
            opt(pair(char('.'), digit1)),
        ))),
        |s: &str| s.parse::<f64>(),
    )(input)
}

// Parse predicate value.
fn parse_predicate_value(input: &str) -> IResult<&str, PredicateValue> {
    alt((
        map(parse_double_quoted_string, PredicateValue::String),
        map(parse_single_quoted_string, PredicateValue::String),
        map(parse_bool, PredicateValue::Bool),
        map(parse_number, PredicateValue::Number),
        map(parse_identifier, PredicateValue::String),
    ))(input)
}

// Parse a single predicate expression: key op value.
fn parse_predicate_inner(input: &str) -> IResult<&str, Predicate> {
    let (input, key) = parse_identifier(input)?;
    let (input, op) = parse_operator(input)?;
    let (input, value) = parse_predicate_value(input)?;
    Ok((input, Predicate { key, op, value }))
}

// Parse bracketed index [0] or predicate [key=val].
fn parse_bracket_expression(input: &str) -> IResult<&str, BracketTarget> {
    delimited(
        char('['),
        delimited(
            multispace0,
            alt((
                map(map_res(digit1, |s: &str| s.parse::<usize>()), BracketTarget::Index),
                map(parse_predicate_inner, BracketTarget::Pred),
            )),
            multispace0,
        ),
        char(']'),
    )(input)
}

#[derive(Debug, Clone)]
enum BracketTarget {
    Index(usize),
    Pred(Predicate),
}

// Parse primary key shorthand #ID.
fn parse_id_shorthand(input: &str) -> IResult<&str, Segment> {
    preceded(
        char(PRIMARY_KEY_SHORTHAND_PREFIX),
        map(take_while1(|c: char| is_ident_char(c) || c == '.'), |s: &str| {
            Segment::Id(s.to_string())
        }),
    )(input)
}

// Parse a token and any attached bracket or ID modifiers.
fn parse_token_with_modifiers(input: &str) -> IResult<&str, Vec<Segment>> {
    let mut segments = Vec::new();

    let (mut rest, base_token) = alt((
        map(tag(WILDCARD_RECURSIVE), |_| Segment::RecursiveWildcard),
        map(tag(WILDCARD_SINGLE), |_| Segment::Wildcard),
        map(map_res(digit1, |s: &str| s.parse::<usize>()), Segment::Index),
        map(parse_identifier, Segment::Key),
    ))(input)?;

    segments.push(base_token);

    loop {
        if rest.starts_with(PRIMARY_KEY_SHORTHAND_PREFIX) {
            let (next_rest, id_seg) = parse_id_shorthand(rest)?;
            segments.push(id_seg);
            rest = next_rest;
        } else if rest.starts_with('[') {
            let (next_rest, bracket) = parse_bracket_expression(rest)?;
            match bracket {
                BracketTarget::Index(idx) => {
                    segments.push(Segment::Index(idx));
                }
                BracketTarget::Pred(pred) => {
                    if let Some(Segment::Predicates(list)) = segments.last_mut() {
                        list.push(pred);
                    } else {
                        segments.push(Segment::Predicates(vec![pred]));
                    }
                }
            }
            rest = next_rest;
        } else {
            break;
        }
    }

    Ok((rest, segments))
}

// Parse a full query path separated by slashes.
fn parse_query_path(input: &str) -> IResult<&str, QueryPath> {
    let (input, _) = opt(char(PATH_SEPARATOR_CHAR))(input)?;

    let (input, token_groups) = many0(preceded(
        opt(char(PATH_SEPARATOR_CHAR)),
        parse_token_with_modifiers,
    ))(input)?;

    let segments = token_groups.into_iter().flatten().collect();
    Ok((input, QueryPath { segments }))
}

