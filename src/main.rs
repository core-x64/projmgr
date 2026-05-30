use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Language {
    Rust,
    Python,
    Go,
    TypeScript,
    Cpp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Template {
    Standard,
    Dll,
    Raylib,
    Vite,
}

// parse the language
fn parse_language(s: &str) -> Result<Language, String> {
    match s.to_lowercase().as_str() {
        "rust" | "rs" => Ok(Language::Rust),
        "python" | "py" => Ok(Language::Python),
        "go" => Ok(Language::Go),
        "typescript" | "ts" => Ok(Language::TypeScript),
        "c++" | "cpp" => Ok(Language::Cpp),
        _ => Err(format!("'{}' is not a supported language.", s)),
    }
}

// parse the template
fn parse_template(s: &str) -> Result<Template, String> {
    match s.to_lowercase().as_str() {
        "standard" | "std" => Ok(Template::Standard),
        "dll" => Ok(Template::Dll),
        "raylib" => Ok(Template::Raylib),
        "vite" => Ok(Template::Vite),
        _ => Err(format!("'{}' is not a supported template.", s)),
    }
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

    #[arg(short, long, value_parser = parse_language)]
    pub language: Option<Language>,

    #[arg(short, long, value_parser = parse_template)]
    pub template: Option<Template>,

    #[arg(value_name = "NAME")]
    pub name: String,
}

// make sure valid combination is selected.
fn validate_args(args: &CliArgs) -> Result<(), String> {
    if let (Some(lang), Some(tmpl)) = (args.language, args.template) {
        match tmpl {
            Template::Standard => {} // all languages allowed
            Template::Raylib => {
                if lang != Language::Cpp {
                    return Err("The 'raylib' template is only available for C++ (cpp/c++).".to_string());
                }
            }
            Template::Dll => {
                if !matches!(lang, Language::Rust | Language::Cpp | Language::Go) {
                    return Err("The 'dll' template is only available for Rust, C++, and Go.".to_string());
                }
            }
            Template::Vite => {
                if lang != Language::TypeScript {
                    return Err("The 'vite' template is only available for TypeScript (ts/typescript).".to_string());
                }
            }
        }
    }
    Ok(())
}

pub fn parse_arguments() -> CliArgs {
    let args = CliArgs::parse();
    
    // parse, validate, and return arguments
    if let Err(err) = validate_args(&args) {
        use clap::CommandFactory;
        let mut cmd = CliArgs::command();
        cmd.error(clap::error::ErrorKind::ArgumentConflict, err).exit();
    }
    
    args
}

fn main() {
    let args = parse_arguments();
    println!("{:?}", args);
}
