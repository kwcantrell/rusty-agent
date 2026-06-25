use agent_server::wire::{EventOut, ServerEvent};
use tauri::ipc::Channel;

/// `EventOut` backed by a Tauri IPC channel. The only Tauri-aware sink; lives
/// here so `agent-server` stays transport-agnostic.
pub struct ChannelOut(pub Channel<ServerEvent>);

impl EventOut for ChannelOut {
    fn send(&self, ev: ServerEvent) {
        let _ = self.0.send(ev); // closed channel → drop (infallible emit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tauri::ipc::{Channel, InvokeResponseBody};

    #[test]
    fn forwards_to_underlying_channel() {
        let seen = Arc::new(Mutex::new(Vec::<ServerEvent>::new()));
        let s2 = seen.clone();
        let ch: Channel<ServerEvent> = Channel::new(move |body: InvokeResponseBody| {
            if let InvokeResponseBody::Json(txt) = body {
                s2.lock().unwrap().push(serde_json::from_str(&txt).unwrap());
            }
            Ok(())
        });
        let out = ChannelOut(ch);
        out.send(ServerEvent::Token { text: "hi".into() });
        assert!(matches!(seen.lock().unwrap().as_slice(),
            [ServerEvent::Token { text }] if text == "hi"));
    }
}
