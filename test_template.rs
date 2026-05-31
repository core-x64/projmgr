use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

fn test_json_template_creation() -> Result<(), Box<dyn std::error::Error>> {
    // Create a temp directory for testing
    let temp_dir = tempdir()?;
    let template_dir = temp_dir.path().join("templates");
    fs::create_dir_all(&template_dir)?;

    // HOME directory for testing
    let home = temp_dir.path();

    // Create template name and path
    let template_name = "default";
    let template_path = template_dir.join(format!("{}.json", template_name));

    // Create the template structure (same as in create_new_template)
    let mut structure = HashMap::new();

    // Add src/ directory with files
    let mut src_files = HashMap::new();
    src_files.insert("README.md".to_string(), "# Project\n\nA new project created with unit-projman.".to_string());
    src_files.insert("main.rs".to_string(), "fn main() {\n    println!(\"Hello, world!\");\n}".to_string());
    structure.insert("src/".to_string(), src_files);

    // Add empty include/ directory for future use
    let include_files = HashMap::new();
    structure.insert("include/".to_string(), include_files);

    // Create template def (same struct as in data.rs)
    #[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
    struct TemplateDef {
        name: String,
        #[serde(default)]
        structure: HashMap<String, HashMap<String, String>>,
    }

    let template_def = TemplateDef {
        name: template_name.to_string(),
        structure,
    };

    // Serialize to JSON
    let template_content = serde_json::to_string_pretty(&template_def)?;

    // Write to file
    fs::write(&template_path, &template_content)?;

    // Verify file was created
    assert!(template_path.exists());

    // Read it back and deserialize
    let read_content = fs::read_to_string(&template_path)?;
    let parsed: TemplateDef = serde_json::from_str(&read_content)?;

    // Verify contents
    assert_eq!(parsed.name, "default");
    assert!(parsed.structure.contains_key("src/"));
    assert!(parsed.structure.contains_key("include/"));

    let src_files = parsed.structure.get("src/").unwrap();
    assert_eq!(src_files.get("README.md").unwrap(), "# Project\n\nA new project created with unit-projman.");
    assert_eq!(src_files.get("main.rs").unwrap(), "fn main() {\n    println!(\"Hello, world!\");\n}");

    println!("Template creation test passed!");
    Ok(())
}

fn main() {
    if let Err(e) = test_json_template_creation() {
        eprintln!("Test failed: {}", e);
        std::process::exit(1);
    }
}