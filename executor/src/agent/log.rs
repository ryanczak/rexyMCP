use crate::security::redact::Redactor;
use crate::store::sessions::event::SessionEvent;
use crate::store::sessions::jsonl::{SessionLogHandle, session_log};

pub(super) fn log_event(
    handle: &Option<SessionLogHandle>,
    redactor: &Redactor,
    clock: &dyn Fn() -> u64,
    turn: usize,
    event: SessionEvent,
) {
    let Some(handle) = handle else {
        return;
    };
    session_log(handle, clock(), turn, redact_event(redactor, event));
}

pub(super) fn log_session_end(
    handle: &Option<SessionLogHandle>,
    redactor: &Redactor,
    clock: &dyn Fn() -> u64,
    status: &str,
    turns: usize,
) {
    log_event(
        handle,
        redactor,
        clock,
        turns,
        SessionEvent::SessionEnd {
            status: status.to_string(),
            turns,
        },
    );
}

/// Round-trip an event through the redactor: serialize → redact the JSON →
/// deserialize. This redacts every string the event carries (prompt, completion,
/// tool output, the nested `ParseFailure` / `ToolCall` payloads) in one pass; the
/// `[REDACTED:<kind>]` markers are JSON-safe, so the parse round-trips. On the
/// can't-happen serde failure, fall back to the un-redacted event's structure
/// only after redaction was attempted — but serialization of these types is
/// effectively infallible, so this is a safety net, not a swallow.
fn redact_event(redactor: &Redactor, event: SessionEvent) -> SessionEvent {
    let Ok(json) = serde_json::to_string(&event) else {
        return event;
    };
    let redacted = redactor.redact(&json);
    serde_json::from_str(&redacted).unwrap_or(event)
}
