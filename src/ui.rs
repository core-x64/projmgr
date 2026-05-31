use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs},
};
use std::io;

const CAT_MAUVE: Color = Color::Rgb(203, 166, 247);
const CAT_BLUE: Color = Color::Rgb(137, 180, 250);
const CAT_GREEN: Color = Color::Rgb(166, 227, 161);
const CAT_RED: Color = Color::Rgb(243, 139, 168);
const CAT_PEACH: Color = Color::Rgb(250, 179, 135);
const CAT_SURFACE0: Color = Color::Rgb(49, 50, 68);
const CAT_TEXT: Color = Color::Rgb(205, 214, 244);
const CAT_SUBTEXT0: Color = Color::Rgb(166, 173, 200);

pub struct Project {
    pub name: String,
    pub path: String,
    pub git_status_parsed: String,
    pub env_content: Option<String>,
}

pub struct CleanerItem {
    pub project_name: String,
    pub size_str: String,
    pub artifact_details: String,
}

pub struct TemplateDef {
    pub name: String,
    pub preview: String,
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
        let current = Self::ALL.iter().position(|&t| t == self).unwrap();
        Self::ALL[(current + 1) % Self::ALL.len()]
    }

    fn previous(self) -> Self {
        let current = Self::ALL.iter().position(|&t| t == self).unwrap();
        Self::ALL[(current + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    fn title(self) -> &'static str {
        match self {
            AppTab::Projects => " 󰆧 Projects ",
            AppTab::Cleaner => " 󰃢 Cleaner ",
            AppTab::Secrets => " 󰌆 Secrets ",
            AppTab::Templates => " 󰏪 Templates ",
            AppTab::Git => " 󰊢 Git ",
        }
    }
}

#[derive(PartialEq)]
enum InputMode {
    Normal,
    Editing,
}

pub struct App {
    pub exit: bool,
    tab: AppTab,

    input_mode: InputMode,
    input_text: String,
    input_title: String,

    projects: Vec<Project>,
    cleaner_items: Vec<CleanerItem>,
    templates: Vec<TemplateDef>,

    project_state: ListState,
    cleaner_state: ListState,
    template_state: ListState,
}

impl App {
    pub fn new() -> Self {
        Self {
            exit: false,
            tab: AppTab::Projects,
            input_mode: InputMode::Normal,
            input_text: String::new(),
            input_title: String::new(),

            projects: Vec::new(),
            cleaner_items: Vec::new(),
            templates: Vec::new(),

            project_state: ListState::default(),
            cleaner_state: ListState::default(),
            template_state: ListState::default(),
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match self.input_mode {
                        InputMode::Normal => self.handle_normal_key(key)?,
                        InputMode::Editing => self.handle_editing_key(key)?,
                    }
                }
            }
        }
        Ok(())
    }

    fn draw(&mut self, frame: &mut Frame) {
        let size = frame.area();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(size);

        self.draw_tabs(frame, chunks[0]);

        match self.tab {
            AppTab::Projects => self.draw_projects(frame, chunks[1]),
            AppTab::Cleaner => self.draw_cleaner(frame, chunks[1]),
            AppTab::Secrets => self.draw_secrets(frame, chunks[1]),
            AppTab::Templates => self.draw_templates(frame, chunks[1]),
            AppTab::Git => self.draw_git(frame, chunks[1]),
        }

        self.draw_footer(frame, chunks[2]);

        if self.input_mode == InputMode::Editing {
            self.draw_popup(frame);
        }
    }

    fn base_block(title: &str) -> Block<'static> {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(title.to_string())
    }

    fn draw_tabs(&self, frame: &mut Frame, area: Rect) {
        let titles: Vec<Line> = AppTab::ALL
            .iter()
            .map(|t| Line::from(t.title().bold()))
            .collect();

        let tabs = Tabs::new(titles)
            .block(Self::base_block(" Project Manager ").fg(CAT_TEXT))
            .highlight_style(
                Style::default()
                    .fg(CAT_MAUVE)
                    .add_modifier(Modifier::REVERSED),
            )
            .select(AppTab::ALL.iter().position(|&t| t == self.tab).unwrap());

        frame.render_widget(tabs, area);
    }

    fn get_highlight_style() -> Style {
        Style::default()
            .bg(CAT_SURFACE0)
            .fg(CAT_MAUVE)
            .add_modifier(Modifier::BOLD)
    }

    fn draw_projects(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
            .split(area);

        let items: Vec<ListItem> = self
            .projects
            .iter()
            .map(|p| ListItem::new(p.name.clone()).fg(CAT_TEXT))
            .collect();

        let list = List::new(items)
            .block(Self::base_block(" Tracked Projects ").fg(CAT_SUBTEXT0))
            .highlight_style(Self::get_highlight_style())
            .highlight_symbol(">> ");
        frame.render_stateful_widget(list, chunks[0], &mut self.project_state);

        let detail_text = if let Some(i) = self.project_state.selected() {
            self.projects[i].git_status_parsed.clone()
        } else {
            "No project selected or tracking list is empty.".to_string()
        };

        let detail = Paragraph::new(detail_text)
            .block(Self::base_block(" Git Status (Parsed) ").fg(CAT_SUBTEXT0))
            .style(Style::default().fg(CAT_PEACH));
        frame.render_widget(detail, chunks[1]);
    }

    fn draw_cleaner(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(area);

        let items: Vec<ListItem> = self
            .cleaner_items
            .iter()
            .map(|c| ListItem::new(format!("{} ({})", c.project_name, c.size_str)).fg(CAT_TEXT))
            .collect();

        let list = List::new(items)
            .block(Self::base_block(" Heavy Projects ").fg(CAT_SUBTEXT0))
            .highlight_style(Self::get_highlight_style())
            .highlight_symbol(">> ");

        frame.render_stateful_widget(list, chunks[0], &mut self.cleaner_state);

        let detail_text = if let Some(i) = self.cleaner_state.selected() {
            self.cleaner_items[i].artifact_details.clone()
        } else {
            "No project selected or no heavy artifacts detected.".to_string()
        };

        let detail = Paragraph::new(detail_text)
            .block(Self::base_block(" Artifact Details ").fg(CAT_SUBTEXT0))
            .style(Style::default().fg(CAT_RED));
        frame.render_widget(detail, chunks[1]);
    }

    fn draw_secrets(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
            .split(area);

        let list = List::new(
            self.projects
                .iter()
                .map(|p| ListItem::new(p.name.clone()).fg(CAT_TEXT))
                .collect::<Vec<_>>(),
        )
        .block(Self::base_block(" Projects ").fg(CAT_SUBTEXT0))
        .highlight_style(Self::get_highlight_style())
        .highlight_symbol(">> ");
        frame.render_stateful_widget(list, chunks[0], &mut self.project_state.clone());

        let detail_text = if let Some(i) = self.project_state.selected() {
            self.projects[i]
                .env_content
                .clone()
                .unwrap_or_else(|| "No .env found or currently locked.".to_string())
        } else {
            "No project selected.".to_string()
        };

        let detail = Paragraph::new(detail_text)
            .block(Self::base_block(" Environment Variables ").fg(CAT_SUBTEXT0))
            .style(Style::default().fg(CAT_MAUVE));
        frame.render_widget(detail, chunks[1]);
    }

    fn draw_templates(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
            .split(area);

        let items: Vec<ListItem> = self
            .templates
            .iter()
            .map(|t| ListItem::new(t.name.clone()).fg(CAT_TEXT))
            .collect();
        let list = List::new(items)
            .block(Self::base_block(" Installed Templates ").fg(CAT_SUBTEXT0))
            .highlight_style(Self::get_highlight_style())
            .highlight_symbol(">> ");
        frame.render_stateful_widget(list, chunks[0], &mut self.template_state);

        let detail_text = if let Some(i) = self.template_state.selected() {
            self.templates[i].preview.clone()
        } else {
            "No template selected.".to_string()
        };

        let detail = Paragraph::new(detail_text)
            .block(Self::base_block(" Template Config Preview ").fg(CAT_SUBTEXT0))
            .style(Style::default().fg(CAT_BLUE));
        frame.render_widget(detail, chunks[1]);
    }

    fn draw_git(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
            .split(area);

        let list = List::new(
            self.projects
                .iter()
                .map(|p| ListItem::new(p.name.clone()).fg(CAT_TEXT))
                .collect::<Vec<_>>(),
        )
        .block(Self::base_block(" Repositories ").fg(CAT_SUBTEXT0))
        .highlight_style(Self::get_highlight_style())
        .highlight_symbol(">> ");
        frame.render_stateful_widget(list, chunks[0], &mut self.project_state.clone());

        let git_actions = "Available Git Actions:\n\n[c] Commit\n[s] Switch Branch\n[b] Create Branch\n[p] Push\n[u] Pull (Update)\n[m] Merge Branch";
        let detail = Paragraph::new(git_actions)
            .block(Self::base_block(" Git Actions Menu ").fg(CAT_SUBTEXT0))
            .style(Style::default().fg(CAT_GREEN));
        frame.render_widget(detail, chunks[1]);
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let binds = match self.tab {
            AppTab::Projects => " [c] Create | [a] Add | [d] Delete | [o] Open IDE ",
            AppTab::Cleaner => " [d] Delete Artifacts | [a] Archive (.tar.gz) ",
            AppTab::Secrets => " [e] Encrypt | [d] Decrypt ",
            AppTab::Templates => " [a] Create New Template ",
            AppTab::Git => " Press corresponding letter to trigger git action ",
        };

        let instructions = Line::from(vec![
            Span::styled(
                " [h/l] Tab | [j/k] Navigate | [q] Quit ",
                Style::default().fg(CAT_SUBTEXT0),
            ),
            Span::styled(binds, Style::default().fg(CAT_TEXT).bold()),
        ]);

        let block = Self::base_block("").fg(CAT_SUBTEXT0);
        let paragraph = Paragraph::new(instructions).block(block).centered();
        frame.render_widget(paragraph, area);
    }

    fn draw_popup(&self, frame: &mut Frame) {
        let area = frame.area();
        let popup_area = centered_rect(50, 20, area);

        frame.render_widget(Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(format!(" {} ", self.input_title))
            .style(Style::default().fg(CAT_MAUVE));

        let display_text = format!("{}█", self.input_text);

        let paragraph = Paragraph::new(display_text)
            .block(block)
            .style(Style::default().fg(CAT_TEXT));

        frame.render_widget(paragraph, popup_area);
    }

    fn trigger_input(&mut self, title: &str) {
        self.input_mode = InputMode::Editing;
        self.input_title = title.to_string();
        self.input_text.clear();
    }

    fn handle_normal_key(&mut self, key: KeyEvent) -> io::Result<()> {
        match key.code {
            KeyCode::Char('q') => self.exit = true,

            KeyCode::Char('l') => self.tab = self.tab.next(),
            KeyCode::Char('h') => self.tab = self.tab.previous(),
            KeyCode::Char('j') => self.list_next(),
            KeyCode::Char('k') => self.list_previous(),
            _ => {}
        }

        match (self.tab, key.code) {
            (AppTab::Projects, KeyCode::Char('c')) => self.trigger_input("Project Name"),
            (AppTab::Projects, KeyCode::Char('a')) => self.trigger_input("Add Project Path"),
            (AppTab::Projects, KeyCode::Char('d')) => self.action_delete_project(),
            (AppTab::Projects, KeyCode::Char('o')) => self.action_open_ide(),

            (AppTab::Cleaner, KeyCode::Char('d')) => self.action_delete_artifacts(),
            (AppTab::Cleaner, KeyCode::Char('a')) => self.action_archive_project(),

            (AppTab::Secrets, KeyCode::Char('e')) => self.trigger_input("Encryption Password"),
            (AppTab::Secrets, KeyCode::Char('d')) => self.trigger_input("Decryption Password"),

            (AppTab::Templates, KeyCode::Char('a')) => self.trigger_input("New Template Name"),

            (AppTab::Git, KeyCode::Char('c')) => self.trigger_input("Commit Message"),
            (AppTab::Git, KeyCode::Char('s')) => self.trigger_input("Switch to Branch"),
            (AppTab::Git, KeyCode::Char('b')) => self.trigger_input("New Branch Name"),
            (AppTab::Git, KeyCode::Char('m')) => self.trigger_input("Merge Branch Name"),
            (AppTab::Git, KeyCode::Char('p')) => self.action_git_push(),
            (AppTab::Git, KeyCode::Char('u')) => self.action_git_pull(),
            _ => {}
        }

        Ok(())
    }

    fn handle_editing_key(&mut self, key: KeyEvent) -> io::Result<()> {
        match key.code {
            KeyCode::Char(c) => self.input_text.push(c),
            KeyCode::Backspace => {
                self.input_text.pop();
            }
            KeyCode::Enter => {
                self.process_input_submission();
                self.input_mode = InputMode::Normal;
            }
            KeyCode::Esc => self.input_mode = InputMode::Normal,
            _ => {}
        }
        Ok(())
    }

    fn list_next(&mut self) {
        match self.tab {
            AppTab::Projects | AppTab::Secrets | AppTab::Git => {
                if self.projects.is_empty() {
                    return;
                }
                let i = match self.project_state.selected() {
                    Some(i) => {
                        if i >= self.projects.len() - 1 {
                            0
                        } else {
                            i + 1
                        }
                    }
                    None => 0,
                };
                self.project_state.select(Some(i));
            }
            AppTab::Cleaner => {
                if self.cleaner_items.is_empty() {
                    return;
                }
                let i = match self.cleaner_state.selected() {
                    Some(i) => {
                        if i >= self.cleaner_items.len() - 1 {
                            0
                        } else {
                            i + 1
                        }
                    }
                    None => 0,
                };
                self.cleaner_state.select(Some(i));
            }
            AppTab::Templates => {
                if self.templates.is_empty() {
                    return;
                }
                let i = match self.template_state.selected() {
                    Some(i) => {
                        if i >= self.templates.len() - 1 {
                            0
                        } else {
                            i + 1
                        }
                    }
                    None => 0,
                };
                self.template_state.select(Some(i));
            }
        }
    }

    fn list_previous(&mut self) {
        match self.tab {
            AppTab::Projects | AppTab::Secrets | AppTab::Git => {
                if self.projects.is_empty() {
                    return;
                }
                let i = match self.project_state.selected() {
                    Some(i) => {
                        if i == 0 {
                            self.projects.len() - 1
                        } else {
                            i - 1
                        }
                    }
                    None => 0,
                };
                self.project_state.select(Some(i));
            }
            AppTab::Cleaner => {
                if self.cleaner_items.is_empty() {
                    return;
                }
                let i = match self.cleaner_state.selected() {
                    Some(i) => {
                        if i == 0 {
                            self.cleaner_items.len() - 1
                        } else {
                            i - 1
                        }
                    }
                    None => 0,
                };
                self.cleaner_state.select(Some(i));
            }
            AppTab::Templates => {
                if self.templates.is_empty() {
                    return;
                }
                let i = match self.template_state.selected() {
                    Some(i) => {
                        if i == 0 {
                            self.templates.len() - 1
                        } else {
                            i - 1
                        }
                    }
                    None => 0,
                };
                self.template_state.select(Some(i));
            }
        }
    }

    fn process_input_submission(&mut self) {
        let input = self.input_text.clone();
        match self.input_title.as_str() {
            "Project Name" => { /* TODO: Create project logic */ }
            "Add Project Path" => { /* TODO: Add project to tracking */ }
            "Encryption Password" => { /* TODO: Encrypt .env */ }
            "Decryption Password" => { /* TODO: Decrypt .env */ }
            "New Template Name" => { /* TODO: Create template */ }
            "Commit Message" => { /* TODO: git commit -m */ }
            "Switch to Branch" => { /* TODO: git checkout */ }
            "New Branch Name" => { /* TODO: git checkout -b */ }
            "Merge Branch Name" => { /* TODO: git merge */ }
            _ => {}
        }
    }

    fn action_delete_project(&mut self) {}

    fn action_open_ide(&mut self) {}

    fn action_delete_artifacts(&mut self) {}

    fn action_archive_project(&mut self) {}

    fn action_git_push(&mut self) {}

    fn action_git_pull(&mut self) {}
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
