use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::Duration;

mod git_process;
#[cfg(windows)]
mod mutation_lock;
mod branches;
mod remotes;
mod history_search;
mod partial_staging;

pub use branches::{create_branch, switch_branch, Branch};
pub use remotes::{add_remote, fetch_remote, pull_remote, push_remote, update_remote, Remote};
pub use history_search::{search_repository_history, HistorySearchCursor, HistorySearchPage};
pub use partial_staging::{apply_partial_selection, prepare_partial_diff, PartialDiffResult};

const MAX_STATUS_BYTES: usize = 16 * 1024 * 1024;
const MAX_WORKTREE_BYTES: usize = 4 * 1024 * 1024;
const MAX_LOG_BYTES: usize = 4 * 1024 * 1024;
const MAX_DIFF_BYTES: usize = 4 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 1024 * 1024;
const MAX_INDEX_BYTES: usize = 64 * 1024 * 1024;
const MAX_COMMIT_MESSAGE_BYTES: usize = 1024 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);
const NETWORK_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
static MUTATION_LOCK: Mutex<()> = Mutex::new(());

/// A stable, machine-readable error kind for callers of the Git API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GitErrorKind {
    InvalidPath,
    NotRepository,
    GitUnavailable,
    GitCommandFailed,
    TimedOut,
    OutputLimit,
    InvalidOutput,
    InvalidDiffPath,
    InvalidRevision,
    InvalidHistorySearch,
    Unsupported,
    Busy,
    StaleState,
    InvalidMutationPath,
    InvalidCommitMessage,
    Conflicts,
    NothingStaged,
    InvalidBranchName,
    BranchNotFound,
}

/// An error returned by an Enogram Git operation. A failed write is not a
/// rollback: callers must refresh before the next action and must not retry it
/// automatically, because Git hooks or the command itself may have written data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitError {
    pub kind: GitErrorKind,
    pub message: String,
}

impl GitError {
    fn new(kind: GitErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GitError {}

/// A status record. `indexStatus` and `worktreeStatus` use porcelain-v2 status
/// characters. Commit-file records use the commit change code in `indexStatus`
/// and `.` in `worktreeStatus`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub path: String,
    pub original_path: Option<String>,
    pub index_status: String,
    pub worktree_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub oid: String,
    pub parents: Vec<String>,
    pub subject: String,
    pub author: String,
    pub date: String,
    pub refs: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Worktree {
    pub path: String,
    pub branch: String,
    pub head: String,
    pub is_bare: bool,
    pub is_detached: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositorySnapshot {
    pub repo_path: String,
    pub repo_name: String,
    pub branch: String,
    pub head: Option<String>,
    pub git_version: String,
    pub is_unborn: bool,
    pub merge_in_progress: bool,
    pub mutation_token: String,
    pub changes: Vec<Change>,
    pub commits: Vec<Commit>,
    pub worktrees: Vec<Worktree>,
    pub branches: Vec<Branch>,
    pub remotes: Vec<Remote>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationOutcome {
    pub snapshot: RepositorySnapshot,
    pub commit_oid: Option<String>,
    pub commit_tree_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffResult {
    pub text: String,
    pub truncated: bool,
    pub binary: bool,
}

/// Canonical directories that identify one checkout and its shared Git data.
/// Adapters can pin these at registration without granting access to discovered
/// worktrees or accepting a later Git metadata/configuration redirection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryIdentity {
    pub worktree_root: PathBuf,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
}

pub fn repository_identity(path: &str) -> Result<RepositoryIdentity, GitError> {
    let context = repository_context(path)?;
    let output = run_git(
        &context.git,
        &context.root,
        ["rev-parse", "--git-common-dir"],
        64 * 1024,
        false,
    )?;
    ensure_success(&output, "locate the shared Git directory")?;
    let common_path = PathBuf::from(
        output_text(&output.stdout, "shared Git directory")?.trim_end_matches(['\r', '\n']),
    );
    let common_dir = fs::canonicalize(context.root.join(common_path)).map_err(|_| {
        GitError::new(GitErrorKind::InvalidPath, "The shared Git directory is inaccessible.")
    })?;
    Ok(RepositoryIdentity {
        worktree_root: context.root,
        git_dir: context.git_dir,
        common_dir,
    })
}

/// Read status, recent history, and worktree information for a repository.
pub fn inspect_repository(path: &str) -> Result<RepositorySnapshot, GitError> {
    let context = repository_context(path)?;
    inspect_context(&context)
}

fn inspect_context(context: &RepositoryContext) -> Result<RepositorySnapshot, GitError> {
    let mutation_token = read_mutation_token(context)?;
    let git_version = get_git_version(&context.git)?;
    let (branch, head, is_unborn, changes) = read_status(&context)?;
    let merge_in_progress = context.git_dir.join("MERGE_HEAD").is_file();
    let commits = read_log(&context, !is_unborn)?;
    let worktrees = read_worktrees(&context)?;
    let branches = branches::read_branches(context)?;
    let remotes = remotes::read_remotes(context)?;
    if mutation_token != read_mutation_token(context)? {
        return Err(GitError::new(
            GitErrorKind::StaleState,
            "Repository HEAD, index, or operation state changed while refreshing. Refresh again.",
        ));
    }

    Ok(RepositorySnapshot {
        repo_path: path_to_string(&context.root)?,
        repo_name: context
            .root
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or_else(|| context.root.as_os_str().to_str().unwrap_or(""))
            .to_owned(),
        branch,
        head,
        git_version,
        is_unborn,
        merge_in_progress,
        mutation_token,
        changes,
        commits,
        worktrees,
        branches,
        remotes,
    })
}

/// Stage the current working-tree content of explicit status files. This is a
/// whole-file operation, including any previously unstaged part of that file.
pub fn stage_paths(
    repo_path: &str,
    paths: &[String],
    expected_token: &str,
) -> Result<MutationOutcome, GitError> {
    mutate_paths(repo_path, paths, expected_token, true)
}

/// Restore explicit files in the index to HEAD (or remove them from an unborn
/// index), leaving their working-tree contents untouched.
pub fn unstage_paths(
    repo_path: &str,
    paths: &[String],
    expected_token: &str,
) -> Result<MutationOutcome, GitError> {
    mutate_paths(repo_path, paths, expected_token, false)
}

/// Commit the existing staged index, using the user's Git identity, hooks, and
/// signing configuration. Report when the created tree differs from that index,
/// for example because a hook changed it. This never adds, amends, or retries.
/// A pending merge can be completed even when its tree is identical to HEAD.
pub fn commit_index(
    repo_path: &str,
    message: &str,
    expected_token: &str,
) -> Result<MutationOutcome, GitError> {
    let _guard = acquire_mutation_lock()?;
    if message.trim().is_empty()
        || message.contains('\0')
        || message.len() > MAX_COMMIT_MESSAGE_BYTES
    {
        return Err(GitError::new(
            GitErrorKind::InvalidCommitMessage,
            "Commit message must contain non-whitespace text, contain no NUL, and be at most 1 MiB.",
        ));
    }
    let context = repository_context(repo_path)?;
    validate_token(&context, expected_token)?;
    let (branch, head, _, changes) = read_status(&context)?;
    if changes.iter().any(is_conflict) {
        return Err(GitError::new(
            GitErrorKind::Conflicts,
            "Resolve all unmerged files before committing.",
        ));
    }
    if [
        "rebase-merge",
        "rebase-apply",
        "sequencer",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
    ]
    .iter()
    .any(|name| context.git_dir.join(name).exists())
    {
        return Err(GitError::new(
            GitErrorKind::Unsupported,
            "Finish the active rebase, cherry-pick, or revert in Git before committing here.",
        ));
    }
    let staged = run_git(
        &context.git,
        &context.root,
        [
            "--no-pager",
            "diff",
            "--cached",
            "--quiet",
            "--no-ext-diff",
            "--no-textconv",
            "--ignore-submodules=none",
        ],
        4096,
        false,
    )?;
    let merge_in_progress = context.git_dir.join("MERGE_HEAD").is_file();
    match staged.status.code() {
        Some(0) if !merge_in_progress => {
            return Err(GitError::new(
                GitErrorKind::NothingStaged,
                "Stage at least one change before committing.",
            ))
        }
        Some(0 | 1) => {}
        _ => ensure_success(&staged, "check staged changes")?,
    }

    let expected_tree_delta = read_tree_delta(&context, head.as_deref(), None)?;
    // Enogram guards serialize cooperating writes. Git owns its actual index/ref
    // locks; other Git processes can still race this last check and the write.
    validate_token(&context, expected_token)?;
    ensure_index_unlocked(&context)?;
    let result = run_git_with_input(
        &context.git,
        &context.root,
        ["--no-pager", "commit", "--file=-", "--cleanup=verbatim"],
        MAX_LOG_BYTES,
        true,
        Some(message.as_bytes()),
    );
    ensure_mutation_success(result, "commit the staged index")?;
    finish_mutation(
        &context,
        "Commit",
        Some((&branch, head.as_deref(), &expected_tree_delta)),
    )
}

/// Read either the working-tree diff, staged diff, or selected commit's diff.
/// Paths are repository-relative and treated as literal pathspecs.
pub fn read_diff(
    repo_path: &str,
    file_path: &str,
    staged: bool,
    commit_oid: Option<&str>,
) -> Result<DiffResult, GitError> {
    let context = repository_context(repo_path)?;
    let path = validate_relative_path(file_path)?;
    let mut allows_no_index_difference = false;

    let result = if let Some(oid) = commit_oid {
        let actual_oid = resolve_commit_oid(&context, oid)?;
        let parent = first_parent(&context, &actual_oid)?;
        let mut args = vec![
            OsString::from("--no-pager"),
            OsString::from("--literal-pathspecs"),
        ];
        if let Some(parent) = parent {
            args.extend([
                OsString::from("diff"),
                OsString::from(parent),
                OsString::from(actual_oid),
            ]);
        } else {
            args.extend([
                OsString::from("show"),
                OsString::from("--format="),
                OsString::from("--root"),
                OsString::from("--no-show-signature"),
                OsString::from("--no-notes"),
                OsString::from(actual_oid),
            ]);
        }
        args.extend([
            OsString::from("--no-ext-diff"),
            OsString::from("--no-textconv"),
            OsString::from("--no-color"),
            OsString::from("--find-renames"),
            OsString::from("--"),
            path.as_os_str().to_owned(),
        ]);
        run_git(&context.git, &context.root, args, MAX_DIFF_BYTES, true)?
    } else if !staged && is_untracked(&context, &path)? {
        allows_no_index_difference = true;
        read_untracked_diff(&context, &path)?
    } else {
        let mut args = vec![
            OsString::from("--no-pager"),
            OsString::from("--literal-pathspecs"),
            OsString::from("diff"),
            OsString::from("--no-ext-diff"),
            OsString::from("--no-textconv"),
            OsString::from("--no-color"),
        ];
        if staged {
            args.push(OsString::from("--cached"));
        }
        args.push(OsString::from("--"));
        args.push(path.as_os_str().to_owned());
        run_git(&context.git, &context.root, args, MAX_DIFF_BYTES, true)?
    };

    if allows_no_index_difference {
        if !result.status.success() && result.status.code() != Some(1) {
            return Err(GitError::new(
                GitErrorKind::GitCommandFailed,
                format!(
                    "Git could not read the untracked diff: {}",
                    stderr_text(&result)
                ),
            ));
        }
    } else {
        ensure_success(&result, "read diff")?;
    }
    let binary = diff_is_binary(&result.stdout);
    Ok(DiffResult {
        text: String::from_utf8_lossy(&result.stdout).into_owned(),
        truncated: result.stdout_truncated,
        binary,
    })
}

fn diff_is_binary(bytes: &[u8]) -> bool {
    let mut in_hunk = false;
    bytes.split(|byte| *byte == b'\n').any(|line| {
        if line.starts_with(b"diff --") {
            in_hunk = false;
        } else if line.starts_with(b"@@") {
            in_hunk = true;
        }
        // Binary markers are standalone metadata, never path or hunk content.
        !in_hunk
            && (line == b"GIT binary patch"
                || (line.starts_with(b"Binary files ") && line.ends_with(b" differ")))
    })
}

/// List the paths changed by a commit after validating that `commit_oid` is a
/// full object ID naming an existing commit.
pub fn list_commit_files(repo_path: &str, commit_oid: &str) -> Result<Vec<Change>, GitError> {
    let context = repository_context(repo_path)?;
    let actual_oid = resolve_commit_oid(&context, commit_oid)?;
    let parent = first_parent(&context, &actual_oid)?;
    let mut args = vec![
        OsString::from("--no-pager"),
        OsString::from("diff-tree"),
        OsString::from("--root"),
        OsString::from("--no-commit-id"),
        OsString::from("--name-status"),
        OsString::from("--find-renames"),
        OsString::from("-r"),
        OsString::from("-z"),
    ];
    if let Some(parent) = parent {
        args.push(OsString::from(parent));
    }
    args.push(OsString::from(actual_oid));
    let result = run_git(&context.git, &context.root, args, MAX_STATUS_BYTES, false)?;
    ensure_success(&result, "list commit files")?;
    if result.stdout_truncated {
        return Err(GitError::new(
            GitErrorKind::OutputLimit,
            "Commit file list exceeded the configured output limit.",
        ));
    }
    parse_commit_files(&result.stdout)
}

struct MutationGuard {
    #[cfg(windows)]
    _shared: mutation_lock::SharedWriteGuard,
    _local: MutexGuard<'static, ()>,
}

fn acquire_mutation_lock() -> Result<MutationGuard, GitError> {
    let local = MUTATION_LOCK.try_lock().map_err(|error| match error {
        TryLockError::WouldBlock => GitError::new(GitErrorKind::Busy, "Another Enogram Git write is still running."),
        TryLockError::Poisoned(_) => GitError::new(
            GitErrorKind::Busy,
            "A previous Git write was interrupted. Restart Enogram and refresh the repository before writing again.",
        ),
    })?;
    Ok(MutationGuard {
        _local: local,
        #[cfg(windows)]
        _shared: mutation_lock::acquire()?,
    })
}

fn mutate_paths(
    repo_path: &str,
    paths: &[String],
    expected_token: &str,
    stage: bool,
) -> Result<MutationOutcome, GitError> {
    let _guard = acquire_mutation_lock()?;
    let context = repository_context(repo_path)?;
    validate_token(&context, expected_token)?;
    let (_, _, is_unborn, changes) = read_status(&context)?;
    let selected = mutation_paths(&context, &changes, paths, stage)?;
    let input: Vec<u8> = selected
        .iter()
        .flat_map(|path| path.bytes().chain(std::iter::once(0)))
        .collect();
    let mut args = vec!["--no-pager", "--literal-pathspecs"];
    let operation = if stage {
        args.extend([
            "add",
            "--all",
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
        ]);
        "stage the selected files"
    } else if is_unborn {
        // --cached never removes worktree content. --force permits unstaging a
        // new file even when its staged content differs from the worktree.
        args.extend([
            "rm",
            "--cached",
            "--force",
            "--ignore-unmatch",
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
        ]);
        "unstage the selected files"
    } else {
        args.extend([
            "reset",
            "--quiet",
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
            "HEAD",
        ]);
        "unstage the selected files"
    };
    validate_token(&context, expected_token)?;
    ensure_index_unlocked(&context)?;
    let output = run_git_with_input(
        &context.git,
        &context.root,
        args,
        MAX_STATUS_BYTES,
        true,
        Some(&input),
    );
    ensure_mutation_success(output, operation)?;
    finish_mutation(&context, if stage { "Stage" } else { "Unstage" }, None)
}

fn mutation_paths(
    context: &RepositoryContext,
    changes: &[Change],
    supplied: &[String],
    stage: bool,
) -> Result<BTreeSet<String>, GitError> {
    if supplied.is_empty() {
        return Err(GitError::new(
            GitErrorKind::InvalidMutationPath,
            "Select at least one changed file.",
        ));
    }
    let mut selected = BTreeSet::new();
    for path in supplied {
        validate_mutation_path(context, path, changes)?;
        let change = changes.iter().find(|change| change.path == *path).ok_or_else(|| {
            GitError::new(GitErrorKind::InvalidMutationPath, "Selected path is not an exact file in the current status. Refresh and select it again.")
        })?;
        if is_conflict(change) {
            return Err(GitError::new(
                GitErrorKind::Conflicts,
                "Resolve selected unmerged files in Git before staging or unstaging them here.",
            ));
        }
        let applicable = if stage {
            change.worktree_status != "."
        } else {
            change.index_status != "."
        };
        if !applicable {
            return Err(GitError::new(
                GitErrorKind::InvalidMutationPath,
                if stage {
                    "A selected file has no unstaged changes. Refresh and select it again."
                } else {
                    "A selected file has no staged changes. Refresh and select it again."
                },
            ));
        }
        selected.insert(change.path.clone());
        if !stage && change.index_status == "R" {
            if let Some(original) = &change.original_path {
                // A staged rename is one status row but two index paths. Copy
                // records deliberately do not reset their unchanged source.
                validate_mutation_path(context, original, changes)?;
                selected.insert(original.clone());
            }
        }
    }
    Ok(selected)
}

fn validate_mutation_path(
    context: &RepositoryContext,
    supplied: &str,
    changes: &[Change],
) -> Result<(), GitError> {
    let invalid = || {
        GitError::new(
        GitErrorKind::InvalidMutationPath,
        "Writes require exact repository-relative file paths without traversal, directories, or Git metadata.",
    )
    };
    if supplied.is_empty() || supplied.contains('\0') || supplied.ends_with('/') {
        return Err(invalid());
    }
    let path = Path::new(supplied);
    if path.is_absolute()
        || supplied
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(invalid());
    }
    let components: Vec<_> = path.components().collect();
    if components.is_empty() {
        return Err(invalid());
    }
    for component in &components {
        match component {
            Component::Normal(name) => {
                let name = name.to_str().ok_or_else(invalid)?;
                if name
                    .trim_end_matches(['.', ' '])
                    .eq_ignore_ascii_case(".git")
                {
                    return Err(invalid());
                }
                #[cfg(windows)]
                if name.contains(':') || name.ends_with(['.', ' ']) || supplied.contains('\\') {
                    return Err(invalid());
                }
            }
            _ => return Err(invalid()),
        }
    }
    let mut parent = context.root.clone();
    for component in components.iter().take(components.len() - 1) {
        parent.push(component.as_os_str());
        match fs::symlink_metadata(&parent) {
            Ok(metadata) => {
                if !metadata.is_dir()
                    || metadata.file_type().is_symlink()
                    || !fs::canonicalize(&parent)
                        .map_err(|_| invalid())?
                        .starts_with(&context.root)
                {
                    return Err(invalid());
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(invalid()),
        }
    }
    match fs::symlink_metadata(context.root.join(path)) {
        Ok(metadata) if metadata.is_dir() => return Err(invalid()),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(invalid()),
    }
    let prefix = format!("{supplied}/");
    if changes
        .iter()
        .any(|change| change.path.starts_with(&prefix))
    {
        // Even literal Git pathspecs can match descendants. Reject file/dir
        // overlaps instead of silently acting on other status entries.
        return Err(invalid());
    }
    Ok(())
}

fn is_conflict(change: &Change) -> bool {
    change.index_status == "U"
        || change.worktree_status == "U"
        || matches!(
            (
                change.index_status.as_str(),
                change.worktree_status.as_str()
            ),
            ("A", "A") | ("D", "D")
        )
}

fn validate_token(context: &RepositoryContext, expected: &str) -> Result<(), GitError> {
    if expected.is_empty() || read_mutation_token(context)? != expected {
        return Err(GitError::new(
            GitErrorKind::StaleState,
            "Repository HEAD, index, or operation state changed since the displayed snapshot. Refresh before writing.",
        ));
    }
    Ok(())
}

fn ensure_index_unlocked(context: &RepositoryContext) -> Result<(), GitError> {
    if context.git_dir.join("index.lock").exists() {
        return Err(GitError::new(
            GitErrorKind::Busy,
            "The Git index is locked by another operation. Wait for it to finish and refresh.",
        ));
    }
    Ok(())
}

fn ensure_mutation_success(
    result: Result<CapturedOutput, GitError>,
    operation: &str,
) -> Result<(), GitError> {
    let result = result.and_then(|output| {
        if output.status.success() {
            Ok(())
        } else {
            let diagnostic = stderr_text(&output);
            let kind = if diagnostic.contains(".lock")
                && (diagnostic.contains("File exists")
                    || diagnostic.contains("another git process"))
            {
                GitErrorKind::Busy
            } else {
                GitErrorKind::GitCommandFailed
            };
            Err(GitError::new(
                kind,
                format!("Git could not {operation}: {diagnostic}"),
            ))
        }
    });
    result.map_err(|error| GitError::new(
        error.kind,
        format!("{} Repository state may have changed. Refresh before another action; do not automatically retry this write.", error.message),
    ))
}

fn finish_mutation(
    context: &RepositoryContext,
    operation: &str,
    expected_commit: Option<(&str, Option<&str>, &[(Vec<u8>, Vec<u8>)])>,
) -> Result<MutationOutcome, GitError> {
    let snapshot = inspect_context(context).map_err(|error| GitError::new(
        error.kind,
        format!("{operation} succeeded, but the updated snapshot could not be read: {} Refresh before another action; do not repeat the write automatically.", error.message),
    ))?;
    let (commit_oid, commit_tree_changed) = if let Some((branch, previous_head, expected_tree_delta)) = expected_commit {
        let changed_head = || {
            GitError::new(
            GitErrorKind::StaleState,
            "Git reported a successful commit, but HEAD or repository state changed unexpectedly afterward. A hook or another Git process may have moved it. Refresh and inspect history; do not repeat the commit automatically.",
        )
        };
        let oid = snapshot.head.as_deref().ok_or_else(changed_head)?;
        if snapshot.branch != branch || Some(oid) == previous_head {
            return Err(changed_head());
        }
        let parent = first_parent(context, oid).map_err(|error| GitError::new(
            error.kind,
            format!("Commit succeeded, but its parent could not be verified: {} Refresh and inspect history; do not repeat the commit automatically.", error.message),
        ))?;
        if parent.as_deref() != previous_head {
            return Err(changed_head());
        }
        let actual_tree_delta = read_tree_delta(context, previous_head, Some(oid)).map_err(|error| GitError::new(
            error.kind,
            format!("Commit succeeded, but the tree of commit {oid} could not be verified: {} Refresh and inspect history; do not repeat the commit automatically.", error.message),
        ))?;
        let current_token = read_mutation_token(context).map_err(|error| GitError::new(
            error.kind,
            format!("Commit succeeded, but its final state could not be verified: {} Refresh and inspect history; do not repeat the commit automatically.", error.message),
        ))?;
        if current_token != snapshot.mutation_token {
            return Err(changed_head());
        }
        (
            Some(oid.to_owned()),
            actual_tree_delta.as_slice() != expected_tree_delta,
        )
    } else {
        (None, false)
    };
    Ok(MutationOutcome {
        snapshot,
        commit_oid,
        commit_tree_changed,
    })
}

fn read_tree_delta(
    context: &RepositoryContext,
    baseline: Option<&str>,
    commit: Option<&str>,
) -> Result<Vec<(Vec<u8>, Vec<u8>)>, GitError> {
    // Full modes, object IDs, and paths against the same captured parent (or
    // the empty unborn base) uniquely describe the final tree. Unlike
    // write-tree, this does not update cache-tree data in the real index, so
    // the caller can still revalidate its original mutation token afterward.
    let mut args = vec!["--no-pager"];
    if commit.is_some() {
        args.extend(["diff-tree", "--root", "--no-commit-id"]);
    } else {
        args.extend(["diff", "--cached"]);
    }
    args.extend([
        "-r",
        "--raw",
        "--no-abbrev",
        "-z",
        "--no-renames",
        "--no-ext-diff",
        "--no-textconv",
        "--ignore-submodules=none",
        "--no-relative",
        "--no-color",
        "--ita-invisible-in-index",
    ]);
    if let Some(baseline) = baseline {
        args.push(baseline);
    }
    if let Some(commit) = commit {
        args.push(commit);
    }
    args.push("--");
    let output = run_git(&context.git, &context.root, args, MAX_STATUS_BYTES, false)?;
    ensure_success(&output, "read the prepared or committed tree changes")?;
    parse_tree_delta(&output.stdout)
}

fn parse_tree_delta(bytes: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, GitError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let invalid = || {
        GitError::new(
            GitErrorKind::InvalidOutput,
            "Git returned an invalid raw tree change record.",
        )
    };
    let fields = nul_records(bytes.strip_suffix(b"\0").ok_or_else(invalid)?);
    if fields.len() % 2 != 0 {
        return Err(invalid());
    }
    let mut changes = Vec::new();
    for record in fields.chunks_exact(2) {
        let header = record[0];
        let path = record[1];
        let parts: Vec<_> = header
            .strip_prefix(b":")
            .ok_or_else(invalid)?
            .split(|byte| *byte == b' ')
            .collect();
        let valid_mode = |mode: &[u8]| {
            mode.len() == 6 && mode.iter().all(|byte| matches!(byte, b'0'..=b'7'))
        };
        let valid_oid = |oid: &[u8]| {
            matches!(oid.len(), 40 | 64) && oid.iter().all(u8::is_ascii_hexdigit)
        };
        if parts.len() != 5
            || !valid_mode(parts[0])
            || !valid_mode(parts[1])
            || !valid_oid(parts[2])
            || !valid_oid(parts[3])
            || parts[2].len() != parts[3].len()
            || parts[4].is_empty()
            || path.is_empty()
        {
            return Err(invalid());
        }
        changes.push((path.to_vec(), header.to_vec()));
    }
    // Hooks can change diff.orderFile; ordering is not part of a tree's identity.
    changes.sort_unstable();
    Ok(changes)
}

fn read_mutation_token(context: &RepositoryContext) -> Result<String, GitError> {
    // Git hashes this data without -w, so no object or index is written. Include
    // worktree identity and symbolic HEAD as well as its target: branches and
    // linked worktrees may otherwise share identical HEAD and index contents.
    let mut fingerprint = b"enogram-mutation-v1\0".to_vec();
    append_fingerprint_field(&mut fingerprint, path_to_string(&context.root)?.as_bytes());
    append_fingerprint_field(
        &mut fingerprint,
        path_to_string(&context.git_dir)?.as_bytes(),
    );
    append_optional_file(&mut fingerprint, &context.git_dir.join("HEAD"), 64 * 1024)?;
    let head = run_git(
        &context.git,
        &context.root,
        [
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            "HEAD^{commit}",
        ],
        4096,
        false,
    )?;
    if !head.status.success() && head.status.code() != Some(1) {
        ensure_success(&head, "read HEAD for the mutation token")?;
    }
    append_fingerprint_field(&mut fingerprint, &head.stdout);
    append_optional_file(
        &mut fingerprint,
        &context.git_dir.join("index"),
        MAX_INDEX_BYTES,
    )?;
    for name in [
        "MERGE_HEAD",
        "MERGE_MODE",
        "MERGE_MSG",
        "AUTO_MERGE",
        "SQUASH_MSG",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "REBASE_HEAD",
    ] {
        append_fingerprint_field(&mut fingerprint, name.as_bytes());
        append_optional_file(
            &mut fingerprint,
            &context.git_dir.join(name),
            MAX_COMMIT_MESSAGE_BYTES,
        )?;
    }
    for name in ["rebase-merge", "rebase-apply", "sequencer"] {
        append_fingerprint_field(&mut fingerprint, name.as_bytes());
        let directory = context.git_dir.join(name);
        match fs::read_dir(&directory) {
            Ok(entries) => {
                fingerprint.push(1);
                let mut files = entries
                    .map(|entry| entry.map(|entry| entry.path()))
                    .collect::<io::Result<Vec<_>>>()
                    .map_err(fingerprint_io_error)?;
                if files.len() > 128 {
                    return Err(GitError::new(
                        GitErrorKind::OutputLimit,
                        "Git operation state has too many files to fingerprint.",
                    ));
                }
                files.sort();
                for file in files {
                    append_fingerprint_field(&mut fingerprint, path_to_string(&file)?.as_bytes());
                    if file.is_file() {
                        append_optional_file(&mut fingerprint, &file, MAX_COMMIT_MESSAGE_BYTES)?;
                    } else {
                        fingerprint.push(2);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => fingerprint.push(0),
            Err(error) => return Err(fingerprint_io_error(error)),
        }
    }
    let output = run_git_with_input(
        &context.git,
        &context.root,
        ["hash-object", "--no-filters", "--stdin"],
        4096,
        false,
        Some(&fingerprint),
    )?;
    ensure_success(&output, "fingerprint repository state")?;
    let hash = output_text(&output.stdout, "mutation token")?.trim();
    if !matches!(hash.len(), 40 | 64) || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(GitError::new(
            GitErrorKind::InvalidOutput,
            "Git returned an invalid state fingerprint.",
        ));
    }
    Ok(format!("v1:{hash}"))
}

fn append_fingerprint_field(target: &mut Vec<u8>, bytes: &[u8]) {
    target.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    target.extend_from_slice(bytes);
}

fn append_optional_file(target: &mut Vec<u8>, path: &Path, limit: usize) -> Result<(), GitError> {
    match fs::File::open(path) {
        Ok(file) => {
            target.push(1);
            let mut bytes = Vec::new();
            file.take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(fingerprint_io_error)?;
            if bytes.len() > limit {
                return Err(GitError::new(
                    GitErrorKind::OutputLimit,
                    "Git index or operation state exceeds the fingerprint size limit.",
                ));
            }
            append_fingerprint_field(target, &bytes);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => target.push(0),
        Err(error) => return Err(fingerprint_io_error(error)),
    }
    Ok(())
}

fn fingerprint_io_error(error: io::Error) -> GitError {
    GitError::new(
        GitErrorKind::GitCommandFailed,
        format!("Could not read Git state for the mutation token: {error}"),
    )
}

struct RepositoryContext {
    git: PathBuf,
    root: PathBuf,
    git_dir: PathBuf,
}

#[derive(Debug)]
struct CapturedOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_truncated: bool,
    stderr_truncated: bool,
    status: ExitStatus,
}

fn repository_context(path: &str) -> Result<RepositoryContext, GitError> {
    let input_path = Path::new(path);
    let canonical_input = fs::canonicalize(input_path).map_err(|error| {
        GitError::new(
            GitErrorKind::InvalidPath,
            format!("Repository path is missing or inaccessible: {error}"),
        )
    })?;
    if !canonical_input.is_dir() {
        return Err(GitError::new(
            GitErrorKind::InvalidPath,
            "Repository path must be an existing directory.",
        ));
    }

    let git = resolve_git()?;
    let top = run_git(
        &git,
        &canonical_input,
        [
            OsString::from("--no-pager"),
            OsString::from("rev-parse"),
            OsString::from("--show-toplevel"),
        ],
        64 * 1024,
        false,
    )?;
    if !top.status.success() {
        return Err(GitError::new(
            GitErrorKind::NotRepository,
            format!("Git could not open this worktree: {}", stderr_text(&top)),
        ));
    }
    let top_text = output_text(&top.stdout, "repository root")?;
    let root_from_git = PathBuf::from(top_text.trim_end_matches(['\r', '\n']));
    let root = fs::canonicalize(&root_from_git).map_err(|error| {
        GitError::new(
            GitErrorKind::InvalidPath,
            format!("Git returned an inaccessible worktree root: {error}"),
        )
    })?;
    let git_dir_output = run_git(
        &git,
        &root,
        ["rev-parse", "--absolute-git-dir"],
        64 * 1024,
        false,
    )?;
    ensure_success(&git_dir_output, "locate the worktree Git directory")?;
    let git_dir_text =
        output_text(&git_dir_output.stdout, "Git directory")?.trim_end_matches(['\r', '\n']);
    let git_dir = fs::canonicalize(git_dir_text).map_err(|error| {
        GitError::new(
            GitErrorKind::InvalidPath,
            format!("Git directory is inaccessible: {error}"),
        )
    })?;
    Ok(RepositoryContext { git, root, git_dir })
}

fn resolve_git() -> Result<PathBuf, GitError> {
    let mut candidates = Vec::new();
    #[cfg(windows)]
    {
        candidates.push(PathBuf::from(r"C:\Program Files\Git\cmd\git.exe"));
        candidates.push(PathBuf::from(r"C:\Program Files\Git\bin\git.exe"));
        candidates.push(PathBuf::from(r"C:\Program Files (x86)\Git\cmd\git.exe"));
        for key in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(base) = std::env::var_os(key) {
                let base = PathBuf::from(base);
                candidates.push(base.join(r"Git\cmd\git.exe"));
                candidates.push(base.join(r"Git\bin\git.exe"));
            }
        }
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            candidates.push(PathBuf::from(local_app_data).join(r"Programs\Git\cmd\git.exe"));
        }
    }
    #[cfg(not(windows))]
    {
        candidates.extend([
            PathBuf::from("/usr/bin/git"),
            PathBuf::from("/usr/local/bin/git"),
            PathBuf::from("/opt/homebrew/bin/git"),
        ]);
    }

    for candidate in candidates {
        if let Ok(path) = fs::canonicalize(candidate) {
            if path.is_absolute() && path.is_file() {
                return Ok(path);
            }
        }
    }
    Err(GitError::new(
        GitErrorKind::GitUnavailable,
        "Could not find Git in a supported absolute installation path.",
    ))
}

fn run_git<I, S>(
    git: &Path,
    current_dir: &Path,
    args: I,
    stdout_limit: usize,
    allow_stdout_truncation: bool,
) -> Result<CapturedOutput, GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_git_with_input(
        git,
        current_dir,
        args,
        stdout_limit,
        allow_stdout_truncation,
        None,
    )
}

fn run_git_with_input<I, S>(
    git: &Path,
    current_dir: &Path,
    args: I,
    stdout_limit: usize,
    allow_stdout_truncation: bool,
    input: Option<&[u8]>,
) -> Result<CapturedOutput, GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = git_command(git, current_dir, args);
    git_process::run(
        &mut command,
        stdout_limit,
        allow_stdout_truncation,
        input,
        COMMAND_TIMEOUT,
    )
}

fn run_git_network<I, S>(
    git: &Path,
    current_dir: &Path,
    args: I,
    stdout_limit: usize,
    allow_stdout_truncation: bool,
) -> Result<CapturedOutput, GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = git_command(git, current_dir, args);
    // Reuse saved credentials without opening a hidden credential dialog.
    // SSH and other helpers remain owned by the same bounded process job.
    command
        .env("GCM_INTERACTIVE", "Never")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        .env("SSH_ASKPASS_REQUIRE", "never");
    git_process::run(
        &mut command,
        stdout_limit,
        allow_stdout_truncation,
        None,
        NETWORK_COMMAND_TIMEOUT,
    )
}

fn git_command<I, S>(git: &Path, current_dir: &Path, args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(git);
    command
        .arg("-c")
        .arg("core.fsmonitor=false")
        .args(args)
        .current_dir(current_dir)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_PREFIX")
        .env_remove("GIT_CONFIG")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_LITERAL_PATHSPECS")
        .env_remove("GIT_GLOB_PATHSPECS")
        .env_remove("GIT_NOGLOB_PATHSPECS")
        .env_remove("GIT_ICASE_PATHSPECS");
    // Avoid fsmonitor side effects during reads. Explicit write commands still
    // use normal user identity, clean filters, commit hooks, and signing.
    command
}

fn ensure_success(output: &CapturedOutput, operation: &str) -> Result<(), GitError> {
    if output.status.success() {
        return Ok(());
    }
    Err(GitError::new(
        GitErrorKind::GitCommandFailed,
        format!("Git could not {operation}: {}", stderr_text(output)),
    ))
}

fn stderr_text(output: &CapturedOutput) -> String {
    let mut text = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if output.stderr_truncated {
        text.push_str(" … (stderr truncated)");
    }
    if text.is_empty() {
        "Git returned an error without diagnostic output.".to_owned()
    } else {
        text
    }
}

fn get_git_version(git: &Path) -> Result<String, GitError> {
    let output = run_git(
        git,
        Path::new(std::env::current_dir().as_deref().unwrap_or(Path::new("."))),
        [OsString::from("--version")],
        4096,
        false,
    )?;
    ensure_success(&output, "read its version")?;
    Ok(output_text(&output.stdout, "Git version")?
        .trim()
        .to_owned())
}

fn read_status(
    context: &RepositoryContext,
) -> Result<(String, Option<String>, bool, Vec<Change>), GitError> {
    let output = run_git(
        &context.git,
        &context.root,
        [
            OsString::from("--no-pager"),
            OsString::from("status"),
            OsString::from("--porcelain=v2"),
            OsString::from("-z"),
            OsString::from("--untracked-files=all"),
            OsString::from("--branch"),
            OsString::from("--renames"),
            OsString::from("--ignore-submodules=none"),
        ],
        MAX_STATUS_BYTES,
        false,
    )?;
    ensure_success(&output, "read status")?;
    parse_status(&output.stdout)
}

fn parse_status(bytes: &[u8]) -> Result<(String, Option<String>, bool, Vec<Change>), GitError> {
    let records = nul_records(bytes);
    let mut branch = String::new();
    let mut head = None;
    let mut is_unborn = false;
    let mut changes = Vec::new();
    let mut index = 0;
    while index < records.len() {
        let record = records[index];
        index += 1;
        if record.is_empty() {
            continue;
        }
        if record.starts_with(b"# ") {
            if let Some(value) = record.strip_prefix(b"# branch.head ") {
                branch = utf8(value, "branch name")?.to_owned();
            } else if let Some(value) = record.strip_prefix(b"# branch.oid ") {
                let value = utf8(value, "HEAD object ID")?;
                if value == "(initial)" {
                    is_unborn = true;
                    head = None;
                } else {
                    head = Some(value.to_owned());
                }
            }
            continue;
        }

        match record.first().copied() {
            Some(b'1') => changes.push(parse_porcelain_change(record, 8, None)?),
            Some(b'2') => {
                let mut change = parse_porcelain_change(record, 9, None)?;
                let original = records.get(index).ok_or_else(|| {
                    GitError::new(
                        GitErrorKind::InvalidOutput,
                        "Rename status record is missing its original path.",
                    )
                })?;
                change.original_path = Some(utf8(original, "renamed source path")?.to_owned());
                index += 1;
                changes.push(change);
            }
            Some(b'u') => changes.push(parse_porcelain_change(record, 10, None)?),
            Some(b'?') if record.starts_with(b"? ") => {
                changes.push(Change {
                    path: utf8(&record[2..], "untracked path")?.to_owned(),
                    original_path: None,
                    index_status: ".".to_owned(),
                    worktree_status: "?".to_owned(),
                });
            }
            _ => {
                return Err(GitError::new(
                    GitErrorKind::InvalidOutput,
                    "Git returned an unrecognized porcelain-v2 status record.",
                ));
            }
        }
    }
    if branch.is_empty() {
        branch = "HEAD".to_owned();
    }
    Ok((branch, head, is_unborn, changes))
}

fn parse_porcelain_change(
    record: &[u8],
    field_count: usize,
    original_path: Option<String>,
) -> Result<Change, GitError> {
    let (fields, path) = split_fixed_fields(record, field_count)?;
    let status_index = if record.first() == Some(&b'u') { 1 } else { 1 };
    let xy = fields.get(status_index).ok_or_else(|| {
        GitError::new(
            GitErrorKind::InvalidOutput,
            "Git returned a status record without an XY field.",
        )
    })?;
    if xy.len() != 2 {
        return Err(GitError::new(
            GitErrorKind::InvalidOutput,
            "Git returned an invalid status XY field.",
        ));
    }
    Ok(Change {
        path: utf8(path, "changed path")?.to_owned(),
        original_path,
        index_status: char::from(xy[0]).to_string(),
        worktree_status: char::from(xy[1]).to_string(),
    })
}

fn split_fixed_fields(record: &[u8], count: usize) -> Result<(Vec<&[u8]>, &[u8]), GitError> {
    let mut fields = Vec::with_capacity(count);
    let mut start = 0;
    for _ in 0..count {
        let relative = record[start..]
            .iter()
            .position(|byte| *byte == b' ')
            .ok_or_else(|| {
                GitError::new(
                    GitErrorKind::InvalidOutput,
                    "Git returned a truncated status record.",
                )
            })?;
        let delimiter = start + relative;
        fields.push(&record[start..delimiter]);
        start = delimiter + 1;
    }
    Ok((fields, &record[start..]))
}

fn read_log(context: &RepositoryContext, has_head: bool) -> Result<Vec<Commit>, GitError> {
    let mut args = vec![
        OsString::from("--no-pager"),
        OsString::from("log"),
        OsString::from("--all"),
        OsString::from("--topo-order"),
        OsString::from("--max-count=80"),
        OsString::from("--no-color"),
        OsString::from("--no-show-signature"),
        OsString::from("--no-notes"),
        OsString::from("--pretty=tformat:%H%x00%P%x00%s%x00%an%x00%aI%x00%D"),
        OsString::from("-z"),
    ];
    // --all includes other branches even when this HEAD is unborn. Explicit
    // HEAD also keeps a detached commit that is not reachable from any ref.
    if has_head {
        args.push(OsString::from("HEAD"));
    }
    args.push(OsString::from("--"));
    let output = run_git(&context.git, &context.root, args, MAX_LOG_BYTES, false)?;
    ensure_success(&output, "read recent commits")?;
    parse_log(&output.stdout)
}

fn parse_log(bytes: &[u8]) -> Result<Vec<Commit>, GitError> {
    let mut records = nul_records(bytes);
    if records.last().is_some_and(|record| record.is_empty()) {
        records.pop();
    }
    if records.len() % 6 != 0 {
        return Err(GitError::new(
            GitErrorKind::InvalidOutput,
            "Git returned an incomplete log record.",
        ));
    }
    let mut commits = Vec::new();
    for fields in records.chunks_exact(6) {
        commits.push(commit_from_fields(fields)?);
    }
    if commits.len() > 80 {
        commits.truncate(80);
    }
    Ok(commits)
}

fn commit_from_fields(fields: &[&[u8]]) -> Result<Commit, GitError> {
    let parents_text = utf8(fields[1], "commit parents")?;
    Ok(Commit {
        oid: utf8(fields[0], "commit object ID")?.to_owned(),
        parents: parents_text
            .split_ascii_whitespace()
            .map(str::to_owned)
            .collect(),
        subject: utf8(fields[2], "commit subject")?.to_owned(),
        author: utf8(fields[3], "commit author")?.to_owned(),
        date: utf8(fields[4], "commit date")?.to_owned(),
        refs: utf8(fields[5], "commit refs")?.to_owned(),
    })
}

fn read_worktrees(context: &RepositoryContext) -> Result<Vec<Worktree>, GitError> {
    let output = run_git(
        &context.git,
        &context.root,
        [
            OsString::from("--no-pager"),
            OsString::from("worktree"),
            OsString::from("list"),
            OsString::from("--porcelain"),
            OsString::from("-z"),
        ],
        MAX_WORKTREE_BYTES,
        false,
    )?;
    ensure_success(&output, "list worktrees")?;
    parse_worktrees(&output.stdout)
}

fn parse_worktrees(bytes: &[u8]) -> Result<Vec<Worktree>, GitError> {
    let mut worktrees = Vec::new();
    let mut record = Vec::new();
    for field in nul_records(bytes) {
        if field.is_empty() {
            if !record.is_empty() {
                worktrees.push(worktree_from_fields(&record)?);
                record.clear();
            }
        } else {
            record.push(field);
        }
    }
    if !record.is_empty() {
        worktrees.push(worktree_from_fields(&record)?);
    }
    Ok(worktrees)
}

fn worktree_from_fields(fields: &[&[u8]]) -> Result<Worktree, GitError> {
    let mut path = None;
    let mut head = None;
    let mut branch = String::new();
    let mut is_bare = false;
    let mut is_detached = false;
    for field in fields {
        if let Some(value) = field.strip_prefix(b"worktree ") {
            path = Some(utf8(value, "worktree path")?.to_owned());
        } else if let Some(value) = field.strip_prefix(b"HEAD ") {
            head = Some(utf8(value, "worktree HEAD")?.to_owned());
        } else if let Some(value) = field.strip_prefix(b"branch ") {
            let reference = utf8(value, "worktree branch")?;
            branch = reference
                .strip_prefix("refs/heads/")
                .unwrap_or(reference)
                .to_owned();
        } else if *field == b"bare" {
            is_bare = true;
        } else if *field == b"detached" {
            is_detached = true;
        }
    }
    if branch.is_empty() && is_detached {
        branch = "HEAD".to_owned();
    }
    Ok(Worktree {
        path: path.ok_or_else(|| {
            GitError::new(GitErrorKind::InvalidOutput, "Worktree record has no path.")
        })?,
        branch,
        head: match head {
            Some(head) if !head.is_empty() => head,
            _ if is_bare => String::new(),
            _ => {
                return Err(GitError::new(
                    GitErrorKind::InvalidOutput,
                    "Worktree record has no HEAD.",
                ))
            }
        },
        is_bare,
        is_detached,
    })
}

fn parse_commit_files(bytes: &[u8]) -> Result<Vec<Change>, GitError> {
    let records = nul_records(bytes);
    let mut changes = Vec::new();
    let mut index = 0;
    while index < records.len() {
        let status = records[index];
        index += 1;
        if status.is_empty() {
            continue;
        }
        let status_text = utf8(status, "commit change status")?;
        let code = status_text.chars().next().ok_or_else(|| {
            GitError::new(
                GitErrorKind::InvalidOutput,
                "Git returned an empty commit change status.",
            )
        })?;
        if code == 'R' || code == 'C' {
            let old_path = records.get(index).ok_or_else(|| {
                GitError::new(
                    GitErrorKind::InvalidOutput,
                    "Rename status record is missing its source path.",
                )
            })?;
            let new_path = records.get(index + 1).ok_or_else(|| {
                GitError::new(
                    GitErrorKind::InvalidOutput,
                    "Rename status record is missing its destination path.",
                )
            })?;
            changes.push(Change {
                path: utf8(new_path, "commit destination path")?.to_owned(),
                original_path: Some(utf8(old_path, "commit source path")?.to_owned()),
                index_status: code.to_string(),
                worktree_status: ".".to_owned(),
            });
            index += 2;
        } else {
            let path = records.get(index).ok_or_else(|| {
                GitError::new(
                    GitErrorKind::InvalidOutput,
                    "Commit change record is missing its path.",
                )
            })?;
            changes.push(Change {
                path: utf8(path, "commit path")?.to_owned(),
                original_path: None,
                index_status: code.to_string(),
                worktree_status: ".".to_owned(),
            });
            index += 1;
        }
    }
    Ok(changes)
}

fn resolve_commit_oid(context: &RepositoryContext, supplied: &str) -> Result<String, GitError> {
    if !matches!(supplied.len(), 40 | 64) || !supplied.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(GitError::new(
            GitErrorKind::InvalidRevision,
            "Commit must be named by a full hexadecimal object ID.",
        ));
    }
    let expression = format!("{supplied}^{{commit}}");
    let output = run_git(
        &context.git,
        &context.root,
        [
            OsString::from("--no-pager"),
            OsString::from("rev-parse"),
            OsString::from("--verify"),
            OsString::from("--end-of-options"),
            OsString::from(expression),
        ],
        4096,
        false,
    )?;
    if !output.status.success() {
        return Err(GitError::new(
            GitErrorKind::InvalidRevision,
            "Commit object ID does not name an available commit.",
        ));
    }
    let actual = output_text(&output.stdout, "resolved commit object ID")?
        .trim()
        .to_owned();
    if actual.len() != supplied.len() || !actual.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(GitError::new(
            GitErrorKind::InvalidOutput,
            "Git returned an invalid resolved commit ID.",
        ));
    }
    Ok(actual)
}

fn first_parent(context: &RepositoryContext, oid: &str) -> Result<Option<String>, GitError> {
    let output = run_git(
        &context.git,
        &context.root,
        ["rev-list", "--parents", "--max-count=1", oid, "--"],
        4096,
        false,
    )?;
    ensure_success(&output, "read the commit's first parent")?;
    let text = output_text(&output.stdout, "commit parents")?;
    let mut fields = text.split_ascii_whitespace();
    if fields.next() != Some(oid) {
        return Err(GitError::new(
            GitErrorKind::InvalidOutput,
            "Git returned an unexpected commit parent record.",
        ));
    }
    let parent = fields.next().map(str::to_owned);
    if parent.as_ref().is_some_and(|value| {
        value.len() != oid.len() || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        return Err(GitError::new(
            GitErrorKind::InvalidOutput,
            "Git returned an invalid first parent ID.",
        ));
    }
    Ok(parent)
}

fn is_untracked(context: &RepositoryContext, path: &Path) -> Result<bool, GitError> {
    let output = run_git(
        &context.git,
        &context.root,
        [
            OsString::from("--no-pager"),
            OsString::from("--literal-pathspecs"),
            OsString::from("status"),
            OsString::from("--porcelain=v2"),
            OsString::from("-z"),
            OsString::from("--untracked-files=all"),
            OsString::from("--"),
            path.as_os_str().to_owned(),
        ],
        1024 * 1024,
        false,
    )?;
    ensure_success(&output, "check whether a file is untracked")?;
    Ok(nul_records(&output.stdout)
        .iter()
        .any(|record| record.starts_with(b"? ")))
}

fn read_untracked_diff(
    context: &RepositoryContext,
    path: &Path,
) -> Result<CapturedOutput, GitError> {
    let full_path = context.root.join(path);
    let metadata = fs::symlink_metadata(&full_path).map_err(|error| {
        GitError::new(
            GitErrorKind::InvalidDiffPath,
            format!("Untracked diff target is missing or inaccessible: {error}"),
        )
    })?;
    let canonical = fs::canonicalize(&full_path).map_err(|error| {
        GitError::new(
            GitErrorKind::InvalidDiffPath,
            format!("Could not resolve untracked diff target: {error}"),
        )
    })?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || !canonical.starts_with(&context.root)
    {
        return Err(GitError::new(
            GitErrorKind::Unsupported,
            "Untracked diff supports regular files located inside the repository only.",
        ));
    }
    let output = run_git(
        &context.git,
        &context.root,
        [
            OsString::from("--no-pager"),
            OsString::from("--literal-pathspecs"),
            OsString::from("diff"),
            OsString::from("--no-index"),
            OsString::from("--no-ext-diff"),
            OsString::from("--no-textconv"),
            OsString::from("--no-color"),
            OsString::from("--"),
            OsString::from("/dev/null"),
            path.as_os_str().to_owned(),
        ],
        MAX_DIFF_BYTES,
        true,
    )?;
    // `git diff --no-index` uses exit code 1 to mean that the files differ.
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(GitError::new(
            GitErrorKind::GitCommandFailed,
            format!(
                "Git could not read the untracked diff: {}",
                stderr_text(&output)
            ),
        ));
    }
    Ok(output)
}

fn validate_relative_path(file_path: &str) -> Result<PathBuf, GitError> {
    if file_path.is_empty() || file_path.contains('\0') {
        return Err(GitError::new(
            GitErrorKind::InvalidDiffPath,
            "Diff path must be a non-empty relative path.",
        ));
    }
    let path = Path::new(file_path);
    if path.is_absolute() {
        return Err(GitError::new(
            GitErrorKind::InvalidDiffPath,
            "Diff path must be repository-relative.",
        ));
    }
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(GitError::new(
                    GitErrorKind::InvalidDiffPath,
                    "Diff path cannot escape the repository root.",
                ));
            }
        }
    }
    if path
        .components()
        .all(|component| matches!(component, Component::CurDir))
    {
        return Err(GitError::new(
            GitErrorKind::InvalidDiffPath,
            "Diff path must identify a file.",
        ));
    }
    Ok(path.to_path_buf())
}

fn output_text<'a>(bytes: &'a [u8], label: &str) -> Result<&'a str, GitError> {
    std::str::from_utf8(bytes).map_err(|_| {
        GitError::new(
            GitErrorKind::InvalidOutput,
            format!("Git returned non-UTF-8 {label}."),
        )
    })
}

fn utf8<'a>(bytes: &'a [u8], label: &str) -> Result<&'a str, GitError> {
    output_text(bytes, label)
}

fn path_to_string(path: &Path) -> Result<String, GitError> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        GitError::new(
            GitErrorKind::InvalidOutput,
            "Filesystem path is not valid Unicode.",
        )
    })
}

fn nul_records(bytes: &[u8]) -> Vec<&[u8]> {
    bytes.split(|byte| *byte == 0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);
    pub(super) static MUTATION_TEST_LOCK: Mutex<()> = Mutex::new(());

    struct Fixture {
        path: PathBuf,
        git: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let git = resolve_git().expect("Git is installed for integration tests");
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("enogram-core-fixture-{}-{id}", std::process::id()));
            fs::create_dir_all(&path).expect("create fixture directory");
            let fixture = Self { path, git };
            fixture.git_ok(["init", "-b", "main"]);
            // Isolate fixtures from real identity, signing, line endings, and
            // hooks. Production commands keep the user's own configuration.
            fixture.git_ok(["config", "user.name", "Enogram Test"]);
            fixture.git_ok(["config", "user.email", "enogram@example.invalid"]);
            fixture.git_ok(["config", "commit.gpgsign", "false"]);
            fixture.git_ok(["config", "core.autocrlf", "false"]);
            let hooks = fixture.path.join(".git").join("test-hooks");
            fs::create_dir_all(&hooks).expect("create test hook directory");
            fixture.git_ok([
                OsString::from("config"),
                OsString::from("core.hooksPath"),
                hooks.into_os_string(),
            ]);
            fixture
        }

        fn git_ok<I, S>(&self, args: I)
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            let output = Command::new(&self.git)
                .args(args)
                .current_dir(&self.path)
                .env("GIT_AUTHOR_NAME", "Enogram Test")
                .env("GIT_AUTHOR_EMAIL", "enogram@example.invalid")
                .env("GIT_COMMITTER_NAME", "Enogram Test")
                .env("GIT_COMMITTER_EMAIL", "enogram@example.invalid")
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()
                .expect("run Git fixture command");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        fn commit(&self, subject: &str) -> String {
            self.git_ok(["add", "--all"]);
            let mut args = vec![
                OsString::from("commit"),
                OsString::from("-m"),
                OsString::from(subject),
            ];
            let output = Command::new(&self.git)
                .args(args.drain(..))
                .current_dir(&self.path)
                .env("GIT_AUTHOR_NAME", "Enogram Test")
                .env("GIT_AUTHOR_EMAIL", "enogram@example.invalid")
                .env("GIT_COMMITTER_NAME", "Enogram Test")
                .env("GIT_COMMITTER_EMAIL", "enogram@example.invalid")
                .output()
                .expect("commit fixture changes");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = Command::new(&self.git)
                .args(["rev-parse", "HEAD"])
                .current_dir(&self.path)
                .output()
                .expect("read fixture HEAD");
            String::from_utf8(output.stdout)
                .expect("HEAD is UTF-8")
                .trim()
                .to_owned()
        }

        fn path_str(&self) -> &str {
            self.path.to_str().expect("fixture path is UTF-8")
        }

        fn git_text(&self, args: &[&str]) -> String {
            let output = Command::new(&self.git)
                .args(args)
                .current_dir(&self.path)
                .output()
                .expect("read fixture Git output");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).expect("fixture output is UTF-8")
        }

        fn snapshot(&self) -> RepositorySnapshot {
            inspect_repository(self.path_str()).expect("inspect fixture")
        }

        fn write(&self, path: &str, contents: &str) {
            fs::write(self.path.join(path), contents).expect("write fixture file");
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn inspect_unborn_repository_reports_branch_and_empty_head() {
        let fixture = Fixture::new();
        let snapshot = inspect_repository(fixture.path_str()).expect("inspect unborn repo");
        assert_eq!(snapshot.branch, "main");
        assert_eq!(snapshot.head, None);
        assert!(snapshot.is_unborn);
        assert!(snapshot.commits.is_empty());
        assert_eq!(snapshot.worktrees.len(), 1);
        assert_eq!(snapshot.worktrees[0].branch, "main");
    }

    #[test]
    fn partial_stage_status_and_both_diff_scopes_are_distinct() {
        let fixture = Fixture::new();
        fs::write(fixture.path.join("notes.txt"), "before\nkeep\n").expect("write file");
        let first_oid = fixture.commit("initial note");
        fs::write(fixture.path.join("notes.txt"), "staged version\nkeep\n").expect("stage version");
        fixture.git_ok(["add", "--", "notes.txt"]);
        fs::write(fixture.path.join("notes.txt"), "working version\nkeep\n")
            .expect("working version");

        let snapshot = inspect_repository(fixture.path_str()).expect("inspect staged repo");
        let change = snapshot
            .changes
            .iter()
            .find(|change| change.path == "notes.txt")
            .expect("status row");
        assert_eq!(change.index_status, "M");
        assert_eq!(change.worktree_status, "M");

        let staged = read_diff(fixture.path_str(), "notes.txt", true, None).expect("staged diff");
        let working =
            read_diff(fixture.path_str(), "notes.txt", false, None).expect("working diff");
        assert!(staged.text.contains("+staged version"));
        assert!(!staged.text.contains("+working version"));
        assert!(working.text.contains("+working version"));
        assert!(!working.text.contains("+staged version"));

        let historical = read_diff(fixture.path_str(), "notes.txt", false, Some(&first_oid))
            .expect("commit diff");
        assert!(historical.text.contains("before"));
        assert!(historical.text.contains("+before"));
        let files = list_commit_files(fixture.path_str(), &first_oid).expect("commit file list");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "notes.txt");
        assert_eq!(files[0].index_status, "A");
    }

    #[test]
    fn porcelain_rename_and_untracked_paths_are_parsed_without_quoting() {
        let fixture = Fixture::new();
        fs::write(fixture.path.join("old name.txt"), "content\n").expect("write source");
        fixture.commit("source");
        fixture.git_ok(["mv", "--", "old name.txt", "new name.txt"]);
        fs::write(fixture.path.join("untracked.txt"), "untracked\n").expect("write untracked");

        let snapshot = inspect_repository(fixture.path_str()).expect("inspect rename");
        let renamed = snapshot
            .changes
            .iter()
            .find(|change| change.path == "new name.txt")
            .expect("rename row");
        assert_eq!(renamed.original_path.as_deref(), Some("old name.txt"));
        assert_eq!(renamed.index_status, "R");
        assert!(snapshot
            .changes
            .iter()
            .any(|change| change.path == "untracked.txt" && change.worktree_status == "?"));
        let untracked_diff =
            read_diff(fixture.path_str(), "untracked.txt", false, None).expect("untracked diff");
        assert!(untracked_diff.text.contains("+untracked"));
    }

    #[test]
    fn unicode_paths_and_commit_subjects_round_trip() {
        let fixture = Fixture::new();
        let relative = Path::new("données").join("ıslak çay.txt");
        fs::create_dir_all(fixture.path.join("données")).expect("create unicode dir");
        fs::write(fixture.path.join(&relative), "başlangıç\n").expect("write unicode path");
        let oid = fixture.commit("Türkçe konu satırı");
        fs::write(fixture.path.join(&relative), "değişiklik\n").expect("modify unicode path");

        let snapshot = inspect_repository(fixture.path_str()).expect("inspect unicode repo");
        assert_eq!(snapshot.commits[0].subject, "Türkçe konu satırı");
        assert_eq!(snapshot.changes[0].path, "données/ıslak çay.txt");
        let diff = read_diff(fixture.path_str(), "données/ıslak çay.txt", false, None)
            .expect("unicode diff");
        assert!(diff.text.contains("değişiklik"));
        let files = list_commit_files(fixture.path_str(), &oid).expect("list unicode commit files");
        assert_eq!(files[0].path, "données/ıslak çay.txt");
    }

    #[test]
    fn linked_worktree_from_bare_root_is_inspectable_without_a_bare_head() {
        let fixture = Fixture::new();
        fixture.write("file.txt", "base\n");
        let oid = fixture.commit("base");
        let bare_path = fixture.path.join("bare.git");
        let linked_path = fixture.path.join("linked");
        let bare = bare_path.to_str().unwrap();
        let linked = linked_path.to_str().unwrap();
        fixture.git_ok(["clone", "--bare", "--no-hardlinks", ".", bare]);
        fixture.git_ok([
            "-C", bare, "worktree", "add", "-b", "linked-branch", linked, "main",
        ]);

        let snapshot = inspect_repository(linked).expect("inspect worktree with bare base");
        assert_eq!(snapshot.head.as_deref(), Some(oid.as_str()));
        assert_eq!(snapshot.branch, "linked-branch");
        assert_eq!(snapshot.worktrees.len(), 2);
        let bare_record = snapshot.worktrees.iter().find(|tree| tree.is_bare).unwrap();
        assert_eq!(
            Path::new(&bare_record.path).canonicalize().unwrap(),
            bare_path.canonicalize().unwrap()
        );
        assert!(bare_record.head.is_empty());
        assert!(bare_record.branch.is_empty());
        let linked_record = snapshot
            .worktrees
            .iter()
            .find(|tree| tree.branch == "linked-branch")
            .unwrap();
        assert_eq!(linked_record.head, oid);
        assert!(!linked_record.is_bare);
        assert_eq!(
            Path::new(&linked_record.path).canonicalize().unwrap(),
            linked_path.canonicalize().unwrap()
        );
    }

    #[test]
    fn worktree_parser_requires_a_head_for_ordinary_records() {
        for record in [
            &b"worktree /repo\0branch refs/heads/main\0\0"[..],
            &b"worktree /repo\0HEAD \0branch refs/heads/main\0\0"[..],
        ] {
            let error = parse_worktrees(record).expect_err("ordinary worktree needs a HEAD");
            assert_eq!(error.kind, GitErrorKind::InvalidOutput);
            assert!(error.message.contains("no HEAD"));
        }
        assert_eq!(
            parse_worktrees(b"bare\0\0").unwrap_err().kind,
            GitErrorKind::InvalidOutput,
            "bare records still need their path"
        );
    }

    #[test]
    fn linked_worktree_is_reported_and_selected_commit_revision_is_strict() {
        let fixture = Fixture::new();
        fs::write(fixture.path.join("file.txt"), "one\n").expect("write file");
        let oid = fixture.commit("one");
        let linked_path = fixture.path.with_file_name(format!(
            "{}-linked",
            fixture.path.file_name().unwrap().to_string_lossy()
        ));
        let linked = linked_path.to_string_lossy().into_owned();
        fixture.git_ok(["worktree", "add", "-b", "linked-branch", &linked]);
        let canonical_linked =
            fs::canonicalize(&linked_path).expect("canonicalize linked worktree");

        let snapshot =
            inspect_repository(fixture.path_str()).expect("inspect linked worktree repo");
        assert!(snapshot.worktrees.iter().any(|worktree| {
            worktree.branch == "linked-branch"
                && Path::new(&worktree.path).canonicalize().ok().as_ref() == Some(&canonical_linked)
        }));
        let linked_snapshot = inspect_repository(&linked).expect("inspect linked checkout");
        assert_eq!(
            Path::new(&linked_snapshot.repo_path)
                .canonicalize()
                .unwrap(),
            canonical_linked
        );
        assert_eq!(linked_snapshot.head.as_deref(), Some(oid.as_str()));

        let error = read_diff(
            fixture.path_str(),
            "file.txt",
            false,
            Some("HEAD;touch-pwned"),
        )
        .expect_err("reject revision expression");
        assert_eq!(error.kind, GitErrorKind::InvalidRevision);
        let error = read_diff(fixture.path_str(), "file.txt", false, Some(&"0".repeat(40)))
            .expect_err("reject stale oid");
        assert_eq!(error.kind, GitErrorKind::InvalidRevision);
        let _ = Command::new(&fixture.git)
            .args(["worktree", "remove", "--force", &linked])
            .current_dir(&fixture.path)
            .status();
        let _ = fs::remove_dir_all(linked_path);
    }

    #[test]
    fn malicious_diff_paths_are_literal_and_parent_traversal_is_rejected() {
        let fixture = Fixture::new();
        fs::write(fixture.path.join("--output"), "safe\n").expect("write option-like filename");
        fixture.commit("option-like filename");
        let diff =
            read_diff(fixture.path_str(), "--output", false, None).expect("literal filename diff");
        assert!(diff.text.is_empty());

        let error = read_diff(fixture.path_str(), "../../outside", false, None)
            .expect_err("reject traversal");
        assert_eq!(error.kind, GitErrorKind::InvalidDiffPath);
        let error = read_diff(fixture.path_str(), ":(glob)*", false, None)
            .expect("magic syntax is literal");
        assert!(error.text.is_empty());
    }

    #[test]
    fn unmerged_porcelain_records_are_reported_as_conflicts() {
        let fixture = Fixture::new();
        fs::write(fixture.path.join("conflict.txt"), "base\n").expect("write base");
        fixture.commit("base");
        fixture.git_ok(["checkout", "-b", "side"]);
        fs::write(fixture.path.join("conflict.txt"), "side version\n").expect("write side");
        fixture.commit("side change");
        fixture.git_ok(["checkout", "main"]);
        fs::write(fixture.path.join("conflict.txt"), "main version\n").expect("write main");
        fixture.commit("main change");

        let merge = Command::new(&fixture.git)
            .args(["merge", "side"])
            .current_dir(&fixture.path)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .expect("run conflicting merge");
        assert!(!merge.status.success(), "fixture merge should conflict");
        let snapshot = inspect_repository(fixture.path_str()).expect("inspect conflicted repo");
        let conflict = snapshot
            .changes
            .iter()
            .find(|change| change.path == "conflict.txt")
            .expect("conflict row");
        assert_eq!(conflict.index_status, "U");
        assert_eq!(conflict.worktree_status, "U");
    }

    #[test]
    fn binary_diff_metadata_is_distinct_from_paths_and_hunks() {
        let binary_patches: &[&[u8]] = &[
            b"diff --git a/data b/data\nBinary files a/data and b/data differ\n",
            b"diff --git a/data b/data\ndeleted file mode 100644\nBinary files a/data and /dev/null differ\n",
            b"diff --git a/old b/new\nrename from old\nrename to new\nBinary files a/old and b/new differ\n",
            b"diff --git a/data b/data\nGIT binary patch\nliteral 4\n",
            b"diff --git a/text b/text\n@@ -1 +1 @@\n-before\n+after\ndiff --git a/data b/data\nBinary files a/data and b/data differ\n",
        ];
        for patch in binary_patches {
            assert!(diff_is_binary(patch));
        }
        assert!(!diff_is_binary(b"diff --git a/GIT binary patch b/new\nrename from GIT binary patch\nrename to Binary files old and new differ\n"));
        assert!(!diff_is_binary(b"diff --git a/text b/text\n@@ -1 +1 @@\nGIT binary patch\nBinary files a/text and b/text differ\n"));
        assert!(!diff_is_binary(b"diff --git a/data b/data\nBinary files a/data and b/da"));
    }

    #[test]
    fn text_diff_binary_markers_are_not_metadata() {
        let fixture = Fixture::new();
        let path = "Binary files notes.txt";
        fixture.write(path, "GIT binary patch\nbefore\nBinary files old and new differ\n");
        let initial = fixture.commit("text containing binary metadata words");
        fixture.write(path, "GIT binary patch\nafter\nBinary files old and new differ\n");
        let working = read_diff(fixture.path_str(), path, false, None).expect("working text diff");
        fixture.git_ok(["add", "--", path]);
        let staged = read_diff(fixture.path_str(), path, true, None).expect("staged text diff");
        let historical = read_diff(fixture.path_str(), path, false, Some(&initial))
            .expect("historical text diff");
        fixture.write("new.txt", "GIT binary patch\nBinary files old and new differ\n");
        let untracked = read_diff(fixture.path_str(), "new.txt", false, None)
            .expect("untracked text diff");
        fs::remove_file(fixture.path.join(path)).expect("delete text file");
        let deleted = read_diff(fixture.path_str(), path, false, None).expect("deleted text diff");
        for diff in [working, staged, historical, untracked, deleted] {
            assert!(diff.text.contains("@@"), "expected a text hunk: {}", diff.text);
            assert!(!diff.binary, "text was classified as binary: {}", diff.text);
            assert!(!diff.truncated);
        }
    }

    #[test]
    fn snapshot_json_uses_camel_case_and_diff_binary_flag() {
        let fixture = Fixture::new();
        fs::write(fixture.path.join("bytes.bin"), [0, 1, 2, 3]).expect("write binary file");
        fixture.commit("binary file");
        fs::write(fixture.path.join("bytes.bin"), [0, 1, 2, 4]).expect("modify binary file");
        let diff = read_diff(fixture.path_str(), "bytes.bin", false, None).expect("binary diff");
        assert!(diff.binary);
        let snapshot = inspect_repository(fixture.path_str()).expect("inspect binary repo");
        let value = serde_json::to_value(snapshot).expect("serialize snapshot");
        assert!(value.get("repoPath").is_some());
        assert!(value.get("repoName").is_some());
        assert!(value.get("mutationToken").is_some());
        assert!(value.get("mergeInProgress").is_some());
        assert!(value["changes"][0].get("originalPath").is_some());
    }

    fn paths(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn stage_and_unstage_unborn_index_preserve_worktree_and_literal_names() {
        let _test_guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("--all", "first staged version\n");
        fixture.write("semi;touch-owned.txt", "literal semicolon\n");
        fixture.write("other.txt", "not selected\n");
        let initial = fixture.snapshot();
        assert_eq!(initial.mutation_token, fixture.snapshot().mutation_token);
        let staged = stage_paths(
            fixture.path_str(),
            &paths(&["--all", "semi;touch-owned.txt"]),
            &initial.mutation_token,
        )
        .unwrap();
        assert!(staged.commit_oid.is_none());
        assert!(!staged.commit_tree_changed);
        assert_ne!(staged.snapshot.mutation_token, initial.mutation_token);
        assert_eq!(
            fixture.git_text(&["ls-files"]),
            "--all\nsemi;touch-owned.txt\n"
        );
        fixture.write("--all", "unstaged later version\n");
        let partial = fixture.snapshot();
        assert_eq!(
            partial.mutation_token, staged.snapshot.mutation_token,
            "worktree edits do not alter the HEAD/index token"
        );
        let unstaged = unstage_paths(
            fixture.path_str(),
            &paths(&["--all"]),
            &partial.mutation_token,
        )
        .unwrap();
        assert!(unstaged.snapshot.is_unborn);
        assert_eq!(fixture.git_text(&["ls-files"]), "semi;touch-owned.txt\n");
        assert_eq!(
            fs::read_to_string(fixture.path.join("--all")).unwrap(),
            "unstaged later version\n"
        );
        assert!(!fixture.path.join("touch-owned.txt").exists());
        let json = serde_json::to_value(unstaged).unwrap();
        assert!(json.get("commitOid").is_some());
        assert_eq!(json["commitTreeChanged"], false);
        assert!(json["snapshot"].get("mutationToken").is_some());
    }

    #[test]
    fn stage_and_unstage_handle_partial_files_and_deletions() {
        let _test_guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("partial.txt", "base\n");
        fixture.write("deleted.txt", "delete later\n");
        fixture.commit("base");
        fixture.write("partial.txt", "index version\n");
        let staged = stage_paths(
            fixture.path_str(),
            &paths(&["partial.txt"]),
            &fixture.snapshot().mutation_token,
        )
        .unwrap();
        fixture.write("partial.txt", "working version\n");
        let unstaged = unstage_paths(
            fixture.path_str(),
            &paths(&["partial.txt"]),
            &staged.snapshot.mutation_token,
        )
        .unwrap();
        assert_eq!(fixture.git_text(&["show", ":partial.txt"]), "base\n");
        assert_eq!(
            fs::read_to_string(fixture.path.join("partial.txt")).unwrap(),
            "working version\n"
        );
        let staged = stage_paths(
            fixture.path_str(),
            &paths(&["partial.txt"]),
            &unstaged.snapshot.mutation_token,
        )
        .unwrap();
        assert_eq!(
            fixture.git_text(&["show", ":partial.txt"]),
            "working version\n"
        );
        fs::remove_file(fixture.path.join("deleted.txt")).unwrap();
        let deleted = stage_paths(
            fixture.path_str(),
            &paths(&["deleted.txt"]),
            &staged.snapshot.mutation_token,
        )
        .unwrap();
        assert!(deleted
            .snapshot
            .changes
            .iter()
            .any(|change| change.path == "deleted.txt" && change.index_status == "D"));
        let restored_index = unstage_paths(
            fixture.path_str(),
            &paths(&["deleted.txt"]),
            &deleted.snapshot.mutation_token,
        )
        .unwrap();
        assert!(restored_index
            .snapshot
            .changes
            .iter()
            .any(|change| change.path == "deleted.txt" && change.worktree_status == "D"));
        assert!(
            !fixture.path.join("deleted.txt").exists(),
            "unstage must not restore worktree content"
        );
        assert_eq!(
            fixture.git_text(&["show", ":deleted.txt"]),
            "delete later\n"
        );
    }

    #[test]
    fn unstage_expands_a_staged_rename_and_stage_preserves_its_pair() {
        let _test_guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        let original = "one\ntwo\nthree\nfour\nfive\nsix\n";
        fixture.write("old name.txt", original);
        fixture.commit("before rename");
        fixture.git_ok(["mv", "--", "old name.txt", "new name.txt"]);
        let renamed = fixture.snapshot();
        assert_eq!(
            renamed.changes[0].original_path.as_deref(),
            Some("old name.txt")
        );
        let unstaged = unstage_paths(
            fixture.path_str(),
            &paths(&["new name.txt"]),
            &renamed.mutation_token,
        )
        .unwrap();
        assert!(unstaged
            .snapshot
            .changes
            .iter()
            .all(|change| change.index_status == "."));
        assert_eq!(fixture.git_text(&["ls-files"]), "old name.txt\n");
        assert!(!fixture.path.join("old name.txt").exists());
        assert_eq!(
            fs::read_to_string(fixture.path.join("new name.txt")).unwrap(),
            original
        );
        let staged = stage_paths(
            fixture.path_str(),
            &paths(&["old name.txt", "new name.txt"]),
            &unstaged.snapshot.mutation_token,
        )
        .unwrap();
        assert_eq!(staged.snapshot.changes[0].index_status, "R");
        fixture.write("new name.txt", "one\ntwo\nthree\nfour\nfive\nsix\nseven\n");
        let restaged = stage_paths(
            fixture.path_str(),
            &paths(&["new name.txt"]),
            &staged.snapshot.mutation_token,
        )
        .unwrap();
        assert_eq!(restaged.snapshot.changes[0].index_status, "R");
        assert_eq!(restaged.snapshot.changes[0].worktree_status, ".");
        assert_eq!(fixture.git_text(&["ls-files"]), "new name.txt\n");
    }

    #[test]
    fn mutation_tokens_reject_external_index_branch_repository_and_merge_state_changes() {
        let _test_guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("file.txt", "base\n");
        let oid = fixture.commit("base");
        fixture.write("file.txt", "next\n");
        let before = fixture.snapshot();
        fixture.git_ok(["add", "--", "file.txt"]);
        let error = unstage_paths(
            fixture.path_str(),
            &paths(&["file.txt"]),
            &before.mutation_token,
        )
        .unwrap_err();
        assert_eq!(error.kind, GitErrorKind::StaleState);
        assert_eq!(fixture.git_text(&["show", ":file.txt"]), "next\n");
        let staged = fixture.snapshot();
        fixture.git_ok(["checkout", "-b", "same-head"]);
        let branch = fixture.snapshot();
        assert_eq!(branch.head, staged.head);
        assert_ne!(branch.mutation_token, staged.mutation_token);
        assert_eq!(
            commit_index(fixture.path_str(), "should fail", &staged.mutation_token)
                .unwrap_err()
                .kind,
            GitErrorKind::StaleState
        );
        fs::write(fixture.path.join(".git/MERGE_HEAD"), format!("{oid}\n")).unwrap();
        assert_eq!(
            commit_index(fixture.path_str(), "should fail", &branch.mutation_token)
                .unwrap_err()
                .kind,
            GitErrorKind::StaleState
        );
        fs::remove_file(fixture.path.join(".git/MERGE_HEAD")).unwrap();
        let another = Fixture::new();
        another.write("file.txt", "next\n");
        assert_eq!(
            stage_paths(
                another.path_str(),
                &paths(&["file.txt"]),
                &branch.mutation_token
            )
            .unwrap_err()
            .kind,
            GitErrorKind::StaleState
        );
    }

    #[test]
    fn commit_uses_only_staged_content_and_treats_message_as_data() {
        let _test_guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("partial.txt", "base\n");
        fixture.write("other.txt", "other base\n");
        let before_oid = fixture.commit("base");
        fixture.write("partial.txt", "index version\n");
        let staged = stage_paths(
            fixture.path_str(),
            &paths(&["partial.txt"]),
            &fixture.snapshot().mutation_token,
        )
        .unwrap();
        fixture.write("partial.txt", "working version\n");
        fixture.write("other.txt", "not staged\n");
        fixture.write("untracked.txt", "not selected\n");
        let message = "--allow-empty\n\n$(touch owned); `touch owned` & echo safe\nTürkçe gövde\n";
        let outcome =
            commit_index(fixture.path_str(), message, &staged.snapshot.mutation_token).unwrap();
        assert_eq!(outcome.commit_oid, outcome.snapshot.head);
        assert!(!outcome.commit_tree_changed);
        assert_ne!(outcome.commit_oid.as_deref(), Some(before_oid.as_str()));
        assert_eq!(
            fixture.git_text(&["show", "HEAD:partial.txt"]),
            "index version\n"
        );
        assert_eq!(
            fixture.git_text(&["show", "HEAD:other.txt"]),
            "other base\n"
        );
        assert_eq!(
            fs::read_to_string(fixture.path.join("partial.txt")).unwrap(),
            "working version\n"
        );
        assert_eq!(
            fixture.git_text(&["log", "-1", "--format=%B"]).trim_end(),
            message.trim_end()
        );
        assert!(!fixture.path.join("owned").exists());
        assert!(outcome
            .snapshot
            .changes
            .iter()
            .all(|change| change.index_status == "."));
        assert_eq!(outcome.snapshot.changes.len(), 3);
    }

    #[test]
    fn unborn_commit_ignores_intent_to_add_and_hook_diff_order_changes() {
        let _test_guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("first.txt", "first staged file\n");
        fixture.write("last.txt", "last staged file\n");
        fixture.write("intent.txt", "not staged\n");
        fixture.git_ok(["add", "--", "first.txt", "last.txt"]);
        fixture.git_ok(["add", "--intent-to-add", "--", "intent.txt"]);
        let before = fixture.snapshot();
        let context = repository_context(fixture.path_str()).unwrap();
        let index_before = fs::read(fixture.path.join(".git/index")).unwrap();
        let prepared = read_tree_delta(&context, None, None).unwrap();
        assert_eq!(prepared.len(), 2, "intent-to-add is not committed content");
        assert_eq!(
            fs::read(fixture.path.join(".git/index")).unwrap(),
            index_before,
            "capturing the prepared tree must not update index metadata"
        );
        assert_eq!(read_mutation_token(&context).unwrap(), before.mutation_token);
        let hook = fixture.path.join(".git/test-hooks/pre-commit");
        fs::write(
            &hook,
            "#!/bin/sh\nprintf 'last.txt\\nfirst.txt\\n' > .git/diff-order\ngit config diff.orderFile .git/diff-order\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let outcome = commit_index(fixture.path_str(), "initial", &before.mutation_token).unwrap();
        assert!(!outcome.commit_tree_changed);
        assert_eq!(outcome.commit_oid, outcome.snapshot.head);
        assert_eq!(
            fixture.git_text(&["ls-tree", "--name-only", "HEAD"]),
            "first.txt\nlast.txt\n"
        );
        assert_eq!(fixture.git_text(&["show", "HEAD:first.txt"]), "first staged file\n");
        assert_eq!(fixture.git_text(&["show", "HEAD:last.txt"]), "last staged file\n");
        assert_eq!(fixture.git_text(&["rev-list", "--count", "HEAD"]).trim(), "1");
        assert_eq!(serde_json::to_value(&outcome).unwrap()["commitTreeChanged"], false);
        assert!(outcome.snapshot.changes.iter().any(|change| {
            change.path == "intent.txt" && change.index_status == "."
        }));
    }

    #[test]
    fn raw_tree_delta_keeps_complete_records_and_rejects_truncation() {
        let zeros = "0".repeat(40);
        let first = format!(":000000 100644 {zeros} {} A\0first.txt\0", "1".repeat(40));
        let last = format!(":000000 100755 {zeros} {} A\0last\nfile.txt\0", "2".repeat(40));
        let ordered = parse_tree_delta(format!("{first}{last}").as_bytes()).unwrap();
        assert_eq!(
            ordered,
            parse_tree_delta(format!("{last}{first}").as_bytes()).unwrap(),
            "raw diff ordering is not part of tree identity"
        );
        assert_eq!(ordered[1].0, b"last\nfile.txt");
        for invalid in [
            first.trim_end_matches('\0').to_owned(),
            format!("{first}:000000 100644 {zeros} {} A\0", "3".repeat(40)),
            first.replace("100644", "badmode"),
            first.replace(&"1".repeat(40), "1234"),
        ] {
            assert_eq!(parse_tree_delta(invalid.as_bytes()).unwrap_err().kind, GitErrorKind::InvalidOutput);
        }
        assert_ne!(
            ordered,
            parse_tree_delta(format!("{first}{}", last.replace("100755", "100644")).as_bytes()).unwrap(),
            "file modes are part of tree identity"
        );
    }

    #[test]
    fn writes_reject_empty_messages_empty_index_and_non_file_pathspecs() {
        let _test_guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("visible.txt", "visible\n");
        fs::create_dir(fixture.path.join("folder")).unwrap();
        fixture.write("folder/nested.txt", "nested\n");
        let snapshot = fixture.snapshot();
        for message in ["", " \n\t", "text\0suffix"] {
            assert_eq!(
                commit_index(fixture.path_str(), message, &snapshot.mutation_token)
                    .unwrap_err()
                    .kind,
                GitErrorKind::InvalidCommitMessage
            );
        }
        assert_eq!(
            commit_index(
                fixture.path_str(),
                "no staged changes",
                &snapshot.mutation_token
            )
            .unwrap_err()
            .kind,
            GitErrorKind::NothingStaged
        );
        for path in [
            "folder",
            "folder/",
            "../outside",
            "./visible.txt",
            ".git/config",
            "folder/../../outside",
            ":(glob)*",
            "visible.txt\0other",
            "not-present.txt",
        ] {
            assert_eq!(
                stage_paths(
                    fixture.path_str(),
                    &paths(&[path]),
                    &snapshot.mutation_token
                )
                .unwrap_err()
                .kind,
                GitErrorKind::InvalidMutationPath,
                "{path}"
            );
        }
        assert_eq!(
            stage_paths(fixture.path_str(), &[], &snapshot.mutation_token)
                .unwrap_err()
                .kind,
            GitErrorKind::InvalidMutationPath
        );
        assert!(fixture.git_text(&["ls-files"]).is_empty());
        let staged = stage_paths(
            fixture.path_str(),
            &paths(&["folder/nested.txt"]),
            &snapshot.mutation_token,
        )
        .unwrap();
        assert_eq!(fixture.git_text(&["ls-files"]), "folder/nested.txt\n");
        assert_eq!(
            stage_paths(
                fixture.path_str(),
                &paths(&["folder/nested.txt"]),
                &staged.snapshot.mutation_token
            )
            .unwrap_err()
            .kind,
            GitErrorKind::InvalidMutationPath
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn pathspec_magic_is_literal_when_it_is_a_real_status_file() {
        let _test_guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write(":(glob)*", "literal\n");
        fixture.write("other.txt", "not staged\n");
        stage_paths(
            fixture.path_str(),
            &paths(&[":(glob)*"]),
            &fixture.snapshot().mutation_token,
        )
        .unwrap();
        assert_eq!(fixture.git_text(&["ls-files"]), ":(glob)*\n");
    }

    #[test]
    fn app_and_git_index_locks_report_busy_without_writing() {
        let _test_guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("file.txt", "content\n");
        let snapshot = fixture.snapshot();
        let app_lock = MUTATION_LOCK.lock().unwrap();
        assert_eq!(
            stage_paths(
                fixture.path_str(),
                &paths(&["file.txt"]),
                &snapshot.mutation_token
            )
            .unwrap_err()
            .kind,
            GitErrorKind::Busy
        );
        drop(app_lock);
        fs::write(fixture.path.join(".git/index.lock"), "fixture lock").unwrap();
        assert_eq!(
            stage_paths(
                fixture.path_str(),
                &paths(&["file.txt"]),
                &snapshot.mutation_token
            )
            .unwrap_err()
            .kind,
            GitErrorKind::Busy
        );
        assert_eq!(fixture.snapshot().mutation_token, snapshot.mutation_token);
        assert!(fixture.git_text(&["ls-files"]).is_empty());
        fs::remove_file(fixture.path.join(".git/index.lock")).unwrap();
    }

    #[test]
    fn successful_commit_hook_changes_tree_without_retry_or_rollback() {
        let _test_guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("file.txt", "base\n");
        let old_head = fixture.commit("base");
        fixture.write("file.txt", "staged\n");
        let staged = stage_paths(
            fixture.path_str(),
            &paths(&["file.txt"]),
            &fixture.snapshot().mutation_token,
        )
        .unwrap();
        fixture.write("file.txt", "working copy\n");
        let hook = fixture.path.join(".git/test-hooks/pre-commit");
        fs::write(
            &hook,
            "#!/bin/sh\nprintf 'hook addition\\n' > hook.txt\ngit add -- hook.txt\nprintf x >> .git/hook-runs\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let outcome = commit_index(
            fixture.path_str(),
            "hook adds content",
            &staged.snapshot.mutation_token,
        )
        .unwrap();
        assert!(outcome.commit_tree_changed);
        assert_eq!(outcome.commit_oid, outcome.snapshot.head);
        assert_eq!(outcome.commit_oid.as_deref(), Some(fixture.git_text(&["rev-parse", "HEAD"]).trim()));
        assert_eq!(fixture.git_text(&["rev-parse", "HEAD^"]).trim(), old_head);
        assert_eq!(fixture.git_text(&["show", "HEAD:file.txt"]), "staged\n");
        assert_eq!(fixture.git_text(&["show", "HEAD:hook.txt"]), "hook addition\n");
        assert_eq!(fs::read_to_string(fixture.path.join("file.txt")).unwrap(), "working copy\n");
        assert_eq!(fs::read_to_string(fixture.path.join(".git/hook-runs")).unwrap(), "x");
        assert_eq!(fixture.git_text(&["rev-list", "--count", "HEAD"]).trim(), "2");
        assert_eq!(serde_json::to_value(&outcome).unwrap()["commitTreeChanged"], true);
    }

    #[test]
    fn failed_commit_hook_can_change_index_and_is_reported_without_retry_or_rollback() {
        let _test_guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("file.txt", "base\n");
        let old_head = fixture.commit("base");
        fixture.write("file.txt", "staged\n");
        let staged = stage_paths(
            fixture.path_str(),
            &paths(&["file.txt"]),
            &fixture.snapshot().mutation_token,
        )
        .unwrap();
        let hook = fixture.path.join(".git/test-hooks/pre-commit");
        fs::write(&hook, "#!/bin/sh\nprintf 'hook changed index\\n' > hook.txt\ngit add -- hook.txt\nprintf 'fixture hook rejected\\n' >&2\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let error = commit_index(
            fixture.path_str(),
            "rejected",
            &staged.snapshot.mutation_token,
        )
        .unwrap_err();
        assert_eq!(error.kind, GitErrorKind::GitCommandFailed);
        assert!(error.message.contains("fixture hook rejected"));
        assert!(error.message.contains("state may have changed"));
        let after = fixture.snapshot();
        assert_eq!(after.head.as_deref(), Some(old_head.as_str()));
        assert_ne!(after.mutation_token, staged.snapshot.mutation_token);
        assert!(after
            .changes
            .iter()
            .any(|change| change.path == "hook.txt" && change.index_status == "A"));
    }

    #[test]
    fn successful_commit_with_post_commit_head_move_reports_uncertain_result() {
        let _test_guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("file.txt", "base\n");
        let old_head = fixture.commit("base");
        fixture.write("file.txt", "staged\n");
        let staged = stage_paths(
            fixture.path_str(),
            &paths(&["file.txt"]),
            &fixture.snapshot().mutation_token,
        )
        .unwrap();
        let hook = fixture.path.join(".git/test-hooks/post-commit");
        fs::write(&hook, "#!/bin/sh\ngit reset --soft HEAD^\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let error = commit_index(
            fixture.path_str(),
            "hook moves HEAD",
            &staged.snapshot.mutation_token,
        )
        .unwrap_err();
        assert_eq!(error.kind, GitErrorKind::StaleState);
        assert!(error.message.contains("successful commit"));
        assert!(error.message.contains("do not repeat"));
        assert_eq!(fixture.snapshot().head.as_deref(), Some(old_head.as_str()));
        assert!(
            fixture
                .git_text(&["reflog", "--format=%gs"])
                .contains("commit: hook moves HEAD"),
            "the created commit remains in reflog; error must not claim a rollback"
        );
    }

    #[cfg(windows)]
    #[test]
    fn timed_out_commit_hook_keeps_its_index_changes_and_reports_uncertainty() {
        let _test_guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("file.txt", "staged\n");
        fixture.git_ok(["add", "--", "file.txt"]);
        fs::write(
            fixture.path.join(".git/test-hooks/pre-commit"),
            "#!/bin/sh\nprintf 'hook changed index\\n' > hook.txt\ngit add -- hook.txt\nprintf x >> .git/hook-runs\nsleep 60\n",
        ).unwrap();
        let mut command = Command::new(&fixture.git);
        command
            .args(["commit", "-m", "timeout fixture"])
            .current_dir(&fixture.path);
        let result = git_process::run(
            &mut command,
            MAX_STATUS_BYTES,
            false,
            None,
            Duration::from_millis(1200),
        );
        let error = ensure_mutation_success(result, "commit the index").unwrap_err();
        assert_eq!(error.kind, GitErrorKind::TimedOut);
        assert!(error.message.contains("state may have changed"));
        assert!(error.message.contains("do not automatically retry"));
        assert_eq!(fs::read(fixture.path.join(".git/hook-runs")).unwrap(), b"x");
        assert_eq!(
            fixture
                .git_text(&["ls-files", "--error-unmatch", "hook.txt"])
                .trim(),
            "hook.txt"
        );
        assert!(fixture.snapshot().head.is_none());
        // No retry/rollback or index.lock deletion: the disposable fixture's
        // actual post-timeout index remains the source of truth.
    }

    #[test]
    fn commit_honors_signing_failure_and_blocks_rebase_state() {
        let _test_guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("file.txt", "staged\n");
        let staged = stage_paths(
            fixture.path_str(),
            &paths(&["file.txt"]),
            &fixture.snapshot().mutation_token,
        )
        .unwrap();
        fixture.git_ok(["config", "commit.gpgsign", "true"]);
        fixture.git_ok(["config", "gpg.format", "openpgp"]);
        fixture.git_ok(["config", "user.signingkey", "enogram-test-missing-key"]);
        let missing_signer = fixture.path.join("missing-gpg-executable");
        fixture.git_ok([
            OsString::from("config"),
            OsString::from("gpg.program"),
            missing_signer.into_os_string(),
        ]);
        let error = commit_index(
            fixture.path_str(),
            "must use signing",
            &staged.snapshot.mutation_token,
        )
        .unwrap_err();
        assert_eq!(error.kind, GitErrorKind::GitCommandFailed);
        assert!(fixture.snapshot().is_unborn);
        fs::create_dir(fixture.path.join(".git/rebase-merge")).unwrap();
        let token = read_mutation_token(&repository_context(fixture.path_str()).unwrap()).unwrap();
        assert_eq!(
            commit_index(fixture.path_str(), "no rebase continuation", &token)
                .unwrap_err()
                .kind,
            GitErrorKind::Unsupported
        );
    }

    #[test]
    fn conflicts_block_commit_and_selected_paths_but_not_unaffected_staging() {
        let _test_guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("conflict.txt", "base\n");
        fixture.commit("base");
        fixture.git_ok(["checkout", "-b", "side"]);
        fixture.write("conflict.txt", "side\n");
        fixture.commit("side");
        fixture.git_ok(["checkout", "main"]);
        fixture.write("conflict.txt", "main\n");
        fixture.commit("main");
        let merge = Command::new(&fixture.git)
            .args(["merge", "side"])
            .current_dir(&fixture.path)
            .output()
            .unwrap();
        assert!(!merge.status.success());
        fixture.write("unaffected.txt", "safe to stage\n");
        let snapshot = fixture.snapshot();
        assert_eq!(
            stage_paths(
                fixture.path_str(),
                &paths(&["conflict.txt"]),
                &snapshot.mutation_token
            )
            .unwrap_err()
            .kind,
            GitErrorKind::Conflicts
        );
        assert_eq!(
            unstage_paths(
                fixture.path_str(),
                &paths(&["conflict.txt"]),
                &snapshot.mutation_token
            )
            .unwrap_err()
            .kind,
            GitErrorKind::Conflicts
        );
        let staged = stage_paths(
            fixture.path_str(),
            &paths(&["unaffected.txt"]),
            &snapshot.mutation_token,
        )
        .unwrap();
        assert_eq!(
            commit_index(
                fixture.path_str(),
                "unresolved",
                &staged.snapshot.mutation_token
            )
            .unwrap_err()
            .kind,
            GitErrorKind::Conflicts
        );
        for (index, worktree) in [("D", "D"), ("A", "A"), ("A", "U"), ("U", "D"), ("U", "U")] {
            assert!(is_conflict(&Change {
                path: "file".to_owned(),
                original_path: None,
                index_status: index.to_owned(),
                worktree_status: worktree.to_owned()
            }));
        }
    }

    #[test]
    fn merge_commit_lists_first_parent_changes_and_history_is_topological() {
        let _test_guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("base.txt", "base\n");
        fixture.commit("base");
        fixture.git_ok(["checkout", "-b", "side"]);
        fixture.write("side.txt", "side addition\n");
        let side = fixture.commit("side");
        fixture.git_ok(["checkout", "main"]);
        fixture.write("main.txt", "main addition\n");
        let main = fixture.commit("main");
        let branched = fixture.snapshot();
        assert!(
            branched.commits.iter().any(|commit| commit.oid == side),
            "history includes the other branch"
        );
        fixture.git_ok(["merge", "--no-ff", "--no-commit", "side"]);
        let merged = commit_index(
            fixture.path_str(),
            "merge side",
            &fixture.snapshot().mutation_token,
        )
        .unwrap();
        let oid = merged.commit_oid.unwrap();
        let row = merged
            .snapshot
            .commits
            .iter()
            .find(|commit| commit.oid == oid)
            .unwrap();
        assert_eq!(row.parents, vec![main, side]);
        for (index, commit) in merged.snapshot.commits.iter().enumerate() {
            for parent in &commit.parents {
                if let Some(parent_index) = merged
                    .snapshot
                    .commits
                    .iter()
                    .position(|row| &row.oid == parent)
                {
                    assert!(
                        parent_index > index,
                        "topological history must show children before parents"
                    );
                }
            }
        }
        let files = list_commit_files(fixture.path_str(), &oid).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "side.txt");
        let side_diff = read_diff(fixture.path_str(), "side.txt", false, Some(&oid)).unwrap();
        assert!(side_diff.text.contains("+side addition"));
        assert!(read_diff(fixture.path_str(), "main.txt", false, Some(&oid))
            .unwrap()
            .text
            .is_empty());
        assert!(!fixture.path.join(".git/MERGE_HEAD").exists());
    }

    #[test]
    fn tree_identical_pending_merge_can_be_committed_without_staged_file_changes() {
        let _test_guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("file.txt", "base\n");
        let main = fixture.commit("base");
        fixture.git_ok(["checkout", "-b", "side"]);
        fixture.write("file.txt", "temporary side change\n");
        fixture.commit("side change");
        fixture.write("file.txt", "base\n");
        let side = fixture.commit("revert side change");
        fixture.git_ok(["checkout", "main"]);
        assert!(!fixture.snapshot().merge_in_progress);
        fixture.git_ok(["merge", "--no-ff", "--no-commit", "side"]);
        let pending = fixture.snapshot();
        assert!(pending.merge_in_progress);
        assert!(
            pending.changes.is_empty(),
            "a merge can have no staged tree difference"
        );
        let result = commit_index(
            fixture.path_str(),
            "merge reverted side",
            &pending.mutation_token,
        )
        .unwrap();
        assert!(!result.snapshot.merge_in_progress);
        let oid = result.commit_oid.unwrap();
        let commit = result
            .snapshot
            .commits
            .iter()
            .find(|row| row.oid == oid)
            .unwrap();
        assert_eq!(commit.parents, vec![main, side]);
        assert!(list_commit_files(fixture.path_str(), &oid)
            .unwrap()
            .is_empty());
        assert!(read_diff(fixture.path_str(), "file.txt", false, Some(&oid))
            .unwrap()
            .text
            .is_empty());
    }

    #[test]
    fn history_includes_detached_head_and_refs_while_current_branch_is_unborn() {
        let fixture = Fixture::new();
        fixture.write("base.txt", "base\n");
        let base = fixture.commit("base");
        fixture.git_ok(["checkout", "--detach"]);
        fixture.write("detached.txt", "detached\n");
        let detached = fixture.commit("detached");
        fixture.git_ok(["config", "log.showSignature", "true"]);
        let snapshot = fixture.snapshot();
        assert_eq!(snapshot.head.as_deref(), Some(detached.as_str()));
        assert!(snapshot.commits.iter().any(|commit| commit.oid == detached));
        assert!(snapshot.commits.iter().any(|commit| commit.oid == base));
        fixture.git_ok(["checkout", "--orphan", "unborn"]);
        let unborn = fixture.snapshot();
        assert!(unborn.is_unborn);
        assert!(unborn.commits.iter().any(|commit| commit.oid == base));
    }
}
