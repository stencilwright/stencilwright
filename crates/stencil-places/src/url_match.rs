use anyhow::{Result, bail};

#[derive(Debug, Clone)]
struct ParsedUrl<'a> {
    scheme: &'a str,
    authority: &'a str,
    path_segments: Vec<&'a str>,
    query: Vec<QueryPair<'a>>,
    fragment: Option<&'a str>,
}

#[derive(Debug, Clone)]
struct QueryPair<'a> {
    key: &'a str,
    value: Option<&'a str>,
}

pub(crate) fn matches_signature_url(signature_url: &str, current_url: &str) -> Result<bool> {
    let sig = parse_url(signature_url)?;
    let current = match parse_url(current_url) {
        Ok(current) => current,
        Err(_) => return Ok(false),
    };

    if !component_matches(sig.scheme, current.scheme) {
        return Ok(false);
    }
    if !component_matches(sig.authority, current.authority) {
        return Ok(false);
    }
    if !path_matches(&sig.path_segments, &current.path_segments) {
        return Ok(false);
    }
    if !query_matches(&sig.query, &current.query) {
        return Ok(false);
    }
    if let Some(sig_fragment) = sig.fragment {
        let Some(current_fragment) = current.fragment else {
            return Ok(false);
        };
        if !component_matches(sig_fragment, current_fragment) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn parse_url(input: &str) -> Result<ParsedUrl<'_>> {
    let Some((scheme, rest)) = input.split_once("://") else {
        bail!("signature url must be absolute: {input}");
    };
    let (before_fragment, fragment) = split_once_optional(rest, '#');
    let (before_query, query_raw) = split_once_optional(before_fragment, '?');
    let (authority, path_raw) = match before_query.split_once('/') {
        Some((authority, path)) => (authority, path),
        None => (before_query, ""),
    };
    if scheme.is_empty() || authority.is_empty() {
        bail!("signature url must include scheme and host: {input}");
    }

    Ok(ParsedUrl {
        scheme,
        authority,
        path_segments: path_segments(path_raw),
        query: query_pairs(query_raw),
        fragment,
    })
}

fn split_once_optional(input: &str, delimiter: char) -> (&str, Option<&str>) {
    match input.split_once(delimiter) {
        Some((before, after)) => (before, Some(after)),
        None => (input, None),
    }
}

fn path_segments(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn query_pairs(query: Option<&str>) -> Vec<QueryPair<'_>> {
    let Some(query) = query else {
        return Vec::new();
    };
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((key, value)) => QueryPair {
                key,
                value: Some(value),
            },
            None => QueryPair {
                key: pair,
                value: None,
            },
        })
        .collect()
}

fn path_matches(signature: &[&str], current: &[&str]) -> bool {
    path_matches_from(signature, current)
}

fn path_matches_from(signature: &[&str], current: &[&str]) -> bool {
    match signature.split_first() {
        None => current.is_empty(),
        Some((sig, rest)) if *sig == "**" => {
            path_matches_from(rest, current)
                || (!current.is_empty() && path_matches_from(signature, &current[1..]))
        }
        Some((sig, rest)) => {
            let Some((cur, current_rest)) = current.split_first() else {
                return false;
            };
            component_matches(sig, cur) && path_matches_from(rest, current_rest)
        }
    }
}

fn query_matches(signature: &[QueryPair<'_>], current: &[QueryPair<'_>]) -> bool {
    let mut used = vec![false; current.len()];
    for sig in signature {
        let Some(index) = current.iter().enumerate().position(|(idx, cur)| {
            !used[idx]
                && component_matches(sig.key, cur.key)
                && optional_value_matches(sig.value, cur.value)
        }) else {
            return false;
        };
        used[index] = true;
    }
    true
}

fn optional_value_matches(signature: Option<&str>, current: Option<&str>) -> bool {
    match (signature, current) {
        (Some(sig), Some(cur)) => component_matches(sig, cur),
        (None, None) => true,
        _ => false,
    }
}

fn component_matches(pattern: &str, value: &str) -> bool {
    component_matches_from(pattern.as_bytes(), value.as_bytes())
}

fn component_matches_from(pattern: &[u8], value: &[u8]) -> bool {
    if pattern.is_empty() {
        return value.is_empty();
    }
    if pattern[0] == b'*' {
        return component_matches_from(&pattern[1..], value)
            || (!value.is_empty() && component_matches_from(pattern, &value[1..]));
    }
    if pattern[0] == b'{' {
        if let Some(end) = pattern[1..].iter().position(|byte| *byte == b'}') {
            let rest = &pattern[end + 2..];
            return (1..=value.len()).any(|idx| component_matches_from(rest, &value[idx..]));
        }
    }
    !value.is_empty()
        && pattern[0] == value[0]
        && component_matches_from(&pattern[1..], &value[1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(sig: &str, current: &str) -> bool {
        matches_signature_url(sig, current).unwrap()
    }

    #[test]
    fn root_does_not_match_subpath() {
        assert!(matches(
            "https://www.example.com/",
            "https://www.example.com/"
        ));
        assert!(matches("https://www.example.com/", "https://www.example.com"));
        assert!(!matches(
            "https://www.example.com/",
            "https://www.example.com/feed/main/"
        ));
    }

    #[test]
    fn non_url_current_location_is_not_a_match() {
        assert!(!matches("https://www.example.com/login/", "about:blank"));
    }

    #[test]
    fn trailing_slashes_are_normalized() {
        assert!(matches(
            "https://www.example.com/feed/main/",
            "https://www.example.com/feed/main"
        ));
        assert!(matches(
            "https://www.example.com/feed/main",
            "https://www.example.com/feed/main/"
        ));
    }

    #[test]
    fn query_order_does_not_matter_and_extras_are_allowed() {
        assert!(matches(
            "https://example.test/search?q=rust&type=posts",
            "https://example.test/search?utm=1&type=posts&q=rust"
        ));
    }

    #[test]
    fn repeated_query_keys_require_repeated_values() {
        assert!(matches(
            "https://example.test/search?tag=rust&tag=wasm",
            "https://example.test/search?tag=wasm&tag=rust&tag=cli"
        ));
        assert!(!matches(
            "https://example.test/search?tag=rust&tag=wasm",
            "https://example.test/search?tag=rust"
        ));
    }

    #[test]
    fn fragment_is_required_only_when_present_in_signature() {
        assert!(matches(
            "https://example.test/docs",
            "https://example.test/docs#intro"
        ));
        assert!(matches(
            "https://example.test/docs#intro",
            "https://example.test/docs#intro"
        ));
        assert!(!matches(
            "https://example.test/docs#intro",
            "https://example.test/docs"
        ));
    }

    #[test]
    fn star_and_placeholders_work_in_components() {
        assert!(matches(
            "https://*.example.test/acct/{account_id}/positions?tab=pos*#row-{row_id}",
            "https://client.example.test/acct/12345/positions?tab=positions#row-abc"
        ));
    }

    #[test]
    fn double_star_matches_zero_or_more_path_segments() {
        assert!(matches(
            "https://example.test/accounts/**",
            "https://example.test/accounts"
        ));
        assert!(matches(
            "https://example.test/accounts/**",
            "https://example.test/accounts/a/b/c"
        ));
    }
}
