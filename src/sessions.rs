use std::error::Error;
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::string::ToString;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, Serializer};
use uuid::Uuid;
use crate::repo;

use crate::settings::read_settings;

const DIR: &str = "data/sessions";
const PID_FILE: &str = "session.pid";
const PATCH_FILE: &str = "session.patch";

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub date: DateTime<Utc>,
    password: Option<String>, // TODO: Serialize only when writing the session.toml file
    // Serialize as `running: bool` for use in the html templates
    #[serde(skip_deserializing, rename(serialize = "running"), serialize_with = "serialize_running")]
    pid: Option<u32>,
}

impl Session {
    pub fn is_running(&self) -> bool {
        self.pid.is_some()
    }

    pub fn check_is_running(&mut self) -> std::io::Result<bool> {
        self.check_process()?;
        Ok(self.is_running())
    }

    fn check_process(&mut self) -> std::io::Result<()> {
        if let Some(pid) = self.pid {
            let status = Command::new("kill")
                .arg("-0")
                .arg(pid.to_string())
                .status()?;
            if !status.success() {
                self.invalidate_pid()?
            }
        }

        Ok(())
    }

    fn invalidate_pid(&mut self) -> std::io::Result<()> {
        self.pid = None;

        fs::remove_file(self.get_file(PID_FILE))
    }

    fn get_dir(&self) -> PathBuf {
        PathBuf::from(DIR).join(self.id.to_string())
    }

    fn get_file(&self, file: &str) -> PathBuf {
        self.get_dir().join(file)
    }

    pub fn get_patch_file(&self) -> PathBuf {
        self.get_file(PATCH_FILE)
    }

    fn deserialize<P: AsRef<Path>>(path: P) -> Result<Session> {
        let toml_str = fs::read_to_string(path)?;
        let s = toml::from_str(toml_str.as_str())?;
        Ok(s)
    }

    fn read_pid<P: AsRef<Path>>(path: P) -> Result<Option<u32>> {
        Ok(if path.as_ref().exists() {
            Some(fs::read_to_string(path)?.parse()?)
        } else {
            None
        })
    }

    pub fn read<P: AsRef<Path>>(path: P) -> Result<Session> {
        let path = path.as_ref();
        let mut session = Self::deserialize(path.join("session.toml"))?;
        session.pid = Self::read_pid(path.join(PID_FILE))?;

        Ok(session)
    }

    fn write_pid<P: AsRef<Path>>(path: P, pid: u32) -> std::io::Result<()> {
        fs::write(path, pid.to_string())
    }

    fn serialize<P: AsRef<Path>>(path: P, session: &Session) -> Result<()> {
        let toml_str = toml::to_string(session)?;
        fs::write(path, toml_str)?;

        Ok(())
    }

    fn write(&self) -> Result<()> {
        Self::serialize(self.get_file("session.toml"), &self)
    }

    pub async fn new(password: Option<String>) -> Result<Session> {
        let mut session = Session {
            id: Uuid::new_v4(),
            date: Utc::now(),
            password,
            pid: None,
        };

        session.launch().await?;
        session.write()?;

        Ok(session)
    }

    async fn launch(&mut self) -> Result<()> {
        let dir = self.get_dir();
        fs::create_dir_all(&dir)?;

        let settings = read_settings().await?;

        repo::run_command(&settings.pre_session_cmd)?;

        let stdout = File::create(dir.join("stdout.log"))?;
        let stderr = File::create(dir.join("stderr.log"))?;
        let mut command = Command::new("java");

        command
            .current_dir("data/repo/")
            .stdout(stdout)
            .stderr(stderr)
            .arg("-cp")
            .arg(settings.classpath)
            .arg(settings.enigma_main_class)
            .arg("-jar")
            .arg(settings.jar_file)
            .arg("-mappings")
            .arg(settings.mappings_file);

        if let Some(password) = &self.password {
            command.arg("-password")
                .arg(password);
        }

        for arg in settings.enigma_args.split(" ") {
            command.arg(arg);
        }

        let pid = command.spawn()?.id();
        Session::write_pid(dir.join(PID_FILE), pid)?;
        self.pid = Some(pid);

        Ok(())
    }

    pub async fn finish(&mut self) -> Result<()> {
        if !self.check_is_running()? {
            return Ok(())
        }

        let pid = self.pid.unwrap();

        Command::new("kill")
            .arg(pid.to_string())
            .status()?;

        self.invalidate_pid()?;
        self.write()?;

        let patch = repo::create_patch().await?;
        repo::clear_working_tree().await?;
        fs::write(self.get_file(PATCH_FILE), patch)?;

        repo::run_command(&read_settings().await?.post_session_cmd)?;

        Ok(())
    }
}

pub fn load_sessions() -> Result<Vec<Session>> {
    let mut sessions = vec![];
    let dir = Path::new(DIR);

    if dir.exists() {
        let entries = fs::read_dir(dir)?;

        for entry in entries {
            let entry = entry?;
            let file_type = entry.file_type()?;

            if file_type.is_dir() {
                let session = Session::read(entry.path())?;
                sessions.push(session);
            }
        }
    }

    Ok(sessions)
}

pub fn serialize_running<S>(value: &Option<u32>, serializer: S) -> std::result::Result<S::Ok, S::Error> where S: Serializer {
    serializer.serialize_bool(value.is_some())
}
