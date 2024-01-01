use std::fs;
use std::path::Path;
use rocket::serde::{Deserialize, Serialize};
use serde::de::Error;

#[derive(Debug, Deserialize, Serialize, FromForm)]
pub struct Settings {
    repo_url: String,
    repo_branch: String,
    jar_file: String,
    mappings_file: String,
    auto_save_interval: u16,
    pull_cmd: String,
    pre_session_cmd: String,
    post_session_cmd: String,
    enigma_args: String,
    classpath: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            repo_url: "".to_string(),
            repo_branch: "master".to_string(),
            jar_file: "file.jar".to_string(),
            mappings_file: "mappings/".to_string(),
            auto_save_interval: 120,
            pull_cmd: "".to_string(),
            pre_session_cmd: "".to_string(),
            post_session_cmd: "".to_string(),
            enigma_args: "".to_string(),
            classpath: "".to_string(),
        }
    }
}

pub async fn read_settings() -> Result<Settings, Box<dyn std::error::Error>> {
    let path = Path::new("data/CoLab.toml");
    if path.exists() {
        let toml_str = fs::read_to_string(path)?;
        let settings: Settings = toml::from_str(&toml_str)?;
        Ok(settings)
    } else {
        let settings = Settings::default();
        write_settings(&settings).await?;
        Ok(settings)
    }
}

pub async fn write_settings(settings: &Settings) -> Result<(), Box<dyn std::error::Error>> {
    let toml_str = toml::to_string_pretty(&settings)?;
    fs::create_dir_all("data/")?;
    fs::write("data/CoLab.toml", toml_str)?;
    Ok(())
}
