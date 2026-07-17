//! Docker Compose YAML parser for PyStack Runner.
//!
//! Replaces `compose_support.py` — parses and normalizes Compose YAML files
//! into the typed project model defined in `pystack-types`.

pub use pystack_types::{
    ComposeBuild, ComposeDependsOn, ComposeHealthcheck, ComposeNetworkAttachment, ComposePort,
    ComposeProject, ComposeResource, ComposeRestartPolicy, ComposeService, ComposeVolumeMount,
};

use regex::Regex;
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ComposeError {
    #[error("YAML parse error: {0}")]
    Yaml(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("variable interpolation error: {0}")]
    Interpolation(String),
    #[error("invalid compose file: {0}")]
    Validation(String),
}

// ---------------------------------------------------------------------------
// Top-level API
// ---------------------------------------------------------------------------

/// Load a Compose file and return a normalized project model.
pub fn load_compose_file(
    path: &Path,
    project_name: Option<&str>,
    profiles: &[String],
    env: Option<&HashMap<String, String>>,
    env_file: Option<&Path>,
) -> Result<ComposeProject, ComposeError> {
    let working_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let text = std::fs::read_to_string(path)?;
    let interpolation_env = build_interpolation_env(&working_dir, env_file, env);
    let interpolated = interpolate_compose_text(&text, &interpolation_env)?;
    let raw = load_yaml(&interpolated)?;
    normalize_compose_project(&raw, project_name, profiles, Some(&working_dir))
}

/// Parse Compose YAML text directly.
pub fn parse_compose(
    text: &str,
    project_name: Option<&str>,
    profiles: &[String],
    env: Option<&HashMap<String, String>>,
    working_dir: Option<&Path>,
) -> Result<ComposeProject, ComposeError> {
    let base_dir = working_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let interpolated = interpolate_compose_text(text, env.unwrap_or(&HashMap::new()))?;
    let raw = load_yaml(&interpolated)?;
    normalize_compose_project(&raw, project_name, profiles, Some(&base_dir))
}

// ---------------------------------------------------------------------------
// YAML loading
// ---------------------------------------------------------------------------

/// Load YAML text into a raw mapping.
pub fn load_yaml(text: &str) -> Result<serde_yaml::Value, ComposeError> {
    let data: serde_yaml::Value =
        serde_yaml::from_str(text).map_err(|e| ComposeError::Yaml(e.to_string()))?;
    match data {
        serde_yaml::Value::Mapping(_) => Ok(data),
        serde_yaml::Value::Null => Ok(serde_yaml::Value::Mapping(serde_yaml::Mapping::new())),
        _ => Err(ComposeError::Validation(
            "Compose document must be a mapping".into(),
        )),
    }
}

// ---------------------------------------------------------------------------
// .env file parsing
// ---------------------------------------------------------------------------

/// Parse a Compose-style .env file.
pub fn parse_env_file(path: &Path) -> Result<HashMap<String, String>, ComposeError> {
    let mut result = HashMap::new();
    if !path.exists() {
        return Err(ComposeError::Validation(format!(
            "env_file not found: {}",
            path.display()
        )));
    }
    let text = std::fs::read_to_string(path)?;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = if line.starts_with("export ") {
            line[7..].trim_start()
        } else {
            line
        };
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            if !key.is_empty() {
                result.insert(key.to_string(), parse_env_value(value.trim()));
            }
        }
    }
    Ok(result)
}

fn parse_env_value(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && bytes[0] == bytes[bytes.len() - 1]
        && (bytes[0] == b'\'' || bytes[0] == b'"')
    {
        return value[1..value.len() - 1].to_string();
    }
    if let Some(idx) = value.find(" #") {
        return value[..idx].trim_end().to_string();
    }
    value.to_string()
}

// ---------------------------------------------------------------------------
// Variable interpolation
// ---------------------------------------------------------------------------

/// Apply Compose variable interpolation to raw YAML text.
pub fn interpolate_compose_text(
    text: &str,
    env_vars: &HashMap<String, String>,
) -> Result<String, ComposeError> {
    let sentinel = "\0COMPOSE_DOLLAR\0";
    let protected = text.replace("$$", sentinel);

    let re =
        Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(?:(:?[-?])(.*?))?\}|\$([A-Za-z_][A-Za-z0-9_]*)")
            .expect("invalid interpolation regex");

    let mut result = protected;
    let mut error: Option<ComposeError> = None;

    result = re
        .replace_all(&result, |caps: &regex::Captures| -> String {
            let var_name = caps.get(1).or_else(|| caps.get(4)).unwrap().as_str();
            let operator = caps.get(2).map(|m| m.as_str());
            let fallback = caps.get(3).map(|m| m.as_str()).unwrap_or("");

            let is_set = env_vars.contains_key(var_name);
            let value = env_vars.get(var_name).map(|s| s.as_str()).unwrap_or("");
            let is_non_empty = is_set && !value.is_empty();

            match operator {
                None => {
                    if is_set {
                        value.to_string()
                    } else {
                        String::new()
                    }
                }
                Some(":-") => {
                    if is_non_empty {
                        value.to_string()
                    } else {
                        fallback.to_string()
                    }
                }
                Some("-") => {
                    if is_set {
                        value.to_string()
                    } else {
                        fallback.to_string()
                    }
                }
                Some(":?") => {
                    if is_non_empty {
                        value.to_string()
                    } else {
                        let msg = if fallback.is_empty() {
                            format!("{} is required", var_name)
                        } else {
                            fallback.to_string()
                        };
                        error = Some(ComposeError::Interpolation(msg));
                        caps.get(0).unwrap().as_str().to_string()
                    }
                }
                Some("?") => {
                    if is_set {
                        value.to_string()
                    } else {
                        let msg = if fallback.is_empty() {
                            format!("{} is required", var_name)
                        } else {
                            fallback.to_string()
                        };
                        error = Some(ComposeError::Interpolation(msg));
                        caps.get(0).unwrap().as_str().to_string()
                    }
                }
                _ => caps.get(0).unwrap().as_str().to_string(),
            }
        })
        .into_owned();

    if let Some(err) = error {
        return Err(err);
    }

    Ok(result.replace(sentinel, "$"))
}

// ---------------------------------------------------------------------------
// Project normalization
// ---------------------------------------------------------------------------

/// Normalize a loaded Compose mapping into typed structs.
pub fn normalize_compose_project(
    raw: &serde_yaml::Value,
    project_name: Option<&str>,
    profiles: &[String],
    working_dir: Option<&Path>,
) -> Result<ComposeProject, ComposeError> {
    let base_dir = working_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let raw_mapping = as_mapping(raw);
    let dir_name = base_dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let raw_name = raw_mapping
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(&dir_name);
    let name = normalize_project_name(project_name.unwrap_or(raw_name))?;

    let active_profiles: std::collections::HashSet<&str> =
        profiles.iter().map(|s| s.as_str()).collect();

    let raw_services = as_mapping_opt(raw_mapping.get("services"), "services")?;
    let networks = normalize_resources(raw_mapping.get("networks"), &name, "network")?;
    let volumes = normalize_resources(raw_mapping.get("volumes"), &name, "volume")?;
    let secrets = normalize_resources(raw_mapping.get("secrets"), &name, "secret")?;
    let configs = normalize_resources(raw_mapping.get("configs"), &name, "config")?;

    let mut net = networks.clone();
    if !net.contains_key("default") {
        net.insert(
            "default".to_string(),
            ComposeResource {
                key: "default".to_string(),
                name: format!("{}_default", name),
                labels: compose_labels(&name),
                ..Default::default()
            },
        );
    }

    let mut services = HashMap::new();
    if let Some(svc_map) = raw_services {
        for (service_name, raw_service) in svc_map {
            let raw_service = match raw_service {
                serde_yaml::Value::Mapping(ref m) => m,
                _ => {
                    return Err(ComposeError::Validation(format!(
                        "services.{:?} must be a mapping",
                        service_name
                    )))
                }
            };
            let service_profiles: Vec<String> =
                string_list(raw_service.get(&serde_yaml::Value::String("profiles".into())));
            if !service_profiles.is_empty()
                && !service_profiles
                    .iter()
                    .any(|p| active_profiles.contains(p.as_str()))
            {
                continue;
            }
            let sname = service_name.as_str().unwrap_or_default();
            let service = normalize_service(sname, raw_service, &name, &net)?;
            services.insert(service.name.clone(), service);
        }
    }

    // Collect x- extensions
    let x_extensions: HashMap<String, serde_json::Value> = raw_mapping
        .iter()
        .filter_map(|(k, v)| {
            let key = k.as_str()?;
            if key.starts_with("x-") {
                Some((key.to_string(), yaml_value_to_json(v)))
            } else {
                None
            }
        })
        .collect();

    Ok(ComposeProject {
        name,
        working_dir: base_dir,
        services,
        networks: net,
        volumes,
        secrets,
        configs,
        x_extensions,
    })
}

/// Normalize a project name following Compose conventions.
pub fn normalize_project_name(name: &str) -> Result<String, ComposeError> {
    let re = Regex::new(r"[^a-z0-9_-]+").unwrap();
    let lower = name.to_lowercase().replace('.', "-");
    let normalized = re.replace_all(&lower, "").to_string();
    let normalized = normalized
        .trim_matches(|c| c == '_' || c == '-')
        .to_string();

    if normalized.is_empty() {
        return Err(ComposeError::Validation(
            "project name must contain letters or digits".into(),
        ));
    }

    let mut result = normalized;
    if !result
        .chars()
        .next()
        .map(|c| c.is_alphanumeric())
        .unwrap_or(false)
    {
        result = format!("p{}", result);
    }
    Ok(result)
}

/// Return the concrete Compose resource name for networks, volumes, secrets, configs.
pub fn resource_name(project_name: &str, key: &str, raw: &serde_yaml::Mapping) -> String {
    if let Some(external) = raw.get(&serde_yaml::Value::String("external".into())) {
        if let serde_yaml::Value::Mapping(ext_map) = external {
            if let Some(name) = ext_map.get(&serde_yaml::Value::String("name".into())) {
                if let Some(s) = name.as_str() {
                    return s.to_string();
                }
            }
        }
    }
    if let Some(name) = raw.get(&serde_yaml::Value::String("name".into())) {
        if let Some(s) = name.as_str() {
            return s.to_string();
        }
    }
    if let Some(external) = raw.get(&serde_yaml::Value::String("external".into())) {
        if external.as_bool() == Some(true) {
            return key.to_string();
        }
    }
    format!("{}_{}", project_name, key)
}

// ---------------------------------------------------------------------------
// Service normalization
// ---------------------------------------------------------------------------

fn normalize_service(
    name: &str,
    raw: &serde_yaml::Mapping,
    _project_name: &str,
    known_networks: &HashMap<String, ComposeResource>,
) -> Result<ComposeService, ComposeError> {
    let known_keys: std::collections::HashSet<&str> = [
        "image",
        "build",
        "command",
        "entrypoint",
        "environment",
        "env_file",
        "profiles",
        "depends_on",
        "ports",
        "volumes",
        "networks",
        "secrets",
        "configs",
        "healthcheck",
        "restart",
        "deploy",
        "scale",
        "container_name",
        "labels",
    ]
    .into_iter()
    .collect();

    let extra: HashMap<String, serde_json::Value> = raw
        .iter()
        .filter_map(|(k, v)| {
            let key = k.as_str()?;
            if !known_keys.contains(key) {
                Some((key.to_string(), yaml_value_to_json(v)))
            } else {
                None
            }
        })
        .collect();

    Ok(ComposeService {
        name: name.to_string(),
        image: optional_string(raw.get("image")),
        build: normalize_build(raw.get("build"))?,
        command: raw.get("command").map(yaml_value_to_json),
        entrypoint: raw.get("entrypoint").map(yaml_value_to_json),
        environment: normalize_environment(raw.get("environment")),
        env_file: string_list(raw.get("env_file")),
        profiles: string_list(raw.get("profiles")),
        depends_on: normalize_depends_on(raw.get("depends_on"))?,
        ports: normalize_ports(raw.get("ports"))?,
        volumes: normalize_volume_mounts(raw.get("volumes"))?,
        networks: normalize_network_attachments(raw.get("networks"), known_networks)?,
        secrets: normalize_service_resources(raw.get("secrets"))?,
        configs: normalize_service_resources(raw.get("configs"))?,
        healthcheck: normalize_healthcheck(raw.get("healthcheck"))?,
        restart: normalize_restart(raw.get("restart"), raw.get("deploy")),
        replicas: normalize_replicas(raw),
        container_name: optional_string(raw.get("container_name")),
        labels: normalize_labels(raw.get("labels")),
        extra,
    })
}

fn normalize_build(raw: Option<&serde_yaml::Value>) -> Result<Option<ComposeBuild>, ComposeError> {
    let raw = match raw {
        None => return Ok(None),
        Some(r) => r,
    };

    match raw {
        serde_yaml::Value::String(s) => Ok(Some(ComposeBuild {
            context: Some(s.clone()),
            ..Default::default()
        })),
        serde_yaml::Value::Mapping(m) => {
            let known: std::collections::HashSet<&str> = [
                "context",
                "dockerfile",
                "target",
                "args",
                "cache_from",
                "cache_to",
            ]
            .into_iter()
            .collect();
            let extra: HashMap<String, serde_json::Value> = m
                .iter()
                .filter_map(|(k, v)| {
                    let key = k.as_str()?;
                    if !known.contains(key) {
                        Some((key.to_string(), yaml_value_to_json(v)))
                    } else {
                        None
                    }
                })
                .collect();

            Ok(Some(ComposeBuild {
                context: optional_string(m.get("context")),
                dockerfile: optional_string(m.get("dockerfile")),
                target: optional_string(m.get("target")),
                args: normalize_build_args(m.get("args")),
                cache_from: string_list(m.get("cache_from")),
                cache_to: string_list(m.get("cache_to")),
                extra,
            }))
        }
        _ => Err(ComposeError::Validation(
            "build must be a string or mapping".into(),
        )),
    }
}

fn normalize_build_args(raw: Option<&serde_yaml::Value>) -> HashMap<String, Option<String>> {
    let raw = match raw {
        None => return HashMap::new(),
        Some(r) => r,
    };
    match raw {
        serde_yaml::Value::Mapping(m) => m
            .iter()
            .map(|(k, v)| {
                let key = k.as_str().unwrap_or_default().to_string();
                let val = v.as_str().map(|s| s.to_string());
                (key, val)
            })
            .collect(),
        serde_yaml::Value::Sequence(seq) => seq
            .iter()
            .map(|item| {
                let text = item.as_str().unwrap_or_default();
                if let Some((k, v)) = text.split_once('=') {
                    (k.to_string(), Some(v.to_string()))
                } else {
                    (text.to_string(), None)
                }
            })
            .collect(),
        _ => HashMap::new(),
    }
}

fn normalize_depends_on(
    raw: Option<&serde_yaml::Value>,
) -> Result<Vec<ComposeDependsOn>, ComposeError> {
    let raw = match raw {
        None => return Ok(Vec::new()),
        Some(r) => r,
    };
    match raw {
        serde_yaml::Value::Sequence(seq) => Ok(seq
            .iter()
            .map(|item| ComposeDependsOn {
                service: item.as_str().unwrap_or_default().to_string(),
                condition: "service_started".to_string(),
                restart: false,
                required: true,
            })
            .collect()),
        serde_yaml::Value::Mapping(m) => {
            let mut result = Vec::new();
            for (k, v) in m {
                let service = k.as_str().unwrap_or_default().to_string();
                match v {
                    serde_yaml::Value::String(s) => {
                        result.push(ComposeDependsOn {
                            service,
                            condition: s.clone(),
                            restart: false,
                            required: true,
                        });
                    }
                    serde_yaml::Value::Mapping(cfg) => {
                        result.push(ComposeDependsOn {
                            service,
                            condition: cfg
                                .get(&serde_yaml::Value::String("condition".into()))
                                .and_then(|v| v.as_str())
                                .unwrap_or("service_started")
                                .to_string(),
                            restart: cfg
                                .get(&serde_yaml::Value::String("restart".into()))
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false),
                            required: cfg
                                .get(&serde_yaml::Value::String("required".into()))
                                .and_then(|v| v.as_bool())
                                .unwrap_or(true),
                        });
                    }
                    _ => {
                        result.push(ComposeDependsOn {
                            service,
                            condition: "service_started".to_string(),
                            restart: false,
                            required: true,
                        });
                    }
                }
            }
            Ok(result)
        }
        _ => Err(ComposeError::Validation(
            "depends_on must be a list or mapping".into(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------

fn normalize_ports(raw: Option<&serde_yaml::Value>) -> Result<Vec<ComposePort>, ComposeError> {
    let raw = match raw {
        None => return Ok(Vec::new()),
        Some(r) => r,
    };
    let items = as_list(raw);
    let mut result = Vec::new();
    for item in items {
        match item {
            serde_yaml::Value::Mapping(m) => {
                let target = m
                    .get("target")
                    .and_then(|v| v.as_i64())
                    .ok_or_else(|| ComposeError::Validation("port target is required".into()))
                    .and_then(|n| checked_u16(n, "port target"))?;
                let published = m
                    .get("published")
                    .and_then(|v| v.as_i64())
                    .map(|n| checked_u16(n, "published port"))
                    .transpose()?;
                result.push(ComposePort {
                    target,
                    published,
                    host_ip: optional_string(m.get("host_ip")),
                    protocol: m
                        .get("protocol")
                        .and_then(|v| v.as_str())
                        .unwrap_or("tcp")
                        .to_string(),
                    mode: optional_string(m.get("mode")),
                    name: optional_string(m.get("name")),
                    app_protocol: optional_string(m.get("app_protocol")),
                });
            }
            other => {
                let s = other.as_str().unwrap_or_default();
                result.extend(parse_port_string(s)?);
            }
        }
    }
    Ok(result)
}

fn parse_port_range(value: &str) -> Option<(u16, u16)> {
    if value.is_empty() {
        return None;
    }
    if let Some((start, end)) = value.split_once('-') {
        let start = start.parse::<u16>().ok()?;
        let end = end.parse::<u16>().ok()?;
        if start <= end {
            Some((start, end))
        } else {
            None
        }
    } else {
        let port = value.parse::<u16>().ok()?;
        Some((port, port))
    }
}

fn parse_port_string(value: &str) -> Result<Vec<ComposePort>, ComposeError> {
    let (value, protocol) = if let Some(idx) = value.rfind('/') {
        (&value[..idx], &value[idx + 1..])
    } else {
        (value, "tcp")
    };

    let parts: Vec<&str> = value.split(':').collect();
    let (host_ip, pub_range, target_text) = match parts.len() {
        1 => (None, None, parts[0]),
        2 => (None, parse_port_range(parts[0]), parts[1]),
        _ => {
            let ip = parts[..parts.len() - 2].join(":");
            let pub_range = parse_port_range(parts[parts.len() - 2]);
            let target = parts[parts.len() - 1];
            (
                if ip.is_empty() { None } else { Some(ip) },
                pub_range,
                target,
            )
        }
    };

    let target_range = parse_port_range(target_text)
        .ok_or_else(|| ComposeError::Validation(format!("invalid target port: {}", value)))?;

    let mut ports = Vec::new();
    let (t_start, t_end) = target_range;

    if let Some((p_start, p_end)) = pub_range {
        if (t_end - t_start) != (p_end - p_start) && t_start != t_end && p_start != p_end {
            return Err(ComposeError::Validation(format!(
                "port ranges must be of equal length: {}",
                value
            )));
        }

        let mut p = p_start;
        for t in t_start..=t_end {
            ports.push(ComposePort {
                target: t,
                published: Some(p),
                host_ip: host_ip.clone(),
                protocol: protocol.to_string(),
                mode: None,
                name: None,
                app_protocol: None,
            });
            if p < p_end {
                p += 1;
            }
        }
    } else {
        for t in t_start..=t_end {
            ports.push(ComposePort {
                target: t,
                published: None,
                host_ip: host_ip.clone(),
                protocol: protocol.to_string(),
                mode: None,
                name: None,
                app_protocol: None,
            });
        }
    }

    Ok(ports)
}

// ---------------------------------------------------------------------------
// Volumes
// ---------------------------------------------------------------------------

fn normalize_volume_mounts(
    raw: Option<&serde_yaml::Value>,
) -> Result<Vec<ComposeVolumeMount>, ComposeError> {
    let raw = match raw {
        None => return Ok(Vec::new()),
        Some(r) => r,
    };
    let items = as_list(raw);
    let mut result = Vec::new();
    for item in items {
        match item {
            serde_yaml::Value::Mapping(m) => {
                let mount_type = m
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("volume")
                    .to_string();
                let read_only = m
                    .get("read_only")
                    .or_else(|| m.get("readonly"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                result.push(ComposeVolumeMount {
                    r#type: mount_type,
                    source: optional_string(m.get("source")),
                    target: m
                        .get("target")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    read_only,
                    consistency: optional_string(m.get("consistency")),
                    bind: yaml_mapping_to_json_map(m.get("bind")),
                    volume: yaml_mapping_to_json_map(m.get("volume")),
                    tmpfs: yaml_mapping_to_json_map(m.get("tmpfs")),
                });
            }
            other => {
                let s = other.as_str().unwrap_or_default();
                result.push(parse_volume_string(s));
            }
        }
    }
    Ok(result)
}

fn parse_volume_string(value: &str) -> ComposeVolumeMount {
    let parts = split_volume_spec(value);
    let (source, target, mode) = match parts.len() {
        1 => (None, parts[0].as_str(), ""),
        2 => (Some(parts[0].as_str()), parts[1].as_str(), ""),
        _ => (
            Some(parts[0].as_str()),
            parts[1].as_str(),
            parts[2].as_str(),
        ),
    };

    let read_only = mode.split(',').any(|token| token.trim() == "ro");
    let mount_type = if source.map_or(false, |s| looks_like_path(s)) {
        "bind"
    } else {
        "volume"
    };

    ComposeVolumeMount {
        r#type: mount_type.to_string(),
        source: source.map(|s| s.to_string()),
        target: target.to_string(),
        read_only,
        consistency: if mode.is_empty() {
            None
        } else {
            Some(mode.to_string())
        },
        ..Default::default()
    }
}

fn split_volume_spec(value: &str) -> Vec<String> {
    let re = Regex::new(r"^[A-Za-z]:[\\/]").unwrap();
    if re.is_match(value) {
        let drive = &value[..2];
        let rest = &value[2..];
        let mut pieces: Vec<String> = rest.split(':').map(|s| s.to_string()).collect();
        if !pieces.is_empty() {
            pieces[0] = format!("{}{}", drive, pieces[0]);
        }
        pieces
    } else {
        value.split(':').map(|s| s.to_string()).collect()
    }
}

fn looks_like_path(value: &str) -> bool {
    let re = Regex::new(r"^[A-Za-z]:[\\/]").unwrap();
    value.starts_with('.')
        || value.starts_with('/')
        || value.starts_with('~')
        || value.contains('\\')
        || re.is_match(value)
}

// ---------------------------------------------------------------------------
// Networks
// ---------------------------------------------------------------------------

fn normalize_network_attachments(
    raw: Option<&serde_yaml::Value>,
    known_networks: &HashMap<String, ComposeResource>,
) -> Result<Vec<ComposeNetworkAttachment>, ComposeError> {
    let default_net = known_networks
        .get("default")
        .cloned()
        .unwrap_or_else(|| ComposeResource {
            key: "default".to_string(),
            name: "default".to_string(),
            ..Default::default()
        });

    let raw = match raw {
        None => {
            return Ok(vec![ComposeNetworkAttachment {
                name: default_net.name,
                ..Default::default()
            }])
        }
        Some(r) => r,
    };

    match raw {
        serde_yaml::Value::Sequence(seq) => Ok(seq
            .iter()
            .map(|item| {
                let key = item.as_str().unwrap_or_default();
                let net = known_networks
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| ComposeResource {
                        key: key.to_string(),
                        name: key.to_string(),
                        ..Default::default()
                    });
                ComposeNetworkAttachment {
                    name: net.name,
                    ..Default::default()
                }
            })
            .collect()),
        serde_yaml::Value::Mapping(m) => {
            let mut result = Vec::new();
            for (k, v) in m {
                let key = k.as_str().unwrap_or_default();
                let net = known_networks
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| ComposeResource {
                        key: key.to_string(),
                        name: key.to_string(),
                        ..Default::default()
                    });
                let config: &serde_yaml::Mapping = match v {
                    serde_yaml::Value::Mapping(ref cfg) => cfg,
                    _ => &serde_yaml::Mapping::new(),
                };
                result.push(ComposeNetworkAttachment {
                    name: net.name,
                    aliases: string_list(config.get("aliases")),
                    ipv4_address: optional_string(config.get("ipv4_address")),
                    ipv6_address: optional_string(config.get("ipv6_address")),
                    priority: optional_int(config.get("priority")),
                });
            }
            Ok(result)
        }
        _ => Err(ComposeError::Validation(
            "networks must be a list or mapping".into(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Healthcheck
// ---------------------------------------------------------------------------

fn normalize_healthcheck(
    raw: Option<&serde_yaml::Value>,
) -> Result<Option<ComposeHealthcheck>, ComposeError> {
    let raw = match raw {
        None => return Ok(None),
        Some(r) => r,
    };
    match raw {
        serde_yaml::Value::Mapping(m) => Ok(Some(ComposeHealthcheck {
            test: m.get("test").map(yaml_value_to_json),
            interval: normalize_duration(m.get("interval")),
            timeout: normalize_duration(m.get("timeout")),
            retries: optional_int(m.get("retries")),
            start_period: normalize_duration(m.get("start_period")),
            disable: m.get("disable").and_then(|v| v.as_bool()).unwrap_or(false),
        })),
        _ => Err(ComposeError::Validation(
            "healthcheck must be a mapping".into(),
        )),
    }
}

fn normalize_duration(raw: Option<&serde_yaml::Value>) -> Option<String> {
    let raw = match raw {
        None => return None,
        Some(r) => r,
    };
    match raw {
        serde_yaml::Value::Number(n) => Some(format!("{}s", n)),
        serde_yaml::Value::String(s) => {
            if s.is_empty() {
                None
            } else {
                Some(s.clone())
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Restart policy
// ---------------------------------------------------------------------------

fn normalize_restart(
    raw_restart: Option<&serde_yaml::Value>,
    raw_deploy: Option<&serde_yaml::Value>,
) -> ComposeRestartPolicy {
    if let Some(serde_yaml::Value::Mapping(deploy)) = raw_deploy {
        if let Some(serde_yaml::Value::Mapping(rp)) = deploy.get("restart_policy") {
            let condition = rp
                .get(&serde_yaml::Value::String("condition".into()))
                .and_then(|v| v.as_str())
                .map(|s| s.replace('_', "-"));
            let attempts = optional_int(rp.get(&serde_yaml::Value::String("max_attempts".into())));
            if let Some(cond) = condition {
                return ComposeRestartPolicy {
                    name: cond,
                    maximum_retry_count: attempts,
                };
            }
            if let Some(att) = attempts {
                return ComposeRestartPolicy {
                    name: "no".to_string(),
                    maximum_retry_count: Some(att),
                };
            }
        }
    }

    match raw_restart {
        None | Some(serde_yaml::Value::Null) => ComposeRestartPolicy::default(),
        Some(serde_yaml::Value::Bool(false)) => ComposeRestartPolicy::default(),
        Some(serde_yaml::Value::Bool(true)) => ComposeRestartPolicy {
            name: "always".to_string(),
            ..Default::default()
        },
        Some(serde_yaml::Value::String(s)) => {
            if s == "no" {
                return ComposeRestartPolicy::default();
            }
            if let Some((_, count)) = s.split_once(':') {
                if let Ok(n) = count.parse::<u32>() {
                    return ComposeRestartPolicy {
                        name: "on-failure".to_string(),
                        maximum_retry_count: Some(n),
                    };
                }
            }
            ComposeRestartPolicy {
                name: s.clone(),
                ..Default::default()
            }
        }
        _ => ComposeRestartPolicy::default(),
    }
}

// ---------------------------------------------------------------------------
// Replicas
// ---------------------------------------------------------------------------

fn normalize_replicas(raw: &serde_yaml::Mapping) -> u32 {
    if let Some(scale) = raw.get(&serde_yaml::Value::String("scale".into())) {
        return std::cmp::max(0, scale.as_i64().unwrap_or(1)) as u32;
    }
    if let Some(serde_yaml::Value::Mapping(deploy)) =
        raw.get(&serde_yaml::Value::String("deploy".into()))
    {
        if let Some(replicas) = deploy.get(&serde_yaml::Value::String("replicas".into())) {
            return std::cmp::max(0, replicas.as_i64().unwrap_or(1)) as u32;
        }
    }
    1
}

// ---------------------------------------------------------------------------
// Resources (networks, volumes, secrets, configs)
// ---------------------------------------------------------------------------

fn normalize_resources(
    raw: Option<&serde_yaml::Value>,
    project_name: &str,
    kind: &str,
) -> Result<HashMap<String, ComposeResource>, ComposeError> {
    let raw = match raw {
        None => return Ok(HashMap::new()),
        Some(r) => r,
    };
    let mapping = match raw {
        serde_yaml::Value::Mapping(m) => m,
        serde_yaml::Value::Null => return Ok(HashMap::new()),
        _ => {
            return Err(ComposeError::Validation(format!(
                "{}s must be a mapping",
                kind
            )))
        }
    };
    let mut result = HashMap::new();
    for (k, v) in mapping {
        let config: &serde_yaml::Mapping = match v {
            serde_yaml::Value::Mapping(ref m) => m,
            _ => &serde_yaml::Mapping::new(),
        };
        let key = k.as_str().unwrap_or_default().to_string();
        let external_raw = config
            .get(&serde_yaml::Value::String("external".into()))
            .and_then(|val: &serde_yaml::Value| val.as_bool())
            .unwrap_or(false);
        result.insert(
            key.clone(),
            ComposeResource {
                key: key.clone(),
                name: resource_name(project_name, &key, config),
                external: external_raw,
                driver: optional_string(config.get("driver")),
                labels: normalize_labels(config.get("labels")),
                options: yaml_mapping_to_json_map(
                    config.get("driver_opts").or_else(|| config.get("options")),
                ),
                file: optional_string(config.get("file")),
                environment: optional_string(config.get("environment")),
                content: optional_string(config.get("content")),
            },
        );
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Environment
// ---------------------------------------------------------------------------

fn normalize_environment(raw: Option<&serde_yaml::Value>) -> HashMap<String, Option<String>> {
    let raw = match raw {
        None => return HashMap::new(),
        Some(r) => r,
    };
    match raw {
        serde_yaml::Value::Mapping(m) => m
            .iter()
            .map(|(k, v)| {
                let key = k.as_str().unwrap_or_default().to_string();
                let val = match v {
                    serde_yaml::Value::Null => None,
                    serde_yaml::Value::String(s) => Some(s.clone()),
                    serde_yaml::Value::Number(n) => Some(n.to_string()),
                    serde_yaml::Value::Bool(b) => Some(b.to_string()),
                    _ => v.as_str().map(|s| s.to_string()),
                };
                (key, val)
            })
            .collect(),
        serde_yaml::Value::Sequence(seq) => seq
            .iter()
            .map(|item| {
                let text = item.as_str().unwrap_or_default();
                if let Some((k, v)) = text.split_once('=') {
                    (k.to_string(), Some(v.to_string()))
                } else {
                    let val = env::var(text).ok();
                    (text.to_string(), val)
                }
            })
            .collect(),
        _ => HashMap::new(),
    }
}

// ---------------------------------------------------------------------------
// Labels
// ---------------------------------------------------------------------------

fn normalize_labels(raw: Option<&serde_yaml::Value>) -> HashMap<String, String> {
    let raw = match raw {
        None => return HashMap::new(),
        Some(r) => r,
    };
    match raw {
        serde_yaml::Value::Mapping(m) => m
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().unwrap_or_default().to_string(),
                    v.as_str().unwrap_or_default().to_string(),
                )
            })
            .collect(),
        serde_yaml::Value::Sequence(seq) => seq
            .iter()
            .map(|item| {
                let text = item.as_str().unwrap_or_default();
                if let Some((k, v)) = text.split_once('=') {
                    (k.to_string(), v.to_string())
                } else {
                    (text.to_string(), String::new())
                }
            })
            .collect(),
        _ => HashMap::new(),
    }
}

fn compose_labels(project_name: &str) -> HashMap<String, String> {
    let mut labels = HashMap::new();
    labels.insert(
        "com.docker.compose.project".to_string(),
        project_name.to_string(),
    );
    labels
}

// ---------------------------------------------------------------------------
// Service resources (secrets/configs)
// ---------------------------------------------------------------------------

fn normalize_service_resources(
    raw: Option<&serde_yaml::Value>,
) -> Result<Vec<serde_json::Value>, ComposeError> {
    let raw = match raw {
        None => return Ok(Vec::new()),
        Some(r) => r,
    };
    let items = as_list(raw);
    let mut result = Vec::new();
    for item in items {
        match item {
            serde_yaml::Value::String(s) => {
                let mut map = serde_json::Map::new();
                map.insert("source".into(), serde_json::Value::String(s.clone()));
                map.insert("target".into(), serde_json::Value::String(s.clone()));
                result.push(serde_json::Value::Object(map));
            }
            serde_yaml::Value::Mapping(m) => {
                let mut json_map = serde_json::Map::new();
                let mut source_val = None;
                for (k, v) in m {
                    let key = k.as_str().unwrap_or_default();
                    if key == "source" || key == "target" {
                        source_val = source_val.or(v.as_str().map(|s| s.to_string()));
                    }
                    json_map.insert(key.to_string(), yaml_value_to_json(v));
                }
                if let Some(src) = source_val {
                    json_map.insert("source".into(), serde_json::Value::String(src));
                }
                result.push(serde_json::Value::Object(json_map));
            }
            _ => {
                return Err(ComposeError::Validation(
                    "service secrets/configs entries must be strings or mappings".into(),
                ))
            }
        }
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_interpolation_env(
    working_dir: &Path,
    env_file: Option<&Path>,
    env: Option<&HashMap<String, String>>,
) -> HashMap<String, String> {
    let env_path = env_file
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| working_dir.join(".env"));
    let mut result = parse_env_file(&env_path).unwrap_or_default();

    // Layer in OS environment
    for (k, v) in env::vars() {
        result.insert(k, v);
    }
    // Layer in explicit env overrides
    if let Some(overrides) = env {
        for (k, v) in overrides {
            result.insert(k.clone(), v.clone());
        }
    }
    result
}

fn as_mapping(value: &serde_yaml::Value) -> serde_yaml::Mapping {
    match value {
        serde_yaml::Value::Mapping(m) => m.clone(),
        _ => serde_yaml::Mapping::new(),
    }
}

fn as_mapping_opt<'a>(
    value: Option<&'a serde_yaml::Value>,
    field_name: &str,
) -> Result<Option<&'a serde_yaml::Mapping>, ComposeError> {
    match value {
        None => Ok(None),
        Some(serde_yaml::Value::Mapping(m)) => Ok(Some(m)),
        Some(_) => Err(ComposeError::Validation(format!(
            "{} must be a mapping",
            field_name
        ))),
    }
}

fn as_list(value: &serde_yaml::Value) -> Vec<&serde_yaml::Value> {
    match value {
        serde_yaml::Value::Sequence(seq) => seq.iter().collect(),
        serde_yaml::Value::Null => Vec::new(),
        other => vec![other],
    }
}

fn string_list(raw: Option<&serde_yaml::Value>) -> Vec<String> {
    match raw {
        None => Vec::new(),
        Some(serde_yaml::Value::Sequence(seq)) => seq
            .iter()
            .map(|v| v.as_str().unwrap_or_default().to_string())
            .collect(),
        Some(serde_yaml::Value::String(s)) => vec![s.clone()],
        Some(other) => vec![other.as_str().unwrap_or_default().to_string()],
    }
}

fn optional_string(raw: Option<&serde_yaml::Value>) -> Option<String> {
    raw.and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn optional_int(raw: Option<&serde_yaml::Value>) -> Option<u32> {
    raw.and_then(|v| v.as_i64())
        .and_then(|n| u32::try_from(n).ok())
}

fn checked_u16(value: i64, field: &str) -> Result<u16, ComposeError> {
    u16::try_from(value).map_err(|_| {
        ComposeError::Validation(format!(
            "{} must be between 0 and 65535, got {}",
            field, value
        ))
    })
}

fn yaml_value_to_json(v: &serde_yaml::Value) -> serde_json::Value {
    match v {
        serde_yaml::Value::Null => serde_json::Value::Null,
        serde_yaml::Value::Bool(b) => serde_json::Value::Bool(*b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_json::Value::Number(i.into())
            } else if let Some(f) = n.as_f64() {
                serde_json::Value::Number(
                    serde_json::Number::from_f64(f).unwrap_or(serde_json::Number::from(0)),
                )
            } else {
                serde_json::Value::Null
            }
        }
        serde_yaml::Value::String(s) => serde_json::Value::String(s.clone()),
        serde_yaml::Value::Sequence(seq) => {
            serde_json::Value::Array(seq.iter().map(yaml_value_to_json).collect())
        }
        serde_yaml::Value::Mapping(m) => {
            let mut map = serde_json::Map::new();
            for (k, v) in m {
                let key = k.as_str().unwrap_or_default().to_string();
                map.insert(key, yaml_value_to_json(v));
            }
            serde_json::Value::Object(map)
        }
        serde_yaml::Value::Tagged(t) => yaml_value_to_json(&t.value),
    }
}

fn yaml_mapping_to_json_map(raw: Option<&serde_yaml::Value>) -> HashMap<String, serde_json::Value> {
    match raw {
        Some(serde_yaml::Value::Mapping(m)) => m
            .iter()
            .map(|(k, v)| {
                let key = k.as_str().unwrap_or_default().to_string();
                (key, yaml_value_to_json(v))
            })
            .collect(),
        _ => HashMap::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_project_name() {
        assert_eq!(normalize_project_name("My-Project").unwrap(), "my-project");
        assert_eq!(normalize_project_name("my_project").unwrap(), "my_project");
        assert_eq!(normalize_project_name("foo.bar").unwrap(), "foo-bar");
        assert_eq!(normalize_project_name("_test").unwrap(), "test");
    }

    #[test]
    fn test_normalize_project_name_invalid() {
        assert!(normalize_project_name("---").is_err());
        assert!(normalize_project_name("!!!").is_err());
    }

    #[test]
    fn test_interpolate_basic() {
        let mut env = HashMap::new();
        env.insert("APP_PORT".to_string(), "8080".to_string());

        let result = interpolate_compose_text("ports:\n  - \"${APP_PORT}:80\"", &env).unwrap();
        assert_eq!(result, "ports:\n  - \"8080:80\"");
    }

    #[test]
    fn test_interpolate_default() {
        let env = HashMap::new();
        let result = interpolate_compose_text("image: ${TAG:-latest}", &env).unwrap();
        assert_eq!(result, "image: latest");
    }

    #[test]
    fn test_interpolate_default_with_value() {
        let mut env = HashMap::new();
        env.insert("TAG".to_string(), "v2".to_string());
        let result = interpolate_compose_text("image: ${TAG:-latest}", &env).unwrap();
        assert_eq!(result, "image: v2");
    }

    #[test]
    fn test_interpolate_required_missing() {
        let env = HashMap::new();
        let result = interpolate_compose_text("image: ${DB_PASS:?}", &env);
        assert!(result.is_err());
    }

    #[test]
    fn test_interpolate_dollar_escape() {
        let env = HashMap::new();
        let result = interpolate_compose_text("value: $$not_var", &env).unwrap();
        assert_eq!(result, "value: $not_var");
    }

    #[test]
    fn test_parse_port_string() {
        let ports = parse_port_string("8080:80").unwrap();
        let port = &ports[0];
        assert_eq!(port.published, Some(8080));
        assert_eq!(port.target, 80);
        assert_eq!(port.protocol, "tcp");

        let ports = parse_port_string("127.0.0.1:8080:80").unwrap();
        let port = &ports[0];
        assert_eq!(port.host_ip, Some("127.0.0.1".to_string()));
        assert_eq!(port.published, Some(8080));
        assert_eq!(port.target, 80);

        let ports = parse_port_string("80/udp").unwrap();
        let port = &ports[0];
        assert_eq!(port.target, 80);
        assert_eq!(port.protocol, "udp");
        assert_eq!(port.published, None);
    }

    #[test]
    fn test_parse_port_range_string() {
        let ports = parse_port_string("8000-8001:80-81").unwrap();
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].published, Some(8000));
        assert_eq!(ports[0].target, 80);
        assert_eq!(ports[1].published, Some(8001));
        assert_eq!(ports[1].target, 81);
    }

    #[test]
    fn test_long_port_syntax_rejects_out_of_range_values() {
        let yaml = r#"
services:
  app:
    image: myapp
    ports:
      - target: 70000
        published: -1
"#;
        let err = parse_compose(yaml, Some("test"), &[], None, None).unwrap_err();
        assert!(err.to_string().contains("port target"));
    }

    #[test]
    fn test_invalid_services_type_returns_validation_error() {
        let err = parse_compose("services: nope", Some("test"), &[], None, None).unwrap_err();
        assert!(err.to_string().contains("services must be a mapping"));
    }

    #[test]
    fn test_split_volume_spec() {
        assert_eq!(split_volume_spec("data:/app"), vec!["data", "/app"]);
        assert_eq!(split_volume_spec("C:\\data:/app"), vec!["C:\\data", "/app"]);
    }

    #[test]
    fn test_parse_env_file() {
        let dir = std::env::temp_dir().join("pystack_test_env");
        std::fs::create_dir_all(&dir).unwrap();
        let env_path = dir.join(".env");
        std::fs::write(
            &env_path,
            "DB_HOST=localhost\n# comment\nDB_PORT=5432\nexport DB_USER=admin\n",
        )
        .unwrap();

        let env = parse_env_file(&env_path).unwrap();
        assert_eq!(env.get("DB_HOST").unwrap(), "localhost");
        assert_eq!(env.get("DB_PORT").unwrap(), "5432");
        assert_eq!(env.get("DB_USER").unwrap(), "admin");
        assert!(!env.contains_key("comment"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_parse_compose_simple() {
        let yaml = r#"
services:
  web:
    image: nginx:latest
    ports:
      - "8080:80"
    restart: always
  db:
    image: postgres:15
    environment:
      POSTGRES_PASSWORD: secret
"#;
        let project = parse_compose(yaml, Some("test"), &[], None, None).unwrap();
        assert_eq!(project.name, "test");
        assert_eq!(project.services.len(), 2);

        let web = project.services.get("web").unwrap();
        assert_eq!(web.image, Some("nginx:latest".to_string()));
        assert_eq!(web.ports.len(), 1);
        assert_eq!(web.ports[0].published, Some(8080));
        assert_eq!(web.ports[0].target, 80);
        assert_eq!(web.restart.name, "always");

        let db = project.services.get("db").unwrap();
        assert_eq!(
            db.environment.get("POSTGRES_PASSWORD"),
            Some(&Some("secret".to_string()))
        );
    }

    #[test]
    fn test_parse_compose_depends_on() {
        let yaml = r#"
services:
  api:
    image: myapp
    depends_on:
      db:
        condition: service_healthy
      redis:
        condition: service_started
        required: false
  db:
    image: postgres
  redis:
    image: redis
"#;
        let project = parse_compose(yaml, Some("app"), &[], None, None).unwrap();
        let api = project.services.get("api").unwrap();
        assert_eq!(api.depends_on.len(), 2);

        let db_dep = api.depends_on.iter().find(|d| d.service == "db").unwrap();
        assert_eq!(db_dep.condition, "service_healthy");
        assert!(db_dep.required);

        let redis_dep = api
            .depends_on
            .iter()
            .find(|d| d.service == "redis")
            .unwrap();
        assert_eq!(redis_dep.condition, "service_started");
        assert!(!redis_dep.required);
    }

    #[test]
    fn test_parse_compose_volumes() {
        let yaml = r#"
services:
  app:
    image: myapp
    volumes:
      - data:/var/lib/data
      - ./config:/etc/app:ro
volumes:
  data:
"#;
        let project = parse_compose(yaml, Some("test"), &[], None, None).unwrap();
        let app = project.services.get("app").unwrap();
        assert_eq!(app.volumes.len(), 2);

        let data_vol = &app.volumes[0];
        assert_eq!(data_vol.r#type, "volume");
        assert_eq!(data_vol.source, Some("data".to_string()));
        assert_eq!(data_vol.target, "/var/lib/data");

        let config_vol = &app.volumes[1];
        assert_eq!(config_vol.r#type, "bind");
        assert_eq!(config_vol.source, Some("./config".to_string()));
        assert!(config_vol.read_only);
    }

    #[test]
    fn test_parse_compose_profiles() {
        let yaml = r#"
services:
  web:
    image: nginx
  debug:
    image: debug-tool
    profiles:
      - debug
"#;
        let project = parse_compose(yaml, Some("test"), &[], None, None).unwrap();
        assert_eq!(project.services.len(), 1);
        assert!(project.services.contains_key("web"));

        let profiles = vec!["debug".to_string()];
        let project = parse_compose(yaml, Some("test"), &profiles, None, None).unwrap();
        assert_eq!(project.services.len(), 2);
    }

    #[test]
    fn test_normalize_restart_policy() {
        let policy = normalize_restart(None, None);
        assert_eq!(policy.name, "no");

        let policy = normalize_restart(Some(&serde_yaml::Value::String("always".into())), None);
        assert_eq!(policy.name, "always");

        let policy = normalize_restart(
            Some(&serde_yaml::Value::String("on-failure:3".into())),
            None,
        );
        assert_eq!(policy.name, "on-failure");
        assert_eq!(policy.maximum_retry_count, Some(3));

        let policy = normalize_restart(Some(&serde_yaml::Value::Bool(true)), None);
        assert_eq!(policy.name, "always");
    }

    #[test]
    fn test_yaml_value_to_json() {
        let yaml_val: serde_yaml::Value = serde_yaml::from_str("hello").unwrap();
        let json_val = yaml_value_to_json(&yaml_val);
        assert_eq!(json_val, serde_json::Value::String("hello".to_string()));

        let yaml_val: serde_yaml::Value = serde_yaml::from_str("42").unwrap();
        let json_val = yaml_value_to_json(&yaml_val);
        assert_eq!(json_val, serde_json::Value::Number(42.into()));

        let yaml_val: serde_yaml::Value = serde_yaml::from_str("true").unwrap();
        let json_val = yaml_value_to_json(&yaml_val);
        assert_eq!(json_val, serde_json::Value::Bool(true));
    }

    #[test]
    fn test_resource_name() {
        let mut m = serde_yaml::Mapping::new();
        assert_eq!(resource_name("myapp", "db", &m), "myapp_db");

        m.insert("name".into(), "custom_name".into());
        assert_eq!(resource_name("myapp", "db", &m), "custom_name");

        let mut m = serde_yaml::Mapping::new();
        m.insert("external".into(), true.into());
        assert_eq!(resource_name("myapp", "db", &m), "db");
    }
}
