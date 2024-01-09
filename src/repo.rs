use std::error::Error;
use std::fs;
use std::io::{Result as IoResult, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::str::from_utf8;

use git2::{BranchType, DiffDelta, DiffFormat, DiffHunk, DiffLine, DiffLineType, FetchOptions, IndexAddOption, ObjectType, Oid, Repository, ResetType, StatusOptions};
use git2::build::{CheckoutBuilder, RepoBuilder};

use crate::settings::read_settings;
use crate::util::throw;

pub const DIR: &str = "data/repo";

type Git2Result<T> = Result<T, git2::Error>;

pub fn run_command(command: &String) -> IoResult<Option<ExitStatus>> {
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

fn open_repo() -> Git2Result<Repository> {
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
    let repo = open_repo()?;
    fetch_repo(&repo)
}

/// Based on libgit2's [example fetch.c](https://libgit2.org/libgit2/ex/v1.7.1/fetch.html)
pub fn fetch_repo(repo: &Repository) -> Git2Result<()> {
    let mut options = FetchOptions::new(); // TODO: Progress message
    let remotes = repo.remotes()?;
    let mut remotes_iter = remotes.iter();

    while let Some(Some(remote_name)) = remotes_iter.next() {
        println!("Fetching {}", remote_name); // TODO: Custom feedback function
        let mut remote = repo.find_remote(remote_name)?;

        // No refspecs to use the base ones
        remote.fetch::<&str>(&[], Some(&mut options), None)?;

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
pub fn pull() -> Result<Result<String, String>, Box<dyn Error>> {
    let repo = open_repo()?;
    let mut head_ref = repo.head()?;

    if let Some(current_branch) = head_ref.shorthand() {
        let branch = repo.find_branch(current_branch, BranchType::Local)?;
        // current_branch is the simple name, we need it's full name (i.e. refs/heads/branch)
        let branch_ref = match branch.get().name() {
            Some(s) => s,
            None => { throw!("Branch ref has an invalid name"); }
        };

        let remote_name = repo.branch_upstream_remote(branch_ref)?;
        let remote_name = remote_name.as_str().unwrap_or("<unknown remote>");
        let mut remote = repo.find_remote(remote_name)?;

        remote.fetch::<&str>(&[], None, None)?;

        let remote_branch = branch.upstream()?;
        let merge_target = repo.reference_to_annotated_commit(remote_branch.get())?;

        let (analysis, preference) = repo.merge_analysis(&[&merge_target])?;

        if analysis.is_up_to_date() {
            return Ok(Err("Already up to date".to_string()));
        } else if analysis.is_fast_forward() && !preference.is_no_fast_forward() {
            // println!("Fast-forward");
            let target_oid = merge_target.id();
            let target = repo.find_object(target_oid, Some(ObjectType::Commit))?;

            let mut options = CheckoutBuilder::new();
            repo.checkout_tree(&target, Some(options.safe()))?;

            let remote_branch_name = remote_branch.name()?.unwrap_or("<unknown branch>");
            let reflog_msg = format!("pull {} {}: Fast-forward", remote_name, remote_branch_name);
            head_ref.set_target(target_oid, reflog_msg.as_str())?;

            return Ok(Ok(target_oid.to_string()));
        } else if analysis.is_normal() {
            throw!("Merge required, please resolve it manually")
        }
    }

    throw!("Not currently on a branch")
}

pub fn add(repo: &Repository, path: String) -> Git2Result<()> {
    let mut index = repo.index()?;
    index.add_all([path].iter(), IndexAddOption::DEFAULT, None)?;
    index.write()
}

fn diff_print(buf: &mut Vec<u8>) -> impl FnMut(DiffDelta<'_>, Option<DiffHunk<'_>>, DiffLine<'_>) -> bool + '_ {
    return |_, _, line| {
        let line_type = line.origin_value();
        let content = match from_utf8(line.content()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to parse diff line: {e}");
                return false;
            }
        };

        let r = if line_type == DiffLineType::Addition || line_type == DiffLineType::Deletion || line_type == DiffLineType::Context {
            write!(buf, "{}{}", line.origin(), content)
        } else {
            write!(buf, "{}", content)
        };

        if let Err(e) = r {
            eprintln!("Failed to print diff line: {e}");
            return false;
        }

        true
    }
}

/// Equivalent to `git diff --cached`
pub fn diff_bytes(repo: &Repository) -> Git2Result<Vec<u8>> {
    let head = repo.revparse_single("HEAD")?;
    let head_tree = head.peel_to_tree()?;

    let mut buf = Vec::new();
    let diff = repo.diff_tree_to_index(Some(&head_tree), None, None)?;
    diff.print(DiffFormat::Patch, diff_print(&mut buf))?;

    Ok(buf)
}

pub async fn create_patch() -> Result<Vec<u8>, Box<dyn Error>> {
    let settings = read_settings().await?;
    let repo = open_repo()?;

    // Stage changes
    add(&repo, settings.mappings_file)?;

    // Create the patch
    let patch = diff_bytes(&repo)?;

    Ok(patch)
}

/// Equivalent to `git reset --hard`
pub fn hard_reset(repo: &Repository) -> Git2Result<()> {
    let head = repo.head()?;
    let head_commit = head.peel_to_commit()?;
    repo.reset(head_commit.as_object(), ResetType::Hard, None)?;

    Ok(())
}

/// Equivalent to `git clean -f -d [<path>]`
pub fn clean_repo(repo: &Repository, path: Option<String>) -> Result<(), Box<dyn Error>> {
    let mut options = StatusOptions::new();
    options.include_untracked(true);
    if let Some(path) = path {
        options.pathspec(path);
    }

    let statuses = repo.statuses(Some(&mut options))?;

    for status_entry in statuses.iter() {
        let status = status_entry.status();
        if status.is_index_new() || status.is_wt_new() {
            if let Some(path) = status_entry.path() {
                let path = Path::new(path);

                if path.is_dir() {
                    fs::remove_dir_all(path)?;
                } else if path.exists() {
                    fs::remove_file(path)?;
                }
            }
        }
    }

    Ok(())
}

pub async fn clear_working_tree() -> Result<(), Box<dyn Error>> {
    let settings = read_settings().await?;
    let repo = open_repo()?;

    // Remove staged and working dir changes
    hard_reset(&repo)?;

    // Remove any untracked files
    clean_repo(&repo, Some(settings.mappings_file))?;

    Ok(())
}

pub fn get_head() -> Git2Result<String> {
    let repo = open_repo()?;
    let direct_head = repo.head()?.resolve()?;
    let target = direct_head.target().unwrap_or(Oid::zero()); // Safe to unwrap, only None if the reference isn't direct
    Ok(target.to_string())
}
