// JSON output emitter for `--json` mode.
//
// Every event is a single JSON object written to stdout followed by a newline
// (NDJSON). `println!`/locked stdout writes are line-atomic, so rayon threads
// in the render pipeline can emit concurrently without interleaving.

use serde::Serialize;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

static JSON_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_json_mode(enabled: bool) {
    JSON_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn is_json() -> bool {
    JSON_ENABLED.load(Ordering::Relaxed)
}

/// Serialize `value` to stdout as a single JSON line. Silently drops write
/// errors (broken pipe on a consumer that stopped reading — we still want the
/// command to run to completion).
pub fn emit(value: &impl Serialize) {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    if serde_json::to_writer(&mut handle, value).is_ok() {
        let _ = handle.write_all(b"\n");
        let _ = handle.flush();
    }
}

/// Convenience wrapper: emit only when `--json` is on.
pub fn emit_if_json(value: &impl Serialize) {
    if is_json() {
        emit(value);
    }
}
