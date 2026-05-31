use clap::{Parser, ValueEnum};
use std::io;

mod ui;
mod data;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Language {
    #[clap(alias = "rs")]
    Rust,
    #[clap(alias = "py")]
    Python,
    Go,
    #[clap(alias = "ts")]
    TypeScript,
    #[clap(alias = "c++")]
    Cpp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Template {
    #[clap(alias = "std")]
    Standard,
    Dll,
    Raylib,
    Vite,
}

#[derive(Parser, Debug)]
#[command(
    name = env!("CARGO_PKG_NAME"),
    version = env!("CARGO_PKG_VERSION"),
    propagate_version = true 
)]
pub struct CliArgs {
    #[arg(short, long, default_value_t = false)]
    pub quiet: bool,

    #[arg(short, long, value_enum)]
    pub language: Language,

    #[arg(short, long, value_enum)]
    pub template: Template,

    #[arg(value_name = "NAME")]
    pub name: String,
}

fn validate_args(args: &CliArgs) -> Result<(), String> {
    match args.template {
        Template::Standard => {} 
        Template::Raylib => {
            if args.language != Language::Cpp {
                return Err("The 'raylib' template is only available for C++.".to_string());
            }
        }
        Template::Dll => {
            if !matches!(args.language, Language::Rust | Language::Cpp | Language::Go) {
                return Err("The 'dll' template is only available for Rust, C++, and Go.".to_string());
            }
        }
        Template::Vite => {
            if args.language != Language::TypeScript {
                return Err("The 'vite' template is only available for TypeScript.".to_string());
            }
        }
    }
    Ok(())
}

pub fn parse_arguments() -> CliArgs {
    let args = CliArgs::parse();
    
    if let Err(err) = validate_args(&args) {
        use clap::CommandFactory;
        let mut cmd = CliArgs::command();
        cmd.error(clap::error::ErrorKind::ArgumentConflict, err).exit();
    }
    
    args
}

fn main() -> io::Result<()> {
    let args = parse_arguments();
    let mut terminal = ratatui::init();

    let provider = Box::new(data::SqliteProjectProvider::new("projects.db").expect("Failed to init DB"));
    
    let mut app = ui::App::new(provider);
    let app_result = app.run(&mut terminal);

    ratatui::restore();
    app_result
}
