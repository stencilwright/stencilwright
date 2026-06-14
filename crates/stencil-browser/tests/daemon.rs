//! Integration tests for the public daemon API. Exercises file-on-disk
//! semantics only — the actual daemon body (Playwright + Chrome) is
//! manual-smoke territory.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use stencil_browser::daemon::{DaemonStatus, SessionInfo, live_socket, status, stop};
use tempfile::tempdir;

// A pid that's almost certainly dead — well above macOS PID_MAX.
const DEAD_PID: u32 = 0x7FFF_FFFE;

fn write_session(dir: &Path, info: &SessionInfo) {
    let body = serde_json::to_vec_pretty(info).unwrap();
    fs::write(dir.join(".session"), body).unwrap();
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[test]
fn live_socket_none_when_no_session_file() {
    let tmp = tempdir().unwrap();
    assert!(live_socket(tmp.path()).unwrap().is_none());
}

#[test]
fn live_socket_none_for_stale_pid() {
    let tmp = tempdir().unwrap();
    write_session(
        tmp.path(),
        &SessionInfo {
            pid: DEAD_PID,
            sock: "/tmp/nope.sock".into(),
            started_at: 0,
        },
    );
    assert!(live_socket(tmp.path()).unwrap().is_none());
}

#[test]
fn live_socket_none_when_socket_missing() {
    let tmp = tempdir().unwrap();
    write_session(
        tmp.path(),
        &SessionInfo {
            pid: std::process::id(), // we are alive
            sock: "/tmp/definitely-does-not-exist.sock".into(),
            started_at: 0,
        },
    );
    assert!(live_socket(tmp.path()).unwrap().is_none());
}

#[test]
fn live_socket_some_when_pid_and_socket_present() {
    let tmp = tempdir().unwrap();
    let sock = tmp.path().join(".session.sock");
    // A regular file standing in for the socket — live_socket only
    // checks existence, not file type.
    fs::write(&sock, b"").unwrap();
    write_session(
        tmp.path(),
        &SessionInfo {
            pid: std::process::id(),
            sock: sock.to_str().unwrap().into(),
            started_at: now(),
        },
    );
    assert_eq!(live_socket(tmp.path()).unwrap(), Some(sock));
}

#[test]
fn status_not_running_then_running_then_stale() {
    let tmp = tempdir().unwrap();
    assert!(matches!(
        status(tmp.path()).unwrap(),
        DaemonStatus::NotRunning,
    ));

    let started = now().saturating_sub(7);
    write_session(
        tmp.path(),
        &SessionInfo {
            pid: std::process::id(),
            sock: "/tmp/whatever.sock".into(),
            started_at: started,
        },
    );
    match status(tmp.path()).unwrap() {
        DaemonStatus::Running { pid, uptime_secs } => {
            assert_eq!(pid, std::process::id());
            assert!(uptime_secs >= 7, "uptime should be >= 7, got {uptime_secs}");
        }
        other => panic!("expected Running, got {other:?}"),
    }

    write_session(
        tmp.path(),
        &SessionInfo {
            pid: DEAD_PID,
            sock: "/tmp/whatever.sock".into(),
            started_at: started,
        },
    );
    match status(tmp.path()).unwrap() {
        DaemonStatus::Stale { pid } => assert_eq!(pid, DEAD_PID),
        other => panic!("expected Stale, got {other:?}"),
    }
}

#[tokio::test]
async fn stop_is_noop_when_no_session() {
    let tmp = tempdir().unwrap();
    stop(tmp.path()).await.unwrap();
}

#[tokio::test]
async fn stop_cleans_up_stale_session() {
    let tmp = tempdir().unwrap();
    write_session(
        tmp.path(),
        &SessionInfo {
            pid: DEAD_PID,
            sock: "/tmp/x.sock".into(),
            started_at: 0,
        },
    );
    stop(tmp.path()).await.unwrap();
    assert!(!tmp.path().join(".session").exists());
}
