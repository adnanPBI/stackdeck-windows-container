use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Projects,
    Containers,
    Images,
    Volumes,
    Networks,
    Vm,
    Logs,
}

#[derive(Debug, Clone)]
struct ProjectEntry {
    name: String,
    root: String,
    config: String,
    backend: String,
}

#[derive(Debug, Clone)]
struct ServiceEntry {
    name: String,
    image: String,
    backend: String,
    ports: Vec<String>,
    urls: Vec<String>,
}

#[derive(Debug)]
enum WorkerMessage {
    CommandFinished {
        label: String,
        output: String,
    },
    OverviewRefreshed {
        containers: String,
        images: String,
        volumes: String,
        networks: String,
        vm_health: String,
    },
}

struct DesktopApp {
    tab: Tab,
    projects: Vec<ProjectEntry>,
    selected_project: Option<String>,
    selected_service: String,
    services: Vec<ServiceEntry>,
    containers: String,
    images: String,
    volumes: String,
    networks: String,
    vm_health: String,
    logs: String,
    status: String,
    register_name: String,
    register_root: String,
    register_config: String,
    register_backend: String,
    tx: Sender<WorkerMessage>,
    rx: Receiver<WorkerMessage>,
}

impl DesktopApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = mpsc::channel();
        let mut app = Self {
            tab: Tab::Projects,
            projects: Vec::new(),
            selected_project: None,
            selected_service: String::new(),
            services: Vec::new(),
            containers: String::new(),
            images: String::new(),
            volumes: String::new(),
            networks: String::new(),
            vm_health: String::new(),
            logs: String::new(),
            status: "Ready".to_string(),
            register_name: String::new(),
            register_root: ".".to_string(),
            register_config: "stack.json".to_string(),
            register_backend: "native".to_string(),
            tx,
            rx,
        };
        app.reload_projects();
        app.reload_services();
        app.spawn_refresh();
        app
    }

    fn reload_projects(&mut self) {
        self.projects = read_projects();
        if self.selected_project.is_none() {
            self.selected_project = self.projects.first().map(|p| p.name.clone());
        }
    }

    fn reload_services(&mut self) {
        self.services = self
            .selected_project_entry()
            .map(|project| read_project_services(&project))
            .unwrap_or_default();
        if self.selected_service.is_empty() {
            self.selected_service = self
                .services
                .first()
                .map(|service| service.name.clone())
                .unwrap_or_default();
        }
        if !self.selected_service.is_empty()
            && !self
                .services
                .iter()
                .any(|service| service.name == self.selected_service)
        {
            self.selected_service = self
                .services
                .first()
                .map(|service| service.name.clone())
                .unwrap_or_default();
        }
    }

    fn collect_hyperv_overview() -> (String, String, String, String, String) {
        let cfg = match pystack_hyperv::HyperVManager::load_config() {
            Ok(cfg) => cfg,
            Err(err) => {
                return (
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    format!("Could not load Hyper-V config: {err}"),
                );
            }
        };
        let mgr = pystack_hyperv::HyperVManager::new(cfg);
        let containers = mgr.container_ps().unwrap_or_else(|err| err.to_string());
        let images = mgr.image_list(false).unwrap_or_else(|err| err.to_string());
        let volumes = mgr.volume_list().unwrap_or_else(|err| err.to_string());
        let networks = mgr.network_list().unwrap_or_else(|err| err.to_string());
        let vm_health = mgr
            .runtime_health_check()
            .map(|health| serde_json::to_string_pretty(&health).unwrap_or_default())
            .unwrap_or_else(|err| err.to_string());
        (containers, images, volumes, networks, vm_health)
    }

    fn spawn_refresh(&mut self) {
        let tx = self.tx.clone();
        self.status = "Refreshing runtime overview".to_string();
        thread::spawn(move || {
            let (containers, images, volumes, networks, vm_health) =
                Self::collect_hyperv_overview();
            let _ = tx.send(WorkerMessage::OverviewRefreshed {
                containers,
                images,
                volumes,
                networks,
                vm_health,
            });
        });
    }

    fn selected_project_entry(&self) -> Option<ProjectEntry> {
        let selected = self.selected_project.as_ref()?;
        self.projects
            .iter()
            .find(|project| &project.name == selected)
            .cloned()
    }

    fn spawn_project_command(&mut self, action: &str) {
        let Some(project) = self.selected_project_entry() else {
            self.status = "No project selected".to_string();
            return;
        };
        let service = self.selected_service.trim().to_string();
        self.spawn_project_command_for(project, action, service);
    }

    fn spawn_project_command_for(&mut self, project: ProjectEntry, action: &str, service: String) {
        let action = action.to_string();
        let tx = self.tx.clone();
        self.status = format!("Running {} for {}", action, project.name);
        thread::spawn(move || {
            let output = run_pystack_command(&project, &action, &service);
            let _ = tx.send(WorkerMessage::CommandFinished {
                label: action,
                output,
            });
        });
    }

    fn spawn_register_project(&mut self) {
        let name = self.register_name.trim().to_string();
        let root = self.register_root.trim().to_string();
        let config = self.register_config.trim().to_string();
        let backend = self.register_backend.trim().to_string();
        if name.is_empty() {
            self.status = "Project name is required".to_string();
            return;
        }
        if root.is_empty() {
            self.status = "Project root is required".to_string();
            return;
        }
        if config.is_empty() {
            self.status = "Config path is required".to_string();
            return;
        }
        let tx = self.tx.clone();
        self.status = format!("Registering {name}");
        thread::spawn(move || {
            let output = run_pystack_args(vec![
                "register".into(),
                "--name".into(),
                name,
                "--path".into(),
                root,
                "--config".into(),
                config,
                "--backend".into(),
                backend,
                "--allow-invalid".into(),
            ]);
            let _ = tx.send(WorkerMessage::CommandFinished {
                label: "Register".to_string(),
                output,
            });
        });
    }

    fn spawn_hyperv_command(&mut self, label: &str, args: Vec<String>) {
        let tx = self.tx.clone();
        let label = label.to_string();
        self.status = format!("Running {}", label);
        thread::spawn(move || {
            let output = run_current_exe(args);
            let _ = tx.send(WorkerMessage::CommandFinished { label, output });
        });
    }

    fn drain_messages(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                WorkerMessage::CommandFinished { label, output } => {
                    self.status = format!("{} finished", label);
                    self.logs = output;
                    self.reload_projects();
                    self.reload_services();
                    self.spawn_refresh();
                }
                WorkerMessage::OverviewRefreshed {
                    containers,
                    images,
                    volumes,
                    networks,
                    vm_health,
                } => {
                    self.containers = containers;
                    self.images = images;
                    self.volumes = volumes;
                    self.networks = networks;
                    self.vm_health = vm_health;
                    self.status = "Runtime overview refreshed".to_string();
                }
            }
        }
    }
}

impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_messages();
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("PyStack Desktop");
                ui.separator();
                tab_button(ui, &mut self.tab, Tab::Projects, "Projects");
                tab_button(ui, &mut self.tab, Tab::Containers, "Containers");
                tab_button(ui, &mut self.tab, Tab::Images, "Images");
                tab_button(ui, &mut self.tab, Tab::Volumes, "Volumes");
                tab_button(ui, &mut self.tab, Tab::Networks, "Networks");
                tab_button(ui, &mut self.tab, Tab::Vm, "VM");
                tab_button(ui, &mut self.tab, Tab::Logs, "Logs");
            });
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                if ui.button("Refresh").clicked() {
                    self.reload_projects();
                    self.reload_services();
                    self.spawn_refresh();
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Projects => self.ui_projects(ui),
            Tab::Containers => self.ui_text_table(ui, "Containers", &self.containers),
            Tab::Images => self.ui_text_table(ui, "Images", &self.images),
            Tab::Volumes => self.ui_text_table(ui, "Volumes", &self.volumes),
            Tab::Networks => self.ui_text_table(ui, "Networks", &self.networks),
            Tab::Vm => self.ui_vm(ui),
            Tab::Logs => self.ui_text_table(ui, "Command Output", &self.logs),
        });
    }
}

impl DesktopApp {
    fn ui_projects(&mut self, ui: &mut egui::Ui) {
        let before_project = self.selected_project.clone();
        ui.horizontal(|ui| {
            ui.label("Project");
            egui::ComboBox::from_id_source("project_select")
                .selected_text(
                    self.selected_project
                        .as_deref()
                        .unwrap_or("No registered projects"),
                )
                .show_ui(ui, |ui| {
                    for project in &self.projects {
                        ui.selectable_value(
                            &mut self.selected_project,
                            Some(project.name.clone()),
                            &project.name,
                        );
                    }
                });
            if before_project != self.selected_project {
                self.reload_services();
            }
            ui.label("Service");
            egui::ComboBox::from_id_source("service_select")
                .selected_text(if self.selected_service.is_empty() {
                    "All services"
                } else {
                    &self.selected_service
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.selected_service, String::new(), "All services");
                    for service in &self.services {
                        ui.selectable_value(
                            &mut self.selected_service,
                            service.name.clone(),
                            &service.name,
                        );
                    }
                });
        });

        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Start").clicked() {
                self.spawn_project_command("up");
            }
            if ui.button("Stop").clicked() {
                self.spawn_project_command("down");
            }
            if ui.button("Restart").clicked() {
                self.spawn_project_command("restart");
            }
            if ui.button("Status").clicked() {
                self.spawn_project_command("status");
            }
            if ui.button("Logs").clicked() {
                self.spawn_project_command("logs");
            }
            if ui.button("Reload services").clicked() {
                self.reload_services();
            }
        });

        ui.separator();
        self.ui_services(ui);

        ui.separator();
        ui.collapsing("Register Project", |ui| {
            egui::Grid::new("register_project_grid")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut self.register_name);
                    ui.end_row();
                    ui.label("Root");
                    ui.text_edit_singleline(&mut self.register_root);
                    ui.end_row();
                    ui.label("Config");
                    ui.text_edit_singleline(&mut self.register_config);
                    ui.end_row();
                    ui.label("Backend");
                    egui::ComboBox::from_id_source("register_backend")
                        .selected_text(&self.register_backend)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.register_backend,
                                "native".to_string(),
                                "native",
                            );
                            ui.selectable_value(
                                &mut self.register_backend,
                                "hyperv".to_string(),
                                "hyperv",
                            );
                        });
                    ui.end_row();
                });
            if ui.button("Register").clicked() {
                self.spawn_register_project();
            }
        });

        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("projects_grid")
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("Name");
                    ui.strong("Backend");
                    ui.strong("Config");
                    ui.strong("Root");
                    ui.end_row();
                    for project in &self.projects {
                        ui.label(&project.name);
                        ui.label(&project.backend);
                        ui.label(&project.config);
                        ui.label(&project.root);
                        ui.end_row();
                    }
                });
        });
    }

    fn ui_services(&mut self, ui: &mut egui::Ui) {
        ui.heading("Services");
        let Some(project) = self.selected_project_entry() else {
            ui.label("No project selected.");
            return;
        };
        if self.services.is_empty() {
            ui.label("No services were found in the selected config.");
            return;
        }
        let services = self.services.clone();
        egui::ScrollArea::vertical()
            .max_height(360.0)
            .show(ui, |ui| {
                egui::Grid::new("services_grid")
                    .striped(true)
                    .num_columns(7)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.strong("Service");
                        ui.strong("Backend");
                        ui.strong("Image");
                        ui.strong("Ports");
                        ui.strong("Open");
                        ui.strong("Controls");
                        ui.strong("Logs");
                        ui.end_row();
                        for service in services {
                            ui.label(&service.name);
                            ui.label(&service.backend);
                            ui.label(if service.image.is_empty() {
                                "-"
                            } else {
                                &service.image
                            });
                            ui.label(if service.ports.is_empty() {
                                "-".to_string()
                            } else {
                                service.ports.join(", ")
                            });
                            ui.horizontal(|ui| {
                                if service.urls.is_empty() {
                                    ui.label("-");
                                }
                                for url in &service.urls {
                                    let label = url
                                        .strip_prefix("http://127.0.0.1:")
                                        .unwrap_or(url)
                                        .to_string();
                                    if ui.button(label).clicked() {
                                        self.status = open_url(url);
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                if ui.button("Start").clicked() {
                                    self.spawn_project_command_for(
                                        project.clone(),
                                        "up",
                                        service.name.clone(),
                                    );
                                }
                                if ui.button("Stop").clicked() {
                                    self.spawn_project_command_for(
                                        project.clone(),
                                        "down",
                                        service.name.clone(),
                                    );
                                }
                                if ui.button("Restart").clicked() {
                                    self.spawn_project_command_for(
                                        project.clone(),
                                        "restart",
                                        service.name.clone(),
                                    );
                                }
                            });
                            if ui.button("Logs").clicked() {
                                self.spawn_project_command_for(
                                    project.clone(),
                                    "logs",
                                    service.name.clone(),
                                );
                            }
                            ui.end_row();
                        }
                    });
            });
    }

    fn ui_vm(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Start VM").clicked() {
                self.spawn_hyperv_command("Start VM", vec!["hyperv".into(), "start-vm".into()]);
            }
            if ui.button("Stop VM").clicked() {
                self.spawn_hyperv_command("Stop VM", vec!["hyperv".into(), "stop-vm".into()]);
            }
            if ui.button("Health").clicked() {
                self.spawn_hyperv_command("Health", vec!["hyperv".into(), "health".into()]);
            }
            if ui.button("Doctor").clicked() {
                self.spawn_hyperv_command("Doctor", vec!["hyperv".into(), "doctor".into()]);
            }
        });
        ui.separator();
        self.ui_text_table(ui, "VM Health", &self.vm_health);
    }

    fn ui_text_table(&self, ui: &mut egui::Ui, title: &str, text: &str) {
        ui.heading(title);
        ui.separator();
        let mut text = text.to_string();
        egui::ScrollArea::both().show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .font(egui::TextStyle::Monospace)
                    .desired_rows(28)
                    .desired_width(f32::INFINITY),
            );
        });
    }
}

fn tab_button(ui: &mut egui::Ui, tab: &mut Tab, value: Tab, label: &str) {
    if ui.selectable_label(*tab == value, label).clicked() {
        *tab = value;
    }
}

fn read_projects() -> Vec<ProjectEntry> {
    let path = pystack_types::registry_file();
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
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
    projects
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
            }]
        }
    };
    let mut services = parsed
        .services
        .values()
        .map(|service| {
            let ports = service
                .ports
                .iter()
                .map(|port| {
                    let published = port.published.unwrap_or(port.target);
                    format!("{published}:{}", port.target)
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
                urls: ports_to_urls(&ports),
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
                urls: ports_to_urls(&ports),
                ports,
            }
        })
        .collect::<Vec<_>>();
    services.sort_by(|a, b| a.name.cmp(&b.name));
    services
}

fn ports_to_urls(ports: &[String]) -> Vec<String> {
    ports
        .iter()
        .filter_map(|port| host_port_from_spec(port))
        .map(|port| format!("http://127.0.0.1:{port}"))
        .collect()
}

fn host_port_from_spec(port: &str) -> Option<u16> {
    let text = port.trim().trim_matches('"').trim_matches('\'');
    if text.is_empty() {
        return None;
    }
    let without_proto = text.split('/').next().unwrap_or(text);
    let parts = without_proto.split(':').collect::<Vec<_>>();
    match parts.as_slice() {
        [only] => only.parse().ok(),
        [host, _container] => host.parse().ok(),
        [_ip, host, _container] => host.parse().ok(),
        _ => None,
    }
}

fn open_url(url: &str) -> String {
    let result = if cfg!(target_os = "windows") {
        Command::new("cmd").args(["/C", "start", "", url]).status()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(url).status()
    } else {
        Command::new("xdg-open").arg(url).status()
    };
    match result {
        Ok(status) if status.success() => format!("Opened {url}"),
        Ok(status) => format!("Open browser exited with {status} for {url}"),
        Err(err) => format!("Could not open {url}: {err}"),
    }
}

fn run_pystack_command(project: &ProjectEntry, action: &str, service: &str) -> String {
    let mut args = vec![
        "--config".to_string(),
        project.config.clone(),
        "--backend".to_string(),
        project.backend.clone(),
        action.to_string(),
    ];
    if !service.is_empty() {
        args.push(service.to_string());
    }
    run_pystack_in(PathBuf::from(&project.root), args)
}

fn run_current_exe(args: Vec<String>) -> String {
    run_pystack_args(args)
}

fn run_pystack_args(args: Vec<String>) -> String {
    run_pystack_in(
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        args,
    )
}

fn run_pystack_in(cwd: PathBuf, args: Vec<String>) -> String {
    let exe = match find_pystack_exe() {
        Some(path) => path,
        None => {
            return "Could not locate the pystack CLI next to pystack-desktop. Build or package both binaries together.".to_string()
        }
    };
    match Command::new(exe).current_dir(cwd).args(args).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            format!(
                "exit={}\n{}\n{}",
                output.status.code().unwrap_or(-1),
                stdout.trim(),
                stderr.trim()
            )
        }
        Err(err) => err.to_string(),
    }
}

fn find_pystack_exe() -> Option<PathBuf> {
    let current = std::env::current_exe().ok()?;
    let sibling = current.with_file_name("pystack.exe");
    if sibling.exists() {
        return Some(sibling);
    }
    let sibling = current.with_file_name("pystack");
    if sibling.exists() {
        return Some(sibling);
    }
    None
}

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PyStack Desktop")
            .with_inner_size([1180.0, 760.0]),
        ..Default::default()
    };
    eframe::run_native(
        "PyStack Desktop",
        native_options,
        Box::new(|cc| Box::new(DesktopApp::new(cc))),
    )
}
