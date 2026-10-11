//! Shared data types for StackDeck Runner.
//!
//! These types mirror the Python dataclasses from `core.py`, `compose_support.py`,
//! and `hyperv_backend.py`, providing a common vocabulary for all crates.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const APP_NAME: &str = "stackdeck-runner";
pub const VERSION: &str = "0.6.0";
pub const DEFAULT_CONFIG: &str = "stack.json";
pub const DEFAULT_STATE_DIR: &str = ".stackdeck";
pub const DEFAULT_LOG_DIR: &str = ".stackdeck/logs";
pub const STACKDECK_CONFIG_DIR_ENV: &str = "STACKDECK_CONFIG_DIR";

/// Sensitive environment variable name hints used for log redaction.
pub const SAFE_ENV_NAME_HINTS: &[&str] = &[
    "PASS",
    "PASSWORD",
    "SECRET",
    "TOKEN",
    "KEY",
    "PRIVATE",
    "CREDENTIAL",
];

pub const REDACTION: &str = "***REDACTED***";

pub fn generate_random_password() -> String {
    use rand::RngCore;
    let mut bytes = [0_u8; 18];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let body = bytes
        .iter()
        .map(|byte| ALPHABET[*byte as usize % ALPHABET.len()] as char)
        .collect::<String>();
    format!("StackDeck_{body}!")
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!(
        "tmp.{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let write_result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        replace_file(&tmp, path)?;
        sync_parent(path)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    write_result
}

/// Cross-process exclusive lock for read-modify-write state files.
///
/// The lock lives beside the protected file and is deliberately separate from
/// the atomically replaced data file so replacing the data never drops the
/// lock. Callers must keep this guard alive for the complete transaction.
pub struct FileLock {
    file: Option<File>,
}

impl FileLock {
    pub fn exclusive(path: &Path) -> std::io::Result<Self> {
        let lock_path = path.with_extension(format!(
            "{}.lock",
            path.extension()
                .and_then(|value| value.to_str())
                .unwrap_or("state")
        ));
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Keep a stable lock inode/path and let the operating system arbitrate
        // ownership. Deleting age-based sentinel files is unsafe: a delayed
        // stale-lock cleaner can unlink a replacement lock owned by another
        // process. OS locks are released automatically if a process exits.
        let file = OpenOptions::new()
            .create(true)
            .truncate(false) // Lock file identity/content must persist across acquisitions.
            .read(true)
            .write(true)
            .open(&lock_path)?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(Self { file: Some(file) }),
                Err(error) if lock_is_contended(&error) => {
                    if Instant::now() >= deadline {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            format!("timed out waiting for state lock {}", lock_path.display()),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(error) => return Err(error),
            }
        }
    }
}

fn lock_is_contended(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock
        || (cfg!(windows) && error.raw_os_error() == Some(33)) // ERROR_LOCK_VIOLATION
}

impl Drop for FileLock {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = fs2::FileExt::unlock(&file);
        }
    }
}

#[cfg(windows)]
fn replace_file(tmp: &Path, path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    if !path.exists() {
        return std::fs::rename(tmp, path);
    }

    const REPLACEFILE_WRITE_THROUGH: u32 = 0x00000001;
    let path_w = wide(path);
    let tmp_w = wide(tmp);
    let ok = unsafe {
        ReplaceFileW(
            path_w.as_ptr(),
            tmp_w.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_WRITE_THROUGH,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(tmp: &Path, path: &Path) -> std::io::Result<()> {
    std::fs::rename(tmp, path)
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum StackError {
    #[error("{0}")]
    Config(String),
    #[error("{0}")]
    Process(String),
    #[error("{0}")]
    State(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Core types (from core.py)
// ---------------------------------------------------------------------------

/// Health check configuration for a service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthCheck {
    #[serde(default = "HealthCheck::default_type")]
    pub r#type: String,
    #[serde(default)]
    pub test: Option<serde_json::Value>,
    #[serde(default)]
    pub url: String,
    #[serde(default = "HealthCheck::default_host")]
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default = "HealthCheck::default_timeout")]
    pub timeout_seconds: u32,
    #[serde(default = "HealthCheck::default_interval")]
    pub interval_seconds: u32,
    #[serde(default = "HealthCheck::default_retries")]
    pub retries: u32,
    #[serde(default)]
    pub start_period_seconds: u32,
}

impl HealthCheck {
    fn default_type() -> String {
        "process".into()
    }
    fn default_host() -> String {
        "127.0.0.1".into()
    }
    fn default_timeout() -> u32 {
        20
    }
    fn default_interval() -> u32 {
        2
    }
    fn default_retries() -> u32 {
        1
    }
}

impl Default for HealthCheck {
    fn default() -> Self {
        Self {
            r#type: Self::default_type(),
            test: None,
            url: String::new(),
            host: Self::default_host(),
            port: 0,
            timeout_seconds: Self::default_timeout(),
            interval_seconds: Self::default_interval(),
            retries: Self::default_retries(),
            start_period_seconds: 0,
        }
    }
}

/// Resource limits for a service (Windows Job Objects).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceLimits {
    #[serde(default)]
    pub memory_mb: u32,
    #[serde(default)]
    pub process_count: u32,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            memory_mb: 0,
            process_count: 0,
        }
    }
}

/// Service configuration parsed from `stack.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ServiceConfig {
    pub name: String,
    pub cwd: PathBuf,
    pub command: serde_json::Value,
    #[serde(default)]
    pub entrypoint: Option<serde_json::Value>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub env_file: Option<PathBuf>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default = "ServiceConfig::default_restart")]
    pub restart: String,
    #[serde(default = "ServiceConfig::default_max_restarts")]
    pub max_restarts: u32,
    #[serde(default = "ServiceConfig::default_restart_backoff")]
    pub restart_backoff_seconds: u32,
    #[serde(default)]
    pub allow_shell: bool,
    #[serde(default)]
    pub healthcheck: HealthCheck,
    #[serde(default = "ServiceConfig::default_stop_grace")]
    pub stop_grace_seconds: u32,
    #[serde(default = "ServiceConfig::default_log_max_bytes")]
    pub log_max_bytes: u64,
    #[serde(default = "ServiceConfig::default_log_backups")]
    pub log_backups: u32,
    #[serde(default)]
    pub resources: ResourceLimits,
    #[serde(default)]
    pub volumes: Vec<String>,
    #[serde(default)]
    pub ports: Vec<String>,
    #[serde(default)]
    pub networks: Vec<String>,
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub build: Option<serde_json::Value>,
    #[serde(default)]
    pub logging: Option<serde_json::Value>,
    #[serde(default = "ServiceConfig::default_backend")]
    pub backend: String,
    #[serde(default = "ServiceConfig::default_replicas")]
    pub replicas: u32,
    #[serde(default)]
    pub depends_on_conditions: Vec<serde_json::Value>,
    #[serde(default)]
    pub secrets_meta: Vec<serde_json::Value>,
    #[serde(default)]
    pub configs_meta: Vec<serde_json::Value>,
}

impl ServiceConfig {
    fn default_restart() -> String {
        "no".into()
    }
    fn default_max_restarts() -> u32 {
        5
    }
    fn default_restart_backoff() -> u32 {
        3
    }
    fn default_stop_grace() -> u32 {
        10
    }
    fn default_log_max_bytes() -> u64 {
        1_000_000
    }
    fn default_log_backups() -> u32 {
        3
    }
    fn default_backend() -> String {
        "native".into()
    }
    fn default_replicas() -> u32 {
        1
    }
}

/// Full stack configuration parsed from `stack.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StackConfig {
    pub project: String,
    pub root: PathBuf,
    pub config_path: PathBuf,
    pub state_dir: PathBuf,
    pub log_dir: PathBuf,
    pub services: HashMap<String, ServiceConfig>,
    #[serde(default)]
    pub volumes: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub secrets: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub configs: HashMap<String, serde_json::Value>,
    #[serde(default = "StackConfig::default_source_format")]
    pub source_format: String,
}

impl StackConfig {
    fn default_source_format() -> String {
        "stackdeck".into()
    }
}

/// Result of starting a service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StartResult {
    pub service: String,
    pub ok: bool,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Compose types (from compose_support.py)
// ---------------------------------------------------------------------------

/// Service dependency declaration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComposeDependsOn {
    pub service: String,
    #[serde(default = "ComposeDependsOn::default_condition")]
    pub condition: String,
    #[serde(default)]
    pub restart: bool,
    #[serde(default = "default_true")]
    pub required: bool,
}

impl ComposeDependsOn {
    fn default_condition() -> String {
        "service_started".into()
    }
}

fn default_true() -> bool {
    true
}

/// Port mapping from compose spec.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ComposePort {
    pub target: u16,
    pub published: Option<u16>,
    pub host_ip: Option<String>,
    #[serde(default = "ComposePort::default_protocol")]
    pub protocol: String,
    pub mode: Option<String>,
    pub name: Option<String>,
    pub app_protocol: Option<String>,
}

impl ComposePort {
    fn default_protocol() -> String {
        "tcp".into()
    }
}

/// Volume mount specification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ComposeVolumeMount {
    pub r#type: String,
    pub source: Option<String>,
    pub target: String,
    #[serde(default)]
    pub read_only: bool,
    pub consistency: Option<String>,
    #[serde(default)]
    pub bind: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub volume: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub tmpfs: HashMap<String, serde_json::Value>,
}

/// Network attachment specification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ComposeNetworkAttachment {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub ipv4_address: Option<String>,
    pub ipv6_address: Option<String>,
    pub priority: Option<u32>,
}

/// Build configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ComposeBuild {
    pub context: Option<String>,
    pub dockerfile: Option<String>,
    pub target: Option<String>,
    #[serde(default)]
    pub args: HashMap<String, Option<String>>,
    #[serde(default)]
    pub cache_from: Vec<String>,
    #[serde(default)]
    pub cache_to: Vec<String>,
    #[serde(default)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Health check definition from compose spec.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComposeHealthcheck {
    pub test: Option<serde_json::Value>,
    pub interval: Option<String>,
    pub timeout: Option<String>,
    pub retries: Option<u32>,
    pub start_period: Option<String>,
    #[serde(default)]
    pub disable: bool,
}

/// Restart policy from compose spec.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComposeRestartPolicy {
    pub name: String,
    pub maximum_retry_count: Option<u32>,
}

impl Default for ComposeRestartPolicy {
    fn default() -> Self {
        Self {
            name: "no".into(),
            maximum_retry_count: None,
        }
    }
}

/// Named resource (network, volume, secret, config).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ComposeResource {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub external: bool,
    pub driver: Option<String>,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
    pub file: Option<String>,
    pub environment: Option<String>,
    pub content: Option<String>,
}

/// A single service from a compose file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComposeService {
    pub name: String,
    pub image: Option<String>,
    pub build: Option<ComposeBuild>,
    pub command: Option<serde_json::Value>,
    pub entrypoint: Option<serde_json::Value>,
    #[serde(default)]
    pub environment: HashMap<String, Option<String>>,
    #[serde(default)]
    pub env_file: Vec<String>,
    #[serde(default)]
    pub profiles: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<ComposeDependsOn>,
    #[serde(default)]
    pub ports: Vec<ComposePort>,
    #[serde(default)]
    pub volumes: Vec<ComposeVolumeMount>,
    #[serde(default)]
    pub networks: Vec<ComposeNetworkAttachment>,
    #[serde(default)]
    pub secrets: Vec<serde_json::Value>,
    #[serde(default)]
    pub configs: Vec<serde_json::Value>,
    pub healthcheck: Option<ComposeHealthcheck>,
    #[serde(default)]
    pub restart: ComposeRestartPolicy,
    #[serde(default = "ComposeService::default_replicas")]
    pub replicas: u32,
    pub container_name: Option<String>,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    pub logging: Option<serde_json::Value>,
    #[serde(default)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl ComposeService {
    fn default_replicas() -> u32 {
        1
    }
}

/// A parsed compose project.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComposeProject {
    pub name: String,
    pub working_dir: PathBuf,
    #[serde(default)]
    pub services: HashMap<String, ComposeService>,
    #[serde(default)]
    pub networks: HashMap<String, ComposeResource>,
    #[serde(default)]
    pub volumes: HashMap<String, ComposeResource>,
    #[serde(default)]
    pub secrets: HashMap<String, ComposeResource>,
    #[serde(default)]
    pub configs: HashMap<String, ComposeResource>,
    #[serde(default)]
    pub x_extensions: HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Hyper-V types (from hyperv_backend.py)
// ---------------------------------------------------------------------------

/// Hyper-V VM configuration stored in the StackDeck config root.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HyperVConfig {
    #[serde(default = "HyperVConfig::default_vm_name")]
    pub vm_name: String,
    #[serde(default)]
    pub ssh_host: String,
    #[serde(default = "HyperVConfig::default_ssh_user")]
    pub ssh_user: String,
    #[serde(default = "HyperVConfig::default_ssh_port")]
    pub ssh_port: u16,
    #[serde(default)]
    pub ssh_identity: String,
    #[serde(default = "HyperVConfig::default_switch")]
    pub switch_name: String,
    #[serde(default = "HyperVConfig::default_vm_memory")]
    pub vm_memory_mb: u32,
    #[serde(default = "HyperVConfig::default_vm_cpu")]
    pub vm_cpu_count: u32,
    #[serde(default = "HyperVConfig::default_vm_disk")]
    pub vm_disk_gb: u32,
    #[serde(default = "HyperVConfig::default_vm_root")]
    pub vm_root: String,
    #[serde(default = "HyperVConfig::default_runtime")]
    pub container_runtime: String,
    #[serde(default = "HyperVConfig::default_namespace")]
    pub namespace: String,
    #[serde(default = "default_true")]
    pub portproxy: bool,
    #[serde(default)]
    pub ssh_public_key: String,
    #[serde(default)]
    pub windows_host: String,
    #[serde(default)]
    pub smb_user: String,
    // Runtime-only secret. HyperVManager persists it separately with Windows
    // DPAPI; it must never be serialized into stack.json/config JSON.
    #[serde(default, skip_serializing)]
    pub smb_password: String,
    #[serde(default)]
    pub shares: HashMap<String, String>,
    #[serde(default)]
    pub registry_mirrors: HashMap<String, Vec<String>>,
}

impl HyperVConfig {
    fn default_vm_name() -> String {
        "stackdeck-linux".into()
    }
    fn default_ssh_user() -> String {
        "stackdeck".into()
    }
    fn default_ssh_port() -> u16 {
        22
    }
    fn default_switch() -> String {
        "Default Switch".into()
    }
    fn default_vm_memory() -> u32 {
        4096
    }
    fn default_vm_cpu() -> u32 {
        2
    }
    fn default_vm_disk() -> u32 {
        10
    }
    fn default_vm_root() -> String {
        #[cfg(target_os = "windows")]
        {
            let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
            let drive = drive.trim_end_matches(|c| c == '\\' || c == '/');
            return format!(r"{}\StackDeck\VMs\stackdeck-linux", drive);
        }
        #[cfg(not(target_os = "windows"))]
        {
            let home = dirs_home();
            format!("{}/StackDeckVMs/stackdeck-linux", home)
        }
    }
    fn default_runtime() -> String {
        "nerdctl".into()
    }
    fn default_namespace() -> String {
        "stackdeck".into()
    }
}

impl Default for HyperVConfig {
    fn default() -> Self {
        Self {
            vm_name: Self::default_vm_name(),
            ssh_host: String::new(),
            ssh_user: Self::default_ssh_user(),
            ssh_port: Self::default_ssh_port(),
            ssh_identity: String::new(),
            switch_name: Self::default_switch(),
            vm_memory_mb: Self::default_vm_memory(),
            vm_cpu_count: Self::default_vm_cpu(),
            vm_disk_gb: Self::default_vm_disk(),
            vm_root: Self::default_vm_root(),
            container_runtime: Self::default_runtime(),
            namespace: Self::default_namespace(),
            portproxy: true,
            ssh_public_key: String::new(),
            windows_host: String::new(),
            smb_user: String::new(),
            smb_password: String::new(),
            shares: HashMap::new(),
            registry_mirrors: HashMap::new(),
        }
    }
}

/// A service to run on the Hyper-V backend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct HyperVService {
    pub project: String,
    pub name: String,
    pub root: PathBuf,
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub build: Option<serde_json::Value>,
    pub command: Option<serde_json::Value>,
    #[serde(default)]
    pub entrypoint: Option<serde_json::Value>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub ports: Vec<String>,
    #[serde(default)]
    pub volumes: Vec<String>,
    #[serde(default)]
    pub networks: Vec<String>,
    #[serde(default = "HyperVService::default_restart")]
    pub restart: String,
    #[serde(default)]
    pub restart_max_retries: Option<u32>,
    #[serde(default)]
    pub logging: Option<serde_json::Value>,
    #[serde(default)]
    pub secrets: Vec<serde_json::Value>,
    #[serde(default)]
    pub configs: Vec<serde_json::Value>,
    #[serde(default)]
    pub secret_resources: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub config_resources: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub healthcheck: Option<HealthCheck>,
}

impl HyperVService {
    fn default_restart() -> String {
        "no".into()
    }
}

/// Service status in the state database.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceStatus {
    pub project: String,
    pub service: String,
    pub pid: Option<u32>,
    pub status: String,
    pub command_hash: Option<String>,
    pub cwd: Option<String>,
    pub started_at: Option<String>,
    pub updated_at: Option<String>,
    pub restart_count: u32,
    pub last_exit_code: Option<i32>,
    pub last_error: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get the user's home directory, falling back to `.` if unknown.
#[cfg(not(target_os = "windows"))]
fn dirs_home() -> String {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into())
}

fn configured_config_dir() -> Option<PathBuf> {
    std::env::var_os(STACKDECK_CONFIG_DIR_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Get the machine-scoped StackDeck config directory.
pub fn registry_dir() -> PathBuf {
    if let Some(path) = configured_config_dir() {
        return path;
    }
    #[cfg(target_os = "windows")]
    {
        let program_data =
            std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".to_string());
        PathBuf::from(program_data).join("StackDeck").join("config")
    }
    #[cfg(not(target_os = "windows"))]
    {
        PathBuf::from(dirs_home()).join(".stackdeck_runner")
    }
}

/// Get the projects registry file path.
pub fn registry_file() -> PathBuf {
    registry_dir().join("projects.json")
}

/// Get the GUI token file path.
pub fn token_file() -> PathBuf {
    registry_dir().join("gui_token")
}

/// Get the Hyper-V config file path.
pub fn hyperv_config_file() -> PathBuf {
    registry_dir().join("hyperv.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthcheck_defaults() {
        let hc = HealthCheck::default();
        assert_eq!(hc.r#type, "process");
        assert_eq!(hc.host, "127.0.0.1");
        assert_eq!(hc.timeout_seconds, 20);
        assert_eq!(hc.interval_seconds, 2);
        assert_eq!(hc.retries, 1);
    }

    #[test]
    fn resource_limits_defaults() {
        let rl = ResourceLimits::default();
        assert_eq!(rl.memory_mb, 0);
        assert_eq!(rl.process_count, 0);
    }

    #[test]
    fn hyperv_config_defaults() {
        let cfg = HyperVConfig::default();
        assert_eq!(cfg.vm_name, "stackdeck-linux");
        assert_eq!(cfg.ssh_user, "stackdeck");
        assert_eq!(cfg.ssh_port, 22);
        assert_eq!(cfg.switch_name, "Default Switch");
        assert_eq!(cfg.vm_memory_mb, 4096);
        assert_eq!(cfg.vm_cpu_count, 2);
        assert_eq!(cfg.vm_disk_gb, 10);
        assert!(cfg.portproxy);
    }

    #[test]
    fn compose_depends_on_defaults() {
        let dep = ComposeDependsOn {
            service: "db".into(),
            condition: ComposeDependsOn::default_condition(),
            restart: false,
            required: true,
        };
        assert_eq!(dep.condition, "service_started");
        assert!(dep.required);
    }

    #[test]
    fn serialize_deserialize_service_config() {
        let sc = ServiceConfig {
            name: "web".into(),
            cwd: PathBuf::from("."),
            command: serde_json::json!(["python", "-m", "http.server", "8000"]),
            restart: "on-failure".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&sc).unwrap();
        let parsed: ServiceConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "web");
        assert_eq!(parsed.restart, "on-failure");
    }

    #[test]
    fn serialize_deserialize_hyperv_config() {
        let cfg = HyperVConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: HyperVConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, cfg);
    }

    #[test]
    fn hyperv_config_never_serializes_smb_password() {
        let mut cfg = HyperVConfig::default();
        cfg.smb_password = "do-not-write-this-secret".into();
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(!json.contains("do-not-write-this-secret"));
        assert!(!json.contains("smb_password"));

        // Legacy plaintext configs remain readable so HyperVManager can migrate
        // the secret into DPAPI storage on the next load.
        let legacy = r#"{"smb_password":"legacy-secret"}"#;
        let parsed: HyperVConfig = serde_json::from_str(legacy).unwrap();
        assert_eq!(parsed.smb_password, "legacy-secret");
    }

    #[test]
    fn file_lock_uses_stable_os_lock_without_unlinking_owner_path() {
        let root = std::env::temp_dir().join(format!(
            "stackdeck-file-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let state = root.join("state.json");
        let lock_path = root.join("state.json.lock");
        let guard = FileLock::exclusive(&state).unwrap();
        let contender = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        let error = fs2::FileExt::try_lock_exclusive(&contender).unwrap_err();
        assert!(lock_is_contended(&error));
        drop(guard);
        fs2::FileExt::try_lock_exclusive(&contender).unwrap();
        fs2::FileExt::unlock(&contender).unwrap();
        assert!(lock_path.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
