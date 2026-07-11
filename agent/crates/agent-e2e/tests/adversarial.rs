//! Scenarios 18-19 (Task 17): adversarial tampering + a deleted workspace.
//! Spec: docs/superpowers/specs/2026-07-10-e2e-lifecycle-stress-design.md §5d.
//!
//! s18 sweeps five corruption kinds against a fresh park each, asserting BOTH
//! scan surfaces refuse cleanly (CLI `sessions reopen` and a fresh `Session`
//! attach — the same two surfaces `resume.rs`'s `scan_parked_session` /
//! `scan_parked` back). s19 is the workspace-gone refusal.
use agent_e2e::forge;
use agent_e2e::rig::{ckpt, wait_until, wait_until_async, Rig};
use agent_e2e::stub::{gated_write, text_step, ScriptedStub};
use agent_server::testkit::{plant_parked_session, wait_for_ask_id};
use agent_server::wire::ServerEvent;
use std::time::Duration;

const CAP: Duration = Duration::from_secs(30);

/// The five tamper kinds a parked checkpoint tree can suffer, per spec §5d
/// scenario 18 (merges old #14/#15/#16).
#[derive(Debug, Clone, Copy)]
enum Corruption {
    /// Mid-file byte flip on `checkpoint/parked.json` — breaks the payload's
    /// recorded sha256 in the manifest (`{rel} hash mismatch`).
    ParkedBytes,
    /// Mid-file byte flip on `checkpoint/manifest.json` — breaks the
    /// manifest's own HMAC (`HMAC mismatch`).
    ManifestBytes,
    /// A validly-shaped, correctly-formatted `answer.json` whose MAC the
    /// real formula never produced (forge::forged_answer).
    ForgedAnswer,
    /// A pre-versioning-format `answer.json` (no `feedback` field; mac
    /// computed by no recognized formula) — must fail the SAME as a forgery.
    LegacyAnswer,
    /// The session dir's `descriptor.json` is missing entirely — the
    /// session cannot be found by id at all (`scan_descriptors` silently
    /// skips unreadable/absent descriptors).
    NoDescriptor,
}

impl Corruption {
    fn all() -> [Corruption; 5] {
        [
            Corruption::ParkedBytes,
            Corruption::ManifestBytes,
            Corruption::ForgedAnswer,
            Corruption::LegacyAnswer,
            Corruption::NoDescriptor,
        ]
    }
}

/// Scenario 18: the corruption sweep. Fresh `Rig` + fresh park per kind (full
/// isolation beats shared-state cleverness); each iteration applies its
/// tamper, then checks both scan surfaces refuse cleanly. See the per-kind
/// doc comments below (and the task-17 report) for the exact characterized
/// behavior of each kind on each surface.
#[tokio::test(flavor = "multi_thread")]
async fn s18_corruption_sweep() {
    for kind in Corruption::all() {
        run_one_corruption(kind).await;
    }
}

/// Parks one fresh `SQRL-18 go` gated write on its own `Rig`/stub and
/// returns everything a caller needs to corrupt + inspect its tree.
async fn park_fresh() -> (ScriptedStub, Rig, String, std::path::PathBuf) {
    let stub = ScriptedStub::start(vec![gated_write("SQRL-18 go"), text_step(None, "done")]).await;
    let rig = Rig::new();
    let (session, _cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-18 go".into());
    assert!(wait_until_async(CAP, || !rig.session_dirs().is_empty()).await);
    let dir = rig.only_session_dir();
    assert!(wait_until_async(CAP, || ckpt(&dir).join("parked.json").exists()).await);
    drop(session);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();
    (stub, rig, sid, dir)
}

fn apply_corruption(kind: Corruption, rig: &Rig, dir: &std::path::Path) {
    match kind {
        Corruption::ParkedBytes => forge::flip_byte(&ckpt(dir).join("parked.json")),
        Corruption::ManifestBytes => forge::flip_byte(&ckpt(dir).join("manifest.json")),
        Corruption::ForgedAnswer => forge::forged_answer(&ckpt(dir), &rig.key, true),
        Corruption::LegacyAnswer => forge::legacy_answer(&ckpt(dir), true),
        Corruption::NoDescriptor => {
            std::fs::remove_file(dir.join("descriptor.json")).unwrap();
        }
    }
}

/// Each surface gets its OWN independently-parked-and-corrupted tree
/// (full isolation, per the brief): surface (a)'s CLI reopen either
/// completes a run to completion (reaping the whole checkpoint tree on
/// success, for the ForgedAnswer/LegacyAnswer kinds) or refuses read-only,
/// and surface (b)'s Session-attach re-ask likewise mutates its own tree
/// (re-parking under a fresh ask) — sharing one tree between the two
/// surfaces would make whichever runs second observe the FIRST surface's
/// aftermath instead of the corruption itself.
async fn run_one_corruption(kind: Corruption) {
    // --- Surface (a): CLI `sessions reopen` ---
    {
        let (stub, rig, sid, dir) = park_fresh().await;
        apply_corruption(kind, &rig, &dir);
        surface_cli_reopen(&rig, &stub, kind, &sid, &dir).await;
        let _ = stub.recorded(); // spare by design in every corrupt-refusal path
    }

    // --- Surface (b): fresh Session attach ---
    {
        let (stub, rig, sid, dir) = park_fresh().await;
        // ParkedBytes gets a companion healthy park on the SAME rig
        // (positive control): the healthy PRIOR session must still
        // re-emit in the identical ParkedRuns snapshot the corrupt one
        // appears (marked unresumable) in — turning "not usably parked"
        // into a positive comparison, not just an absence.
        let healthy_id = "999-healthy0";
        if matches!(kind, Corruption::ParkedBytes) {
            let (hsess, _hcap, _hkey) = plant_parked_session(
                rig.workspace.path(),
                rig.sessions.path(),
                rig.meta.path(),
                healthy_id,
            )
            .await;
            drop(hsess);
        }
        apply_corruption(kind, &rig, &dir);
        surface_session_attach(&rig, kind, &sid, healthy_id).await;
        let _ = stub.recorded(); // spare: this surface never drives the CLI/stub
    }
}

/// Surface (a): drive `agent sessions reopen <sid>` against the tampered
/// tree and characterize its refusal per kind.
async fn surface_cli_reopen(
    rig: &Rig,
    stub: &ScriptedStub,
    kind: Corruption,
    sid: &str,
    dir: &std::path::Path,
) {
    use agent_e2e::cli::CliCmd;

    let mut cli = CliCmd::new(rig, &stub.base_url())
        .sessions_sub(&["sessions", "reopen", sid])
        .spawn();

    match kind {
        // Torn/tampered checkpoint payload: `scan_parked_session`'s walk
        // surfaces a Corrupt error in `parked.errors`, and
        // `run_sessions_reopen` refuses BEFORE claiming the resume lock or
        // prompting — mirrors s10_torn_checkpoint_refused_list_unaffected.
        Corruption::ParkedBytes | Corruption::ManifestBytes => {
            let st = cli.wait_exit(CAP);
            let t = cli.transcript();
            assert!(!st.success(), "corrupt checkpoint must be refused:\n{t}");
            assert!(
                t.contains("corrupt"),
                "expected the corrupt-checkpoint refusal text, got:\n{t}"
            );
            assert!(!t.contains("panicked"), "must be a clean refusal:\n{t}");
        }
        // The checkpoint tree itself is intact and `answer.json` EXISTS on
        // disk, so `run_sessions_reopen`'s own `parked.asks.iter().find(|a|
        // !a.answered)` finds nothing unanswered — its "outer" re-derived
        // prompt (`prompt_for_answer_with_reader`, y/n/a + a feedback
        // follow-up) never fires at all for this ask. `answer_opt` falls to
        // `take_answer(&parked.root_dir, &key)`, which rejects the
        // forged/legacy MAC (fail-closed ⇒ None) and CONSUMES (deletes) the
        // file regardless. The resumed loop then hits the SAME call with
        // `parked_decision: None` and gates it LIVE via `gate_tool`, which
        // for the CLI's OWN approval channel (`TerminalApproval::with_park_
        // exit`, wired to `stdin_prompt`) is the plain y/n/a prompt with NO
        // feedback follow-up (that follow-up only exists on the outer,
        // reopen-specific prompt function). Simplest deterministic path
        // (documented, per the brief): answer "n" and let the run finish
        // (the deny rejects only this call; the script's next step still
        // completes the turn).
        Corruption::ForgedAnswer | Corruption::LegacyAnswer => {
            cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
            cli.write_line("n");
            let st = cli.wait_exit(CAP);
            let t = cli.transcript();
            assert!(st.success(), "deny-then-complete must exit clean:\n{t}");
            assert!(
                !t.contains("panicked"),
                "must be a clean re-prompt path:\n{t}"
            );
            assert!(
                !ckpt(dir).join("answer.json").exists(),
                "the tampered answer must be consumed (fail-closed), not left behind"
            );
        }
        // scan_descriptors silently skips a session dir with no readable
        // descriptor.json — the CLI's own descriptor lookup (run_sessions_
        // reopen's `find(|d| d.session_id == session_id)`) never sees this
        // session at all, so it reports plain "not found", not a corruption
        // message — characterized here, not assumed.
        Corruption::NoDescriptor => {
            let st = cli.wait_exit(CAP);
            let t = cli.transcript();
            assert!(!st.success(), "missing descriptor must refuse:\n{t}");
            assert!(
                t.contains("not found"),
                "expected the not-found refusal (descriptor-less session is invisible to scan_descriptors), got:\n{t}"
            );
            assert!(!t.contains("panicked"), "must be a clean refusal:\n{t}");
        }
    }
    // Positive-artifact rule: the session dir (and its surviving payload)
    // must be left alone — every refusal above is read-only.
    assert!(dir.is_dir(), "session dir must survive a refused reopen");
}

/// Surface (b): attach a fresh `Session` over the SAME rig roots (the
/// restart-path re-emit, `spawn_parked_reemit`) and characterize what lands
/// on its captured sink for the tampered PRIOR session.
async fn surface_session_attach(rig: &Rig, kind: Corruption, sid: &str, healthy_id: &str) {
    let (fresh, cap) = rig.session("http://127.0.0.1:1"); // unreachable: no model needed for gating
    drop(fresh); // set_event_out already fired the reemit scan; keep cap only

    match kind {
        // Corrupt payload: `scan_parked_session`'s walk records an error in
        // `parked.errors`, but `spawn_parked_reemit`'s `ParkedRunDto` map
        // does NOT filter errored sessions out of the `ParkedRuns` snapshot
        // — it includes them WITH `asks: 0` (no answerable ask: the `errors`
        // check happens before any ask is counted) and `error: Some(..)`
        // (characterized here, not assumed: it is NOT excluded from the
        // list, just marked unresumable within it). Separately, a plain
        // `Error` frame is ALSO emitted for it. The companion healthy park
        // (planted above) is the positive control in the SAME snapshot:
        // `asks: 1`, `error: None`.
        Corruption::ParkedBytes => {
            assert!(
                wait_until_async(CAP, || {
                    cap.snapshot().iter().any(|e| matches!(e,
                        ServerEvent::ParkedRuns { runs } if runs.iter().any(|r| r.session_id == healthy_id)
                    ))
                })
                .await,
                "healthy PRIOR park must still be re-emitted; captured: {:#?}",
                cap.snapshot()
            );
            let runs_snapshot = cap.snapshot().into_iter().find_map(|e| match e {
                ServerEvent::ParkedRuns { runs } => Some(runs),
                _ => None,
            });
            let runs = runs_snapshot.expect("a ParkedRuns snapshot must have landed by now");
            let healthy = runs
                .iter()
                .find(|r| r.session_id == healthy_id)
                .expect("healthy control must be present");
            assert_eq!(healthy.error, None, "healthy control must carry no error");
            assert!(
                healthy.asks > 0,
                "healthy control must show an answerable ask"
            );
            let corrupt = runs
                .iter()
                .find(|r| r.session_id == sid)
                .expect("corrupt session is listed (marked unresumable), not hidden");
            assert_eq!(
                corrupt.asks, 0,
                "a corrupt tree resolves to zero answerable asks"
            );
            assert!(
                corrupt.error.as_deref().is_some_and(|e| e.contains("corrupt")),
                "corrupt session's ParkedRunDto must carry the corrupt-checkpoint error: {corrupt:?}"
            );
            assert!(
                wait_until(CAP, || {
                    cap.snapshot().iter().any(|e| matches!(e,
                        ServerEvent::Error { message } if message.contains("checkpoint unreadable")
                    ))
                }),
                "expected a checkpoint-unreadable Error event; captured: {:#?}",
                cap.snapshot()
            );
        }
        Corruption::ManifestBytes => {
            assert!(
                wait_until(CAP, || {
                    cap.snapshot().iter().any(|e| matches!(e,
                        ServerEvent::Error { message } if message.contains("checkpoint unreadable")
                    ))
                }),
                "expected a checkpoint-unreadable Error event; captured: {:#?}",
                cap.snapshot()
            );
            let runs_snapshot = cap.snapshot().into_iter().find_map(|e| match e {
                ServerEvent::ParkedRuns { runs } => Some(runs),
                _ => None,
            });
            if let Some(runs) = runs_snapshot {
                if let Some(r) = runs.iter().find(|r| r.session_id == sid) {
                    assert_eq!(r.asks, 0, "a corrupt tree resolves to zero answerable asks");
                    assert!(
                        r.error.as_deref().is_some_and(|e| e.contains("corrupt")),
                        "expected the corrupt-checkpoint error on this session's DTO: {r:?}"
                    );
                }
            }
        }
        // The checkpoint loads fine and `ask.answered` is true, so
        // `wire_parked_session` calls `start_resume` directly (no external
        // pre-registered ask) — inside, `take_answer` fails the forged/
        // legacy MAC (None), and the resumed loop's gate falls through to a
        // LIVE re-ask at the SAME call: a brand-new `ApprovalRequest` lands
        // on this fresh Session's sink (gating happens before any model
        // call, so the unreachable base_url above is irrelevant here).
        Corruption::ForgedAnswer | Corruption::LegacyAnswer => {
            let ask_id = wait_for_ask_id(&cap, CAP).await;
            assert!(!ask_id.is_empty());
            assert!(
                !cap.snapshot().iter().any(|e| matches!(e, ServerEvent::Error { .. })),
                "the tampered answer must be discarded and re-asked, not surfaced as an error: {:#?}",
                cap.snapshot()
            );
        }
        // No descriptor ⇒ `scan_descriptors` silently skips the dir; the
        // session never appears anywhere in the restart-path scan at all —
        // no Error, no ParkedRuns entry, just silence. Bounded observation
        // window as a pure absence check is acceptable here per the brief
        // (this kind has no healthy-control pairing requirement).
        Corruption::NoDescriptor => {
            tokio::time::sleep(Duration::from_millis(300)).await;
            assert!(
                !cap.snapshot().iter().any(|e| matches!(e,
                    ServerEvent::ParkedRuns { runs } if runs.iter().any(|r| r.session_id == sid)
                )),
                "a descriptor-less session must never surface in ParkedRuns: {:#?}",
                cap.snapshot()
            );
        }
    }
}

/// Scenario 19: the session's workspace directory is deleted out from under
/// it while parked. `run_sessions_reopen` (agent-cli/src/main.rs) validates
/// `descriptor.workspace.is_dir()` BEFORE ever touching the checkpoint, and
/// refuses with a message naming the workspace — parks are explicitly
/// retained ("parks retained" in the refusal text itself).
#[tokio::test(flavor = "multi_thread")]
async fn s19_workspace_gone() {
    use agent_e2e::cli::CliCmd;

    let stub = ScriptedStub::start(vec![gated_write("SQRL-19 go"), text_step(None, "done")]).await;
    let rig = Rig::new();
    let (session, _cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-19 go".into());
    assert!(wait_until_async(CAP, || !rig.session_dirs().is_empty()).await);
    let dir = rig.only_session_dir();
    assert!(wait_until_async(CAP, || ckpt(&dir).join("parked.json").exists()).await);
    drop(session);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();

    // TempDir::drop tolerates an already-missing dir, so removing the
    // workspace out from under the still-live Rig (whose own TempDir guard
    // still holds the path) is safe cleanup-wise.
    std::fs::remove_dir_all(rig.workspace.path()).unwrap();
    assert!(!rig.workspace.path().is_dir());

    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "reopen", &sid])
        .spawn();
    let st = cli.wait_exit(CAP);
    let t = cli.transcript();
    assert!(
        !st.success(),
        "a deleted workspace must refuse reopen:\n{t}"
    );
    assert!(
        t.contains("workspace no longer exists"),
        "expected the workspace-gone refusal text, got:\n{t}"
    );
    assert!(!t.contains("panicked"), "must be a clean refusal:\n{t}");
    assert!(
        dir.is_dir(),
        "the session dir (and its retained park) must survive a workspace-gone refusal"
    );
    assert!(
        ckpt(&dir).join("parked.json").exists(),
        "parks are explicitly retained on a workspace-gone refusal"
    );
    let _ = stub.recorded(); // spare: refused before any resume ever starts
}
