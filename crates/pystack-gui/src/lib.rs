//! Web GUI for PyStack Runner.
//!
//! Replaces the WSGI web GUI from `core.py` — provides a browser-based
//! dashboard for managing services via axum HTTP server.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Form, Router,
};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

fn create_command<S: AsRef<std::ffi::OsStr>>(program: S) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    cmd
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

pub struct AppState {
    pub token: String,
    pub registry_dir: PathBuf,
}

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GuiQuery {
    pub token: Option<String>,
    pub project: Option<String>,
    pub action: Option<String>,
    pub service: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ActionForm {
    pub token: Option<String>,
    pub project: Option<String>,
    pub action: Option<String>,
    pub service: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ProjectEntry {
    root: String,
    config: String,
    #[serde(default = "default_backend")]
    backend: String,
}

fn default_backend() -> String {
    "native".to_string()
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

pub async fn run_server(
    host: &str,
    port: u16,
    state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error>> {
    let app = Router::new()
        .route("/", get(dashboard_handler))
        .route("/", post(action_handler))
        .with_state(state.clone());

    let addr = format!("{}:{}", host, port);
    println!("PyStack GUI listening on http://{}", addr);
    println!("Token: {}", state.token);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Token generation
// ---------------------------------------------------------------------------

pub fn get_or_create_token() -> String {
    let token_path = pystack_types::token_file();
    if token_path.exists() {
        if let Ok(token) = std::fs::read_to_string(&token_path) {
            let trimmed = token.trim().to_string();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }
    }

    let token: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(43)
        .map(char::from)
        .collect();

    if let Some(parent) = token_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&token_path, &token);
    token
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn dashboard_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<GuiQuery>,
) -> Response {
    // Verify token
    if !verify_token(&state, query.token.as_deref()) {
        return (
            StatusCode::FORBIDDEN,
            Html("403 Forbidden: Invalid or missing token"),
        )
            .into_response();
    }

    let token = query.token.as_deref().unwrap_or(&state.token);
    let project_name = query.project.as_deref().unwrap_or("");

    if project_name.is_empty() {
        // Show project registry
        let html = render_project_list(token, &state);
        Html(html).into_response()
    } else {
        // Show project detail
        let html = render_project_detail(token, project_name, &state);
        Html(html).into_response()
    }
}

async fn action_handler(
    State(state): State<Arc<AppState>>,
    Form(form): Form<ActionForm>,
) -> Response {
    if !verify_token(&state, form.token.as_deref()) {
        return (StatusCode::FORBIDDEN, Html("403 Forbidden")).into_response();
    }

    let token = form.token.clone().unwrap_or_else(|| state.token.clone());
    let project = form.project.clone().unwrap_or_default();
    let action = form.action.clone().unwrap_or_default();
    let service = form.service.clone().unwrap_or_default();

    let state_clone = state.clone();
    let project_clone = project.clone();
    let service_clone = service.clone();

    let message = tokio::task::spawn_blocking(move || match action.as_str() {
        "start" => execute_project_action(&state_clone, &project_clone, "up", &service_clone),
        "stop" => execute_project_action(&state_clone, &project_clone, "down", &service_clone),
        "restart" => {
            execute_project_action(&state_clone, &project_clone, "restart", &service_clone)
        }
        "start-all" => execute_project_action(&state_clone, &project_clone, "up", ""),
        "stop-all" => execute_project_action(&state_clone, &project_clone, "down", ""),
        _ => format!("Unknown action: {}", action),
    })
    .await
    .unwrap_or_else(|e| format!("Task failed: {}", e));

    let html = render_action_result(&token, &project, &message, &state);
    Html(html).into_response()
}

// ---------------------------------------------------------------------------
// Token verification
// ---------------------------------------------------------------------------

fn verify_token(state: &AppState, provided: Option<&str>) -> bool {
    match provided {
        Some(t) => {
            use subtle::ConstantTimeEq;
            let expected = state.token.as_bytes();
            let actual = t.as_bytes();
            expected.ct_eq(actual).into()
        }
        None => false,
    }
}

// ---------------------------------------------------------------------------
// HTML rendering
// ---------------------------------------------------------------------------

fn render_project_list(token: &str, state: &AppState) -> String {
    let registry = read_registry(state);
    let project_items = if registry.is_empty() {
        "<li>No registered projects.</li>".to_string()
    } else {
        registry
            .keys()
            .map(|name| {
                format!(
                    "<li><a href='/?token={token}&amp;project={name}'>{name}</a></li>",
                    token = html_escape(token),
                    name = html_escape(name)
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };
    format!(
        r#"<!doctype html>
<html lang='en'>
<head><meta charset='utf-8'><meta name='viewport' content='width=device-width, initial-scale=1'>
<title>PyStack Runner</title>
<style>
body {{margin:0;background:#f4f6f8;color:#17202a;font:14px/1.5 system-ui,sans-serif}}
main {{max-width:720px;margin:48px auto;padding:0 18px}}
.panel {{background:#fff;border:1px solid #d9e0e7;border-radius:8px;padding:18px;box-shadow:0 12px 28px rgba(23,32,42,.08)}}
h1 {{font-size:22px;margin:0 0 8px}}
p {{margin:0;color:#667085}}
a {{color:#1f6feb;text-decoration:none}}
a:hover {{text-decoration:underline}}
.projects {{list-style:none;padding:0;margin:16px 0}}
.projects li {{padding:8px 0;border-bottom:1px solid #eef2f5}}
.projects li:last-child {{border-bottom:none}}
.brand {{display:flex;align-items:baseline;gap:8px;margin-bottom:16px}}
</style></head><body><main>
<section class='panel'>
<div class='brand'><h1>PyStack Runner</h1><span style='color:#667085'>v{version}</span></div>
<p>Manage your process stacks from the browser.</p>
<ul class='projects'>
{project_items}
</ul>
</section></main></body></html>"#,
        version = env!("CARGO_PKG_VERSION"),
        project_items = project_items
    )
}

fn render_project_detail(token: &str, project: &str, state: &AppState) -> String {
    let registry = read_registry(state);
    let Some(entry) = registry.get(project) else {
        return render_action_result(
            token,
            project,
            &format!("Project not registered: {}", project),
            state,
        );
    };
    let services = read_service_names(entry);
    let rows = if services.is_empty() {
        "<tr><td colspan='4'>No services found.</td></tr>".to_string()
    } else {
        services
            .iter()
            .map(|service| {
                format!(
                    r#"<tr>
  <td><strong>{service}</strong></td>
  <td style='color:#667085'>Configured</td>
  <td>{backend}</td>
  <td class='actions'>
    {start}{restart}{stop}
  </td>
</tr>"#,
                    service = html_escape(service),
                    backend = html_escape(&entry.backend),
                    start =
                        action_button(token, project, "start", service, "Start", "btn btn-primary"),
                    restart = action_button(token, project, "restart", service, "Restart", "btn"),
                    stop = action_button(token, project, "stop", service, "Stop", "btn btn-danger"),
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };
    format!(
        r#"<!doctype html>
<html lang='en'>
<head><meta charset='utf-8'><meta name='viewport' content='width=device-width, initial-scale=1'>
<title>{project} - PyStack</title>
<style>
body {{margin:0;background:#f4f6f8;color:#17202a;font:14px/1.5 system-ui,sans-serif}}
main {{max-width:960px;margin:48px auto;padding:0 18px}}
.panel {{background:#fff;border:1px solid #d9e0e7;border-radius:8px;padding:18px;box-shadow:0 12px 28px rgba(23,32,42,.08)}}
h1 {{font-size:22px;margin:0 0 16px}}
table {{width:100%;border-collapse:collapse}}
th,td {{text-align:left;padding:10px 12px;border-bottom:1px solid #eef2f5}}
th {{color:#667085;font-weight:600;font-size:12px;text-transform:uppercase}}
.actions {{display:flex;gap:6px}}
.btn {{padding:6px 14px;border-radius:6px;border:1px solid #d9e0e7;background:#fff;cursor:pointer;font-size:13px}}
.btn:hover {{background:#f4f6f8}}
.btn-primary {{background:#1f6feb;color:#fff;border-color:#1f6feb}}
.btn-primary:hover {{background:#1a5fd0}}
.btn-danger {{color:#b42318;border-color:#ffe4df}}
a {{color:#1f6feb;text-decoration:none}}
</style></head><body><main>
<section class='panel'>
<h1>{project}</h1>
<p style='color:#667085;margin-bottom:16px'>Service dashboard</p>
<table>
<tr><th>Service</th><th>Status</th><th>Backend</th><th>Actions</th></tr>
{rows}
</table>
<div style='margin-top:16px'>
  <form method='post' action='/' style='display:inline'>
    <input type='hidden' name='token' value='{token}'>
    <input type='hidden' name='project' value='{project}'>
    <input type='hidden' name='action' value='start-all'>
    <button class='btn btn-primary' type='submit'>Start All</button>
  </form>
  <form method='post' action='/' style='display:inline'>
    <input type='hidden' name='token' value='{token}'>
    <input type='hidden' name='project' value='{project}'>
    <input type='hidden' name='action' value='stop-all'>
    <button class='btn btn-danger' type='submit'>Stop All</button>
  </form>
</div>
</section></main></body></html>"#,
        project = html_escape(project),
        token = html_escape(token),
        rows = rows
    )
}

fn render_action_result(token: &str, project: &str, message: &str, _state: &AppState) -> String {
    format!(
        r#"<!doctype html>
<html lang='en'>
<head><meta charset='utf-8'><meta name='viewport' content='width=device-width, initial-scale=1'>
<title>Result - PyStack</title>
<style>
body {{margin:0;background:#f4f6f8;color:#17202a;font:14px/1.5 system-ui,sans-serif}}
main {{max-width:720px;margin:48px auto;padding:0 18px}}
.panel {{background:#fff;border:1px solid #d9e0e7;border-radius:8px;padding:18px;box-shadow:0 12px 28px rgba(23,32,42,.08)}}
h1 {{font-size:22px;margin:0 0 16px}}
.notice {{padding:12px;border-radius:8px;margin-bottom:16px;background:#e2ecff;color:#175cd3}}
a {{color:#1f6feb;text-decoration:none}}
a:hover {{text-decoration:underline}}
</style></head><body><main>
<section class='panel'>
<h1>Action Result</h1>
<div class='notice'>{message}</div>
<p><a href='/?token={token}&amp;project={project}'>&larr; Back to {project}</a></p>
</section></main></body></html>"#,
        message = html_escape(message),
        token = html_escape(token),
        project = html_escape(project)
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn read_registry(state: &AppState) -> HashMap<String, ProjectEntry> {
    let path = state.registry_dir.join("projects.json");
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn project_config_path(entry: &ProjectEntry) -> PathBuf {
    let config = PathBuf::from(&entry.config);
    if config.is_absolute() {
        config
    } else {
        PathBuf::from(&entry.root).join(config)
    }
}

fn read_service_names(entry: &ProjectEntry) -> Vec<String> {
    let path = project_config_path(entry);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let raw = if is_yaml_path(&path) {
        serde_yaml::from_str::<serde_json::Value>(&text).unwrap_or_default()
    } else {
        serde_json::from_str::<serde_json::Value>(&text).unwrap_or_default()
    };
    raw.get("services")
        .and_then(|v| v.as_object())
        .map(|services| services.keys().cloned().collect())
        .unwrap_or_default()
}

fn is_yaml_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("yaml" | "yml")
    )
}

fn action_button(
    token: &str,
    project: &str,
    action: &str,
    service: &str,
    label: &str,
    class_name: &str,
) -> String {
    format!(
        r#"<form method='post' action='/' style='display:inline'>
      <input type='hidden' name='token' value='{token}'>
      <input type='hidden' name='project' value='{project}'>
      <input type='hidden' name='action' value='{action}'>
      <input type='hidden' name='service' value='{service}'>
      <button class='{class_name}' type='submit'>{label}</button>
    </form>"#,
        token = html_escape(token),
        project = html_escape(project),
        action = html_escape(action),
        service = html_escape(service),
        class_name = html_escape(class_name),
        label = html_escape(label),
    )
}

fn execute_project_action(state: &AppState, project: &str, action: &str, service: &str) -> String {
    let registry = read_registry(state);
    let Some(entry) = registry.get(project) else {
        return format!("Project not registered: {}", project);
    };
    let exe = std::env::current_exe()
        .ok()
        .and_then(|mut p| {
            p.set_file_name(if cfg!(windows) {
                "pystack.exe"
            } else {
                "pystack"
            });
            if p.exists() {
                Some(p)
            } else {
                None
            }
        })
        .unwrap_or_else(|| PathBuf::from("pystack"));
    let mut cmd = create_command(exe);
    cmd.current_dir(&entry.root)
        .arg("--config")
        .arg(&entry.config)
        .arg("--backend")
        .arg(&entry.backend)
        .arg(action);
    if !service.is_empty() {
        cmd.arg(service);
    }
    match cmd.output() {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if stdout.is_empty() {
                format!("{} succeeded", action)
            } else {
                stdout
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if stderr.is_empty() {
                format!("{} failed: {}", action, stdout)
            } else {
                format!("{} failed: {}", action, stderr)
            }
        }
        Err(err) => format!("{} failed: {}", action, err),
    }
}
