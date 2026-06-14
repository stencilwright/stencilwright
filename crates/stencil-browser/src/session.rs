//! Client-side session: a Unix-socket connection to a running daemon.
//!
//! [`Session::connect`] takes an already-known socket path (used in
//! tests and after `daemon::ensure_running`). [`Session::attach`] is
//! the convenience wrapper that auto-starts the daemon — the caller
//! provides the spawner closure because `stencil-browser` doesn't
//! know its host binary.
//!
//! The wire is purely sequential: a single tokio mutex serializes
//! `write request → read response` per `rpc()` call. CLI commands
//! issue 5–10 RPCs sequentially per invocation, so this is plenty —
//! a multiplexer is YAGNI until something proves otherwise.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::Mutex;

use crate::Page;
use crate::daemon;
use crate::rpc::{Request, Response};
use stencil_secrets::{DiscoveredSecretItem, SecretDiscoveryQuery};

/// RPC handle to a daemon. Cheap to clone (Arc inside).
#[derive(Clone)]
pub struct Session {
    pub(crate) inner: Arc<SessionInner>,
}

pub(crate) struct SessionInner {
    chan: Mutex<RpcChannel>,
    next_id: AtomicU64,
}

struct RpcChannel {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl Session {
    /// Connect to a daemon already known to be running at `sock`.
    pub async fn connect(sock: &Path) -> Result<Self> {
        let stream = UnixStream::connect(sock)
            .await
            .with_context(|| format!("connect {}", sock.display()))?;
        let (read, write) = stream.into_split();
        Ok(Self {
            inner: Arc::new(SessionInner {
                chan: Mutex::new(RpcChannel {
                    reader: BufReader::new(read),
                    writer: write,
                }),
                next_id: AtomicU64::new(1),
            }),
        })
    }

    /// Auto-start the daemon if needed, then connect. The spawner
    /// closure is responsible for forking the daemon subprocess; the
    /// CLI typically passes one that re-execs `current_exe` with the
    /// hidden `daemon` subcommand.
    pub async fn attach<F>(site_dir: &Path, spawner: F) -> Result<Self>
    where
        F: FnOnce() -> Result<()>,
    {
        let sock = daemon::ensure_running(site_dir, spawner).await?;
        Self::connect(&sock).await
    }

    /// Get a Page handle for the daemon's single page.
    pub fn page(&self) -> Page {
        Page::new(self.inner.clone())
    }

    pub async fn discover_secrets(
        &self,
        query: &SecretDiscoveryQuery,
        limit: usize,
    ) -> Result<Vec<DiscoveredSecretItem>> {
        if query
            .search
            .as_deref()
            .is_none_or(|search| search.trim().is_empty())
        {
            bail!("secret discovery requires a non-empty search query");
        }
        let v = self
            .inner
            .rpc(
                "secret_discover",
                serde_json::json!({ "query": query, "limit": limit }),
            )
            .await?;
        serde_json::from_value(v).context("parse secret_discover response")
    }

    /// Drop the connection. The daemon notices on EOF and continues
    /// serving other clients.
    pub async fn close(self) -> Result<()> {
        // Dropping `self` drops the Arc; if this was the last clone
        // the channel is closed.
        Ok(())
    }
}

impl SessionInner {
    /// Send one request, await its matched response. Sequential —
    /// only one RPC in flight at a time.
    pub(crate) async fn rpc(&self, op: &str, args: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = Request {
            id,
            op: op.into(),
            args,
        };
        let mut line = serde_json::to_string(&req).context("serialize request")?;
        line.push('\n');

        let mut chan = self.chan.lock().await;
        chan.writer
            .write_all(line.as_bytes())
            .await
            .context("write request")?;
        chan.writer.flush().await.context("flush request")?;

        let mut response_line = String::new();
        let n = chan
            .reader
            .read_line(&mut response_line)
            .await
            .context("read response")?;
        if n == 0 {
            bail!("daemon closed connection");
        }
        let resp: Response = serde_json::from_str(&response_line)
            .with_context(|| format!("parse response: {}", response_line.trim()))?;
        if resp.id != id {
            bail!("rpc id mismatch: expected {id}, got {}", resp.id);
        }
        if resp.ok {
            Ok(resp.result.unwrap_or(Value::Null))
        } else {
            Err(anyhow!(
                "daemon rpc error: {}",
                resp.error.as_deref().unwrap_or("(no message)")
            ))
        }
    }
}
