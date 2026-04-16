use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use tracing::{info, warn};

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use uuid::Uuid;

use super::shared::MAX_PTY_WRITE_SIZE;
use super::storage;

/// Resolve a command name to its actual path, preferring .exe over .cmd on Windows.
fn resolve_command_path(command: &str) -> String {
    #[cfg(windows)]
    {
        use super::shared::hide_window;
        let mut where_cmd = std::process::Command::new("where");
        where_cmd.arg(command);
        hide_window(&mut where_cmd);
        if let Ok(output) = where_cmd.output() {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let lines: Vec<&str> = stdout.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
                if let Some(exe) = lines.iter().find(|l| l.ends_with(".exe")) {
                    return exe.to_string();
                }
                if let Some(cmd_file) = lines.iter().find(|l| l.ends_with(".cmd")) {
                    return cmd_file.to_string();
                }
                if let Some(first) = lines.first() {
                    return first.to_string();
                }
            }
        }
        format!("{}.cmd", command)
    }
    #[cfg(not(windows))]
    {
        command.to_string()
    }
}

const PTY_TRANSCRIPT_LIMIT_BYTES: usize = 256 * 1024;

pub(crate) fn decode_terminal_chunk(bytes: &[u8], pending: &mut Vec<u8>) -> String {
    let (data, leftover) = {
        let chunk = if pending.is_empty() {
            bytes
        } else {
            pending.extend_from_slice(bytes);
            pending.as_slice()
        };

        let valid_up_to = match std::str::from_utf8(chunk) {
            Ok(_) => chunk.len(),
            Err(e) => e.valid_up_to(),
        };

        let data = String::from_utf8_lossy(&chunk[..valid_up_to]).into_owned();
        let leftover = chunk[valid_up_to..].to_vec();
        (data, leftover)
    };

    *pending = leftover;
    data
}

/// Events emitted by PTY sessions via channels
#[derive(Clone, Debug)]
pub enum PtyEvent {
    /// New output data from a session
    Output { session_id: String, data: String },
    /// Session has exited
    Exit {
        session_id: String,
        exit_code: Option<i32>,
        success: bool,
        killed: bool,
    },
}

/// Info about a running PTY session
#[derive(Clone, Serialize, Debug)]
pub struct PtySessionInfo {
    pub id: String,
    pub project_path: String,
    pub pid: Option<u32>,
    pub alive: bool,
}

#[derive(Clone, Serialize, Debug)]
pub struct PtyTranscript {
    pub session_id: String,
    pub data: String,
    pub truncated: bool,
}

/// Internal state for one PTY session
struct PtySession {
    info: PtySessionInfo,
    killer: Box<dyn ChildKiller + Send + Sync>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    kill_flag: Arc<std::sync::atomic::AtomicBool>,
}

/// Framework-agnostic PTY session manager.
/// Output is delivered via an mpsc channel instead of Tauri events.
pub struct PtyManager {
    sessions: HashMap<String, PtySession>,
    event_tx: mpsc::Sender<PtyEvent>,
}

impl PtyManager {
    pub fn new(event_tx: mpsc::Sender<PtyEvent>) -> Self {
        Self {
            sessions: HashMap::new(),
            event_tx,
        }
    }

    /// Create a new PTY session. Returns the session ID.
    pub fn create_session(
        &mut self,
        project_path: &str,
        cols: u16,
        rows: u16,
        command: &str,
        args: &[String],
    ) -> Result<String, String> {
        let project_dir = std::path::Path::new(project_path);
        if !project_dir.is_dir() {
            return Err(format!(
                "Project path '{}' is not a valid directory",
                project_path
            ));
        }

        info!(command = %command, project_path = %project_path, "Creating PTY session");

        let session_id = Uuid::new_v4().to_string();
        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Failed to open PTY: {}", e))?;

        // Resolve command on Windows — prefer .exe over .cmd
        let resolved_command = resolve_command_path(command);
        let mut cmd = if cfg!(windows) && resolved_command.ends_with(".cmd") {
            let mut c = CommandBuilder::new("cmd.exe");
            c.args(&["/c", &resolved_command]);
            c
        } else {
            CommandBuilder::new(&resolved_command)
        };
        cmd.cwd(project_path);

        for arg in args {
            cmd.arg(arg);
        }

        // Claude-specific env
        if command == "claude" {
            cmd.env_remove("CLAUDECODE");
            cmd.env_remove("CLAUDE_CODE_ENTRYPOINT");
            cmd.env("FLIGHTDECK", "1");
        }

        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        let mut child = pair.slave.spawn_command(cmd).map_err(|e| {
            format!(
                "Failed to spawn {} in PTY: {}. Is {} installed?",
                command, e, command
            )
        })?;

        let pid = child.process_id();
        let killer = child.clone_killer();

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("Failed to take PTY writer: {}", e))?;

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("Failed to clone PTY reader: {}", e))?;

        let kill_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let info = PtySessionInfo {
            id: session_id.clone(),
            project_path: project_path.to_string(),
            pid,
            alive: true,
        };

        if let Some(path) = transcript_path(&session_id) {
            let _ = fs::write(path, "");
        }

        let session = PtySession {
            info: info.clone(),
            killer,
            writer,
            master: pair.master,
            kill_flag: kill_flag.clone(),
        };

        self.sessions.insert(session_id.clone(), session);

        // Spawn output reader thread
        let output_sid = session_id.clone();
        let output_tx = self.event_tx.clone();
        let output_kill_flag = kill_flag.clone();

        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut pending: Vec<u8> = Vec::new();
            loop {
                if output_kill_flag.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }

                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let data = decode_terminal_chunk(&buf[..n], &mut pending);
                        append_transcript(&output_sid, &data);
                        let _ = output_tx.send(PtyEvent::Output {
                            session_id: output_sid.clone(),
                            data,
                        });
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        if err_str.contains("broken pipe")
                            || err_str.contains("The pipe has been ended")
                            || e.kind() == std::io::ErrorKind::BrokenPipe
                        {
                            break;
                        }
                        thread::sleep(std::time::Duration::from_millis(10));
                    }
                }
            }
        });

        let wait_sid = session_id.clone();
        let wait_tx = self.event_tx.clone();
        let wait_kill_flag = kill_flag;

        thread::spawn(move || {
            let wait_result = child.wait();
            let killed = wait_kill_flag.load(std::sync::atomic::Ordering::Relaxed);

            let (exit_code, success) = match wait_result {
                Ok(status) => {
                    let exit_code = Some(status.exit_code() as i32);
                    (exit_code, status.success() && !killed)
                }
                Err(e) => {
                    warn!(session_id = %wait_sid, error = %e, "Failed waiting for PTY child exit");
                    (None, false)
                }
            };

            info!(session_id = %wait_sid, exit_code = ?exit_code, killed, success, "PTY session exited");
            let _ = wait_tx.send(PtyEvent::Exit {
                session_id: wait_sid,
                exit_code,
                success,
                killed,
            });
        });

        Ok(session_id)
    }

    /// Write data to a PTY session's stdin.
    pub fn write(&mut self, session_id: &str, data: &str) -> Result<(), String> {
        if data.len() > MAX_PTY_WRITE_SIZE {
            return Err(format!(
                "PTY write data exceeds max size ({} bytes, limit {})",
                data.len(),
                MAX_PTY_WRITE_SIZE
            ));
        }

        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("PTY session {} not found", session_id))?;

        session
            .writer
            .write_all(data.as_bytes())
            .map_err(|e| format!("Failed to write to PTY: {}", e))?;
        session
            .writer
            .flush()
            .map_err(|e| format!("Failed to flush PTY: {}", e))?;

        Ok(())
    }

    /// Resize a PTY session.
    pub fn resize(&self, session_id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| format!("PTY session {} not found", session_id))?;

        session
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Failed to resize PTY: {}", e))?;

        Ok(())
    }

    /// Kill a PTY session.
    pub fn kill(&mut self, session_id: &str) -> Result<(), String> {
        if let Some(session) = self.sessions.get_mut(session_id) {
            info!(session_id = %session_id, "Killing PTY session");
            session
                .kill_flag
                .store(true, std::sync::atomic::Ordering::Relaxed);
            session.info.alive = false;
            if let Err(e) = session.killer.kill() {
                warn!(session_id = %session_id, error = %e, "Failed to kill PTY child");
            }
            Ok(())
        } else {
            Err(format!("PTY session {} not found", session_id))
        }
    }

    /// Kill a PTY session and wait for the exit event (with timeout).
    /// Returns Ok(true) if the session exited, Ok(false) if it timed out.
    pub fn kill_and_wait(
        &mut self,
        session_id: &str,
        timeout: std::time::Duration,
    ) -> Result<bool, String> {
        self.kill(session_id)?;
        // We can't block on the mpsc channel here since we don't own the receiver.
        // Instead, poll the session's kill_flag and check if the session was removed.
        let start = std::time::Instant::now();
        let sid = session_id.to_string();
        loop {
            if !self.sessions.contains_key(&sid) {
                return Ok(true);
            }
            if start.elapsed() >= timeout {
                warn!(session_id = %sid, "PTY kill timed out after {:?}", timeout);
                // Force remove the session even if it didn't exit cleanly
                self.sessions.remove(&sid);
                return Ok(false);
            }
            thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    /// Kill multiple sessions and wait for all to exit.
    pub fn kill_sessions_and_wait(
        &mut self,
        session_ids: &[String],
        timeout: std::time::Duration,
    ) -> Vec<(String, bool)> {
        // First, send kill signal to all sessions
        for sid in session_ids {
            let _ = self.kill(sid);
        }
        // Then wait for all to exit
        let start = std::time::Instant::now();
        let mut results = Vec::new();
        for sid in session_ids {
            let exited = loop {
                if !self.sessions.contains_key(sid) {
                    break true;
                }
                if start.elapsed() >= timeout {
                    self.sessions.remove(sid);
                    break false;
                }
                thread::sleep(std::time::Duration::from_millis(50));
            };
            results.push((sid.clone(), exited));
        }
        results
    }

    /// List all active sessions.
    pub fn list(&self) -> Vec<PtySessionInfo> {
        self.sessions.values().map(|s| s.info.clone()).collect()
    }

    /// Remove a session from tracking (called when output thread detects exit).
    pub fn remove_session(&mut self, session_id: &str) {
        self.sessions.remove(session_id);
    }
}

pub fn read_transcript(session_id: &str) -> Result<PtyTranscript, String> {
    let path = transcript_path(session_id)
        .ok_or_else(|| "Unable to resolve transcript path".to_string())?;

    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(format!("Failed to read PTY transcript: {}", e)),
    };

    let truncated = bytes.len() > PTY_TRANSCRIPT_LIMIT_BYTES;
    let relevant = if truncated {
        &bytes[bytes.len() - PTY_TRANSCRIPT_LIMIT_BYTES..]
    } else {
        &bytes[..]
    };

    Ok(PtyTranscript {
        session_id: session_id.to_string(),
        data: String::from_utf8_lossy(relevant).to_string(),
        truncated,
    })
}

fn transcript_path(session_id: &str) -> Option<PathBuf> {
    // Validate session_id is a valid UUID to prevent path traversal
    if uuid::Uuid::parse_str(session_id).is_err() {
        return None;
    }
    let dir = storage::data_dir().join("pty-transcripts");
    let _ = fs::create_dir_all(&dir);
    Some(dir.join(format!("{}.log", session_id)))
}

fn append_transcript(session_id: &str, data: &str) {
    let Some(path) = transcript_path(session_id) else {
        return;
    };

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(data.as_bytes());
    }
}

/// Thread-safe wrapper
pub type SharedPtyManager = Arc<Mutex<PtyManager>>;

pub fn create_shared_pty_manager(event_tx: mpsc::Sender<PtyEvent>) -> SharedPtyManager {
    Arc::new(Mutex::new(PtyManager::new(event_tx)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_path_rejects_traversal_attack() {
        assert!(transcript_path("../../etc/passwd").is_none());
    }

    #[test]
    fn transcript_path_rejects_non_uuid() {
        assert!(transcript_path("not-a-uuid").is_none());
        assert!(transcript_path("").is_none());
        assert!(transcript_path("hello world").is_none());
    }

    #[test]
    fn transcript_path_accepts_valid_uuid() {
        let id = uuid::Uuid::new_v4().to_string();
        let path = transcript_path(&id);
        assert!(path.is_some());
        assert!(path.unwrap().to_string_lossy().contains(&id));
    }

    #[test]
    fn decode_terminal_chunk_preserves_ansi_sequences() {
        let mut pending = Vec::new();

        let data = decode_terminal_chunk(b"\x1b[31mred\x1b[0m\r\n", &mut pending);

        assert_eq!(data, "\x1b[31mred\x1b[0m\r\n");
        assert!(pending.is_empty());
    }

    #[test]
    fn decode_terminal_chunk_buffers_split_utf8_sequences() {
        let mut pending = Vec::new();

        let first = decode_terminal_chunk(&[0xE2, 0x94], &mut pending);
        let second = decode_terminal_chunk(&[0x82, b'\n'], &mut pending);

        assert_eq!(first, "");
        assert_eq!(second, "│\n");
        assert!(pending.is_empty());
    }

    #[test]
    fn decode_terminal_chunk_does_not_rewrite_plain_text() {
        let mut pending = Vec::new();

        let data = decode_terminal_chunk("Claude Code for Cursor".as_bytes(), &mut pending);

        assert_eq!(data, "Claude Code for Cursor");
        assert!(pending.is_empty());
    }
}
