//! Wire format for the daemon RPC: line-delimited JSON.
//!
//! Each line on the socket is one full JSON message. The protocol is
//! pure request/response — no streaming, no notifications. Per spec
//! §10.4 / punchlist CP4:
//!
//! ```text
//! → {"id": 7, "op": "goto", "args": {"url": "https://example.com"}}
//! ← {"id": 7, "ok": true,  "result": null}
//! ← {"id": 7, "ok": false, "error": "navigation timed out"}
//! ```
//!
//! `id` is a per-session monotonic counter (u64). The client checks
//! that the response's id matches the request's; a mismatch is a
//! protocol error and the connection is suspect.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub op: String,
    #[serde(default)]
    pub args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Self {
        Self {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: u64, error: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(error.into()),
        }
    }
}
