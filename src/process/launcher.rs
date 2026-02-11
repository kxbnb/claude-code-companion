use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Command;

pub struct CliLauncher {
    port: u16,
    session_id: String,
    cwd: String,
    model: Option<String>,
    env_vars: HashMap<String, String>,
    resume_session_id: Option<String>,
}

impl CliLauncher {
    pub fn new(port: u16, session_id: String, cwd: String, model: Option<String>) -> Self {
        Self {
            port,
            session_id,
            cwd,
            model,
            env_vars: HashMap::new(),
            resume_session_id: None,
        }
    }

    pub fn with_env_vars(mut self, vars: HashMap<String, String>) -> Self {
        self.env_vars = vars;
        self
    }

    pub fn with_resume_session_id(mut self, id: Option<String>) -> Self {
        self.resume_session_id = id;
        self
    }

    /// Spawn the Claude CLI process with --sdk-url pointing back to our WS server.
    /// This function awaits process exit — run it in a spawned task.
    pub async fn spawn(self) -> anyhow::Result<std::process::ExitStatus> {
        let binary = find_claude_binary()?;
        let sdk_url = format!(
            "ws://127.0.0.1:{}/ws/cli/{}",
            self.port, self.session_id
        );

        let mut args = vec![
            "--sdk-url".to_string(),
            sdk_url,
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--input-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
        ];

        if let Some(ref m) = self.model {
            args.push("--model".to_string());
            args.push(m.clone());
        }

        if let Some(ref resume_id) = self.resume_session_id {
            args.push("--resume".to_string());
            args.push(resume_id.clone());
        }

        // In SDK mode (--sdk-url), the CLI stays alive and receives prompts
        // over WebSocket. No -p/--print needed (those cause single-shot exit).

        tracing::info!(
            "Spawning Claude CLI: {} {}",
            binary,
            args.join(" ")
        );

        let mut cmd = Command::new(&binary);
        cmd.args(&args)
            .current_dir(&self.cwd)
            .env("CLAUDECODE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Inject environment variables from profile
        for (k, v) in &self.env_vars {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!(
                "Failed to spawn '{}': {}. Is Claude Code CLI installed?",
                binary,
                e
            )
        })?;

        let pid = child.id();
        tracing::info!("Claude CLI spawned (PID: {:?})", pid);

        // Drain stdout to prevent pipe buffer blocking
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let reader = tokio::io::BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.trim().is_empty() {
                        tracing::debug!("[claude stdout] {}", line);
                    }
                }
            });
        }

        // Pipe stderr for debugging
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let reader = tokio::io::BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.trim().is_empty() {
                        tracing::debug!("[claude stderr] {}", line);
                    }
                }
            });
        }

        let status = child.wait().await?;
        tracing::info!("Claude CLI exited: {:?}", status);

        Ok(status)
    }
}

/// Find the claude binary on PATH
fn find_claude_binary() -> anyhow::Result<String> {
    let candidates = ["claude"];

    for name in &candidates {
        if let Ok(output) = std::process::Command::new("which")
            .arg(name)
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Ok(path);
                }
            }
        }
    }

    // Fall back to just "claude" and let the OS resolve it
    Ok("claude".to_string())
}
