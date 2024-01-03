use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};

use git2::{BranchType, Repository};
use git2::build::RepoBuilder;

use crate::settings::read_settings;

#[derive(Debug)]
struct StrError(String);

impl StrError {
    fn new(msg: &str) -> Self {
        StrError(msg.to_string())
    }
}

impl Display for StrError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Error for StrError {
}

pub fn run_command(command: &String) -> std::io::Result<Option<ExitStatus>> {
    Ok(if !command.is_empty() {
        Some(Command::new("sh")
            .current_dir("data/repo")
            .arg("-c")
            .arg(command)
            .status()?)
    } else {
        None
    })
}

pub async fn clone() -> Result<(String, String), Box<dyn Error>> {
    let settings = read_settings().await?;

    let branch = settings.repo.branch;
    let repo = RepoBuilder::new()
        .branch(branch.as_str())
        .clone(settings.repo.url.as_str(), Path::new("data/repo"))?;

    run_command(&settings.pull_cmd)?;

    let rev = repo.revparse_single("HEAD")?.id();
    Ok((branch, rev.to_string()))
}

pub fn is_cloned() -> bool {
    Path::new("data/repo/.git").exists()
}

pub async fn list_branches() -> Result<Vec<String>, Box<dyn Error>> {
    let repo = Repository::open("data/repo")?;
    let branches = repo.branches(Some(BranchType::Local))?;
    let mut result = Vec::new();

    for branch in branches {
        let (branch, _) = branch?;
        let name = branch.name()?.expect("Invalid branch name!");
        result.push(name.to_string());
    }

    Ok(result)
}

pub fn fetch() -> Result<(), Box<dyn Error>> {
    let status = Command::new("git")
        .current_dir("data/repo")
        .arg("fetch")
        .status()?;
    status.success()
        .then_some(())
        .ok_or(Box::from(StrError::new("fetching failed")))
}

pub async fn create_patch() -> Result<Vec<u8>, Box<dyn Error>> {
    let settings = read_settings().await?;

    // Stage changes
    Command::new("git")
        .current_dir("data/repo")
        .arg("add")
        .arg(settings.mappings_file)
        .stderr(Stdio::inherit())
        .status()?;

    // Create the patch
    let diff = Command::new("git")
        .current_dir("data/repo")
        .arg("diff")
        .arg("--cached")
        .stderr(Stdio::inherit())
        .output()?;

    if !diff.status.success() {
        Ok(vec![])
    } else {
        Ok(diff.stdout)
    }
}

pub async fn clear_working_tree() -> Result<(), Box<dyn Error>> {
    let settings = read_settings().await?;

    // Remove staged and working dir changes
    let reset = Command::new("git")
        .current_dir("data/repo")
        .arg("reset")
        .arg("--hard")
        .stderr(Stdio::inherit())
        .status()?;

    if !reset.success() {
        return Err(Box::from(StrError(format!("git reset failed with code {code}", code = reset.code().unwrap_or(-1)))));
    }

    // Remove any untracked files
    let clean = Command::new("git")
        .current_dir("data/repo")
        .arg("clean")
        .arg("-f") // Force, refuses to delete files by default
        .arg("-d") // Recurse
        .arg(settings.mappings_file)
        .stderr(Stdio::inherit())
        .status()?;

    if !clean.success() {
        return Err(Box::from(StrError(format!("git clean failed with code {code}", code = reset.code().unwrap_or(-1)))));
    }

    Ok(())
}
