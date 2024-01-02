use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::process::Command;

use git2::build::RepoBuilder;
use git2::{BranchType, Repository};

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

fn pull_cmd() {
    // TODO
}

pub async fn clone() -> Result<(String, String), Box<dyn Error>> {
    let settings = read_settings().await?;

    let branch = settings.repo.branch;
    let repo = RepoBuilder::new()
        .branch(branch.as_str())
        .clone(settings.repo.url.as_str(), Path::new("data/repo"))?;

    let rev = repo.revparse_single("HEAD")?.id();
    pull_cmd();
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
