use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use git2::{BranchType, FetchOptions, ObjectType, Repository};
use git2::build::{CheckoutBuilder, RepoBuilder};

use crate::settings::read_settings;

const DIR: &str = "data/repo";

type Git2Result<T> = Result<T, git2::Error>;

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
            .current_dir(DIR)
            .arg("-c")
            .arg(command)
            .status()?)
    } else {
        None
    })
}

fn open_repo() -> Result<Repository, git2::Error> {
    Repository::open(DIR)
}

pub async fn clone() -> Result<(String, String), Box<dyn Error>> {
    let settings = read_settings().await?;

    let branch = settings.repo.branch;
    let repo = RepoBuilder::new()
        .branch(branch.as_str())
        .clone(settings.repo.url.as_str(), Path::new(DIR))?;

    // TODO: Run on another thread
    run_command(&settings.pull_cmd)?;

    let rev = repo.revparse_single("HEAD")?.id();
    Ok((branch, rev.to_string()))
}

pub fn is_cloned() -> bool {
    PathBuf::from(DIR).join(".git").as_path().exists()
}

pub async fn list_local_branches() -> Git2Result<Vec<String>> {
    let repo = open_repo()?;
    let branches = repo.branches(Some(BranchType::Local))?;
    let mut result = Vec::new();

    for branch in branches {
        let (branch, _) = branch?;
        let name = branch.name()?.expect("Invalid branch name!");
        result.push(name.to_string());
    }

    Ok(result)
}

pub fn fetch() -> Git2Result<()> {
    fetch_repo(open_repo()?)
}

/// Based on libgit2's [example fetch.c](https://libgit2.org/libgit2/ex/v1.7.1/fetch.html)
pub fn fetch_repo(repo: Repository) -> Git2Result<()> {
    let mut options = FetchOptions::new(); // TODO: Progress message
    let mut remotes_iter = repo.remotes()?.iter();

    while let Some(Some(remote_name)) = remotes_iter.next() {
        println!("Fetching {}", remote_name); // TODO: Custom feedback function
        let mut remote = repo.find_remote(remote_name)?;

        // No refspecs to use the base ones
        remote.fetch(&[], Some(&mut options), None)?;

        let stats = remote.stats();
        if stats.local_objects() > 0 {
            println!("{}: Received {}/{} objects in {} bytes (used {} local object)", remote_name,
                     stats.indexed_objects(), stats.total_objects(), stats.received_bytes(), stats.local_objects());
        } else {
            println!("{}: Received {}/{} objects in {} bytes", remote_name,
                     stats.indexed_objects(), stats.total_objects(), stats.received_bytes());
        }
    }

    Ok(())
}

/// Based on libgit2's [example merge.c](https://libgit2.org/libgit2/ex/v1.7.1/merge.html)
pub fn pull() -> Git2Result<()> {
    let repo = open_repo()?;
    let mut head_ref = repo.head()?;

    if let Some(current_branch) = head_ref.shorthand() {
        let remote_name = repo.branch_upstream_remote(current_branch)?;
        let remote_name = remote_name.as_str().unwrap_or("<unknown remote>");
        let mut remote = repo.find_remote(remote_name)?;

        remote.fetch(&[], None, None)?;

        let branch = repo.find_branch(current_branch, BranchType::Local)?;
        assert_eq!(&head_ref, branch.get());
        let remote_branch = branch.upstream()?;
        let merge_target = repo.reference_to_annotated_commit(remote_branch.get())?;

        let (analysis, preference) = repo.merge_analysis(&[&merge_target])?;

        if analysis.is_up_to_date() {
            println!("Already up to date");
        } else if analysis.is_fast_forward() && !preference.is_no_fast_forward() {
            println!("Fast-forward");
            let target_oid = merge_target.id();
            let target = repo.find_object(target_oid, Some(ObjectType::Commit))?;

            let options = CheckoutBuilder::new().safe();
            repo.checkout_tree(&target, Some(options))?;

            let remote_branch_name = remote_branch.name()?.unwrap_or("<unknown branch>");
            let reflog_msg = format!("pull {} {}: Fast-forward", remote_name, remote_branch_name);
            head_ref.set_target(target_oid, reflog_msg.as_str())?;

            return Ok(())
        } else if analysis.is_normal() {
            eprintln!("Merge required, please resolve it manually")
        }
    } else {
        eprintln!("Not currently on a branch")
    }

    Ok(())
}

pub async fn create_patch() -> Result<Vec<u8>, Box<dyn Error>> {
    let settings = read_settings().await?;

    // Stage changes
    Command::new("git")
        .current_dir(DIR)
        .arg("add")
        .arg(settings.mappings_file)
        .stderr(Stdio::inherit())
        .status()?;

    // Create the patch
    let diff = Command::new("git")
        .current_dir(DIR)
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
        .current_dir(DIR)
        .arg("reset")
        .arg("--hard")
        .stderr(Stdio::inherit())
        .status()?;

    if !reset.success() {
        return Err(Box::from(StrError(format!("git reset failed with code {code}", code = reset.code().unwrap_or(-1)))));
    }

    // Remove any untracked files
    let clean = Command::new("git")
        .current_dir(DIR)
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
