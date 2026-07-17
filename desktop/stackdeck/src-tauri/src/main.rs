#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

fn create_command<S: AsRef<std::ffi::OsStr>>(program: S) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    cmd
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjectEntry {
    name: String,
    root: String,
    config: String,
    backend: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServiceEntry {
    name: String,
    image: String,
    backend: String,
    ports: Vec<String>,
    urls: Vec<String>,
    endpoints: Vec<EndpointEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EndpointEntry {
    host_port: u16,
    target_port: u16,
    protocol: String,
    label: String,
    url: Option<String>,
    browser_safe: bool,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeOverview {
    containers: String,
    images: String,
    volumes: String,
    networks: String,
    vm_health: String,
}

#[derive(Debug, Clone, Serialize)]
struct CommandResult {
    exit_code: i32,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

#[derive(Debug, Clone, Serialize)]
struct SystemStats {
    /// 0.0 – 100.0 global CPU usage (average of all logical cores)
    cpu_usage: f32,
    /// Per-core usage percentages
    cpu_cores: Vec<f32>,
    /// Used RAM in bytes
    used_memory: u64,
    /// Total RAM in bytes
    total_memory: u64,
    /// Number of logical CPU cores
    cpu_count: usize,
}

#[tauri::command]
fn list_services(project_name: String) -> Result<Vec<ServiceEntry>, String> {
    let project = find_project(&project_name)?;
    Ok(read_project_services(&project))
}

#[tauri::command]
async fn service_action(
    project_name: String,
    action: String,
    service_name: Option<String>,
) -> Result<CommandResult, String> {
    match action.as_str() {
        "up" | "down" | "restart" | "status" | "logs" => {}
        _ => return Err(format!("Unsupported action: {action}")),
    }
    let project = find_project(&project_name)?;
    if action == "logs"
        && service_name
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
    {
        return Err("Select a service before opening logs.".to_string());
    }
    let mut args = vec![
        "--config".to_string(),
        project.config.clone(),
        "--backend".to_string(),
        project.backend.clone(),
    ];
    args.push(action.clone());
    if let Some(service) = service_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.push(service.to_string());
    }
    if action == "logs" {
        args.push("--tail".to_string());
        args.push("200".to_string());
    }
    let timeout = command_timeout(&action);
    tauri::async_runtime::spawn_blocking(move || {
        if action == "restart" {
            return restart_hive_service(PathBuf::from(project.root), args, timeout);
        }
        run_hive_in(PathBuf::from(project.root), args, timeout)
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
async fn clear_project_volumes(
    project_name: String,
    confirmation: String,
) -> Result<CommandResult, String> {
    if confirmation != "CLEAR_VOLUMES" {
        return Err("Volume clearing requires backend confirmation.".to_string());
    }
    let project = find_project(&project_name)?;
    let args = vec![
        "--config".to_string(),
        project.config.clone(),
        "--backend".to_string(),
        project.backend.clone(),
        "down".to_string(),
        "--volumes".to_string(),
    ];
    tauri::async_runtime::spawn_blocking(move || {
        run_hive_in(PathBuf::from(project.root), args, Duration::from_secs(120))
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
fn hyperv_health() -> RuntimeOverview {
    collect_hyperv_overview()
}

static SYSTEM: std::sync::LazyLock<std::sync::Mutex<sysinfo::System>> =
    std::sync::LazyLock::new(|| {
        std::sync::Mutex::new(System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
                .with_memory(MemoryRefreshKind::nothing().with_ram()),
        ))
    });

#[tauri::command]
fn system_stats() -> SystemStats {
    let mut sys = match SYSTEM.lock() {
        Ok(s) => s,
        Err(e) => e.into_inner(),
    };
    sys.refresh_cpu_usage();
    sys.refresh_memory();

    let cores: Vec<f32> = sys.cpus().iter().map(|c| c.cpu_usage()).collect();
    let cpu_usage = if cores.is_empty() {
        0.0
    } else {
        cores.iter().copied().sum::<f32>() / cores.len() as f32
    };

    SystemStats {
        cpu_usage,
        cpu_cores: cores,
        used_memory: sys.used_memory(),
        total_memory: sys.total_memory(),
        cpu_count: sys.cpus().len(),
    }
}

#[tauri::command]
fn open_service_url(
    project_name: String,
    service_name: String,
    host_port: u16,
) -> Result<String, String> {
    let project = find_project(&project_name)?;
    let services = read_project_services(&project);
    let endpoint = services
        .iter()
        .find(|service| service.name == service_name)
        .and_then(|service| {
            service
                .endpoints
                .iter()
                .find(|endpoint| endpoint.host_port == host_port && endpoint.browser_safe)
        })
        .ok_or_else(|| "Endpoint is not managed by StackDeck.".to_string())?;
    let url = endpoint
        .url
        .clone()
        .ok_or_else(|| "Endpoint does not have an openable URL.".to_string())?;
    let (host, port) = validate_service_url(&url)?;
    if !is_service_port_reachable(&host, port) {
        return Err(format!("Port {port} is not reachable on {host}. Start the service and wait for the port to publish before opening it."));
    }
    Ok(open_url(&url))
}

fn validate_service_url(url: &str) -> Result<(String, u16), String> {
    if url.chars().any(|ch| {
        ch.is_control()
            || ch.is_whitespace()
            || matches!(ch, '"' | '\'' | '<' | '>' | '^' | '&' | '|')
    }) {
        return Err("Service URL contains unsupported characters.".to_string());
    }

    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or_else(|| {
            "Only HTTP and HTTPS service URLs can be opened from StackDeck.".to_string()
        })?;

    let host_port = rest
        .split(['/', '?', '#'])
        .next()
        .filter(|text| !text.is_empty())
        .ok_or_else(|| "Service URL must include a host and port.".to_string())?;

    let (host, port_text) = host_port
        .rsplit_once(':')
        .filter(|(host, port)| !host.is_empty() && !port.is_empty())
        .ok_or_else(|| "Service URL must include a host and port.".to_string())?;

    if !is_allowed_service_host(host) {
        return Err(format!("Service host {host} is not managed by StackDeck."));
    }

    let port = port_text
        .parse::<u16>()
        .map_err(|_| "Service URL must include a valid port.".to_string())?;
    if port == 0 {
        return Err("Service URL port must be greater than zero.".to_string());
    }

    Ok((host.to_string(), port))
}

fn is_allowed_service_host(host: &str) -> bool {
    if matches!(host, "127.0.0.1" | "localhost") {
        return true;
    }

    pystack_hyperv::HyperVManager::load_config()
        .map(|cfg| !cfg.ssh_host.is_empty() && host == cfg.ssh_host)
        .unwrap_or(false)
}

fn find_project(project_name: &str) -> Result<ProjectEntry, String> {
    read_projects()?
        .into_iter()
        .find(|project| project.name == project_name)
        .ok_or_else(|| format!("Project not found: {project_name}"))
}

#[tauri::command]
fn read_projects() -> Result<Vec<ProjectEntry>, String> {
    let path = pystack_types::registry_file();
    let text = std::fs::read_to_string(path).unwrap_or_else(|_| "{}".to_string());
    let raw: HashMap<String, serde_json::Value> = serde_json::from_str(&text).unwrap_or_default();
    let mut projects = raw
        .into_iter()
        .map(|(name, value)| ProjectEntry {
            name,
            root: value
                .get("root")
                .and_then(|v| v.as_str())
                .unwrap_or(".")
                .to_string(),
            config: value
                .get("config")
                .and_then(|v| v.as_str())
                .unwrap_or("stack.json")
                .to_string(),
            backend: value
                .get("backend")
                .and_then(|v| v.as_str())
                .unwrap_or("native")
                .to_string(),
        })
        .collect::<Vec<_>>();
    projects.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(projects)
}

fn read_project_services(project: &ProjectEntry) -> Vec<ServiceEntry> {
    let config = project_config_path(project);
    let ext = config
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(ext.as_str(), "yaml" | "yml") {
        return read_compose_services(project, &config);
    }
    read_stack_json_services(project, &config)
}

fn project_config_path(project: &ProjectEntry) -> PathBuf {
    let config = PathBuf::from(&project.config);
    if config.is_absolute() {
        config
    } else {
        PathBuf::from(&project.root).join(config)
    }
}

fn read_compose_services(project: &ProjectEntry, config: &PathBuf) -> Vec<ServiceEntry> {
    let parsed = match pystack_compose::load_compose_file(config, None, &[], None, None) {
        Ok(parsed) => parsed,
        Err(err) => {
            return vec![ServiceEntry {
                name: "Could not load Compose file".to_string(),
                image: err.to_string(),
                backend: project.backend.clone(),
                ports: Vec::new(),
                urls: Vec::new(),
                endpoints: Vec::new(),
            }]
        }
    };
    let host = endpoint_host(project);
    let mut services = parsed
        .services
        .values()
        .map(|service| {
            let ports = service
                .ports
                .iter()
                .map(|port| {
                    let published = port.published.unwrap_or(port.target);
                    let mut value = format!("{published}:{}", port.target);
                    if !port.protocol.eq_ignore_ascii_case("tcp") {
                        value.push('/');
                        value.push_str(&port.protocol);
                    }
                    value
                })
                .collect::<Vec<_>>();
            let endpoints = service
                .ports
                .iter()
                .map(|port| {
                    endpoint_from_parts(
                        port.published.unwrap_or(port.target),
                        port.target,
                        &port.protocol,
                        port.app_protocol.as_deref(),
                        &host,
                    )
                })
                .collect::<Vec<_>>();
            ServiceEntry {
                name: service.name.clone(),
                image: service.image.clone().unwrap_or_else(|| {
                    if service.build.is_some() {
                        "build".to_string()
                    } else {
                        String::new()
                    }
                }),
                backend: project.backend.clone(),
                urls: endpoint_urls(&endpoints),
                endpoints,
                ports,
            }
        })
        .collect::<Vec<_>>();
    services.sort_by(|a, b| a.name.cmp(&b.name));
    services
}

fn read_stack_json_services(project: &ProjectEntry, config: &PathBuf) -> Vec<ServiceEntry> {
    let text = match std::fs::read_to_string(config) {
        Ok(text) => text,
        Err(err) => {
            return vec![ServiceEntry {
                name: "Could not load stack config".to_string(),
                image: err.to_string(),
                backend: project.backend.clone(),
                ports: Vec::new(),
                urls: Vec::new(),
                endpoints: Vec::new(),
            }]
        }
    };
    let raw: serde_json::Value = match serde_json::from_str(&text) {
        Ok(raw) => raw,
        Err(err) => {
            return vec![ServiceEntry {
                name: "Could not parse stack config".to_string(),
                image: err.to_string(),
                backend: project.backend.clone(),
                ports: Vec::new(),
                urls: Vec::new(),
                endpoints: Vec::new(),
            }]
        }
    };
    let Some(services_raw) = raw.get("services").and_then(|value| value.as_object()) else {
        return Vec::new();
    };
    let mut services = services_raw
        .iter()
        .map(|(name, service)| {
            let ports = service
                .get("ports")
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(ToString::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let endpoints = endpoints_from_port_specs(&ports, &endpoint_host(project));
            ServiceEntry {
                name: name.clone(),
                image: service
                    .get("image")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string(),
                backend: service
                    .get("backend")
                    .and_then(|value| value.as_str())
                    .unwrap_or(&project.backend)
                    .to_string(),
                urls: endpoint_urls(&endpoints),
                endpoints,
                ports,
            }
        })
        .collect::<Vec<_>>();
    services.sort_by(|a, b| a.name.cmp(&b.name));
    services
}

fn endpoint_host(project: &ProjectEntry) -> String {
    if project.backend.eq_ignore_ascii_case("hyperv") {
        if let Ok(cfg) = pystack_hyperv::HyperVManager::load_config() {
            if !cfg.ssh_host.trim().is_empty() {
                return cfg.ssh_host;
            }
        }
    }
    "127.0.0.1".to_string()
}

fn endpoints_from_port_specs(ports: &[String], host: &str) -> Vec<EndpointEntry> {
    ports
        .iter()
        .filter_map(|port| {
            port_from_spec(port)
                .map(|parsed| endpoint_from_parts(parsed.0, parsed.1, &parsed.2, None, host))
        })
        .collect()
}

fn endpoint_urls(endpoints: &[EndpointEntry]) -> Vec<String> {
    endpoints
        .iter()
        .filter_map(|endpoint| endpoint.url.clone())
        .collect()
}

fn port_from_spec(port: &str) -> Option<(u16, u16, String)> {
    let text = port.trim().trim_matches('"').trim_matches('\'');
    if text.is_empty() {
        return None;
    }
    let (without_proto, protocol) = text
        .split_once('/')
        .map(|(value, proto)| (value, proto))
        .unwrap_or((text, "tcp"));
    let parts = without_proto.split(':').collect::<Vec<_>>();
    match parts.as_slice() {
        [only] => only
            .parse()
            .ok()
            .map(|port| (port, port, protocol.to_ascii_lowercase())),
        [host, container] => host
            .parse()
            .ok()
            .zip(container.parse().ok())
            .map(|(host, target)| (host, target, protocol.to_ascii_lowercase())),
        [_ip, host, container] => host
            .parse()
            .ok()
            .zip(container.parse().ok())
            .map(|(host, target)| (host, target, protocol.to_ascii_lowercase())),
        _ => None,
    }
}

fn endpoint_from_parts(
    host_port: u16,
    target_port: u16,
    protocol: &str,
    app_protocol: Option<&str>,
    host: &str,
) -> EndpointEntry {
    let label = endpoint_label(host_port, target_port, protocol, app_protocol);
    let browser_safe = can_open_in_browser(host_port, target_port, &label);
    EndpointEntry {
        host_port,
        target_port,
        protocol: protocol.to_ascii_lowercase(),
        label,
        url: browser_safe.then(|| {
            let scheme = if matches!(target_port, 443 | 8443) {
                "https"
            } else {
                "http"
            };
            format!("{scheme}://{host}:{host_port}")
        }),
        browser_safe,
    }
}

fn endpoint_label(
    host_port: u16,
    target_port: u16,
    protocol: &str,
    app_protocol: Option<&str>,
) -> String {
    if let Some(app_protocol) = app_protocol
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return app_protocol.to_ascii_uppercase();
    }

    if protocol.eq_ignore_ascii_case("udp") {
        return "UDP".to_string();
    }

    match target_port {
        443 | 8443 => "HTTPS".to_string(),
        80 | 3000 | 3001 | 5000 | 5173 | 8000 | 8080 | 8081 | 8888 | 8889 | 9000 | 9090 | 9091
        | 9093 | 9997 => "HTTP".to_string(),
        5432 => "PostgreSQL".to_string(),
        6379 => "Redis".to_string(),
        8554 => "RTSP".to_string(),
        1935 => "RTMP".to_string(),
        _ => match host_port {
            443 | 8443 => "HTTPS".to_string(),
            80 | 3000 | 3001 | 5000 | 5173 | 8000 | 8080 | 8081 | 8888 | 8889 | 9000 | 9090
            | 9091 | 9093 | 9997 => "HTTP".to_string(),
            5432 => "PostgreSQL".to_string(),
            6379 => "Redis".to_string(),
            8554 => "RTSP".to_string(),
            1935 => "RTMP".to_string(),
            _ => protocol.to_ascii_uppercase(),
        },
    }
}

fn can_open_in_browser(host_port: u16, target_port: u16, label: &str) -> bool {
    if matches!(label, "HTTP" | "HTTPS") {
        return true;
    }
    matches!(
        target_port,
        80 | 443
            | 3000
            | 3001
            | 5000
            | 5173
            | 8000
            | 8080
            | 8081
            | 8443
            | 8888
            | 8889
            | 9000
            | 9090
            | 9091
            | 9093
            | 9997
    ) || matches!(
        host_port,
        80 | 443
            | 3000
            | 3001
            | 5000
            | 5173
            | 8000
            | 8080
            | 8081
            | 8443
            | 8888
            | 8889
            | 9000
            | 9090
            | 9091
            | 9093
            | 9997
    )
}

fn is_service_port_reachable(host: &str, port: u16) -> bool {
    let addresses = (host, port).to_socket_addrs();
    let Ok(addresses) = addresses else {
        return false;
    };
    addresses
        .into_iter()
        .any(|address| TcpStream::connect_timeout(&address, Duration::from_millis(700)).is_ok())
}

fn collect_hyperv_overview() -> RuntimeOverview {
    let cfg = match pystack_hyperv::HyperVManager::load_config() {
        Ok(cfg) => cfg,
        Err(err) => {
            return RuntimeOverview {
                containers: String::new(),
                images: String::new(),
                volumes: String::new(),
                networks: String::new(),
                vm_health: format!("Could not load Hyper-V config: {err}"),
            };
        }
    };
    let mgr = pystack_hyperv::HyperVManager::new(cfg);
    RuntimeOverview {
        containers: mgr.container_ps().unwrap_or_else(|err| err.to_string()),
        images: mgr.image_list(false).unwrap_or_else(|err| err.to_string()),
        volumes: mgr.volume_list().unwrap_or_else(|err| err.to_string()),
        networks: mgr.network_list().unwrap_or_else(|err| err.to_string()),
        vm_health: mgr
            .runtime_health_check()
            .map(|health| serde_json::to_string_pretty(&health).unwrap_or_default())
            .unwrap_or_else(|err| err.to_string()),
    }
}

fn open_url(url: &str) -> String {
    let result = if cfg!(target_os = "windows") {
        create_command("cmd")
            .args(["/C", "start", "", url])
            .status()
    } else if cfg!(target_os = "macos") {
        create_command("open").arg(url).status()
    } else {
        create_command("xdg-open").arg(url).status()
    };
    match result {
        Ok(status) if status.success() => format!("Opened {url}"),
        Ok(status) => format!("Open browser exited with {status} for {url}"),
        Err(err) => format!("Could not open {url}: {err}"),
    }
}

fn command_timeout(action: &str) -> Duration {
    match action {
        "logs" | "status" => Duration::from_secs(20),
        "down" | "restart" => Duration::from_secs(90),
        _ => Duration::from_secs(180),
    }
}

fn run_hive_in(
    cwd: PathBuf,
    args: Vec<String>,
    timeout: Duration,
) -> Result<CommandResult, String> {
    let exe = find_hive_exe().ok_or_else(|| {
        "Could not locate stackdeck-hive.exe. Build and package StackDeck with npm run package:windows.".to_string()
    })?;
    let output = run_command_with_timeout(exe, cwd, args, timeout)?;
    Ok(CommandResult {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: bounded_output(&output.stdout),
        stderr: bounded_output(&output.stderr),
        timed_out: output.timed_out,
    })
}

fn restart_hive_service(
    cwd: PathBuf,
    args: Vec<String>,
    timeout: Duration,
) -> Result<CommandResult, String> {
    let Some(service) = args.last().cloned() else {
        return Err("Select a service before restarting.".to_string());
    };
    if service == "restart" {
        return Err("Select a service before restarting.".to_string());
    }

    let mut down_args = args.clone();
    if let Some(action) = down_args.iter_mut().find(|item| item.as_str() == "restart") {
        *action = "down".to_string();
    }
    let mut up_args = args;
    if let Some(action) = up_args.iter_mut().find(|item| item.as_str() == "restart") {
        *action = "up".to_string();
    }

    let exe = find_hive_exe().ok_or_else(|| {
        "Could not locate stackdeck-hive.exe. Build and package StackDeck with npm run package:windows.".to_string()
    })?;
    let down = run_command_with_timeout(exe.clone(), cwd.clone(), down_args, timeout)?;
    let up = if down.status.success() {
        Some(run_command_with_timeout(exe, cwd, up_args, timeout)?)
    } else {
        None
    };

    let mut stdout = format!("restart step: stop\n{}", bounded_output(&down.stdout));
    let mut stderr = bounded_output(&down.stderr);
    let mut exit_code = down.status.code().unwrap_or(-1);
    let mut timed_out = down.timed_out;

    if let Some(up) = up {
        stdout.push_str("\n\nrestart step: start\n");
        stdout.push_str(&bounded_output(&up.stdout));
        let up_stderr = bounded_output(&up.stderr);
        if !up_stderr.is_empty() {
            if !stderr.is_empty() {
                stderr.push_str("\n\n");
            }
            stderr.push_str(&up_stderr);
        }
        exit_code = up.status.code().unwrap_or(-1);
        timed_out = timed_out || up.timed_out;
    }

    Ok(CommandResult {
        exit_code,
        stdout,
        stderr,
        timed_out,
    })
}

struct TimedOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
}

fn run_command_with_timeout(
    exe: PathBuf,
    cwd: PathBuf,
    args: Vec<String>,
    timeout: Duration,
) -> Result<TimedOutput, String> {
    let mut child = create_command(exe)
        .current_dir(cwd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| err.to_string())?;
    let child_id = child.id();
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not capture command stdout.".to_string())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Could not capture command stderr.".to_string())?;
    let stdout_reader = std::thread::spawn(move || {
        let mut data = Vec::new();
        let _ = stdout.read_to_end(&mut data);
        data
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut data = Vec::new();
        let _ = stderr.read_to_end(&mut data);
        data
    });

    let start = Instant::now();
    let mut timed_out = false;
    loop {
        if let Some(status) = child.try_wait().map_err(|err| err.to_string())? {
            let stdout = stdout_reader.join().unwrap_or_default();
            let stderr = stderr_reader.join().unwrap_or_default();
            return Ok(TimedOutput {
                status,
                stdout,
                stderr,
                timed_out,
            });
        }
        if start.elapsed() >= timeout {
            timed_out = true;
            kill_process_tree(child_id);
            let _ = child.kill();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn kill_process_tree(pid: u32) {
    if cfg!(target_os = "windows") {
        let _ = create_command("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn bounded_output(bytes: &[u8]) -> String {
    const MAX_CHARS: usize = 80_000;
    let text = String::from_utf8_lossy(bytes).trim().to_string();
    if text.chars().count() <= MAX_CHARS {
        return text;
    }
    let tail = text
        .chars()
        .rev()
        .take(MAX_CHARS)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("[output truncated to last {MAX_CHARS} characters]\n{tail}")
}

fn find_hive_exe() -> Option<PathBuf> {
    #[cfg(not(debug_assertions))]
    {
        let current = std::env::current_exe().ok()?;
        let exe_dir = current.parent()?;
        for candidate in [
            exe_dir.join("stackdeck-hive.exe"),
            exe_dir.join("stackdeck-hive"),
            exe_dir.join("stackdeck-hive-x86_64-pc-windows-msvc.exe"),
            exe_dir.join("bin").join("stackdeck-hive.exe"),
            exe_dir.join("bin").join("stackdeck-hive"),
            exe_dir
                .join("bin")
                .join("stackdeck-hive-x86_64-pc-windows-msvc.exe"),
            exe_dir
                .join("resources")
                .join("bin")
                .join("stackdeck-hive.exe"),
            exe_dir
                .join("resources")
                .join("bin")
                .join("stackdeck-hive-x86_64-pc-windows-msvc.exe"),
            exe_dir
                .join("..")
                .join("resources")
                .join("bin")
                .join("stackdeck-hive.exe"),
            exe_dir
                .join("..")
                .join("resources")
                .join("bin")
                .join("stackdeck-hive-x86_64-pc-windows-msvc.exe"),
            exe_dir
                .join("..")
                .join("Resources")
                .join("bin")
                .join("stackdeck-hive.exe"),
            exe_dir
                .join("..")
                .join("Resources")
                .join("bin")
                .join("stackdeck-hive-x86_64-pc-windows-msvc.exe"),
        ] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        return None;
    }

    #[cfg(debug_assertions)]
    {
        if let Ok(path) = std::env::var("STACKDECK_HIVE_EXE") {
            let path = PathBuf::from(path);
            if path.is_file() {
                return Some(path);
            }
        }

        let current = std::env::current_exe().ok()?;
        let exe_dir = current.parent()?;
        for candidate in [
            exe_dir.join("stackdeck-hive.exe"),
            exe_dir.join("stackdeck-hive"),
            exe_dir.join("bin").join("stackdeck-hive.exe"),
            exe_dir.join("bin").join("stackdeck-hive"),
            exe_dir.join("stackdeck-hive-x86_64-pc-windows-msvc.exe"),
            exe_dir
                .join("bin")
                .join("stackdeck-hive-x86_64-pc-windows-msvc.exe"),
            exe_dir.join("hive.exe"),
            exe_dir.join("hive"),
            exe_dir.join("bin").join("hive.exe"),
            exe_dir.join("bin").join("hive"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("bin")
                .join("stackdeck-hive.exe"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("bin")
                .join("stackdeck-hive-x86_64-pc-windows-msvc.exe"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("..")
                .join("target")
                .join("debug")
                .join("hive.exe"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("..")
                .join("target")
                .join("release")
                .join("hive.exe"),
        ] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }
}

#[tauri::command]
async fn unregister_project(
    project_name: String,
    confirmation: String,
) -> Result<CommandResult, String> {
    if confirmation != "UNREGISTER" {
        return Err("Project unregister requires backend confirmation.".to_string());
    }

    let args = vec!["unregister".to_string(), project_name.clone()];
    let timeout = Duration::from_secs(30);
    tauri::async_runtime::spawn_blocking(move || {
        let exe =
            find_hive_exe().ok_or_else(|| "Could not locate stackdeck-hive.exe.".to_string())?;
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let output = run_command_with_timeout(exe, cwd, args, timeout)?;
        Ok(CommandResult {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: bounded_output(&output.stdout),
            stderr: bounded_output(&output.stderr),
            timed_out: output.timed_out,
        })
    })
    .await
    .map_err(|err| err.to_string())?
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            read_projects,
            list_services,
            service_action,
            clear_project_volumes,
            unregister_project,
            hyperv_health,
            open_service_url,
            system_stats
        ])
        .run(tauri::generate_context!())
        .expect("failed to run StackDeck");
}
