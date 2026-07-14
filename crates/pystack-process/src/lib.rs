//! Process lifecycle management for PyStack Runner.
//!
//! Replaces the process orchestration engine from `core.py` — subprocess
//! spawning, Windows Job Objects, health checks, log rotation, supervision.

pub use pystack_types::{HealthCheck, ResourceLimits, ServiceConfig, StackConfig, StartResult};

use pystack_state::{ServiceUpdate, StateDb};
use std::collections::HashMap;
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(not(target_os = "windows"))]
use std::os::unix::process::CommandExt;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

fn create_command<S: AsRef<std::ffi::OsStr>>(program: S) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    cmd
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Stack(String),
}

// Windows Job Object handle tracking
#[cfg(target_os = "windows")]
#[derive(Debug)]
struct JobHandle(*mut std::ffi::c_void);
#[cfg(target_os = "windows")]
unsafe impl Send for JobHandle {}
#[cfg(target_os = "windows")]
unsafe impl Sync for JobHandle {}

#[cfg(target_os = "windows")]
static JOB_HANDLES: std::sync::LazyLock<std::sync::Mutex<HashMap<u32, JobHandle>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Process manager for native backend services.
pub struct ProcessManager {
    stack: StackConfig,
    db: StateDb,
    #[allow(dead_code)]
    children: HashMap<String, Child>,
}

impl ProcessManager {
    /// Create a new process manager for the given stack configuration.
    pub fn new(stack: StackConfig) -> Result<Self, ProcessError> {
        let db = StateDb::open(&stack.project, &stack.state_dir)
            .map_err(|e| ProcessError::Stack(e.to_string()))?;
        Ok(Self {
            stack,
            db,
            children: HashMap::new(),
        })
    }

    /// Get a reference to the state database.
    pub fn db(&self) -> &StateDb {
        &self.db
    }

    /// Get a reference to the stack configuration.
    pub fn stack(&self) -> &StackConfig {
        &self.stack
    }

    /// Start a service and return the result.
    pub fn start(&mut self, name: &str, force: bool) -> Result<StartResult, ProcessError> {
        let svc =
            self.stack.services.get(name).ok_or_else(|| {
                ProcessError::Stack(format!("service '{}' not found in stack", name))
            })?;

        // Wait for dependencies
        if !self.wait_for_dependencies(name)? {
            return Ok(StartResult {
                service: name.to_string(),
                ok: false,
                message: format!("{}: dependency wait failed; check status", name),
            });
        }

        // Check if already running
        if let Some(row) = self
            .db
            .get(name)
            .map_err(|e| ProcessError::Stack(e.to_string()))?
        {
            if let Some(pid) = row.pid {
                if is_pid_running(pid) && !force {
                    if self.health_ok(name)? {
                        self.db
                            .upsert(
                                name,
                                &ServiceUpdate {
                                    status: Some("running".into()),
                                    ..Default::default()
                                },
                            )
                            .map_err(|e| ProcessError::Stack(e.to_string()))?;
                        return Ok(StartResult {
                            service: name.to_string(),
                            ok: true,
                            message: format!("{}: already running pid={}", name, pid),
                        });
                    }
                    return Ok(StartResult {
                        service: name.to_string(),
                        ok: false,
                        message: format!("{}: running but unhealthy pid={}; check logs", name, pid),
                    });
                }
            }
        }

        // Verify cwd exists
        if !svc.cwd.exists() {
            return Err(ProcessError::Stack(format!(
                "{}: cwd does not exist: {}",
                name,
                svc.cwd.display()
            )));
        }

        // Verify ports are not already in use
        for port_str in &svc.ports {
            let port_to_check: u16 = if let Some(idx) = port_str.rfind(':') {
                port_str[idx + 1..].parse().unwrap_or(0)
            } else {
                port_str.parse().unwrap_or(0)
            };
            if port_to_check > 0 {
                if let Err(_) = std::net::TcpListener::bind(format!("127.0.0.1:{}", port_to_check))
                {
                    return Ok(StartResult {
                        service: name.to_string(),
                        ok: false,
                        message: format!("{}: port {} is already in use", name, port_to_check),
                    });
                }
            }
        }

        // Open log files
        let (stdout_path, stderr_path) = self.open_log_paths(name);

        // Prepare environment
        let env = self.prepare_env(name);

        // Build command
        let args = self.build_command_args(name);
        if args.is_empty() {
            return Err(ProcessError::Stack(format!(
                "{}: native service command must not be empty",
                name
            )));
        }
        let mut cmd = create_command(&args[0]);
        if args.len() > 1 {
            cmd.args(&args[1..]);
        }
        cmd.current_dir(&svc.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        for (k, v) in &env {
            cmd.env(k, v);
        }

        // Windows: CREATE_NEW_PROCESS_GROUP
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
            cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
        }

        // Unix: create a new session/process group so stop() can terminate the full tree.
        #[cfg(not(target_os = "windows"))]
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let mut child = cmd.spawn()?;
        let pid = child.id();

        if let Some(stdout) = child.stdout.take() {
            spawn_and_monitor_logger(stdout, stdout_path, svc.log_max_bytes, svc.log_backups);
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_and_monitor_logger(stderr, stderr_path, svc.log_max_bytes, svc.log_backups);
        }

        // Track the child
        self.children.insert(name.to_string(), child);

        // Update state
        self.db
            .upsert(
                name,
                &ServiceUpdate {
                    pid: Some(pid),
                    status: Some("starting".to_string()),
                    cwd: Some(svc.cwd.to_string_lossy().into_owned()),
                    started_at: Some(now_iso()),
                    last_error: Some(String::new()),
                    ..Default::default()
                },
            )
            .map_err(|e| ProcessError::Stack(e.to_string()))?;

        // Brief pause to check for immediate failure
        std::thread::sleep(Duration::from_millis(250));

        let mut died = false;
        let mut exit_code = None;

        if let Some(child) = self.children.get_mut(name) {
            if let Ok(Some(status)) = child.try_wait() {
                died = true;
                exit_code = status.code();
            }
        }
        if died {
            self.children.remove(name);
            self.db
                .clear_pid(
                    name,
                    "failed",
                    exit_code,
                    &format!("exited immediately with {:?}", exit_code),
                )
                .map_err(|e| ProcessError::Stack(e.to_string()))?;
            return Ok(StartResult {
                service: name.to_string(),
                ok: false,
                message: format!(
                    "{}: failed immediately exit={:?}; check logs",
                    name, exit_code
                ),
            });
        }

        // Apply Windows resource limits
        #[cfg(target_os = "windows")]
        {
            if let Err(err) = apply_windows_resource_limits(pid, &svc.resources) {
                if let Some(mut child) = self.children.remove(name) {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                self.db
                    .clear_pid(
                        name,
                        "failed",
                        None,
                        &format!("failed to apply resource limits: {err}"),
                    )
                    .map_err(|e| ProcessError::Stack(e.to_string()))?;
                return Err(err);
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            if svc.resources.memory_mb > 0 || svc.resources.process_count > 0 {
                if let Some(mut child) = self.children.remove(name) {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                self.db
                    .clear_pid(
                        name,
                        "failed",
                        None,
                        "resource limits are not supported on this platform",
                    )
                    .map_err(|e| ProcessError::Stack(e.to_string()))?;
                return Err(ProcessError::Stack(
                    "resource limits are not supported on this platform".into(),
                ));
            }
        }

        // Wait for health check
        if !self.wait_for_health(name)? {
            return Ok(StartResult {
                service: name.to_string(),
                ok: false,
                message: format!("{}: unhealthy after start pid={}; check logs", name, pid),
            });
        }

        Ok(StartResult {
            service: name.to_string(),
            ok: true,
            message: format!("{}: started pid={}", name, pid),
        })
    }

    /// Stop a service gracefully.
    pub fn stop(&mut self, name: &str) -> Result<String, ProcessError> {
        let svc = self
            .stack
            .services
            .get(name)
            .ok_or_else(|| ProcessError::Stack(format!("service '{}' not found", name)))?;

        let row = self
            .db
            .get(name)
            .map_err(|e| ProcessError::Stack(e.to_string()))?;
        let pid = row.as_ref().and_then(|r| r.pid);

        if pid.is_none() {
            self.db
                .clear_pid(name, "stopped", None, "")
                .map_err(|e| ProcessError::Stack(e.to_string()))?;
            return Ok(format!("{}: not running", name));
        }

        let pid = pid.unwrap();
        if !is_pid_running(pid) {
            self.db
                .clear_pid(name, "stopped", None, "")
                .map_err(|e| ProcessError::Stack(e.to_string()))?;
            return Ok(format!("{}: already stopped", name));
        }

        // Graceful shutdown
        #[cfg(target_os = "windows")]
        {
            // Send CTRL_BREAK_EVENT
            unsafe {
                windows_sys::Win32::System::Console::GenerateConsoleCtrlEvent(
                    1, // CTRL_BREAK_EVENT
                    pid,
                );
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGTERM);
            }
        }

        // Wait for graceful shutdown
        let deadline = Instant::now() + Duration::from_secs(svc.stop_grace_seconds as u64);
        while Instant::now() < deadline && is_pid_running(pid) {
            std::thread::sleep(Duration::from_millis(200));
        }

        // Force kill if still running
        if is_pid_running(pid) {
            #[cfg(target_os = "windows")]
            {
                let _ = create_command("taskkill")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }

            #[cfg(not(target_os = "windows"))]
            {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
        }

        // Clean up job handle on Windows
        #[cfg(target_os = "windows")]
        {
            let mut handles = JOB_HANDLES.lock().unwrap();
            if let Some(job) = handles.remove(&pid) {
                unsafe {
                    windows_sys::Win32::Foundation::CloseHandle(job.0);
                }
            }
        }

        self.children.remove(name);
        self.db
            .clear_pid(name, "stopped", None, "")
            .map_err(|e| ProcessError::Stack(e.to_string()))?;
        Ok(format!("{}: stopped", name))
    }

    /// Check if a service is healthy.
    pub fn health_ok(&self, name: &str) -> Result<bool, ProcessError> {
        let svc = self
            .stack
            .services
            .get(name)
            .ok_or_else(|| ProcessError::Stack(format!("service '{}' not found", name)))?;

        let row = self
            .db
            .get(name)
            .map_err(|e| ProcessError::Stack(e.to_string()))?;
        let pid = row.as_ref().and_then(|r| r.pid);

        if let Some(pid) = pid {
            if !is_pid_running(pid) {
                self.db
                    .clear_pid(name, "stopped", None, "")
                    .map_err(|e| ProcessError::Stack(e.to_string()))?;
                return Ok(false);
            }
        }

        let h = &svc.healthcheck;
        match h.r#type.as_str() {
            "process" => Ok(true),
            "tcp" => Ok(check_tcp(&h.host, h.port, h.timeout_seconds)),
            "http" => Ok(check_http(&h.url, h.timeout_seconds)),
            "command" => {
                let (use_shell, args) = match &h.test {
                    Some(serde_json::Value::Array(arr)) => {
                        let mut args: Vec<String> = arr
                            .iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect();
                        if args.is_empty() {
                            return Ok(false);
                        }
                        let mode = args.remove(0);
                        match mode.as_str() {
                            "CMD" => (false, args),
                            "CMD-SHELL" => (true, args),
                            _ => {
                                args.insert(0, mode);
                                (false, args)
                            }
                        }
                    }
                    Some(serde_json::Value::String(s)) => (true, vec![s.clone()]),
                    _ => return Ok(false),
                };

                if args.is_empty() || args.iter().all(|arg| arg.trim().is_empty()) {
                    return Ok(false);
                }

                Ok(run_health_command(&args, use_shell, h.timeout_seconds))
            }
            _ => Ok(false),
        }
    }

    /// Wait for a service to become healthy.
    pub fn wait_for_health(&self, name: &str) -> Result<bool, ProcessError> {
        let svc = self
            .stack
            .services
            .get(name)
            .ok_or_else(|| ProcessError::Stack(format!("service '{}' not found", name)))?;

        let wait_secs = service_health_wait_seconds(svc);
        let deadline = Instant::now() + Duration::from_secs(wait_secs as u64);
        let interval =
            Duration::from_secs(std::cmp::max(1, svc.healthcheck.interval_seconds) as u64);

        let is_process = svc.healthcheck.r#type == "process";
        let min_process_wait =
            Duration::from_secs(std::cmp::max(1, svc.healthcheck.start_period_seconds) as u64);
        let start_time = Instant::now();

        while Instant::now() < deadline {
            let ok = self.health_ok(name)?;

            if ok {
                if !is_process || Instant::now().duration_since(start_time) >= min_process_wait {
                    self.db
                        .upsert(
                            name,
                            &ServiceUpdate {
                                status: Some("running".into()),
                                ..Default::default()
                            },
                        )
                        .map_err(|e| ProcessError::Stack(e.to_string()))?;
                    return Ok(true);
                }
            } else if is_process {
                break; // Process died
            }

            let mut sleep_time = interval;
            if is_process && ok {
                let elapsed = Instant::now().duration_since(start_time);
                if elapsed < min_process_wait {
                    let remaining = min_process_wait - elapsed;
                    sleep_time = std::cmp::min(interval, remaining.max(Duration::from_millis(100)));
                }
            }

            std::thread::sleep(sleep_time);
        }

        self.db
            .upsert(
                name,
                &ServiceUpdate {
                    status: Some("unhealthy".into()),
                    last_error: Some(if is_process {
                        "process died before becoming healthy".into()
                    } else {
                        "healthcheck timeout".into()
                    }),
                    ..Default::default()
                },
            )
            .map_err(|e| ProcessError::Stack(e.to_string()))?;
        Ok(false)
    }

    /// Wait for all dependencies of a service to be ready.
    fn wait_for_dependencies(&self, name: &str) -> Result<bool, ProcessError> {
        let svc = self
            .stack
            .services
            .get(name)
            .ok_or_else(|| ProcessError::Stack(format!("service '{}' not found", name)))?;

        for dep in &svc.depends_on {
            let dep_row = self
                .db
                .get(dep)
                .map_err(|e| ProcessError::Stack(e.to_string()))?;
            match dep_row {
                Some(row) if row.status == "running" => continue,
                _ => {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// Build the command arguments for a service.
    fn build_command_args(&self, name: &str) -> Vec<String> {
        let svc = self.stack.services.get(name).unwrap();
        match &svc.command {
            serde_json::Value::Array(arr) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            serde_json::Value::String(s) => {
                if svc.allow_shell {
                    #[cfg(target_os = "windows")]
                    {
                        vec!["cmd".to_string(), "/C".to_string(), s.clone()]
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        vec!["sh".to_string(), "-c".to_string(), s.clone()]
                    }
                } else {
                    s.split_whitespace().map(|s| s.to_string()).collect()
                }
            }
            _ => vec![],
        }
    }

    /// Prepare the environment for a service.
    fn prepare_env(&self, name: &str) -> HashMap<String, String> {
        let svc = self.stack.services.get(name).unwrap();
        let mut env: HashMap<String, String> = std::env::vars().collect();

        if let Some(env_file) = &svc.env_file {
            if let Ok(file_env) = pystack_compose::parse_env_file(env_file) {
                env.extend(file_env);
            }
        }

        for (k, v) in &svc.env {
            env.insert(k.clone(), v.clone());
        }

        env.insert("PYSTACK_PROJECT".into(), self.stack.project.clone());
        env.insert("PYSTACK_SERVICE".into(), name.to_string());
        env
    }

    /// Get log file paths for a service.
    fn open_log_paths(&self, name: &str) -> (PathBuf, PathBuf) {
        let svc = self.stack.services.get(name).unwrap();
        let _ = std::fs::create_dir_all(&self.stack.log_dir);
        let out_path = self.stack.log_dir.join(format!("{}.out.log", name));
        let err_path = self.stack.log_dir.join(format!("{}.err.log", name));
        rotate_log(&out_path, svc.log_max_bytes, svc.log_backups);
        rotate_log(&err_path, svc.log_max_bytes, svc.log_backups);
        (out_path, err_path)
    }

    /// Teardown volumes by deleting local host directories mapped as volumes for native services.
    pub fn teardown_volumes(&self) -> Result<(), ProcessError> {
        let root =
            self.stack.root.canonicalize().map_err(|e| {
                ProcessError::Stack(format!("failed to canonicalize stack root: {e}"))
            })?;
        for (_name, svc) in &self.stack.services {
            if svc.backend == "native" || svc.backend.is_empty() {
                for vol in &svc.volumes {
                    if let Some((source, _)) = vol.split_once(':') {
                        let path = PathBuf::from(source);
                        if path.is_absolute() {
                            // Do not attempt to delete absolute host mounts.
                            continue;
                        }
                        let target_path = self.stack.root.join(&path);

                        if !target_path.exists() || !target_path.is_dir() {
                            continue;
                        }
                        let canonical = target_path.canonicalize().map_err(|e| {
                            ProcessError::Stack(format!(
                                "failed to canonicalize volume path {}: {e}",
                                target_path.display()
                            ))
                        })?;
                        if !canonical.starts_with(&root) {
                            continue;
                        }
                        std::fs::remove_dir_all(&canonical)?;
                    }
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Health check functions
// ---------------------------------------------------------------------------

fn check_tcp(host: &str, port: u16, timeout: u32) -> bool {
    if port == 0 {
        return false;
    }
    let timeout = health_timeout(timeout);
    let Ok(addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    addrs
        .into_iter()
        .any(|addr| TcpStream::connect_timeout(&addr, timeout).is_ok())
}

fn check_http(url: &str, timeout: u32) -> bool {
    if url.is_empty() {
        return false;
    }
    // Simple HTTP check using a TCP connection and manual HTTP request
    let parsed = url_parse(url);
    if parsed.is_none() {
        return false;
    }
    let (host, port, path, https) = parsed.unwrap();

    let timeout = health_timeout(timeout);
    let Ok(mut addrs) = (host.as_str(), port).to_socket_addrs() else {
        return false;
    };
    let Some(addr) = addrs.next() else {
        return false;
    };
    let Ok(stream) = TcpStream::connect_timeout(&addr, timeout) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    use std::io::{BufRead, BufReader};
    let request = format!(
        "GET {} HTTP/1.0\r\nHost: {}\r\nConnection: close\r\n\r\n",
        path, host
    );

    let mut first_line = String::new();

    if https {
        let Ok(connector) = native_tls::TlsConnector::new() else {
            return false;
        };
        let Ok(tls_stream) = connector.connect(&host, stream) else {
            return false;
        };
        let mut reader = BufReader::new(tls_stream);
        if reader.get_mut().write_all(request.as_bytes()).is_err() {
            return false;
        }
        if reader.read_line(&mut first_line).is_err() {
            return false;
        }
    } else {
        let mut reader = BufReader::new(stream);
        if reader.get_mut().write_all(request.as_bytes()).is_err() {
            return false;
        }
        if reader.read_line(&mut first_line).is_err() {
            return false;
        }
    }

    // Parse HTTP status code
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() >= 2 {
        if let Ok(code) = parts[1].parse::<u16>() {
            return (200..500).contains(&code);
        }
    }
    false
}

fn url_parse(url: &str) -> Option<(String, u16, String, bool)> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let https = url.starts_with("https://");
    let (host_port, path) = rest.split_once('/').unwrap_or((rest, "/"));
    let (host, port) = if host_port.contains(':') {
        let (h, p) = host_port.rsplit_once(':')?;
        (h, p.parse::<u16>().ok()?)
    } else {
        (host_port, if https { 443 } else { 80 })
    };
    Some((host.to_string(), port, format!("/{}", path), https))
}

fn health_timeout(timeout: u32) -> Duration {
    Duration::from_secs(std::cmp::max(1, timeout) as u64)
}

fn run_health_command(args: &[String], use_shell: bool, timeout_seconds: u32) -> bool {
    let mut cmd = if use_shell {
        shell_command(args.join(" "))
    } else {
        let mut cmd = create_command(&args[0]);
        if args.len() > 1 {
            cmd.args(&args[1..]);
        }
        cmd
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let Ok(mut child) = cmd.spawn() else {
        return false;
    };
    let deadline = Instant::now() + health_timeout(timeout_seconds);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

fn shell_command(script: String) -> Command {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = create_command("cmd");
        cmd.args(["/C", &script]);
        cmd
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut cmd = create_command("sh");
        cmd.args(["-c", &script]);
        cmd
    }
}

fn service_health_wait_seconds(svc: &ServiceConfig) -> u32 {
    let h = &svc.healthcheck;
    let retry_window = std::cmp::max(1, h.retries) * std::cmp::max(1, h.interval_seconds);
    std::cmp::max(
        1,
        h.start_period_seconds + std::cmp::max(h.timeout_seconds, retry_window),
    )
}

// ---------------------------------------------------------------------------
// Log rotation
// ---------------------------------------------------------------------------

fn rotate_log(path: &Path, max_bytes: u64, backups: u32) {
    if !path.exists() {
        return;
    }
    let Ok(metadata) = path.metadata() else {
        return;
    };
    if metadata.len() < max_bytes {
        return;
    }

    // Rotate: .log -> .log.1, .log.1 -> .log.2, etc.
    for i in (1..=backups).rev() {
        let src = path.with_extension(if i == 1 {
            "log.1".to_string()
        } else {
            format!("log.{}", i)
        });
        let dst = path.with_extension(format!("log.{}", i + 1));
        if src.exists() {
            let _ = std::fs::rename(&src, &dst);
        }
    }
    let _ = std::fs::rename(path, path.with_extension("log.1"));
}

fn spawn_and_monitor_logger<R: std::io::Read + Send + 'static>(
    mut pipe: R,
    path: PathBuf,
    max_bytes: u64,
    backups: u32,
) {
    let limit = if max_bytes == 0 {
        10 * 1024 * 1024
    } else {
        max_bytes
    };
    std::thread::spawn(move || {
        let mut file = match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(f) => f,
            Err(_) => return,
        };

        let mut bytes_written = file.metadata().map(|m| m.len()).unwrap_or(0);
        let mut buffer = [0u8; 8192];

        loop {
            match pipe.read(&mut buffer) {
                Ok(0) => break, // EOF
                Ok(n) => {
                    if file.write_all(&buffer[..n]).is_err() {
                        break;
                    }
                    bytes_written += n as u64;
                    if bytes_written > limit {
                        let _ = file.sync_all();
                        drop(file);
                        rotate_log(&path, limit, backups);
                        file = match std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&path)
                        {
                            Ok(f) => f,
                            Err(_) => return,
                        };
                        bytes_written = file.metadata().map(|m| m.len()).unwrap_or(0);
                    }
                }
                Err(_) => break,
            }
        }
    });
}

// ---------------------------------------------------------------------------
// PID utilities
// ---------------------------------------------------------------------------

/// Check if a process is still running.
pub fn is_pid_running(pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    {
        let output = create_command("tasklist")
            .args(["/FI", &format!("PID eq {}", pid)])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();
        match output {
            Ok(out) => String::from_utf8_lossy(&out.stdout).contains(&pid.to_string()),
            Err(_) => false,
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
}

// ---------------------------------------------------------------------------
// Windows Job Objects
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn apply_windows_resource_limits(pid: u32, limits: &ResourceLimits) -> Result<(), ProcessError> {
    if limits.memory_mb == 0 && limits.process_count == 0 {
        return Ok(());
    }

    use windows_sys::Win32::System::Threading::OpenProcess;

    const JOB_OBJECT_LIMIT_PROCESS_MEMORY: u32 = 0x100;
    const JOB_OBJECT_LIMIT_ACTIVE_PROCESS: u32 = 0x8;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;
    const PROCESS_SET_QUOTA: u32 = 0x0100;
    const PROCESS_TERMINATE: u32 = 0x0001;

    #[repr(C)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    struct JobObjectBasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    struct JobObjectExtendedLimitInformation {
        basic_limit_information: JobObjectBasicLimitInformation,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    // Raw FFI to kernel32 functions not exposed by windows-sys 0.59 features
    extern "system" {
        fn CreateJobObjectW(
            lpJobAttributes: *mut std::ffi::c_void,
            lpName: *const u16,
        ) -> *mut std::ffi::c_void;
        fn SetInformationJobObject(
            hJob: *mut std::ffi::c_void,
            JobObjectInformationClass: i32,
            lpJobObjectInformation: *const std::ffi::c_void,
            cbJobObjectInformationLength: u32,
        ) -> i32;
        fn AssignProcessToJobObject(
            hJob: *mut std::ffi::c_void,
            hProcess: *mut std::ffi::c_void,
        ) -> i32;
    }

    unsafe fn close_handle(h: *mut std::ffi::c_void) {
        windows_sys::Win32::Foundation::CloseHandle(h);
    }

    unsafe {
        let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
        if job.is_null() {
            return Err(ProcessError::Stack(
                "failed to create Windows Job Object for resource limits".into(),
            ));
        }

        let mut info: JobObjectExtendedLimitInformation = std::mem::zeroed();
        if limits.memory_mb > 0 {
            info.basic_limit_information.limit_flags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
            info.process_memory_limit = (limits.memory_mb as usize) * 1024 * 1024;
        }
        if limits.process_count > 0 {
            info.basic_limit_information.limit_flags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
            info.basic_limit_information.active_process_limit = limits.process_count;
        }

        let ok = SetInformationJobObject(
            job,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
        );
        if ok == 0 {
            close_handle(job);
            return Err(ProcessError::Stack(
                "failed to configure Windows Job Object resource limits".into(),
            ));
        }

        let handle = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
        if handle.is_null() {
            close_handle(job);
            return Err(ProcessError::Stack(format!(
                "failed to open process {} for resource limit assignment",
                pid
            )));
        }

        let ok = AssignProcessToJobObject(job, handle);
        windows_sys::Win32::Foundation::CloseHandle(handle);
        if ok == 0 {
            close_handle(job);
            return Err(ProcessError::Stack(format!(
                "failed to assign process {} to Windows Job Object",
                pid
            )));
        }

        JOB_HANDLES
            .lock()
            .unwrap()
            .insert(pid, JobHandle(job as *mut _));
    }

    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn apply_windows_resource_limits(_pid: u32, _limits: &ResourceLimits) -> Result<(), ProcessError> {
    Ok(())
}

/// Get the current timestamp as ISO 8601.
fn now_iso() -> String {
    let now = std::time::SystemTime::now();
    let datetime: chrono::DateTime<chrono::Utc> = now.into();
    datetime.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_parse() {
        let (host, port, path, https) = url_parse("http://localhost:8080/health").unwrap();
        assert_eq!(host, "localhost");
        assert_eq!(port, 8080);
        assert_eq!(path, "/health");
        assert!(!https);

        let (host, port, _, https) = url_parse("https://example.com/api").unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 443);
        assert!(https);
    }

    #[test]
    fn test_rotate_log_no_file() {
        let dir = std::env::temp_dir().join("pystack_rotate_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.log");
        // Should not panic on nonexistent file
        rotate_log(&path, 100, 3);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_rotate_log_under_limit() {
        let dir = std::env::temp_dir().join("pystack_rotate_test2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.log");
        std::fs::write(&path, "small content").unwrap();
        rotate_log(&path, 1_000_000, 3);
        assert!(path.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_teardown_volumes_does_not_delete_outside_root() {
        let base = std::env::temp_dir().join("pystack_volume_safety_test");
        let root = base.join("project");
        let outside = base.join("important");
        let inside = root.join("data");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let mut services = HashMap::new();
        services.insert(
            "web".to_string(),
            ServiceConfig {
                name: "web".into(),
                cwd: root.clone(),
                command: serde_json::json!(["echo", "ok"]),
                volumes: vec!["data:/data".into(), "../important:/important".into()],
                ..Default::default()
            },
        );
        let stack = StackConfig {
            project: "volume-safety".into(),
            root: root.clone(),
            config_path: root.join("stack.json"),
            state_dir: root.join(pystack_types::DEFAULT_STATE_DIR),
            log_dir: root.join(pystack_types::DEFAULT_LOG_DIR),
            services,
            volumes: HashMap::new(),
            secrets: HashMap::new(),
            configs: HashMap::new(),
            source_format: "stackdeck".into(),
        };
        let mgr = ProcessManager::new(stack).unwrap();

        mgr.teardown_volumes().unwrap();

        assert!(!inside.exists());
        assert!(outside.exists());
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn test_service_health_wait_seconds() {
        let svc = ServiceConfig {
            name: "test".into(),
            cwd: PathBuf::from("."),
            command: serde_json::json!(["echo"]),
            healthcheck: HealthCheck {
                timeout_seconds: 10,
                interval_seconds: 2,
                retries: 3,
                start_period_seconds: 5,
                ..Default::default()
            },
            ..Default::default()
        };
        let secs = service_health_wait_seconds(&svc);
        // start_period(5) + max(timeout(10), retries(3) * interval(2)) = 5 + 10 = 15
        assert_eq!(secs, 15);
    }
}
