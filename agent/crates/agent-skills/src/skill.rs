use std::path::PathBuf;

/// A parsed skill: identity, markdown body, and bundled files (absolute paths).
#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
    pub dir: PathBuf,
    pub files: Vec<PathBuf>,
}

/// The result of parsing a SKILL.md's text (before a directory/name is attached).
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSkill {
    pub name: Option<String>,
    pub description: String,
    pub body: String,
}

/// Parse a SKILL.md into `(name?, description, body)`. The frontmatter is a
/// leading `---` ... `---` block of simple `key: value` lines (single-line
/// scalar values only; surrounding quotes are stripped). Returns `Err` if the
/// frontmatter block is missing/unterminated or lacks a non-empty `description`.
pub fn parse_skill_md(text: &str) -> Result<ParsedSkill, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text); // tolerate a BOM
    let mut lines = text.lines();
    // First non-empty line must be the opening fence.
    let opened = lines.by_ref().find(|l| !l.trim().is_empty());
    if opened.map(str::trim) != Some("---") {
        return Err("missing front matter (file must start with a `---` block)".into());
    }
    let mut name = None;
    let mut description = None;
    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            let val = unquote(v.trim());
            match key {
                "name" => name = Some(val.to_string()),
                "description" => description = Some(val.to_string()),
                _ => {} // ignore unknown keys for forward-compat
            }
        }
    }
    if !closed {
        return Err("unterminated front matter (no closing `---`)".into());
    }
    let description = description
        .filter(|d| !d.trim().is_empty())
        .ok_or("front matter is missing a non-empty `description`")?;
    let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    Ok(ParsedSkill { name, description, body })
}

fn unquote(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_body() {
        let text = "---\nname: my-skill\ndescription: Does a thing\n---\n\n# Body\nDo the thing.\n";
        let p = parse_skill_md(text).unwrap();
        assert_eq!(p.name.as_deref(), Some("my-skill"));
        assert_eq!(p.description, "Does a thing");
        assert_eq!(p.body, "# Body\nDo the thing.");
    }

    #[test]
    fn strips_surrounding_quotes_from_values() {
        let text = "---\ndescription: \"Quoted desc\"\n---\nbody\n";
        let p = parse_skill_md(text).unwrap();
        assert_eq!(p.description, "Quoted desc");
    }

    #[test]
    fn missing_frontmatter_is_error() {
        let err = parse_skill_md("no front matter here").unwrap_err();
        assert!(err.contains("front matter"));
    }

    #[test]
    fn missing_description_is_error() {
        let err = parse_skill_md("---\nname: x\n---\nbody").unwrap_err();
        assert!(err.contains("description"));
    }

    #[test]
    fn unterminated_frontmatter_is_error() {
        let err = parse_skill_md("---\ndescription: x\nbody with no close").unwrap_err();
        assert!(err.contains("front matter"));
    }
}
