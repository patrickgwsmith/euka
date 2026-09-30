use crate::session;

#[derive(Debug, PartialEq, Eq)]
pub enum Part<'a> {
    Literal(&'a str),
    Agent { model: &'a str, task: &'a str },
}

pub fn parse(line: &str) -> Result<Option<Vec<Part<'_>>>, String> {
    let bytes = line.as_bytes();
    let mut parts = Vec::new();
    let mut literal_start = 0;
    let mut index = 0;
    let mut quote = None;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\\' && quote != Some(b'\'') {
            index = (index + 2).min(bytes.len());
            continue;
        }
        if let Some(active) = quote {
            if byte == active {
                quote = None;
            }
            index += 1;
            continue;
        }
        if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
            index += 1;
            continue;
        }
        let boundary = index == 0
            || bytes[index - 1].is_ascii_whitespace()
            || matches!(bytes[index - 1], b'|' | b'<' | b'>');
        if boundary && byte.is_ascii_alphabetic() {
            let mut end = index;
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'_' | b'-'))
            {
                end += 1;
            }
            if end + 1 < bytes.len()
                && bytes[end] == b'?'
                && bytes[end + 1] == b'('
                && session::is_agent_model(&line[index..end])
            {
                let model = &line[index..end];
                let task_start = end + 2;
                let mut close = task_start;
                while close < bytes.len() && bytes[close] != b')' {
                    if bytes[close] == b'(' {
                        return Err(format!(
                            "{model}?(...): nested parentheses are not supported"
                        ));
                    }
                    close += 1;
                }
                if close == bytes.len() {
                    return Err(format!("{model}?(...): missing closing ')'"));
                }
                let task = line[task_start..close].trim();
                if task.is_empty() {
                    return Err(format!("{model}?(...): task is empty"));
                }
                let after = close + 1;
                if after < bytes.len()
                    && !bytes[after].is_ascii_whitespace()
                    && !matches!(bytes[after], b'|' | b'<' | b'>')
                {
                    return Err(format!("{model}?(...): must occupy one whole argument"));
                }
                parts.push(Part::Literal(&line[literal_start..index]));
                parts.push(Part::Agent { model, task });
                literal_start = after;
                index = after;
                continue;
            }
        }
        index += 1;
    }
    if parts.is_empty() {
        return Ok(None);
    }
    parts.push(Part::Literal(&line[literal_start..]));
    Ok(Some(parts))
}

pub fn shell_substitution(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut quote = None;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\\' && quote != Some(b'\'') {
            index = (index + 2).min(bytes.len());
            continue;
        }
        if let Some(active) = quote {
            if byte == active {
                quote = None;
            }
        } else if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
        }
        if quote != Some(b'\'') && byte == b'$' && bytes.get(index + 1) == Some(&b'(') {
            return true;
        }
        index += 1;
    }
    false
}

pub fn quote_argument(value: &str) -> Result<String, String> {
    if value.contains('\0') {
        return Err("agent reply contains a NUL byte".into());
    }
    Ok(format!("'{}'", value.trim().replace('\'', "'\\''")))
}

#[cfg(test)]
mod tests {
    use super::{parse, quote_argument, shell_substitution, Part};

    #[test]
    fn parses_inline_agent_as_one_argument() {
        assert_eq!(
            parse("git commit -m luna?(Suggest a commit message)").unwrap(),
            Some(vec![
                Part::Literal("git commit -m "),
                Part::Agent {
                    model: "luna",
                    task: "Suggest a commit message"
                },
                Part::Literal("")
            ])
        );
        assert!(parse("echo luna?(an (inner) request)").is_err());
        assert!(parse("echo luna?(unfinished").is_err());
        assert!(parse("echo luna?(task)tail").is_err());
        assert_eq!(parse("echo 'luna?(literal)'").unwrap(), None);
    }

    #[test]
    fn shell_substitution_respects_quotes_and_escapes() {
        assert!(shell_substitution("echo $(date)"));
        assert!(shell_substitution("echo \"$(date)\""));
        assert!(!shell_substitution("echo '$(date)'"));
        assert!(!shell_substitution("echo \\$(date)"));
        assert_eq!(quote_argument("it's $HOME").unwrap(), "'it'\\''s $HOME'");
    }
}
