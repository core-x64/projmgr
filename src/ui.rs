use std::collections::HashSet;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::widgets::ListState;
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Tabs},
};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::data::{AppResult, ProjectData, ProjectProvider, TemplateDef};
use crate::security::{decrypt_env_file, encrypt_env_file};

const CAT_MAUVE: Color = Color::Rgb(203, 166, 247);
const CAT_BLUE: Color = Color::Rgb(137, 180, 250);
const CAT_GREEN: Color = Color::Rgb(166, 227, 161);
const CAT_RED: Color = Color::Rgb(243, 139, 168);
const CAT_SURFACE0: Color = Color::Rgb(49, 50, 68);
const CAT_TEXT: Color = Color::Rgb(205, 214, 244);
const CAT_SUBTEXT0: Color = Color::Rgb(166, 173, 200);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreationStep {
    Idle,
    EnterName,
    SelectTemplate,
    Executing,
}

#[derive(Debug, Clone)]
pub struct CleanerItem {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub kind: String,
}

#[derive(Debug, Clone)]
pub enum Command {
    RefreshData,
    DismissError,
    StartCreateProject,
    UpdateCreationName(String),
    CreationSubmitName,
    CreationTemplateNext,
    CreationTemplatePrev,
    CreationTemplateConfirm,
    CancelCreation,
    AddProjectPath(PathBuf),
    OpenIde {
        project_path: PathBuf,
    },
    OpenTemplate {
        template_path: PathBuf,
    },
    RemoveProject {
        project_path: PathBuf,
    },
    GitCommit {
        project_path: PathBuf,
        message: String,
    },
    GitPush {
        project_path: PathBuf,
    },
    GitPull {
        project_path: PathBuf,
    },
    GitBranch {
        project_path: PathBuf,
        branch: String,
    },
    GitCheckoutNew {
        project_path: PathBuf,
        branch: String,
    },
    GitMerge {
        project_path: PathBuf,
        branch: String,
    },
    GitStatus {
        project_path: PathBuf,
    },
    GitAdd {
        project_path: PathBuf,
        paths: Vec<String>,
    },
    ScanCleaner {
        project_path: PathBuf,
    },
    DeleteArtifacts {
        paths: Vec<PathBuf>,
    },
    ArchiveProject {
        project_path: PathBuf,
    },
    EncryptEnv {
        project_path: PathBuf,
        password: String,
    },
    DecryptEnv {
        project_path: PathBuf,
        password: String,
    },
    CreateTemplate {
        name: String,
    },
    DeleteTemplate {
        name: String,
    },
    GitInit {
        project_path: PathBuf,
    },
    GithubLink {
        project_path: PathBuf,
        repo_name: String,
    },
    ArchiveAndDeleteProject {
        project_path: PathBuf,
    },
    BuildProject {
        project_path: PathBuf,
    },
    RunProject {
        project_path: PathBuf,
    },
}

#[derive(Debug, Clone, Default)]
struct BackgroundUpdate {
    status_message: Option<String>,
    error: Option<String>,
    projects: Option<Vec<ProjectData>>,
    templates: Option<Vec<TemplateDef>>,
    cleaner_items: Option<Vec<CleanerItem>>,
    git_summary: Option<Vec<String>>,
    creation_complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveScreen {
    Dashboard,
    ProjectOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashboardPanel {
    Projects,
    Templates,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectTab {
    Overview,
    Cleaner,
    Secrets,
    Git,
}

impl ProjectTab {
    const ALL: [ProjectTab; 4] = [
        ProjectTab::Overview,
        ProjectTab::Cleaner,
        ProjectTab::Secrets,
        ProjectTab::Git,
    ];

    fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    fn previous(self) -> Self {
        let idx = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    fn title(self) -> &'static str {
        match self {
            ProjectTab::Overview => "Overview",
            ProjectTab::Cleaner => "Cleaner",
            ProjectTab::Secrets => "Secrets",
            ProjectTab::Git => "Git",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingKeyPrefix {
    None,
    Git,
    Secrets,
    Cleaner,
}

#[derive(Clone, PartialEq, Eq)]
enum PromptAction {
    AddProjectPath,
    GitCommit,
    GitBranch,
    GitCheckoutNew,
    GitMerge,
    EncryptEnv,
    DecryptEnv,
    CreateTemplate,
    GithubLink,
    GitAdd,
    BuildProject,
    RunProject,
}

#[derive(Clone)]
struct PromptState {
    title: String,
    input: String,
    action: PromptAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub auto_git: bool,
    pub github_visibility: String,
    pub default_editor: Option<String>,
    pub auto_commit_initial: bool,
    pub default_branch_name: String,
    pub git_commit_all: bool,
    pub minimal_mode: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            auto_git: false,
            github_visibility: "private".to_string(),
            default_editor: Some("nvim".to_string()),
            auto_commit_initial: true,
            default_branch_name: "main".to_string(),
            git_commit_all: false,
            minimal_mode: false,
        }
    }
}

fn resolve_config_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".config/unit-projman/unit.cfg"))
}

pub fn load_config() -> AppConfig {
    if let Some(path) = resolve_config_path() {
        if path.exists() {
            if let Ok(raw) = fs::read_to_string(&path) {
                if let Ok(parsed) = toml::from_str::<AppConfig>(&raw) {
                    // Validate and sanitize config values
                    let mut config = parsed;
                    // Ensure github_visibility is valid
                    if !["private", "public", "internal"]
                        .contains(&config.github_visibility.as_str())
                    {
                        config.github_visibility = "private".to_string();
                    }
                    return config;
                }
            }
        } else {
            let default_cfg = AppConfig::default();
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(serialized) = toml::to_string_pretty(&default_cfg) {
                let _ = fs::write(&path, serialized);
            }
            return default_cfg;
        }
    }
    AppConfig::default()
}

pub struct App {
    provider: Arc<dyn ProjectProvider + Send + Sync>,
    workspace_root: PathBuf,
    pub projects: Vec<ProjectData>,
    pub templates: Vec<TemplateDef>,
    pub cleaner_items: Vec<CleanerItem>,
    pub git_status_summary: Vec<String>,
    pub creation_step: CreationStep,
    pub creation_name_input: String,
    pub selected_template_idx: usize,
    pub is_loading: bool,
    pub status_message: Option<String>,
    pub last_error: Option<String>,
    spinner_index: usize,
    job_rx: Option<Receiver<BackgroundUpdate>>,
    exit: bool,
    pub screen: ActiveScreen,
    pub dashboard_panel: DashboardPanel,
    pub project_tab: ProjectTab,
    pub project_state: ListState,
    pub template_state: ListState,
    prompt: Option<PromptState>,
    pub config: AppConfig,
    pub pending_prefix: PendingKeyPrefix,
    pub skip_template_selection: bool,
    pub minimal_mode: bool,
}

impl App {
    pub fn new(provider: Arc<dyn ProjectProvider + Send + Sync>, workspace_root: PathBuf) -> Self {
        let mut app = Self {
            provider,
            workspace_root,
            projects: Vec::new(),
            templates: Vec::new(),
            cleaner_items: Vec::new(),
            git_status_summary: Vec::new(),
            creation_step: CreationStep::Idle,
            creation_name_input: String::new(),
            selected_template_idx: 0,
            is_loading: false,
            status_message: None,
            last_error: None,
            spinner_index: 0,
            job_rx: None,
            exit: false,
            screen: ActiveScreen::Dashboard,
            dashboard_panel: DashboardPanel::Projects,
            project_tab: ProjectTab::Overview,
            project_state: ListState::default(),
            template_state: ListState::default(),
            prompt: None,
            config: load_config(),
            pending_prefix: PendingKeyPrefix::None,
            skip_template_selection: false,
            minimal_mode: false,
        };
        let _ = app.handle_command(Command::RefreshData, None);
        app
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            self.tick();
            terminal.draw(|frame| self.draw(frame))?;
            if event::poll(Duration::from_millis(16))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        self.on_key(key, terminal);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn handle_command(
        &mut self,
        command: Command,
        terminal: Option<&mut DefaultTerminal>,
    ) -> AppResult<()> {
        match command {
            Command::RefreshData => {
                self.projects = self.provider.get_all_projects()?;
                self.templates = self.provider.get_templates()?;
                self.ensure_valid_selection();
                Ok(())
            }
            Command::DismissError => {
                self.last_error = None;
                Ok(())
            }
            Command::StartCreateProject => self.start_creation_flow(),
            Command::UpdateCreationName(name) => self.update_creation_name(name),
            Command::CreationSubmitName => self.submit_creation_name(),
            Command::CreationTemplateNext => self.move_template_selection(1),
            Command::CreationTemplatePrev => self.move_template_selection(-1),
            Command::CreationTemplateConfirm => self.confirm_template_selection(),
            Command::CancelCreation => {
                self.reset_creation();
                Ok(())
            }
            Command::AddProjectPath(path) => {
                fs::create_dir_all(&path).map_err(|e| e.to_string())?;
                self.provider.add_project(path)?;
                self.projects = self.provider.get_all_projects()?;
                self.ensure_valid_selection();
                self.status_message = Some("Project added".to_string());
                Ok(())
            }
            Command::OpenIde { project_path } => self.action_open_ide(project_path, terminal),
            Command::RemoveProject { project_path } => {
                self.provider.remove_project(&project_path)?;
                self.projects = self.provider.get_all_projects()?;
                self.ensure_valid_selection();
                self.status_message = Some("Project removed from tracking".to_string());
                Ok(())
            }
            Command::GitCommit {
                project_path,
                message,
            } => {
                let mut args = vec!["commit".into(), "-m".into(), message];
                if self.config.git_commit_all {
                    args.insert(2, "-a".into());
                }
                self.action_git(project_path, args, Some("Commit complete".into()))
            }
            Command::GitPush { project_path } => self.action_git(
                project_path,
                vec!["push".into()],
                Some("Push complete".into()),
            ),
            Command::GitPull { project_path } => self.action_git(
                project_path,
                vec!["pull".into()],
                Some("Pull complete".into()),
            ),
            Command::GitBranch {
                project_path,
                branch,
            } => self.action_git(
                project_path,
                vec!["branch".into(), branch],
                Some("Branch created".into()),
            ),
            Command::GitCheckoutNew {
                project_path,
                branch,
            } => self.action_git(
                project_path,
                vec!["checkout".into(), "-b".into(), branch],
                Some("Switched to new branch".into()),
            ),
            Command::GitMerge {
                project_path,
                branch,
            } => self.action_git(
                project_path,
                vec!["merge".into(), branch],
                Some("Merge complete".into()),
            ),
            Command::GitStatus { project_path } => self.action_git_status(project_path),
            Command::GitAdd {
                project_path,
                paths,
            } => self.action_git_add(project_path, paths),
            Command::ScanCleaner { project_path } => self.action_scan_cleaner(project_path),
            Command::DeleteArtifacts { paths } => self.action_delete_artifacts(paths),
            Command::ArchiveProject { project_path } => self.action_archive_project(project_path),
            Command::EncryptEnv {
                project_path,
                password,
            } => self.action_encrypt_env(project_path, password),
            Command::DecryptEnv {
                project_path,
                password,
            } => self.action_decrypt_env(project_path, password),
            Command::CreateTemplate { name } => self.create_new_template(name),
            Command::DeleteTemplate { name } => self.delete_template(name),
            Command::OpenTemplate { template_path } => {
                self.action_open_template(template_path, terminal)
            }
            Command::GitInit { project_path } => self.action_git_init(project_path),
            Command::GithubLink {
                project_path,
                repo_name,
            } => self.action_github_link(project_path, repo_name),
            Command::ArchiveAndDeleteProject { project_path } => {
                self.action_archive_and_delete_project(project_path)
            }
            Command::RunProject { project_path } => self.action_run_project(project_path),
            Command::BuildProject { project_path } => self.action_build_project(project_path),
        }
    }

    pub fn tick(&mut self) {
        if self.is_loading {
            self.spinner_index = (self.spinner_index + 1) % 4;
        }
        if let Some(rx) = &self.job_rx {
            if let Ok(update) = rx.try_recv() {
                self.is_loading = false;
                self.job_rx = None;
                if let Some(err) = update.error {
                    self.last_error = Some(err);
                }
                if let Some(status) = update.status_message {
                    self.status_message = Some(status);
                }
                if let Some(projects) = update.projects {
                    self.projects = projects;
                    self.ensure_valid_selection();
                }
                if let Some(items) = update.cleaner_items {
                    self.cleaner_items = items;
                }
                if let Some(summary) = update.git_summary {
                    self.git_status_summary = summary;
                }
                if let Some(templates) = update.templates {
                    self.templates = templates;
                    self.ensure_valid_selection();
                }
                if update.creation_complete {
                    self.reset_creation();
                }
            }
        }
    }

    fn on_key(&mut self, key: KeyEvent, terminal: &mut DefaultTerminal) {
        if self.prompt.is_some() {
            self.handle_prompt_key(key, terminal);
            return;
        }
        if self.creation_step == CreationStep::EnterName {
            self.handle_creation_name_key(key, terminal);
            return;
        }
        if self.creation_step == CreationStep::SelectTemplate {
            self.handle_creation_template_key(key, terminal);
            return;
        }

        match key.code {
            KeyCode::Char('x') => {
                let _ = self.handle_command(Command::DismissError, Some(terminal));
            }
            KeyCode::Esc => {
                if self.screen == ActiveScreen::ProjectOptions {
                    self.screen = ActiveScreen::Dashboard;
                    self.pending_prefix = PendingKeyPrefix::None;
                } else {
                    self.creation_step = CreationStep::Idle;
                    self.prompt = None;
                }
            }
            _ => match self.screen {
                ActiveScreen::Dashboard => self.handle_dashboard_key(key, terminal),
                ActiveScreen::ProjectOptions => self.handle_project_options_key(key, terminal),
            },
        }
    }

    fn handle_dashboard_key(&mut self, key: KeyEvent, terminal: &mut DefaultTerminal) {
        match key.code {
            KeyCode::Char('q') => self.exit = true,
            KeyCode::Tab => {
                self.dashboard_panel = match self.dashboard_panel {
                    DashboardPanel::Projects => DashboardPanel::Templates,
                    DashboardPanel::Templates => DashboardPanel::Projects,
                };
            }
            KeyCode::Char('m') => {
                self.minimal_mode = !self.minimal_mode;
                if self.minimal_mode {
                    self.status_message = Some("Minimal mode enabled".to_string());
                } else {
                    self.status_message = Some("Minimal mode disabled".to_string());
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.list_next();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.list_prev();
            }
            KeyCode::Enter => match self.dashboard_panel {
                DashboardPanel::Projects => {
                    if self.selected_project().is_some() {
                        self.screen = ActiveScreen::ProjectOptions;
                        self.project_tab = ProjectTab::Overview;
                        self.cleaner_items.clear();
                        self.git_status_summary.clear();
                        self.pending_prefix = PendingKeyPrefix::None;
                    }
                }
                DashboardPanel::Templates => {
                    if let Some(template) = self.selected_template() {
                        if let Some(home_dir) = dirs::home_dir() {
                            let template_dir = home_dir.join(".config/unit-projman/templates");
                            let template_path =
                                template_dir.join(format!("{}.json", template.name));
                            let result = self.handle_command(
                                Command::OpenTemplate { template_path },
                                Some(terminal),
                            );
                            self.apply(result);
                        } else {
                            self.last_error = Some("Unable to resolve HOME directory".to_string());
                        }
                    }
                }
            },
            KeyCode::Char('c') => match self.dashboard_panel {
                DashboardPanel::Projects => {
                    self.skip_template_selection = false;
                    let result = self.handle_command(Command::StartCreateProject, Some(terminal));
                    self.apply(result);
                }
                DashboardPanel::Templates => {
                    self.prompt = Some(PromptState {
                        title: "New Template Name".to_string(),
                        input: String::new(),
                        action: PromptAction::CreateTemplate,
                    });
                }
            },
            KeyCode::Char('a') => match self.dashboard_panel {
                DashboardPanel::Projects => {
                    self.prompt = Some(PromptState {
                        title: "Add Project Path".to_string(),
                        input: String::new(),
                        action: PromptAction::AddProjectPath,
                    });
                }
                DashboardPanel::Templates => {
                    if self.selected_template().is_some() {
                        self.creation_step = CreationStep::EnterName;
                        self.creation_name_input.clear();
                        self.selected_template_idx = self.template_state.selected().unwrap_or(0);
                        self.skip_template_selection = true;
                    }
                }
            },
            KeyCode::Char('d') | KeyCode::Delete => match self.dashboard_panel {
                DashboardPanel::Projects => {
                    if let Some(path) = self.selected_project_path() {
                        let result = self.handle_command(
                            Command::RemoveProject { project_path: path },
                            Some(terminal),
                        );
                        self.apply(result);
                    }
                }
                DashboardPanel::Templates => {
                    if let Some(template) = self.selected_template() {
                        let result = self.handle_command(
                            Command::DeleteTemplate {
                                name: template.name.clone(),
                            },
                            Some(terminal),
                        );
                        self.apply(result);
                    }
                }
            },
            _ => {}
        }
    }

    fn handle_project_options_key(&mut self, key: KeyEvent, terminal: &mut DefaultTerminal) {
        match self.pending_prefix {
            PendingKeyPrefix::None => {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        self.screen = ActiveScreen::Dashboard;
                    }
                    KeyCode::Char('h') | KeyCode::Left => {
                        self.project_tab = self.project_tab.previous();
                    }
                    KeyCode::Char('l') | KeyCode::Right => {
                        self.project_tab = self.project_tab.next();
                    }
                    KeyCode::Tab => {
                        self.project_tab = self.project_tab.next();
                    }
                    KeyCode::Char('g') => {
                        self.pending_prefix = PendingKeyPrefix::Git;
                        if self.minimal_mode {
                            self.status_message = Some("[c]ommit  [p]ush  [u]ll  [s]tatus  [i]nit  [h]ub-link  [b]ranch-new  [n]ame  [m]erge".to_string());
                        } else {
                            self.status_message = Some("Git prefix: [c]ommit  [p]ush  [u]ll  [s]tatus  [i]nit  [h]ub-link  [b]ranch-new  [n]ame  [m]erge".to_string());
                        }
                    }
                    KeyCode::Char('s') => {
                        self.pending_prefix = PendingKeyPrefix::Secrets;
                        if self.minimal_mode {
                            self.status_message = Some("[e]ncrypt  [d]ecrypt".to_string());
                        } else {
                            self.status_message =
                                Some("Secrets prefix: [e]ncrypt  [d]ecrypt".to_string());
                        }
                    }
                    KeyCode::Char('c') => {
                        self.pending_prefix = PendingKeyPrefix::Cleaner;
                        if self.minimal_mode {
                            self.status_message =
                                Some("[a]rchive-delete  [s]can  [d]elete-artifacts".to_string());
                        } else {
                            self.status_message = Some(
                                "Cleaner prefix: [a]rchive-delete  [s]can  [d]elete-artifacts"
                                    .to_string(),
                            );
                        }
                    }
                    _ => {
                        // Handle build/run keys in Overview tab
                        if self.project_tab == ProjectTab::Overview {
                            match key.code {
                                KeyCode::Char('b') => {
                                    if let Some(path) = self.selected_project_path() {
                                        let result = self.handle_command(
                                            Command::BuildProject { project_path: path },
                                            Some(terminal),
                                        );
                                        self.apply(result);
                                    }
                                }
                                KeyCode::Char('r') => {
                                    if let Some(path) = self.selected_project_path() {
                                        let result = self.handle_command(
                                            Command::RunProject { project_path: path },
                                            Some(terminal),
                                        );
                                        self.apply(result);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            PendingKeyPrefix::Git => {
                self.pending_prefix = PendingKeyPrefix::None;
                self.status_message = None;
                match key.code {
                    KeyCode::Char('c') => {
                        self.prompt = Some(PromptState {
                            title: "Commit Message".to_string(),
                            input: String::new(),
                            action: PromptAction::GitCommit,
                        });
                    }
                    KeyCode::Char('p') => {
                        if let Some(path) = self.selected_project_path() {
                            let result = self.handle_command(
                                Command::GitPush { project_path: path },
                                Some(terminal),
                            );
                            self.apply(result);
                        }
                    }
                    KeyCode::Char('u') => {
                        if let Some(path) = self.selected_project_path() {
                            let result = self.handle_command(
                                Command::GitPull { project_path: path },
                                Some(terminal),
                            );
                            self.apply(result);
                        }
                    }
                    KeyCode::Char('s') => {
                        if let Some(path) = self.selected_project_path() {
                            let result = self.handle_command(
                                Command::GitStatus { project_path: path },
                                Some(terminal),
                            );
                            self.apply(result);
                        }
                    }
                    KeyCode::Char('i') => {
                        if let Some(path) = self.selected_project_path() {
                            let result = self.handle_command(
                                Command::GitInit { project_path: path },
                                Some(terminal),
                            );
                            self.apply(result);
                        }
                    }
                    KeyCode::Char('h') => {
                        if let Some(project) = self.selected_project() {
                            self.prompt = Some(PromptState {
                                title: format!(
                                    "Create & Link GitHub repository for '{}'",
                                    project.name
                                ),
                                input: project.name.clone(),
                                action: PromptAction::GithubLink,
                            });
                        }
                    }
                    KeyCode::Char('b') => {
                        self.prompt = Some(PromptState {
                            title: "Checkout New Branch (-b)".to_string(),
                            input: String::new(),
                            action: PromptAction::GitCheckoutNew,
                        });
                    }
                    KeyCode::Char('n') => {
                        self.prompt = Some(PromptState {
                            title: "Create Branch Name".to_string(),
                            input: String::new(),
                            action: PromptAction::GitBranch,
                        });
                    }
                    KeyCode::Char('m') => {
                        self.prompt = Some(PromptState {
                            title: "Merge Branch Name".to_string(),
                            input: String::new(),
                            action: PromptAction::GitMerge,
                        });
                    }
                    _ => {
                        self.status_message = Some("Canceled Git prefix action".to_string());
                    }
                }
            }
            PendingKeyPrefix::Secrets => {
                self.pending_prefix = PendingKeyPrefix::None;
                self.status_message = None;
                match key.code {
                    KeyCode::Char('e') => {
                        self.prompt = Some(PromptState {
                            title: "Secrets Encryption Password".to_string(),
                            input: String::new(),
                            action: PromptAction::EncryptEnv,
                        });
                    }
                    KeyCode::Char('d') => {
                        self.prompt = Some(PromptState {
                            title: "Secrets Decryption Password".to_string(),
                            input: String::new(),
                            action: PromptAction::DecryptEnv,
                        });
                    }
                    _ => {
                        self.status_message = Some("Canceled Secrets prefix action".to_string());
                    }
                }
            }
            PendingKeyPrefix::Cleaner => {
                self.pending_prefix = PendingKeyPrefix::None;
                self.status_message = None;
                match key.code {
                    KeyCode::Char('a') => {
                        if let Some(path) = self.selected_project_path() {
                            let result = self.handle_command(
                                Command::ArchiveAndDeleteProject { project_path: path },
                                Some(terminal),
                            );
                            self.apply(result);
                        }
                    }
                    KeyCode::Char('s') => {
                        if let Some(path) = self.selected_project_path() {
                            let result = self.handle_command(
                                Command::ScanCleaner { project_path: path },
                                Some(terminal),
                            );
                            self.apply(result);
                        }
                    }
                    KeyCode::Char('d') => {
                        let paths = self
                            .cleaner_items
                            .iter()
                            .map(|item| item.path.clone())
                            .collect::<Vec<_>>();
                        let result =
                            self.handle_command(Command::DeleteArtifacts { paths }, Some(terminal));
                        self.apply(result);
                    }
                    _ => {
                        self.status_message = Some("Canceled Cleaner prefix action".to_string());
                    }
                }
            }
        }
    }

    fn handle_prompt_key(&mut self, key: KeyEvent, terminal: &mut DefaultTerminal) {
        if let Some(prompt) = &mut self.prompt {
            match key.code {
                KeyCode::Esc => {
                    self.prompt = None;
                }
                KeyCode::Backspace => {
                    prompt.input.pop();
                }
                KeyCode::Char(c) => {
                    prompt.input.push(c);
                }
                KeyCode::Enter => {
                    let state = self.prompt.take();
                    if let Some(state) = state {
                        self.execute_prompt(state, terminal);
                    }
                }
                _ => {}
            }
        }
    }

    fn execute_prompt(&mut self, prompt: PromptState, terminal: &mut DefaultTerminal) {
        let input = prompt.input.trim().to_string();
        if input.is_empty() {
            self.last_error = Some("Input cannot be empty".to_string());
            return;
        }
        match prompt.action {
            PromptAction::AddProjectPath => {
                match crate::data::SqliteProjectProvider::expand_path(&input) {
                    Ok(path) => {
                        // Validate that the project path exists
                        if !path.exists() {
                            self.last_error =
                                Some(format!("Project path does not exist: {}", path.display()));
                            return;
                        }
                        let result =
                            self.handle_command(Command::AddProjectPath(path), Some(terminal));
                        self.apply(result);
                    }
                    Err(err) => {
                        self.last_error = Some(err);
                    }
                }
            }
            PromptAction::CreateTemplate => {
                let result =
                    self.handle_command(Command::CreateTemplate { name: input }, Some(terminal));
                self.apply(result);
            }
            PromptAction::GitCommit => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(
                        Command::GitCommit {
                            project_path: path,
                            message: input,
                        },
                        Some(terminal),
                    );
                    self.apply(result);
                }
            }
            PromptAction::GitBranch => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(
                        Command::GitBranch {
                            project_path: path,
                            branch: input,
                        },
                        Some(terminal),
                    );
                    self.apply(result);
                }
            }
            PromptAction::GitCheckoutNew => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(
                        Command::GitCheckoutNew {
                            project_path: path,
                            branch: input,
                        },
                        Some(terminal),
                    );
                    self.apply(result);
                }
            }
            PromptAction::GitMerge => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(
                        Command::GitMerge {
                            project_path: path,
                            branch: input,
                        },
                        Some(terminal),
                    );
                    self.apply(result);
                }
            }
            PromptAction::EncryptEnv => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(
                        Command::EncryptEnv {
                            project_path: path,
                            password: input,
                        },
                        Some(terminal),
                    );
                    self.apply(result);
                }
            }
            PromptAction::DecryptEnv => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(
                        Command::DecryptEnv {
                            project_path: path,
                            password: input,
                        },
                        Some(terminal),
                    );
                    self.apply(result);
                }
            }
            PromptAction::GithubLink => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(
                        Command::GithubLink {
                            project_path: path,
                            repo_name: input,
                        },
                        Some(terminal),
                    );
                    self.apply(result);
                }
            }
            PromptAction::GitAdd => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(
                        Command::GitAdd {
                            project_path: path,
                            paths: vec![input],
                        },
                        Some(terminal),
                    );
                    self.apply(result);
                }
            }
            PromptAction::BuildProject => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(
                        Command::BuildProject { project_path: path },
                        Some(terminal),
                    );
                    self.apply(result);
                }
            }
            PromptAction::RunProject => {
                if let Some(path) = self.selected_project_path() {
                    let result = self
                        .handle_command(Command::RunProject { project_path: path }, Some(terminal));
                    self.apply(result);
                }
            }
        }
    }

    fn handle_creation_name_key(&mut self, key: KeyEvent, terminal: &mut DefaultTerminal) {
        match key.code {
            KeyCode::Esc => {
                self.skip_template_selection = false;
                let result = self.handle_command(Command::CancelCreation, Some(terminal));
                self.apply(result);
            }
            KeyCode::Backspace => {
                self.creation_name_input.pop();
                let result = self.handle_command(
                    Command::UpdateCreationName(self.creation_name_input.clone()),
                    Some(terminal),
                );
                self.apply(result);
            }
            KeyCode::Char(c) => {
                self.creation_name_input.push(c);
                let result = self.handle_command(
                    Command::UpdateCreationName(self.creation_name_input.clone()),
                    Some(terminal),
                );
                self.apply(result);
            }
            KeyCode::Enter => {
                let result = self.handle_command(Command::CreationSubmitName, Some(terminal));
                self.apply(result);
            }
            _ => {}
        }
    }

    fn handle_creation_template_key(&mut self, key: KeyEvent, terminal: &mut DefaultTerminal) {
        match key.code {
            KeyCode::Esc => {
                let result = self.handle_command(Command::CancelCreation, Some(terminal));
                self.apply(result);
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let result = self.handle_command(Command::CreationTemplateNext, Some(terminal));
                self.apply(result);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let result = self.handle_command(Command::CreationTemplatePrev, Some(terminal));
                self.apply(result);
            }
            KeyCode::Enter => {
                let result = self.handle_command(Command::CreationTemplateConfirm, Some(terminal));
                self.apply(result);
            }
            _ => {}
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        let size = frame.area();
        match self.screen {
            ActiveScreen::Dashboard => self.draw_dashboard(frame, size),
            ActiveScreen::ProjectOptions => self.draw_project_options(frame, size),
        }
        self.draw_overlay(frame);
    }

    fn draw_dashboard(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(area);

        let title_line = Line::from(vec![
            Span::styled(" ◆ ", Style::default().fg(CAT_MAUVE)),
            Span::styled(
                "UNIT PROJECT MANAGER",
                Style::default().fg(CAT_TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ◆ ", Style::default().fg(CAT_MAUVE)),
        ]);
        let header = Paragraph::new(title_line)
            .alignment(ratatui::layout::Alignment::Center)
            .block(Self::base_block(""));
        frame.render_widget(header, chunks[0]);

        let body_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(chunks[1]);

        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(body_chunks[0]);

        // Projects List
        let is_projects_focused = self.dashboard_panel == DashboardPanel::Projects;
        let projects_border_color = if is_projects_focused {
            CAT_MAUVE
        } else {
            CAT_SUBTEXT0
        };
        let projects_title = format!(" Projects ({}) ", self.projects.len());

        let project_items = self
            .projects
            .iter()
            .map(|p| {
                let tpl = p.template_name.clone().unwrap_or_else(|| "-".to_string());
                ListItem::new(format!(" ❯ {}  [{}]", p.name, tpl))
            })
            .collect::<Vec<_>>();

        let project_list = List::new(project_items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(projects_border_color))
                    .title(projects_title),
            )
            .highlight_style(Self::highlight_style());
        frame.render_stateful_widget(project_list, left_chunks[0], &mut self.project_state);

        // Templates List
        let is_templates_focused = self.dashboard_panel == DashboardPanel::Templates;
        let templates_border_color = if is_templates_focused {
            CAT_MAUVE
        } else {
            CAT_SUBTEXT0
        };
        let templates_title = format!(" Templates ({}) ", self.templates.len());

        let template_items = self
            .templates
            .iter()
            .map(|t| {
                let file_count: usize = t.structure.values().map(|files| files.len()).sum();
                ListItem::new(format!(" ❯ {}  ({} files)", t.name, file_count))
            })
            .collect::<Vec<_>>();

        let template_list = List::new(template_items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(templates_border_color))
                    .title(templates_title),
            )
            .highlight_style(Self::highlight_style());
        frame.render_stateful_widget(template_list, left_chunks[1], &mut self.template_state);

        // Preview Tree Structure Panel
        let preview_title = match self.dashboard_panel {
            DashboardPanel::Projects => " Project Tree Structure Preview ",
            DashboardPanel::Templates => " Template Structure Preview ",
        };

        let tree_lines = match self.dashboard_panel {
            DashboardPanel::Projects => {
                if let Some(project) = self.selected_project() {
                    let mut lines = vec![
                        Line::from(vec![
                            Span::styled("Path:           ", Style::default().fg(CAT_SUBTEXT0)),
                            Span::styled(
                                project.path.display().to_string(),
                                Style::default().fg(CAT_BLUE),
                            ),
                        ]),
                        Line::from(vec![
                            Span::styled("Last Accessed:  ", Style::default().fg(CAT_SUBTEXT0)),
                            Span::styled(
                                format_unix_time(project.last_accessed),
                                Style::default().fg(CAT_TEXT),
                            ),
                        ]),
                        Line::from(vec![
                            Span::styled("Template Hint:  ", Style::default().fg(CAT_SUBTEXT0)),
                            Span::styled(
                                project
                                    .template_name
                                    .clone()
                                    .unwrap_or_else(|| "None".to_string()),
                                Style::default().fg(CAT_MAUVE),
                            ),
                        ]),
                        Line::raw(""),
                    ];
                    let file_lines = get_directory_tree_lines(&project.path, "", 0, 3);
                    if file_lines.is_empty() {
                        lines.push(Line::from(vec![Span::styled(
                            " (Empty project or path does not exist on disk) ",
                            Style::default().fg(CAT_RED).add_modifier(Modifier::ITALIC),
                        )]));
                    } else {
                        lines.extend(file_lines);
                    }
                    lines
                } else {
                    vec![Line::from("No project selected")]
                }
            }
            DashboardPanel::Templates => {
                if let Some(template) = self.selected_template() {
                    get_template_tree_lines(template)
                } else {
                    vec![Line::from("No template selected")]
                }
            }
        };

        let preview_panel = Paragraph::new(tree_lines)
            .block(Self::base_block(preview_title))
            .style(Style::default().fg(CAT_TEXT));
        frame.render_widget(preview_panel, body_chunks[1]);

        self.draw_dashboard_footer(frame, chunks[2]);
    }

    fn draw_dashboard_footer(&self, frame: &mut Frame, area: Rect) {
        let spinner = if self.is_loading {
            Some(['⠋', '⠙', '⠹', '⠸'][self.spinner_index])
        } else {
            None
        };
        let status = self
            .status_message
            .clone()
            .unwrap_or_else(|| "Ready".to_string());

        let help_text = match self.dashboard_panel {
            DashboardPanel::Projects => {
                if self.minimal_mode {
                    "Tab switch • j/k select • Enter options • c create • a path • d untrack • q quit • m minimal"
                } else {
                    "Tab switch panel • j/k select • Enter open options • c create project • a add path • d untrack • q quit • m minimal mode"
                }
            }
            DashboardPanel::Templates => {
                if self.minimal_mode {
                    "Tab switch • j/k select • Enter JSON • a create proj • c template • d template • q quit • m minimal"
                } else {
                    "Tab switch panel • j/k select • Enter edit JSON • a create project with template • c create template • d delete template • q quit • m minimal mode"
                }
            }
        };

        let line = Line::from(vec![
            Span::styled(help_text, Style::default().fg(CAT_SUBTEXT0)),
            Span::raw("   "),
            Span::styled(
                match spinner {
                    Some(s) => format!("{s} Loading..."),
                    None => "Idle".to_string(),
                },
                Style::default().fg(CAT_GREEN),
            ),
            Span::raw("   "),
            Span::styled(status, Style::default().fg(CAT_BLUE)),
        ]);
        let footer = Paragraph::new(line).block(Self::base_block(" Controls "));
        frame.render_widget(footer, area);
    }

    fn draw_project_options(&mut self, frame: &mut Frame, area: Rect) {
        let selected_project = match self.selected_project() {
            Some(p) => p,
            None => return,
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(area);

        let header_line = Line::from(vec![
            Span::styled(" PROJECT: ", Style::default().fg(CAT_SUBTEXT0)),
            Span::styled(
                &selected_project.name,
                Style::default().fg(CAT_MAUVE).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  •  ", Style::default().fg(CAT_SUBTEXT0)),
            Span::styled(
                selected_project.path.display().to_string(),
                Style::default().fg(CAT_BLUE),
            ),
        ]);
        let header = Paragraph::new(header_line)
            .alignment(ratatui::layout::Alignment::Center)
            .block(Self::base_block(""));
        frame.render_widget(header, chunks[0]);

        let titles = ProjectTab::ALL
            .iter()
            .map(|tab| Line::from(tab.title().to_string()))
            .collect::<Vec<_>>();
        let selected = ProjectTab::ALL
            .iter()
            .position(|tab| *tab == self.project_tab)
            .unwrap_or(0);
        let tabs = Tabs::new(titles)
            .block(Self::base_block(" Project Options "))
            .style(Style::default().fg(CAT_SUBTEXT0))
            .highlight_style(Style::default().fg(CAT_MAUVE).add_modifier(Modifier::BOLD))
            .select(selected);
        frame.render_widget(tabs, chunks[1]);

        let pane_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(chunks[2]);

        // Left Pane: Action Help
        let left_content = match self.project_tab {
            ProjectTab::Overview => {
                vec![
                    Line::from(vec![Span::styled(
                        "Overview Actions:",
                        Style::default().fg(CAT_MAUVE).add_modifier(Modifier::BOLD),
                    )]),
                    Line::raw(""),
                    Line::from(vec![
                        Span::styled(
                            " [o] ",
                            Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("Open in Neovim (IDE)", Style::default().fg(CAT_TEXT)),
                    ]),
                    Line::raw(""),
                    Line::from(vec![
                        Span::styled(
                            " [Esc] ",
                            Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("Return to Dashboard", Style::default().fg(CAT_TEXT)),
                    ]),
                ]
            }
            ProjectTab::Cleaner => {
                vec![
                    Line::from(vec![Span::styled(
                        "Cleaner Actions (Vim-Ergonomic):",
                        Style::default().fg(CAT_MAUVE).add_modifier(Modifier::BOLD),
                    )]),
                    Line::raw(""),
                    Line::from(vec![
                        Span::styled(
                            " [c][s] ",
                            Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("Scan heavy directories", Style::default().fg(CAT_TEXT)),
                    ]),
                    Line::from(vec![Span::styled(
                        "         (target, node_modules, build, dist)",
                        Style::default().fg(CAT_SUBTEXT0),
                    )]),
                    Line::raw(""),
                    Line::from(vec![
                        Span::styled(
                            " [c][d] ",
                            Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("Delete scanned artifacts", Style::default().fg(CAT_TEXT)),
                    ]),
                    Line::raw(""),
                    Line::from(vec![
                        Span::styled(
                            " [c][a] ",
                            Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            "Archive project & delete from disk",
                            Style::default().fg(CAT_TEXT),
                        ),
                    ]),
                    Line::from(vec![Span::styled(
                        "         (creates backups/Name.tar.gz)",
                        Style::default().fg(CAT_SUBTEXT0),
                    )]),
                ]
            }
            ProjectTab::Secrets => {
                vec![
                    Line::from(vec![Span::styled(
                        "Secrets Actions (Vim-Ergonomic):",
                        Style::default().fg(CAT_MAUVE).add_modifier(Modifier::BOLD),
                    )]),
                    Line::raw(""),
                    Line::from(vec![
                        Span::styled(
                            " [s][e] ",
                            Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("Encrypt all .env files", Style::default().fg(CAT_TEXT)),
                    ]),
                    Line::raw(""),
                    Line::from(vec![
                        Span::styled(
                            " [s][d] ",
                            Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("Decrypt all .env files", Style::default().fg(CAT_TEXT)),
                    ]),
                ]
            }
            ProjectTab::Git => {
                vec![
                    Line::from(vec![Span::styled(
                        "Git Actions (Vim-Ergonomic):",
                        Style::default().fg(CAT_MAUVE).add_modifier(Modifier::BOLD),
                    )]),
                    Line::raw(""),
                    Line::from(vec![
                        Span::styled(
                            " [g][s] ",
                            Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("Git status (porcelain)", Style::default().fg(CAT_TEXT)),
                    ]),
                    Line::from(vec![
                        Span::styled(
                            " [g][c] ",
                            Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("Git commit -m <message>", Style::default().fg(CAT_TEXT)),
                    ]),
                    Line::from(vec![
                        Span::styled(
                            " [g][p] ",
                            Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("Git push to remote", Style::default().fg(CAT_TEXT)),
                    ]),
                    Line::from(vec![
                        Span::styled(
                            " [g][u] ",
                            Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("Git pull from remote", Style::default().fg(CAT_TEXT)),
                    ]),
                    Line::from(vec![
                        Span::styled(
                            " [g][i] ",
                            Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            "Initialize Git repository (git init)",
                            Style::default().fg(CAT_TEXT),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled(
                            " [g][h] ",
                            Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            "Link & push to new GitHub repo (gh)",
                            Style::default().fg(CAT_TEXT),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled(
                            " [g][b] ",
                            Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("Checkout new branch (-b)", Style::default().fg(CAT_TEXT)),
                    ]),
                    Line::from(vec![
                        Span::styled(
                            " [g][n] ",
                            Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("Create branch", Style::default().fg(CAT_TEXT)),
                    ]),
                    Line::from(vec![
                        Span::styled(
                            " [g][m] ",
                            Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("Merge branch", Style::default().fg(CAT_TEXT)),
                    ]),
                ]
            }
        };

        let left_panel = Paragraph::new(left_content)
            .block(Self::base_block(" Actions "))
            .style(Style::default().fg(CAT_TEXT));
        frame.render_widget(left_panel, pane_chunks[0]);

        // Right Pane: Output Result
        let right_content = match self.project_tab {
            ProjectTab::Overview => {
                let tpl = selected_project
                    .template_name
                    .clone()
                    .unwrap_or_else(|| "None".to_string());
                let auto_git_status = if self.config.auto_git {
                    "Enabled"
                } else {
                    "Disabled"
                };

                // Get template config for build/run options
                let template_config = self.templates.iter().find(|t| t.name == tpl).cloned();

                let has_build_cmd = template_config
                    .as_ref()
                    .map(|t| t.build_cmd.is_some())
                    .unwrap_or(false);
                let has_output_path = template_config
                    .as_ref()
                    .map(|t| t.output_path.is_some())
                    .unwrap_or(false);

                let mut lines = Vec::new();
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![
                    Span::styled("Project Name:   ", Style::default().fg(CAT_SUBTEXT0)),
                    Span::styled(&selected_project.name, Style::default().fg(CAT_TEXT)),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("Project Path:   ", Style::default().fg(CAT_SUBTEXT0)),
                    Span::styled(
                        selected_project.path.display().to_string(),
                        Style::default().fg(CAT_TEXT),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("Template Hint:  ", Style::default().fg(CAT_SUBTEXT0)),
                    Span::styled(tpl, Style::default().fg(CAT_MAUVE)),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("Last Accessed:  ", Style::default().fg(CAT_SUBTEXT0)),
                    Span::styled(
                        format_unix_time(selected_project.last_accessed),
                        Style::default().fg(CAT_TEXT),
                    ),
                ]));
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![
                    Span::styled("Auto-Git Config: ", Style::default().fg(CAT_SUBTEXT0)),
                    Span::styled(auto_git_status, Style::default().fg(CAT_GREEN)),
                    Span::styled("  (via unit.cfg)", Style::default().fg(CAT_SUBTEXT0)),
                ]));

                // Git status section
                if !self.git_status_summary.is_empty() {
                    lines.push(Line::raw(""));
                    lines.push(Line::from(vec![Span::styled(
                        "Git Status:     ",
                        Style::default()
                            .fg(CAT_SUBTEXT0)
                            .add_modifier(Modifier::BOLD),
                    )]));
                    for summary_line in &self.git_status_summary {
                        let fg = if summary_line.starts_with("Untracked") {
                            CAT_SUBTEXT0
                        } else if summary_line.starts_with("Modified") {
                            CAT_MAUVE
                        } else if summary_line.starts_with("Added") {
                            CAT_GREEN
                        } else if summary_line.starts_with("Deleted") {
                            CAT_RED
                        } else {
                            CAT_TEXT
                        };
                        lines.push(Line::from(vec![
                            Span::styled("  ", Style::default().fg(CAT_SUBTEXT0)),
                            Span::styled(summary_line.clone(), Style::default().fg(fg)),
                        ]));
                    }
                } else {
                    lines.push(Line::raw(""));
                    lines.push(Line::from(vec![
                        Span::styled(
                            "Git Status:     ",
                            Style::default()
                                .fg(CAT_SUBTEXT0)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            "No changes. Press [g][s] to refresh.",
                            Style::default().fg(CAT_TEXT),
                        ),
                    ]));
                }

                // Build/Run options section
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![Span::styled(
                    "Build/Run:      ",
                    Style::default()
                        .fg(CAT_SUBTEXT0)
                        .add_modifier(Modifier::BOLD),
                )]));
                if has_build_cmd || has_output_path {
                    if has_build_cmd {
                        lines.push(Line::from(vec![
                            Span::styled(
                                "  [b] Build     ",
                                Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
                            ),
                            Span::styled("Build project", Style::default().fg(CAT_TEXT)),
                        ]));
                    }
                    if has_output_path {
                        lines.push(Line::from(vec![
                            Span::styled(
                                "  [r] Run       ",
                                Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
                            ),
                            Span::styled("Build & run project", Style::default().fg(CAT_TEXT)),
                        ]));
                    }
                } else {
                    lines.push(Line::from(vec![Span::styled(
                        "  No build/run configuration in template",
                        Style::default().fg(CAT_SUBTEXT0),
                    )]));
                }

                lines
            }
            ProjectTab::Cleaner => {
                let mut lines = Vec::new();
                if self.cleaner_items.is_empty() {
                    lines.push(Line::from("No cleaner scan data. Press [c][s] to scan."));
                } else {
                    let total_size: u64 =
                        self.cleaner_items.iter().map(|item| item.size_bytes).sum();
                    lines.push(Line::from(vec![
                        Span::styled(
                            "Scanned heavy directories. Total reclaimable size: ",
                            Style::default().fg(CAT_SUBTEXT0),
                        ),
                        Span::styled(
                            format_size(total_size),
                            Style::default().fg(CAT_GREEN).add_modifier(Modifier::BOLD),
                        ),
                    ]));
                    lines.push(Line::raw(""));
                    for item in &self.cleaner_items {
                        lines.push(Line::from(vec![
                            Span::styled("• ", Style::default().fg(CAT_SUBTEXT0)),
                            Span::styled(
                                item.path.display().to_string(),
                                Style::default().fg(CAT_TEXT),
                            ),
                            Span::styled(
                                format!(" ({}) ", item.kind),
                                Style::default().fg(CAT_MAUVE),
                            ),
                            Span::styled(
                                format!("- {}", format_size(item.size_bytes)),
                                Style::default().fg(CAT_BLUE),
                            ),
                        ]));
                    }
                }
                lines
            }
            ProjectTab::Secrets => {
                let mut lines = Vec::new();
                let env_files = find_env_files(&selected_project.path);
                if env_files.is_empty() {
                    lines.push(Line::from("No .env files found in project."));
                } else {
                    lines.push(Line::from(format!(
                        "Found {} .env file(s) in this project:",
                        env_files.len()
                    )));
                    lines.push(Line::raw(""));
                    for env_path in &env_files {
                        let is_enc = is_env_encrypted(env_path);
                        let status_span = if is_enc {
                            Span::styled(
                                "[ ENCRYPTED ]",
                                Style::default().fg(CAT_GREEN).add_modifier(Modifier::BOLD),
                            )
                        } else {
                            Span::styled("[ PLAINTEXT ]", Style::default().fg(CAT_RED))
                        };
                        lines.push(Line::from(vec![
                            Span::raw("  • "),
                            Span::styled(
                                env_path.display().to_string(),
                                Style::default().fg(CAT_TEXT),
                            ),
                            Span::raw("   "),
                            status_span,
                        ]));
                    }
                }
                lines
            }
            ProjectTab::Git => {
                let mut lines = Vec::new();
                lines.push(Line::from(vec![Span::styled(
                    "Git Status Summary:",
                    Style::default()
                        .fg(CAT_SUBTEXT0)
                        .add_modifier(Modifier::BOLD),
                )]));
                lines.push(Line::raw(""));
                if self.git_status_summary.is_empty() {
                    lines.push(Line::from("No parsed status. Press [g][s] to refresh."));
                } else {
                    for summary_line in &self.git_status_summary {
                        let fg = if summary_line.starts_with("Untracked") {
                            CAT_SUBTEXT0
                        } else if summary_line.starts_with("Modified") {
                            CAT_MAUVE
                        } else if summary_line.starts_with("Added") {
                            CAT_GREEN
                        } else if summary_line.starts_with("Deleted") {
                            CAT_RED
                        } else {
                            CAT_TEXT
                        };
                        lines.push(Line::from(Span::styled(
                            summary_line.clone(),
                            Style::default().fg(fg),
                        )));
                    }
                }
                lines
            }
        };

        let right_panel = Paragraph::new(right_content)
            .block(Self::base_block(" Details / Output "))
            .style(Style::default().fg(CAT_TEXT));
        frame.render_widget(right_panel, pane_chunks[1]);

        self.draw_project_options_footer(frame, chunks[3]);
    }

    fn draw_project_options_footer(&self, frame: &mut Frame, area: Rect) {
        let spinner = if self.is_loading {
            Some(['⠋', '⠙', '⠹', '⠸'][self.spinner_index])
        } else {
            None
        };

        let status = if self.pending_prefix != PendingKeyPrefix::None {
            self.status_message.clone().unwrap_or_default()
        } else {
            self.status_message
                .clone()
                .unwrap_or_else(|| "Ready".to_string())
        };

        let footer_fg = if self.pending_prefix != PendingKeyPrefix::None {
            CAT_MAUVE
        } else {
            CAT_BLUE
        };

        let line = Line::from(vec![
            Span::styled(
                "Tabs: h/l or Tab • Vim-Keys: [g]it [s]ecrets [c]leaner • Esc dashboard • q quit",
                Style::default().fg(CAT_SUBTEXT0),
            ),
            Span::raw("   "),
            Span::styled(
                match spinner {
                    Some(s) => format!("{s} Loading..."),
                    None => "Idle".to_string(),
                },
                Style::default().fg(CAT_GREEN),
            ),
            Span::raw("   "),
            Span::styled(status, Style::default().fg(footer_fg)),
        ]);
        let footer = Paragraph::new(line).block(Self::base_block(" Controls "));
        frame.render_widget(footer, area);
    }

    fn draw_overlay(&mut self, frame: &mut Frame) {
        if self.creation_step == CreationStep::EnterName {
            self.draw_input_popup(
                frame,
                "Step 1: Enter Project Name",
                &self.creation_name_input,
            );
        } else if self.creation_step == CreationStep::SelectTemplate {
            self.draw_template_select_popup(frame);
        } else if self.creation_step == CreationStep::Executing {
            self.draw_info_popup(frame, "Step 3: Creating project...", "Please wait...");
        } else if let Some(prompt) = &self.prompt {
            self.draw_input_popup(frame, &prompt.title, &prompt.input);
        }
        if let Some(error) = &self.last_error {
            self.draw_error_popup(frame, error);
        }
    }

    fn draw_template_select_popup(&self, frame: &mut Frame) {
        let area = centered_rect(60, 60, frame.area());
        frame.render_widget(Clear, area);
        let block = Self::base_block("Step 2: Select Template (j/k + Enter)")
            .border_style(Style::default().fg(CAT_MAUVE));
        frame.render_widget(block.clone(), area);
        let inner = block.inner(area);
        let mut state = self.template_state.clone();
        state.select(Some(
            self.selected_template_idx
                .min(self.templates.len().saturating_sub(1)),
        ));
        let items = self
            .templates
            .iter()
            .map(|t| ListItem::new(t.name.clone()))
            .collect::<Vec<_>>();
        let list = List::new(items)
            .highlight_symbol(">> ")
            .highlight_style(Self::highlight_style());
        frame.render_stateful_widget(list, inner, &mut state);
    }

    fn draw_input_popup(&self, frame: &mut Frame, title: &str, value: &str) {
        let area = centered_rect(60, 20, frame.area());
        frame.render_widget(Clear, area);
        let block = Self::base_block(title).border_style(Style::default().fg(CAT_BLUE));
        let paragraph = Paragraph::new(format!("{value}█"))
            .block(block)
            .style(Style::default().fg(CAT_TEXT));
        frame.render_widget(paragraph, area);
    }

    fn draw_info_popup(&self, frame: &mut Frame, title: &str, body: &str) {
        let area = centered_rect(50, 20, frame.area());
        frame.render_widget(Clear, area);
        let block = Self::base_block(title).border_style(Style::default().fg(CAT_GREEN));
        let paragraph = Paragraph::new(body).block(block);
        frame.render_widget(paragraph, area);
    }

    fn draw_error_popup(&self, frame: &mut Frame, error: &str) {
        let area = centered_rect(70, 25, frame.area());
        frame.render_widget(Clear, area);
        let block = Self::base_block("Error Popup (press x to clear)")
            .border_style(Style::default().fg(CAT_RED));
        let paragraph = Paragraph::new(error)
            .block(block)
            .style(Style::default().fg(CAT_TEXT));
        frame.render_widget(paragraph, area);
    }

    fn ensure_valid_selection(&mut self) {
        if self.projects.is_empty() {
            self.project_state.select(None);
        } else if self.project_state.selected().is_none() {
            self.project_state.select(Some(0));
        } else if let Some(i) = self.project_state.selected() {
            if i >= self.projects.len() {
                self.project_state.select(Some(self.projects.len() - 1));
            }
        }

        if self.templates.is_empty() {
            self.template_state.select(None);
        } else if self.template_state.selected().is_none() {
            self.template_state.select(Some(0));
        } else if let Some(i) = self.template_state.selected() {
            if i >= self.templates.len() {
                self.template_state.select(Some(self.templates.len() - 1));
            }
        }
    }

    fn list_next(&mut self) {
        match self.screen {
            ActiveScreen::Dashboard => match self.dashboard_panel {
                DashboardPanel::Projects => {
                    let next_idx = match self.project_state.selected() {
                        Some(i) if !self.projects.is_empty() => (i + 1) % self.projects.len(),
                        _ if !self.projects.is_empty() => 0,
                        _ => return,
                    };
                    self.project_state.select(Some(next_idx));
                }
                DashboardPanel::Templates => {
                    let next_idx = match self.template_state.selected() {
                        Some(i) if !self.templates.is_empty() => (i + 1) % self.templates.len(),
                        _ if !self.templates.is_empty() => 0,
                        _ => return,
                    };
                    self.template_state.select(Some(next_idx));
                }
            },
            ActiveScreen::ProjectOptions => {}
        }
    }

    fn list_prev(&mut self) {
        match self.screen {
            ActiveScreen::Dashboard => match self.dashboard_panel {
                DashboardPanel::Projects => {
                    let prev_idx = match self.project_state.selected() {
                        Some(0) if !self.projects.is_empty() => self.projects.len() - 1,
                        Some(i) if !self.projects.is_empty() => i.saturating_sub(1),
                        _ if !self.projects.is_empty() => 0,
                        _ => return,
                    };
                    self.project_state.select(Some(prev_idx));
                }
                DashboardPanel::Templates => {
                    let prev_idx = match self.template_state.selected() {
                        Some(0) if !self.templates.is_empty() => self.templates.len() - 1,
                        Some(i) if !self.templates.is_empty() => i.saturating_sub(1),
                        _ if !self.templates.is_empty() => 0,
                        _ => return,
                    };
                    self.template_state.select(Some(prev_idx));
                }
            },
            ActiveScreen::ProjectOptions => {}
        }
    }

    fn selected_project(&self) -> Option<&ProjectData> {
        self.project_state
            .selected()
            .and_then(|i| self.projects.get(i))
    }

    fn selected_project_path(&self) -> Option<PathBuf> {
        self.selected_project().map(|p| p.path.clone())
    }

    fn selected_template(&self) -> Option<&TemplateDef> {
        self.template_state
            .selected()
            .and_then(|i| self.templates.get(i))
    }

    fn start_creation_flow(&mut self) -> AppResult<()> {
        self.templates = self.provider.get_templates()?;
        if self.templates.is_empty() {
            return Err("No templates found under ~/.config/unit-projman/templates/".to_string());
        }
        self.creation_step = CreationStep::EnterName;
        self.creation_name_input.clear();
        self.selected_template_idx = 0;
        Ok(())
    }

    fn update_creation_name(&mut self, name: String) -> AppResult<()> {
        if self.creation_step != CreationStep::EnterName {
            return Err("Creation name input is only valid in EnterName step".to_string());
        }
        self.creation_name_input = name;
        Ok(())
    }

    fn submit_creation_name(&mut self) -> AppResult<()> {
        if self.creation_step != CreationStep::EnterName {
            return Err("Creation submit is only valid in EnterName step".to_string());
        }
        let name = self.creation_name_input.trim();
        if name.is_empty() {
            return Err("Project name cannot be empty".to_string());
        }
        let path = self.workspace_root.join(name);
        if path.exists() {
            return Err(format!(
                "Project '{}' already exists in {}",
                name,
                self.workspace_root.display()
            ));
        }

        if self.skip_template_selection {
            self.creation_step = CreationStep::SelectTemplate;
            let res = self.confirm_template_selection();
            self.skip_template_selection = false;
            res
        } else {
            self.creation_step = CreationStep::SelectTemplate;
            Ok(())
        }
    }

    fn move_template_selection(&mut self, delta: isize) -> AppResult<()> {
        if self.creation_step != CreationStep::SelectTemplate {
            return Err("Template navigation is only valid in SelectTemplate step".to_string());
        }
        if self.templates.is_empty() {
            return Err("No templates available".to_string());
        }
        let len = self.templates.len() as isize;
        let current = self.selected_template_idx as isize;
        self.selected_template_idx = (current + delta).rem_euclid(len) as usize;
        Ok(())
    }

    fn confirm_template_selection(&mut self) -> AppResult<()> {
        if self.creation_step != CreationStep::SelectTemplate {
            return Err("Template confirm is only valid in SelectTemplate step".to_string());
        }
        let template = self
            .templates
            .get(self.selected_template_idx)
            .cloned()
            .ok_or_else(|| "No template selected".to_string())?;

        let project_name = self.creation_name_input.trim().to_string();
        let project_path = self.workspace_root.join(&project_name);
        let provider = Arc::clone(&self.provider);
        self.creation_step = CreationStep::Executing;
        let auto_git = self.config.auto_git;

        self.spawn_task(move || {
            fs::create_dir_all(&project_path).map_err(|e| {
                format!(
                    "Failed to create project directory {}: {e}",
                    project_path.display()
                )
            })?;

            for (dir, files) in &template.structure {
                for (file_name, content) in files {
                    let mut relative_path = dir.clone();
                    relative_path.push_str(file_name);
                    let out_path = project_path.join(relative_path);
                    if let Some(parent) = out_path.parent() {
                        fs::create_dir_all(parent).map_err(|e| {
                            format!("Failed to create directory {}: {e}", parent.display())
                        })?;
                    }
                    fs::write(&out_path, content)
                        .map_err(|e| format!("Failed to write {}: {e}", out_path.display()))?;
                }
            }

            let hint = project_path.join(".unit-template");
            fs::write(&hint, format!("{}\n", template.name))
                .map_err(|e| format!("Failed to write template hint {}: {e}", hint.display()))?;

            if auto_git {
                let _ = ProcessCommand::new("git")
                    .arg("init")
                    .current_dir(&project_path)
                    .output();
            }

            provider.add_project(project_path.clone())?;
            let projects = provider.get_all_projects()?;
            Ok(BackgroundUpdate {
                status_message: Some(format!("Created project '{project_name}'")),
                projects: Some(projects),
                creation_complete: true,
                ..Default::default()
            })
        })
    }

    fn action_open_ide(
        &mut self,
        project_path: PathBuf,
        terminal: Option<&mut DefaultTerminal>,
    ) -> AppResult<()> {
        if let Some(t) = terminal {
            let _ = ratatui::restore();
            let status = ProcessCommand::new("nvim").arg(&project_path).status();
            *t = ratatui::init();
            let _ = t.clear();

            match status {
                Ok(s) if s.success() => {
                    self.status_message =
                        Some(format!("Closed Neovim for {}", project_path.display()));
                    Ok(())
                }
                Ok(s) => Err(format!("Neovim exited with non-zero status: {s}")),
                Err(e) => Err(format!("Failed to launch Neovim: {e}")),
            }
        } else {
            Err("Terminal context is not available to run Neovim".to_string())
        }
    }

    fn action_open_template(
        &mut self,
        template_path: PathBuf,
        terminal: Option<&mut DefaultTerminal>,
    ) -> AppResult<()> {
        if let Some(t) = terminal {
            let _ = ratatui::restore();
            let status = ProcessCommand::new("nvim").arg(&template_path).status();
            *t = ratatui::init();
            let _ = t.clear();

            match status {
                Ok(s) if s.success() => {
                    self.status_message =
                        Some(format!("Closed template {}", template_path.display()));
                    Ok(())
                }
                Ok(s) => Err(format!("Neovim exited with non-zero status: {s}")),
                Err(e) => Err(format!("Failed to launch Neovim: {e}")),
            }
        } else {
            Err("Terminal context is not available to run Neovim".to_string())
        }
    }

    fn action_git(
        &mut self,
        project_path: PathBuf,
        args: Vec<String>,
        status_success: Option<String>,
    ) -> AppResult<()> {
        self.spawn_task(move || {
            let mut cmd = ProcessCommand::new("git");
            cmd.args(&args).current_dir(&project_path);
            let output = cmd.output().map_err(|e| {
                format!(
                    "Failed to execute git {} in {}: {e}",
                    args.join(" "),
                    project_path.display()
                )
            })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                return Ok(BackgroundUpdate {
                    error: Some(format!("Git command failed: {}", stderr.trim())),
                    ..Default::default()
                });
            }

            Ok(BackgroundUpdate {
                status_message: status_success,
                ..Default::default()
            })
        })
    }

    fn action_git_status(&mut self, project_path: PathBuf) -> AppResult<()> {
        self.spawn_task(move || {
            let output = ProcessCommand::new("git")
                .args(["status", "--porcelain"])
                .current_dir(&project_path)
                .output()
                .map_err(|e| {
                    format!(
                        "Failed to execute git status in {}: {e}",
                        project_path.display()
                    )
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                return Ok(BackgroundUpdate {
                    error: Some(format!("git status failed: {}", stderr.trim())),
                    ..Default::default()
                });
            }

            let parsed = parse_porcelain_status(&String::from_utf8_lossy(&output.stdout));
            Ok(BackgroundUpdate {
                status_message: Some("Git status refreshed".to_string()),
                git_summary: Some(parsed),
                ..Default::default()
            })
        })
    }

    fn action_git_add(&mut self, project_path: PathBuf, paths: Vec<String>) -> AppResult<()> {
        self.spawn_task(move || {
            let mut cmd = ProcessCommand::new("git");
            cmd.arg("add");
            for path in &paths {
                cmd.arg(path);
            }
            cmd.current_dir(&project_path);
            let output = cmd.output().map_err(|e| {
                format!(
                    "Failed to execute git add in {}: {e}",
                    project_path.display()
                )
            })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                return Ok(BackgroundUpdate {
                    error: Some(format!("git add failed: {}", stderr.trim())),
                    ..Default::default()
                });
            }

            Ok(BackgroundUpdate {
                status_message: Some("Files added to git index".to_string()),
                ..Default::default()
            })
        })
    }

    fn action_build_project(&mut self, project_path: PathBuf) -> AppResult<()> {
        // Extract template info to get build configuration
        let template_name = match self.selected_project() {
            Some(project) => project
                .template_name
                .clone()
                .unwrap_or_else(|| "".to_string()),
            None => "".to_string(),
        };

        // Find the template to get build config
        let template_config = self
            .templates
            .iter()
            .find(|t| t.name == template_name)
            .cloned();

        self.spawn_task(move || {
            // Change to project directory
            let current_dir =
                env::current_dir().map_err(|e| format!("Failed to get current directory: {e}"))?;

            // Change to build directory if specified
            let build_dir = template_config
                .as_ref()
                .and_then(|t| t.build_dir.as_ref())
                .map(|dir| project_path.join(dir))
                .unwrap_or(project_path.clone());

            if !build_dir.exists() {
                return Err(format!(
                    "Build directory does not exist: {}",
                    build_dir.display()
                ));
            }

            let build_cmd = template_config
                .as_ref()
                .and_then(|t| t.build_cmd.as_ref())
                .map(|cmd| cmd.as_str())
                .unwrap_or("make"); // Default to make if not specified

            // Change to build directory
            if let Err(e) = env::set_current_dir(&build_dir) {
                return Err(format!("Failed to change to build directory: {e}"));
            }

            // Execute build command
            let mut cmd_parts = build_cmd.split_whitespace();
            let mut build_cmd = ProcessCommand::new(cmd_parts.next().unwrap_or("make"));
            for arg in cmd_parts {
                build_cmd.arg(arg);
            }

            let output = build_cmd
                .output()
                .map_err(|e| format!("Failed to execute build command: {e}"))?;

            // Restore original directory
            let _ = env::set_current_dir(&current_dir);

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                return Ok(BackgroundUpdate {
                    error: Some(format!("Build failed: {}", stderr.trim())),
                    ..Default::default()
                });
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            Ok(BackgroundUpdate {
                status_message: Some(format!("Build successful:\n{}", stdout)),
                ..Default::default()
            })
        })
    }

    fn action_run_project(&mut self, project_path: PathBuf) -> AppResult<()> {
        let template_name = match self.selected_project() {
            Some(project) => project
                .template_name
                .clone()
                .unwrap_or_else(|| "".to_string()),
            None => "".to_string(),
        };

        let template_config = self
            .templates
            .iter()
            .find(|t| t.name == template_name)
            .cloned();

        let current_dir =
            env::current_dir().map_err(|e| format!("Failed to get current directory: {e}"))?;

        let build_dir = template_config
            .as_ref()
            .and_then(|t| t.build_dir.as_ref())
            .map(|dir| project_path.join(dir))
            .unwrap_or_else(|| project_path.clone());

        let output_path = template_config
            .as_ref()
            .and_then(|t| t.output_path.as_ref())
            .map(|path| build_dir.join(path))
            .unwrap_or_else(|| build_dir.join("a.out"));

        if !output_path.exists() {
            match self.action_build_project(project_path.clone()) {
                Ok(()) => {
                    // Build succeeded, now recursively run and exit this call
                    return self.action_run_project(project_path);
                }
                Err(_err_string) => {
                    return Err(format!("Error during build. {}", build_dir.display()));
                }
            }
        }

        self.spawn_task(move || {
            if !build_dir.exists() {
                return Err(format!(
                    "Build directory does not exist: {}",
                    build_dir.display()
                ));
            }

            if let Err(e) = env::set_current_dir(&build_dir) {
                return Err(format!("Failed to change to build directory: {e}"));
            }

            let mut cmd = ProcessCommand::new(&output_path);
            let output = cmd
                .output()
                .map_err(|e| format!("Failed to execute program: {e}"))?;

            let _ = env::set_current_dir(&current_dir);

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                return Err(format!("Program execution failed: {}", stderr.trim()));
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let mut result = format!("Program output:\n{}", stdout);
            if !stderr.is_empty() {
                result.push_str(&format!("\nStderr:\n{}", stderr));
            }

            Ok(BackgroundUpdate {
                ..Default::default()
            })
        })
    }

    fn action_scan_cleaner(&mut self, project_path: PathBuf) -> AppResult<()> {
        self.spawn_task(move || {
            let items = find_heavy_artifacts(&project_path)?;
            Ok(BackgroundUpdate {
                status_message: Some("Cleaner scan complete".to_string()),
                cleaner_items: Some(items),
                ..Default::default()
            })
        })
    }

    fn action_delete_artifacts(&mut self, paths: Vec<PathBuf>) -> AppResult<()> {
        if paths.is_empty() {
            return Err("No artifacts selected for deletion".to_string());
        }
        self.spawn_task(move || {
            let mut cmd = ProcessCommand::new("rm");
            cmd.arg("-rf");
            for path in &paths {
                cmd.arg(path);
            }
            let output = cmd
                .output()
                .map_err(|e| format!("Failed to run rm -rf for artifacts: {e}"))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                return Ok(BackgroundUpdate {
                    error: Some(format!("Artifact cleanup failed: {}", stderr.trim())),
                    ..Default::default()
                });
            }
            Ok(BackgroundUpdate {
                status_message: Some("Artifacts deleted".to_string()),
                ..Default::default()
            })
        })
    }

    fn action_archive_project(&mut self, project_path: PathBuf) -> AppResult<()> {
        self.spawn_task(move || {
            let backups_dir = PathBuf::from("/backups");
            let _ = fs::create_dir_all(&backups_dir);

            let final_backups_dir = if fs::metadata(&backups_dir)
                .map(|m| m.permissions().readonly())
                .unwrap_or(true)
            {
                let home = dirs::home_dir()
                    .ok_or_else(|| "Unable to resolve HOME directory".to_string())?;
                home.join("backups")
            } else {
                backups_dir
            };
            fs::create_dir_all(&final_backups_dir)
                .map_err(|e| format!("Unable to create backups directory: {e}"))?;

            let project_name = project_path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| {
                    format!(
                        "Unable to derive project name from {}",
                        project_path.display()
                    )
                })?;
            let archive_path = final_backups_dir.join(format!("{project_name}.tar.gz"));
            let parent = project_path.parent().ok_or_else(|| {
                format!(
                    "Unable to determine parent directory for {}",
                    project_path.display()
                )
            })?;

            let output = ProcessCommand::new("tar")
                .args(["-czf", &archive_path.to_string_lossy(), "-C"])
                .arg(parent)
                .arg(project_name)
                .output()
                .map_err(|e| format!("Failed to run tar for {}: {e}", project_path.display()))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                return Ok(BackgroundUpdate {
                    error: Some(format!("Archive failed: {}", stderr.trim())),
                    ..Default::default()
                });
            }

            Ok(BackgroundUpdate {
                status_message: Some(format!("Archived to {}", archive_path.display())),
                ..Default::default()
            })
        })
    }

    fn action_encrypt_env(&mut self, project_path: PathBuf, password: String) -> AppResult<()> {
        self.spawn_task(move || {
            let env_files = find_env_files(&project_path);
            if env_files.is_empty() {
                return Err("No .env files found in project".to_string());
            }

            for env_path in &env_files {
                encrypt_env_file(env_path, password.clone())?;
            }

            Ok(BackgroundUpdate {
                status_message: Some(format!("Encrypted {} .env files", env_files.len())),
                ..Default::default()
            })
        })
    }

    fn action_decrypt_env(&mut self, project_path: PathBuf, password: String) -> AppResult<()> {
        self.spawn_task(move || {
            let env_files = find_env_files(&project_path);
            if env_files.is_empty() {
                return Err("No .env files found in project".to_string());
            }

            for env_path in &env_files {
                decrypt_env_file(env_path, password.clone())?;
            }

            Ok(BackgroundUpdate {
                status_message: Some(format!("Decrypted {} .env files", env_files.len())),
                ..Default::default()
            })
        })
    }

    fn create_new_template(&mut self, name: String) -> AppResult<()> {
        let provider = Arc::clone(&self.provider);
        self.spawn_task(move || {
            let home =
                dirs::home_dir().ok_or_else(|| "Unable to resolve HOME directory".to_string())?;
            let template_dir = home.join(".config/unit-projman/templates");
            fs::create_dir_all(&template_dir).map_err(|e| e.to_string())?;

            let template_path = template_dir.join(format!("{}.json", name));

            use std::collections::HashMap;
            let mut structure = HashMap::new();

            let mut src_files = HashMap::new();
            src_files.insert(
                "README.md".to_string(),
                format!("# {name}\n\nA new project created with template '{name}'.\n"),
            );
            src_files.insert(
                "main.rs".to_string(),
                "fn main() {\n    println!(\"Hello, world!\");\n}\n".to_string(),
            );
            structure.insert("src/".to_string(), src_files);

            let include_files = HashMap::new();
            structure.insert("include/".to_string(), include_files);

            let template_def = TemplateDef {
                name: name.clone(),
                structure,
                build_dir: None,
                build_cmd: None,
                output_path: None,
                init_cmd: None,
            };

            let template_content = serde_json::to_string_pretty(&template_def)
                .map_err(|e| format!("Failed to serialize template: {e}"))?;

            fs::write(&template_path, template_content).map_err(|e| e.to_string())?;

            let templates = provider.get_templates()?;

            Ok(BackgroundUpdate {
                status_message: Some(format!("Created template '{}'", name)),
                templates: Some(templates),
                ..Default::default()
            })
        })
    }

    fn delete_template(&mut self, name: String) -> AppResult<()> {
        let provider = Arc::clone(&self.provider);
        self.spawn_task(move || {
            let home =
                dirs::home_dir().ok_or_else(|| "Unable to resolve HOME directory".to_string())?;
            let template_dir = home.join(".config/unit-projman/templates");
            let template_path = template_dir.join(format!("{}.json", name));
            if template_path.exists() {
                fs::remove_file(&template_path)
                    .map_err(|e| format!("Failed to delete template file: {e}"))?;
            }
            let templates = provider.get_templates()?;
            Ok(BackgroundUpdate {
                status_message: Some(format!("Deleted template '{}'", name)),
                templates: Some(templates),
                ..Default::default()
            })
        })
    }

    fn action_git_init(&mut self, project_path: PathBuf) -> AppResult<()> {
        self.spawn_task(move || {
            let output = ProcessCommand::new("git")
                .arg("init")
                .current_dir(&project_path)
                .output()
                .map_err(|e| format!("Failed to execute git init: {e}"))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                return Err(format!("git init failed: {}", stderr.trim()));
            }

            Ok(BackgroundUpdate {
                status_message: Some("Initialized empty Git repository".to_string()),
                ..Default::default()
            })
        })
    }

    fn action_github_link(&mut self, project_path: PathBuf, repo_name: String) -> AppResult<()> {
        // Extract config value to avoid capturing self in closure
        let github_visibility = self.config.github_visibility.clone();

        self.spawn_task(move || {
            let git_dir = project_path.join(".git");
            if !git_dir.exists() {
                let output = ProcessCommand::new("git")
                    .arg("init")
                    .current_dir(&project_path)
                    .output()
                    .map_err(|e| format!("Failed to run git init: {e}"))?;
                if !output.status.success() {
                    return Err("Failed to initialize git repository".to_string());
                }
            }

            let has_commit = ProcessCommand::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&project_path)
                .output()
                .map(|out| out.status.success())
                .unwrap_or(false);

            if !has_commit {
                let _ = ProcessCommand::new("git").args(["add", "."]).current_dir(&project_path).output();
                let commit_out = ProcessCommand::new("git")
                    .args(["commit", "-m", "Initial commit"])
                    .current_dir(&project_path)
                    .output()
                    .map_err(|e| format!("Failed to run initial commit: {e}"))?;
                if !commit_out.status.success() {
                    return Err("Failed to create initial commit. Ensure git user.name/email are set.".to_string());
                }
            }

            let mut gh_args = vec![
                "repo".to_string(),
                "create".to_string(),
                repo_name.clone(),
                "--source=.".to_string(),
                "--remote=origin".to_string(),
                "--push".to_string(),
            ];

            // Add visibility flag based on config
            match github_visibility.as_str() {
                "public" => gh_args.insert(3, "--public".to_string()),
                "internal" => gh_args.insert(3, "--internal".to_string()),
                "private" | _ => gh_args.insert(3, "--private".to_string()),
            }

            let output = ProcessCommand::new("gh")
                .args(&gh_args)
                .current_dir(&project_path)
                .output()
                .map_err(|e| format!("Failed to run gh repo create: {e}. Ensure gh is installed and authenticated."))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                return Err(format!("gh repo create failed: {}", stderr.trim()));
            }

            Ok(BackgroundUpdate {
                status_message: Some(format!("Linked and pushed to GitHub repo '{repo_name}'")),
                ..Default::default()
            })
        })
    }

    fn action_archive_and_delete_project(&mut self, project_path: PathBuf) -> AppResult<()> {
        let provider = Arc::clone(&self.provider);
        self.spawn_task(move || {
            let home =
                dirs::home_dir().ok_or_else(|| "Unable to resolve HOME directory".to_string())?;
            let backups_dir = home.join("backups");
            fs::create_dir_all(&backups_dir)
                .map_err(|e| format!("Unable to create backups directory: {e}"))?;

            let project_name = project_path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| {
                    format!(
                        "Unable to derive project name from {}",
                        project_path.display()
                    )
                })?;
            let archive_path = backups_dir.join(format!("{project_name}.tar.gz"));
            let parent = project_path.parent().ok_or_else(|| {
                format!(
                    "Unable to determine parent directory for {}",
                    project_path.display()
                )
            })?;

            let output = ProcessCommand::new("tar")
                .args(["-czf", &archive_path.to_string_lossy(), "-C"])
                .arg(parent)
                .arg(project_name)
                .output()
                .map_err(|e| format!("Failed to run tar for {}: {e}", project_path.display()))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                return Err(format!("Archive failed: {}", stderr.trim()));
            }

            fs::remove_dir_all(&project_path).map_err(|e| {
                format!(
                    "Failed to delete project directory {}: {e}",
                    project_path.display()
                )
            })?;

            provider.remove_project(&project_path)?;
            let projects = provider.get_all_projects()?;

            Ok(BackgroundUpdate {
                status_message: Some(format!(
                    "Archived to {} and deleted project folder",
                    archive_path.display()
                )),
                projects: Some(projects),
                ..Default::default()
            })
        })
    }

    fn spawn_task<F>(&mut self, task: F) -> AppResult<()>
    where
        F: FnOnce() -> AppResult<BackgroundUpdate> + Send + 'static,
    {
        if self.is_loading {
            return Err("Another background action is already running".to_string());
        }
        self.last_error = None;
        self.is_loading = true;
        let (tx, rx) = mpsc::channel::<BackgroundUpdate>();
        self.job_rx = Some(rx);

        std::thread::spawn(move || {
            let result = match task() {
                Ok(update) => update,
                Err(err) => BackgroundUpdate {
                    error: Some(err),
                    ..Default::default()
                },
            };
            let _ = tx.send(result);
        });
        Ok(())
    }

    fn reset_creation(&mut self) {
        self.creation_step = CreationStep::Idle;
        self.creation_name_input.clear();
        self.selected_template_idx = 0;
        self.skip_template_selection = false;
    }

    fn apply(&mut self, result: AppResult<()>) {
        if let Err(err) = result {
            self.last_error = Some(err);
        }
    }

    fn base_block<'a>(title: &'a str) -> Block<'a> {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(CAT_SUBTEXT0))
            .title(title)
    }

    fn highlight_style() -> Style {
        Style::default()
            .bg(CAT_SURFACE0)
            .fg(CAT_MAUVE)
            .add_modifier(Modifier::BOLD)
    }
}

fn parse_porcelain_status(raw: &str) -> Vec<String> {
    if raw.trim().is_empty() {
        return vec!["Clean working tree".to_string()];
    }

    let mut out = Vec::new();
    for line in raw.lines() {
        if line.len() < 3 {
            continue;
        }
        let code = &line[..2];
        let path = line[3..].trim();
        let label = match code {
            "??" => "Untracked",
            " M" | "M " | "MM" => "Modified",
            "A " | " A" | "AM" => "Added",
            "D " | " D" => "Deleted",
            "R " | " R" => "Renamed",
            "C " | " C" => "Copied",
            "UU" => "Unmerged",
            _ => "Changed",
        };
        out.push(format!("{label}: {path}"));
    }
    out
}

fn find_heavy_artifacts(project_root: &Path) -> AppResult<Vec<CleanerItem>> {
    let targets: HashSet<&str> = ["target", "node_modules", "build", "dist"]
        .iter()
        .copied()
        .collect();

    let mut items = Vec::new();
    for entry in WalkDir::new(project_root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_dir())
    {
        let name = entry.file_name().to_string_lossy();
        if !targets.contains(name.as_ref()) {
            continue;
        }
        let path = entry.path().to_path_buf();
        let size_bytes = directory_size(&path)?;
        items.push(CleanerItem {
            path,
            size_bytes,
            kind: name.to_string(),
        });
    }
    items.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    Ok(items)
}

fn directory_size(dir: &Path) -> AppResult<u64> {
    let mut total = 0_u64;
    for entry in WalkDir::new(dir).into_iter().filter_map(Result::ok) {
        if entry.file_type().is_file() {
            let meta = entry.metadata().map_err(|e| {
                format!(
                    "Unable to read metadata for {}: {e}",
                    entry.path().display()
                )
            })?;
            total = total.saturating_add(meta.len());
        }
    }
    Ok(total)
}

fn find_env_files(project_root: &Path) -> Vec<PathBuf> {
    let mut env_files = Vec::new();
    for entry in WalkDir::new(project_root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        if entry.file_name() == ".env" {
            env_files.push(entry.path().to_path_buf());
        }
    }
    env_files
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn format_unix_time(unix: i64) -> String {
    if unix == 0 {
        return "Never".to_string();
    }
    let datetime = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(unix as u64);
    match std::time::SystemTime::now().duration_since(datetime) {
        Ok(duration) => {
            let secs = duration.as_secs();
            if secs < 60 {
                "Just now".to_string()
            } else if secs < 3600 {
                format!("{}m ago", secs / 60)
            } else if secs < 86400 {
                format!("{}h ago", secs / 3600)
            } else {
                format!("{}d ago", secs / 86400)
            }
        }
        Err(_) => "Future".to_string(),
    }
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn is_env_encrypted(path: &Path) -> bool {
    if let Ok(bytes) = fs::read(path) {
        bytes.len() >= 8 && &bytes[..8] == b"UNITENV1"
    } else {
        false
    }
}

fn get_directory_tree_lines(
    path: &Path,
    prefix: &str,
    depth: usize,
    max_depth: usize,
) -> Vec<Line<'static>> {
    if depth > max_depth {
        return vec![Line::from(vec![
            Span::styled(prefix.to_string(), Style::default().fg(CAT_SUBTEXT0)),
            Span::styled(
                "... (depth limit reached)",
                Style::default()
                    .fg(CAT_SUBTEXT0)
                    .add_modifier(Modifier::ITALIC),
            ),
        ])];
    }
    let mut lines = Vec::new();
    let entries = match fs::read_dir(path) {
        Ok(read) => {
            let mut list = Vec::new();
            for entry in read.filter_map(Result::ok) {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == "target"
                    || name == "node_modules"
                    || name == ".git"
                    || name == "build"
                    || name == "dist"
                    || (name.starts_with('.')
                        && name != ".env"
                        && name != ".gitignore"
                        && name != ".unit-template")
                {
                    continue;
                }
                list.push(entry);
            }
            list.sort_by(|a, b| {
                let a_is_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
                let b_is_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if a_is_dir != b_is_dir {
                    b_is_dir.cmp(&a_is_dir)
                } else {
                    a.file_name().cmp(&b.file_name())
                }
            });
            list
        }
        Err(_) => return lines,
    };

    let len = entries.len();
    for (i, entry) in entries.iter().enumerate() {
        let is_last = i == len - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let new_prefix = if is_last { "    " } else { "│   " };
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);

        if is_dir {
            lines.push(Line::from(vec![
                Span::styled(prefix.to_string(), Style::default().fg(CAT_SUBTEXT0)),
                Span::styled(connector.to_string(), Style::default().fg(CAT_SUBTEXT0)),
                Span::styled(
                    format!("{file_name}/"),
                    Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
                ),
            ]));
            let sub_lines = get_directory_tree_lines(
                &entry.path(),
                &format!("{prefix}{new_prefix}"),
                depth + 1,
                max_depth,
            );
            lines.extend(sub_lines);
        } else {
            let file_color = if file_name == ".env" {
                CAT_RED
            } else if file_name == "Cargo.toml" || file_name == "package.json" {
                CAT_MAUVE
            } else {
                CAT_TEXT
            };
            lines.push(Line::from(vec![
                Span::styled(prefix.to_string(), Style::default().fg(CAT_SUBTEXT0)),
                Span::styled(connector.to_string(), Style::default().fg(CAT_SUBTEXT0)),
                Span::styled(file_name, Style::default().fg(file_color)),
            ]));
        }
    }
    lines
}

fn get_template_tree_lines(template: &TemplateDef) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    lines.push(Line::from(vec![
        Span::styled(". (template: ", Style::default().fg(CAT_SUBTEXT0)),
        Span::styled(
            template.name.clone(),
            Style::default().fg(CAT_MAUVE).add_modifier(Modifier::BOLD),
        ),
        Span::styled(")", Style::default().fg(CAT_SUBTEXT0)),
    ]));

    let mut dirs: Vec<&String> = template.structure.keys().collect();
    dirs.sort();

    let dirs_len = dirs.len();
    for (i, dir_name) in dirs.iter().enumerate() {
        let is_last_dir = i == dirs_len - 1;
        let dir_connector = if is_last_dir {
            "└── "
        } else {
            "├── "
        };
        let dir_prefix = if is_last_dir { "    " } else { "│   " };

        lines.push(Line::from(vec![
            Span::styled(dir_connector.to_string(), Style::default().fg(CAT_SUBTEXT0)),
            Span::styled(
                (*dir_name).clone(),
                Style::default().fg(CAT_BLUE).add_modifier(Modifier::BOLD),
            ),
        ]));

        if let Some(files_map) = template.structure.get(*dir_name) {
            let mut file_names: Vec<&String> = files_map.keys().collect();
            file_names.sort();
            let files_len = file_names.len();
            for (j, file_name) in file_names.iter().enumerate() {
                let is_last_file = j == files_len - 1;
                let file_connector = if is_last_file {
                    "└── "
                } else {
                    "├── "
                };
                lines.push(Line::from(vec![
                    Span::styled(dir_prefix.to_string(), Style::default().fg(CAT_SUBTEXT0)),
                    Span::styled(
                        file_connector.to_string(),
                        Style::default().fg(CAT_SUBTEXT0),
                    ),
                    Span::styled((*file_name).clone(), Style::default().fg(CAT_TEXT)),
                ]));
            }
        }
    }

    if template.structure.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "└── (empty structure)",
            Style::default()
                .fg(CAT_SUBTEXT0)
                .add_modifier(Modifier::ITALIC),
        )]));
    }

    lines
}
