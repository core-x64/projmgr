use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs},
};
use std::io;

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
            AppTab::Projects => " Projects ",
            AppTab::Cleaner => " Cleaner ",
            AppTab::Secrets => " Secrets ",
            AppTab::Templates => " Templates ",
            AppTab::Git => " Git ",
        }
    }
}

pub struct App {
    pub exit: bool,
    tab: AppTab,

    // Mock Data States
    project_state: ListState,
    projects: Vec<&'static str>,

    template_state: ListState,
    templates: Vec<&'static str>,
}

impl App {
    pub fn new() -> Self {
        let mut app = Self {
            exit: false,
            tab: AppTab::Projects,
            project_state: ListState::default(),
            projects: vec![
                "nexus-api (Rust)",
                "portfolio-web (Vue)",
                "game-engine (C++)",
                "auth-service (Go)",
            ],
            template_state: ListState::default(),
            templates: vec!["Standard", "DLL", "Raylib", "Vite-React"],
        };
        app.project_state.select(Some(0));
        app.template_state.select(Some(0));
        app
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            if let Event::Key(key) = event::read()? {
                self.handle_key_event(key)?;
            }
        }
        Ok(())
    }

    fn draw(&mut self, frame: &mut Frame) {
        let size = frame.area();

        // Layout: Tab bar (Top), Main Content (Middle), Footer (Bottom)
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
    }

    fn draw_tabs(&self, frame: &mut Frame, area: Rect) {
        let titles: Vec<Line> = AppTab::ALL
            .iter()
            .map(|t| Line::from(t.title().bold()))
            .collect();

        let tabs = Tabs::new(titles)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Project Manager "),
            )
            .highlight_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::REVERSED),
            )
            .select(AppTab::ALL.iter().position(|&t| t == self.tab).unwrap());

        frame.render_widget(tabs, area);
    }

    // --- TAB RENDERING METHODS ---

    fn draw_projects(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
            .split(area);

        let items: Vec<ListItem> = self.projects.iter().map(|p| ListItem::new(*p)).collect();
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Tracked Projects "),
            )
            .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
            .highlight_symbol(">> ");
        frame.render_stateful_widget(list, chunks[0], &mut self.project_state);

        let git_status = "Modified Files:\n  M src/main.rs\n  M Cargo.toml\n  ?? .env";
        let detail = Paragraph::new(git_status)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Git Status (Parsed) "),
            )
            .style(Style::default().fg(Color::Yellow));
        frame.render_widget(detail, chunks[1]);
    }

    fn draw_cleaner(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(area);

        let items: Vec<ListItem> = vec![
            ListItem::new("portfolio-web (1.2 GB)"),
            ListItem::new("nexus-api (850 MB)"),
        ];
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Heavy Projects "),
            )
            .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
            .highlight_symbol(">> ");

        let mut state = ListState::default();
        state.select(Some(0));
        frame.render_stateful_widget(list, chunks[0], &mut state);

        let cleanup_details = "Artifacts Detected:\n  - node_modules/ (1.1 GB)\n  - dist/ (100 MB)\n\nAction: Ready to delete or archive.";
        let detail = Paragraph::new(cleanup_details)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Artifact Details "),
            )
            .style(Style::default().fg(Color::Red));
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
                .map(|p| ListItem::new(*p))
                .collect::<Vec<_>>(),
        )
        .block(Block::default().borders(Borders::ALL).title(" Projects "))
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol(">> ");
        frame.render_stateful_widget(list, chunks[0], &mut self.project_state.clone());

        let env_content =
            "# .env (Encrypted)\nDATABASE_URL=********\nAPI_KEY=********\n\n[Status: Locked]";
        let detail = Paragraph::new(env_content)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Environment Variables "),
            )
            .style(Style::default().fg(Color::Magenta));
        frame.render_widget(detail, chunks[1]);
    }

    fn draw_templates(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
            .split(area);

        let items: Vec<ListItem> = self.templates.iter().map(|t| ListItem::new(*t)).collect();
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Installed Templates "),
            )
            .highlight_style(Style::default().bg(Color::DarkGray))
            .highlight_symbol(">> ");
        frame.render_stateful_widget(list, chunks[0], &mut self.template_state);

        let template_preview = "[template]\nname = \"Standard\"\nlanguage = \"Rust\"\n\n[features]\nlinting = true\ndocker = false";
        let detail = Paragraph::new(template_preview)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Template Config Preview "),
            )
            .style(Style::default().fg(Color::Cyan));
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
                .map(|p| ListItem::new(*p))
                .collect::<Vec<_>>(),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Repositories "),
        )
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol(">> ");
        frame.render_stateful_widget(list, chunks[0], &mut self.project_state.clone());

        let git_actions = "Available Git Actions:\n\n[c] Commit\n[s] Switch Branch\n[b] Create Branch\n[p] Push\n[l] Pull\n[m] Merge Branch";
        let detail = Paragraph::new(git_actions)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Git Actions Menu "),
            )
            .style(Style::default().fg(Color::Green));
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
                " [←/→] Tab | [↑/↓] Navigate | [q] Quit ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(binds, Style::default().fg(Color::White).bold()),
        ]);

        let block = Block::default().borders(Borders::ALL);
        let paragraph = Paragraph::new(instructions).block(block).centered();
        frame.render_widget(paragraph, area);
    }

    // --- EVENT HANDLING ---

    fn handle_key_event(&mut self, key: KeyEvent) -> io::Result<()> {
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        // Global keybinds
        match key.code {
            KeyCode::Char('q') => self.exit = true,
            KeyCode::Right => self.tab = self.tab.next(),
            KeyCode::Left => self.tab = self.tab.previous(),
            KeyCode::Down => self.list_next(),
            KeyCode::Up => self.list_previous(),
            _ => {}
        }

        // Context-sensitive keybinds (where you will implement functionality later)
        match (self.tab, key.code) {
            (AppTab::Projects, KeyCode::Char('c')) => { /* Create project */ }
            (AppTab::Projects, KeyCode::Char('a')) => { /* Add project */ }
            (AppTab::Projects, KeyCode::Char('d')) => { /* Delete project */ }
            (AppTab::Projects, KeyCode::Char('o')) => { /* Open IDE */ }

            (AppTab::Cleaner, KeyCode::Char('d')) => { /* Delete artifacts */ }
            (AppTab::Cleaner, KeyCode::Char('a')) => { /* Archive project */ }

            (AppTab::Secrets, KeyCode::Char('e')) => { /* Encrypt */ }
            (AppTab::Secrets, KeyCode::Char('d')) => { /* Decrypt */ }

            (AppTab::Templates, KeyCode::Char('a')) => { /* Create template */ }
            _ => {}
        }

        Ok(())
    }

    fn list_next(&mut self) {
        match self.tab {
            AppTab::Projects | AppTab::Secrets | AppTab::Git => {
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
            AppTab::Templates => {
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
            _ => {}
        }
    }

    fn list_previous(&mut self) {
        match self.tab {
            AppTab::Projects | AppTab::Secrets | AppTab::Git => {
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
            AppTab::Templates => {
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
            _ => {}
        }
    }
}
