use crate::sink::EventSlot;
use crate::wire::{ApprovalOriginDto, EventOut, ServerEvent};
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use agent_tools::Display;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

/// Fields for an ask that is minted directly (not blocked on a live loop) via
/// `register_external` — the restart-path re-emit (Task 12).
pub struct ExternalAsk {
    pub summary: String,
    pub command: Option<String>,
    pub display: Option<Display>,
    pub origin: Option<ApprovalOriginDto>,
}

/// `ApprovalChannel` that emits an `ApprovalRequest` over the live-read event
/// slot and awaits an `approve` command matched by correlation id.
///
/// E5 semantics: an unanswered ask **parks indefinitely** by default (its frame
/// is stored and re-sent on the next attach). Only when a `timeout` is configured
/// (the `approval_auto_deny_secs` knob, for headless/eval callers) does an
/// unanswered ask auto-deny. A dropped/aborted await removes its map entries via
/// a `PendingGuard`, so a cancelled caller never leaves a zombie prompt.
pub struct IpcApprovalChannel {
    slot: EventSlot,
    pending: Mutex<HashMap<String, oneshot::Sender<ApprovalResponse>>>,
    /// Frames for still-pending asks, re-sent on (re)attach so a frontend that
    /// missed the original emit can answer (spec §2.4 step 5).
    pending_frames: Mutex<HashMap<String, ServerEvent>>,
    /// External (restart-path) asks tagged by group (the parked session id) so
    /// `retract_external_for` can sweep every id that group minted.
    external_groups: Mutex<HashMap<String, String>>,
    counter: AtomicU64,
    /// E5: None = park indefinitely; Some = auto-deny after the duration.
    timeout: Option<Duration>,
}

impl IpcApprovalChannel {
    pub fn new(slot: EventSlot, timeout: Option<Duration>) -> Self {
        Self {
            slot,
            pending: Mutex::new(HashMap::new()),
            pending_frames: Mutex::new(HashMap::new()),
            external_groups: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(0),
            timeout,
        }
    }

    /// Complete a pending approval, called by the `approve` command. Also clears
    /// the stored frame so a later `reemit_pending` won't re-send an answered ask.
    pub fn resolve(&self, id: &str, decision: ApprovalResponse) {
        self.pending_frames.lock().unwrap().remove(id);
        self.external_groups.lock().unwrap().remove(id);
        if let Some(tx) = self.pending.lock().unwrap().remove(id) {
            let _ = tx.send(decision);
        }
    }

    /// Re-send every still-pending approval frame to a (re)attached frontend —
    /// the daemon-alive reattach path (spec §2.4 step 5): the live pending id is
    /// reused, no second id minted.
    pub fn reemit_pending(&self, out: &Arc<dyn EventOut>) {
        for ev in self.pending_frames.lock().unwrap().values() {
            out.send(ev.clone());
        }
    }

    /// Mint a pending entry for an ask that is NOT blocked on a live loop
    /// (restart-path re-emit; Task 12). `group` tags the entry (the parked
    /// session id) so `retract_external_for` can sweep it. Returns (id, rx).
    /// No timeout by design — restart-path asks park until answered; Task 12 owns
    /// clearing the entry if the session becomes unresumable.
    pub fn register_external(
        &self,
        group: &str,
        ask: ExternalAsk,
    ) -> (String, oneshot::Receiver<ApprovalResponse>) {
        let id = format!("c{}", self.counter.fetch_add(1, Ordering::Relaxed));
        let (otx, orx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), otx);
        self.external_groups
            .lock()
            .unwrap()
            .insert(id.clone(), group.to_string());
        let ev = ServerEvent::ApprovalRequest {
            id: id.clone(),
            summary: ask.summary,
            command: ask.command,
            display: ask.display,
            origin: ask.origin,
        };
        self.pending_frames
            .lock()
            .unwrap()
            .insert(id.clone(), ev.clone());
        if let Some(out) = self.slot.lock().unwrap().clone() {
            out.send(ev);
        }
        (id, orx)
    }

    /// Drop every still-pending external entry tagged `group` (first answer won;
    /// stale prompts must not mint a second resume — finding 12). Dropping the
    /// pending oneshot sender makes any in-flight `rx.await` resolve `Err`, which
    /// the Task-12 waiter's `let Ok(resp) = rx.await else { return }` absorbs.
    pub fn retract_external_for(&self, group: &str) {
        let ids: Vec<String> = {
            let groups = self.external_groups.lock().unwrap();
            groups
                .iter()
                .filter(|(_, g)| g.as_str() == group)
                .map(|(id, _)| id.clone())
                .collect()
        };
        let mut pending = self.pending.lock().unwrap();
        let mut frames = self.pending_frames.lock().unwrap();
        let mut groups = self.external_groups.lock().unwrap();
        for id in ids {
            pending.remove(&id); // dropping the sender resolves waiters' rx with Err
            frames.remove(&id);
            groups.remove(&id);
        }
    }
}

#[async_trait]
impl ApprovalChannel for IpcApprovalChannel {
    async fn request(&self, req: ApprovalRequest) -> ApprovalResponse {
        let id = format!("c{}", self.counter.fetch_add(1, Ordering::Relaxed));
        let (otx, orx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), otx);
        let ev = ServerEvent::ApprovalRequest {
            id: id.clone(),
            summary: req.intent.summary.clone(),
            command: req.intent.command.clone(),
            display: req.display.clone(),
            origin: req.origin.as_ref().map(|o| ApprovalOriginDto {
                delegation_id: o.delegation_id.clone(),
                subagent: o.subagent_name.clone(),
                depth: o.depth,
            }),
        };
        self.pending_frames
            .lock()
            .unwrap()
            .insert(id.clone(), ev.clone());
        // No subscriber ⇒ the ask PARKS instead of denying (E5): the frame is
        // re-sent by reemit_pending on the next attach.
        if let Some(out) = self.slot.lock().unwrap().clone() {
            out.send(ev);
        }
        // Drop-safety (plan review finding 3): the caller may be dropped
        // mid-await (e.g. a dispatch deadline cancelling the child future). The
        // guard removes BOTH map entries on any exit — a dropped await must not
        // leave a zombie prompt that re-emits forever.
        struct PendingGuard<'a>(&'a IpcApprovalChannel, String);
        impl Drop for PendingGuard<'_> {
            fn drop(&mut self) {
                self.0.pending.lock().unwrap().remove(&self.1);
                self.0.pending_frames.lock().unwrap().remove(&self.1);
            }
        }
        let _guard = PendingGuard(self, id.clone());
        match self.timeout {
            Some(t) => match tokio::time::timeout(t, orx).await {
                Ok(Ok(resp)) => resp,
                _ => ApprovalResponse::Deny { feedback: None },
            },
            None => orx
                .await
                .unwrap_or(ApprovalResponse::Deny { feedback: None }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_policy::ApprovalOrigin;
    use agent_tools::{Access, ToolIntent};

    #[derive(Default)]
    struct Captured(Mutex<Vec<ServerEvent>>);
    impl EventOut for Captured {
        fn send(&self, ev: ServerEvent) {
            self.0.lock().unwrap().push(ev);
        }
    }
    impl Captured {
        /// Latest captured `ApprovalRequest` id, or `None` if none seen yet.
        fn last_ask_id(&self) -> Option<String> {
            self.0.lock().unwrap().iter().rev().find_map(|ev| match ev {
                ServerEvent::ApprovalRequest { id, .. } => Some(id.clone()),
                _ => None,
            })
        }
        fn ask_count(&self) -> usize {
            self.0
                .lock()
                .unwrap()
                .iter()
                .filter(|ev| matches!(ev, ServerEvent::ApprovalRequest { .. }))
                .count()
        }
    }
    fn slot_with(out: Arc<Captured>) -> crate::sink::EventSlot {
        Arc::new(Mutex::new(Some(out as Arc<dyn EventOut>)))
    }
    fn req() -> ApprovalRequest {
        ApprovalRequest {
            intent: ToolIntent {
                tool: "execute_command".into(),
                access: Access::Write,
                paths: vec![],
                command: Some("touch x".into()),
                summary: "run touch x".into(),
            },
            display: None,
            origin: None,
        }
    }
    /// Spin the runtime until `cap` has captured an ask, returning its id.
    async fn wait_for_ask(cap: &Arc<Captured>) -> String {
        loop {
            if let Some(id) = cap.last_ask_id() {
                break id;
            }
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn emits_request_and_resolves() {
        let cap = Arc::new(Captured::default());
        let ch = Arc::new(IpcApprovalChannel::new(slot_with(cap.clone()), None));
        let ch2 = ch.clone();
        let h = tokio::spawn(async move { ch2.request(req()).await });
        let id = wait_for_ask(&cap).await;
        ch.resolve(&id, ApprovalResponse::ApproveAlways);
        assert_eq!(h.await.unwrap(), ApprovalResponse::ApproveAlways);
    }

    #[tokio::test(start_paused = true)]
    async fn unanswered_ask_parks_indefinitely_without_knob() {
        // No knob (None) ⇒ an unanswered ask never auto-denies, no matter how
        // far virtual time advances.
        let cap = Arc::new(Captured::default());
        let ch = Arc::new(IpcApprovalChannel::new(slot_with(cap.clone()), None));
        let ch2 = ch.clone();
        let mut h = tokio::spawn(async move { ch2.request(req()).await });
        let id = wait_for_ask(&cap).await;
        // Advance far past the old 300s auto-deny window; the future stays pending.
        tokio::time::advance(Duration::from_secs(10_000)).await;
        tokio::task::yield_now().await;
        assert!(
            futures_poll_pending(&mut h).await,
            "unanswered ask must park, not resolve"
        );
        // A later answer completes it.
        ch.resolve(&id, ApprovalResponse::Approve);
        assert_eq!(h.await.unwrap(), ApprovalResponse::Approve);
    }

    #[tokio::test(start_paused = true)]
    async fn knob_auto_denies_after_n_seconds() {
        // With the knob set, an unanswered ask auto-denies once the window elapses.
        let cap = Arc::new(Captured::default());
        let ch = Arc::new(IpcApprovalChannel::new(
            slot_with(cap.clone()),
            Some(Duration::from_secs(2)),
        ));
        let ch2 = ch.clone();
        let h = tokio::spawn(async move { ch2.request(req()).await });
        wait_for_ask(&cap).await;
        tokio::time::advance(Duration::from_secs(3)).await;
        assert_eq!(h.await.unwrap(), ApprovalResponse::Deny { feedback: None });
        // The pending map is drained on timeout (PendingGuard on drop).
        assert!(ch.pending.lock().unwrap().is_empty());
        assert!(ch.pending_frames.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn no_subscriber_no_longer_denies_it_parks() {
        // Empty slot: TODAY this returned Deny immediately. Post-E5 the ask parks
        // (frame stored), then re-emits under its ORIGINAL id on attach.
        let slot: crate::sink::EventSlot = Arc::new(Mutex::new(None));
        let ch = Arc::new(IpcApprovalChannel::new(slot.clone(), None));
        let ch2 = ch.clone();
        let mut h = tokio::spawn(async move { ch2.request(req()).await });
        // Give the spawned request a chance to mint + park.
        while ch.pending_frames.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
        assert!(
            futures_poll_pending(&mut h).await,
            "with no subscriber the ask must park, not deny"
        );
        let parked_id = ch
            .pending_frames
            .lock()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();

        // Attach a subscriber and re-emit: the SAME id reaches the frontend.
        let cap = Arc::new(Captured::default());
        *slot.lock().unwrap() = Some(cap.clone() as Arc<dyn EventOut>);
        ch.reemit_pending(&(cap.clone() as Arc<dyn EventOut>));
        assert_eq!(cap.last_ask_id().as_deref(), Some(parked_id.as_str()));

        ch.resolve(&parked_id, ApprovalResponse::Approve);
        assert_eq!(h.await.unwrap(), ApprovalResponse::Approve);
    }

    #[tokio::test]
    async fn reemit_covers_daemon_alive_reattach_and_clears_on_resolve() {
        let cap_a = Arc::new(Captured::default());
        let ch = Arc::new(IpcApprovalChannel::new(slot_with(cap_a.clone()), None));
        let ch2 = ch.clone();
        let h = tokio::spawn(async move { ch2.request(req()).await });
        let id = wait_for_ask(&cap_a).await;
        assert_eq!(cap_a.ask_count(), 1, "original emit fires once");

        // Reattach frontend B: reemit_pending re-sends the SAME id.
        let cap_b = Arc::new(Captured::default());
        ch.reemit_pending(&(cap_b.clone() as Arc<dyn EventOut>));
        assert_eq!(cap_b.last_ask_id().as_deref(), Some(id.as_str()));

        // Resolve clears the frame; a later reemit to C sends nothing.
        ch.resolve(&id, ApprovalResponse::Approve);
        assert_eq!(h.await.unwrap(), ApprovalResponse::Approve);
        let cap_c = Arc::new(Captured::default());
        ch.reemit_pending(&(cap_c.clone() as Arc<dyn EventOut>));
        assert_eq!(cap_c.ask_count(), 0, "resolved frame must not re-emit");
    }

    #[tokio::test]
    async fn origin_rides_the_wire_frame() {
        let cap = Arc::new(Captured::default());
        let ch = Arc::new(IpcApprovalChannel::new(slot_with(cap.clone()), None));
        let mut r = req();
        r.origin = Some(ApprovalOrigin {
            delegation_id: "c7".into(),
            subagent_name: "explore".into(),
            depth: 1,
        });
        let ch2 = ch.clone();
        let h = tokio::spawn(async move { ch2.request(r).await });
        wait_for_ask(&cap).await;
        let origin = cap.0.lock().unwrap().iter().find_map(|ev| match ev {
            ServerEvent::ApprovalRequest { origin, .. } => Some(origin.clone()),
            _ => None,
        });
        assert_eq!(
            origin.flatten(),
            Some(ApprovalOriginDto {
                delegation_id: "c7".into(),
                subagent: "explore".into(),
                depth: 1,
            })
        );
        let id = cap.last_ask_id().unwrap();
        ch.resolve(&id, ApprovalResponse::Approve);
        let _ = h.await;
    }

    #[tokio::test]
    async fn dropped_await_leaves_no_zombie_prompt() {
        // A caller aborted mid-await must not leave a pending entry or frame that
        // re-emits forever. After abort, reemit sends nothing and resolve is a no-op.
        let cap = Arc::new(Captured::default());
        let ch = Arc::new(IpcApprovalChannel::new(slot_with(cap.clone()), None));
        let ch2 = ch.clone();
        let h = tokio::spawn(async move { ch2.request(req()).await });
        let id = wait_for_ask(&cap).await;
        h.abort();
        // Let the abort run the PendingGuard drop.
        while !ch.pending.lock().unwrap().is_empty()
            || !ch.pending_frames.lock().unwrap().is_empty()
        {
            tokio::task::yield_now().await;
        }
        let cap2 = Arc::new(Captured::default());
        ch.reemit_pending(&(cap2.clone() as Arc<dyn EventOut>));
        assert_eq!(cap2.ask_count(), 0, "aborted ask must not re-emit");
        ch.resolve(&id, ApprovalResponse::Approve); // must not panic; no-op
    }

    /// Poll a spawned task to completion-check: returns true if it is STILL
    /// pending (not finished). Uses a tiny timeout so we don't block forever.
    async fn futures_poll_pending<T>(h: &mut tokio::task::JoinHandle<T>) -> bool {
        // `now_or_never` semantics via a zero-ish poll: if it hasn't finished
        // after yielding, treat it as pending. tokio::time may be paused, so we
        // use a biased select on a ready future to poll `h` exactly once.
        tokio::select! {
            biased;
            r = &mut *h => {
                // Finished — put a sentinel back is impossible; report not-pending.
                let _ = r;
                false
            }
            _ = std::future::ready(()) => true,
        }
    }
}
