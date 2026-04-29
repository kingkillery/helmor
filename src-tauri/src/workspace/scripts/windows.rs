use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use anyhow::{bail, Context, Result};
use serde::Serialize;
use tauri::ipc::Channel;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ScriptEvent {
    Started { pid: u32, command: String },
    Stdout { data: String },
    Stderr { data: String },
    Exited { code: Option<i32> },
    Error { message: String },
}

type ProcessKey = (String, String, Option<String>);

#[derive(Clone)]
struct ProcessHandle {
    pid: u32,
    killed: Arc<AtomicBool>,
    stdin: Arc<Mutex<ChildStdin>>,
}

#[derive(Clone, Default)]
pub struct ScriptProcessManager {
    processes: Arc<Mutex<HashMap<ProcessKey, ProcessHandle>>>,
}

impl ScriptProcessManager {
    pub fn new() -> Self {
        Self::default()
    }

    fn register(
        &self,
        key: ProcessKey,
        pid: u32,
        stdin: Arc<Mutex<ChildStdin>>,
    ) -> Arc<AtomicBool> {
        let killed = Arc::new(AtomicBool::new(false));
        let handle = ProcessHandle {
            pid,
            killed: killed.clone(),
            stdin,
        };
        let mut map = self.processes.lock().expect("process map poisoned");
        if let Some(old) = map.insert(key, handle) {
            old.killed.store(true, Ordering::Release);
            kill_process_tree(old.pid);
        }
        killed
    }

    fn unregister(&self, key: &ProcessKey, pid: u32) {
        let mut map = self.processes.lock().expect("process map poisoned");
        if let Some(handle) = map.get(key) {
            if handle.pid == pid {
                map.remove(key);
            }
        }
    }

    pub fn kill(&self, key: &ProcessKey) -> bool {
        let handle = {
            let map = self.processes.lock().expect("process map poisoned");
            map.get(key).cloned()
        };
        match handle {
            Some(handle) => {
                handle.killed.store(true, Ordering::Release);
                kill_process_tree(handle.pid);
                true
            }
            None => false,
        }
    }

    pub fn write_stdin(&self, key: &ProcessKey, data: &[u8]) -> Result<bool> {
        let stdin = {
            let map = self.processes.lock().expect("process map poisoned");
            map.get(key).map(|handle| handle.stdin.clone())
        };
        let Some(stdin) = stdin else {
            return Ok(false);
        };

        let mut file = stdin.lock().expect("stdin mutex poisoned");
        file.write_all(data)
            .context("terminal stdin write failed on Windows")?;
        file.flush()
            .context("terminal stdin flush failed on Windows")?;
        Ok(true)
    }

    pub fn resize(&self, key: &ProcessKey, _cols: u16, _rows: u16) -> Result<bool> {
        let map = self.processes.lock().expect("process map poisoned");
        Ok(map.contains_key(key))
    }
}

#[derive(Clone)]
pub struct ScriptContext {
    pub root_path: String,
    pub workspace_path: Option<String>,
    pub workspace_name: Option<String>,
    pub default_branch: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn run_script(
    manager: &ScriptProcessManager,
    repo_id: &str,
    script_type: &str,
    workspace_id: Option<&str>,
    script: &str,
    working_dir: &str,
    context: &ScriptContext,
    channel: Channel<ScriptEvent>,
) -> Result<Option<i32>> {
    if script.trim().is_empty() {
        bail!("Script is empty");
    }

    run_shell_session(
        manager,
        repo_id,
        script_type,
        workspace_id,
        Some(script),
        working_dir,
        context,
        channel,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_terminal_session(
    manager: &ScriptProcessManager,
    repo_id: &str,
    script_type: &str,
    workspace_id: Option<&str>,
    working_dir: &str,
    context: &ScriptContext,
    channel: Channel<ScriptEvent>,
) -> Result<Option<i32>> {
    run_shell_session(
        manager,
        repo_id,
        script_type,
        workspace_id,
        None,
        working_dir,
        context,
        channel,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_script_with_shell(
    manager: &ScriptProcessManager,
    repo_id: &str,
    script_type: &str,
    workspace_id: Option<&str>,
    script: Option<&str>,
    working_dir: &str,
    context: &ScriptContext,
    channel: Channel<ScriptEvent>,
    shell_path: &str,
    shell_args: &[&str],
) -> Result<Option<i32>> {
    run_shell_session_with_command(
        manager,
        repo_id,
        script_type,
        workspace_id,
        script,
        working_dir,
        context,
        channel,
        shell_path,
        shell_args,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_shell_session(
    manager: &ScriptProcessManager,
    repo_id: &str,
    script_type: &str,
    workspace_id: Option<&str>,
    script: Option<&str>,
    working_dir: &str,
    context: &ScriptContext,
    channel: Channel<ScriptEvent>,
) -> Result<Option<i32>> {
    let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
    run_shell_session_with_command(
        manager,
        repo_id,
        script_type,
        workspace_id,
        script,
        working_dir,
        context,
        channel,
        &shell,
        &["/Q", "/K"],
    )
}

#[allow(clippy::too_many_arguments)]
fn run_shell_session_with_command(
    manager: &ScriptProcessManager,
    repo_id: &str,
    script_type: &str,
    workspace_id: Option<&str>,
    script: Option<&str>,
    working_dir: &str,
    context: &ScriptContext,
    channel: Channel<ScriptEvent>,
    shell_path: &str,
    shell_args: &[&str],
) -> Result<Option<i32>> {
    let mut cmd = Command::new(shell_path);
    cmd.args(shell_args)
        .current_dir(working_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("FORCE_COLOR", "1")
        .env("CLICOLOR_FORCE", "1")
        .env("HELMOR_ROOT_PATH", &context.root_path);

    if let Some(workspace_path) = &context.workspace_path {
        cmd.env("HELMOR_WORKSPACE_PATH", workspace_path);
    }
    if let Some(workspace_name) = &context.workspace_name {
        cmd.env("HELMOR_WORKSPACE_NAME", workspace_name);
    }
    if let Some(default_branch) = &context.default_branch {
        cmd.env("HELMOR_DEFAULT_BRANCH", default_branch);
    }

    let mut child = cmd
        .spawn()
        .with_context(|| format!("Failed to spawn {shell_path} on Windows"))?;
    let pid = child.id();
    let stdin = Arc::new(Mutex::new(
        child.stdin.take().context("shell stdin missing")?,
    ));
    let stdout = child.stdout.take().context("shell stdout missing")?;
    let stderr = child.stderr.take().context("shell stderr missing")?;

    let started_command = script
        .map(str::to_string)
        .unwrap_or_else(|| format!("{shell_path} {}", shell_args.join(" ")));
    let _ = channel.send(ScriptEvent::Started {
        pid,
        command: started_command,
    });

    let key: ProcessKey = (
        repo_id.to_string(),
        script_type.to_string(),
        workspace_id.map(str::to_string),
    );
    let killed = manager.register(key.clone(), pid, stdin.clone());

    let stdout_thread = spawn_reader(stdout, channel.clone(), true);
    let stderr_thread = spawn_reader(stderr, channel.clone(), false);

    if let Some(script) = script {
        let wrapped = format!(
            "{script}\r\nset HELMOR_EXIT_CODE=%ERRORLEVEL%\r\necho [Completed with exit code %HELMOR_EXIT_CODE%]\r\nexit /b %HELMOR_EXIT_CODE%\r\n"
        );
        let mut file = stdin.lock().expect("stdin mutex poisoned");
        file.write_all(wrapped.as_bytes())
            .context("failed to send initial Windows script")?;
        file.flush()
            .context("failed to flush initial Windows script")?;
    }

    let status = child.wait().ok();

    manager.unregister(&key, pid);

    let _ = stdout_thread.join();
    let _ = stderr_thread.join();

    let exit_code = if killed.load(Ordering::Acquire) {
        None
    } else {
        status.and_then(|status| status.code())
    };
    let _ = channel.send(ScriptEvent::Exited { code: exit_code });
    Ok(exit_code)
}

fn spawn_reader<R: Read + Send + 'static>(
    mut reader: R,
    channel: Channel<ScriptEvent>,
    stdout: bool,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name(if stdout {
            "script-stdout".into()
        } else {
            "script-stderr".into()
        })
        .spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let data = String::from_utf8_lossy(&buf[..n]).into_owned();
                        let _ = if stdout {
                            channel.send(ScriptEvent::Stdout { data })
                        } else {
                            channel.send(ScriptEvent::Stderr { data })
                        };
                    }
                    Err(_) => break,
                }
            }
        })
        .expect("failed to spawn Windows script reader thread")
}

fn kill_process_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}
