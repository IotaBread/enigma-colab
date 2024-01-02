use std::error::Error;
use std::fs;
use std::fs::File;
use std::path::Path;
use std::process::Command;

use chrono::{DateTime, Utc};
use rocket::serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::settings::read_settings;

#[derive(Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    #[serde(skip)]
    pub(crate) password: Option<String>,
    #[serde(skip)]
    pub(crate) pid: Option<u32>,
}

impl Session {
    pub async fn new(password: Option<String>) -> Result<Session, Box<dyn Error>> {
        let password = password.unwrap_or(random_password());

        let mut session = Session {
            id: Uuid::new_v4(),
            start: Utc::now(),
            end: None,
            password: Some(password),
            pid: None,
        };

        session.save()?;
        session.launch().await?;

        Ok(session)
    }

    async fn launch(&mut self) -> Result<(), Box<dyn Error>> {
        let dir = Path::new("data/sessions/").join(self.id.to_string());
        fs::create_dir_all(&dir)?;

        let settings = read_settings().await?;

        let mut command = Command::new("java");
        command
            .current_dir("data/repo/")
            .stdout(File::create(dir.join("stdout.log"))?)
            .stderr(File::create(dir.join("stderr.log"))?)
            .arg("-cp")
            .arg(settings.classpath)
            .arg(settings.enigma_main_class)
            .arg("-jar")
            .arg(settings.jar_file)
            .arg("-mappings")
            .arg(settings.mappings_file)
            .arg("-password")
            .arg(match &self.password {
                Some(p) => p,
                None => ""
            });

        for arg in settings.enigma_args.split(" ") {
            command.arg(arg);
        }

        let pid = command.spawn()?.id();
        fs::write(dir.join("session.pid"), pid.to_string())?;

        self.pid = Some(pid);
        Ok(())
    }

    fn save(&self) -> Result<(), Box<dyn Error>> {
        let dir = Path::new("data/sessions/").join(self.id.to_string());
        fs::create_dir_all(&dir)?;

        let data_file = dir.join("session.toml");
        fs::write(data_file, toml::to_string(&self)?)?;
        Ok(())
    }

    fn read<P: AsRef<Path>>(path: P) -> Result<Session, Box<dyn Error>> {
        let toml_str = fs::read_to_string(path)?;
        let s = toml::from_str(toml_str.as_str())?;
        Ok(s)
    }

    fn dir_open<P: AsRef<Path>>(path: P) -> Result<Session, Box<dyn Error>> {
        Self::read(path.as_ref().join("session.toml"))
    }

    pub fn stop(&mut self) -> Result<(), Box<dyn Error>> {
        if self.end.is_some() {
            return Ok(());
        }

        let file = Path::new("data/sessions/").join(self.id.to_string()).join("session.pid");

        if file.as_path().exists() {
            let pid = if self.pid.is_some() {
                self.pid.unwrap().to_string()
            } else {
                fs::read_to_string(&file)?
            };

            fs::remove_file(file)?;

            self.end = Some(Utc::now());
            self.save()?;

            Command::new("kill")
                .arg(pid)
                .status()?;

            Ok(())
        } else {
            // TODO: shouldn't happen?
            Ok(())
        }
    }
}

fn random_password() -> String {
    String::from("Placeholder")
}

pub fn load_sessions() -> Result<Vec<Session>, Box<dyn Error>> {
    let sessions_dir = Path::new("data/sessions/");
    if !sessions_dir.exists() {
        return Ok(vec![]);
    }

    let paths = fs::read_dir(sessions_dir)?;
    let mut sessions = vec![];

    for entry in paths {
        let dir = entry?;
        let file_type = dir.file_type()?;
        if file_type.is_dir() {
            let session = Session::dir_open(dir.path())?;
            sessions.push(session);
        }
    }

    Ok(sessions)
}
