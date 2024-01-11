use std::error::Error;
use std::fs;
use std::io::{Result as IoResult, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::str::from_utf8;

use git2::{AnnotatedCommit, BranchType, DiffDelta, DiffFormat, DiffHunk, DiffLine, DiffLineType, FetchOptions, IndexAddOption, ObjectType, Oid, Repository, ResetType, StatusOptions};
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
    let url = settings.repo.url;

    let repo = clone_repo(url.as_str(), Some(branch.as_str()), Path::new(DIR))?;

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

pub fn get_repo_head(repo: &Repository) -> Git2Result<String> {
    let direct_head = repo.head()?.resolve()?;
    let target = direct_head.target().unwrap_or(Oid::zero()); // Safe to unwrap, only None if the reference isn't direct
    Ok(target.to_string())
}

pub fn get_head() -> Git2Result<String> {
    let repo = open_repo()?;
    get_repo_head(&repo)
}

pub fn clone_repo<P: AsRef<Path>>(uri: &str, branch: Option<&str>, path: P) -> Git2Result<Repository> {
    let mut builder = RepoBuilder::new();
    if let Some(branch) = branch {
        builder.branch(branch);
    }

    builder.clone(uri, path.as_ref())
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

pub fn pull() -> Result<Result<String, String>, Box<dyn Error>> {
    let repo = open_repo()?;
    pull_repo(&repo).map(|r| { r.map(|id| id.to_string()) })
}

/// Based on libgit2's [example merge.c](https://libgit2.org/libgit2/ex/v1.7.1/merge.html)
///
/// The successful (inner) result has either the new HEAD hash, or a message specifying why it wasn't updated
pub fn pull_repo(repo: &Repository) -> Result<Result<Oid, String>, Box<dyn Error>> {
    let mut head_ref = repo.head()?;

    if let Some(current_branch) = head_ref.shorthand() {
        let branch = repo.find_branch(current_branch, BranchType::Local)?;
        // current_branch is the simple name, we need it's full name (i.e. refs/heads/branch)
        let branch_ref = branch.get().name().ok_or("Branch ref has an invalid name")?;

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

            return Ok(Ok(target_oid));
        } else if analysis.is_normal() {
            throw!("Merge required, please resolve it manually")
        }
    }

    throw!("Not currently on a branch")
}

fn resolve_ref<'r>(repo: &'r Repository, target_ref: &String) -> Git2Result<Option<AnnotatedCommit<'r>>> {
    let resolved = repo.resolve_reference_from_short_name(target_ref.as_str());

    if let Ok(resolved_ref) = resolved {
        let commit = repo.reference_to_annotated_commit(&resolved_ref)?;
        return Ok(Some(commit))
    }

    let resolved = repo.revparse_single(target_ref);
    if let Ok(resolved_obj) = resolved {
        let commit = repo.find_annotated_commit(resolved_obj.id())?;
        return Ok(Some(commit))
    }

    Ok(None)
}

fn guess_ref<'r>(repo: &'r Repository, target_ref: &String) -> Git2Result<Option<AnnotatedCommit<'r>>> {
    let remotes = repo.remotes()?;

    let mut error = None;

    for remote in remotes.iter() {
        if let Some(remote) = remote {
            let refname = format!("refs/remotes/{}/{}", remote, target_ref);

            let found_ref = match repo.find_reference(refname.as_str()) {
                Ok(r) => r,
                Err(e) => {
                    error = Some(e);
                    continue;
                }
            };

            let commit = repo.reference_to_annotated_commit(&found_ref)?;
            return Ok(Some(commit))
        }
    }

    if error.is_some() {
        Err(error.unwrap())
    } else {
        Ok(None)
    }
}

/// Change the HEAD reference to the specified one, updating the working tree
///
/// Based on libgit2's [example checkout.c](https://libgit2.org/libgit2/ex/v1.7.1/checkout.html)
pub fn repo_checkout(repo: &Repository, target_ref: String) -> Result<Oid, Box<dyn Error>> {
    let target = resolve_ref(repo, &target_ref)?
        .or(guess_ref(repo, &target_ref)?)
        .ok_or("Reference not found")?;

    let mut options = CheckoutBuilder::new();
    options.safe();

    let target_oid = target.id();
    let target_commit = repo.find_commit(target_oid)?;

    repo.checkout_tree(target_commit.as_object(), Some(&mut options))?;

    if let Some(target_refname) = target.refname() {
        let checkout_ref = repo.find_reference(target_refname)?;

        if checkout_ref.is_remote() {
            let branch = repo.branch_from_annotated_commit(target_ref.as_str(), &target, false)?;
            let branch_ref = branch.into_reference();
            let refname = branch_ref.name().ok_or("Invalid branch name")?;

            repo.set_head(refname)?;
        } else {
            repo.set_head(target_refname)?;
        };
    } else {
        repo.set_head_detached_from_annotated(target)?;
    }

    Ok(target_oid)
}

/// Add file contents to the index
pub fn add(repo: &Repository, path: &[&str]) -> Git2Result<()> {
    let mut index = repo.index()?;
    index.add_all(path.iter(), IndexAddOption::DEFAULT, None)?;
    index.write()
}

/// Create a new commit with the changes in the index and the given message
///
/// Based on libgit2's [example commit.c](https://libgit2.org/libgit2/ex/v1.7.1/commit.html)
pub fn commit(repo: &Repository, message: &str) -> Git2Result<Oid> {
    let parent = repo.revparse_single("HEAD")?.peel_to_commit()?;
    let mut index = repo.index()?;
    let tree_oid = index.write_tree()?;
    index.write()?;

    let tree = repo.find_tree(tree_oid)?;
    let signature = repo.signature()?;

    repo.commit(Some("HEAD"), &signature, &signature, message, &tree, &[&parent])
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
    };
}

/// Generate a patch diff of the changes in the index, and return its bytes
///
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
    add(&repo, &[settings.mappings_file.as_str()])?;

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
    let workdir = match repo.workdir() {
        Some(p) => p,
        None => { return Ok(()); }
    };

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
                let path = workdir.join(path);

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

#[cfg(test)]
mod tests {
    use std::env;

    use git2::Status;
    use tempfile::TempDir;

    use super::*;

    macro_rules! test_file {
        ($fname:expr) => {
            concat!(env!("CARGO_MANIFEST_DIR"), "/resources/test/", $fname)
        };
    }

    macro_rules! write_assert {
        ($file:expr, $content:literal) => {{
            let s = $content.to_string();
            fs::write(&$file, &s)?;
            assert_eq!(s, fs::read_to_string(&$file)?);
            s
        }};
    }

    fn copy_dir_all<P: AsRef<Path>, P2: AsRef<Path>>(src: P, dst: P2) -> IoResult<()> {
        fs::create_dir_all(&dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let file_type = entry.file_type()?;

            let dst_entry = dst.as_ref().join(entry.file_name());
            if file_type.is_dir() {
                copy_dir_all(entry.path(), dst_entry)?;
            } else {
                fs::copy(entry.path(), dst_entry)?;
            }
        }

        Ok(())
    }

    fn setup_test_repo() -> IoResult<TempDir> {
        let source = PathBuf::from(test_file!("testrepo/")).canonicalize()?;
        let dir = tempfile::Builder::new().prefix("testrepo").tempdir()?;
        let path = dir.path();

        copy_dir_all(source, path)?;
        fs::rename(path.join(".gitted"), path.join(".git"))?;

        Ok(dir)
    }

    fn clone_test_repo(upstream_dir: &TempDir) -> Result<(TempDir, Repository), Box<dyn Error>> {
        let upstream_path = upstream_dir.path().canonicalize()?;

        let upstream = upstream_path.into_os_string().into_string();
        assert!(upstream.is_ok(), "Path contains invalid UTF-8");
        let upstream = upstream.unwrap();

        let repo_dir = tempfile::Builder::new().prefix("testrepo_clone").tempdir()?;
        let repo_path = repo_dir.path();
        let repo = clone_repo(upstream.as_str(), Some("master"), repo_path)?;

        Ok((repo_dir, repo))
    }

    fn open_test_repo() -> Result<(TempDir, Repository), Box<dyn Error>> {
        let dir = setup_test_repo()?;
        let repo = Repository::open(&dir)?;
        Ok((dir, repo))
    }

    #[test]
    fn test_clone() -> Result<(), Box<dyn Error>> {
        let upstream_repo_dir = setup_test_repo()?;
        let (repo_dir, repo) = clone_test_repo(&upstream_repo_dir)?;

        assert!(repo.path().exists());
        let file = repo_dir.path().join("file.txt");
        assert!(file.exists());
        assert_eq!(String::from("Lorem ipsum dolor sit amet\n"), fs::read_to_string(file)?);

        upstream_repo_dir.close()?;
        repo_dir.close()?;
        Ok(())
    }

    #[test]
    fn test_hard_reset() -> Result<(), Box<dyn Error>> {
        let (repo_dir, repo) = open_test_repo()?;

        let file = repo_dir.path().join("file.txt");
        let original_contents = String::from("Lorem ipsum dolor sit amet\n");
        assert_eq!(original_contents, fs::read_to_string(&file)?);

        write_assert!(file, "Replaced contents!\n");

        hard_reset(&repo)?;
        assert_eq!(original_contents, fs::read_to_string(file)?);

        repo_dir.close()?;
        Ok(())
    }

    #[test]
    fn test_clean() -> Result<(), Box<dyn Error>> {
        let (repo_dir, repo) = open_test_repo()?;
        let repo_path = repo_dir.path();

        let file = repo_path.join("file.txt");
        let new_file = repo_path.join("meow.txt");
        let new_file2 = repo_path.join("foo.txt");
        let new_file3 = repo_path.join("baz.txt");
        write_assert!(new_file, "meow meow meow mawww");
        write_assert!(new_file2, "foo bar baz");
        write_assert!(new_file3, "baz bar foo");

        assert!(new_file.exists());
        assert!(new_file2.exists());
        assert!(new_file3.exists());
        assert!(file.exists());

        clean_repo(&repo, Some("file.txt".to_string()))?;

        assert!(file.exists(), "clean_repo() removed a tracked file");
        assert!(new_file.exists(), "clean_repo() removed a file that shouldn't have been affected");
        assert!(new_file2.exists(), "clean_repo() removed a file that shouldn't have been affected");
        assert!(new_file3.exists(), "clean_repo() removed a file that shouldn't have been affected");

        clean_repo(&repo, Some("meow.txt".to_string()))?;

        assert!(!new_file.exists(), "clean_repo() did not remove an untracked file");
        assert!(new_file2.exists(), "clean_repo() removed a file that shouldn't have been affected");
        assert!(new_file3.exists(), "clean_repo() removed a file that shouldn't have been affected");

        clean_repo(&repo, None)?;

        assert!(!new_file2.exists(), "clean_repo() did not remove an untracked file");
        assert!(!new_file3.exists(), "clean_repo() did not remove an untracked file");

        repo_dir.close()?;
        Ok(())
    }

    #[test]
    fn test_add() -> Result<(), Box<dyn Error>> {
        let (repo_dir, repo) = open_test_repo()?;
        let repo_path = repo_dir.path();

        let file = repo_path.join("new.txt");
        write_assert!(file, "New file\n");

        add(&repo, &["new.txt"])?;

        assert!(repo.status_file(Path::new("new.txt"))?.is_index_new());

        repo_dir.close()?;
        Ok(())
    }

    #[test]
    fn test_commit() -> Result<(), Box<dyn Error>> {
        let (repo_dir, repo) = open_test_repo()?;
        let repo_path = repo_dir.path();

        let file = repo_path.join("new.txt");
        write_assert!(file, "New committed file\n");

        add(&repo, &["*"])?;
        commit(&repo, "Add new.txt")?;

        assert_eq!(Status::CURRENT, repo.status_file(Path::new("new.txt"))?);

        repo_dir.close()?;
        Ok(())
    }

    #[test]
    fn test_fetch() -> Result<(), Box<dyn Error>> {
        let (upstream_dir, upstream) = open_test_repo()?;
        let (repo_dir, repo) = clone_test_repo(&upstream_dir)?;
        let upstream_path = upstream_dir.path();
        let repo_path = repo_dir.path();

        let upstream_file = upstream_path.join("file.txt");
        write_assert!(upstream_file, "Lorem ipsum dolor sit amet\nNew line\n");

        add(&upstream, &["file.txt"])?;
        commit(&upstream, "Update file.txt")?;

        let pre_fetch = repo.revparse_single("refs/remotes/origin/master")?.id();
        fetch_repo(&repo)?;
        let post_fetch = repo.revparse_single("refs/remotes/origin/master")?.id();

        assert_ne!(pre_fetch, post_fetch, "refs/remotes/origin/master wasn't updated");

        let repo_file = repo_path.join("file.txt");
        let old_contents = String::from("Lorem ipsum dolor sit amet\n");
        assert_eq!(old_contents, fs::read_to_string(repo_file)?, "Contents of a file were updated after fetching");

        upstream_dir.close()?;
        repo_dir.close()?;
        Ok(())
    }

    #[test]
    fn test_pull() -> Result<(), Box<dyn Error>> {
        let (upstream_dir, upstream) = open_test_repo()?;
        let (repo_dir, repo) = clone_test_repo(&upstream_dir)?;
        let upstream_path = upstream_dir.path();
        let repo_path = repo_dir.path();

        let upstream_file = upstream_path.join("file.txt");
        let new_contents = write_assert!(upstream_file, "Lorem ipsum dolor sit amet\nNew line\n");

        add(&upstream, &["file.txt"])?;
        commit(&upstream, "Update file.txt")?;

        let old_head = repo.head()?.target();
        assert!(old_head.is_some(), "Invalid HEAD in the cloned repo");
        let old_head = old_head.unwrap();

        let pull_result = pull_repo(&repo)?;
        assert!(pull_result.is_ok());
        let new_head = pull_result.unwrap();

        assert_ne!(old_head, new_head, "HEAD wasn't updated");

        let repo_file = repo_path.join("file.txt");
        assert_eq!(new_contents, fs::read_to_string(repo_file)?, "Contents of a file were not updated after pulling");

        upstream_dir.close()?;
        repo_dir.close()?;
        Ok(())
    }

    #[test]
    fn test_diff() -> Result<(), Box<dyn Error>> {
        let (repo_dir, repo) = open_test_repo()?;
        let repo_path = repo_dir.path();

        let file1 = repo_path.join("file.txt");
        write_assert!(file1, "New line\nLorem ipsum dolor sit amet\nLine 3\n");
        let file2 = repo_path.join("meow.txt");
        write_assert!(file2, "Meow\nMeow\nMeow\nMeow\n:3\n:333\n");
        let file3 = repo_path.join("foo.txt");
        write_assert!(file3, "Foo bar baz\n");

        add(&repo, &["file.txt", "meow.txt"])?;

        let diff_bytes = diff_bytes(&repo)?;
        let diff = from_utf8(diff_bytes.as_slice())?;
        assert_eq!(r#"diff --git a/file.txt b/file.txt
index dc8344c..f6110d6 100644
--- a/file.txt
+++ b/file.txt
@@ -1 +1,3 @@
+New line
 Lorem ipsum dolor sit amet
+Line 3
diff --git a/meow.txt b/meow.txt
new file mode 100644
index 0000000..3676365
--- /dev/null
+++ b/meow.txt
@@ -0,0 +1,6 @@
+Meow
+Meow
+Meow
+Meow
+:3
+:333
"#, diff);

        Ok(())
    }

    #[test]
    fn test_checkout() -> Result<(), Box<dyn Error>> {
        let (upstream_dir, upstream) = open_test_repo()?;
        let (repo_dir, repo) = clone_test_repo(&upstream_dir)?;
        let upstream_path = upstream_dir.path();
        let repo_path = repo_dir.path();

        let head_commit = upstream.head()?.peel_to_commit()?;
        upstream.branch("test", &head_commit, false)?;
        let upstream_checkout_oid = repo_checkout(&upstream, "test".to_string())?;

        assert_eq!(head_commit.id(), upstream_checkout_oid, "Checked out a wrong ref");

        let upstream_file = upstream_path.join("file.txt");
        let new_contents = write_assert!(upstream_file, "Lorem ipsum dolor sit amet\nNew line\n");

        add(&upstream, &["file.txt"])?;
        let new_head_oid = commit(&upstream, "Update file.txt")?;

        fetch_repo(&repo)?;
        let checkout_oid = repo_checkout(&repo, "test".to_string())?;

        assert_eq!(new_head_oid, checkout_oid, "Checked out a wrong ref");

        let repo_file = repo_path.join("file.txt");
        assert_eq!(new_contents, fs::read_to_string(repo_file)?, "Contents of a file were not updated after checking out");

        upstream_dir.close()?;
        repo_dir.close()?;
        Ok(())
    }
}
