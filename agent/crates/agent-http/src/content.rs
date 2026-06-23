//! Turn a fetched body into model-ready text, bounded to the return cap.
use agent_tools::ToolError;
use url::Url;

/// Max bytes of rendered text returned to the model (a 32k-context model must not be
/// flooded by one fetch). Owned here because truncation is a content concern.
pub(crate) const MAX_RETURN: usize = 8 * 1024;

#[derive(Debug)]
pub struct Rendered {
    pub kind: &'static str,
    pub text: String,
}

/// Render `body` according to `content_type`. `download_truncated` is true when the
/// 2 MiB stream cap was hit, so the marker is emitted even if the rendered text is short.
pub fn render(
    content_type: &str,
    body: &[u8],
    base: &Url,
    download_truncated: bool,
) -> Result<Rendered, ToolError> {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    let (kind, raw): (&'static str, String) = if mime == "text/html" || mime == "application/xhtml+xml" {
        ("markdown", html_to_text(body, base))
    } else if mime == "application/json" || mime.ends_with("+json") {
        ("json", String::from_utf8_lossy(body).into_owned())
    } else if mime.starts_with("text/") {
        ("text", String::from_utf8_lossy(body).into_owned())
    } else {
        let shown = if mime.is_empty() { "unknown" } else { &mime };
        return Err(ToolError::Failed {
            message: format!("non-text content ({} bytes, {shown})", body.len()),
            stderr: None,
        });
    };

    Ok(truncate(kind, raw, body.len(), download_truncated))
}

/// Extract readable text from HTML (drops nav/header/footer/script/style boilerplate).
fn html_to_text(body: &[u8], base: &Url) -> String {
    let mut cursor = std::io::Cursor::new(body);
    // readability 0.3.0: `extractor::extract(&mut impl Read, &Url) -> Result<Product, Error>`
    // where `Product { title: String, content: String, text: String }`.
    // Confirmed via registry source at:
    //   ~/.cargo/registry/src/.../readability-0.3.0/src/extractor.rs
    match readability::extractor::extract(&mut cursor, base) {
        Ok(product) if !product.text.trim().is_empty() => product.text,
        _ => String::from_utf8_lossy(body).into_owned(),
    }
}

/// Bound the rendered text to `MAX_RETURN` bytes (on a char boundary) and append a
/// marker if anything was dropped (either here or by the 2 MiB download cap).
fn truncate(kind: &'static str, mut text: String, downloaded: usize, download_truncated: bool) -> Rendered {
    let mut note = download_truncated;
    if text.len() > MAX_RETURN {
        let mut cut = MAX_RETURN;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        note = true;
    }
    if note {
        text.push_str(&format!(
            "\n\n[truncated: {downloaded} bytes downloaded]"
        ));
    }
    Rendered { kind, text }
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    fn base() -> Url { Url::parse("https://example.com/").unwrap() }

    #[test]
    fn html_is_reduced_to_readable_text() {
        let html = b"<html><body><nav>menu</nav><h1>Title</h1><p>Hello world body.</p>\
                     <script>evil()</script></body></html>";
        let r = render("text/html; charset=utf-8", html, &base(), false).unwrap();
        assert_eq!(r.kind, "markdown");
        assert!(r.text.contains("Hello world body"));
        assert!(!r.text.contains("evil()"), "script content must be dropped");
    }

    #[test]
    fn json_passes_through_raw() {
        let r = render("application/json", br#"{"a":1}"#, &base(), false).unwrap();
        assert_eq!(r.kind, "json");
        assert_eq!(r.text.trim(), r#"{"a":1}"#);
    }

    #[test]
    fn plain_text_passes_through() {
        let r = render("text/plain", b"just text", &base(), false).unwrap();
        assert_eq!(r.kind, "text");
        assert_eq!(r.text, "just text");
    }

    #[test]
    fn binary_is_refused() {
        let err = render("application/octet-stream", &[0u8, 159, 146, 150], &base(), false).unwrap_err();
        match err {
            agent_tools::ToolError::Failed { message, .. } => assert!(message.contains("non-text content")),
            _ => panic!("expected Failed"),
        }
    }

    #[test]
    fn long_text_is_truncated_with_marker() {
        let big = "x".repeat(MAX_RETURN * 2);
        let r = render("text/plain", big.as_bytes(), &base(), false).unwrap();
        assert!(r.text.len() <= MAX_RETURN + 64);
        assert!(r.text.contains("[truncated"));
    }

    #[test]
    fn download_truncation_is_noted_even_when_short() {
        let r = render("text/plain", b"partial", &base(), true).unwrap();
        assert!(r.text.contains("[truncated"));
    }
}
