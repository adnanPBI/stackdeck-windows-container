//! Docker Engine API shim for StackDeck Runner.
//!
//! Provides a small Docker Engine-compatible HTTP surface for local tooling.

use axum::{
    body::{Body, Bytes},
    extract::{Path, Query},
    http::{header, HeaderMap, HeaderValue, Request, Response, StatusCode},
    middleware::{self, Next},
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use base64::Engine;
use futures_util::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use stackdeck_hyperv::{CommandOutputStream, HyperVManager, HyperVService};
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr, TcpListener as StdTcpListener},
    path::{Path as StdPath, PathBuf},
    sync::{LazyLock, Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::net::TcpListener;

pub const DEFAULT_HOST: &str = "127.0.0.1";
pub const DEFAULT_PORT: u16 = 23750;
pub const API_VERSION: &str = "1.43";
pub const MIN_API_VERSION: &str = "1.24";
const PROJECT: &str = "docker-api";
static STATE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static PORT_RESERVATIONS: LazyLock<Mutex<HashMap<String, Vec<StdTcpListener>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct StateGuard {
    _process: MutexGuard<'static, ()>,
    _file: stackdeck_types::FileLock,
}

fn state_lock() -> Result<StateGuard, ApiError> {
    let process = STATE_LOCK
        .lock()
        .map_err(|_| ApiError::BlockingTask("Docker API state lock is poisoned".to_string()))?;
    let file = stackdeck_types::FileLock::exclusive(&state_file())?;
    Ok(StateGuard {
        _process: process,
        _file: file,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Unauthorized(String),
    #[error("Hyper-V error: {0}")]
    HyperV(#[from] stackdeck_hyperv::HyperVError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("blocking task failed: {0}")]
    BlockingTask(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let status = match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::HyperV(_) | Self::Io(_) | Self::Json(_) | Self::BlockingTask(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (status, Json(json!({ "message": self.to_string() }))).into_response()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ApiState {
    #[serde(default)]
    containers: HashMap<String, ContainerMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContainerMeta {
    id: String,
    name: String,
    runtime_name: String,
    #[serde(default)]
    runtime_id: Option<String>,
    #[serde(default)]
    runtime_pending: bool,
    image: String,
    #[serde(default)]
    cmd: Option<Value>,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default)]
    ports: Vec<String>,
    #[serde(default)]
    binds: Vec<String>,
    created: u64,
    #[serde(default)]
    docker_spec: Value,
}

#[derive(Debug, Deserialize, Default)]
struct CreateQuery {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Deserialize, Default)]
struct ContainerListQuery {
    #[serde(default)]
    all: Option<String>,
}

impl ContainerListQuery {
    fn include_all(&self) -> bool {
        self.all
            .as_deref()
            .map(|value| matches!(value, "1" | "true" | "True" | "TRUE" | "yes"))
            .unwrap_or(false)
    }
}

#[derive(Debug, Deserialize, Default)]
struct TailQuery {
    #[serde(default = "default_tail")]
    tail: u32,
}

#[derive(Debug, Deserialize, Default)]
struct ImageCreateQuery {
    #[serde(rename = "fromImage", default)]
    from_image: String,
}

#[derive(Debug, Deserialize, Default)]
struct RegistryAuth {
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default, rename = "identitytoken")]
    identity_token: String,
    #[serde(default, rename = "serveraddress")]
    server_address: String,
}

fn default_tail() -> u32 {
    100
}

pub async fn serve(host: &str, port: u16, allow_remote: bool) -> Result<(), ApiError> {
    let addr: SocketAddr = format!("{}:{}", host, port)
        .parse()
        .map_err(|err| ApiError::BadRequest(format!("invalid listen address: {err}")))?;
    if !addr.ip().is_loopback() {
        if !allow_remote {
            return Err(ApiError::BadRequest(
                "refusing to bind Docker API shim to a non-loopback address without --allow-remote"
                    .to_string(),
            ));
        }
    }
    if !addr.ip().is_loopback() && !require_api_token() {
        return Err(ApiError::BadRequest(
            "non-loopback Docker API listeners require STACKDECK_DOCKER_API_REQUIRE_TOKEN=1"
                .to_string(),
        ));
    }
    if require_api_token() && auth_token().is_none() {
        return Err(ApiError::BadRequest(
            "STACKDECK_DOCKER_API_TOKEN must be set when token authentication is required"
                .to_string(),
        ));
    }
    let listener = TcpListener::bind(addr).await?;
    restore_port_reservations()?;
    tracing::info!("Docker API shim listening on http://{}", addr);
    axum::serve(listener, app()).await?;
    Ok(())
}

pub fn app() -> Router {
    Router::new()
        .route("/_ping", get(ping))
        .route("/v{version}/_ping", get(ping))
        .route("/version", get(version))
        .route("/v{version}/version", get(version))
        .route("/containers/json", get(containers_json))
        .route("/v{version}/containers/json", get(containers_json))
        .route("/containers/create", post(containers_create))
        .route("/v{version}/containers/create", post(containers_create))
        .route("/containers/{id}/json", get(container_inspect))
        .route("/v{version}/containers/{id}/json", get(container_inspect))
        .route("/containers/{id}/logs", get(container_logs))
        .route("/v{version}/containers/{id}/logs", get(container_logs))
        .route("/containers/{id}/start", post(container_start))
        .route("/v{version}/containers/{id}/start", post(container_start))
        .route("/containers/{id}/stop", post(container_stop))
        .route("/v{version}/containers/{id}/stop", post(container_stop))
        .route("/containers/{id}", delete(container_remove))
        .route("/v{version}/containers/{id}", delete(container_remove))
        .route("/images/json", get(images_json))
        .route("/v{version}/images/json", get(images_json))
        .route("/images/create", post(image_create))
        .route("/v{version}/images/create", post(image_create))
        .route("/images/{*name}", get(image_inspect).delete(image_remove))
        .route(
            "/v{version}/images/{*name}",
            get(image_inspect).delete(image_remove),
        )
        .route("/networks", get(networks_json))
        .route("/v{version}/networks", get(networks_json))
        .route("/networks/create", post(network_create))
        .route("/v{version}/networks/create", post(network_create))
        .route(
            "/networks/{id}",
            get(network_inspect).delete(network_remove),
        )
        .route(
            "/v{version}/networks/{id}",
            get(network_inspect).delete(network_remove),
        )
        .route("/volumes", get(volumes_json))
        .route("/v{version}/volumes", get(volumes_json))
        .route("/volumes/create", post(volume_create))
        .route("/v{version}/volumes/create", post(volume_create))
        .route("/volumes/{name}", get(volume_inspect).delete(volume_remove))
        .route(
            "/v{version}/volumes/{name}",
            get(volume_inspect).delete(volume_remove),
        )
        .layer(middleware::from_fn(auth_middleware))
        .fallback(fallback)
}

async fn auth_middleware(
    request: Request<Body>,
    next: Next,
) -> Result<impl IntoResponse, ApiError> {
    // Authentication is mandatory by default, including on loopback. This
    // prevents browser-originated blind requests and unrelated local
    // processes from operating the daemon. Compatibility-only opt-out must be
    // explicit through STACKDECK_DOCKER_API_REQUIRE_TOKEN=0.
    if !require_api_token() {
        return Ok(next.run(request).await);
    }
    let Some(token) = auth_token() else {
        return Err(ApiError::Unauthorized(
            "STACKDECK_DOCKER_API_TOKEN must be set and provided as a Bearer token".into(),
        ));
    };
    let valid = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(|provided| {
            use subtle::ConstantTimeEq;
            provided.as_bytes().ct_eq(token.as_bytes()).into()
        })
        .unwrap_or(false);
    if !valid {
        return Err(ApiError::Unauthorized(
            "missing or invalid bearer token".into(),
        ));
    }
    Ok(next.run(request).await)
}

async fn ping() -> impl IntoResponse {
    text_response(StatusCode::OK, "OK")
}

async fn version() -> impl IntoResponse {
    Json(version_payload())
}

fn version_payload() -> Value {
    json!({
        "Version": "24.0.0",
        "ApiVersion": API_VERSION,
        "MinAPIVersion": MIN_API_VERSION,
        "Os": std::env::consts::OS,
        "Arch": std::env::consts::ARCH,
    })
}

async fn containers_json(
    Query(query): Query<ContainerListQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let include_all = query.include_all();
    Ok(Json(blocking(move || list_containers(include_all)).await?))
}

async fn containers_create(
    Query(query): Query<CreateQuery>,
    Json(spec): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    let name = query.name;
    let created = blocking(move || create_container(spec, &name)).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn container_inspect(
    Path(params): Path<HashMap<String, String>>,
) -> Result<impl IntoResponse, ApiError> {
    let id = path_param(&params, "id")?;
    Ok(Json(blocking(move || inspect_raw(&id)).await?))
}

async fn container_logs(
    Path(params): Path<HashMap<String, String>>,
    Query(query): Query<TailQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let id = path_param(&params, "id")?;
    let tty = state_match(&id)?
        .and_then(|(_, meta)| {
            meta.docker_spec
                .get("Tty")
                .and_then(|value| value.as_bool())
        })
        .unwrap_or(false);
    let tail = query.tail;
    let logs = blocking(move || container_logs_text(&id, tail)).await?;
    if tty {
        Ok(response(
            StatusCode::OK,
            "application/vnd.docker.raw-stream",
            Body::from(logs),
        ))
    } else {
        Ok(response(
            StatusCode::OK,
            "application/vnd.docker.raw-stream",
            Body::from(docker_log_frames(&logs)),
        ))
    }
}

async fn container_start(
    Path(params): Path<HashMap<String, String>>,
) -> Result<impl IntoResponse, ApiError> {
    let id = path_param(&params, "id")?;
    blocking(move || start_container(&id)).await?;
    Ok(empty_response(StatusCode::NO_CONTENT))
}

async fn container_stop(
    Path(params): Path<HashMap<String, String>>,
) -> Result<impl IntoResponse, ApiError> {
    let id = path_param(&params, "id")?;
    blocking(move || stop_container(&id)).await?;
    Ok(empty_response(StatusCode::NO_CONTENT))
}

async fn container_remove(
    Path(params): Path<HashMap<String, String>>,
) -> Result<impl IntoResponse, ApiError> {
    let id = path_param(&params, "id")?;
    blocking(move || remove_container(&id)).await?;
    Ok(empty_response(StatusCode::NO_CONTENT))
}

async fn images_json() -> Result<impl IntoResponse, ApiError> {
    Ok(Json(blocking(list_images).await?))
}

async fn image_create(
    headers: HeaderMap,
    Query(query): Query<ImageCreateQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let name = query.from_image;
    if name.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "fromImage query parameter is required".to_string(),
        ));
    }
    let registry_auth = registry_auth(&headers, &name)?;
    let (sender, receiver) = tokio::sync::mpsc::channel::<Bytes>(32);
    tokio::task::spawn_blocking(move || {
        let result = (|| -> Result<(), ApiError> {
            let mgr = manager()?;
            if let Some(auth) = registry_auth {
                let secret = if auth.password.is_empty() {
                    auth.identity_token
                } else {
                    auth.password
                };
                mgr.login_registry(&auth.server_address, &auth.username, &secret)?;
            }
            let progress_sender = sender.clone();
            let image_name = name.clone();
            let mut stdout_decoder = DockerPullProgressDecoder::new(&image_name);
            let mut stderr_decoder = DockerPullProgressDecoder::new(&image_name);
            mgr.ssh_stream_output(
                &stackdeck_hyperv::nerdctl_command(mgr.config(), &["pull", &name]),
                true,
                900,
                |stream, chunk| {
                    let decoder = match stream {
                        CommandOutputStream::Stdout => &mut stdout_decoder,
                        CommandOutputStream::Stderr => &mut stderr_decoder,
                    };
                    decoder
                        .push(&chunk)
                        .into_iter()
                        .all(|event| progress_sender.blocking_send(event).is_ok())
                },
            )?;
            for event in stdout_decoder
                .finish()
                .into_iter()
                .chain(stderr_decoder.finish())
            {
                if sender.blocking_send(event).is_err() {
                    return Ok(());
                }
            }
            let _ = sender.blocking_send(docker_pull_status(
                &name,
                &format!("Downloaded newer image for {name}"),
            ));
            Ok(())
        })();
        if let Err(err) = result {
            let _ = sender.blocking_send(Bytes::from(
                json!({ "error": err.to_string(), "errorDetail": { "message": err.to_string() } })
                    .to_string()
                    + "\n",
            ));
        }
    });
    let events = stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|event| (event, receiver))
    })
    .map(Ok::<Bytes, std::convert::Infallible>);
    Ok(response(
        StatusCode::OK,
        "application/json",
        Body::from_stream(events),
    ))
}

async fn image_inspect(
    Path(params): Path<HashMap<String, String>>,
) -> Result<impl IntoResponse, ApiError> {
    let name = docker_image_json_name(&path_param(&params, "name")?)?;
    Ok(Json(blocking(move || inspect_image(&name)).await?))
}

async fn image_remove(
    Path(params): Path<HashMap<String, String>>,
) -> Result<impl IntoResponse, ApiError> {
    let name = path_param(&params, "name")?;
    Ok(Json(blocking(move || remove_image(&name)).await?))
}

async fn fallback() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "message": "unsupported Docker API endpoint" })),
    )
}

async fn networks_json() -> Result<impl IntoResponse, ApiError> {
    Ok(Json(blocking(list_networks).await?))
}

async fn network_create(Json(spec): Json<Value>) -> Result<impl IntoResponse, ApiError> {
    validate_network_create_spec(&spec)?;
    let name = spec
        .get("Name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    Ok((
        StatusCode::CREATED,
        Json(blocking(move || create_network(&name)).await?),
    ))
}

async fn network_inspect(
    Path(params): Path<HashMap<String, String>>,
) -> Result<impl IntoResponse, ApiError> {
    let id = path_param(&params, "id")?;
    Ok(Json(blocking(move || inspect_network(&id)).await?))
}

async fn network_remove(
    Path(params): Path<HashMap<String, String>>,
) -> Result<impl IntoResponse, ApiError> {
    let id = path_param(&params, "id")?;
    blocking(move || remove_network(&id)).await?;
    Ok(empty_response(StatusCode::NO_CONTENT))
}

async fn volumes_json() -> Result<impl IntoResponse, ApiError> {
    Ok(Json(blocking(list_volumes).await?))
}

async fn volume_create(Json(spec): Json<Value>) -> Result<impl IntoResponse, ApiError> {
    validate_volume_create_spec(&spec)?;
    let name = spec
        .get("Name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    Ok((
        StatusCode::CREATED,
        Json(blocking(move || create_volume(&name)).await?),
    ))
}

async fn volume_inspect(
    Path(params): Path<HashMap<String, String>>,
) -> Result<impl IntoResponse, ApiError> {
    let name = path_param(&params, "name")?;
    Ok(Json(blocking(move || inspect_volume(&name)).await?))
}

async fn volume_remove(
    Path(params): Path<HashMap<String, String>>,
) -> Result<impl IntoResponse, ApiError> {
    let name = path_param(&params, "name")?;
    blocking(move || remove_volume(&name)).await?;
    Ok(empty_response(StatusCode::NO_CONTENT))
}

fn manager() -> Result<HyperVManager, ApiError> {
    Ok(HyperVManager::new(HyperVManager::load_config()?))
}

fn map_nerdctl_not_found<T>(
    result: Result<T, stackdeck_hyperv::HyperVError>,
    kind: &str,
    name: &str,
) -> Result<T, ApiError> {
    match result {
        Ok(value) => Ok(value),
        Err(stackdeck_hyperv::HyperVError::Ssh(message)) if is_not_found_message(&message) => {
            Err(ApiError::NotFound(format!("No such {kind}: {name}")))
        }
        Err(err) => Err(err.into()),
    }
}

fn is_not_found_message(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("not found") || message.contains("no such")
}

async fn blocking<F, T>(f: F) -> Result<T, ApiError>
where
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|err| ApiError::BlockingTask(err.to_string()))?
}

fn path_param(params: &HashMap<String, String>, name: &str) -> Result<String, ApiError> {
    params
        .get(name)
        .cloned()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::BadRequest(format!("missing path parameter: {name}")))
}

fn docker_image_json_name(name: &str) -> Result<String, ApiError> {
    name.strip_suffix("/json")
        .map(|name| name.trim_matches('/').to_string())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| ApiError::NotFound("unsupported image endpoint".to_string()))
}

fn state_file() -> PathBuf {
    stackdeck_types::registry_dir().join("docker_api_containers.json")
}

fn read_state() -> Result<ApiState, ApiError> {
    let path = state_file();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ApiState::default())
        }
        Err(error) => return Err(error.into()),
    };
    serde_json::from_str(&text).map_err(|error| {
        ApiError::BlockingTask(format!(
            "refusing to replace corrupt Docker API state {}: {}",
            path.display(),
            error
        ))
    })
}

fn write_state(state: &ApiState) -> Result<(), ApiError> {
    let path = state_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(state)?;
    stackdeck_types::atomic_write(&path, json.as_bytes())?;
    Ok(())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn parse_created(created_val: Option<&Value>) -> u64 {
    created_val
        .and_then(|v| {
            v.as_u64().or_else(|| {
                v.as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.timestamp() as u64)
            })
        })
        .unwrap_or_else(now_unix)
}

fn slug(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-') {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= 80 {
            break;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "container".to_string()
    } else {
        trimmed.to_string()
    }
}

fn new_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:032x}", nanos)
}

fn normalize_cmd(value: Option<&Value>) -> Option<Value> {
    match value {
        Some(Value::Array(items)) => Some(Value::Array(
            items
                .iter()
                .map(|item| Value::String(item.as_str().unwrap_or_default().to_string()))
                .collect(),
        )),
        Some(Value::String(s)) if !s.is_empty() => Some(Value::String(s.clone())),
        _ => None,
    }
}

fn normalize_env(value: Option<&Value>) -> HashMap<String, String> {
    match value {
        Some(Value::Object(map)) => map
            .iter()
            .filter_map(|(key, value)| value.as_str().map(|v| (key.clone(), v.to_string())))
            .collect(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str())
            .filter_map(|item| item.split_once('='))
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
        _ => HashMap::new(),
    }
}

fn port_bindings(
    exposed_ports: Option<&Value>,
    host_config: Option<&Value>,
    reserved: &mut HashSet<u16>,
    reservations: &mut Vec<StdTcpListener>,
) -> Result<Vec<String>, ApiError> {
    let mut ports = Vec::new();
    if let Some(bindings) = host_config
        .and_then(|v| v.get("PortBindings"))
        .and_then(|v| v.as_object())
    {
        for (container_port, entries) in bindings {
            let (target, protocol) = container_port
                .split_once('/')
                .map(|(port, protocol)| (port, protocol.to_ascii_lowercase()))
                .unwrap_or((container_port, "tcp".to_string()));
            let entries = entries
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![entries.clone()]);
            for entry in entries {
                let host_port = entry
                    .as_object()
                    .and_then(|m| m.get("HostPort"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let host_ip = entry
                    .as_object()
                    .and_then(|m| m.get("HostIp"))
                    .and_then(|v| v.as_str())
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or("127.0.0.1");
                let suffix = if protocol == "tcp" {
                    String::new()
                } else {
                    format!("/{protocol}")
                };
                if host_port.is_empty() {
                    let host_port = reserve_host_port(host_ip, None, reserved, reservations)?;
                    ports.push(format!("{host_ip}:{}:{target}{suffix}", host_port));
                } else {
                    let requested = host_port.parse::<u16>().map_err(|_| {
                        ApiError::BadRequest(format!("invalid HostPort: {host_port}"))
                    })?;
                    let host_port =
                        reserve_host_port(host_ip, Some(requested), reserved, reservations)?;
                    ports.push(format!("{host_ip}:{host_port}:{target}{suffix}"));
                }
            }
        }
    }
    let publish_all = host_config
        .and_then(|v| v.get("PublishAllPorts"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if publish_all {
        if let Some(exposed) = exposed_ports.and_then(|v| v.as_object()) {
            for key in exposed.keys() {
                if let Some(target) = key.split('/').next() {
                    if !target.is_empty() {
                        let protocol = key.split_once('/').map(|(_, value)| value).unwrap_or("tcp");
                        let suffix = if protocol.eq_ignore_ascii_case("tcp") {
                            String::new()
                        } else {
                            format!("/{}", protocol.to_ascii_lowercase())
                        };
                        let host_port =
                            reserve_host_port("127.0.0.1", None, reserved, reservations)?;
                        ports.push(format!("127.0.0.1:{}:{}{}", host_port, target, suffix));
                    }
                }
            }
        }
    }
    Ok(ports)
}

fn binds(host_config: Option<&Value>) -> Vec<String> {
    host_config
        .and_then(|v| v.get("Binds"))
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn validate_binds(binds: &[String]) -> Result<(), ApiError> {
    if binds.is_empty() {
        return Ok(());
    }
    if !env_truthy("STACKDECK_DOCKER_API_ALLOW_BINDS") {
        return Err(ApiError::BadRequest(
            "HostConfig.Binds are disabled by default; set STACKDECK_DOCKER_API_ALLOW_BINDS=1 and STACKDECK_DOCKER_API_BIND_ROOTS to opt in"
                .to_string(),
        ));
    }
    let roots = bind_roots()?;
    for bind in binds {
        let Some(source) = bind_source(bind) else {
            return Err(ApiError::BadRequest(format!("invalid bind mount: {bind}")));
        };
        let source_path = StdPath::new(source);
        let source = source_path.canonicalize().map_err(|err| {
            ApiError::BadRequest(format!(
                "bind source {} must exist and be canonicalizable: {err}",
                source_path.display()
            ))
        })?;
        if !roots.iter().any(|root| source.starts_with(root)) {
            return Err(ApiError::BadRequest(format!(
                "bind source {} is outside allowed bind roots",
                source.display()
            )));
        }
    }
    Ok(())
}

fn bind_roots() -> Result<Vec<PathBuf>, ApiError> {
    let raw = std::env::var("STACKDECK_DOCKER_API_BIND_ROOTS").unwrap_or_default();
    let roots: Vec<PathBuf> = raw
        .split(';')
        .map(str::trim)
        .filter(|root| !root.is_empty())
        .map(StdPath::new)
        .map(|root| {
            root.canonicalize().map_err(|err| {
                ApiError::BadRequest(format!(
                    "bind root {} must exist and be canonicalizable: {err}",
                    root.display()
                ))
            })
        })
        .collect::<Result<_, _>>()?;
    if roots.is_empty() {
        return Err(ApiError::BadRequest(
            "STACKDECK_DOCKER_API_BIND_ROOTS must include at least one allowed root when binds are enabled"
                .to_string(),
        ));
    }
    Ok(roots)
}

fn bind_source(bind: &str) -> Option<&str> {
    if bind.len() >= 3 && bind.as_bytes()[1] == b':' && matches!(bind.as_bytes()[2], b'\\' | b'/') {
        let rest = &bind[3..];
        let idx = rest.find(':')?;
        return Some(&bind[..idx + 3]);
    }
    bind.split_once(':').map(|(source, _)| source)
}

fn reserve_host_port(
    host_ip: &str,
    requested: Option<u16>,
    reserved: &mut HashSet<u16>,
    reservations: &mut Vec<StdTcpListener>,
) -> Result<u16, ApiError> {
    let ip = host_ip.parse::<IpAddr>().map_err(|_| {
        ApiError::BadRequest(format!("HostIp must be a numeric IP address: {host_ip}"))
    })?;
    if !ip.is_ipv4() {
        return Err(ApiError::BadRequest(
            "IPv6 HostIp is unsupported by the Windows TCP portproxy backend".into(),
        ));
    }
    if requested == Some(0) {
        return Err(ApiError::BadRequest(
            "HostPort must be empty for automatic allocation or between 1 and 65535".into(),
        ));
    }
    for _ in 0..100 {
        let port = requested.unwrap_or(0);
        if port != 0 && reserved.contains(&port) {
            return Err(ApiError::Conflict(format!(
                "host port {port} is already reserved"
            )));
        }
        let listener = StdTcpListener::bind(SocketAddr::new(ip, port)).map_err(|error| {
            ApiError::Conflict(format!(
                "host port {}:{} is unavailable: {error}",
                host_ip,
                requested.map_or_else(|| "automatic".to_string(), |port| port.to_string())
            ))
        })?;
        let port = listener.local_addr()?.port();
        if reserved.insert(port) {
            reservations.push(listener);
            return Ok(port);
        }
        if requested.is_some() {
            return Err(ApiError::Conflict(format!(
                "host port {port} is already reserved"
            )));
        }
    }
    Err(ApiError::Conflict(
        "failed to allocate a unique ephemeral host port after 100 attempts".into(),
    ))
}

fn hold_port_reservations(
    container_id: &str,
    listeners: Vec<StdTcpListener>,
) -> Result<(), ApiError> {
    let mut held = PORT_RESERVATIONS
        .lock()
        .map_err(|_| ApiError::BlockingTask("port reservation lock is poisoned".into()))?;
    held.insert(container_id.to_string(), listeners);
    Ok(())
}

fn release_port_reservations(container_id: &str) -> Result<(), ApiError> {
    let mut held = PORT_RESERVATIONS
        .lock()
        .map_err(|_| ApiError::BlockingTask("port reservation lock is poisoned".into()))?;
    held.remove(container_id);
    Ok(())
}

fn reserve_meta_ports(meta: &ContainerMeta) -> Result<(), ApiError> {
    let mut reserved = HashSet::new();
    let mut listeners = Vec::new();
    for spec in &meta.ports {
        let (host_ip, host_port, _) = stackdeck_hyperv::parse_port(spec).ok_or_else(|| {
            ApiError::BadRequest(format!("invalid persisted port mapping: {spec}"))
        })?;
        reserve_host_port(&host_ip, Some(host_port), &mut reserved, &mut listeners)?;
    }
    hold_port_reservations(&meta.id, listeners)
}

fn restore_port_reservations() -> Result<(), ApiError> {
    let _guard = state_lock()?;
    let state = read_state()?;
    for meta in state
        .containers
        .values()
        .filter(|meta| meta.runtime_id.is_none())
    {
        let already_held = PORT_RESERVATIONS
            .lock()
            .map_err(|_| ApiError::BlockingTask("port reservation lock is poisoned".into()))?
            .contains_key(&meta.id);
        if !already_held {
            reserve_meta_ports(meta)?;
        }
    }
    Ok(())
}

fn docker_port_key(spec: &str, container_port: u16) -> String {
    let protocol = spec
        .rsplit_once('/')
        .map(|(_, protocol)| protocol.trim().to_ascii_lowercase())
        .filter(|protocol| !protocol.is_empty())
        .unwrap_or_else(|| "tcp".to_string());
    format!("{container_port}/{protocol}")
}

fn auth_token() -> Option<String> {
    std::env::var("STACKDECK_DOCKER_API_TOKEN")
        .ok()
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
}

fn require_api_token() -> bool {
    std::env::var("STACKDECK_DOCKER_API_REQUIRE_TOKEN")
        .ok()
        .map(|value| {
            !matches!(
                value.as_str(),
                "0" | "false" | "FALSE" | "False" | "no" | "NO"
            )
        })
        .unwrap_or(true)
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.as_str(),
                "1" | "true" | "TRUE" | "True" | "yes" | "YES"
            )
        })
        .unwrap_or(false)
}

fn registry_auth(headers: &HeaderMap, image: &str) -> Result<Option<RegistryAuth>, ApiError> {
    let Some(value) = headers.get("X-Registry-Auth") else {
        return Ok(None);
    };
    let encoded = value
        .to_str()
        .map_err(|_| ApiError::BadRequest("X-Registry-Auth must be ASCII base64".to_string()))?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| ApiError::BadRequest("X-Registry-Auth is not valid base64".to_string()))?;
    let mut auth: RegistryAuth = serde_json::from_slice(&decoded)
        .map_err(|_| ApiError::BadRequest("X-Registry-Auth is not valid JSON".to_string()))?;
    if auth.username.trim().is_empty()
        && auth.password.is_empty()
        && auth.identity_token.is_empty()
        && auth.server_address.trim().is_empty()
    {
        return Ok(None);
    }
    if auth.password.is_empty() && auth.identity_token.is_empty() {
        return Err(ApiError::BadRequest(
            "X-Registry-Auth requires password or identitytoken".to_string(),
        ));
    }
    // Docker identity-token auth commonly omits Username. nerdctl login uses
    // the OCI token convention for that case.
    if auth.username.trim().is_empty() && !auth.identity_token.is_empty() {
        auth.username = "oauth2accesstoken".to_string();
    }
    if auth.username.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "X-Registry-Auth requires username when password is used".to_string(),
        ));
    }
    auth.server_address = normalize_registry_address(&auth.server_address, image)?;
    Ok(Some(auth))
}

fn normalize_registry_address(server_address: &str, image: &str) -> Result<String, ApiError> {
    let supplied = server_address
        .trim()
        .strip_prefix("https://")
        .or_else(|| server_address.trim().strip_prefix("http://"))
        .unwrap_or(server_address.trim())
        .trim_end_matches('/')
        .split('/')
        .next()
        .unwrap_or_default();
    let image_registry = image.split('/').next().filter(|first| {
        first.contains('.') || first.contains(':') || first.eq_ignore_ascii_case("localhost")
    });
    let registry = if supplied.is_empty() {
        image_registry.unwrap_or("docker.io")
    } else if supplied.eq_ignore_ascii_case("index.docker.io") {
        "docker.io"
    } else {
        supplied
    };
    if registry.is_empty()
        || !registry
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | ':' | '-'))
    {
        return Err(ApiError::BadRequest(
            "X-Registry-Auth contains an invalid registry host".to_string(),
        ));
    }
    Ok(registry.to_string())
}

fn docker_pull_status(name: &str, status: &str) -> Bytes {
    Bytes::from(json!({ "status": status, "id": name }).to_string() + "\n")
}

const MAX_PULL_PROGRESS_LINE_BYTES: usize = 64 * 1024;

struct DockerPullProgressDecoder {
    image: String,
    pending: Vec<u8>,
}

impl DockerPullProgressDecoder {
    fn new(image: &str) -> Self {
        Self {
            image: image.to_string(),
            pending: Vec::new(),
        }
    }

    fn push(&mut self, chunk: &[u8]) -> Vec<Bytes> {
        self.pending.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(newline) = self.pending.iter().position(|byte| *byte == b'\n') {
            let mut line = self.pending.drain(..=newline).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.push_line(&line, &mut events);
        }
        if self.pending.len() > MAX_PULL_PROGRESS_LINE_BYTES {
            self.pending.clear();
            events.push(docker_pull_status(
                &self.image,
                "pull progress line exceeded 65536 bytes and was discarded",
            ));
        }
        events
    }

    fn finish(&mut self) -> Vec<Bytes> {
        let pending = std::mem::take(&mut self.pending);
        let mut events = Vec::new();
        self.push_line(&pending, &mut events);
        events
    }

    fn push_line(&self, line: &[u8], events: &mut Vec<Bytes>) {
        let status = String::from_utf8_lossy(line);
        let status = status.trim();
        if !status.is_empty() {
            events.push(docker_pull_status(&self.image, status));
        }
    }
}

fn state_matches(identifier: &str) -> Result<Vec<(String, ContainerMeta)>, ApiError> {
    let ident = identifier.trim_start_matches('/');
    if ident.is_empty() {
        return Ok(Vec::new());
    }
    Ok(read_state()?
        .containers
        .into_iter()
        .filter(|(cid, meta)| {
            cid.starts_with(ident) || meta.name == ident || meta.runtime_name == ident
        })
        .collect())
}

fn state_match(identifier: &str) -> Result<Option<(String, ContainerMeta)>, ApiError> {
    let ident = identifier.trim_start_matches('/');
    let mut matches = state_matches(identifier)?;
    if matches.len() > 1 {
        matches.sort_by(|a, b| a.0.cmp(&b.0));
        let candidates = matches
            .iter()
            .map(|(id, meta)| format!("{} ({})", &id[..12.min(id.len())], meta.name))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(ApiError::BadRequest(format!(
            "ambiguous container reference \"{ident}\" matches: {candidates}"
        )));
    }
    Ok(matches.pop())
}

fn container_ref(identifier: &str) -> Result<String, ApiError> {
    Ok(state_match(identifier)?
        .map(|(_, meta)| meta.runtime_name)
        .unwrap_or_else(|| identifier.trim_start_matches('/').to_string()))
}

fn service_from_meta(meta: &ContainerMeta) -> HyperVService {
    HyperVService {
        project: PROJECT.to_string(),
        name: slug(&meta.name),
        root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        image: meta.image.clone(),
        build: None,
        command: meta.cmd.clone(),
        entrypoint: None,
        env: meta.env.clone(),
        ports: meta.ports.clone(),
        volumes: meta.binds.clone(),
        networks: Vec::new(),
        restart: "no".to_string(),
        restart_max_retries: None,
        logging: None,
        secrets: Vec::new(),
        configs: Vec::new(),
        secret_resources: std::collections::HashMap::new(),
        config_resources: std::collections::HashMap::new(),
        healthcheck: None,
    }
}

fn validate_create_spec(spec: &Value) -> Result<(), ApiError> {
    let Some(obj) = spec.as_object() else {
        return Err(ApiError::BadRequest(
            "container create body must be a JSON object".to_string(),
        ));
    };
    for (key, value) in obj {
        match key.as_str() {
            "Image" | "Cmd" | "Env" | "ExposedPorts" => {}
            "HostConfig" => validate_host_config(value)?,
            _ if value_has_semantics(value) => {
                return Err(unsupported_create_field(&format!("Container.{key}")));
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_host_config(value: &Value) -> Result<(), ApiError> {
    let Some(obj) = value.as_object() else {
        if value_has_semantics(value) {
            return Err(ApiError::BadRequest(
                "HostConfig must be a JSON object when provided".to_string(),
            ));
        }
        return Ok(());
    };
    for (key, value) in obj {
        match key.as_str() {
            "Binds" | "PortBindings" | "PublishAllPorts" => {}
            "NetworkMode" => {
                if let Some(mode) = value.as_str() {
                    if !matches!(mode, "" | "default" | "bridge") {
                        return Err(unsupported_create_field("HostConfig.NetworkMode"));
                    }
                } else if value_has_semantics(value) {
                    return Err(unsupported_create_field("HostConfig.NetworkMode"));
                }
            }
            "RestartPolicy" => validate_restart_policy(value)?,
            _ if value_has_semantics(value) => {
                return Err(unsupported_create_field(&format!("HostConfig.{key}")));
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_restart_policy(value: &Value) -> Result<(), ApiError> {
    let Some(obj) = value.as_object() else {
        if value_has_semantics(value) {
            return Err(unsupported_create_field("HostConfig.RestartPolicy"));
        }
        return Ok(());
    };
    let name = obj.get("Name").and_then(|v| v.as_str()).unwrap_or_default();
    let retry_count = obj
        .get("MaximumRetryCount")
        .and_then(|v| v.as_u64())
        .unwrap_or_default();
    if matches!(name, "" | "no") && retry_count == 0 {
        Ok(())
    } else {
        Err(unsupported_create_field("HostConfig.RestartPolicy"))
    }
}

fn validate_network_create_spec(spec: &Value) -> Result<(), ApiError> {
    let Some(obj) = spec.as_object() else {
        return Err(ApiError::BadRequest(
            "network create body must be a JSON object".to_string(),
        ));
    };
    for (key, value) in obj {
        match key.as_str() {
            "Name" => {}
            "Driver" => {
                if value
                    .as_str()
                    .is_some_and(|driver| !matches!(driver, "" | "bridge"))
                {
                    return Err(unsupported_create_field("Network.Driver"));
                }
            }
            _ if value_has_semantics(value) => {
                return Err(unsupported_create_field(&format!("Network.{key}")));
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_volume_create_spec(spec: &Value) -> Result<(), ApiError> {
    let Some(obj) = spec.as_object() else {
        return Err(ApiError::BadRequest(
            "volume create body must be a JSON object".to_string(),
        ));
    };
    for (key, value) in obj {
        match key.as_str() {
            "Name" => {}
            "Driver" => {
                if value
                    .as_str()
                    .is_some_and(|driver| !matches!(driver, "" | "local"))
                {
                    return Err(unsupported_create_field("Volume.Driver"));
                }
            }
            _ if value_has_semantics(value) => {
                return Err(unsupported_create_field(&format!("Volume.{key}")));
            }
            _ => {}
        }
    }
    Ok(())
}

fn unsupported_create_field(field: &str) -> ApiError {
    ApiError::BadRequest(format!(
        "{field} is not supported by this Docker API shim; refusing to ignore requested semantics"
    ))
}

fn value_has_semantics(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|n| n != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

fn create_container(spec: Value, name: &str) -> Result<Value, ApiError> {
    validate_create_spec(&spec)?;
    let image = spec
        .get("Image")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if image.is_empty() {
        return Err(ApiError::BadRequest("Image is required".to_string()));
    }

    let id = new_id();
    let generated_name;
    let requested_name = if name.is_empty() {
        generated_name = format!("container-{}", &id[..12]);
        generated_name.as_str()
    } else {
        name
    };
    let docker_name = requested_name.trim_start_matches('/').to_string();
    let service_name = slug(&docker_name);
    let binds = binds(spec.get("HostConfig"));
    validate_binds(&binds)?;
    let _guard = state_lock()?;
    let mut state = read_state()?;
    let mut reserved_ports: HashSet<u16> = state
        .containers
        .values()
        .flat_map(|container| container.ports.iter())
        .filter_map(|port| stackdeck_hyperv::parse_port(port).map(|(_, host, _)| host))
        .collect();
    let mut port_reservations = Vec::new();
    let ports = port_bindings(
        spec.get("ExposedPorts"),
        spec.get("HostConfig"),
        &mut reserved_ports,
        &mut port_reservations,
    )?;
    if ports
        .iter()
        .any(|port| port.trim().to_ascii_lowercase().ends_with("/udp"))
    {
        return Err(ApiError::BadRequest(
            "UDP published ports are unsupported by the current Windows TCP portproxy backend"
                .to_string(),
        ));
    }
    let meta = ContainerMeta {
        id: id.clone(),
        name: docker_name.clone(),
        runtime_name: stackdeck_hyperv::container_name(PROJECT, &service_name),
        runtime_id: None,
        runtime_pending: false,
        image,
        cmd: normalize_cmd(spec.get("Cmd")),
        env: normalize_env(spec.get("Env")),
        ports,
        binds,
        created: now_unix(),
        docker_spec: spec,
    };

    if state
        .containers
        .values()
        .any(|existing| existing.name == docker_name || existing.runtime_name == meta.runtime_name)
    {
        return Err(ApiError::Conflict(format!(
            "Conflict. The container name \"{docker_name}\" is already in use"
        )));
    }
    state.containers.insert(id.clone(), meta);
    write_state(&state)?;
    hold_port_reservations(&id, port_reservations)?;
    Ok(json!({ "Id": id, "Warnings": [] }))
}

fn start_container(identifier: &str) -> Result<(), ApiError> {
    if let Some((_id, meta)) = state_match(identifier)? {
        let mgr = manager()?;
        {
            // Persist intent before starting the guest runtime. A crash after
            // this commit leaves a recoverable "possibly started" record, so
            // removal will reconcile the deterministic runtime name instead
            // of deleting API state as if no container could exist.
            let _guard = state_lock()?;
            let mut state = read_state()?;
            let stored = state.containers.get_mut(&meta.id).ok_or_else(|| {
                ApiError::Conflict(format!(
                    "container state {} disappeared before start",
                    meta.id
                ))
            })?;
            stored.runtime_pending = true;
            write_state(&state)?;
        }
        let existing = mgr
            .ssh(
                &stackdeck_hyperv::nerdctl_command(mgr.config(), &["inspect", &meta.runtime_name]),
                false,
            )
            .unwrap_or_default();
        let existed_before = !existing.trim().is_empty();
        let was_running = runtime_inspect_is_running(&existing);
        let service = service_from_meta(&meta);
        if let Err(error) = mgr.start_service_deferred_portproxy(&service, false) {
            return Err(error.into());
        }
        release_port_reservations(&meta.id)?;
        if let Err(error) = mgr.ensure_portproxy(&service) {
            if !existed_before {
                let _ = mgr.stop_service(&service);
            } else if !was_running {
                let _ = mgr.ssh(
                    &stackdeck_hyperv::nerdctl_command(mgr.config(), &["stop", &meta.runtime_name]),
                    false,
                );
            }
            let _ = reserve_meta_ports(&meta);
            return Err(error.into());
        }
        let runtime_id = mgr
            .ssh(
                &stackdeck_hyperv::nerdctl_command(
                    mgr.config(),
                    &["inspect", "--format", "{{.Id}}", &meta.runtime_name],
                ),
                true,
            )?
            .trim()
            .to_string();
        if runtime_id.is_empty() {
            if !existed_before {
                let _ = mgr.stop_service(&service_from_meta(&meta));
                let _ = reserve_meta_ports(&meta);
            }
            return Err(ApiError::Conflict(format!(
                "runtime started {} but returned no container identity",
                meta.runtime_name
            )));
        }
        let persist_result = (|| -> Result<(), ApiError> {
            let _guard = state_lock()?;
            let mut state = read_state()?;
            let stored = state.containers.get_mut(&meta.id).ok_or_else(|| {
                ApiError::Conflict(format!(
                    "container state {} disappeared while it was starting",
                    meta.id
                ))
            })?;
            stored.runtime_id = Some(runtime_id);
            stored.runtime_pending = false;
            write_state(&state)
        })();
        if let Err(error) = persist_result {
            if !existed_before {
                let _ = mgr.stop_service(&service_from_meta(&meta));
                let _ = reserve_meta_ports(&meta);
            } else if !was_running {
                let _ = mgr.ssh(
                    &stackdeck_hyperv::nerdctl_command(mgr.config(), &["stop", &meta.runtime_name]),
                    false,
                );
                let _ = mgr.remove_portproxy(&service_from_meta(&meta));
                let _ = reserve_meta_ports(&meta);
            }
            return Err(error);
        }
        return Ok(());
    }
    let mgr = manager()?;
    let reference = container_ref(identifier)?;
    let cmd = stackdeck_hyperv::nerdctl_command(mgr.config(), &["start", &reference]);
    mgr.ssh(&cmd, true)?;
    Ok(())
}

fn stop_container(identifier: &str) -> Result<(), ApiError> {
    let mgr = manager()?;
    if let Some((_id, meta)) = state_match(identifier)? {
        mgr.remove_portproxy(&service_from_meta(&meta))?;
    }
    let reference = container_ref(identifier)?;
    let cmd = stackdeck_hyperv::nerdctl_command(mgr.config(), &["stop", &reference]);
    mgr.ssh(&cmd, true)?;
    Ok(())
}

fn remove_container(identifier: &str) -> Result<(), ApiError> {
    let matched = state_match(identifier)?;
    let matched_id = matched.as_ref().map(|(id, _)| id.clone());
    if let Some((id, meta)) = matched.as_ref() {
        if meta.runtime_id.is_none() && !meta.runtime_pending {
            let _guard = state_lock()?;
            let mut state = read_state()?;
            state.containers.remove(id);
            write_state(&state)?;
            release_port_reservations(id)?;
            return Ok(());
        }
    }

    let mgr = manager()?;
    let reference = if let Some((_id, meta)) = matched.as_ref() {
        mgr.stop_service(&service_from_meta(meta))?;
        meta.runtime_name.clone()
    } else {
        let reference = container_ref(identifier)?;
        let cmd = stackdeck_hyperv::nerdctl_command(mgr.config(), &["rm", "-f", &reference]);
        mgr.ssh(&cmd, true)?;
        reference
    };
    let verify = mgr.ssh(
        &format!(
            "{} >/dev/null 2>&1 && echo STACKDECK_EXISTS || echo STACKDECK_REMOVED",
            stackdeck_hyperv::nerdctl_command(mgr.config(), &["inspect", &reference])
        ),
        false,
    )?;
    if verify.lines().any(|line| line.trim() == "STACKDECK_EXISTS") {
        return Err(ApiError::HyperV(stackdeck_hyperv::HyperVError::Ssh(
            format!("container {reference} still exists after remove"),
        )));
    }

    let ident = identifier.trim_start_matches('/');
    let _guard = state_lock()?;
    let mut state = read_state()?;
    state.containers.retain(|cid, meta| {
        !(cid.starts_with(ident) || meta.name == ident || meta.runtime_name == reference)
    });
    write_state(&state)?;
    if let Some(id) = matched_id {
        release_port_reservations(&id)?;
    }
    Ok(())
}

fn runtime_inspect_is_running(inspect: &str) -> bool {
    serde_json::from_str::<Value>(inspect)
        .ok()
        .and_then(|value| {
            value
                .as_array()
                .and_then(|items| items.first())
                .cloned()
                .or(Some(value))
        })
        .and_then(|value| value.pointer("/State/Running").and_then(Value::as_bool))
        .unwrap_or(false)
}

fn container_logs_text(identifier: &str, tail: u32) -> Result<String, ApiError> {
    let mgr = manager()?;
    let tail = tail.to_string();
    let reference = container_ref(identifier)?;
    let cmd =
        stackdeck_hyperv::nerdctl_command(mgr.config(), &["logs", "--tail", &tail, &reference]);
    Ok(map_nerdctl_not_found(
        mgr.ssh(&cmd, true),
        "container",
        identifier,
    )?)
}

fn inspect_raw(identifier: &str) -> Result<Value, ApiError> {
    if let Some((id, meta)) = state_match(identifier)? {
        if meta.runtime_id.is_none() {
            return Ok(synthetic_inspect(&id, &meta, "created", false));
        }
    }

    let reference = container_ref(identifier)?;
    let mgr = manager()?;
    let cmd = stackdeck_hyperv::nerdctl_command(mgr.config(), &["inspect", &reference]);
    let raw = map_nerdctl_not_found(mgr.ssh(&cmd, true), "container", identifier)?;
    if raw.trim().is_empty() {
        return Err(ApiError::NotFound(format!(
            "No such container: {identifier}"
        )));
    }
    let parsed: Value = serde_json::from_str(&raw)?;

    let fix_created = |mut item: Value| -> Value {
        if let Some(obj) = item.as_object_mut() {
            let created = parse_created(obj.get("Created"));
            obj.insert("Created".to_string(), json!(created));
        }
        item
    };

    if let Some(first) = parsed.as_array().and_then(|items| items.first()) {
        let mut item = fix_created(first.clone());
        if let Some((_id, meta)) = match_runtime_meta(first)?.or(state_match(identifier)?) {
            item = normalize_runtime_inspect(item, &meta);
        }
        return Ok(item);
    }

    let mut item = fix_created(parsed.clone());
    if let Some((_id, meta)) = match_runtime_meta(&parsed)?.or(state_match(identifier)?) {
        item = normalize_runtime_inspect(item, &meta);
    }
    Ok(item)
}

fn synthetic_inspect(id: &str, meta: &ContainerMeta, status: &str, running: bool) -> Value {
    let mut ports = serde_json::Map::new();
    for port in &meta.ports {
        if let Some((host_ip, host_port, container_port)) = stackdeck_hyperv::parse_port(port) {
            ports.insert(
                docker_port_key(port, container_port),
                json!([{ "HostIp": host_ip, "HostPort": host_port.to_string() }]),
            );
        }
    }
    let env: Vec<String> = meta.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    json!({
        "Id": id,
        "Name": format!("/{}", meta.name),
        "Image": meta.image,
        "Config": {
            "Image": meta.image,
            "Cmd": meta.cmd,
            "Env": env,
            "Labels": {},
            "ExposedPorts": ports.keys().map(|k| (k.clone(), json!({}))).collect::<serde_json::Map<_, _>>(),
        },
        "HostConfig": {
            "Binds": meta.binds,
            "PortBindings": ports.clone(),
            "NetworkMode": "default",
            "RestartPolicy": { "Name": "no", "MaximumRetryCount": 0 }
        },
        "NetworkSettings": { "Ports": ports },
        "State": { "Status": status, "Running": running, "ExitCode": 0 },
        "Created": meta.created,
    })
}

fn match_runtime_meta(item: &Value) -> Result<Option<(String, ContainerMeta)>, ApiError> {
    let runtime_id = item.get("Id").and_then(|v| v.as_str()).unwrap_or_default();
    let runtime_name = item
        .get("Name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim_start_matches('/');
    Ok(read_state()?.containers.into_iter().find(|(_id, meta)| {
        meta.runtime_id.as_deref() == Some(runtime_id) || meta.runtime_name == runtime_name
    }))
}

fn normalize_runtime_inspect(mut item: Value, meta: &ContainerMeta) -> Value {
    if let Some(obj) = item.as_object_mut() {
        obj.insert("Id".to_string(), Value::String(meta.id.clone()));
        obj.insert("Name".to_string(), Value::String(format!("/{}", meta.name)));

        let mut ports = serde_json::Map::new();
        for port in &meta.ports {
            if let Some((host_ip, host_port, container_port)) = stackdeck_hyperv::parse_port(port) {
                ports.insert(
                    docker_port_key(port, container_port),
                    json!([{ "HostIp": host_ip, "HostPort": host_port.to_string() }]),
                );
            }
        }

        if let Some(host_config) = obj.get_mut("HostConfig").and_then(|v| v.as_object_mut()) {
            if !host_config.contains_key("NetworkMode") {
                host_config.insert("NetworkMode".to_string(), json!("default"));
            }
            if !host_config.contains_key("RestartPolicy") {
                host_config.insert(
                    "RestartPolicy".to_string(),
                    json!({ "Name": "no", "MaximumRetryCount": 0 }),
                );
            }
            if !host_config.contains_key("PortBindings") {
                host_config.insert("PortBindings".to_string(), Value::Object(ports.clone()));
            }
        } else {
            obj.insert(
                "HostConfig".to_string(),
                json!({
                    "NetworkMode": "default",
                    "RestartPolicy": { "Name": "no", "MaximumRetryCount": 0 },
                    "Binds": meta.binds,
                    "PortBindings": ports.clone(),
                }),
            );
        }

        if let Some(network_settings) = obj
            .get_mut("NetworkSettings")
            .and_then(|v| v.as_object_mut())
        {
            if !network_settings.contains_key("Ports") {
                network_settings.insert("Ports".to_string(), Value::Object(ports));
            }
        } else {
            obj.insert("NetworkSettings".to_string(), json!({ "Ports": ports }));
        }
    }
    item
}

fn list_containers(include_all: bool) -> Result<Vec<Value>, ApiError> {
    let mut result = Vec::new();
    let mut seen = Vec::new();
    let state_snapshot = read_state()?;
    let can_return_local_created = include_all
        && !state_snapshot.containers.is_empty()
        && state_snapshot
            .containers
            .values()
            .all(|meta| meta.runtime_id.is_none());
    let mgr = match manager() {
        Ok(mgr) => mgr,
        Err(_err) if can_return_local_created => {
            return Ok(local_created_container_summaries(state_snapshot));
        }
        Err(err) => return Err(err),
    };
    let ps_args = if include_all {
        vec!["ps", "-a", "--quiet"]
    } else {
        vec!["ps", "--quiet"]
    };
    let raw = match mgr.ssh(
        &stackdeck_hyperv::nerdctl_command(mgr.config(), &ps_args),
        true,
    ) {
        Ok(raw) => raw,
        Err(_err) if can_return_local_created => {
            return Ok(local_created_container_summaries(state_snapshot));
        }
        Err(err) => return Err(err.into()),
    };
    for reference in raw.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let cmd = stackdeck_hyperv::nerdctl_command(mgr.config(), &["inspect", reference]);
        let raw = map_nerdctl_not_found(mgr.ssh(&cmd, true), "container", reference)?;
        if raw.trim().is_empty() {
            continue;
        }
        let parsed: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
        let Some(item) = parsed
            .as_array()
            .and_then(|items| items.first())
            .cloned()
            .or_else(|| {
                if parsed.is_object() {
                    Some(parsed)
                } else {
                    None
                }
            })
        else {
            continue;
        };
        if let Some((_id, meta)) = match_runtime_meta(&item)? {
            seen.push(meta.id.clone());
            result.push(container_summary(&normalize_runtime_inspect(item, &meta)));
        } else {
            if let Some(id) = item.get("Id").and_then(|v| v.as_str()) {
                seen.push(id.to_string());
            }
            result.push(container_summary(&item));
        }
    }
    if include_all {
        let state = read_state()?;
        let mut unseen_names = Vec::new();
        let mut unseen_meta = Vec::new();
        for (id, meta) in state.containers {
            if !seen.iter().any(|seen_id| {
                seen_id == &id
                    || seen_id == &meta.runtime_name
                    || meta.runtime_id.as_ref() == Some(seen_id)
            }) {
                unseen_names.push(meta.runtime_name.clone());
                unseen_meta.push((id, meta));
            }
        }

        let statuses = if !unseen_names.is_empty() {
            let names_refs: Vec<&str> = unseen_names.iter().map(|s| s.as_str()).collect();
            inspect_container_statuses(&mgr, &names_refs)?
        } else {
            std::collections::HashMap::new()
        };

        let mut phantom_ids = Vec::new();

        for (id, meta) in unseen_meta {
            let info = statuses.get(&meta.runtime_name);
            let exists = info
                .and_then(|v| v.get("exists"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let status = info
                .and_then(|v| v.get("status"))
                .and_then(|v| v.as_str())
                .unwrap_or("created");
            let running = info
                .and_then(|v| v.get("running"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if exists || meta.runtime_id.is_none() {
                result.push(container_summary(&synthetic_inspect(
                    &id, &meta, status, running,
                )));
            } else {
                phantom_ids.push(id.clone());
            }
        }

        if !phantom_ids.is_empty() {
            let _guard = state_lock()?;
            let mut state = read_state()?;
            let mut changed = false;
            for id in phantom_ids {
                if state.containers.remove(&id).is_some() {
                    changed = true;
                }
            }
            if changed {
                let _ = write_state(&state);
            }
        }
    }
    Ok(result)
}

fn local_created_container_summaries(state: ApiState) -> Vec<Value> {
    state
        .containers
        .into_iter()
        .map(|(id, meta)| container_summary(&synthetic_inspect(&id, &meta, "created", false)))
        .collect()
}

fn inspect_container_statuses(
    mgr: &HyperVManager,
    names: &[&str],
) -> Result<HashMap<String, Value>, ApiError> {
    let mut result = HashMap::new();
    for name in names {
        let cmd = stackdeck_hyperv::nerdctl_command(mgr.config(), &["inspect", name]);
        let raw = match map_nerdctl_not_found(mgr.ssh(&cmd, true), "container", name) {
            Ok(raw) => raw,
            Err(ApiError::NotFound(_)) => {
                result.insert(
                    (*name).to_string(),
                    json!({"exists": false, "status": "stopped", "running": false}),
                );
                continue;
            }
            Err(err) => return Err(err),
        };
        let parsed: Value = serde_json::from_str(&raw)?;
        let Some(item) = parsed
            .as_array()
            .and_then(|items| items.first())
            .or_else(|| parsed.as_object().map(|_| &parsed))
        else {
            result.insert(
                (*name).to_string(),
                json!({"exists": true, "status": "unknown", "running": false}),
            );
            continue;
        };
        let state = item.get("State").cloned().unwrap_or_default();
        let running = state
            .get("Running")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let mut status = state
            .get("Status")
            .and_then(|v| v.as_str())
            .unwrap_or(if running { "running" } else { "stopped" })
            .to_string();
        if let Some(health_status) = state.pointer("/Health/Status").and_then(|v| v.as_str()) {
            if health_status == "unhealthy" {
                status = "unhealthy".to_string();
            }
        }
        let exit_code = state.get("ExitCode").and_then(|v| v.as_i64()).unwrap_or(0);
        result.insert(
            (*name).to_string(),
            json!({
                "exists": true,
                "status": status,
                "running": running,
                "exit_code": exit_code,
            }),
        );
    }
    Ok(result)
}

fn container_summary(item: &Value) -> Value {
    let state = item.get("State").and_then(|v| v.as_object());
    let config = item.get("Config").and_then(|v| v.as_object());
    let name = item
        .get("Name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim_start_matches('/');
    let id = item
        .get("Id")
        .or_else(|| item.get("ID"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let image = config
        .and_then(|c| c.get("Image"))
        .or_else(|| item.get("Image"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let status = state
        .and_then(|s| s.get("Status"))
        .and_then(|v| v.as_str())
        .unwrap_or("created");
    let cmd = config
        .and_then(|c| c.get("Cmd"))
        .map(command_string)
        .unwrap_or_default();
    let network_mode = item
        .pointer("/HostConfig/NetworkMode")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    json!({
        "Id": id,
        "Names": if name.is_empty() { json!([]) } else { json!([format!("/{name}")]) },
        "Image": image,
        "ImageID": item.get("ImageID").or_else(|| item.get("Image")).cloned().unwrap_or(Value::String(String::new())),
        "Command": cmd,
        "Created": parse_created(item.get("Created")),
        "Ports": summary_ports(item.pointer("/NetworkSettings/Ports")),
        "Labels": config.and_then(|c| c.get("Labels")).cloned().unwrap_or_else(|| json!({})),
        "State": status,
        "Status": status,
        "HostConfig": {
            "NetworkMode": network_mode
        },
        "NetworkSettings": {
            "Networks": {
                network_mode: {
                    "NetworkID": "",
                    "EndpointID": "",
                    "Gateway": "",
                    "IPAddress": "",
                    "IPPrefixLen": 0,
                    "IPv6Gateway": "",
                    "GlobalIPv6Address": "",
                    "GlobalIPv6PrefixLen": 0,
                    "MacAddress": ""
                }
            }
        }
    })
}

fn command_string(value: &Value) -> String {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

fn summary_ports(ports: Option<&Value>) -> Value {
    let mut result = Vec::new();
    let Some(map) = ports.and_then(|v| v.as_object()) else {
        return Value::Array(result);
    };
    for (private, bindings) in map {
        let (private_port, proto) = private
            .split_once('/')
            .map(|(port, proto)| (port, proto))
            .unwrap_or((private.as_str(), "tcp"));
        let base_private = private_port.parse::<u16>().unwrap_or(0);
        if let Some(items) = bindings.as_array() {
            for binding in items {
                let mut row = json!({ "PrivatePort": base_private, "Type": proto });
                if let Some(obj) = binding.as_object() {
                    if let Some(host_port) = obj
                        .get("HostPort")
                        .and_then(|v| v.as_str())
                        .and_then(|v| v.parse::<u16>().ok())
                    {
                        row["PublicPort"] = json!(host_port);
                    }
                    row["IP"] = obj
                        .get("HostIp")
                        .or_else(|| obj.get("HostIP"))
                        .cloned()
                        .unwrap_or_else(|| json!("0.0.0.0"));
                }
                result.push(row);
            }
        } else {
            result.push(json!({ "PrivatePort": base_private, "Type": proto }));
        }
    }
    Value::Array(result)
}

fn list_networks() -> Result<Vec<Value>, ApiError> {
    let mgr = manager()?;
    let raw = mgr.ssh(
        &stackdeck_hyperv::nerdctl_command(
            mgr.config(),
            &["network", "ls", "--format", "{{json .}}"],
        ),
        true,
    )?;
    Ok(raw.lines().filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok()).map(|row| {
        let name = row.get("Name").or_else(|| row.get("NetworkName")).and_then(|v| v.as_str()).unwrap_or_default();
        json!({
            "Name": name,
            "Id": row.get("ID").or_else(|| row.get("Id")).and_then(|v| v.as_str()).unwrap_or(name),
            "Driver": row.get("Driver").and_then(|v| v.as_str()).unwrap_or("bridge"),
            "Scope": "local",
            "Internal": false,
            "Attachable": true,
            "Containers": {},
            "Options": {},
            "Labels": {},
        })
    }).collect())
}

fn create_network(name: &str) -> Result<Value, ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::BadRequest("network Name is required".into()));
    }
    manager()?.network_create(name)?;
    Ok(json!({"Id": name, "Warning": ""}))
}

fn inspect_network(name: &str) -> Result<Value, ApiError> {
    let mgr = manager()?;
    let out = map_nerdctl_not_found(
        mgr.ssh(
            &stackdeck_hyperv::nerdctl_command(mgr.config(), &["network", "inspect", name]),
            true,
        ),
        "network",
        name,
    )?;
    let parsed: Value = serde_json::from_str(&out)?;
    if let Some(first) = parsed.as_array().and_then(|items| items.first()) {
        return Ok(first.clone());
    }
    Err(ApiError::NotFound(format!("No such network: {name}")))
}

fn remove_network(name: &str) -> Result<(), ApiError> {
    let mgr = manager()?;
    map_nerdctl_not_found(
        mgr.ssh(
            &stackdeck_hyperv::nerdctl_command(mgr.config(), &["network", "rm", name]),
            true,
        ),
        "network",
        name,
    )?;
    Ok(())
}

fn list_volumes() -> Result<Value, ApiError> {
    let mgr = manager()?;
    let raw = mgr.ssh(
        &stackdeck_hyperv::nerdctl_command(
            mgr.config(),
            &["volume", "ls", "--format", "{{json .}}"],
        ),
        true,
    )?;
    let volumes: Vec<Value> = raw.lines().filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok()).map(|row| {
        let name = row.get("Name").and_then(|v| v.as_str()).unwrap_or_default();
        json!({"Name": name, "Driver": "local", "Mountpoint": "", "Labels": {}, "Options": {}, "Scope": "local"})
    }).collect();
    Ok(json!({"Volumes": volumes, "Warnings": null}))
}

fn create_volume(name: &str) -> Result<Value, ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::BadRequest("volume Name is required".into()));
    }
    manager()?.volume_create(name)?;
    Ok(
        json!({"Name": name, "Driver": "local", "Mountpoint": "", "Labels": {}, "Options": {}, "Scope": "local"}),
    )
}

fn inspect_volume(name: &str) -> Result<Value, ApiError> {
    let mgr = manager()?;
    let out = map_nerdctl_not_found(
        mgr.ssh(
            &stackdeck_hyperv::nerdctl_command(mgr.config(), &["volume", "inspect", name]),
            true,
        ),
        "volume",
        name,
    )?;
    let parsed: Value = serde_json::from_str(&out)?;
    if let Some(first) = parsed.as_array().and_then(|items| items.first()) {
        return Ok(first.clone());
    }
    Err(ApiError::NotFound(format!("No such volume: {name}")))
}

fn remove_volume(name: &str) -> Result<(), ApiError> {
    let mgr = manager()?;
    map_nerdctl_not_found(
        mgr.ssh(
            &stackdeck_hyperv::nerdctl_command(mgr.config(), &["volume", "rm", name]),
            true,
        ),
        "volume",
        name,
    )?;
    Ok(())
}

fn list_images() -> Result<Vec<Value>, ApiError> {
    let mgr = manager()?;
    let raw = mgr.ssh(
        &stackdeck_hyperv::nerdctl_command(mgr.config(), &["images", "--format", "{{json .}}"]),
        false,
    )?;
    Ok(raw
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .map(image_summary)
        .collect())
}

fn image_summary(row: Value) -> Value {
    let repo = row
        .get("Repository")
        .or_else(|| row.get("Name"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let tag = row.get("Tag").and_then(|v| v.as_str()).unwrap_or("latest");
    let image_id = row
        .get("ID")
        .or_else(|| row.get("ImageID"))
        .or_else(|| row.get("Digest"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let size = row.get("Size").and_then(|v| v.as_str()).unwrap_or_default();
    json!({
        "Id": image_id,
        "RepoTags": if repo.is_empty() { json!([]) } else { json!([format!("{repo}:{tag}")]) },
        "RepoDigests": [],
        "Created": 0,
        "Size": parse_size(size),
        "VirtualSize": parse_size(size),
        "Labels": {},
        "Containers": -1,
        "SharedSize": -1,
        "CreatedAt": row.get("CreatedAt").or_else(|| row.get("CreatedSince")).cloned().unwrap_or(Value::String(String::new())),
    })
}

fn parse_size(value: &str) -> u64 {
    let text = value.trim().to_ascii_uppercase().replace(' ', "");
    let number: String = text
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let Ok(number) = number.parse::<f64>() else {
        return 0;
    };
    let unit = text.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
    let scale = if unit.starts_with('K') {
        1024_f64
    } else if unit.starts_with('M') {
        1024_f64.powi(2)
    } else if unit.starts_with('G') {
        1024_f64.powi(3)
    } else if unit.starts_with('T') {
        1024_f64.powi(4)
    } else {
        1_f64
    };
    (number * scale) as u64
}

fn inspect_image(name: &str) -> Result<Value, ApiError> {
    let mgr = manager()?;
    let cmd = format!(
        "{} 2>/dev/null || true",
        stackdeck_hyperv::nerdctl_command(mgr.config(), &["image", "inspect", name])
    );
    let raw = mgr.ssh(&cmd, false)?;
    if raw.trim().is_empty() {
        return Err(ApiError::NotFound(format!("No such image: {name}")));
    }
    let parsed: Value = serde_json::from_str(&raw)?;
    if let Some(first) = parsed.as_array().and_then(|items| items.first()) {
        return Ok(first.clone());
    }
    Ok(parsed)
}

fn remove_image(name: &str) -> Result<Value, ApiError> {
    let mgr = manager()?;
    mgr.image_remove(&[name], false)?;
    Ok(json!([{ "Deleted": name }]))
}

fn docker_log_frames(text: &str) -> Vec<u8> {
    let mut framed = Vec::new();
    for chunk in text.as_bytes().chunks(u32::MAX as usize) {
        framed.push(1);
        framed.extend_from_slice(&[0, 0, 0]);
        framed.extend_from_slice(&(chunk.len() as u32).to_be_bytes());
        framed.extend_from_slice(chunk);
    }
    framed
}

fn text_response(status: StatusCode, text: &str) -> Response<Body> {
    response(
        status,
        "text/plain; charset=utf-8",
        Body::from(text.to_string()),
    )
}

fn empty_response(status: StatusCode) -> Response<Body> {
    response(status, "application/json", Body::empty())
}

fn response(status: StatusCode, content_type: &str, body: Body) -> Response<Body> {
    let mut response = Response::new(body);
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body;
    use axum::http::{Method, Request};
    use std::sync::{LazyLock, Mutex, MutexGuard};
    use tower::ServiceExt;

    static TEST_ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    const TEST_TOKEN: &str = "test-token";

    #[test]
    fn registry_auth_is_decoded_and_normalized_without_shell_values() {
        let mut headers = HeaderMap::new();
        let encoded = base64::engine::general_purpose::STANDARD.encode(
            r#"{"username":"octo","password":"secret","serveraddress":"https://index.docker.io/v1/"}"#,
        );
        headers.insert("X-Registry-Auth", HeaderValue::from_str(&encoded).unwrap());

        let auth = registry_auth(&headers, "library/alpine:latest")
            .unwrap()
            .unwrap();
        assert_eq!(auth.username, "octo");
        assert_eq!(auth.server_address, "docker.io");
        assert_eq!(auth.password, "secret");

        let token = base64::engine::general_purpose::STANDARD
            .encode(r#"{"identitytoken":"registry-token"}"#);
        headers.insert("X-Registry-Auth", HeaderValue::from_str(&token).unwrap());
        let token_auth = registry_auth(&headers, "registry.example/team/app")
            .unwrap()
            .unwrap();
        assert_eq!(token_auth.username, "oauth2accesstoken");
        assert_eq!(token_auth.server_address, "registry.example");

        headers.insert("X-Registry-Auth", HeaderValue::from_static("not-base64"));
        assert!(registry_auth(&headers, "alpine").is_err());
    }

    #[test]
    fn pull_progress_is_newline_delimited_docker_status_json() {
        let mut decoder = DockerPullProgressDecoder::new("alpine");
        assert!(decoder.push(b"pulling la").is_empty());
        let events = decoder.push(b"yer\ncomplete\n");
        let lines: Vec<Value> = events
            .into_iter()
            .map(|event| serde_json::from_slice(&event).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["id"], "alpine");
        assert_eq!(lines[0]["status"], "pulling layer");
        assert_eq!(lines[1]["status"], "complete");
    }

    fn available_loopback_port() -> u16 {
        let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    fn authed(builder: axum::http::request::Builder) -> axum::http::request::Builder {
        builder.header(header::AUTHORIZATION, "Bearer test-token")
    }

    struct IsolatedHome {
        path: PathBuf,
        old_userprofile: Option<String>,
        old_home: Option<String>,
        old_token: Option<String>,
        old_require_token: Option<String>,
        old_allow_binds: Option<String>,
        old_bind_roots: Option<String>,
        old_config_dir: Option<String>,
        _guard: MutexGuard<'static, ()>,
    }

    impl Drop for IsolatedHome {
        fn drop(&mut self) {
            if let Some(value) = &self.old_userprofile {
                std::env::set_var("USERPROFILE", value);
            } else {
                std::env::remove_var("USERPROFILE");
            }
            if let Some(value) = &self.old_home {
                std::env::set_var("HOME", value);
            } else {
                std::env::remove_var("HOME");
            }
            restore_env("STACKDECK_DOCKER_API_TOKEN", &self.old_token);
            restore_env(
                "STACKDECK_DOCKER_API_REQUIRE_TOKEN",
                &self.old_require_token,
            );
            restore_env("STACKDECK_DOCKER_API_ALLOW_BINDS", &self.old_allow_binds);
            restore_env("STACKDECK_DOCKER_API_BIND_ROOTS", &self.old_bind_roots);
            restore_env(
                stackdeck_types::STACKDECK_CONFIG_DIR_ENV,
                &self.old_config_dir,
            );
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn restore_env(name: &str, value: &Option<String>) {
        if let Some(value) = value {
            std::env::set_var(name, value);
        } else {
            std::env::remove_var(name);
        }
    }

    fn isolated_home() -> IsolatedHome {
        let guard = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let path = std::env::temp_dir().join(format!("stackdeck-api-test-{}", new_id()));
        std::fs::create_dir_all(&path).unwrap();
        let old_userprofile = std::env::var("USERPROFILE").ok();
        let old_home = std::env::var("HOME").ok();
        let old_token = std::env::var("STACKDECK_DOCKER_API_TOKEN").ok();
        let old_require_token = std::env::var("STACKDECK_DOCKER_API_REQUIRE_TOKEN").ok();
        let old_allow_binds = std::env::var("STACKDECK_DOCKER_API_ALLOW_BINDS").ok();
        let old_bind_roots = std::env::var("STACKDECK_DOCKER_API_BIND_ROOTS").ok();
        let old_config_dir = std::env::var(stackdeck_types::STACKDECK_CONFIG_DIR_ENV).ok();
        std::env::set_var("USERPROFILE", &path);
        std::env::remove_var("HOME");
        std::env::set_var(stackdeck_types::STACKDECK_CONFIG_DIR_ENV, &path);
        std::env::set_var("STACKDECK_DOCKER_API_TOKEN", TEST_TOKEN);
        std::env::remove_var("STACKDECK_DOCKER_API_REQUIRE_TOKEN");
        std::env::remove_var("STACKDECK_DOCKER_API_ALLOW_BINDS");
        std::env::remove_var("STACKDECK_DOCKER_API_BIND_ROOTS");
        IsolatedHome {
            path,
            old_userprofile,
            old_home,
            old_token,
            old_require_token,
            old_allow_binds,
            old_bind_roots,
            old_config_dir,
            _guard: guard,
        }
    }

    async fn json_body(response: axum::response::Response) -> Value {
        let bytes = body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn parses_docker_size_units() {
        assert_eq!(parse_size("1KB"), 1024);
        assert_eq!(parse_size("1.5 MB"), 1_572_864);
        assert_eq!(parse_size("2GB"), 2_147_483_648);
    }

    #[test]
    fn normalizes_docker_env_list() {
        let env = normalize_env(Some(&json!(["A=1", "B=two", "ignored"])));
        assert_eq!(env.get("A"), Some(&"1".to_string()));
        assert_eq!(env.get("B"), Some(&"two".to_string()));
        assert!(!env.contains_key("ignored"));
    }

    #[test]
    fn extracts_port_bindings() {
        let host_port = available_loopback_port();
        let mut reserved = HashSet::new();
        let mut listeners = Vec::new();
        let ports = port_bindings(
            None,
            Some(&json!({ "PortBindings": { "80/tcp": [{ "HostPort": host_port.to_string() }] } })),
            &mut reserved,
            &mut listeners,
        )
        .unwrap();
        assert_eq!(ports, vec![format!("127.0.0.1:{host_port}:80")]);
        assert!(StdTcpListener::bind(("127.0.0.1", host_port)).is_err());
    }

    #[test]
    fn preserves_udp_and_explicit_host_ip() {
        let host_port = available_loopback_port();
        let mut reserved = HashSet::new();
        let mut listeners = Vec::new();
        let ports = port_bindings(
            None,
            Some(&json!({
                "PortBindings": {
                    "53/udp": [{ "HostIp": "0.0.0.0", "HostPort": host_port.to_string() }]
                }
            })),
            &mut reserved,
            &mut listeners,
        )
        .unwrap();
        assert_eq!(ports, vec![format!("0.0.0.0:{host_port}:53/udp")]);
        assert_eq!(docker_port_key(&ports[0], 53), "53/udp");
    }

    #[test]
    fn docker_log_frames_have_stream_header_and_big_endian_length() {
        let framed = docker_log_frames("one\ntwo");
        assert_eq!(&framed[0..4], &[1, 0, 0, 0]);
        assert_eq!(u32::from_be_bytes(framed[4..8].try_into().unwrap()), 7);
        assert_eq!(&framed[8..15], b"one\ntwo");
    }

    #[test]
    fn exposed_ports_do_not_publish_without_publish_all() {
        let mut reserved = HashSet::new();
        let mut listeners = Vec::new();
        let ports = port_bindings(
            Some(&json!({ "5432/tcp": {} })),
            Some(&json!({})),
            &mut reserved,
            &mut listeners,
        )
        .unwrap();
        assert!(ports.is_empty());
    }

    #[test]
    fn publish_all_uses_exposed_ports() {
        let mut reserved = HashSet::new();
        let mut listeners = Vec::new();
        let ports = port_bindings(
            Some(&json!({ "5432/tcp": {} })),
            Some(&json!({ "PublishAllPorts": true })),
            &mut reserved,
            &mut listeners,
        )
        .unwrap();
        assert_eq!(ports.len(), 1);
        let parts: Vec<&str> = ports[0].split(':').collect();
        assert_eq!(parts[0], "127.0.0.1");
        assert!(parts[1].parse::<u16>().unwrap() > 0);
        assert_eq!(parts[2], "5432");
        assert!(StdTcpListener::bind(("127.0.0.1", parts[1].parse::<u16>().unwrap())).is_err());
    }

    #[test]
    fn one_create_request_never_reuses_an_ephemeral_port() {
        let mut reserved = HashSet::new();
        let mut listeners = Vec::new();
        let ports = port_bindings(
            None,
            Some(&json!({
                "PortBindings": {
                    "80/tcp": [{ "HostPort": "" }],
                    "81/tcp": [{ "HostPort": "" }]
                }
            })),
            &mut reserved,
            &mut listeners,
        )
        .unwrap();
        let allocated: HashSet<u16> = ports
            .iter()
            .filter_map(|port| stackdeck_hyperv::parse_port(port).map(|(_, host, _)| host))
            .collect();
        assert_eq!(allocated.len(), 2);
        assert_eq!(listeners.len(), 2);
    }

    #[test]
    fn image_json_route_name_strips_action_suffix() {
        assert_eq!(
            docker_image_json_name("ghcr.io/org/app:tag/json").unwrap(),
            "ghcr.io/org/app:tag"
        );
        assert!(docker_image_json_name("search").is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn http_ping_and_version_routes_work() {
        let _home = isolated_home();
        let ping = app()
            .oneshot(
                authed(Request::builder())
                    .method(Method::GET)
                    .uri("/v1.43/_ping")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ping.status(), StatusCode::OK);
        let body = body::to_bytes(ping.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"OK");

        let version = app()
            .oneshot(
                authed(Request::builder())
                    .method(Method::GET)
                    .uri("/v1.43/version")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(version.status(), StatusCode::OK);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unsupported_image_get_does_not_reach_backend() {
        let _home = isolated_home();
        let response = app()
            .oneshot(
                authed(Request::builder())
                    .method(Method::GET)
                    .uri("/v1.43/images/search")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn container_create_missing_image_returns_bad_request() {
        let _home = isolated_home();
        let response = app()
            .oneshot(
                authed(Request::builder())
                    .method(Method::POST)
                    .uri("/v1.43/containers/create")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn container_create_rejects_udp_publish_before_runtime_mutation() {
        let _home = isolated_home();
        let response = app()
            .oneshot(
                authed(Request::builder())
                    .method(Method::POST)
                    .uri("/v1.43/containers/create?name=dns")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"Image":"alpine","HostConfig":{"PortBindings":{"53/udp":[{"HostPort":"5353"}]}}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn auth_token_is_enforced_by_default() {
        let _home = isolated_home();
        std::env::set_var("STACKDECK_DOCKER_API_TOKEN", "secret");

        let unauthorized = app()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/v1.43/_ping")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let authorized = app()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/v1.43/_ping")
                    .header(header::AUTHORIZATION, "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(authorized.status(), StatusCode::OK);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn container_create_rejects_binds_by_default() {
        let _home = isolated_home();
        let response = app()
            .oneshot(
                authed(Request::builder())
                    .method(Method::POST)
                    .uri("/v1.43/containers/create?name=web")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"Image":"alpine","HostConfig":{"Binds":["C:\\secret:/data"]}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn container_create_rejects_unsupported_active_fields() {
        let _home = isolated_home();
        for body in [
            r#"{"Image":"alpine","Entrypoint":["/bin/sh"]}"#,
            r#"{"Image":"alpine","HostConfig":{"Privileged":true}}"#,
            r#"{"Image":"alpine","HostConfig":{"RestartPolicy":{"Name":"always"}}}"#,
            r#"{"Image":"alpine","HostConfig":{"NetworkMode":"host"}}"#,
        ] {
            let response = app()
                .oneshot(
                    authed(Request::builder())
                        .method(Method::POST)
                        .uri("/v1.43/containers/create")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
    }

    #[test]
    fn state_match_rejects_ambiguous_prefixes() {
        let _home = isolated_home();
        let mut state = ApiState::default();
        for (id, name) in [
            ("aaaaaaaaaaaa11111111111111111111", "web"),
            ("aaaaaaaaaaaa22222222222222222222", "worker"),
        ] {
            state.containers.insert(
                id.to_string(),
                ContainerMeta {
                    id: id.to_string(),
                    name: name.to_string(),
                    runtime_name: format!("docker-api-{name}"),
                    runtime_id: None,
                    runtime_pending: false,
                    image: "alpine".to_string(),
                    cmd: None,
                    env: HashMap::new(),
                    ports: Vec::new(),
                    binds: Vec::new(),
                    created: 1,
                    docker_spec: json!({"Image":"alpine"}),
                },
            );
        }
        write_state(&state).unwrap();

        let err = state_match("aaaaaaaaaaaa").unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
        assert!(err.to_string().contains("ambiguous container reference"));
    }

    #[test]
    fn state_write_replaces_existing_state_file() {
        let _home = isolated_home();
        let mut state = ApiState::default();
        state.containers.insert(
            "first".to_string(),
            ContainerMeta {
                id: "first".to_string(),
                name: "first".to_string(),
                runtime_name: "docker-api-first".to_string(),
                runtime_id: None,
                runtime_pending: false,
                image: "alpine".to_string(),
                cmd: None,
                env: HashMap::new(),
                ports: Vec::new(),
                binds: Vec::new(),
                created: 1,
                docker_spec: json!({"Image":"alpine"}),
            },
        );
        write_state(&state).unwrap();

        state.containers.clear();
        write_state(&state).unwrap();

        assert!(read_state().unwrap().containers.is_empty());
    }

    #[test]
    fn corrupt_state_is_not_silently_replaced_with_defaults() {
        let _home = isolated_home();
        let path = state_file();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{ definitely not valid json").unwrap();

        let err = read_state().unwrap_err().to_string();
        assert!(err.contains("refusing to replace corrupt"));
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "{ definitely not valid json"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn http_container_state_routes_preserve_docker_name_and_remove_with_204() {
        let _home = isolated_home();
        let create = app()
            .oneshot(
                authed(Request::builder())
                    .method(Method::POST)
                    .uri("/v1.43/containers/create?name=web")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"Image":"alpine","Cmd":["sleep","60"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create.status(), StatusCode::CREATED);
        let created = json_body(create).await;
        let id = created["Id"].as_str().unwrap();

        let inspect = app()
            .oneshot(
                authed(Request::builder())
                    .method(Method::GET)
                    .uri(format!("/v1.43/containers/{id}/json"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(inspect.status(), StatusCode::OK);
        let inspected = json_body(inspect).await;
        assert_eq!(inspected["Name"], "/web");

        let remove = app()
            .oneshot(
                authed(Request::builder())
                    .method(Method::DELETE)
                    .uri(format!("/v1.43/containers/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(remove.status(), StatusCode::NO_CONTENT);
        let body = body::to_bytes(remove.into_body(), 1024).await.unwrap();
        assert!(body.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn http_container_create_rejects_duplicate_names() {
        let _home = isolated_home();
        for expected_status in [StatusCode::CREATED, StatusCode::CONFLICT] {
            let response = app()
                .oneshot(
                    authed(Request::builder())
                        .method(Method::POST)
                        .uri("/v1.43/containers/create?name=web")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(r#"{"Image":"alpine"}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected_status);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn http_containers_json_honors_all_for_created_state() {
        let _home = isolated_home();
        let create = app()
            .oneshot(
                authed(Request::builder())
                    .method(Method::POST)
                    .uri("/v1.43/containers/create?name=web")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"Image":"alpine"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create.status(), StatusCode::CREATED);

        let default_list = app()
            .oneshot(
                authed(Request::builder())
                    .method(Method::GET)
                    .uri("/v1.43/containers/json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(default_list.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let all_list = app()
            .oneshot(
                authed(Request::builder())
                    .method(Method::GET)
                    .uri("/v1.43/containers/json?all=1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(all_list.status(), StatusCode::OK);
        let containers = json_body(all_list).await;
        assert_eq!(containers.as_array().unwrap().len(), 1);
        assert_eq!(containers[0]["Names"][0], "/web");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runtime_collection_routes_propagate_missing_backend_config() {
        let _home = isolated_home();
        for uri in [
            "/v1.43/containers/json",
            "/v1.43/volumes",
            "/v1.43/networks",
        ] {
            let response = app()
                .oneshot(
                    authed(Request::builder())
                        .method(Method::GET)
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn volume_and_network_create_reject_unsupported_fields_before_backend() {
        let _home = isolated_home();
        for (uri, body) in [
            (
                "/v1.43/volumes/create",
                r#"{"Name":"data","Driver":"local","Labels":{"team":"dev"}}"#,
            ),
            (
                "/v1.43/networks/create",
                r#"{"Name":"mesh","Driver":"bridge","Internal":true}"#,
            ),
        ] {
            let response = app()
                .oneshot(
                    authed(Request::builder())
                        .method(Method::POST)
                        .uri(uri)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
    }
}
