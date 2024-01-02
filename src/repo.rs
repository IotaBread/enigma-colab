use std::error::Error;
use std::path::Path;

use git2::build::RepoBuilder;
use git2::{BranchType, Repository};

use crate::settings::read_settings;

pub async fn clone() -> Result<(String, String), Box<dyn Error>> {
    let settings = read_settings().await?;

    let branch = settings.repo.branch;
    let repo = RepoBuilder::new()
        .branch(branch.as_str())
        .clone(settings.repo.url.as_str(), Path::new("data/repo"))?;

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
