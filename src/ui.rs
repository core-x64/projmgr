use std::collections::HashSet;
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
    OpenIde { project_path: PathBuf },
    OpenTemplate { template_path: PathBuf },
    RemoveProject { project_path: PathBuf },
    GitCommit { project_path: PathBuf, message: String },
    GitPush { project_path: PathBuf },
    GitPull { project_path: PathBuf },
    GitBranch { project_path: PathBuf, branch: String },
    GitCheckoutNew { project_path: PathBuf, branch: String },
    GitMerge { project_path: PathBuf, branch: String },
    GitStatus { project_path: PathBuf },
    ScanCleaner { project_path: PathBuf },
    DeleteArtifacts { paths: Vec<PathBuf> },
    ArchiveProject { project_path: PathBuf },
    EncryptEnv { project_path: PathBuf, password: String },
    DecryptEnv { project_path: PathBuf, password: String },
    CreateTemplate,
}

#[derive(Debug, Clone)]
struct BackgroundUpdate {
    status_message: Option<String>,
    error: Option<String>,
    projects: Option<Vec<ProjectData>>,
    templates: Option<Vec<TemplateDef>>,
    cleaner_items: Option<Vec<CleanerItem>>,
    git_summary: Option<Vec<String>>,
    creation_complete: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppTab {
    Projects,
    Cleaner,
    Secrets,
    Templates,
    Git,
}

impl AppTab {
    const ALL: [AppTab; 5] = [
        AppTab::Projects,
        AppTab::Cleaner,
        AppTab::Secrets,
        AppTab::Templates,
        AppTab::Git,
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
            AppTab::Projects => "Projects",
            AppTab::Cleaner => "Cleaner",
            AppTab::Secrets => "Secrets",
            AppTab::Templates => "Templates",
            AppTab::Git => "Git",
        }
    }
}

#[derive(Clone)]
enum PromptAction {
    AddProjectPath,
    GitCommit,
    GitBranch,
    GitCheckoutNew,
    GitMerge,
    EncryptEnv,
    DecryptEnv,
}

#[derive(Clone)]
struct PromptState {
    title: String,
    input: String,
    action: PromptAction,
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
    tab: AppTab,
    project_state: ListState,
    template_state: ListState,
    prompt: Option<PromptState>,
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
            tab: AppTab::Projects,
            project_state: ListState::default(),
            template_state: ListState::default(),
            prompt: None,
        };
        let _ = app.handle_command(Command::RefreshData);
        app
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            self.tick();
            terminal.draw(|frame| self.draw(frame))?;
            if event::poll(Duration::from_millis(16))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        self.on_key(key);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn handle_command(&mut self, command: Command) -> AppResult<()> {
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
                // Create the directory if it doesn't exist
                fs::create_dir_all(&path).map_err(|e| e.to_string())?;
                self.provider.add_project(path)?;
                self.projects = self.provider.get_all_projects()?;
                self.ensure_valid_selection();
                self.status_message = Some("Project added".to_string());
                Ok(())
            }
            Command::OpenIde { project_path } => self.action_open_ide(project_path),
            Command::RemoveProject { project_path } => {
                self.provider.remove_project(&project_path)?;
                self.projects = self.provider.get_all_projects()?;
                self.ensure_valid_selection();
                self.status_message = Some("Project removed".to_string());
                Ok(())
            }
            Command::GitCommit {
                project_path,
                message,
            } => self.action_git(
                project_path,
                vec!["commit".into(), "-m".into(), message],
                Some("Commit complete".into()),
            ),
            Command::GitPush { project_path } => {
                self.action_git(project_path, vec!["push".into()], Some("Push complete".into()))
            }
            Command::GitPull { project_path } => {
                self.action_git(project_path, vec!["pull".into()], Some("Pull complete".into()))
            }
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
            Command::CreateTemplate => self.create_new_template(),
            Command::OpenTemplate { template_path } => self.action_open_template(template_path),
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
                }
                if update.creation_complete {
                    self.reset_creation();
                }
            }
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        if self.prompt.is_some() {
            self.handle_prompt_key(key);
            return;
        }
        if self.creation_step == CreationStep::EnterName {
            self.handle_creation_name_key(key);
            return;
        }
        if self.creation_step == CreationStep::SelectTemplate {
            self.handle_creation_template_key(key);
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.exit = true,
            KeyCode::Char('h') => self.tab = self.tab.previous(),
            KeyCode::Char('l') => self.tab = self.tab.next(),
            KeyCode::Char('j') => self.list_next(),
            KeyCode::Char('k') => self.list_prev(),
            KeyCode::Esc => {
                self.creation_step = CreationStep::Idle;
                self.prompt = None;
            }
            KeyCode::Char('x') => {
                let _ = self.handle_command(Command::DismissError);
            }
            _ => self.handle_tab_shortcuts(key),
        }
    }

    fn handle_tab_shortcuts(&mut self, key: KeyEvent) {
        match (self.tab, key.code) {
            (AppTab::Projects, KeyCode::Char('c')) => {
                let result = self.handle_command(Command::StartCreateProject);
                self.apply(result);
            }
            (AppTab::Projects, KeyCode::Char('a')) => {
                self.prompt = Some(PromptState {
                    title: "Add Project Path".to_string(),
                    input: String::new(),
                    action: PromptAction::AddProjectPath,
                });
            }
            (AppTab::Projects, KeyCode::Char('d')) => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(Command::RemoveProject { project_path: path });
                    self.apply(result);
                }
            }
            (AppTab::Projects, KeyCode::Char('o')) => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(Command::OpenIde { project_path: path });
                    self.apply(result);
                }
            }
            (AppTab::Cleaner, KeyCode::Char('s')) => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(Command::ScanCleaner { project_path: path });
                    self.apply(result);
                }
            }
            (AppTab::Cleaner, KeyCode::Char('d')) => {
                let paths = self
                    .cleaner_items
                    .iter()
                    .map(|item| item.path.clone())
                    .collect::<Vec<_>>();
                let result = self.handle_command(Command::DeleteArtifacts { paths });
                self.apply(result);
            }
            (AppTab::Cleaner, KeyCode::Char('a')) => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(Command::ArchiveProject { project_path: path });
                    self.apply(result);
                }
            }
            (AppTab::Secrets, KeyCode::Char('e')) => {
                self.prompt = Some(PromptState {
                    title: "Encryption Password".to_string(),
                    input: String::new(),
                    action: PromptAction::EncryptEnv,
                });
            }
            (AppTab::Secrets, KeyCode::Char('d')) => {
                self.prompt = Some(PromptState {
                    title: "Decryption Password".to_string(),
                    input: String::new(),
                    action: PromptAction::DecryptEnv,
                });
            }
            (AppTab::Templates, KeyCode::Char('a')) => {
                let result = self.handle_command(Command::CreateTemplate);
                self.apply(result);
            }
            (AppTab::Templates, KeyCode::Char('o')) => {
                if let Some(template) = self.selected_template() {
                    if let Some(home_dir) = dirs::home_dir() {
                        let template_dir = home_dir.join(".config/unit-projman/templates");
                        let template_path = template_dir.join(format!("{}.json", template.name));
                        let result = self.handle_command(Command::OpenTemplate { template_path });
                        self.apply(result);
                    } else {
                        self.last_error = Some("Unable to resolve HOME directory".to_string());
                    }
                }
            }
            (AppTab::Git, KeyCode::Char('c')) => {
                self.prompt = Some(PromptState {
                    title: "Commit Message".to_string(),
                    input: String::new(),
                    action: PromptAction::GitCommit,
                });
            }
            (AppTab::Git, KeyCode::Char('b')) => {
                self.prompt = Some(PromptState {
                    title: "New Branch Name".to_string(),
                    input: String::new(),
                    action: PromptAction::GitCheckoutNew,
                });
            }
            (AppTab::Git, KeyCode::Char('n')) => {
                self.prompt = Some(PromptState {
                    title: "Create Branch (git branch)".to_string(),
                    input: String::new(),
                    action: PromptAction::GitBranch,
                });
            }
            (AppTab::Git, KeyCode::Char('m')) => {
                self.prompt = Some(PromptState {
                    title: "Merge Branch".to_string(),
                    input: String::new(),
                    action: PromptAction::GitMerge,
                });
            }
            (AppTab::Git, KeyCode::Char('p')) => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(Command::GitPush { project_path: path });
                    self.apply(result);
                }
            }
            (AppTab::Git, KeyCode::Char('u')) => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(Command::GitPull { project_path: path });
                    self.apply(result);
                }
            }
            (AppTab::Git, KeyCode::Char('s')) => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(Command::GitStatus { project_path: path });
                    self.apply(result);
                }
            }
            _ => {}
        }
    }

    fn handle_prompt_key(&mut self, key: KeyEvent) {
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
                        self.execute_prompt(state);
                    }
                }
                _ => {}
            }
        }
    }

    fn execute_prompt(&mut self, prompt: PromptState) {
        let input = prompt.input.trim().to_string();
        if input.is_empty() {
            self.last_error = Some("Input cannot be empty".to_string());
            return;
        }
        match prompt.action {
            PromptAction::AddProjectPath => {
                let result = self.handle_command(Command::AddProjectPath(PathBuf::from(input)));
                self.apply(result);
            }
            PromptAction::GitCommit => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(Command::GitCommit {
                        project_path: path,
                        message: input,
                    });
                    self.apply(result);
                }
            }
            PromptAction::GitBranch => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(Command::GitBranch {
                        project_path: path,
                        branch: input,
                    });
                    self.apply(result);
                }
            }
            PromptAction::GitCheckoutNew => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(Command::GitCheckoutNew {
                        project_path: path,
                        branch: input,
                    });
                    self.apply(result);
                }
            }
            PromptAction::GitMerge => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(Command::GitMerge {
                        project_path: path,
                        branch: input,
                    });
                    self.apply(result);
                }
            }
            PromptAction::EncryptEnv => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(Command::EncryptEnv {
                        project_path: path,
                        password: input,
                    });
                    self.apply(result);
                }
            }
            PromptAction::DecryptEnv => {
                if let Some(path) = self.selected_project_path() {
                    let result = self.handle_command(Command::DecryptEnv {
                        project_path: path,
                        password: input,
                    });
                    self.apply(result);
                }
            }
        }
    }

    fn handle_creation_name_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                let result = self.handle_command(Command::CancelCreation);
                self.apply(result);
            }
            KeyCode::Backspace => {
                self.creation_name_input.pop();
                let result = self.handle_command(Command::UpdateCreationName(self.creation_name_input.clone()));
                self.apply(result);
            }
            KeyCode::Char(c) => {
                self.creation_name_input.push(c);
                let result = self.handle_command(Command::UpdateCreationName(self.creation_name_input.clone()));
                self.apply(result);
            }
            KeyCode::Enter => {
                let result = self.handle_command(Command::CreationSubmitName);
                self.apply(result);
            }
            _ => {}
        }
    }

    fn handle_creation_template_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                let result = self.handle_command(Command::CancelCreation);
                self.apply(result);
            }
            KeyCode::Char('j') => {
                let result = self.handle_command(Command::CreationTemplateNext);
                self.apply(result);
            }
            KeyCode::Char('k') => {
                let result = self.handle_command(Command::CreationTemplatePrev);
                self.apply(result);
            }
            KeyCode::Enter => {
                let result = self.handle_command(Command::CreationTemplateConfirm);
                self.apply(result);
            }
            _ => {}
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        let size = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0), Constraint::Length(3)])
            .split(size);
        self.draw_tabs(frame, chunks[0]);
        self.draw_body(frame, chunks[1]);
        self.draw_footer(frame, chunks[2]);
        self.draw_overlay(frame);
    }

    fn draw_tabs(&self, frame: &mut Frame, area: Rect) {
        let titles = AppTab::ALL
            .iter()
            .map(|tab| Line::from(tab.title().to_string()))
            .collect::<Vec<_>>();
        let selected = AppTab::ALL.iter().position(|tab| *tab == self.tab).unwrap_or(0);
        let tabs = Tabs::new(titles)
            .block(Self::base_block(" Unit Project Manager "))
            .style(Style::default().fg(CAT_SUBTEXT0))
            .highlight_style(Style::default().fg(CAT_MAUVE).add_modifier(Modifier::BOLD))
            .select(selected);
        frame.render_widget(tabs, area);
    }

    fn draw_body(&mut self, frame: &mut Frame, area: Rect) {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(area);

        let project_items = self
            .projects
            .iter()
            .map(|p| {
                let tpl = p.template_name.clone().unwrap_or_else(|| "-".to_string());
                ListItem::new(format!("{}  [{}]", p.name, tpl))
            })
            .collect::<Vec<_>>();

        let project_list = List::new(project_items)
            .block(Self::base_block(" Projects "))
            .highlight_symbol(">> ")
            .highlight_style(Self::highlight_style());
        frame.render_stateful_widget(project_list, body[0], &mut self.project_state);

        let right = match self.tab {
            AppTab::Projects => self.projects_panel_text(),
            AppTab::Cleaner => self.cleaner_panel_text(),
            AppTab::Secrets => self.secrets_panel_text(),
            AppTab::Templates => self.templates_panel_text(),
            AppTab::Git => self.git_panel_text(),
        };
        let panel = Paragraph::new(right)
            .block(Self::base_block(" Details "))
            .style(Style::default().fg(CAT_TEXT));
        frame.render_widget(panel, body[1]);
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let spinner = if self.is_loading {
            Some(['⠋', '⠙', '⠹', '⠸'][self.spinner_index])
        } else {
            None
        };
        let status = self
            .status_message
            .clone()
            .unwrap_or_else(|| "Ready".to_string());
        let line = Line::from(vec![
            Span::styled("h/l tabs • j/k list • q quit • x clear error", Style::default().fg(CAT_SUBTEXT0)),
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

    fn draw_overlay(&mut self, frame: &mut Frame) {
        if self.creation_step == CreationStep::EnterName {
            self.draw_input_popup(frame, "Step 1: Enter Project Name", &self.creation_name_input);
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
        state.select(Some(self.selected_template_idx.min(self.templates.len().saturating_sub(1))));
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
        let paragraph = Paragraph::new(format!("{value}█")).block(block).style(Style::default().fg(CAT_TEXT));
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
        let paragraph = Paragraph::new(error).block(block).style(Style::default().fg(CAT_TEXT));
        frame.render_widget(paragraph, area);
    }

    fn projects_panel_text(&self) -> String {
        let selected = self
            .selected_project()
            .map(|p| format!("Selected: {}\nPath: {}", p.name, p.path.display()))
            .unwrap_or_else(|| "No project selected".to_string());
        format!("{selected}\n\nActions:\n[c] create wizard\n[a] add existing path\n[d] remove tracked project\n[o] open in Neovim")
    }

    fn cleaner_panel_text(&self) -> String {
        let mut lines = vec![
            "Actions: [s] scan heavy dirs  [d] delete artifacts  [a] archive project".to_string(),
            "".to_string(),
        ];
        if self.cleaner_items.is_empty() {
            lines.push("No cleaner data. Run scan.".to_string());
        } else {
            for item in &self.cleaner_items {
                lines.push(format!(
                    "{} ({}) - {} bytes",
                    item.path.display(),
                    item.kind,
                    item.size_bytes
                ));
            }
        }
        lines.join("\n")
    }

    fn secrets_panel_text(&self) -> String {
        let selected = self.selected_project();
        let mut lines = vec!["Actions:".to_string()];

        if let Some(project) = selected {
            // Find all .env files in the project
            let env_files = find_env_files(&project.path);
            if env_files.is_empty() {
                lines.push("No .env files found in project".to_string());
            } else {
                lines.push(format!("Found {} .env files:", env_files.len()));
                for file in &env_files {
                    lines.push(format!("  - {}", file.display()));
                }
            }
            lines.push("".to_string());
            lines.push("[e] encrypt selected .env file".to_string());
            lines.push("[d] decrypt selected .env file".to_string());
        } else {
            lines.push("No project selected".to_string());
        }

        lines.join("\n")
    }

    fn templates_panel_text(&self) -> String {
        let mut lines = vec!["Templates loaded from ~/.config/unit-projman/templates/".to_string(), "".to_string()];
        for t in &self.templates {
            let file_count: usize = t.structure.values().map(|files| files.len()).sum();
            lines.push(format!("{}: {} files", t.name, file_count));
        }
        if self.templates.is_empty() {
            lines.push("No templates found.".to_string());
        }
        lines.join("\n")
    }

    fn git_panel_text(&self) -> String {
        let mut lines = vec![
            "Actions:".to_string(),
            "[s] git status --porcelain (parsed)".to_string(),
            "[c] commit  [p] push  [u] pull".to_string(),
            "[n] git branch <name>".to_string(),
            "[b] git checkout -b <name>".to_string(),
            "[m] git merge <branch>".to_string(),
            "".to_string(),
            "Status:".to_string(),
        ];
        if self.git_status_summary.is_empty() {
            lines.push("No parsed status yet.".to_string());
        } else {
            lines.extend(self.git_status_summary.clone());
        }
        lines.join("\n")
    }

    fn list_next(&mut self) {
        let next_idx = match self.project_state.selected() {
            Some(i) if !self.projects.is_empty() => (i + 1) % self.projects.len(),
            _ if !self.projects.is_empty() => 0,
            _ => return,
        };
        self.project_state.select(Some(next_idx));
    }

    fn list_prev(&mut self) {
        let prev_idx = match self.project_state.selected() {
            Some(0) if !self.projects.is_empty() => self.projects.len() - 1,
            Some(i) if !self.projects.is_empty() => i.saturating_sub(1),
            _ if !self.projects.is_empty() => 0,
            _ => return,
        };
        self.project_state.select(Some(prev_idx));
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
        self.creation_step = CreationStep::SelectTemplate;
        Ok(())
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

            provider.add_project(project_path.clone())?;
            let projects = provider.get_all_projects()?;
            Ok(BackgroundUpdate {
                status_message: Some(format!("Created project '{project_name}'")),
                error: None,
                projects: Some(projects),
                templates: None,
                cleaner_items: None,
                git_summary: None,
                creation_complete: true,
            })
        })
    }

    fn action_open_ide(&mut self, project_path: PathBuf) -> AppResult<()> {
        self.spawn_task(move || {
            ProcessCommand::new("nvim")
                .arg(&project_path)
                .spawn()
                .map_err(|e| {
                    format!(
                        "Unable to launch Neovim for {}. Ensure 'nvim' is in PATH. {e}",
                        project_path.display()
                    )
                })?;

            Ok(BackgroundUpdate {
                status_message: Some(format!("Opened IDE for {}", project_path.display())),
                error: None,
                projects: None,
                templates: None,
                cleaner_items: None,
                git_summary: None,
                creation_complete: false,
            })
        })
    }

    fn action_open_template(&mut self, template_path: PathBuf) -> AppResult<()> {
        self.spawn_task(move || {
            ProcessCommand::new("nvim")
                .arg(&template_path)
                .spawn()
                .map_err(|e| {
                    format!(
                        "Unable to open template in Neovim. Ensure 'nvim' is in PATH. {e}",
                        e = template_path.display()
                    )
                })?;

            Ok(BackgroundUpdate {
                status_message: Some(format!("Opened template {}", template_path.display())),
                error: None,
                projects: None,
                cleaner_items: None,
                git_summary: None,
                creation_complete: false,
            })
        })
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
                    status_message: None,
                    error: Some(format!("Git command failed: {}", stderr.trim())),
                    projects: None,
                    cleaner_items: None,
                    git_summary: None,
                    creation_complete: false,
                });
            }

            Ok(BackgroundUpdate {
                status_message: status_success,
                error: None,
                projects: None,
                cleaner_items: None,
                git_summary: None,
                creation_complete: false,
            })
        })
    }

    fn action_git_status(&mut self, project_path: PathBuf) -> AppResult<()> {
        self.spawn_task(move || {
            let output = ProcessCommand::new("git")
                .args(["status", "--porcelain"])
                .current_dir(&project_path)
                .output()
                .map_err(|e| format!("Failed to execute git status in {}: {e}", project_path.display()))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                return Ok(BackgroundUpdate {
                    status_message: None,
                    error: Some(format!("git status failed: {}", stderr.trim())),
                    projects: None,
                    cleaner_items: None,
                    git_summary: None,
                    creation_complete: false,
                });
            }

            let parsed = parse_porcelain_status(&String::from_utf8_lossy(&output.stdout));
            Ok(BackgroundUpdate {
                status_message: Some("Git status refreshed".to_string()),
                error: None,
                projects: None,
                cleaner_items: None,
                git_summary: Some(parsed),
                creation_complete: false,
            })
        })
    }

    fn action_scan_cleaner(&mut self, project_path: PathBuf) -> AppResult<()> {
        self.spawn_task(move || {
            let items = find_heavy_artifacts(&project_path)?;
            Ok(BackgroundUpdate {
                status_message: Some("Cleaner scan complete".to_string()),
                error: None,
                projects: None,
                cleaner_items: Some(items),
                git_summary: None,
                creation_complete: false,
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
                    status_message: None,
                    error: Some(format!("Artifact cleanup failed: {}", stderr.trim())),
                    projects: None,
                    cleaner_items: None,
                    git_summary: None,
                    creation_complete: false,
                });
            }
            Ok(BackgroundUpdate {
                status_message: Some("Artifacts deleted".to_string()),
                error: None,
                projects: None,
                cleaner_items: None,
                git_summary: None,
                creation_complete: false,
            })
        })
    }

    fn action_archive_project(&mut self, project_path: PathBuf) -> AppResult<()> {
        self.spawn_task(move || {
            let backups_dir = PathBuf::from("/backups");
            fs::create_dir_all(&backups_dir)
                .map_err(|e| format!("Unable to create backups directory {}: {e}", backups_dir.display()))?;

            let project_name = project_path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| format!("Unable to derive project name from {}", project_path.display()))?;
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
                return Ok(BackgroundUpdate {
                    status_message: None,
                    error: Some(format!("Archive failed: {}", stderr.trim())),
                    projects: None,
                    cleaner_items: None,
                    git_summary: None,
                    creation_complete: false,
                });
            }

            Ok(BackgroundUpdate {
                status_message: Some(format!("Archived to {}", archive_path.display())),
                error: None,
                projects: None,
                cleaner_items: None,
                git_summary: None,
                creation_complete: false,
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
                error: None,
                projects: None,
                cleaner_items: None,
                git_summary: None,
                creation_complete: false,
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
                error: None,
                projects: None,
                cleaner_items: None,
                git_summary: None,
                creation_complete: false,
            })
        })
    }

    fn create_new_template(&mut self) -> AppResult<()> {
        self.spawn_task(move || {
            // Create a simple template config
            let home = dirs::home_dir().ok_or_else(|| "Unable to resolve HOME directory")?;
            let template_dir = home.join(".config/unit-projman/templates");
            fs::create_dir_all(&template_dir).map_err(|e| e.to_string())?;

            // Prompt user for template name
            // For now, we'll create a default template - in a real implementation,
            // this would open a prompt for the template name
            let template_name = "default";
            let template_path = template_dir.join(format!("{}.json", template_name));

            // Create a basic template file using JSON structure
            use std::collections::HashMap;
            let mut structure = HashMap::new();

            // Add src/ directory with files
            let mut src_files = HashMap::new();
            src_files.insert("README.md".to_string(), "# Project\n\nA new project created with unit-projman.".to_string());
            src_files.insert("main.rs".to_string(), "fn main() {\n    println!(\"Hello, world!\");\n}".to_string());
            structure.insert("src/".to_string(), src_files);

            // Add empty include/ directory for future use
            let include_files = HashMap::new();
            structure.insert("include/".to_string(), include_files);

            let template_def = TemplateDef {
                name: template_name.to_string(),
                structure,
            };

            let template_content = serde_json::to_string_pretty(&template_def)
                .map_err(|e| format!("Failed to serialize template: {e}"))?;

            fs::write(&template_path, template_content).map_err(|e| e.to_string())?;

            // Refresh templates list after creation
            let templates = self.provider.get_templates()?;

            Ok(BackgroundUpdate {
                status_message: Some(format!("Created template '{}'", template_name)),
                error: None,
                projects: None,
                templates: Some(templates),
                cleaner_items: None,
                git_summary: None,
                creation_complete: false,
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
                    status_message: None,
                    error: Some(err),
                    projects: None,
                    templates: None,
                    cleaner_items: None,
                    git_summary: None,
                    creation_complete: false,
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
            let meta = entry
                .metadata()
                .map_err(|e| format!("Unable to read metadata for {}: {e}", entry.path().display()))?;
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