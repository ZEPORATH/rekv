use crate::constants::{PATH_SEPARATOR_CHAR, ROOT_PATH, WILDCARD_SINGLE};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    Id(String),
    Index(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Key(String),
    Selector(Selector),
    Wildcard,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct QueryPath {
    pub segments: Vec<Segment>,
}

impl QueryPath {
    pub fn parse(input: &str) -> Result<Self, String> {
        let input = input.trim();
        if input.is_empty() || input == ROOT_PATH {
            return Ok(Self::default());
        }
        if !input.starts_with(PATH_SEPARATOR_CHAR) {
            return Err("path must start with '/'".to_string());
        }

        let mut segments = Vec::new();
        let mut start = 1;
        let mut in_selector = false;
        for (offset, character) in input.char_indices().skip(1) {
            match character {
                '[' if !in_selector => in_selector = true,
                '[' => return Err("nested selectors are invalid".to_string()),
                ']' if in_selector => in_selector = false,
                ']' => return Err("unexpected ']' in path".to_string()),
                PATH_SEPARATOR_CHAR if !in_selector => {
                    let token = &input[start..offset];
                    if token.is_empty() {
                        return Err("empty path segment".to_string());
                    }
                    parse_segment(token, &mut segments)?;
                    start = offset + character.len_utf8();
                }
                _ => {}
            }
        }
        if in_selector {
            return Err("selector is missing closing ']'".to_string());
        }
        let token = &input[start..];
        if token.is_empty() {
            return Err("empty path segment".to_string());
        }
        parse_segment(token, &mut segments)?;

        Ok(Self { segments })
    }

    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }
}

fn parse_segment(token: &str, segments: &mut Vec<Segment>) -> Result<(), String> {
    if token == WILDCARD_SINGLE {
        segments.push(Segment::Wildcard);
        return Ok(());
    }
    if token.contains('#') {
        return Err("'#' selectors are not supported; use [id = value]".to_string());
    }
    if token.contains('*') {
        return Err("only a single '*' path segment is supported".to_string());
    }

    if let Some(open) = token.find('[') {
        if !token.ends_with(']') || token[open + 1..token.len() - 1].contains(']') {
            return Err("invalid selector syntax".to_string());
        }
        let name = &token[..open];
        if !is_valid_name(name) {
            return Err("selector must follow an array name".to_string());
        }
        segments.push(Segment::Key(name.to_string()));
        segments.push(parse_selector(&token[open + 1..token.len() - 1])?);
        return Ok(());
    }

    if !is_valid_name(token) {
        return Err(format!(
            "invalid path segment '{}': expected a name or '*'",
            token
        ));
    }
    segments.push(Segment::Key(token.to_string()));
    Ok(())
}

fn parse_selector(input: &str) -> Result<Segment, String> {
    let (key, raw_value) = input
        .split_once('=')
        .ok_or_else(|| "selector must contain '='".to_string())?;
    if raw_value.contains('=') {
        return Err("selector must contain exactly one '='".to_string());
    }
    let key = key.trim();
    let raw_value = raw_value.trim();
    if raw_value.is_empty() {
        return Err("selector value cannot be empty".to_string());
    }

    match key {
        "id" => parse_id_value(raw_value).map(|value| Segment::Selector(Selector::Id(value))),
        "idx" => {
            if raw_value.starts_with(['"', '\'']) {
                return Err("idx selector requires a zero-based integer".to_string());
            }
            raw_value
                .parse::<usize>()
                .map(Selector::Index)
                .map(Segment::Selector)
                .map_err(|_| "idx selector requires a zero-based integer".to_string())
        }
        _ => Err("selector key must be 'id' or 'idx'".to_string()),
    }
}

fn parse_id_value(value: &str) -> Result<String, String> {
    let unquoted = if value.starts_with('"') || value.starts_with('\'') {
        let quote = value.chars().next().unwrap();
        if value.len() < 2 || !value.ends_with(quote) {
            return Err("unterminated id selector value".to_string());
        }
        &value[1..value.len() - 1]
    } else {
        value
    };
    if unquoted.is_empty() || unquoted.chars().any(char::is_whitespace) {
        return Err("id selector value must be one non-empty token".to_string());
    }
    Ok(unquoted.to_string())
}

fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_alphanumeric() || character == '_' || character == '-')
}

#[cfg(test)]
mod tests {
    use super::{QueryPath, Segment, Selector};

    #[test]
    fn parses_plain_and_selector_paths_with_spacing_normalized() {
        assert_eq!(
            QueryPath::parse("/platform_manager/io_devices")
                .unwrap()
                .segments,
            vec![
                Segment::Key("platform_manager".into()),
                Segment::Key("io_devices".into())
            ]
        );
        let compact = QueryPath::parse("/io_devices[id=ECU0]/baud_rate").unwrap();
        let spaced = QueryPath::parse("/io_devices[id = ECU0 ]/baud_rate").unwrap();
        assert_eq!(compact, spaced);
        assert_eq!(
            compact.segments,
            vec![
                Segment::Key("io_devices".into()),
                Segment::Selector(Selector::Id("ECU0".into())),
                Segment::Key("baud_rate".into())
            ]
        );
    }

    #[test]
    fn parses_index_and_direct_child_wildcard() {
        assert_eq!(
            QueryPath::parse("/ports[idx = 0]/name").unwrap().segments,
            vec![
                Segment::Key("ports".into()),
                Segment::Selector(Selector::Index(0)),
                Segment::Key("name".into())
            ]
        );
        assert_eq!(
            QueryPath::parse("/io_devices[id=ECU0]/*").unwrap().segments,
            vec![
                Segment::Key("io_devices".into()),
                Segment::Selector(Selector::Id("ECU0".into())),
                Segment::Wildcard
            ]
        );
    }

    #[test]
    fn rejects_legacy_and_invalid_selector_syntax() {
        assert!(QueryPath::parse("/io_devices#ECU0/baud_rate").is_err());
        assert!(QueryPath::parse("/io_devices[type = ECU0]").is_err());
        assert!(QueryPath::parse("/io_devices[id ECU0]").is_err());
        assert!(QueryPath::parse("/**/baud_rate").is_err());
    }
}
