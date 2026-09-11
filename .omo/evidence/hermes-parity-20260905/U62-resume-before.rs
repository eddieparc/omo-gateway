//! Managed lifecycle for the local `omo app-server` daemon.
//!
//! The gateway is zero-config: when the app-server URL points at a local
//! address and nothing is listening there yet, the gateway spawns
//! `omo app-server --listen <url>` itself, waits for readiness, keeps it
//! alive (bounded restarts), and kills it on shutdown. Externally managed
//! daemons (already listening, or non-local URLs) are detected and left
//! alone.

use crate::agent::OmoBackendConfig;
use crate::{OmonError, Result};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout, Instant};

static SPAWN_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

const READY_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const READY_WAIT_AFTER_SPAWN: Duration = Duration::from_secs(30);
const RESTART_BACKOFF: Duration = Duration::from_secs(2);
const RESTART_WINDOW: Duration = Duration::from_secs(60);
const MAX_RESTARTS: usize = 3;

/// True when the daemon URL targets this machine (auto-spawn eligible).
pub fn is_local_url(url: &str) -> bool {
    let host = url
        .trim()
        .strip_prefix("ws://")
        .unwrap_or_else(|| url.trim())
        .split(['/', ':'])
        .next()
        .unwrap_or("");
    host.eq_ignore_ascii_case("127.0.0.1") || host.eq_ignore_ascii_case("localhost")
}

/// GET `<http-url>/readyz` over a raw TCP connection; true on HTTP 200.
async fn probe_readyz(ws_url: &str, limit: Duration) -> bool {
    let Some(rest) = ws_url.trim().strip_prefix("ws://") else {
        return false; // wss:// daemons are externally managed; no local probe
    };
    let Some(authority) = rest.split('/').next() else {
        return false;
    };
    let request = format!("GET /readyz HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n");
    timeout(limit, async move {
        let mut stream = tokio::net::TcpStream::connect(authority).await.ok()?;
        stream.write_all(request.as_bytes()).await.ok()?;
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.ok()?;
        let head = String::from_utf8_lossy(&buf);
        Some(head.starts_with("HTTP/1.1 200") || head.starts_with("HTTP/1.0 200"))
    })
    .await
    .unwrap_or(Some(false))
    .unwrap_or(false)
}

/// Resolve the daemon binary to something spawnable.
///
/// Under launchd the gateway inherits a minimal `PATH`
/// (`/usr/bin:/bin:/usr/sbin:/sbin`) which excludes user-level install
/// directories such as `~/.bun/bin`, so a bare `omo` fails with ENOENT even
/// though it resolves fine in an interactive shell. When the name carries no
/// path separator and cannot be found on `path_env`, fall back to well-known
/// absolute install locations. Explicit paths are returned untouched.
fn resolve_daemon_bin(bin: &str, path_env: &str) -> String {
    if bin.contains('/') {
        return bin.to_string();
    }

    let on_path = path_env
        .split(':')
        .filter(|dir| !dir.is_empty())
        .any(|dir| {
            std::path::Path::new(dir)
                .join(bin)
                .try_exists()
                .unwrap_or(false)
        });
    if on_path {
        return bin.to_string();
    }

    if let Ok(home) = std::env::var("HOME") {
        for candidate in [
            format!("{home}/.bun/bin/{bin}"),
            format!("{home}/.local/bin/{bin}"),
            format!("{home}/.npm-global/bin/{bin}"),
        ] {
            if std::path::Path::new(&candidate)
                .try_exists()
                .unwrap_or(false)
            {
                return candidate;
            }
        }
    }
    for candidate in [
        format!("/opt/homebrew/bin/{bin}"),
        format!("/usr/local/bin/{bin}"),
    ] {
        if std::path::Path::new(&candidate)
            .try_exists()
            .unwrap_or(false)
        {
            return candidate;
        }
    }

    bin.to_string()
}

/// Build the spawn command for a local daemon. Exposed for tests.
fn daemon_command(bin: &str, listen_url: &str) -> Command {
    let mut cmd = Command::new(bin);
    cmd.args(["app-server", "--listen", listen_url, "--ws-auth", "off"])
        .stdin(Stdio::null())
        .kill_on_drop(true);

    let log_file = std::env::var("HOME")
        .ok()
        .map(|home| {
            let port = listen_url
                .rsplit(':')
                .next()
                .and_then(|p| p.split('/').next())
                .unwrap_or("default");
            std::path::PathBuf::from(home)
                .join(".omon")
                .join(format!("omo-appserver-{port}.log"))
        })
        .and_then(|path| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()
        });

    if let Some(file) = log_file {
        if let Ok(file_clone) = file.try_clone() {
            cmd.stdout(Stdio::from(file));
            cmd.stderr(Stdio::from(file_clone));
            return cmd;
        }
    }

    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    cmd
}

fn autospawn_enabled() -> bool {
    !matches!(
        std::env::var("OMON_OMO_AUTOSPAWN")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "off" | "false" | "0"
    )
}

/// Owns the spawned daemon child for the lifetime of the gateway and
/// restarts it (bounded backoff) if it dies. Killing the supervisor kills
/// the daemon.
pub struct OmoDaemonSupervisor {
    child: Arc<Mutex<Option<Child>>>,
    shutdown: Arc<AtomicBool>,
}

impl OmoDaemonSupervisor {
    /// False after shutdown or a terminal restart/readiness failure.
    /// Live backend readiness still requires probing the daemon's readyz endpoint.
    pub fn is_available(&self) -> bool {
        !self.shutdown.load(Ordering::Acquire)
    }

    /// Ensure a daemon is serving `cfg.appserver_url`, spawning a local one
    /// when necessary. Returns `Ok(None)` when an externally managed daemon
    /// is already serving (or autospawn is disabled / URL is non-local).
    pub async fn ensure(cfg: &OmoBackendConfig) -> Result<Option<Self>> {
        let _guard = SPAWN_LOCK.lock().await;
        let bin = resolve_daemon_bin(
            &std::env::var("OMON_OMO_BIN").unwrap_or_else(|_| "omo".to_string()),
            &std::env::var("PATH").unwrap_or_default(),
        );
        if !autospawn_enabled() || !is_local_url(&cfg.appserver_url) {
            return Ok(None);
        }
        if probe_readyz(&cfg.appserver_url, READY_PROBE_TIMEOUT).await {
            tracing::debug!(url = %cfg.appserver_url, "external omo app-server already serving");
            return Ok(None);
        }

        tracing::info!(url = %cfg.appserver_url, bin = %bin, "spawning omo app-server daemon");
        let child = daemon_command(&bin, &cfg.appserver_url)
            .spawn()
            .map_err(|e| {
                OmonError::Config(format!(
                    "failed to spawn '{bin} app-server' for {url}: {e} \
                     (is the omo CLI installed? set OMON_OMO_BIN or OMON_OMO_AUTOSPAWN=off to manage the daemon yourself)",
                    url = cfg.appserver_url
                ))
            })?;

        let supervisor = Self {
            child: Arc::new(Mutex::new(Some(child))),
            shutdown: Arc::new(AtomicBool::new(false)),
        };

        let deadline = Instant::now() + READY_WAIT_AFTER_SPAWN;
        while Instant::now() < deadline {
            if probe_readyz(&cfg.appserver_url, READY_PROBE_TIMEOUT).await {
                tracing::info!(url = %cfg.appserver_url, "omo app-server daemon ready");
                supervisor.spawn_watcher(cfg.appserver_url.clone(), bin);
                return Ok(Some(supervisor));
            }
            if supervisor.child_exited() {
                break;
            }
            sleep(Duration::from_millis(400)).await;
        }

        supervisor.kill();
        Err(OmonError::Config(format!(
            "spawned 'omo app-server' did not become ready at {} within {}s",
            cfg.appserver_url,
            READY_WAIT_AFTER_SPAWN.as_secs()
        )))
    }

    fn child_exited(&self) -> bool {
        self.child
            .try_lock()
            .ok()
            .and_then(|mut guard| {
                guard
                    .as_mut()
                    .and_then(|child| child.try_wait().ok().flatten())
                    .map(|_| true)
            })
            .unwrap_or(false)
    }

    fn spawn_watcher(&self, url: String, bin: String) {
        let child_slot = Arc::clone(&self.child);
        let shutdown = Arc::clone(&self.shutdown);
        tokio::spawn(async move {
            let mut restarts = std::collections::VecDeque::new();
            loop {
                if shutdown.load(Ordering::Acquire) {
                    return;
                }
                loop {
                    if shutdown.load(Ordering::Acquire) {
                        return;
                    }
                    let exited = {
                        let mut guard = child_slot.lock().await;
                        match guard.as_mut() {
                            Some(child) => matches!(child.try_wait(), Ok(Some(_))),
                            None => true,
                        }
                    };
                    if exited {
                        break;
                    }
                    sleep(Duration::from_millis(500)).await;
                }
                if shutdown.load(Ordering::Acquire) {
                    return;
                }
                tracing::warn!(url = %url, "omo app-server daemon exited; checking before restart");
                sleep(RESTART_BACKOFF).await;
                if shutdown.load(Ordering::Acquire) {
                    return;
                }
                // A forked survivor or an external daemon may have taken over
                // the port; adopt it instead of spawning a child that would
                // die on bind conflict forever.
                if probe_readyz(&url, READY_PROBE_TIMEOUT).await {
                    tracing::info!(
                        url = %url,
                        "port already serving after daemon exit; deferring to the external daemon"
                    );
                    return;
                }
                let now = Instant::now();
                while restarts
                    .front()
                    .is_some_and(|at| now.duration_since(*at) >= RESTART_WINDOW)
                {
                    restarts.pop_front();
                }
                if restarts.len() == MAX_RESTARTS {
                    shutdown.store(true, Ordering::Release);
                    tracing::error!(url = %url, available = false, "omo app-server restart circuit open");
                    return;
                }
                restarts.push_back(now);
                match daemon_command(&bin, &url).spawn() {
                    Ok(child) => {
                        child_slot.lock().await.replace(child);
                        let deadline = Instant::now() + READY_WAIT_AFTER_SPAWN;
                        let mut ready = false;
                        let mut exited = false;
                        while Instant::now() < deadline {
                            if shutdown.load(Ordering::Acquire) {
                                return;
                            }
                            let remaining = deadline.saturating_duration_since(Instant::now());
                            if probe_readyz(&url, READY_PROBE_TIMEOUT.min(remaining)).await {
                                tracing::info!(url = %url, "restarted omo app-server daemon ready");
                                ready = true;
                                break;
                            }
                            exited = child_slot
                                .lock()
                                .await
                                .as_mut()
                                .is_some_and(|child| matches!(child.try_wait(), Ok(Some(_))));
                            if exited {
                                break;
                            }
                            sleep(Duration::from_millis(200)).await;
                        }
                        if !ready && !exited {
                            if let Some(mut child) = child_slot.lock().await.take() {
                                if let Err(error) = child.kill().await {
                                    tracing::error!(%error, "failed to kill/reap unready omo app-server");
                                }
                            }
                            shutdown.store(true, Ordering::Release);
                            tracing::error!(url = %url, available = false, "omo app-server replacement readiness deadline expired");
                            return;
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "failed to restart omo app-server daemon");
                    }
                }
            }
        });
    }

    fn kill(&self) {
        self.shutdown.store(true, Ordering::Release);
        if let Ok(mut guard) = self.child.try_lock() {
            if let Some(child) = guard.as_mut() {
                let _ = child.start_kill();
            }
        }
    }
}

impl Drop for OmoDaemonSupervisor {
    fn drop(&mut self) {
        self.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Re-exec only this test: daemon_command opens logs in the supervisor's
    // HOME, so changing just the spawned daemon's environment is insufficient.
    async fn isolated_home(test: &str) -> bool {
        let id = format!("agent::omo_daemon::tests::{test}");
        if std::env::var("U62_ISOLATED_TEST").as_deref() == Ok(id.as_str()) {
            return false;
        }
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join(".omon")).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([&id, "--exact", "--nocapture"])
            .env("U62_ISOLATED_TEST", &id)
            .env("HOME", home.path())
            .env("OMON_OMO_AUTOSPAWN", "on")
            .kill_on_drop(true);
        let output = timeout(Duration::from_secs(45), command.output())
            .await
            .unwrap()
            .unwrap();
        println!(
            "isolated {id}: {}\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success());
        if matches!(
            test,
            "daemon_exiting_child_budget_local_surface"
                | "daemon_restart_budget_stops_spawn_failures"
                | "daemon_restart_budget_and_unready_child"
                | "daemon_ensure_owned_local_surface"
                | "test_daemon_command_arguments"
        ) {
            assert!(
                std::fs::read_dir(home.path().join(".omon"))
                    .unwrap()
                    .next()
                    .is_some(),
                "production log opening must execute inside the isolated HOME"
            );
        }
        home.close().unwrap();
        true
    }

    #[tokio::test]
    async fn daemon_exiting_child_budget_local_surface() {
        if isolated_home("daemon_exiting_child_budget_local_surface").await {
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let events = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bin = dir.path().join("exit-daemon");
        std::fs::write(
            &bin,
            format!(
                r#"#!/usr/bin/env python3
import socket
s = socket.create_connection(('127.0.0.1', {}))
s.sendall(b'S')
s.recv(1)
"#,
                events.local_addr().unwrap().port()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        let supervisor = OmoDaemonSupervisor {
            child: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(AtomicBool::new(false)),
        };
        supervisor.spawn_watcher("invalid-url".into(), bin.to_string_lossy().into_owned());
        for _ in 0..MAX_RESTARTS {
            let (mut event, _) = timeout(Duration::from_secs(10), events.accept())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(event.read_u8().await.unwrap(), b'S');
            // Observe actual process exit before advancing the restart clock.
            let mut slot = supervisor.child.lock().await;
            event.write_all(b"X").await.unwrap();
            timeout(Duration::from_secs(5), slot.as_mut().unwrap().wait())
                .await
                .unwrap()
                .unwrap();
            drop(slot);
        }
        tokio::time::pause();
        sleep(Duration::from_secs(4)).await;
        assert!(!supervisor.is_available());
        assert!(timeout(Duration::from_secs(120), events.accept())
            .await
            .is_err());
        assert!(supervisor
            .child
            .lock()
            .await
            .as_mut()
            .unwrap()
            .try_wait()
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn daemon_external_takeover_is_untouched() {
        if isolated_home("daemon_external_takeover_is_untouched").await {
            return;
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let supervisor = OmoDaemonSupervisor {
            child: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(AtomicBool::new(false)),
        };
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = [0; 1024];
                socket.read(&mut buf).await.unwrap();
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                    .await
                    .unwrap();
            }
        });
        supervisor.spawn_watcher(url.clone(), "/nonexistent/U62-daemon".into());
        // The external server survives supervisor shutdown; no child is adopted.
        assert!(probe_readyz(&url, Duration::from_secs(5)).await);
        timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
        assert!(supervisor.child.lock().await.is_none());
        assert!(supervisor.is_available());
        supervisor.kill();
    }

    #[tokio::test]
    async fn daemon_ensure_external_local_surface() {
        if isolated_home("daemon_ensure_external_local_surface").await {
            return;
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cfg = OmoBackendConfig::new(format!("ws://{}", listener.local_addr().unwrap()));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 1024];
            socket.read(&mut buf).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        assert!(OmoDaemonSupervisor::ensure(&cfg).await.unwrap().is_none());
        timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn daemon_ensure_owned_local_surface() {
        if isolated_home("daemon_ensure_owned_local_surface").await {
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let port = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cfg = OmoBackendConfig::new(format!("ws://{}", port.local_addr().unwrap()));
        drop(port);
        let bin = dir.path().join("ready-daemon");
        std::fs::write(
            &bin,
            r#"#!/usr/bin/env python3
import socket, sys
s = socket.socket()
s.bind(('127.0.0.1', int(sys.argv[3].rsplit(':', 1)[1])))
s.listen()
while True:
    c, _ = s.accept()
    c.recv(4096)
    c.sendall(b'HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n')
    c.close()
"#,
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        // This process re-executes exactly one test with its own HOME/env.
        std::env::set_var("OMON_OMO_BIN", &bin);
        let supervisor = timeout(Duration::from_secs(10), OmoDaemonSupervisor::ensure(&cfg))
            .await
            .unwrap()
            .unwrap()
            .expect("public ensure must own the newly spawned daemon");
        assert!(supervisor.is_available());
        assert!(probe_readyz(&cfg.appserver_url, READY_PROBE_TIMEOUT).await);
        supervisor.kill();
        let mut slot = supervisor.child.lock().await;
        let status = timeout(Duration::from_secs(5), slot.as_mut().unwrap().wait())
            .await
            .unwrap()
            .unwrap();
        assert!(!status.success());
        assert!(!supervisor.is_available());
        assert!(!probe_readyz(&cfg.appserver_url, READY_PROBE_TIMEOUT).await);
        println!("public ensure: owned HTTP-200 child ready, shutdown child reaped, socket refused");
    }

    #[tokio::test]
    async fn daemon_restart_budget_stops_spawn_failures() {
        if isolated_home("daemon_restart_budget_stops_spawn_failures").await {
            return;
        }
        tokio::time::pause();
        let supervisor = OmoDaemonSupervisor {
            child: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(AtomicBool::new(false)),
        };
        supervisor.spawn_watcher("invalid-url".into(), "/nonexistent/U62-daemon".into());
        // Auto-advance drives the actual backoff timers, not wall time.
        sleep(Duration::from_secs(15)).await;
        let stopped = supervisor.shutdown.load(Ordering::Acquire);
        supervisor.kill();
        assert!(
            stopped,
            "restart circuit did not stop failed spawn attempts"
        );
    }

    #[tokio::test]
    async fn daemon_restart_budget_and_unready_child() {
        if isolated_home("daemon_restart_budget_and_unready_child").await {
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        // Given an exited owned daemon and a replacement that stays alive,
        // announces its PID over a real socket, and only serves HTTP 503.
        let dir = tempfile::tempdir().unwrap();
        let events = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", port.local_addr().unwrap());
        drop(port);
        let bin = dir.path().join("daemon");
        std::fs::write(
            &bin,
            format!(
                r#"#!/usr/bin/env python3
import os, socket, sys
s = socket.socket()
s.bind(('127.0.0.1', int(sys.argv[3].rsplit(':', 1)[1])))
s.listen()
event = socket.create_connection(('127.0.0.1', {}))
event.sendall(str(os.getpid()).encode() + b'\n')
while True:
    c, _ = s.accept()
    c.recv(4096)
    c.sendall(b'HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n')
    c.close()
"#,
                events.local_addr().unwrap().port()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut initial = Command::new("/usr/bin/true").spawn().unwrap();
        initial.wait().await.unwrap();
        let supervisor = OmoDaemonSupervisor {
            child: Arc::new(Mutex::new(Some(initial))),
            shutdown: Arc::new(AtomicBool::new(false)),
        };
        // Subscribe before starting the real production watcher.
        let started = events.accept();
        supervisor.spawn_watcher(url.clone(), bin.to_string_lossy().into_owned());
        let (mut event, _) = timeout(Duration::from_secs(10), started)
            .await
            .unwrap()
            .unwrap();
        let mut pid = Vec::new();
        loop {
            let byte = event.read_u8().await.unwrap();
            if byte == b'\n' {
                break;
            }
            pid.push(byte);
        }
        assert!(!probe_readyz(&url, READY_PROBE_TIMEOUT).await);
        // Time is the behavior under test: advance beyond the readiness bound.
        tokio::time::pause();
        tokio::time::advance(READY_WAIT_AFTER_SPAWN + Duration::from_secs(3)).await;
        tokio::time::resume();
        let eof = timeout(Duration::from_secs(2), event.read_u8()).await;
        let mut slot = supervisor.child.lock().await;
        let reaped = match slot.as_mut() {
            Some(child) => child.try_wait().unwrap().is_some(),
            None => true,
        };
        // Cleanup also on RED, before the assertion.
        if let Some(child) = slot.as_mut() {
            if !reaped {
                child.kill().await.unwrap();
            }
        }
        drop(slot);
        let unavailable = !supervisor.is_available();
        supervisor.kill();
        assert!(unavailable);
        assert!(
            reaped && matches!(eof, Ok(Err(_))),
            "owned HTTP-503 replacement PID {} survived readiness deadline",
            String::from_utf8_lossy(&pid)
        );
    }

    #[test]
    fn test_is_local_url() {
        assert!(is_local_url("ws://127.0.0.1:19742"));
        assert!(is_local_url("ws://localhost:19742"));
        assert!(is_local_url("ws://LOCALHOST:9"));
        assert!(!is_local_url("ws://10.1.2.3:19742"));
        assert!(!is_local_url("wss://relay.example.com"));
    }

    #[tokio::test]
    async fn test_probe_readyz_detects_http_200_and_refusal() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 1024];
            let _ = sock.read(&mut buf).await;
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await
                .unwrap();
        });
        assert!(probe_readyz(&format!("ws://{addr}"), Duration::from_secs(2)).await);
        server.await.unwrap();

        // Nothing listening on this ephemeral port range slot: probe fails.
        let dead = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead_port = dead.local_addr().unwrap().port();
        drop(dead);
        assert!(
            !probe_readyz(
                &format!("ws://127.0.0.1:{dead_port}"),
                Duration::from_secs(2)
            )
            .await
        );
    }

    #[test]
    fn test_resolve_daemon_bin_falls_back_to_known_install_paths() {
        // launchd hands the gateway a minimal PATH (/usr/bin:/bin:/usr/sbin:/sbin)
        // that excludes ~/.bun/bin, so a bare "omo" fails to spawn with ENOENT.
        // Resolution must fall back to well-known absolute install locations.
        let home = std::env::var("HOME").unwrap();
        let bun_omo = format!("{home}/.bun/bin/omo");
        if !std::path::Path::new(&bun_omo).exists() {
            return; // fallback target absent on this machine
        }

        let resolved = resolve_daemon_bin("omo", "/usr/bin:/bin:/usr/sbin:/sbin");
        assert_eq!(
            resolved, bun_omo,
            "bare 'omo' must resolve to an absolute path when PATH lacks the install dir"
        );

        // An explicit absolute override is always honoured verbatim.
        assert_eq!(
            resolve_daemon_bin("/custom/omo", "/usr/bin:/bin"),
            "/custom/omo"
        );
    }

    #[tokio::test]
    async fn test_daemon_command_arguments() {
        if isolated_home("test_daemon_command_arguments").await {
            return;
        }
        let cmd = daemon_command("omo", "ws://127.0.0.1:19742");
        // Command internals are not inspectable portably; assert via as_std.
        let std_cmd = cmd.as_std();
        let args: Vec<String> = std_cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args,
            vec![
                "app-server",
                "--listen",
                "ws://127.0.0.1:19742",
                "--ws-auth",
                "off"
            ]
        );
        assert_eq!(std_cmd.get_program().to_string_lossy(), "omo");
    }
}
