use crate::project::config::{ProjectConfig, CONFIG_FILE};
use std::path::PathBuf;

pub fn config(args: &[String]) -> std::io::Result<()> {
    let initialize = args.iter().any(|arg| arg == "--init");
    let root = PathBuf::from(
        args.iter()
            .find(|arg| arg.as_str() != "--init")
            .map(String::as_str)
            .unwrap_or("."),
    );

    if initialize {
        std::fs::create_dir_all(&root)?;
        let path = ProjectConfig::write_default(&root)?;
        println!("Created {}", path.display());
    }

    let config = ProjectConfig::load(&root)?;
    let path = root.join(CONFIG_FILE);
    if path.is_file() {
        println!("Configuration: {}", path.display());
    } else {
        println!(
            "Configuration: built-in defaults ({} does not exist)",
            path.display()
        );
    }
    println!();
    print!("{}", config.to_toml()?);
    Ok(())
}
