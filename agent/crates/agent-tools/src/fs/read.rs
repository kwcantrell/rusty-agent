use crate::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;

fn arg_path(args: &serde_json::Value) -> Result<String, ToolError> {
    args.get("path")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs("missing string field `path`".into()))
}

pub struct ReadFile {
    pub max_bytes: usize,
}

#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "Read the contents of a file within the workspace."
    }
    fn when_not_to_call(&self) -> Option<&str> {
        Some(
            "Not for files bundled inside a loaded skill's directory — use \
              read_skill_file for those. Use read_file for workspace paths.",
        )
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({"type":"object","properties":{
                "path":{"type":"string","description":"Workspace-relative path of the file to read."},
                "offset":{"type":"integer","description":"1-based line number to start reading from (default 1)."},
                "limit":{"type":"integer","description":"Maximum number of lines to return (default: all lines)."},
                "byte_offset":{"type":"integer","description":
                    "Raw byte offset to continue a large read from (from a previous \
                     page's continuation marker). Returns raw bytes with no line header. \
                     Mutually exclusive with offset/limit."}},
                "required":["path"]}),
        }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let path = arg_path(args)?;
        Ok(ToolIntent {
            tool: "read_file".into(),
            access: Access::Read,
            paths: vec![path.clone().into()],
            command: None,
            summary: format!("read {path}"),
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolCtx,
    ) -> Result<ToolOutput, ToolError> {
        let path = arg_path(&args)?;
        let content = ctx.backend.read(&path).await.map_err(|e| match e {
            crate::backend::FsError::NotUtf8(m) => ToolError::Failed {
                message: format!("{m} (binary file) — file tools are text-only"),
                stderr: None,
            },
            other => crate::fs::fs_err(other),
        })?;
        let byte_offset = args
            .get("byte_offset")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);
        let offset = args
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|v| (v as usize).max(1));
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);
        if byte_offset.is_some() && (offset.is_some() || limit.is_some()) {
            return Err(ToolError::InvalidArgs(
                "byte_offset is mutually exclusive with offset/limit".into(),
            ));
        }
        if limit == Some(0) {
            return Err(ToolError::InvalidArgs("limit must be >= 1".into()));
        }
        if let Some(start) = byte_offset {
            return byte_page(&path, &content, start, self.max_bytes).map(|content| ToolOutput {
                content,
                display: None,
            });
        }
        let rendered = render_lines(&path, &content, offset, limit)?; // today's logic, extracted
        if rendered.len() <= self.max_bytes {
            return Ok(ToolOutput {
                content: rendered,
                display: None,
            });
        }
        // Over-cap: whole lines that fit + line-mode marker; monster line → byte mode.
        Ok(ToolOutput {
            content: capped_lines(&path, &content, offset.unwrap_or(1), self.max_bytes),
            display: None,
        })
    }
}

/// Today's `(offset, limit)` match block, extracted verbatim (moved, not
/// rewritten) so the existing under-cap pins keep passing byte-for-byte.
fn render_lines(
    path: &str,
    content: &str,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<String, ToolError> {
    Ok(match (offset, limit) {
        (None, None) => content.to_string(), // whole-file fast path, byte-identical
        (o, l) => {
            let first = o.unwrap_or(1);
            let lines: Vec<&str> = content.lines().collect();
            let n = lines.len();
            if first > n {
                return Err(ToolError::InvalidArgs(format!(
                    "offset {first} is past the end of {path} ({n} lines)"
                )));
            }
            // Saturating: l >= 1 is guaranteed by the limit==0 guard above,
            // and first.saturating_add avoids overflow when limit is huge
            // (e.g. u64::MAX) so we never wrap into a bad slice range.
            let last = l.map_or(n, |l| first.saturating_add(l - 1).min(n));
            if first == 1 && last == n {
                content.to_string() // limit covers the whole file: unchanged
            } else {
                format!(
                    "[lines {first}–{last} of {n}]\n{}",
                    lines[first - 1..last].join("\n")
                )
            }
        }
    })
}

fn byte_marker(path: &str, start: usize, end: usize, total: usize) -> String {
    format!(
        "\n[bytes {start}–{end} of {total} — continue with read_file(path: \"{path}\", byte_offset: {end})]"
    )
}

/// Raw byte page, char-boundary-snapped on both ends (spec §5.4 byte mode).
fn byte_page(path: &str, content: &str, offset: usize, cap: usize) -> Result<String, ToolError> {
    let total = content.len();
    if offset > 0 && offset >= total {
        return Err(ToolError::InvalidArgs(format!(
            "byte_offset {offset} is past the end of {path} ({total} bytes)"
        )));
    }
    let mut start = offset;
    while !content.is_char_boundary(start) {
        start -= 1;
    }
    let rest = &content[start..];
    if rest.len() <= cap {
        return Ok(rest.to_string());
    }
    let worst = byte_marker(path, start, total, total);
    let budget = cap.saturating_sub(worst.len()).max(1);
    let mut cut = start + budget;
    while !content.is_char_boundary(cut) {
        cut -= 1;
    }
    if cut <= start {
        cut = start + rest.chars().next().map_or(1, |c| c.len_utf8());
    }
    Ok(format!(
        "{}{}",
        &content[start..cut],
        byte_marker(path, start, cut, total)
    ))
}

/// Over-cap line mode: greedily keep whole lines under the cap; if not even
/// one line fits, fall to a byte page starting at the first requested line.
fn capped_lines(path: &str, content: &str, first: usize, cap: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let n = lines.len();
    let worst = format!(
        "\n[lines {first}–{n} of {n} — continue with read_file(path: \"{path}\", offset: {})]",
        n + 1
    );
    let header_worst = format!("[lines {first}–{n} of {n}]\n");
    let budget = cap.saturating_sub(worst.len() + header_worst.len());
    let mut kept = 0usize;
    let mut used = 0usize;
    for l in &lines[first - 1..] {
        let add = l.len() + 1;
        if used + add > budget {
            break;
        }
        used += add;
        kept += 1;
    }
    if kept == 0 {
        // Monster line: byte page from the first requested line's byte offset.
        let start: usize = lines[..first - 1].iter().map(|l| l.len() + 1).sum();
        return byte_page(path, content, start, cap).expect("start < total by construction");
    }
    let last = first + kept - 1;
    format!(
        "[lines {first}–{last} of {n}]\n{}\n[lines {first}–{last} of {n} — continue with read_file(path: \"{path}\", offset: {})]",
        lines[first - 1..last].join("\n"),
        last + 1
    )
}

pub struct ListDirectory;

#[async_trait]
impl Tool for ListDirectory {
    fn name(&self) -> &str {
        "list_directory"
    }
    fn description(&self) -> &str {
        "List entries of a directory within the workspace."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({"type":"object","properties":{
                "path":{"type":"string","description":"Workspace-relative path of the directory to list."}},
                "required":["path"]}),
        }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let path = arg_path(args)?;
        Ok(ToolIntent {
            tool: "list_directory".into(),
            access: Access::Read,
            paths: vec![path.clone().into()],
            command: None,
            summary: format!("list {path}"),
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolCtx,
    ) -> Result<ToolOutput, ToolError> {
        let path = arg_path(&args)?;
        let entries = ctx.backend.ls(&path).await.map_err(crate::fs::fs_err)?;
        let names: Vec<String> = entries.into_iter().map(|e| e.name).collect();
        Ok(ToolOutput {
            content: names.join("\n"),
            display: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use serde_json::json;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    fn ctx(ws: std::path::PathBuf) -> ToolCtx {
        use std::sync::Arc;
        ToolCtx {
            workspace: ws.clone(),
            timeout: Duration::from_secs(5),
            cancel: CancellationToken::new(),
            sandbox: Arc::new(crate::HostExecutor),
            backend: Arc::new(crate::backend::HostBackend::new(ws)),
            call_id: "test".into(),
        }
    }

    #[tokio::test]
    async fn read_file_returns_contents() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        let out = ReadFile {
            max_bytes: 16 * 1024,
        }
        .execute(json!({"path":"a.txt"}), &ctx(dir.path().into()))
        .await
        .unwrap();
        assert_eq!(out.content, "hello");
    }

    #[tokio::test]
    async fn read_file_slices_with_offset_and_limit() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
        let out = ReadFile {
            max_bytes: 16 * 1024,
        }
        .execute(
            json!({"path": "f.txt", "offset": 2, "limit": 2}),
            &ctx(dir.path().into()),
        )
        .await
        .unwrap();
        assert_eq!(out.content, "[lines 2–3 of 5]\nl2\nl3");
    }

    #[tokio::test]
    async fn read_file_limit_clamps_to_eof() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "l1\nl2\nl3\n").unwrap();
        let out = ReadFile {
            max_bytes: 16 * 1024,
        }
        .execute(
            json!({"path": "f.txt", "offset": 3, "limit": 99}),
            &ctx(dir.path().into()),
        )
        .await
        .unwrap();
        assert_eq!(out.content, "[lines 3–3 of 3]\nl3");
    }

    #[tokio::test]
    async fn read_file_default_is_whole_file_unchanged() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "l1\nl2\n").unwrap();
        let out = ReadFile {
            max_bytes: 16 * 1024,
        }
        .execute(json!({"path": "f.txt"}), &ctx(dir.path().into()))
        .await
        .unwrap();
        assert_eq!(out.content, "l1\nl2\n"); // byte-identical, incl. trailing newline
    }

    #[tokio::test]
    async fn read_file_limit_zero_is_invalid_args() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "l1\nl2\n").unwrap();
        let err = ReadFile {
            max_bytes: 16 * 1024,
        }
        .execute(
            json!({"path": "f.txt", "limit": 0}),
            &ctx(dir.path().into()),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn read_file_offset_with_u64_max_limit_does_not_panic() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
        // limit == u64::MAX would overflow `first + limit - 1` without
        // saturating arithmetic; offset 2 must still return lines 2..n.
        let out = ReadFile {
            max_bytes: 16 * 1024,
        }
        .execute(
            json!({"path": "f.txt", "offset": 2, "limit": u64::MAX}),
            &ctx(dir.path().into()),
        )
        .await
        .unwrap();
        assert_eq!(out.content, "[lines 2–5 of 5]\nl2\nl3\nl4\nl5");
    }

    #[tokio::test]
    async fn read_file_offset_past_eof_is_invalid_args() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "l1\n").unwrap();
        let err = ReadFile {
            max_bytes: 16 * 1024,
        }
        .execute(
            json!({"path": "f.txt", "offset": 5}),
            &ctx(dir.path().into()),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn read_file_offset_limit_params_are_described() {
        let schema = (ReadFile {
            max_bytes: 16 * 1024,
        })
        .schema();
        for p in ["offset", "limit"] {
            let d = schema.parameters["properties"][p]["description"]
                .as_str()
                .unwrap_or("");
            assert!(!d.is_empty(), "{p} must be described");
        }
    }

    #[test]
    fn read_file_intent_is_read_access() {
        let intent = (ReadFile {
            max_bytes: 16 * 1024,
        })
        .intent(&json!({"path":"a.txt"}))
        .unwrap();
        assert_eq!(intent.access, Access::Read);
        assert_eq!(intent.tool, "read_file");
    }

    /// Extract the continuation byte offset from a page's trailing marker.
    fn byte_continuation(page: &str) -> Option<usize> {
        let tail = page.rsplit("byte_offset: ").next()?;
        tail.split(')').next()?.trim().parse().ok()
    }

    #[tokio::test]
    async fn byte_mode_pages_reassemble_exact_bytes() {
        // Ports recall_pages_a_large_entry_to_completion (spec §7).
        let dir = tempdir().unwrap();
        let content: String = (0..10_000)
            .map(|i| char::from(b'a' + (i % 26) as u8))
            .collect();
        std::fs::write(dir.path().join("blob"), &content).unwrap();
        let tool = ReadFile { max_bytes: 4096 };
        let mut reassembled = String::new();
        let mut offset = 0usize;
        loop {
            let out = tool
                .execute(
                    json!({"path": "blob", "byte_offset": offset}),
                    &ctx(dir.path().into()),
                )
                .await
                .unwrap();
            assert!(out.content.len() <= 4096, "page exceeds cap");
            match byte_continuation(&out.content) {
                Some(next) if out.content.contains("continue with read_file") => {
                    let body = out.content.rsplit_once("\n[bytes ").unwrap().0;
                    assert!(next > offset, "no forward progress");
                    reassembled.push_str(body);
                    offset = next;
                }
                _ => {
                    reassembled.push_str(&out.content);
                    break;
                }
            }
        }
        assert_eq!(
            reassembled, content,
            "byte pages must reassemble the original exactly"
        );
    }

    #[tokio::test]
    async fn byte_mode_slices_on_char_boundaries() {
        // Ports recall_slices_on_char_boundaries (spec §7).
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("crab"), "🦀".repeat(3000)).unwrap();
        let tool = ReadFile { max_bytes: 4096 };
        let out = tool
            .execute(
                json!({"path": "crab", "byte_offset": 0}),
                &ctx(dir.path().into()),
            )
            .await
            .unwrap();
        assert!(out.content.starts_with('🦀'));
    }

    #[tokio::test]
    async fn byte_offset_past_end_is_invalid_args() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("s"), "short").unwrap();
        let tool = ReadFile { max_bytes: 4096 };
        let err = tool
            .execute(
                json!({"path": "s", "byte_offset": 999}),
                &ctx(dir.path().into()),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn byte_offset_and_offset_are_mutually_exclusive() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("s"), "short").unwrap();
        let tool = ReadFile { max_bytes: 4096 };
        let err = tool
            .execute(
                json!({"path": "s", "offset": 1, "byte_offset": 0}),
                &ctx(dir.path().into()),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn over_cap_multiline_read_truncates_to_whole_lines_with_marker() {
        let dir = tempdir().unwrap();
        let content: String = (0..500)
            .map(|i| format!("line number {i} with padding\n"))
            .collect();
        std::fs::write(dir.path().join("big"), &content).unwrap();
        let tool = ReadFile { max_bytes: 2048 };
        let out = tool
            .execute(json!({"path": "big"}), &ctx(dir.path().into()))
            .await
            .unwrap();
        assert!(out.content.len() <= 2048);
        assert!(
            out.content.starts_with("[lines 1–"),
            "{}",
            &out.content[..40]
        );
        assert!(out
            .content
            .contains("continue with read_file(path: \"big\", offset: "));
    }

    #[tokio::test]
    async fn monster_single_line_falls_to_byte_mode() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("mono"), "x".repeat(50_000)).unwrap();
        let tool = ReadFile { max_bytes: 2048 };
        let out = tool
            .execute(json!({"path": "mono"}), &ctx(dir.path().into()))
            .await
            .unwrap();
        assert!(out.content.len() <= 2048);
        assert!(out.content.contains("byte_offset: "), "{}", out.content);
    }

    #[tokio::test]
    async fn binary_file_is_an_honest_error() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("bin"), [0xFF, 0xFE, 0x00]).unwrap();
        let tool = ReadFile { max_bytes: 4096 };
        let err = tool
            .execute(json!({"path": "bin"}), &ctx(dir.path().into()))
            .await
            .unwrap_err();
        match err {
            ToolError::Failed { message, .. } => assert!(message.contains("not valid UTF-8")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_directory_lists_entries() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("x.txt"), "").unwrap();
        let out = ListDirectory
            .execute(json!({"path":"."}), &ctx(dir.path().into()))
            .await
            .unwrap();
        assert!(out.content.contains("x.txt"));
    }
}
