//! Shared agent loop wiring (tool registry, protocol picker, command lists)
//! used by both the CLI (`agent-cli`) and the daemon (`agent-server`).

mod runtime_config;
pub use runtime_config::{ModelRef, RuntimeConfig, HARD_FLOOR_DENYLIST};

mod assemble;
pub use assemble::{assemble_loop, loop_config_from, BuiltLoop, LoopParts};

pub mod prompts;
pub use prompts::BASE_SYSTEM_PROMPT;

pub mod trace;
pub use trace::{build_trace, ObservedSink, TraceWriter};

pub mod eval;

use agent_http::{FetchUrl, NetworkPolicy};
use agent_mcp::McpServersConfig;
use agent_memory::{build_tools, build_tools_and_retriever, MemoryConfig};
use agent_model::{
    ClaudeCliClient, ModelClient, NativeProtocol, OpenAiCompatClient, PromptedJsonProtocol,
    ToolCallProtocol,
};
use agent_sandbox::{validate_mount, DockerSandbox, SandboxPolicy};
use agent_skills::{CreateSkill, ListSkills, ReadSkillFile, SkillRegistry, UseSkill};
use agent_tools::fs::{EditFile, ListDirectory, ReadFile, WriteFile};
use agent_tools::{
    git::{GitCommit, GitDiff, GitStatus},
    shell::ExecuteCommand,
    ToolRegistry,
};
use agent_tools::{HostExecutor, Limits, Mode, RenderArtifact, SandboxStrategy, Tool};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub use agent_mcp::{McpManager, ServerStatus};

/// Load `mcp.json` at `path` and connect its servers. A missing file yields an
/// empty manager (MCP disabled); a malformed file warns and yields empty. The
/// returned `McpManager` owns the server processes — keep it alive for the session.
pub async fn connect_mcp(
    path: &Path,
    workspace: &Path,
    sandbox: Arc<dyn SandboxStrategy>,
) -> McpManager {
    let (cfg, warning) = McpServersConfig::load_or_empty(path);
    if let Some(w) = warning {
        eprintln!("warning: {} ({}); MCP disabled", w, path.display());
    }
    McpManager::connect(
        &cfg,
        Duration::from_secs(15),
        workspace.to_path_buf(),
        sandbox,
    )
    .await
}

pub fn protocol_name_is_valid(name: &str) -> bool {
    matches!(name, "native" | "prompted")
}

pub fn pick_protocol(name: &str) -> Arc<dyn ToolCallProtocol> {
    match name {
        "prompted" => Arc::new(PromptedJsonProtocol),
        _ => Arc::new(NativeProtocol),
    }
}

pub fn backend_name_is_valid(name: &str) -> bool {
    matches!(name, "openai" | "claude-cli")
}

/// Build the model client for the selected backend.
/// `claude-cli` ignores `base_url`/`api_key`; `openai` ignores `claude_binary`.
pub fn build_model(
    backend: &str,
    base_url: &str,
    model: &str,
    claude_binary: &str,
    api_key: Option<String>,
) -> Arc<dyn ModelClient> {
    match backend {
        "claude-cli" => Arc::new(ClaudeCliClient::new(claude_binary, model)),
        _ => Arc::new(OpenAiCompatClient::new(
            base_url.to_string(),
            model.to_string(),
            api_key,
        )),
    }
}

/// Build a routed model client from a partial `ModelRef`, inheriting every
/// unset field from the primary config (spec G1).
pub fn build_routed_model(
    cfg: &RuntimeConfig,
    r: &ModelRef,
    claude_binary: &str,
    api_key: Option<String>,
) -> Arc<dyn ModelClient> {
    let (backend, base_url, model, bin) = r.resolve(cfg, claude_binary);
    build_model(&backend, &base_url, &model, &bin, api_key)
}

pub fn build_registry(http_allow_hosts: &[String]) -> ToolRegistry {
    let mut r = ToolRegistry::new();
    r.register(Arc::new(ReadFile));
    r.register(Arc::new(WriteFile));
    r.register(Arc::new(EditFile));
    r.register(Arc::new(ListDirectory));
    r.register(Arc::new(ExecuteCommand));
    r.register(Arc::new(GitStatus));
    r.register(Arc::new(GitDiff));
    r.register(Arc::new(GitCommit));
    r.register(Arc::new(RenderArtifact));
    r.register(Arc::new(FetchUrl::new(NetworkPolicy::new(
        http_allow_hosts,
    ))));
    r
}

/// Build the shared skill registry (from `--skills-dir`, or defaults) and the four
/// skill tools that wrap it. Register the returned tools into the `ToolRegistry`,
/// and use the returned `SkillRegistry` to compose preset bodies into the system prompt.
pub fn build_skills(
    skills_dirs: &[String],
    workspace: &Path,
) -> (Arc<SkillRegistry>, Vec<Arc<dyn Tool>>) {
    let registry = Arc::new(SkillRegistry::from_config(skills_dirs, workspace));
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(ListSkills::new(registry.clone())),
        Arc::new(UseSkill::new(registry.clone())),
        Arc::new(ReadSkillFile::new(registry.clone())),
        Arc::new(CreateSkill::new(registry.clone())),
    ];
    (registry, tools)
}

/// Build the three memory tools, or an empty vec when disabled or when construction fails
/// (model unavailable, DB unopenable). Memory is best-effort: failure disables it, never aborts.
pub fn build_memory(
    enabled: bool,
    db_path: Option<PathBuf>,
    model_dir: Option<PathBuf>,
    workspace: &Path,
) -> Vec<Arc<dyn Tool>> {
    if !enabled {
        return Vec::new();
    }
    let mut cfg = MemoryConfig::default();
    if let Some(p) = db_path {
        cfg.db_path = p;
    }
    cfg.model_cache_dir = model_dir;
    match build_tools(cfg, workspace) {
        Ok(tools) => tools,
        Err(e) => {
            tracing::warn!(target: "memory", "disabled: {e}");
            Vec::new()
        }
    }
}

/// Result of building memory with auto-retrieval: the tools to register, an
/// optional retriever to attach to the loop, and the recall-block token budget.
pub struct MemoryBuild {
    pub tools: Vec<Arc<dyn Tool>>,
    pub retriever: Option<Arc<dyn agent_core::Retriever>>,
    pub recall_token_budget: usize,
}

/// Build memory tools AND an auto-retrieval retriever sharing the same store/embedder.
/// Disabled, `auto_recall = false`, or a build failure all yield `retriever: None`
/// (memory is best-effort — never fatal). `recall_token_budget` always reflects config.
pub fn build_memory_full(
    enabled: bool,
    db_path: Option<PathBuf>,
    model_dir: Option<PathBuf>,
    workspace: &Path,
) -> MemoryBuild {
    let mut cfg = MemoryConfig::default();
    if let Some(p) = db_path {
        cfg.db_path = p;
    }
    cfg.model_cache_dir = model_dir;
    let recall_token_budget = cfg.recall_token_budget;
    let auto_recall = cfg.auto_recall;

    if !enabled {
        return MemoryBuild {
            tools: Vec::new(),
            retriever: None,
            recall_token_budget,
        };
    }
    match build_tools_and_retriever(cfg, workspace) {
        Ok((tools, retriever)) => MemoryBuild {
            tools,
            retriever: if auto_recall { Some(retriever) } else { None },
            recall_token_budget,
        },
        Err(e) => {
            tracing::warn!(target: "memory", "disabled: {e}");
            MemoryBuild {
                tools: Vec::new(),
                retriever: None,
                recall_token_budget,
            }
        }
    }
}

pub fn default_allowlist() -> Vec<String> {
    // Exec-capable programs (`git`, `cargo`) are exposed as subcommand prefixes only —
    // a bare entry would auto-allow destructive forms (`git push --force`,
    // `git reset --hard`, `git clean -fdx`). Unknown subcommands fail safe to Ask.
    // The cargo set still runs build scripts: the documented exec-vehicle residual.
    // Users may add a bare "git"/"cargo" entry back in command_allowlist to opt out.
    //
    // CLOSED (2026-07-02): `git {log,diff,show} --output=<path>`/`-o` now falls to
    // Ask via the arg-scan in `agent-policy::is_auto_allowed` (see
    // 2026-07-02-git-output-argscan spec).
    [
        "ls",
        "cat",
        "pwd",
        "echo",
        "grep",
        "find",
        "rg",
        "head",
        "tail",
        "wc",
        "git status",
        "git log",
        "git diff",
        "git show",
        "git blame",
        "git rev-parse",
        "git ls-files",
        "cargo build",
        "cargo check",
        "cargo test",
        "cargo fmt",
        "cargo clippy",
        "cargo metadata",
        "cargo tree",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}
pub fn default_denylist() -> Vec<String> {
    // Bare program-name catastrophes (sudo/mkfs) are handled position-aware in agent-policy;
    // keeping them here as substrings would re-introduce the `man mkfs` false positive on the CLI.
    // Mirrors HARD_FLOOR_DENYLIST.
    ["rm -rf /", ":(){", "dd if="]
        .into_iter()
        .map(String::from)
        .collect()
}

/// Map a `RuntimeConfig` to a `SandboxStrategy`:
/// - `sandbox_mode == "off"` → `HostExecutor` (no Docker overhead).
/// - `"enforce"` → `DockerSandbox` in `Mode::Enforce` (fails if Docker absent).
/// - anything else (e.g. `"auto"`) → `DockerSandbox` in `Mode::Auto` (degrades to host).
///
/// Pick the sandbox image: the built-in default falls back to
/// [`FALLBACK_SANDBOX_IMAGE`] when it hasn't been built locally; an image the
/// user configured explicitly is NEVER substituted (a missing explicit image
/// stays a launch-time `docker run` error).
fn resolve_sandbox_image(configured: &str, image_exists: impl Fn(&str) -> bool) -> String {
    use crate::runtime_config::{DEFAULT_SANDBOX_IMAGE, FALLBACK_SANDBOX_IMAGE};
    if configured == DEFAULT_SANDBOX_IMAGE && !image_exists(configured) {
        tracing::warn!(target: "sandbox",
            "default sandbox image {DEFAULT_SANDBOX_IMAGE} not found locally; \
             falling back to {FALLBACK_SANDBOX_IMAGE} — build the dev image with \
             sandbox-image/build.sh");
        return FALLBACK_SANDBOX_IMAGE.to_string();
    }
    configured.to_string()
}

/// Invalid mount paths in `sandbox_extra_rw`/`sandbox_extra_ro` are dropped with a
/// `tracing::warn!` rather than panicking.
pub fn build_sandbox(cfg: &RuntimeConfig) -> Arc<dyn SandboxStrategy> {
    let mode = match cfg.sandbox_mode.as_str() {
        "off" => return Arc::new(HostExecutor),
        "enforce" => Mode::Enforce,
        _ => Mode::Auto,
    };

    let home = dirs_home();
    let resolve = |list: &[String]| {
        list.iter()
            .filter_map(|p| match validate_mount(p, home.as_deref()) {
                Ok(c) => Some(c),
                Err(e) => {
                    tracing::warn!(target: "sandbox", "dropping mount {p}: {e}");
                    None
                }
            })
            .collect::<Vec<_>>()
    };

    let policy = SandboxPolicy {
        mode,
        image: resolve_sandbox_image(&cfg.sandbox_image, DockerSandbox::image_exists),
        network: cfg.sandbox_network,
        limits: Limits {
            memory: cfg.sandbox_memory.clone(),
            cpus: cfg.sandbox_cpus.clone(),
            pids: cfg.sandbox_pids,
            fsize: cfg.sandbox_fsize.clone(),
            tmp_size: cfg.sandbox_tmp_size.clone(),
        },
        extra_rw: resolve(&cfg.sandbox_extra_rw),
        extra_ro: resolve(&cfg.sandbox_extra_ro),
    };
    let uid_gid = current_uid_gid();
    Arc::new(DockerSandbox::new(policy, uid_gid, DockerSandbox::probe()))
}

/// Return `"uid:gid"` of the current process on Unix. On any failure (or on
/// non-Unix), fall back to nobody (`65534:65534`) — NEVER `0:0`, which would
/// run container workloads as root.
fn current_uid_gid() -> String {
    #[cfg(unix)]
    {
        fn id_part(flag: &str) -> Option<String> {
            let out = std::process::Command::new("id").arg(flag).output().ok()?;
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            (!s.is_empty() && out.status.success()).then_some(s)
        }
        uid_gid_or_nobody(id_part("-u"), id_part("-g"))
    }
    #[cfg(not(unix))]
    {
        "65534:65534".into()
    }
}

/// Pure fallback logic, unit-tested: any missing part degrades BOTH to nobody.
fn uid_gid_or_nobody(uid: Option<String>, gid: Option<String>) -> String {
    match (uid, gid) {
        (Some(u), Some(g)) => format!("{u}:{g}"),
        _ => {
            tracing::warn!(target: "sandbox",
                "could not determine uid/gid via `id`; container will run as nobody (65534:65534)");
            "65534:65534".into()
        }
    }
}

/// Return the user's home directory from `$HOME`, if set.
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_config::{ModelRef, RuntimeConfig};

    #[test]
    fn build_routed_model_inherits_primary_for_none_fields() {
        let cfg = RuntimeConfig::from_launch(
            "claude-cli".into(),
            "http://x".into(),
            "opus".into(),
            "native".into(),
            8192,
        );
        // model-only override on the primary backend: constructs a claude-cli client.
        let r = ModelRef {
            model: Some("haiku".into()),
            ..ModelRef::default()
        };
        let _m = build_routed_model(&cfg, &r, "claude", None);
        // backend override to openai: constructs without touching claude_binary.
        let r2 = ModelRef {
            backend: Some("openai".into()),
            base_url: Some("http://127.0.0.1:1".into()),
            model: Some("qwen-mini".into()),
            ..ModelRef::default()
        };
        let _m2 = build_routed_model(&cfg, &r2, "claude", None);
        // Construction is the contract here (build_model is already the tested seam);
        // resolution correctness is pinned by resolve() below.
        let (be, url, model, bin) = r2.resolve(&cfg, "claude");
        assert_eq!((be.as_str(), model.as_str()), ("openai", "qwen-mini"));
        assert_eq!(url, "http://127.0.0.1:1");
        assert_eq!(bin, "claude");
        let (be1, _, model1, _) = r.resolve(&cfg, "claude");
        assert_eq!((be1.as_str(), model1.as_str()), ("claude-cli", "haiku"));
    }

    fn base_cfg() -> RuntimeConfig {
        RuntimeConfig::from_launch(
            "openai".into(),
            "http://localhost:8080".into(),
            "m1".into(),
            "native".into(),
            8192,
        )
    }

    #[test]
    fn uid_gid_fallback_is_nobody_never_root() {
        assert_eq!(uid_gid_or_nobody(None, None), "65534:65534");
        assert_eq!(uid_gid_or_nobody(Some("1000".into()), None), "65534:65534");
        assert_eq!(uid_gid_or_nobody(None, Some("1000".into())), "65534:65534");
        assert_eq!(
            uid_gid_or_nobody(Some("1000".into()), Some("1000".into())),
            "1000:1000"
        );
    }

    #[test]
    fn build_sandbox_off_is_host() {
        let mut cfg = base_cfg();
        cfg.sandbox_mode = "off".into();
        assert_eq!(build_sandbox(&cfg).describe().mechanism, "host");
    }

    #[test]
    fn build_sandbox_auto_is_docker_descriptor() {
        let mut cfg = base_cfg();
        cfg.sandbox_mode = "auto".into();
        // Explicit image: hermetic — resolve_sandbox_image never probes Docker
        // for a non-default name.
        cfg.sandbox_image = "explicit-img:1".into();
        let d = build_sandbox(&cfg).describe();
        assert_eq!(d.mechanism, "docker");
        assert_eq!(d.image.as_deref(), Some("explicit-img:1"));
    }

    #[test]
    fn build_registry_includes_render() {
        let r = build_registry(&[]);
        assert!(r.get("render").is_some(), "render tool must be registered");
    }

    #[test]
    fn backend_validation() {
        assert!(backend_name_is_valid("openai"));
        assert!(backend_name_is_valid("claude-cli"));
        assert!(!backend_name_is_valid("bogus"));
    }
    #[test]
    fn pick_protocol_selects_by_name() {
        assert!(protocol_name_is_valid("native"));
        assert!(protocol_name_is_valid("prompted"));
        assert!(!protocol_name_is_valid("bogus"));
    }
    #[test]
    fn registry_has_all_core_tools() {
        let r = build_registry(&[]);
        for name in [
            "read_file",
            "write_file",
            "edit_file",
            "list_directory",
            "execute_command",
            "git_status",
            "git_diff",
            "git_commit",
            "fetch_url",
        ] {
            assert!(r.get(name).is_some(), "missing {name}");
        }
    }
    #[test]
    fn build_skills_returns_four_tools() {
        let (_reg, tools) = build_skills(&[], std::path::Path::new("/tmp/ws"));
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        for expected in [
            "list_skills",
            "use_skill",
            "read_skill_file",
            "create_skill",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }
    }

    #[test]
    fn build_memory_disabled_returns_no_tools() {
        let tools = build_memory(false, None, None, std::path::Path::new("/tmp/ws"));
        assert!(tools.is_empty());
    }

    #[test]
    #[ignore = "constructs the real embedding model (network/model download)"]
    fn build_memory_enabled_returns_three_tools() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("memory.db");
        let cache = tmp.path().join("models");
        let tools = build_memory(true, Some(db), Some(cache), tmp.path());
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        for n in ["remember", "recall", "forget"] {
            assert!(names.contains(&n), "missing {n}");
        }
    }

    #[test]
    fn build_memory_full_disabled_has_no_retriever() {
        let mb = build_memory_full(false, None, None, std::path::Path::new("/tmp/ws"));
        assert!(mb.tools.is_empty());
        assert!(mb.retriever.is_none());
        assert_eq!(mb.recall_token_budget, 512);
    }

    #[test]
    fn resolve_sandbox_image_falls_back_only_for_missing_default() {
        use crate::runtime_config::{DEFAULT_SANDBOX_IMAGE, FALLBACK_SANDBOX_IMAGE};
        // default + present locally → default
        assert_eq!(
            resolve_sandbox_image(DEFAULT_SANDBOX_IMAGE, |_| true),
            DEFAULT_SANDBOX_IMAGE
        );
        // default + missing → fallback
        assert_eq!(
            resolve_sandbox_image(DEFAULT_SANDBOX_IMAGE, |_| false),
            FALLBACK_SANDBOX_IMAGE
        );
        // explicit + missing → kept verbatim (never silently substituted)
        assert_eq!(resolve_sandbox_image("my-img:1", |_| false), "my-img:1");
        // explicit + present → kept, and the probe must not even run
        assert_eq!(
            resolve_sandbox_image("my-img:1", |_| panic!("explicit image must not probe")),
            "my-img:1"
        );
    }

    #[test]
    #[ignore = "constructs the real embedding model (network/model download)"]
    fn build_memory_full_enabled_has_retriever_and_tools() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("memory.db");
        let mb = build_memory_full(true, Some(db), None, tmp.path());
        assert_eq!(mb.tools.len(), 3);
        assert!(mb.retriever.is_some());
    }
}
