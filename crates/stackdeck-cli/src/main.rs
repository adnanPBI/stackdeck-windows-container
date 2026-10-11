//! StackDeck Runner CLI entry point.
//!
//! Replaces `__main__.py` and the argparse CLI from `core.py`.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_CONFIG: &str = "stack.json";

#[derive(Parser)]
#[command(
    name = "stackdeck-cli",
    version = VERSION,
    about = "Docker-free process orchestrator"
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

        /// Activate Compose profiles
        #[arg(long = "profile")]
        profiles: Vec<String>,

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

        /// Activate Compose profiles
        #[arg(long = "profile")]
        profiles: Vec<String>,

        services: Vec<String>,
    },

    /// Stop compose services
    Down {
        #[arg(short = 'f', long, default_value = "docker-compose.yml")]
        file: String,

        #[arg(long, default_value = "hyperv")]
        backend: String,

        /// Activate Compose profiles
        #[arg(long = "profile")]
        profiles: Vec<String>,

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

        /// Activate Compose profiles
        #[arg(long = "profile")]
        profiles: Vec<String>,

        #[arg(long)]
        json: bool,
    },

    /// Show compose service logs
    Logs {
        #[arg(short = 'f', long, default_value = "docker-compose.yml")]
        file: String,

        #[arg(long, default_value = "hyperv")]
        backend: String,

        /// Activate Compose profiles
        #[arg(long = "profile")]
        profiles: Vec<String>,

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
        /// Read the SMB password from stdin instead of exposing it in argv.
        #[arg(long, conflicts_with = "smb_password")]
        smb_password_stdin: bool,
    },

    /// Ensure SSH key exists
    EnsureKey,

    /// Download cloud image
    DownloadImage {
        /// Image URL. If omitted, uses the immutable release runtime lock.
        #[arg(long)]
        url: Option<String>,
        #[arg(long, default_value = "")]
        output: String,
        #[arg(long, default_value = "")]
        sha256: String,
        #[arg(long, default_value = "")]
        sha512: String,
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
        #[arg(long)]
        sha512: Option<String>,
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

fn load_stack(config_path: &str) -> Result<stackdeck_types::StackConfig> {
    load_stack_with_backend(config_path, "native")
}

fn load_stack_with_backend(
    config_path: &str,
    backend_override: &str,
) -> Result<stackdeck_types::StackConfig> {
    let path = PathBuf::from(config_path);
    if !path.exists() {
        anyhow::bail!("Configuration file not found: {}", config_path);
    }
    if is_compose_path(&path) {
        let project = stackdeck_compose::load_compose_file(&path, None, &[], None, None)?;
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
    let state_dir = root.join(".stackdeck");
    let log_dir = root.join(".stackdeck/logs");

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

    Ok(stackdeck_types::StackConfig {
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
    stack: &stackdeck_types::StackConfig,
    backend_override: &str,
) -> Result<()> {
    for (name, svc) in &stack.services {
        if matches!(
            effective_backend(backend_override, &svc.backend),
            "native" | ""
        ) && command_args(&svc.command).is_empty()
            && svc.entrypoint.is_none()
        {
            anyhow::bail!("{name}: native service command must not be empty");
        }
        if svc.replicas == 0 {
            anyhow::bail!("{name}: replicas must be at least 1");
        }
        if matches!(
            effective_backend(backend_override, &svc.backend),
            "native" | ""
        ) && (!svc.secrets_meta.is_empty() || !svc.configs_meta.is_empty())
        {
            anyhow::bail!(
                "{name}: Compose secrets/configs require the Hyper-V backend; refusing to ignore them"
            );
        }
        if svc
            .build
            .as_ref()
            .and_then(|build| build.get("extra"))
            .and_then(|extra| extra.as_object())
            .is_some_and(|extra| !extra.is_empty())
        {
            anyhow::bail!(
                "{name}: unsupported Compose build options are present; refusing to ignore build semantics"
            );
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
    project: stackdeck_types::ComposeProject,
    config_path: &Path,
    backend: &str,
) -> Result<stackdeck_types::StackConfig> {
    for (name, network) in &project.networks {
        if network
            .driver
            .as_deref()
            .is_some_and(|driver| driver != "bridge")
            || !network.options.is_empty()
        {
            anyhow::bail!(
                "compose network '{}' requests unsupported driver/options; refusing to ignore network semantics",
                name
            );
        }
    }
    for (name, volume) in &project.volumes {
        if volume
            .driver
            .as_deref()
            .is_some_and(|driver| driver != "local")
            || !volume.options.is_empty()
        {
            anyhow::bail!(
                "compose volume '{}' requests unsupported driver/options; refusing to ignore volume semantics",
                name
            );
        }
    }
    let root = project.working_dir.clone();
    let state_dir = root.join(".stackdeck");
    let log_dir = root.join(".stackdeck/logs");
    let mut services = HashMap::new();
    for (name, service) in &project.services {
        if service.replicas == 0 {
            anyhow::bail!("compose service '{}' replicas must be at least 1", name);
        }
        if service.replicas > 1 && service.ports.iter().any(|port| port.published.is_some()) {
            anyhow::bail!(
                "compose service '{}' cannot scale fixed published ports without a load balancer",
                name
            );
        }
        if service.replicas > 1 && service.container_name.is_some() {
            anyhow::bail!(
                "compose service '{}' cannot combine container_name with replicas",
                name
            );
        }
        for replica in 1..=service.replicas.max(1) {
            let instance_name = if replica == 1 {
                name.clone()
            } else {
                format!("{name}-{replica}")
            };
            let mut normalized = compose_service_to_stack_service(&project, service, backend)?;
            normalized.name = instance_name.clone();
            normalized.replicas = 1;
            services.insert(instance_name, normalized);
        }
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
    Ok(stackdeck_types::StackConfig {
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
    project: &stackdeck_types::ComposeProject,
    svc: &stackdeck_types::ComposeService,
    backend: &str,
) -> Result<stackdeck_types::ServiceConfig> {
    if !svc.extra.is_empty() {
        anyhow::bail!(
            "compose service '{}' contains unsupported keys {:?}; refusing to ignore semantics",
            svc.name,
            svc.extra.keys().collect::<Vec<_>>()
        );
    }
    if svc.container_name.is_some() {
        anyhow::bail!(
            "compose service '{}' uses container_name, which is not supported by StackDeck naming",
            svc.name
        );
    }
    if svc.networks.iter().any(|network| {
        !network.aliases.is_empty()
            || network.ipv4_address.is_some()
            || network.ipv6_address.is_some()
            || network.priority.is_some()
    }) {
        anyhow::bail!(
            "compose service '{}' requests network aliases/static addressing/priority; refusing to ignore network semantics",
            svc.name
        );
    }
    let command = svc.command.clone().unwrap_or_else(|| serde_json::json!([]));
    if backend == "native" && command == serde_json::json!([]) && svc.entrypoint.is_none() {
        anyhow::bail!(
            "compose service '{}' has no native command; use --backend hyperv for image services",
            svc.name
        );
    }
    Ok(stackdeck_types::ServiceConfig {
        name: svc.name.clone(),
        cwd: project.working_dir.clone(),
        command,
        entrypoint: svc.entrypoint.clone(),
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
        max_restarts: svc.restart.maximum_retry_count.unwrap_or(5),
        healthcheck: compose_healthcheck(svc.healthcheck.as_ref()),
        ports: svc.ports.iter().filter_map(compose_port_string).collect(),
        volumes: svc
            .volumes
            .iter()
            .filter_map(|volume| compose_volume_string(project, volume))
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
        logging: svc.logging.clone(),
        backend: backend.to_string(),
        replicas: svc.replicas,
        secrets_meta: svc.secrets.clone(),
        configs_meta: svc.configs.clone(),
        ..Default::default()
    })
}

fn compose_environment(
    project: &stackdeck_types::ComposeProject,
    svc: &stackdeck_types::ComposeService,
) -> Result<HashMap<String, String>> {
    let mut env = HashMap::new();
    for file in &svc.env_file {
        let path = PathBuf::from(file);
        let path = if path.is_absolute() {
            path
        } else {
            project.working_dir.join(path)
        };
        env.extend(stackdeck_compose::parse_env_file(&path)?);
    }
    for (key, value) in &svc.environment {
        if let Some(value) = value {
            env.insert(key.clone(), value.clone());
        } else {
            let env_path = project.working_dir.join(".env");
            let proj_env = stackdeck_compose::parse_env_file(&env_path).unwrap_or_default();
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
    healthcheck: Option<&stackdeck_types::ComposeHealthcheck>,
) -> stackdeck_types::HealthCheck {
    let Some(healthcheck) = healthcheck else {
        return Default::default();
    };
    if healthcheck.disable {
        return Default::default();
    }
    stackdeck_types::HealthCheck {
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

fn compose_port_string(port: &stackdeck_types::ComposePort) -> Option<String> {
    let published = port.published?;
    let mut s = String::new();
    if let Some(host_ip) = &port.host_ip {
        if !host_ip.is_empty() {
            s.push_str(host_ip);
            s.push(':');
        }
    }

    s.push_str(&published.to_string());
    s.push(':');

    s.push_str(&port.target.to_string());

    if port.protocol != "tcp" && !port.protocol.is_empty() {
        s.push('/');
        s.push_str(&port.protocol);
    }

    Some(s)
}

fn compose_volume_string(
    project: &stackdeck_types::ComposeProject,
    volume: &stackdeck_types::ComposeVolumeMount,
) -> Option<String> {
    let mut spec = if let Some(source) = volume.source.as_ref() {
        let resolved_source = if volume.r#type == "volume" {
            project
                .volumes
                .get(source)
                .map(|resource| resource.name.as_str())
                .unwrap_or(source)
        } else {
            source
        };
        format!("{}:{}", resolved_source, volume.target)
    } else {
        // A container-only target is an anonymous volume in Compose.
        volume.target.clone()
    };
    if volume.read_only {
        spec.push_str(":ro");
    }
    Some(spec)
}

fn removable_hyperv_volume_names(stack: &stackdeck_types::StackConfig) -> Vec<String> {
    let mut volume_names = Vec::new();
    for (vname, vconfig) in &stack.volumes {
        let is_external = vconfig
            .get("external")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if is_external {
            continue;
        }
        let name = vconfig
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                if stack.source_format == "compose" {
                    format!("{}_{}", stack.project, vname)
                } else {
                    format!(
                        "{}_{}",
                        stackdeck_hyperv::project_slug(&stack.project),
                        vname
                    )
                }
            });
        volume_names.push(name);
    }
    volume_names.sort();
    volume_names
}

fn stack_service_to_hyperv(
    stack: &stackdeck_types::StackConfig,
    svc: &stackdeck_types::ServiceConfig,
) -> stackdeck_hyperv::HyperVService {
    stackdeck_hyperv::HyperVService {
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
        entrypoint: svc.entrypoint.clone(),
        env: svc.env.clone(),
        ports: svc.ports.clone(),
        volumes: svc.volumes.clone(),
        networks: svc.networks.clone(),
        restart: svc.restart.clone(),
        restart_max_retries: Some(svc.max_restarts),
        logging: svc.logging.clone(),
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
    profiles: &[String],
) -> Result<stackdeck_types::StackConfig> {
    let path = PathBuf::from(file);
    let project = stackdeck_compose::load_compose_file(&path, None, profiles, None, None)?;
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
) -> Result<stackdeck_types::ServiceConfig> {
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
    let entrypoint = obj.get("entrypoint").cloned();

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
    let healthcheck = stackdeck_types::HealthCheck {
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
    let resources = stackdeck_types::ResourceLimits {
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

    Ok(stackdeck_types::ServiceConfig {
        name: name.to_string(),
        cwd,
        command,
        entrypoint,
        env,
        depends_on,
        depends_on_conditions,
        restart,
        healthcheck,
        resources,
        backend,
        image,
        logging: obj.get("logging").cloned(),
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

        // ---- Compose commands ----
        Commands::Compose { compose_cmd } => match compose_cmd {
            ComposeCommands::Up {
                file,
                backend,
                profiles,
                detach,
                build,
                interval,
                services,
            } => cmd_compose_up(
                &file, &backend, &profiles, detach, build, interval, &services,
            )?,
            ComposeCommands::Build {
                file,
                backend,
                profiles,
                services,
            } => cmd_compose_build(&file, &backend, &profiles, &services)?,
            ComposeCommands::Down {
                file,
                backend,
                profiles,
                volumes,
                services,
            } => cmd_compose_down(&file, &backend, &profiles, volumes, &services)?,
            ComposeCommands::Status {
                file,
                backend,
                profiles,
                json,
            } => cmd_compose_status(&file, &backend, &profiles, json)?,
            ComposeCommands::Logs {
                file,
                backend,
                profiles,
                service,
                tail,
            } => cmd_compose_logs(&file, &backend, &profiles, &service, tail)?,
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
    let cfg = stackdeck_types::HyperVConfig::default();
    let mgr = stackdeck_hyperv::HyperVManager::new(cfg);
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
    let mut native_mgr = stackdeck_process::ProcessManager::new(stack.clone())?;
    let hyperv_mgr =
        stackdeck_hyperv::HyperVManager::new(stackdeck_hyperv::HyperVManager::load_config()?);
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
        let volume_names = removable_hyperv_volume_names(&stack);
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
    let has_hyperv = backend_override == "hyperv"
        || stack
            .services
            .values()
            .any(|svc| effective_backend(backend_override, &svc.backend) == "hyperv");
    let has_native = stack.services.values().any(|svc| {
        matches!(
            effective_backend(backend_override, &svc.backend),
            "native" | ""
        )
    });
    let mut hyperv_status: HashMap<String, serde_json::Value> = HashMap::new();
    if has_hyperv {
        let hyperv_mgr =
            stackdeck_hyperv::HyperVManager::new(stackdeck_hyperv::HyperVManager::load_config()?);
        let names: Vec<String> = stack
            .services
            .values()
            .filter(|svc| effective_backend(backend_override, &svc.backend) == "hyperv")
            .map(|svc| stackdeck_hyperv::container_name(&stack.project, &svc.name))
            .collect();
        let refs: Vec<&str> = names.iter().map(|name| name.as_str()).collect();
        let inspected = hyperv_mgr.inspect_containers(&refs)?;
        if !json_output {
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
        hyperv_status = inspected;
    }
    let native_status = if has_native {
        let mgr = stackdeck_process::ProcessManager::new(stack.clone())?;
        mgr.db().all().map_err(|e| anyhow::anyhow!("{}", e))?
    } else {
        Vec::new()
    };
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "native": native_status,
                "hyperv": hyperv_status,
            }))?
        );
    } else if has_native {
        println!(
            "{:<20} {:<10} {:<8} {}",
            "SERVICE", "STATUS", "PID", "RESTARTS"
        );
        for svc in &native_status {
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
    stack: stackdeck_types::StackConfig,
    backend_override: &str,
    targets: &[String],
    force: bool,
    build: bool,
    supervise: bool,
    interval: u32,
) -> Result<()> {
    validate_stack_for_run(&stack, backend_override)?;
    let mut native_mgr = stackdeck_process::ProcessManager::new(stack.clone())?;
    let hyperv_mgr =
        stackdeck_hyperv::HyperVManager::new(stackdeck_hyperv::HyperVManager::load_config()?);

    let mut ordered_targets = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut visiting = std::collections::HashSet::new();

    fn visit(
        name: &str,
        stack: &stackdeck_types::StackConfig,
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
        let mut restart_attempts: HashMap<String, u32> = HashMap::new();
        let mut restart_after: HashMap<String, std::time::Instant> = HashMap::new();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(interval.max(1) as u64));
            for name in &ordered_targets {
                let Some(svc) = stack.services.get(name) else {
                    continue;
                };
                match effective_backend(backend_override, &svc.backend) {
                    "hyperv" => {
                        let cname = stackdeck_hyperv::container_name(&stack.project, name);
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
                            if !restart_is_due(name, svc, &mut restart_attempts, &mut restart_after)
                            {
                                continue;
                            }
                            println!("{}: {} under Hyper-V, restarting...", name, status);
                            let hyperv_svc = stack_service_to_hyperv(&stack, svc);
                            let _ = hyperv_mgr.stop_service(&hyperv_svc);
                            if let Err(err) = hyperv_mgr.start_service(&hyperv_svc, build) {
                                eprintln!("{}: restart failed: {}", name, err);
                            }
                        } else if !bad {
                            restart_attempts.remove(name);
                            restart_after.remove(name);
                        }
                    }
                    "native" | "" => {
                        if !native_mgr.health_ok(name).unwrap_or(false) && svc.restart != "no" {
                            if !restart_is_due(name, svc, &mut restart_attempts, &mut restart_after)
                            {
                                continue;
                            }
                            println!("{}: unhealthy, restarting...", name);
                            if let Err(err) = native_mgr.start(name, true) {
                                eprintln!("{}: restart failed: {}", name, err);
                            }
                        } else {
                            restart_attempts.remove(name);
                            restart_after.remove(name);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

fn restart_is_due(
    name: &str,
    svc: &stackdeck_types::ServiceConfig,
    attempts: &mut HashMap<String, u32>,
    restart_after: &mut HashMap<String, std::time::Instant>,
) -> bool {
    let now = std::time::Instant::now();
    if restart_after
        .get(name)
        .is_some_and(|deadline| now < *deadline)
    {
        return false;
    }
    let attempt = attempts.entry(name.to_string()).or_default();
    if *attempt >= svc.max_restarts {
        return false;
    }
    *attempt += 1;
    let exponent = (*attempt - 1).min(8);
    let delay = (svc.restart_backoff_seconds as u64)
        .saturating_mul(1_u64 << exponent)
        .min(300);
    restart_after.insert(
        name.to_string(),
        now + std::time::Duration::from_secs(delay),
    );
    true
}

#[derive(Debug, Clone)]
struct DependencySpec {
    service: String,
    condition: String,
    required: bool,
}

fn dependency_specs(svc: &stackdeck_types::ServiceConfig) -> Vec<DependencySpec> {
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
    stack: &stackdeck_types::StackConfig,
    backend_override: &str,
    hyperv_mgr: &stackdeck_hyperv::HyperVManager,
    native_mgr: &mut stackdeck_process::ProcessManager,
    dep: &DependencySpec,
    timeout_secs: u64,
) -> Result<bool> {
    let started = std::time::Instant::now();
    loop {
        let Some(dep_svc) = stack.services.get(&dep.service) else {
            return Ok(!dep.required);
        };
        let ready = if effective_backend(backend_override, &dep_svc.backend) == "hyperv" {
            let cname = stackdeck_hyperv::container_name(&stack.project, &dep.service);
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
                            || (status == "running"
                                && hyperv_mgr
                                    .service_healthcheck_ok(&stack_service_to_hyperv(
                                        stack, dep_svc,
                                    ))
                                    .unwrap_or(false))
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
                "service_completed_successfully" => native_mgr
                    .service_completed_successfully(&dep.service)
                    .unwrap_or(false),
                _ => {
                    native_mgr.health_ok(&dep.service).unwrap_or(false)
                        || native_mgr
                            .service_completed_successfully(&dep.service)
                            .unwrap_or(false)
                }
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
    stack: stackdeck_types::StackConfig,
    backend_override: &str,
    targets: &[String],
    volumes: bool,
) -> Result<()> {
    let mut native_mgr = stackdeck_process::ProcessManager::new(stack.clone())?;
    let hyperv_mgr =
        stackdeck_hyperv::HyperVManager::new(stackdeck_hyperv::HyperVManager::load_config()?);
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
        let volume_names = removable_hyperv_volume_names(&stack);
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
    stack: stackdeck_types::StackConfig,
    backend_override: &str,
    json_output: bool,
) -> Result<()> {
    let has_hyperv = backend_override == "hyperv"
        || stack
            .services
            .values()
            .any(|svc| effective_backend(backend_override, &svc.backend) == "hyperv");
    let has_native = stack.services.values().any(|svc| {
        matches!(
            effective_backend(backend_override, &svc.backend),
            "native" | ""
        )
    });
    let mut hyperv_status: HashMap<String, serde_json::Value> = HashMap::new();
    if has_hyperv {
        let hyperv_mgr =
            stackdeck_hyperv::HyperVManager::new(stackdeck_hyperv::HyperVManager::load_config()?);
        let names: Vec<String> = stack
            .services
            .values()
            .filter(|svc| effective_backend(backend_override, &svc.backend) == "hyperv")
            .map(|svc| stackdeck_hyperv::container_name(&stack.project, &svc.name))
            .collect();
        let refs: Vec<&str> = names.iter().map(|name| name.as_str()).collect();
        let inspected = hyperv_mgr.inspect_containers(&refs)?;
        if !json_output {
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
        hyperv_status = inspected;
    }

    let native_status = if has_native {
        let mgr = stackdeck_process::ProcessManager::new(stack)?;
        mgr.db().all().map_err(|e| anyhow::anyhow!("{}", e))?
    } else {
        Vec::new()
    };
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "native": native_status,
                "hyperv": hyperv_status,
            }))?
        );
    } else if has_native {
        println!(
            "{:<20} {:<10} {:<8} {}",
            "SERVICE", "STATUS", "PID", "RESTARTS"
        );
        for svc in &native_status {
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
    stack: stackdeck_types::StackConfig,
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
            stackdeck_hyperv::HyperVManager::new(stackdeck_hyperv::HyperVManager::load_config()?);
        let container = stackdeck_hyperv::container_name(&stack.project, service);
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
    if let Ok(mgr) = stackdeck_process::ProcessManager::new(stack.clone()) {
        if let Ok(all) = mgr.db().all() {
            status_json["native"] =
                serde_json::to_value(all).unwrap_or_else(|_| serde_json::json!([]));
        }
    }
    if let Ok(cfg) = stackdeck_hyperv::HyperVManager::load_config() {
        let mgr = stackdeck_hyperv::HyperVManager::new(cfg);
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
    for hint in stackdeck_types::SAFE_ENV_NAME_HINTS {
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
            .replace_all(&out, format!("$1{}$3", stackdeck_types::REDACTION))
            .to_string();
        out = re2
            .replace_all(&out, format!("$1{}", stackdeck_types::REDACTION))
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
    let mut native_mgr = stackdeck_process::ProcessManager::new(stack.clone())?;
    let hyperv_mgr =
        stackdeck_hyperv::HyperVManager::new(stackdeck_hyperv::HyperVManager::load_config()?);
    println!(
        "Watching services (interval={}s, Ctrl+C to stop)...",
        interval
    );
    let mut restart_attempts: HashMap<String, u32> = HashMap::new();
    let mut restart_after: HashMap<String, std::time::Instant> = HashMap::new();
    loop {
        for name in stack.services.keys().cloned().collect::<Vec<_>>() {
            let Some(svc) = stack.services.get(&name) else {
                continue;
            };
            match effective_backend("", &svc.backend) {
                "hyperv" => {
                    let cname = stackdeck_hyperv::container_name(&stack.project, &name);
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
                        if !restart_is_due(&name, svc, &mut restart_attempts, &mut restart_after) {
                            continue;
                        }
                        println!("{}: {} under Hyper-V, restarting...", name, status);
                        let hyperv_svc = stack_service_to_hyperv(&stack, svc);
                        let _ = hyperv_mgr.stop_service(&hyperv_svc);
                        if let Err(err) = hyperv_mgr.start_service(&hyperv_svc, false) {
                            eprintln!("{}: restart failed: {}", name, err);
                        }
                    } else if !bad {
                        restart_attempts.remove(&name);
                        restart_after.remove(&name);
                    }
                }
                "native" | "" => {
                    if !native_mgr.health_ok(&name).unwrap_or(false) && svc.restart != "no" {
                        if !restart_is_due(&name, svc, &mut restart_attempts, &mut restart_after) {
                            continue;
                        }
                        println!("{}: unhealthy, restarting...", name);
                        if let Err(err) = native_mgr.start(&name, true) {
                            eprintln!("{}: restart failed: {}", name, err);
                        }
                    } else {
                        restart_attempts.remove(&name);
                        restart_after.remove(&name);
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
    allow_invalid: bool,
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
    let project_root = PathBuf::from(path)
        .canonicalize()
        .map_err(|error| anyhow::anyhow!("project path is not accessible: {error}"))?;
    if !allow_invalid {
        let config_path = {
            let value = PathBuf::from(config);
            if value.is_absolute() {
                value
            } else {
                project_root.join(value)
            }
        };
        let stack = load_stack_with_backend(&config_path.to_string_lossy(), backend.unwrap_or(""))?;
        validate_stack_for_run(&stack, backend.unwrap_or(""))?;
    }
    let reg_path = stackdeck_types::registry_file();
    let _lock = stackdeck_types::FileLock::exclusive(&reg_path)?;
    let mut reg: HashMap<String, serde_json::Value> = if reg_path.exists() {
        let text = std::fs::read_to_string(&reg_path)?;
        serde_json::from_str(&text).map_err(|error| {
            anyhow::anyhow!(
                "refusing to replace corrupt project registry {}: {}",
                reg_path.display(),
                error
            )
        })?
    } else {
        HashMap::new()
    };
    reg.insert(
        project_name.clone(),
        serde_json::json!({
            "root": project_root.to_string_lossy(),
            "config": config,
            "backend": backend.unwrap_or("native"),
        }),
    );
    if let Some(parent) = reg_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    stackdeck_types::atomic_write(&reg_path, serde_json::to_string_pretty(&reg)?.as_bytes())?;
    println!("Registered {}", project_name);
    Ok(())
}

fn cmd_unregister(name: &str) -> Result<()> {
    let reg_path = stackdeck_types::registry_file();
    let _lock = stackdeck_types::FileLock::exclusive(&reg_path)?;
    let mut reg: HashMap<String, serde_json::Value> = if reg_path.exists() {
        let text = std::fs::read_to_string(&reg_path)?;
        serde_json::from_str(&text).map_err(|error| {
            anyhow::anyhow!(
                "refusing to replace corrupt project registry {}: {}",
                reg_path.display(),
                error
            )
        })?
    } else {
        HashMap::new()
    };
    if reg.remove(name).is_some() {
        stackdeck_types::atomic_write(&reg_path, serde_json::to_string_pretty(&reg)?.as_bytes())?;
        println!("Unregistered {}", name);
    } else {
        println!("No registered project named {}", name);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Compose commands
// ---------------------------------------------------------------------------

fn cmd_compose_up(
    file: &str,
    backend: &str,
    profiles: &[String],
    detach: bool,
    build: bool,
    interval: u32,
    services: &[String],
) -> Result<()> {
    let stack = load_compose_stack(file, backend, profiles)?;
    println!("Project: {} (backend={})", stack.project, backend);
    let targets = if services.is_empty() {
        stack.services.keys().cloned().collect::<Vec<_>>()
    } else {
        services.to_vec()
    };
    run_stack_up(stack, backend, &targets, false, build, !detach, interval)
}

fn cmd_compose_build(
    file: &str,
    backend: &str,
    profiles: &[String],
    services: &[String],
) -> Result<()> {
    let stack = load_compose_stack(file, backend, profiles)?;
    println!("Building project: {}", stack.project);
    let targets = if services.is_empty() {
        stack.services.keys().cloned().collect::<Vec<_>>()
    } else {
        services.to_vec()
    };

    let hyperv_mgr =
        stackdeck_hyperv::HyperVManager::new(stackdeck_hyperv::HyperVManager::load_config()?);
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

fn cmd_compose_down(
    file: &str,
    backend: &str,
    profiles: &[String],
    volumes: bool,
    services: &[String],
) -> Result<()> {
    let stack = load_compose_stack(file, backend, profiles)?;
    println!("Stopping project: {}", stack.project);
    let targets = if services.is_empty() {
        stack.services.keys().cloned().collect::<Vec<_>>()
    } else {
        services.to_vec()
    };
    run_stack_down(stack, backend, &targets, volumes)
}

fn cmd_compose_status(
    file: &str,
    backend: &str,
    profiles: &[String],
    json_output: bool,
) -> Result<()> {
    let stack = load_compose_stack(file, backend, profiles)?;
    run_stack_status(stack, backend, json_output)
}

fn cmd_compose_logs(
    file: &str,
    backend: &str,
    profiles: &[String],
    service: &str,
    tail: u32,
) -> Result<()> {
    let stack = load_compose_stack(file, backend, profiles)?;
    if !stack.services.contains_key(service) {
        anyhow::bail!("Service '{}' not found in compose file", service);
    }
    run_stack_logs(stack, backend, service, tail)
}

// ---------------------------------------------------------------------------
// Hyper-V commands
// ---------------------------------------------------------------------------

fn run_hyperv_command(cmd: HypervCommands) -> Result<()> {
    let cfg = stackdeck_hyperv::HyperVManager::load_config()?;
    let mgr = stackdeck_hyperv::HyperVManager::new(cfg);

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
            // Health is a release/automation gate, not informational output:
            // return non-zero unless the VM, SSH, containerd, nerdctl and
            // BuildKit are all genuinely usable.
            mgr.assert_runtime_ready()?;
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
            stackdeck_hyperv::HyperVManager::save_config(&cfg)?;
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
            sha512,
            timeout,
        } => {
            println!("Initializing Hyper-V runtime...");
            let use_release_lock = image_vhdx.trim().is_empty()
                && url.is_none()
                && sha256.as_deref().is_none_or(str::is_empty)
                && sha512.as_deref().is_none_or(str::is_empty);
            if use_release_lock {
                let release_lock = stackdeck_hyperv::release_runtime_lock()?;
                mgr.init_vm(
                    &image_vhdx,
                    Some(&release_lock.image_url),
                    None,
                    Some(&release_lock.image_sha512),
                    timeout,
                )?;
            } else {
                mgr.init_vm(
                    &image_vhdx,
                    url.as_deref(),
                    sha256.as_deref(),
                    sha512.as_deref(),
                    timeout,
                )?;
            }
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
            sha512,
            force,
        } => {
            println!("Downloading image...");
            if let Some(url) = url {
                mgr.download_image(
                    &url,
                    &output,
                    if sha256.is_empty() {
                        None
                    } else {
                        Some(&sha256)
                    },
                    if sha512.is_empty() {
                        None
                    } else {
                        Some(&sha512)
                    },
                    force,
                )?;
            } else {
                let release_lock = stackdeck_hyperv::release_runtime_lock()?;
                let effective_sha512 = if sha256.is_empty() && sha512.is_empty() {
                    release_lock.image_sha512.as_str()
                } else {
                    sha512.as_str()
                };
                mgr.download_image(
                    &release_lock.image_url,
                    &output,
                    if sha256.is_empty() {
                        None
                    } else {
                        Some(&sha256)
                    },
                    if effective_sha512.is_empty() {
                        None
                    } else {
                        Some(effective_sha512)
                    },
                    force,
                )?;
            }
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
                cfg.ssh_identity = stackdeck_types::registry_dir()
                    .join("id_ed25519")
                    .to_string_lossy()
                    .to_string();
            }
            stackdeck_hyperv::HyperVManager::save_config(&cfg)?;
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
            smb_password_stdin,
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
            } else if smb_password_stdin {
                use std::io::BufRead;
                let mut secret = String::new();
                std::io::stdin().lock().read_line(&mut secret)?;
                while matches!(secret.chars().last(), Some('\r' | '\n')) {
                    secret.pop();
                }
                if secret.is_empty() {
                    anyhow::bail!("SMB password from stdin must not be empty");
                }
                cfg.smb_password = secret;
            }
            stackdeck_hyperv::HyperVManager::save_config(&cfg)?;
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
            VolumeCommands::Create { name, project } => {
                let effective_name = match project {
                    Some(project) => {
                        format!("{}_{}", stackdeck_hyperv::project_slug(&project), name)
                    }
                    None => name,
                };
                let output = mgr.volume_create(&effective_name)?;
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
            NetworkCommands::Create {
                name,
                project,
                driver,
            } => {
                if driver != "bridge" {
                    anyhow::bail!(
                        "network driver '{}' is unsupported; refusing to ignore --driver",
                        driver
                    );
                }
                let effective_name = match project {
                    Some(project) => {
                        format!("{}_{}", stackdeck_hyperv::project_slug(&project), name)
                    }
                    None => name,
                };
                let output = mgr.network_create(&effective_name)?;
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
    stackdeck_api::serve(host, port, allow_remote).await?;
    Ok(())
}

fn preflight_check() -> Result<()> {
    let output = std::process::Command::new("where.exe").arg("tar").output();
    if output.is_err() || !output.unwrap().status.success() {
        eprintln!("WARNING: tar.exe not found in PATH. Some features might not work properly.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_profiles_are_explicit_not_service_selection() {
        let file = write_compose(
            "profiles",
            r#"
name: demo
services:
  web:
    image: nginx
  debug:
    image: busybox
    profiles:
      - debug
"#,
        );

        let selected_services = vec!["debug".to_string()];
        let stack = load_compose_stack(&file, "hyperv", &[]).unwrap();

        assert!(stack.services.contains_key("web"));
        assert!(!stack.services.contains_key("debug"));
        assert_eq!(selected_services, vec!["debug".to_string()]);

        let active_profiles = vec!["debug".to_string()];
        let stack = load_compose_stack(&file, "hyperv", &active_profiles).unwrap();

        assert!(stack.services.contains_key("debug"));
    }

    #[test]
    fn compose_volume_mounts_and_down_v_use_resolved_resource_names() {
        let file = write_compose(
            "volumes",
            r#"
name: demo
services:
  web:
    image: nginx
    volumes:
      - data:/data
      - external_data:/external:ro
      - ./src:/src
volumes:
  data:
    name: fixed_data
  external_data:
    external:
      name: already_there
"#,
        );

        let stack = load_compose_stack(&file, "hyperv", &[]).unwrap();
        let web = stack.services.get("web").unwrap();

        assert!(web.volumes.contains(&"fixed_data:/data".to_string()));
        assert!(web
            .volumes
            .contains(&"already_there:/external:ro".to_string()));
        assert!(web.volumes.contains(&"./src:/src".to_string()));
        assert_eq!(removable_hyperv_volume_names(&stack), vec!["fixed_data"]);
    }

    #[test]
    fn compose_preserves_anonymous_volumes_and_does_not_publish_target_only_ports() {
        let file = write_compose(
            "anonymous",
            r#"
name: demo
services:
  web:
    image: nginx
    volumes:
      - /var/cache/nginx
    ports:
      - "80"
"#,
        );
        let stack = load_compose_stack(&file, "hyperv", &[]).unwrap();
        let web = stack.services.get("web").unwrap();
        assert_eq!(web.volumes, vec!["/var/cache/nginx"]);
        assert!(web.ports.is_empty());
    }

    #[test]
    fn compose_replicas_expand_without_sharing_fixed_ports() {
        let file = write_compose(
            "replicas",
            r#"
name: demo
services:
  worker:
    image: busybox
    deploy:
      replicas: 3
"#,
        );
        let stack = load_compose_stack(&file, "hyperv", &[]).unwrap();
        assert!(stack.services.contains_key("worker"));
        assert!(stack.services.contains_key("worker-2"));
        assert!(stack.services.contains_key("worker-3"));
    }

    fn write_compose(name: &str, contents: &str) -> String {
        let dir = std::env::temp_dir().join(format!(
            "stackdeck-cli-test-{}-{}",
            name,
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("compose.yml");
        std::fs::write(&path, contents).unwrap();
        path.to_string_lossy().to_string()
    }
}
