//! Docker Engine API shim for StackDeck.
//!
//! Provides a small Docker Engine-compatible HTTP surface for local tooling.

use axum::{
    body::Body,
    extract::{Path, Query},
    http::{header, HeaderValue, Request, Response, StatusCode},
    middleware::{self, Next},
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use pystack_hyperv::{HyperVManager, HyperVService};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path as StdPath, PathBuf},
    sync::{LazyLock, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::net::TcpListener;

pub const DEFAULT_HOST: &str = "127.0.0.1";
pub const DEFAULT_PORT: u16 = 23750;
pub const API_VERSION: &str = "1.43";
pub const MIN_API_VERSION: &str = "1.24";
const PROJECT: &str = "docker-api";
static STATE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

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
    HyperV(#[from] pystack_hyperv::HyperVError),
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
    if auth_token().is_none() {
        return Err(ApiError::BadRequest(
            "STACKDECK_DOCKER_API_TOKEN must be set to start the API".to_string(),
        ));
    }
    let listener = TcpListener::bind(addr).await?;
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
    let tail = query.tail;
    let logs = blocking(move || container_logs_text(&id, tail)).await?;
    Ok(text_response(StatusCode::OK, &logs))
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
    Query(query): Query<ImageCreateQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let name = query.from_image;
    Ok(Json(blocking(move || pull_image(&name)).await?))
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
    pystack_types::registry_dir().join("docker_api_containers.json")
}

fn read_state() -> ApiState {
    let path = state_file();
    let Ok(text) = std::fs::read_to_string(path) else {
        return ApiState::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn write_state(state: &ApiState) -> Result<(), ApiError> {
    let path = state_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("json.{}.tmp", new_id()));
    std::fs::write(&tmp, serde_json::to_string_pretty(state)?)?;
    std::fs::rename(tmp, path)?;
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

fn port_bindings(exposed_ports: Option<&Value>, host_config: Option<&Value>) -> Vec<String> {
    let mut ports = Vec::new();
    if let Some(bindings) = host_config
        .and_then(|v| v.get("PortBindings"))
        .and_then(|v| v.as_object())
    {
        for (container_port, entries) in bindings {
            let target = container_port.split('/').next().unwrap_or(container_port);
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
                if host_port.is_empty() {
                    ports.push(target.to_string());
                } else {
                    ports.push(format!("{host_port}:{target}"));
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
                        ports.push(format!("{}:{}", allocate_host_port(), target));
                    }
                }
            }
        }
    }
    ports
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

fn allocate_host_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|listener| listener.local_addr().ok())
        .map(|addr| addr.port())
        .unwrap_or(0)
}

fn auth_token() -> Option<String> {
    std::env::var("STACKDECK_DOCKER_API_TOKEN")
        .ok()
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
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

fn state_match(identifier: &str) -> Option<(String, ContainerMeta)> {
    let ident = identifier.trim_start_matches('/');
    read_state().containers.into_iter().find(|(cid, meta)| {
        cid.starts_with(ident) || meta.name == ident || meta.runtime_name == ident
    })
}

fn container_ref(identifier: &str) -> String {
    state_match(identifier)
        .map(|(_, meta)| meta.runtime_name)
        .unwrap_or_else(|| identifier.trim_start_matches('/').to_string())
}

fn service_from_meta(meta: &ContainerMeta) -> HyperVService {
    HyperVService {
        project: PROJECT.to_string(),
        name: slug(&meta.name),
        root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        image: meta.image.clone(),
        build: None,
        command: meta.cmd.clone(),
        env: meta.env.clone(),
        ports: meta.ports.clone(),
        volumes: meta.binds.clone(),
        networks: Vec::new(),
        restart: "no".to_string(),
        secrets: Vec::new(),
        configs: Vec::new(),
        secret_resources: std::collections::HashMap::new(),
        config_resources: std::collections::HashMap::new(),
        healthcheck: None,
    }
}

fn create_container(spec: Value, name: &str) -> Result<Value, ApiError> {
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
    let meta = ContainerMeta {
        id: id.clone(),
        name: docker_name.clone(),
        runtime_name: pystack_hyperv::container_name(PROJECT, &service_name),
        runtime_id: None,
        image,
        cmd: normalize_cmd(spec.get("Cmd")),
        env: normalize_env(spec.get("Env")),
        ports: port_bindings(spec.get("ExposedPorts"), spec.get("HostConfig")),
        binds,
        created: now_unix(),
        docker_spec: spec,
    };

    let _guard = STATE_LOCK.lock().unwrap();
    let mut state = read_state();
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
    Ok(json!({ "Id": id, "Warnings": [] }))
}

fn start_container(identifier: &str) -> Result<(), ApiError> {
    if let Some((_id, meta)) = state_match(identifier) {
        let output = manager()?.start_service(&service_from_meta(&meta), false)?;
        let runtime_id = output.lines().last().unwrap_or_default().trim();
        if !runtime_id.is_empty() {
            let _guard = STATE_LOCK.lock().unwrap();
            let mut state = read_state();
            if let Some(stored) = state.containers.get_mut(&meta.id) {
                stored.runtime_id = Some(runtime_id.to_string());
            }
            write_state(&state)?;
        }
        return Ok(());
    }
    let mgr = manager()?;
    let cmd = pystack_hyperv::nerdctl_command(mgr.config(), &["start", &container_ref(identifier)]);
    mgr.ssh(&cmd, true)?;
    Ok(())
}

fn stop_container(identifier: &str) -> Result<(), ApiError> {
    let mgr = manager()?;
    let cmd = pystack_hyperv::nerdctl_command(mgr.config(), &["stop", &container_ref(identifier)]);
    mgr.ssh(&cmd, false)?;
    Ok(())
}

fn remove_container(identifier: &str) -> Result<(), ApiError> {
    if let Some((id, meta)) = state_match(identifier) {
        if meta.runtime_id.is_none() {
            let _guard = STATE_LOCK.lock().unwrap();
            let mut state = read_state();
            state.containers.remove(&id);
            write_state(&state)?;
            return Ok(());
        }
    }

    let reference = container_ref(identifier);
    let mgr = manager()?;
    let cmd = pystack_hyperv::nerdctl_command(mgr.config(), &["rm", "-f", &reference]);
    mgr.ssh(&cmd, false)?;

    let ident = identifier.trim_start_matches('/');
    let _guard = STATE_LOCK.lock().unwrap();
    let mut state = read_state();
    state.containers.retain(|cid, meta| {
        !(cid.starts_with(ident) || meta.name == ident || meta.runtime_name == reference)
    });
    write_state(&state)?;
    Ok(())
}

fn container_logs_text(identifier: &str, tail: u32) -> Result<String, ApiError> {
    let mgr = manager()?;
    Ok(mgr.logs(&container_ref(identifier), tail)?)
}

fn inspect_raw(identifier: &str) -> Result<Value, ApiError> {
    if let Some((id, meta)) = state_match(identifier) {
        if meta.runtime_id.is_none() {
            return Ok(synthetic_inspect(&id, &meta, "created", false));
        }
    }

    let reference = container_ref(identifier);
    let mgr = manager()?;
    let cmd = format!(
        "{} 2>/dev/null || true",
        pystack_hyperv::nerdctl_command(mgr.config(), &["inspect", &reference])
    );
    let raw = mgr.ssh(&cmd, false)?;
    if raw.trim().is_empty() {
        if let Some((id, meta)) = state_match(identifier) {
            return Ok(synthetic_inspect(&id, &meta, "exited", false));
        }
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
        if let Some((_id, meta)) = match_runtime_meta(first).or_else(|| state_match(identifier)) {
            item = normalize_runtime_inspect(item, &meta);
        }
        return Ok(item);
    }

    let mut item = fix_created(parsed.clone());
    if let Some((_id, meta)) = match_runtime_meta(&parsed).or_else(|| state_match(identifier)) {
        item = normalize_runtime_inspect(item, &meta);
    }
    Ok(item)
}

fn synthetic_inspect(id: &str, meta: &ContainerMeta, status: &str, running: bool) -> Value {
    let mut ports = serde_json::Map::new();
    for port in &meta.ports {
        if let Some((host_ip, host_port, container_port)) = pystack_hyperv::parse_port(port) {
            ports.insert(
                format!("{container_port}/tcp"),
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

fn match_runtime_meta(item: &Value) -> Option<(String, ContainerMeta)> {
    let runtime_id = item.get("Id").and_then(|v| v.as_str()).unwrap_or_default();
    let runtime_name = item
        .get("Name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim_start_matches('/');
    read_state().containers.into_iter().find(|(_id, meta)| {
        meta.runtime_id.as_deref() == Some(runtime_id) || meta.runtime_name == runtime_name
    })
}

fn normalize_runtime_inspect(mut item: Value, meta: &ContainerMeta) -> Value {
    if let Some(obj) = item.as_object_mut() {
        obj.insert("Id".to_string(), Value::String(meta.id.clone()));
        obj.insert("Name".to_string(), Value::String(format!("/{}", meta.name)));

        let mut ports = serde_json::Map::new();
        for port in &meta.ports {
            if let Some((host_ip, host_port, container_port)) = pystack_hyperv::parse_port(port) {
                ports.insert(
                    format!("{container_port}/tcp"),
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
    let mgr = match manager() {
        Ok(mgr) => mgr,
        Err(_err) if include_all => {
            for (id, meta) in read_state().containers {
                result.push(container_summary(&synthetic_inspect(
                    &id, &meta, "created", false,
                )));
            }
            return Ok(result);
        }
        Err(_) => return Ok(result),
    };
    let ps_args = if include_all {
        vec!["ps", "-a", "--quiet"]
    } else {
        vec!["ps", "--quiet"]
    };
    let raw = match mgr.ssh(
        &pystack_hyperv::nerdctl_command(mgr.config(), &ps_args),
        false,
    ) {
        Ok(raw) => raw,
        Err(err) if include_all => {
            tracing::debug!("falling back to synthetic container state: {err}");
            String::new()
        }
        Err(_err) => return Ok(result),
    };
    for reference in raw.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let cmd = format!(
            "{} 2>/dev/null || true",
            pystack_hyperv::nerdctl_command(mgr.config(), &["inspect", reference])
        );
        let raw = mgr.ssh(&cmd, false)?;
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
        if let Some((_id, meta)) = match_runtime_meta(&item) {
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
        let state = read_state();
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
            mgr.inspect_containers(&names_refs).unwrap_or_default()
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
            let _guard = STATE_LOCK.lock().unwrap();
            let mut state = read_state();
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
        &pystack_hyperv::nerdctl_command(
            mgr.config(),
            &["network", "ls", "--format", "{{json .}}"],
        ),
        false,
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
    let out = manager()?.network_inspect(&[name])?;
    let parsed: Value = serde_json::from_str(&out).unwrap_or_else(|_| json!([]));
    if let Some(first) = parsed.as_array().and_then(|items| items.first()) {
        return Ok(first.clone());
    }
    Ok(
        json!({"Name": name, "Id": name, "Driver": "bridge", "Scope": "local", "Containers": {}, "Options": {}, "Labels": {}}),
    )
}

fn remove_network(name: &str) -> Result<(), ApiError> {
    manager()?.network_remove(&[name])?;
    Ok(())
}

fn list_volumes() -> Result<Value, ApiError> {
    let mgr = manager()?;
    let raw = mgr.ssh(
        &pystack_hyperv::nerdctl_command(mgr.config(), &["volume", "ls", "--format", "{{json .}}"]),
        false,
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
    let out = manager()?.volume_inspect(&[name])?;
    let parsed: Value = serde_json::from_str(&out).unwrap_or_else(|_| json!([]));
    if let Some(first) = parsed.as_array().and_then(|items| items.first()) {
        return Ok(first.clone());
    }
    Ok(
        json!({"Name": name, "Driver": "local", "Mountpoint": "", "Labels": {}, "Options": {}, "Scope": "local"}),
    )
}

fn remove_volume(name: &str) -> Result<(), ApiError> {
    manager()?.volume_remove(&[name], false)?;
    Ok(())
}

fn list_images() -> Result<Vec<Value>, ApiError> {
    let mgr = manager()?;
    let raw = mgr.ssh(
        &pystack_hyperv::nerdctl_command(mgr.config(), &["images", "--format", "{{json .}}"]),
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

fn pull_image(name: &str) -> Result<Value, ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "fromImage query parameter is required".to_string(),
        ));
    }
    let mgr = manager()?;
    let output = mgr.ssh(
        &pystack_hyperv::nerdctl_command(mgr.config(), &["pull", name]),
        true,
    )?;
    Ok(json!({ "status": "pulled", "id": name, "output": output }))
}

fn inspect_image(name: &str) -> Result<Value, ApiError> {
    let mgr = manager()?;
    let cmd = format!(
        "{} 2>/dev/null || true",
        pystack_hyperv::nerdctl_command(mgr.config(), &["image", "inspect", name])
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

    fn authed(builder: axum::http::request::Builder) -> axum::http::request::Builder {
        builder.header(header::AUTHORIZATION, "Bearer test-token")
    }

    struct IsolatedHome {
        path: PathBuf,
        old_userprofile: Option<String>,
        old_home: Option<String>,
        old_token: Option<String>,
        old_allow_binds: Option<String>,
        old_bind_roots: Option<String>,
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
            restore_env("STACKDECK_DOCKER_API_ALLOW_BINDS", &self.old_allow_binds);
            restore_env("STACKDECK_DOCKER_API_BIND_ROOTS", &self.old_bind_roots);
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
        let old_allow_binds = std::env::var("STACKDECK_DOCKER_API_ALLOW_BINDS").ok();
        let old_bind_roots = std::env::var("STACKDECK_DOCKER_API_BIND_ROOTS").ok();
        std::env::set_var("USERPROFILE", &path);
        std::env::remove_var("HOME");
        std::env::set_var("STACKDECK_DOCKER_API_TOKEN", TEST_TOKEN);
        std::env::remove_var("STACKDECK_DOCKER_API_ALLOW_BINDS");
        std::env::remove_var("STACKDECK_DOCKER_API_BIND_ROOTS");
        IsolatedHome {
            path,
            old_userprofile,
            old_home,
            old_token,
            old_allow_binds,
            old_bind_roots,
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
        let ports = port_bindings(
            None,
            Some(&json!({ "PortBindings": { "80/tcp": [{ "HostPort": "8080" }] } })),
        );
        assert_eq!(ports, vec!["8080:80"]);
    }

    #[test]
    fn exposed_ports_do_not_publish_without_publish_all() {
        let ports = port_bindings(Some(&json!({ "5432/tcp": {} })), Some(&json!({})));
        assert!(ports.is_empty());
    }

    #[test]
    fn publish_all_uses_exposed_ports() {
        let ports = port_bindings(
            Some(&json!({ "5432/tcp": {} })),
            Some(&json!({ "PublishAllPorts": true })),
        );
        assert_eq!(ports.len(), 1);
        let (host, target) = ports[0].split_once(':').unwrap();
        assert_ne!(host, "5432");
        assert!(host.parse::<u16>().unwrap() > 0);
        assert_eq!(target, "5432");
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
    async fn auth_token_is_enforced_when_configured() {
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
        assert_eq!(default_list.status(), StatusCode::OK);
        assert!(json_body(default_list).await.as_array().unwrap().is_empty());

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
}
