//! StackDeck CLI entry point.
//!
//! Replaces `__main__.py` and the argparse CLI from `core.py`.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_CONFIG: &str = "stack.json";

#[derive(Parser)]
#[command(
    name = "stackdeck",
    version = VERSION,
    about = "WSL-free, Docker Desktop-free Hyper-V container runtime"
)]
struct Cli {
    /// Path to stack.json
    #[arg(long, global = true, default_value = DEFAULT_CONFIG)]
    config: String,

    /// Backend override for Compose files
    #[arg(long, global = true, default_value = "")]
    backend: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new stack.json
    Init {
        #[arg(long)]
        force: bool,
    },

    /// Validate stack configuration
    Validate,

    /// Show system readiness checks
    Doctor,

    /// Start services
    Up {
        /// Services to start (default: all)
        services: Vec<String>,

        /// Force restart even if already running
        #[arg(long)]
        force: bool,

        /// Keep running and enforce restart policies
        #[arg(long)]
        supervise: bool,

        /// Supervisor polling interval in seconds
        #[arg(long, default_value_t = 10)]
        interval: u32,
    },

    /// Stop services
    Down {
        /// Services to stop (default: all)
        services: Vec<String>,

        /// Remove declared non-external Hyper-V named volumes
        #[arg(short = 'v', long)]
        volumes: bool,
    },

    /// Restart services
    Restart { services: Vec<String> },

    /// Show service status
    Status {
        #[arg(long)]
        json: bool,
    },

    /// Show service logs
    Logs {
        /// Service name
        service: String,

        /// Number of lines to show
        #[arg(long, default_value_t = 100)]
        tail: u32,
    },

    /// Docker-like container list (`docker ps`)
    Ps {
        #[arg(short = 'a', long)]
        all: bool,
    },

    /// Docker-like image list (`docker images`)
    Images {
        #[arg(short = 'a', long)]
        all: bool,
    },

    /// Docker-like image pull (`docker pull`)
    Pull { image: String },

    /// Docker-like image removal (`docker rmi`)
    Rmi {
        #[arg(short = 'f', long)]
        force: bool,
        images: Vec<String>,
    },

    /// Docker-like image build (`docker build`)
    Build {
        #[arg(short = 't', long = "tag")]
        tag: Vec<String>,
        #[arg(short = 'f', long, default_value = "Dockerfile")]
        file: String,
        #[arg(long = "build-arg")]
        build_arg: Vec<String>,
        #[arg(default_value = ".")]
        context: String,
    },

    /// Docker-like registry login (`docker login`)
    Login {
        #[arg(default_value = "docker.io")]
        registry: String,
        #[arg(short = 'u', long)]
        username: String,
        #[arg(short = 'p', long)]
        password: String,
    },

    /// Docker-like container inspect
    Inspect { containers: Vec<String> },

    /// Docker-like container exec
    Exec {
        container: String,
        #[arg(trailing_var_arg = true)]
        command: Vec<String>,
    },

    /// Docker-like container start
    Start { containers: Vec<String> },

    /// Docker-like container stop
    Stop { containers: Vec<String> },

    /// Docker-like container removal
    Rm {
        #[arg(short = 'f', long)]
        force: bool,
        containers: Vec<String>,
    },

    /// Docker-like volume commands
    Volume {
        #[command(subcommand)]
        volume_cmd: VolumeCommands,
    },

    /// Docker-like network commands
    Network {
        #[command(subcommand)]
        network_cmd: NetworkCommands,
    },

    /// Write a redacted diagnostics bundle
    Diagnostics {
        #[arg(long, default_value = "")]
        output: String,

        #[arg(long, default_value_t = 120)]
        tail: u32,

        /// Write a single text report instead of a zip bundle
        #[arg(long)]
        text: bool,
    },

    /// Watch and restart services
    Watch {
        #[arg(long, default_value_t = 10)]
        interval: u32,
    },

    /// Register a project
    Register {
        #[arg(long, default_value = "")]
        name: String,

        #[arg(long, default_value = ".")]
        path: String,

        #[arg(long)]
        backend: Option<String>,

        #[arg(long)]
        allow_invalid: bool,
    },

    /// Unregister a project
    Unregister { name: String },

    /// Start the web GUI
    Gui {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        #[arg(long, default_value_t = 8787)]
        port: u16,
    },

    /// Docker Compose-like commands
    Compose {
        #[command(subcommand)]
        compose_cmd: ComposeCommands,
    },

    /// Manage the Hyper-V Linux container backend
    Hyperv {
        #[command(subcommand)]
        hyperv_cmd: HypervCommands,
    },

    /// Run Docker Engine-compatible local API shim
    Daemon {
        #[command(subcommand)]
        daemon_cmd: DaemonCommands,
    },
}

#[derive(Subcommand)]
enum ComposeCommands {
    /// Start compose services
    Up {
        #[arg(short = 'f', long, default_value = "docker-compose.yml")]
        file: String,

        #[arg(long, default_value = "hyperv")]
        backend: String,

        /// Run in background
        #[arg(short = 'd', long)]
        detach: bool,

        #[arg(long)]
        build: bool,

        #[arg(long, default_value_t = 10)]
        interval: u32,

        services: Vec<String>,
    },

    /// Build or rebuild compose services
    Build {
        #[arg(short = 'f', long, default_value = "docker-compose.yml")]
        file: String,

        #[arg(long, default_value = "hyperv")]
        backend: String,

        services: Vec<String>,
    },

    /// Stop compose services
    Down {
        #[arg(short = 'f', long, default_value = "docker-compose.yml")]
        file: String,

        #[arg(long, default_value = "hyperv")]
        backend: String,

        #[arg(short = 'v', long)]
        volumes: bool,

        services: Vec<String>,
    },

    /// Show compose service status
    Status {
        #[arg(short = 'f', long, default_value = "docker-compose.yml")]
        file: String,

        #[arg(long, default_value = "hyperv")]
        backend: String,

        #[arg(long)]
        json: bool,
    },

    /// Show compose service logs
    Logs {
        #[arg(short = 'f', long, default_value = "docker-compose.yml")]
        file: String,

        #[arg(long, default_value = "hyperv")]
        backend: String,

        service: String,

        #[arg(long, default_value_t = 100)]
        tail: u32,
    },
}

#[derive(Subcommand)]
enum HypervCommands {
    /// Show Hyper-V readiness checks
    Doctor,

    /// Configure Hyper-V backend settings
    Configure {
        #[arg(long)]
        vm_name: Option<String>,
        #[arg(long)]
        ssh_host: Option<String>,
        #[arg(long)]
        ssh_user: Option<String>,
        #[arg(long)]
        ssh_port: Option<u16>,
        #[arg(long)]
        ssh_identity: Option<String>,
        #[arg(long)]
        switch_name: Option<String>,
        #[arg(long)]
        memory_mb: Option<u32>,
        #[arg(long)]
        cpus: Option<u32>,
        #[arg(long)]
        disk_gb: Option<u32>,
        #[arg(long)]
        vm_root: Option<String>,
        #[arg(long)]
        portproxy: Option<bool>,
        #[arg(long)]
        windows_host: Option<String>,
        #[arg(long)]
        smb_user: Option<String>,
        #[arg(long)]
        smb_password: Option<String>,
    },

    /// Ensure SSH key exists
    EnsureKey,

    /// Download Ubuntu cloud image
    DownloadImage {
        #[arg(
            long,
            default_value = "https://cloud-images.ubuntu.com/jammy/current/jammy-server-cloudimg-amd64.img"
        )]
        url: String,
        #[arg(long, default_value = "")]
        output: String,
        #[arg(long, default_value = "")]
        sha256: String,
        #[arg(long)]
        force: bool,
    },

    /// Initialize Hyper-V runtime (download, create VM, bootstrap)
    Init {
        #[arg(long, default_value = "")]
        image_vhdx: String,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        sha256: Option<String>,
        #[arg(long, default_value_t = 300)]
        timeout: u32,
    },

    /// Create VM from ISO
    CreateVm {
        #[arg(long)]
        iso: String,
    },

    /// Create VM from cloud image
    CreateCloudVm {
        #[arg(long)]
        image_vhdx: String,
        #[arg(long)]
        no_start: bool,
        #[arg(long)]
        discover_ip: bool,
        #[arg(long, default_value_t = 180)]
        timeout: u32,
    },

    /// Start the VM
    StartVm,

    /// Stop the VM
    StopVm,

    /// Show VM IP address
    Ip,

    /// Discover and save VM IP
    DiscoverIp {
        #[arg(long, default_value_t = 180)]
        timeout: u32,
    },

    /// Bootstrap container runtime in VM
    Bootstrap,

    /// Show runtime health
    Health,

    /// Repair runtime
    Repair {
        #[arg(long, default_value_t = 180)]
        timeout: u32,
    },

    /// List containers
    Ps,

    /// Execute command in container
    Exec {
        container: String,
        #[arg(trailing_var_arg = true)]
        command: Vec<String>,
    },

    /// Manage VM snapshots
    Snapshot {
        #[command(subcommand)]
        snapshot_cmd: SnapshotCommands,
    },

    /// Manage container images
    Image {
        #[command(subcommand)]
        image_cmd: ImageCommands,
    },

    /// Manage container volumes
    Volume {
        #[command(subcommand)]
        volume_cmd: VolumeCommands,
    },

    /// Manage container networks
    Network {
        #[command(subcommand)]
        network_cmd: NetworkCommands,
    },

    /// Manage SMB shares
    Share {
        #[command(subcommand)]
        share_cmd: ShareCommands,
    },

    /// Configure registry mirrors
    Mirror {
        #[command(subcommand)]
        mirror_cmd: MirrorCommands,
    },
}

#[derive(Subcommand)]
enum SnapshotCommands {
    Create {
        #[arg(long)]
        name: Option<String>,
    },
    Ls,
    Restore {
        name: String,
    },
    Rm {
        name: String,
    },
    Export {
        name: String,
        #[arg(long)]
        output: String,
    },
}

#[derive(Subcommand)]
enum ImageCommands {
    Ls {
        #[arg(short = 'a', long)]
        all: bool,
    },
    Rm {
        #[arg(short = 'f', long)]
        force: bool,
        images: Vec<String>,
    },
    Prune {
        #[arg(short = 'a', long)]
        all: bool,
    },
    Login {
        registry: String,
        #[arg(short = 'u')]
        username: String,
        #[arg(short = 'p')]
        password: String,
    },
}

#[derive(Subcommand)]
enum VolumeCommands {
    Ls,
    Create {
        name: String,
        #[arg(long)]
        project: Option<String>,
    },
    Rm {
        #[arg(short = 'f', long)]
        force: bool,
        volumes: Vec<String>,
    },
    Prune,
    Inspect {
        volumes: Vec<String>,
    },
}

#[derive(Subcommand)]
enum NetworkCommands {
    Ls,
    Create {
        name: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long, default_value = "bridge")]
        driver: String,
    },
    Rm {
        networks: Vec<String>,
    },
    Inspect {
        networks: Vec<String>,
    },
}

#[derive(Subcommand)]
enum ShareCommands {
    Add {
        #[arg(long)]
        path: String,
        #[arg(long)]
        name: Option<String>,
    },
    Mount {
        name: String,
    },
}

#[derive(Subcommand)]
enum MirrorCommands {
    Ls,
    Set {
        registry: String,
        endpoints: Vec<String>,
    },
    Rm {
        registry: String,
    },
    Apply,
}

#[derive(Subcommand)]
enum DaemonCommands {
    /// Start the Docker-compatible API server
    Serve {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 23750)]
        port: u16,
        #[arg(long)]
        allow_remote: bool,
    },
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

fn load_stack(config_path: &str) -> Result<pystack_types::StackConfig> {
    load_stack_with_backend(config_path, "native")
}

fn load_stack_with_backend(
    config_path: &str,
    backend_override: &str,
) -> Result<pystack_types::StackConfig> {
    let path = PathBuf::from(config_path);
    if !path.exists() {
        anyhow::bail!("Configuration file not found: {}", config_path);
    }
    if is_compose_path(&path) {
        let project = pystack_compose::load_compose_file(&path, None, &[], None, None)?;
        let backend = if backend_override.is_empty() {
            "native"
        } else {
            backend_override
        };
        return compose_project_to_stack(project, &path, backend);
    }
    let text = std::fs::read_to_string(&path)?;
    let raw: serde_json::Value = serde_json::from_str(&text)?;
    // Parse stack.json into StackConfig
    let project = raw
        .get("project")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();
    let root = path
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf()
        .canonicalize()
        .unwrap_or_else(|_| path.parent().unwrap_or(Path::new(".")).to_path_buf());
    let state_dir = root.join(pystack_types::DEFAULT_STATE_DIR);
    let log_dir = root.join(pystack_types::DEFAULT_LOG_DIR);

    let defaults = raw
        .get("defaults")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    let raw_services = raw
        .get("services")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let mut services = HashMap::new();
    for (name, svc_raw) in raw_services {
        let svc = normalize_service(&name, &svc_raw, &defaults, &root, &path)?;
        services.insert(name, svc);
    }

    Ok(pystack_types::StackConfig {
        project,
        root: root.clone(),
        config_path: path,
        state_dir,
        log_dir,
        services,
        volumes: HashMap::new(),
        secrets: HashMap::new(),
        configs: HashMap::new(),
        source_format: "stackdeck".to_string(),
    })
}

fn is_compose_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("yaml" | "yml")
    )
}

fn effective_backend<'a>(override_backend: &'a str, service_backend: &'a str) -> &'a str {
    if override_backend.is_empty() {
        service_backend
    } else {
        override_backend
    }
}

fn validate_stack_for_run(
    stack: &pystack_types::StackConfig,
    backend_override: &str,
) -> Result<()> {
    for (name, svc) in &stack.services {
        if matches!(
            effective_backend(backend_override, &svc.backend),
            "native" | ""
        ) && command_args(&svc.command).is_empty()
        {
            anyhow::bail!("{name}: native service command must not be empty");
        }
    }
    Ok(())
}

fn command_args(command: &serde_json::Value) -> Vec<String> {
    match command {
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .filter(|s| !s.trim().is_empty())
            .collect(),
        serde_json::Value::String(s) => s
            .split_whitespace()
            .map(|s| s.to_string())
            .filter(|s| !s.trim().is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn compose_project_to_stack(
    project: pystack_types::ComposeProject,
    config_path: &Path,
    backend: &str,
) -> Result<pystack_types::StackConfig> {
    let root = project.working_dir.clone();
    let state_dir = root.join(pystack_types::DEFAULT_STATE_DIR);
    let log_dir = root.join(pystack_types::DEFAULT_LOG_DIR);
    let mut services = HashMap::new();
    for (name, service) in &project.services {
        services.insert(
            name.clone(),
            compose_service_to_stack_service(&project, service, backend)?,
        );
    }
    let volumes = project
        .volumes
        .iter()
        .map(|(key, resource)| {
            (
                key.clone(),
                serde_json::to_value(resource).unwrap_or_default(),
            )
        })
        .collect();
    let secrets = project
        .secrets
        .iter()
        .map(|(key, resource)| {
            (
                key.clone(),
                serde_json::to_value(resource).unwrap_or_default(),
            )
        })
        .collect();
    let configs = project
        .configs
        .iter()
        .map(|(key, resource)| {
            (
                key.clone(),
                serde_json::to_value(resource).unwrap_or_default(),
            )
        })
        .collect();
    Ok(pystack_types::StackConfig {
        project: project.name,
        root,
        config_path: config_path.to_path_buf(),
        state_dir,
        log_dir,
        services,
        volumes,
        secrets,
        configs,
        source_format: "compose".to_string(),
    })
}

fn compose_service_to_stack_service(
    project: &pystack_types::ComposeProject,
    svc: &pystack_types::ComposeService,
    backend: &str,
) -> Result<pystack_types::ServiceConfig> {
    let command = compose_command(svc);
    if backend == "native" && command == serde_json::json!([]) {
        anyhow::bail!(
            "compose service '{}' has no native command; use --backend hyperv for image services",
            svc.name
        );
    }
    Ok(pystack_types::ServiceConfig {
        name: svc.name.clone(),
        cwd: project.working_dir.clone(),
        command,
        env: compose_environment(project, svc)?,
        depends_on: svc
            .depends_on
            .iter()
            .map(|dep| dep.service.clone())
            .collect(),
        depends_on_conditions: svc
            .depends_on
            .iter()
            .map(|dep| {
                serde_json::json!({
                    "service": dep.service,
                    "condition": dep.condition,
                    "required": dep.required,
                    "restart": dep.restart,
                })
            })
            .collect(),
        restart: svc.restart.name.clone(),
        healthcheck: compose_healthcheck(svc.healthcheck.as_ref()),
        ports: svc.ports.iter().filter_map(compose_port_string).collect(),
        volumes: svc
            .volumes
            .iter()
            .filter_map(compose_volume_string)
            .collect(),
        networks: svc
            .networks
            .iter()
            .map(|network| network.name.clone())
            .collect(),
        image: svc.image.clone().unwrap_or_default(),
        build: svc
            .build
            .as_ref()
            .map(|build| serde_json::to_value(build).unwrap_or_default()),
        backend: backend.to_string(),
        replicas: svc.replicas,
        ..Default::default()
    })
}

fn compose_command(svc: &pystack_types::ComposeService) -> serde_json::Value {
    match (&svc.entrypoint, &svc.command) {
        (Some(serde_json::Value::Array(entry)), Some(serde_json::Value::Array(cmd))) => {
            let mut merged = entry.clone();
            merged.extend(cmd.clone());
            serde_json::Value::Array(merged)
        }
        (Some(serde_json::Value::String(entry)), Some(serde_json::Value::Array(cmd))) => {
            let mut merged = vec![serde_json::Value::String(entry.clone())];
            merged.extend(cmd.clone());
            serde_json::Value::Array(merged)
        }
        (Some(entry), None) => entry.clone(),
        (None, Some(cmd)) => cmd.clone(),
        (Some(entry), Some(serde_json::Value::String(cmd))) => match entry {
            serde_json::Value::Array(entry) => {
                let mut merged = entry.clone();
                merged.push(serde_json::Value::String(cmd.clone()));
                serde_json::Value::Array(merged)
            }
            serde_json::Value::String(entry) => serde_json::Value::String(format!("{entry} {cmd}")),
            _ => cmd.clone().into(),
        },
        _ => serde_json::json!([]),
    }
}

fn compose_environment(
    project: &pystack_types::ComposeProject,
    svc: &pystack_types::ComposeService,
) -> Result<HashMap<String, String>> {
    let mut env = HashMap::new();
    for file in &svc.env_file {
        let path = PathBuf::from(file);
        let path = if path.is_absolute() {
            path
        } else {
            project.working_dir.join(path)
        };
        env.extend(pystack_compose::parse_env_file(&path)?);
    }
    for (key, value) in &svc.environment {
        if let Some(value) = value {
            env.insert(key.clone(), value.clone());
        } else {
            let env_path = project.working_dir.join(".env");
            let proj_env = pystack_compose::parse_env_file(&env_path).unwrap_or_default();
            if let Some(proj_val) = proj_env.get(key) {
                env.insert(key.clone(), proj_val.clone());
            } else if let Ok(sys_val) = std::env::var(key) {
                env.insert(key.clone(), sys_val);
            }
        }
    }
    Ok(env)
}

fn compose_healthcheck(
    healthcheck: Option<&pystack_types::ComposeHealthcheck>,
) -> pystack_types::HealthCheck {
    let Some(healthcheck) = healthcheck else {
        return Default::default();
    };
    if healthcheck.disable {
        return Default::default();
    }
    pystack_types::HealthCheck {
        r#type: "command".to_string(),
        test: healthcheck.test.clone(),
        timeout_seconds: parse_duration_seconds(healthcheck.timeout.as_deref()).unwrap_or(20),
        interval_seconds: parse_duration_seconds(healthcheck.interval.as_deref()).unwrap_or(2),
        retries: healthcheck.retries.unwrap_or(1),
        start_period_seconds: parse_duration_seconds(healthcheck.start_period.as_deref())
            .unwrap_or(0),
        ..Default::default()
    }
}

fn parse_duration_seconds(value: Option<&str>) -> Option<u32> {
    let raw = value?.trim().replace(' ', "");
    if raw.is_empty() {
        return None;
    }

    if let Ok(n) = raw.parse::<f64>() {
        return Some(n.ceil().max(0.0) as u32);
    }

    let mut total_seconds = 0.0;
    let mut num_start = 0;
    let mut i = 0;
    let bytes = raw.as_bytes();

    while i < bytes.len() {
        while i < bytes.len()
            && (bytes[i].is_ascii_digit()
                || bytes[i] == b'.'
                || bytes[i] == b'-'
                || bytes[i] == b'+')
        {
            i += 1;
        }
        if num_start == i {
            return None;
        }
        let num_str = &raw[num_start..i];
        let num = num_str.parse::<f64>().ok()?;

        let unit_start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        let unit_str = &raw[unit_start..i];

        let scale = match unit_str {
            "ms" => 0.001,
            "s" => 1.0,
            "m" => 60.0,
            "h" => 3600.0,
            "" => 1.0,
            _ => return None,
        };
        total_seconds += num * scale;
        num_start = i;
    }

    Some(total_seconds.ceil().max(0.0) as u32)
}

fn compose_port_string(port: &pystack_types::ComposePort) -> Option<String> {
    let mut s = String::new();
    if let Some(host_ip) = &port.host_ip {
        if !host_ip.is_empty() {
            s.push_str(host_ip);
            s.push(':');
        }
    }

    if let Some(published) = port.published {
        s.push_str(&published.to_string());
        s.push(':');
    } else if s.ends_with(':') {
        s.push(':');
    }

    s.push_str(&port.target.to_string());

    if port.protocol != "tcp" && !port.protocol.is_empty() {
        s.push('/');
        s.push_str(&port.protocol);
    }

    Some(s)
}

fn compose_volume_string(volume: &pystack_types::ComposeVolumeMount) -> Option<String> {
    let source = volume.source.as_ref()?;
    let mut spec = format!("{}:{}", source, volume.target);
    if volume.read_only {
        spec.push_str(":ro");
    }
    Some(spec)
}

fn stack_service_to_hyperv(
    stack: &pystack_types::StackConfig,
    svc: &pystack_types::ServiceConfig,
) -> pystack_hyperv::HyperVService {
    pystack_hyperv::HyperVService {
        project: stack.project.clone(),
        name: svc.name.clone(),
        root: stack.root.clone(),
        image: svc.image.clone(),
        build: svc.build.clone(),
        command: if svc.command == serde_json::json!([]) {
            None
        } else {
            Some(svc.command.clone())
        },
        env: svc.env.clone(),
        ports: svc.ports.clone(),
        volumes: svc.volumes.clone(),
        networks: svc.networks.clone(),
        restart: svc.restart.clone(),
        secrets: svc.secrets_meta.clone(),
        configs: svc.configs_meta.clone(),
        secret_resources: stack.secrets.clone(),
        config_resources: stack.configs.clone(),
        healthcheck: Some(svc.healthcheck.clone()),
    }
}

fn load_compose_stack(
    file: &str,
    backend: &str,
    services: &[String],
) -> Result<pystack_types::StackConfig> {
    let path = PathBuf::from(file);
    let project = pystack_compose::load_compose_file(&path, None, services, None, None)?;
    compose_project_to_stack(project, &path, backend)
}

fn normalize_stack_depends_on(
    raw: Option<&serde_json::Value>,
) -> (Vec<String>, Vec<serde_json::Value>) {
    match raw {
        Some(serde_json::Value::Array(items)) => {
            let names: Vec<String> = items
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            let conds = names.iter().map(|name| serde_json::json!({"service": name, "condition": "service_started", "required": true})).collect();
            (names, conds)
        }
        Some(serde_json::Value::Object(map)) => {
            let mut names = Vec::new();
            let mut conds = Vec::new();
            for (service, cfg) in map {
                names.push(service.clone());
                let condition = cfg
                    .get("condition")
                    .and_then(|v| v.as_str())
                    .unwrap_or("service_started");
                let required = cfg
                    .get("required")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                conds.push(serde_json::json!({"service": service, "condition": condition, "required": required}));
            }
            (names, conds)
        }
        _ => (Vec::new(), Vec::new()),
    }
}

fn normalize_service(
    name: &str,
    raw: &serde_json::Value,
    defaults: &serde_json::Value,
    root: &Path,
    _config_path: &Path,
) -> Result<pystack_types::ServiceConfig> {
    let obj = raw.as_object().cloned().unwrap_or_default();
    let def_obj = defaults.as_object().cloned().unwrap_or_default();

    let cwd_raw = obj
        .get("cwd")
        .or_else(|| obj.get("workdir"))
        .or_else(|| def_obj.get("cwd"))
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let cwd = if PathBuf::from(cwd_raw).is_absolute() {
        PathBuf::from(cwd_raw)
    } else {
        root.join(cwd_raw)
    };

    let command = obj.get("command").cloned().unwrap_or(serde_json::json!([]));

    let env_raw = obj.get("env").or_else(|| def_obj.get("env"));
    let env = match env_raw {
        Some(serde_json::Value::Object(m)) => m
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
            .collect(),
        _ => HashMap::new(),
    };

    let (depends_on, depends_on_conditions) = normalize_stack_depends_on(obj.get("depends_on"));

    let restart = obj
        .get("restart")
        .or_else(|| def_obj.get("restart"))
        .and_then(|v| v.as_str())
        .unwrap_or("no")
        .to_string();

    let healthcheck_raw = obj
        .get("healthcheck")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    let hc_obj = healthcheck_raw.as_object().cloned().unwrap_or_default();
    let healthcheck = pystack_types::HealthCheck {
        r#type: hc_obj
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("process")
            .to_string(),
        host: hc_obj
            .get("host")
            .and_then(|v| v.as_str())
            .unwrap_or("127.0.0.1")
            .to_string(),
        port: hc_obj.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u16,
        timeout_seconds: hc_obj
            .get("timeout_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(20) as u32,
        interval_seconds: hc_obj
            .get("interval_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(2) as u32,
        retries: hc_obj.get("retries").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
        ..Default::default()
    };

    let resources_raw = obj
        .get("resources")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    let res_obj = resources_raw.as_object().cloned().unwrap_or_default();
    let resources = pystack_types::ResourceLimits {
        memory_mb: res_obj
            .get("memory_mb")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        process_count: res_obj
            .get("process_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
    };

    let backend = obj
        .get("backend")
        .or_else(|| def_obj.get("backend"))
        .and_then(|v| v.as_str())
        .unwrap_or("native")
        .to_string();

    let image = obj
        .get("image")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let ports = obj
        .get("ports")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let volumes = obj
        .get("volumes")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let networks = obj
        .get("networks")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    Ok(pystack_types::ServiceConfig {
        name: name.to_string(),
        cwd,
        command,
        env,
        depends_on,
        depends_on_conditions,
        restart,
        healthcheck,
        resources,
        backend,
        image,
        ports,
        volumes,
        networks,
        ..Default::default()
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    if let Err(err) = preflight_check() {
        eprintln!("Preflight warning: {}", err);
    }

    let cli = Cli::parse();

    match cli.command {
        // ---- Core commands ----
        Commands::Init { force } => cmd_init(&cli.config, force)?,
        Commands::Validate => cmd_validate(&cli.config, &cli.backend)?,
        Commands::Doctor => cmd_doctor()?,
        Commands::Up {
            services,
            force,
            supervise,
            interval,
        } => cmd_up(
            &cli.config,
            &cli.backend,
            &services,
            force,
            supervise,
            interval,
        )?,
        Commands::Down { services, volumes } => {
            cmd_down(&cli.config, &cli.backend, &services, volumes)?
        }
        Commands::Restart { services } => cmd_restart(&cli.config, &cli.backend, &services)?,
        Commands::Status { json } => cmd_status(&cli.config, &cli.backend, json)?,
        Commands::Logs { service, tail } => cmd_logs(&cli.config, &cli.backend, &service, tail)?,
        Commands::Ps { all } => cmd_docker_ps(all)?,
        Commands::Images { all } => cmd_docker_images(all)?,
        Commands::Pull { image } => cmd_docker_pull(&image)?,
        Commands::Rmi { force, images } => cmd_docker_rmi(force, &images)?,
        Commands::Build {
            tag,
            file,
            build_arg,
            context,
        } => cmd_docker_build(&tag, &file, &build_arg, &context)?,
        Commands::Login {
            registry,
            username,
            password,
        } => cmd_docker_login(&registry, &username, &password)?,
        Commands::Inspect { containers } => cmd_docker_inspect(&containers)?,
        Commands::Exec { container, command } => cmd_docker_exec(&container, &command)?,
        Commands::Start { containers } => {
            cmd_docker_container_command("start", false, &containers)?
        }
        Commands::Stop { containers } => cmd_docker_container_command("stop", false, &containers)?,
        Commands::Rm { force, containers } => cmd_docker_rm(force, &containers)?,
        Commands::Volume { volume_cmd } => cmd_docker_volume(volume_cmd)?,
        Commands::Network { network_cmd } => cmd_docker_network(network_cmd)?,
        Commands::Diagnostics { output, tail, text } => {
            cmd_diagnostics(&cli.config, &output, tail, text)?
        }
        Commands::Watch { interval } => cmd_watch(&cli.config, interval)?,
        Commands::Register {
            name,
            path,
            backend,
            allow_invalid,
        } => cmd_register(&name, &path, backend.as_deref(), &cli.config, allow_invalid)?,
        Commands::Unregister { name } => cmd_unregister(&name)?,
        Commands::Gui { host, port } => cmd_gui(&host, port).await?,

        // ---- Compose commands ----
        Commands::Compose { compose_cmd } => match compose_cmd {
            ComposeCommands::Up {
                file,
                backend,
                detach,
                build,
                interval,
                services,
            } => cmd_compose_up(&file, &backend, detach, build, interval, &services)?,
            ComposeCommands::Build {
                file,
                backend,
                services,
            } => cmd_compose_build(&file, &backend, &services)?,
            ComposeCommands::Down {
                file,
                backend,
                volumes,
                services,
            } => cmd_compose_down(&file, &backend, volumes, &services)?,
            ComposeCommands::Status {
                file,
                backend,
                json,
            } => cmd_compose_status(&file, &backend, json)?,
            ComposeCommands::Logs {
                file,
                backend,
                service,
                tail,
            } => cmd_compose_logs(&file, &backend, &service, tail)?,
        },

        // ---- Hyper-V commands ----
        Commands::Hyperv { hyperv_cmd } => run_hyperv_command(hyperv_cmd)?,

        // ---- Daemon commands ----
        Commands::Daemon { daemon_cmd } => match daemon_cmd {
            DaemonCommands::Serve {
                host,
                port,
                allow_remote,
            } => cmd_daemon_serve(&host, port, allow_remote).await?,
        },
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Core command implementations
// ---------------------------------------------------------------------------

fn cmd_init(config_path: &str, force: bool) -> Result<()> {
    let path = PathBuf::from(config_path);
    if path.exists() && !force {
        anyhow::bail!("{} already exists. Use --force to overwrite.", config_path);
    }
    let template = serde_json::json!({
        "project": path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()).unwrap_or("demo-stack"),
        "defaults": {
            "env_file": ".env",
            "restart": "no",
            "stop_grace_seconds": 10,
        },
        "services": {
            "web": {
                "cwd": ".",
                "command": ["python", "-m", "http.server", "8000"],
                "env": { "APP_ENV": "local" },
                "restart": "on-failure",
                "healthcheck": { "type": "tcp", "port": 8000, "timeout_seconds": 10 },
            }
        }
    });
    let json = serde_json::to_string_pretty(&template)?;
    std::fs::write(&path, json)?;
    println!("Created {}", config_path);
    Ok(())
}

fn cmd_validate(config_path: &str, backend_override: &str) -> Result<()> {
    let mut stack = load_stack_with_backend(config_path, backend_override)?;
    if !backend_override.is_empty() {
        for svc in stack.services.values_mut() {
            svc.backend = backend_override.to_string();
        }
    }
    validate_stack_for_run(&stack, backend_override)?;
    println!(
        "Valid: {} ({} services)",
        stack.project,
        stack.services.len()
    );
    for name in stack.services.keys() {
        println!("  - {}", name);
    }
    Ok(())
}

fn cmd_doctor() -> Result<()> {
    let cfg = pystack_types::HyperVConfig::default();
    let mgr = pystack_hyperv::HyperVManager::new(cfg);
    let checks = mgr.preflight();
    let max_len = checks.keys().map(|k| k.len()).max().unwrap_or(10);
    for (key, value) in &checks {
        println!("{:>width$}  {}", key, value, width = max_len);
    }
    Ok(())
}

fn cmd_up(
    config_path: &str,
    backend_override: &str,
    services: &[String],
    force: bool,
    supervise: bool,
    interval: u32,
) -> Result<()> {
    let stack = load_stack_with_backend(config_path, backend_override)?;
    validate_stack_for_run(&stack, backend_override)?;
    let targets = if services.is_empty() {
        stack.services.keys().cloned().collect::<Vec<_>>()
    } else {
        services.to_vec()
    };
    run_stack_up(
        stack,
        backend_override,
        &targets,
        force,
        false,
        supervise,
        interval,
    )
}

fn cmd_down(
    config_path: &str,
    backend_override: &str,
    services: &[String],
    volumes: bool,
) -> Result<()> {
    let stack = load_stack_with_backend(config_path, backend_override)?;
    let targets = if services.is_empty() {
        stack.services.keys().cloned().collect::<Vec<_>>()
    } else {
        services.to_vec()
    };
    let mut native_mgr = pystack_process::ProcessManager::new(stack.clone())?;
    let hyperv_mgr =
        pystack_hyperv::HyperVManager::new(pystack_hyperv::HyperVManager::load_config()?);
    let mut failed = Vec::new();
    for name in &targets {
        let Some(svc) = stack.services.get(name) else {
            failed.push(format!("{}: service not found", name));
            continue;
        };
        match effective_backend(backend_override, &svc.backend) {
            "hyperv" => match hyperv_mgr.stop_service(&stack_service_to_hyperv(&stack, svc)) {
                Ok(output) => println!("{}: stopped {}", name, output),
                Err(e) => failed.push(format!("{}: {}", name, e)),
            },
            "native" | "" => match native_mgr.stop(name) {
                Ok(msg) => println!("{}", msg),
                Err(e) => failed.push(format!("{}: {}", name, e)),
            },
            other => failed.push(format!("{}: unsupported backend '{}'", name, other)),
        }
    }
    if volumes {
        let mut volume_names = Vec::new();
        for (vname, vconfig) in &stack.volumes {
            let is_external = vconfig
                .get("external")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !is_external {
                let name = format!("{}_{}", pystack_hyperv::project_slug(&stack.project), vname);
                volume_names.push(name);
            }
        }
        if !volume_names.is_empty() {
            let refs: Vec<&str> = volume_names.iter().map(|s| s.as_str()).collect();
            match hyperv_mgr.volume_remove(&refs, true) {
                Ok(output) => println!("Removed volumes: {}", output),
                Err(e) => failed.push(format!("Failed to remove volumes: {}", e)),
            }
        }
        if let Err(e) = native_mgr.teardown_volumes() {
            failed.push(format!("Failed to remove native volumes: {}", e));
        }
    }
    if !failed.is_empty() {
        for msg in &failed {
            eprintln!("{}", msg);
        }
        anyhow::bail!("{} service(s) failed to stop", failed.len());
    }
    Ok(())
}

fn cmd_restart(config_path: &str, backend_override: &str, services: &[String]) -> Result<()> {
    let stack = load_stack_with_backend(config_path, backend_override)?;
    let targets = if services.is_empty() {
        stack.services.keys().cloned().collect::<Vec<_>>()
    } else {
        services.to_vec()
    };
    run_stack_down(stack.clone(), backend_override, &targets, false)?;
    run_stack_up(stack, backend_override, &targets, false, false, false, 10)
}

fn cmd_status(config_path: &str, backend_override: &str, json_output: bool) -> Result<()> {
    let stack = load_stack_with_backend(config_path, backend_override)?;
    if backend_override == "hyperv"
        || stack
            .services
            .values()
            .any(|svc| effective_backend(backend_override, &svc.backend) == "hyperv")
    {
        let hyperv_mgr =
            pystack_hyperv::HyperVManager::new(pystack_hyperv::HyperVManager::load_config()?);
        let names: Vec<String> = stack
            .services
            .values()
            .filter(|svc| effective_backend(backend_override, &svc.backend) == "hyperv")
            .map(|svc| pystack_hyperv::container_name(&stack.project, &svc.name))
            .collect();
        let refs: Vec<&str> = names.iter().map(|name| name.as_str()).collect();
        let inspected = hyperv_mgr.inspect_containers(&refs)?;
        if json_output {
            println!("{}", serde_json::to_string_pretty(&inspected)?);
        } else {
            println!("{:<32} {}", "CONTAINER", "STATUS");
            for name in &names {
                let status = inspected
                    .get(name)
                    .and_then(|value| value.get("status"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown");
                println!("{:<32} {}", name, status);
            }
        }
        return Ok(());
    }
    let mgr = pystack_process::ProcessManager::new(stack)?;
    let all = mgr.db().all().map_err(|e| anyhow::anyhow!("{}", e))?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&all)?);
    } else {
        println!(
            "{:<20} {:<10} {:<8} {}",
            "SERVICE", "STATUS", "PID", "RESTARTS"
        );
        for svc in &all {
            let pid = svc.pid.map(|p| p.to_string()).unwrap_or("-".to_string());
            println!(
                "{:<20} {:<10} {:<8} {}",
                svc.service, svc.status, pid, svc.restart_count
            );
        }
    }
    Ok(())
}

fn cmd_logs(config_path: &str, backend_override: &str, service: &str, tail: u32) -> Result<()> {
    let stack = load_stack_with_backend(config_path, backend_override)?;
    run_stack_logs(stack, backend_override, service, tail)
}

fn run_stack_up(
    stack: pystack_types::StackConfig,
    backend_override: &str,
    targets: &[String],
    force: bool,
    build: bool,
    supervise: bool,
    interval: u32,
) -> Result<()> {
    validate_stack_for_run(&stack, backend_override)?;
    let mut native_mgr = pystack_process::ProcessManager::new(stack.clone())?;
    let hyperv_mgr =
        pystack_hyperv::HyperVManager::new(pystack_hyperv::HyperVManager::load_config()?);

    let mut ordered_targets = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut visiting = std::collections::HashSet::new();

    fn visit(
        name: &str,
        stack: &pystack_types::StackConfig,
        visited: &mut std::collections::HashSet<String>,
        visiting: &mut std::collections::HashSet<String>,
        sorted: &mut Vec<String>,
    ) -> Result<()> {
        if visited.contains(name) {
            return Ok(());
        }
        if visiting.contains(name) {
            anyhow::bail!("dependency cycle detected at service '{}'", name);
        }
        visiting.insert(name.to_string());
        if let Some(svc) = stack.services.get(name) {
            for dep in dependency_specs(svc) {
                if dep.required && !stack.services.contains_key(&dep.service) {
                    anyhow::bail!(
                        "service '{}' depends on missing service '{}'",
                        name,
                        dep.service
                    );
                }
                if stack.services.contains_key(&dep.service) {
                    visit(&dep.service, stack, visited, visiting, sorted)?;
                }
            }
        }
        visiting.remove(name);
        visited.insert(name.to_string());
        sorted.push(name.to_string());
        Ok(())
    }

    for name in targets {
        visit(
            name,
            &stack,
            &mut visited,
            &mut visiting,
            &mut ordered_targets,
        )?;
    }

    let mut failed = Vec::new();
    for name in &ordered_targets {
        let Some(svc) = stack.services.get(name) else {
            failed.push(format!("{}: service not found", name));
            continue;
        };

        let mut deps_ready = true;
        for dep in dependency_specs(svc) {
            if !stack.services.contains_key(&dep.service) {
                if dep.required {
                    failed.push(format!("{}: missing dependency {}", name, dep.service));
                    deps_ready = false;
                    break;
                }
                continue;
            }
            println!(
                "{}: waiting for dependency {} ({})...",
                name, dep.service, dep.condition
            );
            if !wait_for_dependency(
                &stack,
                backend_override,
                &hyperv_mgr,
                &mut native_mgr,
                &dep,
                120,
            )? {
                if dep.required {
                    failed.push(format!(
                        "{}: dependency {} did not satisfy '{}' within timeout",
                        name, dep.service, dep.condition
                    ));
                    deps_ready = false;
                    break;
                }
            }
        }
        if !deps_ready {
            continue;
        }

        match effective_backend(backend_override, &svc.backend) {
            "hyperv" => {
                let hyperv_svc = stack_service_to_hyperv(&stack, svc);
                if force {
                    let _ = hyperv_mgr.stop_service(&hyperv_svc);
                }
                match hyperv_mgr.start_service(&hyperv_svc, build) {
                    Ok(output) => println!("{}: started {}", name, output),
                    Err(e) => failed.push(format!("{}: {}", name, e)),
                }
            }
            "native" | "" => match native_mgr.start(name, force) {
                Ok(result) if result.ok => println!("{}", result.message),
                Ok(result) => failed.push(result.message),
                Err(e) => failed.push(format!("{}: {}", name, e)),
            },
            other => failed.push(format!("{}: unsupported backend '{}'", name, other)),
        }
    }
    if !failed.is_empty() {
        for msg in &failed {
            eprintln!("{}", msg);
        }
        anyhow::bail!("{} service(s) failed to start", failed.len());
    }

    if supervise {
        println!(
            "Supervising services (interval={}s, Ctrl+C to stop)...",
            interval
        );
        loop {
            std::thread::sleep(std::time::Duration::from_secs(interval.max(1) as u64));
            for name in &ordered_targets {
                let Some(svc) = stack.services.get(name) else {
                    continue;
                };
                match effective_backend(backend_override, &svc.backend) {
                    "hyperv" => {
                        let cname = pystack_hyperv::container_name(&stack.project, name);
                        let status = hyperv_mgr
                            .container_health_status(&cname)
                            .unwrap_or_else(|_| "missing".to_string());
                        let bad =
                            if svc.healthcheck.test.is_some() && svc.healthcheck.r#type != "none" {
                                status != "healthy"
                            } else {
                                status != "running" && status != "healthy"
                            };
                        if bad && svc.restart != "no" {
                            println!("{}: {} under Hyper-V, restarting...", name, status);
                            let hyperv_svc = stack_service_to_hyperv(&stack, svc);
                            let _ = hyperv_mgr.stop_service(&hyperv_svc);
                            if let Err(err) = hyperv_mgr.start_service(&hyperv_svc, build) {
                                eprintln!("{}: restart failed: {}", name, err);
                            }
                        }
                    }
                    "native" | "" => {
                        if !native_mgr.health_ok(name).unwrap_or(false) && svc.restart != "no" {
                            println!("{}: unhealthy, restarting...", name);
                            let _ = native_mgr.start(name, true);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct DependencySpec {
    service: String,
    condition: String,
    required: bool,
}

fn dependency_specs(svc: &pystack_types::ServiceConfig) -> Vec<DependencySpec> {
    let mut specs = Vec::new();
    for raw in &svc.depends_on_conditions {
        let service = raw
            .get("service")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if service.is_empty() {
            continue;
        }
        specs.push(DependencySpec {
            service: service.to_string(),
            condition: raw
                .get("condition")
                .and_then(|v| v.as_str())
                .unwrap_or("service_started")
                .to_string(),
            required: raw
                .get("required")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        });
    }
    if specs.is_empty() {
        specs.extend(svc.depends_on.iter().map(|service| DependencySpec {
            service: service.clone(),
            condition: "service_started".to_string(),
            required: true,
        }));
    }
    specs
}

fn wait_for_dependency(
    stack: &pystack_types::StackConfig,
    backend_override: &str,
    hyperv_mgr: &pystack_hyperv::HyperVManager,
    native_mgr: &mut pystack_process::ProcessManager,
    dep: &DependencySpec,
    timeout_secs: u64,
) -> Result<bool> {
    let started = std::time::Instant::now();
    loop {
        let Some(dep_svc) = stack.services.get(&dep.service) else {
            return Ok(!dep.required);
        };
        let ready = if effective_backend(backend_override, &dep_svc.backend) == "hyperv" {
            let cname = pystack_hyperv::container_name(&stack.project, &dep.service);
            let inspected = hyperv_mgr.inspect_containers(&[&cname]).unwrap_or_default();
            let cinfo = inspected.get(&cname).cloned().unwrap_or(
                serde_json::json!({ "exists": false, "status": "missing", "exit_code": 1 }),
            );
            let status = cinfo
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("missing");
            let exit_code = cinfo.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(1);
            match dep.condition.as_str() {
                "service_healthy" => {
                    if dep_svc.healthcheck.test.is_some() && dep_svc.healthcheck.r#type != "none" {
                        status == "healthy"
                    } else {
                        status == "running" || status == "healthy"
                    }
                }
                "service_completed_successfully" => {
                    (status == "exited" || status == "stopped") && exit_code == 0
                }
                _ => status == "running" || status == "healthy",
            }
        } else {
            match dep.condition.as_str() {
                "service_healthy" => native_mgr.health_ok(&dep.service).unwrap_or(false),
                "service_completed_successfully" => {
                    !native_mgr.health_ok(&dep.service).unwrap_or(false)
                }
                _ => native_mgr.health_ok(&dep.service).unwrap_or(false),
            }
        };
        if ready {
            return Ok(true);
        }
        if started.elapsed().as_secs() >= timeout_secs {
            return Ok(false);
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

fn run_stack_down(
    stack: pystack_types::StackConfig,
    backend_override: &str,
    targets: &[String],
    volumes: bool,
) -> Result<()> {
    let mut native_mgr = pystack_process::ProcessManager::new(stack.clone())?;
    let hyperv_mgr =
        pystack_hyperv::HyperVManager::new(pystack_hyperv::HyperVManager::load_config()?);
    let mut failed = Vec::new();
    for name in targets {
        let Some(svc) = stack.services.get(name) else {
            failed.push(format!("{}: service not found", name));
            continue;
        };
        match effective_backend(backend_override, &svc.backend) {
            "hyperv" => match hyperv_mgr.stop_service(&stack_service_to_hyperv(&stack, svc)) {
                Ok(output) => println!("{}: stopped {}", name, output),
                Err(e) => failed.push(format!("{}: {}", name, e)),
            },
            "native" | "" => match native_mgr.stop(name) {
                Ok(msg) => println!("{}", msg),
                Err(e) => failed.push(format!("{}: {}", name, e)),
            },
            other => failed.push(format!("{}: unsupported backend '{}'", name, other)),
        }
    }
    if volumes {
        let mut volume_names = Vec::new();
        for (vname, vconfig) in &stack.volumes {
            let is_external = vconfig
                .get("external")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !is_external {
                let name = format!("{}_{}", pystack_hyperv::project_slug(&stack.project), vname);
                volume_names.push(name);
            }
        }
        if !volume_names.is_empty() {
            let refs: Vec<&str> = volume_names.iter().map(|s| s.as_str()).collect();
            match hyperv_mgr.volume_remove(&refs, true) {
                Ok(output) => println!("Removed volumes: {}", output),
                Err(e) => failed.push(format!("Failed to remove volumes: {}", e)),
            }
        }
        if let Err(e) = native_mgr.teardown_volumes() {
            failed.push(format!("Failed to remove native volumes: {}", e));
        }
    }
    if !failed.is_empty() {
        for msg in &failed {
            eprintln!("{}", msg);
        }
        anyhow::bail!("{} service(s) failed to stop", failed.len());
    }
    Ok(())
}

fn run_stack_status(
    stack: pystack_types::StackConfig,
    backend_override: &str,
    json_output: bool,
) -> Result<()> {
    if backend_override == "hyperv"
        || stack
            .services
            .values()
            .any(|svc| effective_backend(backend_override, &svc.backend) == "hyperv")
    {
        let hyperv_mgr =
            pystack_hyperv::HyperVManager::new(pystack_hyperv::HyperVManager::load_config()?);
        let names: Vec<String> = stack
            .services
            .values()
            .filter(|svc| effective_backend(backend_override, &svc.backend) == "hyperv")
            .map(|svc| pystack_hyperv::container_name(&stack.project, &svc.name))
            .collect();
        let refs: Vec<&str> = names.iter().map(|name| name.as_str()).collect();
        let inspected = hyperv_mgr.inspect_containers(&refs)?;
        if json_output {
            println!("{}", serde_json::to_string_pretty(&inspected)?);
        } else {
            println!("{:<32} {}", "CONTAINER", "STATUS");
            for name in &names {
                let status = inspected
                    .get(name)
                    .and_then(|value| value.get("status"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown");
                println!("{:<32} {}", name, status);
            }
        }
        return Ok(());
    }

    let mgr = pystack_process::ProcessManager::new(stack)?;
    let all = mgr.db().all().map_err(|e| anyhow::anyhow!("{}", e))?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&all)?);
    } else {
        println!(
            "{:<20} {:<10} {:<8} {}",
            "SERVICE", "STATUS", "PID", "RESTARTS"
        );
        for svc in &all {
            let pid = svc.pid.map(|p| p.to_string()).unwrap_or("-".to_string());
            println!(
                "{:<20} {:<10} {:<8} {}",
                svc.service, svc.status, pid, svc.restart_count
            );
        }
    }
    Ok(())
}

fn run_stack_logs(
    stack: pystack_types::StackConfig,
    backend_override: &str,
    service: &str,
    tail: u32,
) -> Result<()> {
    let svc = stack
        .services
        .get(service)
        .ok_or_else(|| anyhow::anyhow!("Service '{}' not found", service))?;
    if effective_backend(backend_override, &svc.backend) == "hyperv" {
        let hyperv_mgr =
            pystack_hyperv::HyperVManager::new(pystack_hyperv::HyperVManager::load_config()?);
        let container = pystack_hyperv::container_name(&stack.project, service);
        println!("{}", hyperv_mgr.logs(&container, tail)?);
        return Ok(());
    }
    let log_dir = &stack.log_dir;
    let log_path = log_dir.join(format!("{}.out.log", service));
    if !log_path.exists() {
        anyhow::bail!("No logs found for service: {}", service);
    }
    let content = std::fs::read_to_string(&log_path)?;
    let lines: Vec<&str> = content.lines().collect();
    let start = if lines.len() > tail as usize {
        lines.len() - tail as usize
    } else {
        0
    };
    for line in &lines[start..] {
        println!("{}", line);
    }
    Ok(())
}

fn cmd_diagnostics(config_path: &str, output: &str, tail: u32, text: bool) -> Result<()> {
    let stack = load_stack(config_path)?;
    let now = chrono_like_timestamp();
    let out_root = if output.is_empty() {
        PathBuf::from(format!(
            "diagnostics-{}",
            now.replace(':', "-").replace('T', "_")
        ))
    } else {
        PathBuf::from(output)
    };

    let mut report = String::new();
    report.push_str("# StackDeck diagnostics\n\n");
    report.push_str(&format!("generated_at: {}\n", now));
    report.push_str(&format!("project: {}\n", stack.project));
    report.push_str(&format!("root: {}\n", stack.root.display()));
    report.push_str(&format!("config: {}\n", stack.config_path.display()));
    report.push_str(&format!("services: {}\n\n", stack.services.len()));

    report.push_str("## Services\n");
    for (name, svc) in &stack.services {
        report.push_str(&format!(
            "- {} backend={} image={} restart={}\n",
            name,
            svc.backend,
            redact_text(&svc.image),
            svc.restart
        ));
    }

    if text {
        let path = if output.is_empty() {
            PathBuf::from("diagnostics.txt")
        } else {
            out_root
        };
        std::fs::write(&path, redact_text(&report))?;
        println!("Diagnostics text generated: {}", path.display());
        return Ok(());
    }

    std::fs::create_dir_all(&out_root)?;
    std::fs::write(out_root.join("summary.md"), redact_text(&report))?;
    std::fs::write(
        out_root.join("stack.redacted.json"),
        redact_text(&serde_json::to_string_pretty(&stack)?),
    )?;

    let mut status_json = serde_json::json!({"native": [], "hyperv": {}});
    if let Ok(mgr) = pystack_process::ProcessManager::new(stack.clone()) {
        if let Ok(all) = mgr.db().all() {
            status_json["native"] =
                serde_json::to_value(all).unwrap_or_else(|_| serde_json::json!([]));
        }
    }
    if let Ok(cfg) = pystack_hyperv::HyperVManager::load_config() {
        let mgr = pystack_hyperv::HyperVManager::new(cfg);
        if let Ok(health) = mgr.runtime_health_check() {
            status_json["hyperv"] =
                serde_json::to_value(health).unwrap_or_else(|_| serde_json::json!({}));
        }
    }
    std::fs::write(
        out_root.join("status.redacted.json"),
        redact_text(&serde_json::to_string_pretty(&status_json)?),
    )?;

    let logs_dir = out_root.join("logs");
    std::fs::create_dir_all(&logs_dir)?;
    for (name, _svc) in &stack.services {
        for suffix in ["out", "err"] {
            let log_path = stack.log_dir.join(format!("{}.{}.log", name, suffix));
            if log_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&log_path) {
                    let tailed = tail_lines(&content, tail as usize);
                    std::fs::write(
                        logs_dir.join(format!("{}.{}.redacted.log", name, suffix)),
                        redact_text(&tailed),
                    )?;
                }
            }
        }
    }

    let manifest = serde_json::json!({
        "format": "stackdeck-diagnostics-bundle-v1",
        "generated_at": now,
        "redacted": true,
        "files": ["summary.md", "stack.redacted.json", "status.redacted.json", "logs/"]
    });
    std::fs::write(
        out_root.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    println!("Diagnostics bundle generated: {}", out_root.display());
    Ok(())
}

fn tail_lines(content: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

fn redact_text(input: &str) -> String {
    let mut out = input.to_string();
    for hint in pystack_types::SAFE_ENV_NAME_HINTS {
        let prefix = r#"["']?[^\s:="']*"#;
        let suffix = r#"[^\s:="']*["']?"#;
        let re1 = regex::Regex::new(&format!(
            r#"(?i)({}{}{}\s*[:=]\s*["'])([^"']*)(["'])"#,
            prefix,
            regex::escape(hint),
            suffix
        ))
        .unwrap();
        let re2 = regex::Regex::new(&format!(
            r#"(?i)({}{}{}\s*[:=]\s*)([^\s,"']+)"#,
            prefix,
            regex::escape(hint),
            suffix
        ))
        .unwrap();
        out = re1
            .replace_all(&out, format!("$1{}$3", pystack_types::REDACTION))
            .to_string();
        out = re2
            .replace_all(&out, format!("$1{}", pystack_types::REDACTION))
            .to_string();
    }
    out
}

fn chrono_like_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix-{}", secs)
}

fn cmd_watch(config_path: &str, interval: u32) -> Result<()> {
    let stack = load_stack(config_path)?;
    let mut native_mgr = pystack_process::ProcessManager::new(stack.clone())?;
    let hyperv_mgr =
        pystack_hyperv::HyperVManager::new(pystack_hyperv::HyperVManager::load_config()?);
    println!(
        "Watching services (interval={}s, Ctrl+C to stop)...",
        interval
    );
    loop {
        for name in stack.services.keys().cloned().collect::<Vec<_>>() {
            let Some(svc) = stack.services.get(&name) else {
                continue;
            };
            match effective_backend("", &svc.backend) {
                "hyperv" => {
                    let cname = pystack_hyperv::container_name(&stack.project, &name);
                    let status = hyperv_mgr
                        .container_health_status(&cname)
                        .unwrap_or_else(|_| "missing".to_string());
                    let bad = if svc.healthcheck.test.is_some() && svc.healthcheck.r#type != "none"
                    {
                        status != "healthy"
                    } else {
                        status != "running" && status != "healthy"
                    };
                    if bad && svc.restart != "no" {
                        println!("{}: {} under Hyper-V, restarting...", name, status);
                        let hyperv_svc = stack_service_to_hyperv(&stack, svc);
                        let _ = hyperv_mgr.stop_service(&hyperv_svc);
                        if let Err(err) = hyperv_mgr.start_service(&hyperv_svc, false) {
                            eprintln!("{}: restart failed: {}", name, err);
                        }
                    }
                }
                "native" | "" => {
                    if !native_mgr.health_ok(&name).unwrap_or(false) && svc.restart != "no" {
                        println!("{}: unhealthy, restarting...", name);
                        let _ = native_mgr.start(&name, true);
                    }
                }
                _ => {}
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(interval.max(1) as u64));
    }
}

fn cmd_register(
    name: &str,
    path: &str,
    backend: Option<&str>,
    config: &str,
    _allow_invalid: bool,
) -> Result<()> {
    let project_name = if name.is_empty() {
        PathBuf::from(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("default")
            .to_string()
    } else {
        name.to_string()
    };
    let reg_path = pystack_types::registry_file();
    let mut reg: HashMap<String, serde_json::Value> = if reg_path.exists() {
        let text = std::fs::read_to_string(&reg_path)?;
        serde_json::from_str(&text).unwrap_or_default()
    } else {
        HashMap::new()
    };
    reg.insert(project_name.clone(), serde_json::json!({
        "root": PathBuf::from(path).canonicalize().unwrap_or_else(|_| PathBuf::from(path)).to_string_lossy(),
        "config": config,
        "backend": backend.unwrap_or("native"),
    }));
    if let Some(parent) = reg_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&reg_path, serde_json::to_string_pretty(&reg)?)?;
    println!("Registered {}", project_name);
    Ok(())
}

fn cmd_unregister(name: &str) -> Result<()> {
    let reg_path = pystack_types::registry_file();
    let mut reg: HashMap<String, serde_json::Value> = if reg_path.exists() {
        let text = std::fs::read_to_string(&reg_path)?;
        serde_json::from_str(&text).unwrap_or_default()
    } else {
        HashMap::new()
    };
    if reg.remove(name).is_some() {
        std::fs::write(&reg_path, serde_json::to_string_pretty(&reg)?)?;
        println!("Unregistered {}", name);
    } else {
        println!("No registered project named {}", name);
    }
    Ok(())
}

async fn cmd_gui(host: &str, port: u16) -> Result<()> {
    let token = pystack_gui::get_or_create_token();
    let state = Arc::new(pystack_gui::AppState {
        token,
        registry_dir: pystack_types::registry_dir(),
    });
    pystack_gui::run_server(host, port, state)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Compose commands
// ---------------------------------------------------------------------------

fn cmd_compose_up(
    file: &str,
    backend: &str,
    detach: bool,
    build: bool,
    interval: u32,
    services: &[String],
) -> Result<()> {
    let stack = load_compose_stack(file, backend, services)?;
    println!("Project: {} (backend={})", stack.project, backend);
    let targets = if services.is_empty() {
        stack.services.keys().cloned().collect::<Vec<_>>()
    } else {
        services.to_vec()
    };
    run_stack_up(stack, backend, &targets, false, build, !detach, interval)
}

fn cmd_compose_build(file: &str, backend: &str, services: &[String]) -> Result<()> {
    let stack = load_compose_stack(file, backend, services)?;
    println!("Building project: {}", stack.project);
    let targets = if services.is_empty() {
        stack.services.keys().cloned().collect::<Vec<_>>()
    } else {
        services.to_vec()
    };

    let hyperv_mgr =
        pystack_hyperv::HyperVManager::new(pystack_hyperv::HyperVManager::load_config()?);
    let mut failed = Vec::new();
    for name in targets {
        let Some(svc) = stack.services.get(&name) else {
            failed.push(format!("{}: service not found", name));
            continue;
        };
        if effective_backend(backend, &svc.backend) == "hyperv" {
            let hyperv_svc = stack_service_to_hyperv(&stack, svc);
            if hyperv_svc.build.is_some() {
                let image = if hyperv_svc.image.is_empty() {
                    format!(
                        "stackdeck/{}-{}:latest",
                        hyperv_svc.project, hyperv_svc.name
                    )
                } else {
                    hyperv_svc.image.clone()
                };
                match hyperv_mgr.build_image(&hyperv_svc, &image) {
                    Ok(_) => println!("{}: built successfully", name),
                    Err(e) => failed.push(format!("{}: {}", name, e)),
                }
            } else {
                println!("{}: no build configuration", name);
            }
        }
    }
    if !failed.is_empty() {
        anyhow::bail!("Build failed for: {}", failed.join(", "));
    }
    Ok(())
}

fn cmd_compose_down(file: &str, backend: &str, volumes: bool, services: &[String]) -> Result<()> {
    let stack = load_compose_stack(file, backend, &[])?;
    println!("Stopping project: {}", stack.project);
    let targets = if services.is_empty() {
        stack.services.keys().cloned().collect::<Vec<_>>()
    } else {
        services.to_vec()
    };
    run_stack_down(stack, backend, &targets, volumes)
}

fn cmd_compose_status(file: &str, backend: &str, json_output: bool) -> Result<()> {
    let stack = load_compose_stack(file, backend, &[])?;
    run_stack_status(stack, backend, json_output)
}

fn cmd_compose_logs(file: &str, backend: &str, service: &str, tail: u32) -> Result<()> {
    let stack = load_compose_stack(file, backend, &[])?;
    if !stack.services.contains_key(service) {
        anyhow::bail!("Service '{}' not found in compose file", service);
    }
    run_stack_logs(stack, backend, service, tail)
}

// ---------------------------------------------------------------------------
// Docker-like top-level command aliases
// ---------------------------------------------------------------------------

fn hyperv_manager() -> Result<pystack_hyperv::HyperVManager> {
    Ok(pystack_hyperv::HyperVManager::new(
        pystack_hyperv::HyperVManager::load_config()?,
    ))
}

fn cmd_docker_ps(all: bool) -> Result<()> {
    let mgr = hyperv_manager()?;
    if all {
        println!("{}", mgr.container_ps()?);
    } else {
        println!(
            "{}",
            mgr.ssh(
                &pystack_hyperv::nerdctl_command(mgr.config(), &["ps"]),
                false,
            )?
        );
    }
    Ok(())
}

fn cmd_docker_images(all: bool) -> Result<()> {
    println!("{}", hyperv_manager()?.image_list(all)?);
    Ok(())
}

fn cmd_docker_pull(image: &str) -> Result<()> {
    let mgr = hyperv_manager()?;
    mgr.ssh_stream(
        &pystack_hyperv::nerdctl_command(mgr.config(), &["pull", image]),
        true,
    )?;
    Ok(())
}

fn cmd_docker_rmi(force: bool, images: &[String]) -> Result<()> {
    if images.is_empty() {
        anyhow::bail!("at least one image is required");
    }
    let mgr = hyperv_manager()?;
    println!(
        "{}",
        mgr.image_remove(
            &images.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            force
        )?
    );
    Ok(())
}

fn cmd_docker_build(
    tags: &[String],
    dockerfile: &str,
    build_args: &[String],
    context: &str,
) -> Result<()> {
    if tags.is_empty() {
        anyhow::bail!("docker-like build requires at least one -t/--tag value");
    }
    let root = std::fs::canonicalize(context).unwrap_or_else(|_| PathBuf::from(context));
    let args_map: HashMap<String, String> = build_args
        .iter()
        .filter_map(|item| item.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    let build = serde_json::json!({
        "context": ".",
        "dockerfile": dockerfile,
        "args": args_map,
    });
    let svc = pystack_types::HyperVService {
        project: "docker-build".to_string(),
        name: "image".to_string(),
        root,
        image: tags[0].clone(),
        build: Some(build),
        command: None,
        env: HashMap::new(),
        ports: Vec::new(),
        volumes: Vec::new(),
        networks: Vec::new(),
        restart: "no".to_string(),
        secrets: Vec::new(),
        configs: Vec::new(),
        secret_resources: HashMap::new(),
        config_resources: HashMap::new(),
        healthcheck: None,
    };
    let mgr = hyperv_manager()?;
    mgr.build_image(&svc, &tags[0])?;
    for extra in tags.iter().skip(1) {
        mgr.ssh(
            &pystack_hyperv::nerdctl_command(mgr.config(), &["tag", &tags[0], extra]),
            true,
        )?;
    }
    Ok(())
}

fn cmd_docker_login(registry: &str, username: &str, password: &str) -> Result<()> {
    println!(
        "{}",
        hyperv_manager()?.image_login(registry, username, password)?
    );
    Ok(())
}

fn cmd_docker_inspect(containers: &[String]) -> Result<()> {
    if containers.is_empty() {
        anyhow::bail!("at least one container is required");
    }
    let mgr = hyperv_manager()?;
    let refs = containers.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    println!(
        "{}",
        serde_json::to_string_pretty(&mgr.inspect_containers(&refs)?)?
    );
    Ok(())
}

fn cmd_docker_exec(container: &str, command: &[String]) -> Result<()> {
    if command.is_empty() {
        anyhow::bail!("exec requires a command");
    }
    println!(
        "{}",
        hyperv_manager()?.exec_container(
            container,
            &command.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        )?
    );
    Ok(())
}

fn cmd_docker_container_command(command: &str, check: bool, containers: &[String]) -> Result<()> {
    if containers.is_empty() {
        anyhow::bail!("at least one container is required");
    }
    let mgr = hyperv_manager()?;
    for container in containers {
        println!(
            "{}",
            mgr.ssh(
                &pystack_hyperv::nerdctl_command(mgr.config(), &[command, container]),
                check,
            )?
        );
    }
    Ok(())
}

fn cmd_docker_rm(force: bool, containers: &[String]) -> Result<()> {
    if containers.is_empty() {
        anyhow::bail!("at least one container is required");
    }
    let mgr = hyperv_manager()?;
    let mut args = vec!["rm".to_string()];
    if force {
        args.push("-f".to_string());
    }
    args.extend(containers.iter().cloned());
    println!(
        "{}",
        mgr.ssh(&pystack_hyperv::nerdctl_command(mgr.config(), &args), false)?
    );
    Ok(())
}

fn cmd_docker_volume(cmd: VolumeCommands) -> Result<()> {
    let mgr = hyperv_manager()?;
    match cmd {
        VolumeCommands::Ls => println!("{}", mgr.volume_list()?),
        VolumeCommands::Create { name, .. } => println!("{}", mgr.volume_create(&name)?),
        VolumeCommands::Rm { force, volumes } => println!(
            "{}",
            mgr.volume_remove(
                &volumes.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                force
            )?
        ),
        VolumeCommands::Prune => println!("{}", mgr.volume_prune()?),
        VolumeCommands::Inspect { volumes } => println!(
            "{}",
            mgr.volume_inspect(&volumes.iter().map(|s| s.as_str()).collect::<Vec<_>>())?
        ),
    }
    Ok(())
}

fn cmd_docker_network(cmd: NetworkCommands) -> Result<()> {
    let mgr = hyperv_manager()?;
    match cmd {
        NetworkCommands::Ls => println!("{}", mgr.network_list()?),
        NetworkCommands::Create { name, .. } => println!("{}", mgr.network_create(&name)?),
        NetworkCommands::Rm { networks } => println!(
            "{}",
            mgr.network_remove(&networks.iter().map(|s| s.as_str()).collect::<Vec<_>>())?
        ),
        NetworkCommands::Inspect { networks } => println!(
            "{}",
            mgr.network_inspect(&networks.iter().map(|s| s.as_str()).collect::<Vec<_>>())?
        ),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Hyper-V commands
// ---------------------------------------------------------------------------

fn run_hyperv_command(cmd: HypervCommands) -> Result<()> {
    let cfg = pystack_hyperv::HyperVManager::load_config()?;
    let mgr = pystack_hyperv::HyperVManager::new(cfg);

    match cmd {
        HypervCommands::Doctor => {
            let checks = mgr.preflight();
            let max_len = checks.keys().map(|k| k.len()).max().unwrap_or(10);
            for (key, value) in &checks {
                println!("{:>width$}  {}", key, value, width = max_len);
            }
        }
        HypervCommands::Health => {
            let health = mgr.runtime_health_check()?;
            for (key, value) in &health {
                println!("{:<20} {}", key, value);
            }
        }
        HypervCommands::Ps => {
            let output = mgr.container_ps()?;
            println!("{}", output);
        }
        HypervCommands::StartVm => {
            let state = mgr.vm_start()?;
            println!("VM state: {}", state);
        }
        HypervCommands::StopVm => {
            let state = mgr.vm_stop()?;
            println!("VM state: {}", state);
        }
        HypervCommands::Ip => {
            let ip = mgr.vm_ip()?;
            println!("{}", ip);
        }
        HypervCommands::DiscoverIp { timeout } => {
            println!("Discovering IP (timeout={}s)...", timeout);
            let ip = mgr.discover_ip(timeout)?;
            let mut cfg = mgr.config().clone();
            cfg.ssh_host = ip.clone();
            pystack_hyperv::HyperVManager::save_config(&cfg)?;
            println!("VM IP: {}", ip);
        }
        HypervCommands::Bootstrap => {
            println!("Bootstrapping container runtime...");
            mgr.bootstrap()?;
        }
        HypervCommands::Init {
            image_vhdx,
            url,
            sha256,
            timeout,
        } => {
            println!("Initializing Hyper-V runtime...");
            mgr.init_vm(&image_vhdx, url.as_deref(), sha256.as_deref(), timeout)?;
        }
        HypervCommands::CreateVm { iso } => {
            println!("Creating VM...");
            mgr.create_vm(&iso)?;
        }
        HypervCommands::CreateCloudVm {
            image_vhdx,
            no_start,
            discover_ip,
            timeout,
        } => {
            println!("Creating cloud VM...");
            mgr.create_cloud_vm(&image_vhdx, no_start, discover_ip, timeout)?;
        }
        HypervCommands::DownloadImage {
            url,
            output,
            sha256,
            force,
        } => {
            println!("Downloading image...");
            mgr.download_image(
                &url,
                &output,
                if sha256.is_empty() {
                    None
                } else {
                    Some(&sha256)
                },
                force,
            )?;
        }
        HypervCommands::Repair { timeout } => {
            println!("Repairing runtime (timeout={}s)...", timeout);
            mgr.repair(timeout)?;
        }
        HypervCommands::EnsureKey => {
            let public_key = mgr.ensure_ssh_key()?;
            let mut cfg = mgr.config().clone();
            cfg.ssh_public_key = public_key;
            if cfg.ssh_identity.is_empty() {
                cfg.ssh_identity = pystack_types::registry_dir()
                    .join("id_ed25519")
                    .to_string_lossy()
                    .to_string();
            }
            pystack_hyperv::HyperVManager::save_config(&cfg)?;
            println!("SSH key is ready: {}", cfg.ssh_identity);
        }
        HypervCommands::Configure {
            vm_name,
            ssh_host,
            ssh_user,
            ssh_port,
            ssh_identity,
            switch_name,
            memory_mb,
            cpus,
            disk_gb,
            vm_root,
            portproxy,
            windows_host,
            smb_user,
            smb_password,
        } => {
            let mut cfg = mgr.config().clone();
            if let Some(v) = vm_name {
                cfg.vm_name = v;
            }
            if let Some(v) = ssh_host {
                cfg.ssh_host = v;
            }
            if let Some(v) = ssh_user {
                cfg.ssh_user = v;
            }
            if let Some(v) = ssh_port {
                cfg.ssh_port = v;
            }
            if let Some(v) = ssh_identity {
                cfg.ssh_identity = v;
            }
            if let Some(v) = switch_name {
                cfg.switch_name = v;
            }
            if let Some(v) = memory_mb {
                cfg.vm_memory_mb = v;
            }
            if let Some(v) = cpus {
                cfg.vm_cpu_count = v;
            }
            if let Some(v) = disk_gb {
                cfg.vm_disk_gb = v;
            }
            if let Some(v) = vm_root {
                cfg.vm_root = v;
            }
            if let Some(v) = portproxy {
                cfg.portproxy = v;
            }
            if let Some(v) = windows_host {
                cfg.windows_host = v;
            }
            if let Some(v) = smb_user {
                cfg.smb_user = v;
            }
            if let Some(v) = smb_password {
                cfg.smb_password = v;
            }
            pystack_hyperv::HyperVManager::save_config(&cfg)?;
            println!("Configuration saved.");
        }
        HypervCommands::Exec { container, command } => {
            let output = mgr.exec_container(
                &container,
                &command.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            )?;
            println!("{}", output);
        }
        HypervCommands::Image { image_cmd } => match image_cmd {
            ImageCommands::Ls { all } => {
                let output = mgr.image_list(all)?;
                println!("{}", output);
            }
            ImageCommands::Rm { force, images } => {
                let output = mgr.image_remove(
                    &images.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                    force,
                )?;
                println!("{}", output);
            }
            ImageCommands::Prune { all } => {
                let output = mgr.image_prune(all)?;
                println!("{}", output);
            }
            ImageCommands::Login {
                registry,
                username,
                password,
            } => {
                let output = mgr.image_login(&registry, &username, &password)?;
                println!("{}", output);
            }
        },
        HypervCommands::Volume { volume_cmd } => match volume_cmd {
            VolumeCommands::Ls => {
                let output = mgr.volume_list()?;
                println!("{}", output);
            }
            VolumeCommands::Create { name, .. } => {
                let output = mgr.volume_create(&name)?;
                println!("{}", output);
            }
            VolumeCommands::Rm { force, volumes } => {
                let output = mgr.volume_remove(
                    &volumes.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                    force,
                )?;
                println!("{}", output);
            }
            VolumeCommands::Prune => {
                let output = mgr.volume_prune()?;
                println!("{}", output);
            }
            VolumeCommands::Inspect { volumes } => {
                let output =
                    mgr.volume_inspect(&volumes.iter().map(|s| s.as_str()).collect::<Vec<_>>())?;
                println!("{}", output);
            }
        },
        HypervCommands::Network { network_cmd } => match network_cmd {
            NetworkCommands::Ls => {
                let output = mgr.network_list()?;
                println!("{}", output);
            }
            NetworkCommands::Create { name, .. } => {
                let output = mgr.network_create(&name)?;
                println!("{}", output);
            }
            NetworkCommands::Rm { networks } => {
                let output =
                    mgr.network_remove(&networks.iter().map(|s| s.as_str()).collect::<Vec<_>>())?;
                println!("{}", output);
            }
            NetworkCommands::Inspect { networks } => {
                let output =
                    mgr.network_inspect(&networks.iter().map(|s| s.as_str()).collect::<Vec<_>>())?;
                println!("{}", output);
            }
        },
        HypervCommands::Snapshot { snapshot_cmd } => match snapshot_cmd {
            SnapshotCommands::Create { name } => {
                println!("{}", mgr.snapshot_create(name.as_deref())?)
            }
            SnapshotCommands::Ls => println!("{}", mgr.snapshot_list()?),
            SnapshotCommands::Restore { name } => println!("{}", mgr.snapshot_restore(&name)?),
            SnapshotCommands::Rm { name } => println!("{}", mgr.snapshot_remove(&name)?),
            SnapshotCommands::Export { name, output } => {
                println!("{}", mgr.snapshot_export(&name, &output)?)
            }
        },
        HypervCommands::Share { share_cmd } => match share_cmd {
            ShareCommands::Add { path, name } => {
                println!("{}", mgr.share_add(&path, name.as_deref())?)
            }
            ShareCommands::Mount { name } => println!("{}", mgr.share_mount(&name)?),
        },
        HypervCommands::Mirror { mirror_cmd } => match mirror_cmd {
            MirrorCommands::Ls => println!("{}", mgr.mirror_list()?),
            MirrorCommands::Set {
                registry,
                endpoints,
            } => {
                mgr.mirror_set(&registry, &endpoints)?;
                println!("mirror saved");
            }
            MirrorCommands::Rm { registry } => {
                mgr.mirror_remove(&registry)?;
                println!("mirror removed");
            }
            MirrorCommands::Apply => {
                mgr.apply_registry_mirrors()?;
                println!("mirrors applied");
            }
        },
    }

    Ok(())
}

async fn cmd_daemon_serve(host: &str, port: u16, allow_remote: bool) -> Result<()> {
    println!("Starting Docker API shim on http://{}:{}", host, port);
    pystack_api::serve(host, port, allow_remote).await?;
    Ok(())
}

fn preflight_check() -> Result<()> {
    let output = std::process::Command::new("where.exe").arg("tar").output();
    if output.is_err() || !output.unwrap().status.success() {
        eprintln!("WARNING: tar.exe not found in PATH. Some features might not work properly.");
    }
    Ok(())
}
