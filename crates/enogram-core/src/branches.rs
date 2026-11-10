use super::*;

/// A local branch and its locally known tracking/worktree state. Tracking
/// counts compare local refs; reading this list never fetches from a remote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Branch {
    pub name: String,
    pub head: String,
    pub current: bool,
    pub upstream: Option<String>,
    pub upstream_remote: Option<String>,
    pub upstream_branch: Option<String>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub worktree_path: Option<String>,
}

pub(super) fn read_branches(context: &RepositoryContext) -> Result<Vec<Branch>, GitError> {
    let output = run_git(
        &context.git,
        &context.root,
        [
            "--no-pager",
            "for-each-ref",
            "--sort=refname",
            "--format=%(refname)%00%(objectname)%00%(HEAD)%00%(upstream)%00%(upstream:remotename)%00%(upstream:remoteref)%00%(upstream:track,nobracket)%00%(worktreepath)%00",
            "refs/heads/",
        ],
        MAX_STATUS_BYTES,
        false,
    )?;
    ensure_success(&output, "list local branches")?;
    parse_branches(&output.stdout)
}

fn parse_branches(mut bytes: &[u8]) -> Result<Vec<Branch>, GitError> {
    let mut branches = Vec::new();
    while !bytes.is_empty() {
        let mut fields = Vec::with_capacity(8);
        for _ in 0..8 {
            let end = bytes.iter().position(|byte| *byte == 0).ok_or_else(|| {
                invalid_branch_output("Local branch record is missing a NUL-delimited field.")
            })?;
            fields.push(utf8(&bytes[..end], "local branch field")?);
            bytes = &bytes[end + 1..];
        }
        // for-each-ref appends one newline to each format result. Paths may
        // themselves contain newlines, so only consume this after all fields.
        bytes = bytes
            .strip_prefix(b"\n")
            .or_else(|| bytes.strip_prefix(b"\r\n"))
            .ok_or_else(|| invalid_branch_output("Local branch record has no terminator."))?;
        let name = fields[0]
            .strip_prefix("refs/heads/")
            .filter(|name| !name.is_empty())
            .ok_or_else(|| invalid_branch_output("Git returned a non-local branch ref."))?;
        if !is_full_oid(fields[1]) || !matches!(fields[2], "*" | " ") {
            return Err(invalid_branch_output(
                "Git returned invalid local branch state.",
            ));
        }
        let upstream = optional_branch_field(fields[3]);
        let (ahead, behind) = parse_tracking_counts(fields[6], upstream.is_some())?;
        branches.push(Branch {
            name: name.to_owned(),
            head: fields[1].to_owned(),
            current: fields[2] == "*",
            upstream,
            upstream_remote: optional_branch_field(fields[4]),
            upstream_branch: optional_branch_field(fields[5]),
            ahead,
            behind,
            worktree_path: optional_branch_field(fields[7]),
        });
    }
    Ok(branches)
}

fn optional_branch_field(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn parse_tracking_counts(
    tracking: &str,
    has_upstream: bool,
) -> Result<(Option<u32>, Option<u32>), GitError> {
    if !has_upstream || tracking == "gone" {
        return Ok((None, None));
    }
    let mut ahead = None;
    let mut behind = None;
    if !tracking.is_empty() {
        // for-each-ref uses untranslated plumbing messages for these atoms.
        for part in tracking.split(", ") {
            let (slot, number) = if let Some(number) = part.strip_prefix("ahead ") {
                (&mut ahead, number)
            } else if let Some(number) = part.strip_prefix("behind ") {
                (&mut behind, number)
            } else {
                return Err(invalid_branch_output(
                    "Git returned an invalid upstream count.",
                ));
            };
            if slot.is_some()
                || number.is_empty()
                || !number.bytes().all(|byte| byte.is_ascii_digit())
            {
                return Err(invalid_branch_output(
                    "Git returned an invalid upstream count.",
                ));
            }
            *slot = Some(number.parse::<u32>().map_err(|_| {
                invalid_branch_output("Git upstream count exceeds the supported range.")
            })?);
        }
    }
    Ok((Some(ahead.unwrap_or(0)), Some(behind.unwrap_or(0))))
}

fn invalid_branch_output(message: &str) -> GitError {
    GitError::new(GitErrorKind::InvalidOutput, message)
}

fn is_full_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_branch_name(context: &RepositoryContext, name: &str) -> Result<(), GitError> {
    let invalid = || {
        GitError::new(
            GitErrorKind::InvalidBranchName,
            "Use a valid literal local branch name, without options or revision expressions.",
        )
    };
    // --branch expands @{-n} inside repositories. Reject that syntax rather
    // than letting Git silently select a name different from the user's text.
    if name.is_empty()
        || name.contains('\0')
        || name.starts_with('-')
        || name.contains("@{")
        || matches!(name, "@" | "HEAD")
    {
        return Err(invalid());
    }
    let output = run_git(
        &context.git,
        &context.root,
        ["check-ref-format", "--branch", name],
        64 * 1024,
        false,
    )?;
    if !output.status.success()
        || output_text(&output.stdout, "validated branch name")?.trim_end_matches(['\r', '\n'])
            != name
    {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn ensure_no_active_operation(context: &RepositoryContext) -> Result<(), GitError> {
    if [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
        "sequencer",
    ]
    .iter()
    .any(|name| context.git_dir.join(name).exists())
    {
        return Err(GitError::new(
            GitErrorKind::Unsupported,
            "Finish the active merge, rebase, cherry-pick, revert, or am operation in Git before changing branches or pulling.",
        ));
    }
    Ok(())
}

fn ensure_no_branch_conflicts(changes: &[Change]) -> Result<(), GitError> {
    if changes.iter().any(is_conflict) {
        return Err(GitError::new(
            GitErrorKind::Conflicts,
            "Resolve all unmerged files before creating or switching branches.",
        ));
    }
    Ok(())
}

fn read_local_branch_head(
    context: &RepositoryContext,
    name: &str,
) -> Result<Option<String>, GitError> {
    let reference = format!("refs/heads/{name}");
    let output = run_git(
        &context.git,
        &context.root,
        [
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &reference,
        ],
        4096,
        false,
    )?;
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    ensure_success(&output, "read the exact local branch")?;
    let head = output_text(&output.stdout, "local branch object ID")?.trim();
    if !is_full_oid(head) {
        return Err(invalid_branch_output(
            "Git returned an invalid local branch object ID.",
        ));
    }
    Ok(Some(head.to_owned()))
}

fn ensure_direct_branch_ref(context: &RepositoryContext, name: &str) -> Result<(), GitError> {
    // Resolve the ref itself, not its object ID: a dangling symbolic ref has
    // no resolved tip but can still redirect Git's branch creation write.
    let reference = format!("refs/heads/{name}");
    let symbolic = run_git(
        &context.git,
        &context.root,
        ["symbolic-ref", "--quiet", &reference],
        64 * 1024,
        false,
    )?;
    if symbolic.status.success() {
        return Err(GitError::new(
            GitErrorKind::Unsupported,
            "This local ref is a symbolic alias. Select an ordinary local branch or choose a different new branch name.",
        ));
    }
    if symbolic.status.code() != Some(1) {
        ensure_success(&symbolic, "verify the local branch ref")?;
    }
    Ok(())
}

/// Create a local branch at the displayed HEAD. Checkout uses Git's safe,
/// transactional --create mode; this never resets an existing branch.
pub fn create_branch(
    repo_path: &str,
    name: &str,
    checkout: bool,
    expected_token: &str,
) -> Result<MutationOutcome, GitError> {
    let _guard = acquire_mutation_lock()?;
    let context = repository_context(repo_path)?;
    validate_branch_name(&context, name)?;
    validate_token(&context, expected_token)?;
    let (previous_branch, head, _, changes) = read_status(&context)?;
    ensure_no_branch_conflicts(&changes)?;
    ensure_no_active_operation(&context)?;
    let head = head.ok_or_else(|| {
        GitError::new(
            GitErrorKind::Unsupported,
            "Create the first commit before creating a branch from HEAD.",
        )
    })?;
    if read_local_branch_head(&context, name)?.is_some() {
        return Err(GitError::new(
            GitErrorKind::GitCommandFailed,
            "A local branch with that name already exists. Choose a different name.",
        ));
    }

    let mut args = vec!["--no-pager"];
    if checkout {
        args.extend([
            "switch",
            "--no-guess",
            "--no-recurse-submodules",
            "--no-overwrite-ignore",
            "--no-track",
            "--create",
            name,
            "--",
            &head,
        ]);
    } else {
        args.extend(["branch", "--no-track", "--", name, &head]);
    }
    // Git owns the actual ref/index locks. The app lock and final token check
    // cannot make a transaction with unrelated Git processes.
    ensure_index_unlocked(&context)?;
    validate_token(&context, expected_token)?;
    ensure_direct_branch_ref(&context, name)?;
    let result = run_git(&context.git, &context.root, args, MAX_STATUS_BYTES, true);
    ensure_mutation_success(result, "create the local branch")?;
    finish_branch_mutation(
        &context,
        "Branch creation",
        if checkout { name } else { &previous_branch },
        &head,
        name,
        &head,
    )
}

/// Switch to an exact local branch at its displayed tip. Safe Git switch may
/// carry nonconflicting edits; it is never forced, merged, stashed, or retried.
pub fn switch_branch(
    repo_path: &str,
    name: &str,
    expected_head: &str,
    expected_token: &str,
) -> Result<MutationOutcome, GitError> {
    let _guard = acquire_mutation_lock()?;
    let context = repository_context(repo_path)?;
    validate_branch_name(&context, name)?;
    if !is_full_oid(expected_head) {
        return Err(GitError::new(
            GitErrorKind::InvalidRevision,
            "Select a local branch with a full commit object ID from the current snapshot.",
        ));
    }
    validate_token(&context, expected_token)?;
    let (_, _, _, changes) = read_status(&context)?;
    ensure_no_branch_conflicts(&changes)?;
    ensure_no_active_operation(&context)?;
    let branch = read_branches(&context)?
        .into_iter()
        .find(|branch| branch.name == name)
        .ok_or_else(|| {
            GitError::new(
                GitErrorKind::BranchNotFound,
                "The selected local branch no longer exists. Refresh and select an existing local branch.",
            )
        })?;
    if branch.head != expected_head {
        return Err(stale_branch_tip());
    }
    if !branch.current && branch.worktree_path.is_some() {
        return Err(GitError::new(
            GitErrorKind::Busy,
            "The selected branch is checked out in another worktree. Open that worktree or select another branch.",
        ));
    }
    // A symbolic ref under refs/heads can alias a different branch. Refuse it
    // before writing rather than claiming an exact branch switch afterward.
    ensure_direct_branch_ref(&context, name)?;
    ensure_index_unlocked(&context)?;
    validate_token(&context, expected_token)?;
    if read_local_branch_head(&context, name)?.as_deref() != Some(expected_head) {
        return Err(stale_branch_tip());
    }
    if !branch.current {
        let result = run_git(
            &context.git,
            &context.root,
            [
                "--no-pager",
                "switch",
                "--no-guess",
                "--no-recurse-submodules",
                "--no-overwrite-ignore",
                "--",
                name,
            ],
            MAX_STATUS_BYTES,
            true,
        );
        ensure_mutation_success(result, "switch the local branch")?;
    }
    finish_branch_mutation(
        &context,
        "Branch switch",
        name,
        expected_head,
        name,
        expected_head,
    )
}

fn stale_branch_tip() -> GitError {
    GitError::new(
        GitErrorKind::StaleState,
        "The selected branch tip changed since the displayed snapshot. Refresh before switching.",
    )
}

fn finish_branch_mutation(
    context: &RepositoryContext,
    operation: &str,
    expected_branch: &str,
    expected_head: &str,
    reference_name: &str,
    reference_head: &str,
) -> Result<MutationOutcome, GitError> {
    let outcome = finish_mutation(context, operation, None)?;
    let verify_error = |error: GitError| {
        GitError::new(
            error.kind,
            format!("{operation} succeeded, but its final state could not be verified: {} Refresh and inspect the repository; do not repeat the write automatically.", error.message),
        )
    };
    let actual_ref = read_local_branch_head(context, reference_name).map_err(verify_error)?;
    let current_token = read_mutation_token(context).map_err(verify_error)?;
    let snapshot = &outcome.snapshot;
    if snapshot.branch != expected_branch
        || snapshot.head.as_deref() != Some(expected_head)
        || actual_ref.as_deref() != Some(reference_head)
        || !snapshot
            .branches
            .iter()
            .any(|branch| branch.name == reference_name && branch.head == reference_head)
        || current_token != snapshot.mutation_token
    {
        return Err(GitError::new(
            GitErrorKind::StaleState,
            format!("Git reported a successful {operation}, but HEAD or the branch ref changed unexpectedly afterward. A hook or another Git process may have moved it. Refresh and inspect the repository; do not repeat the write automatically."),
        ));
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::MUTATION_TEST_LOCK;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

    struct Fixture {
        directory: PathBuf,
        path: PathBuf,
        git: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let directory = std::env::temp_dir().join(format!(
                "enogram-branches-fixture-{}-{id}",
                std::process::id()
            ));
            fs::create_dir(&directory).expect("create disposable fixture directory");
            let path = directory.join("repository");
            fs::create_dir(&path).unwrap();
            let fixture = Self {
                directory,
                path,
                git: resolve_git().unwrap(),
            };
            fixture.git_ok(["init", "-b", "main"]);
            fixture.git_ok(["config", "user.name", "Enogram Branch Test"]);
            fixture.git_ok(["config", "user.email", "enogram-branches@example.invalid"]);
            fixture.git_ok(["config", "commit.gpgsign", "false"]);
            fixture.git_ok(["config", "core.autocrlf", "false"]);
            let hooks = fixture.path.join(".git/test-hooks");
            fs::create_dir(&hooks).unwrap();
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
            let output = run_git(&self.git, &self.path, args, MAX_STATUS_BYTES, false).unwrap();
            assert!(output.status.success(), "{}", stderr_text(&output));
        }

        fn git_text(&self, args: &[&str]) -> String {
            let output = run_git(&self.git, &self.path, args, MAX_STATUS_BYTES, false).unwrap();
            assert!(output.status.success(), "{}", stderr_text(&output));
            String::from_utf8(output.stdout).unwrap()
        }

        fn path_str(&self) -> &str {
            self.path.to_str().unwrap()
        }

        fn write(&self, name: &str, contents: &str) {
            fs::write(self.path.join(name), contents).unwrap();
        }

        fn commit(&self, subject: &str) -> String {
            self.git_ok(["add", "--all"]);
            self.git_ok(["commit", "-m", subject]);
            self.git_text(&["rev-parse", "HEAD"]).trim().to_owned()
        }

        fn snapshot(&self) -> RepositorySnapshot {
            inspect_repository(self.path_str()).unwrap()
        }

        fn token(&self) -> String {
            read_mutation_token(&repository_context(self.path_str()).unwrap()).unwrap()
        }

        fn hook(&self, name: &str, script: &str) {
            let path = self.path.join(".git/test-hooks").join(name);
            fs::write(&path, script).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if let (Ok(directory), Ok(temp)) = (
                fs::canonicalize(&self.directory),
                fs::canonicalize(std::env::temp_dir()),
            ) {
                if directory.parent() == Some(temp.as_path())
                    && directory
                        .file_name()
                        .and_then(OsStr::to_str)
                        .is_some_and(|name| name.starts_with("enogram-branches-fixture-"))
                {
                    let _ = fs::remove_dir_all(directory);
                }
            }
        }
    }

    #[test]
    fn branch_records_keep_path_newlines_and_distinguish_zero_from_unknown_tracking() {
        let oid = "a".repeat(40);
        let records = format!(
            "refs/heads/ışık\0{oid}\0*\0refs/remotes/origin/main\0origin\0refs/heads/main\0ahead 2, behind 3\0/work tree/with\nnewline\0\nrefs/heads/plain\0{oid}\0 \0\0\0\0\0\0\n"
        );
        let branches = parse_branches(records.as_bytes()).unwrap();
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0].name, "ışık");
        assert_eq!(branches[0].ahead, Some(2));
        assert_eq!(branches[0].behind, Some(3));
        assert_eq!(
            branches[0].worktree_path.as_deref(),
            Some("/work tree/with\nnewline")
        );
        assert_eq!((branches[1].ahead, branches[1].behind), (None, None));
        assert_eq!(parse_tracking_counts("", true).unwrap(), (Some(0), Some(0)));
        assert_eq!(parse_tracking_counts("gone", true).unwrap(), (None, None));
        for invalid in [
            "ahead -1",
            "ahead 1, ahead 2",
            "behind x",
            "ahead 4294967296",
        ] {
            assert!(parse_tracking_counts(invalid, true).is_err(), "{invalid}");
        }
        assert!(parse_branches(b"refs/heads/incomplete\0").is_err());
    }

    #[test]
    fn creation_from_head_preserves_partial_edits_with_and_without_checkout() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("partial.txt", "base\n");
        let head = fixture.commit("base");
        fixture.write("partial.txt", "staged\n");
        fixture.git_ok(["add", "--", "partial.txt"]);
        fixture.write("partial.txt", "working\n");
        fixture.write("untracked.txt", "keep\n");
        fixture.git_ok(["config", "branch.autoSetupMerge", "always"]);
        let before = fixture.snapshot();
        let created = create_branch(
            fixture.path_str(),
            "feature/ışık",
            false,
            &before.mutation_token,
        )
        .unwrap();
        assert_eq!(created.snapshot.branch, "main");
        assert_eq!(created.snapshot.head.as_deref(), Some(head.as_str()));
        assert_eq!(created.commit_oid, None);
        assert_eq!(created.snapshot.changes, before.changes);
        let branch = created
            .snapshot
            .branches
            .iter()
            .find(|branch| branch.name == "feature/ışık")
            .unwrap();
        assert_eq!(branch.head, head);
        assert!(!branch.current);
        assert_eq!(branch.upstream, None);
        let selected = create_branch(
            fixture.path_str(),
            "selected",
            true,
            &created.snapshot.mutation_token,
        )
        .unwrap();
        assert_eq!(selected.snapshot.branch, "selected");
        assert_eq!(selected.snapshot.head.as_deref(), Some(head.as_str()));
        assert!(selected
            .snapshot
            .branches
            .iter()
            .any(|branch| branch.name == "selected" && branch.current));
        assert_eq!(selected.snapshot.changes, before.changes);
        assert_eq!(fixture.git_text(&["show", ":partial.txt"]), "staged\n");
        assert_eq!(
            fs::read_to_string(fixture.path.join("partial.txt")).unwrap(),
            "working\n"
        );
        assert_eq!(
            fs::read_to_string(fixture.path.join("untracked.txt")).unwrap(),
            "keep\n"
        );
    }

    #[test]
    fn switch_keeps_nonconflicting_staged_and_unstaged_content() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("partial.txt", "base\n");
        fixture.write("branch.txt", "main\n");
        fixture.commit("base");
        fixture.git_ok(["switch", "-c", "side"]);
        fixture.write("branch.txt", "side\n");
        let side = fixture.commit("side");
        fixture.git_ok(["switch", "main"]);
        fixture.write("partial.txt", "staged\n");
        fixture.git_ok(["add", "--", "partial.txt"]);
        fixture.write("partial.txt", "working\n");
        fixture.write("untracked.txt", "keep\n");
        let before = fixture.snapshot();
        let outcome =
            switch_branch(fixture.path_str(), "side", &side, &before.mutation_token).unwrap();
        assert_eq!(outcome.snapshot.branch, "side");
        assert_eq!(outcome.snapshot.head.as_deref(), Some(side.as_str()));
        assert_eq!(outcome.snapshot.changes, before.changes);
        assert_eq!(fixture.git_text(&["show", ":partial.txt"]), "staged\n");
        assert_eq!(
            fs::read_to_string(fixture.path.join("partial.txt")).unwrap(),
            "working\n"
        );
        assert_eq!(
            fs::read_to_string(fixture.path.join("branch.txt")).unwrap(),
            "side\n"
        );
        assert_eq!(
            fs::read_to_string(fixture.path.join("untracked.txt")).unwrap(),
            "keep\n"
        );
    }

    #[test]
    fn conflicting_edits_and_ignored_files_block_switch_without_losing_content() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write(".gitignore", "ignored.txt\n");
        fixture.write("conflict.txt", "base\n");
        let main = fixture.commit("base");
        fixture.git_ok(["switch", "-c", "side"]);
        fixture.write("conflict.txt", "side\n");
        fixture.write("ignored.txt", "tracked on side\n");
        fixture.git_ok(["add", "--force", "--", "ignored.txt"]);
        let side = fixture.commit("side");
        fixture.git_ok(["switch", "main"]);
        fixture.write("conflict.txt", "local edit\n");
        fixture.write("ignored.txt", "local ignored content\n");
        let index = fixture.git_text(&["write-tree"]);
        let error = switch_branch(fixture.path_str(), "side", &side, &fixture.token()).unwrap_err();
        assert_eq!(error.kind, GitErrorKind::GitCommandFailed);
        assert_eq!(fixture.git_text(&["rev-parse", "HEAD"]).trim(), main);
        assert_eq!(fixture.git_text(&["write-tree"]), index);
        assert_eq!(
            fs::read_to_string(fixture.path.join("conflict.txt")).unwrap(),
            "local edit\n"
        );
        assert_eq!(
            fs::read_to_string(fixture.path.join("ignored.txt")).unwrap(),
            "local ignored content\n"
        );
        fixture.write("conflict.txt", "base\n");
        let error = switch_branch(fixture.path_str(), "side", &side, &fixture.token()).unwrap_err();
        assert_eq!(error.kind, GitErrorKind::GitCommandFailed);
        assert_eq!(
            fixture
                .git_text(&["symbolic-ref", "--short", "HEAD"])
                .trim(),
            "main"
        );
        assert_eq!(
            fs::read_to_string(fixture.path.join("ignored.txt")).unwrap(),
            "local ignored content\n"
        );
    }

    #[test]
    fn invalid_names_and_duplicate_branches_never_redirect_or_reset_refs() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("file.txt", "base\n");
        let base = fixture.commit("base");
        fixture.git_ok(["branch", "existing"]);
        fixture.write("file.txt", "second\n");
        fixture.commit("second");
        let token = fixture.token();
        for name in [
            "",
            "-",
            "--force",
            "@",
            "@{-1}",
            "HEAD",
            "HEAD^",
            "bad..name",
            "bad name",
            "bad\nname",
            "bad:name",
            "trailing/",
            "bad\0name",
        ] {
            assert_eq!(
                create_branch(fixture.path_str(), name, true, &token)
                    .unwrap_err()
                    .kind,
                GitErrorKind::InvalidBranchName,
                "{name:?}"
            );
            assert_eq!(
                switch_branch(fixture.path_str(), name, &base, &token)
                    .unwrap_err()
                    .kind,
                GitErrorKind::InvalidBranchName,
                "{name:?}"
            );
        }
        for checkout in [false, true] {
            assert_eq!(
                create_branch(fixture.path_str(), "existing", checkout, &token)
                    .unwrap_err()
                    .kind,
                GitErrorKind::GitCommandFailed
            );
        }
        assert_eq!(
            fixture
                .git_text(&["rev-parse", "refs/heads/existing"])
                .trim(),
            base
        );
        assert_eq!(
            fixture
                .git_text(&["symbolic-ref", "--short", "HEAD"])
                .trim(),
            "main"
        );
        assert_eq!(fixture.snapshot().branches.len(), 2);
    }

    #[test]
    fn dangling_symbolic_branch_blocks_creation_in_both_checkout_modes() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("file.txt", "base\n");
        let main = fixture.commit("base");
        fixture.git_ok([
            "symbolic-ref",
            "refs/heads/requested",
            "refs/heads/unrequested",
        ]);
        let context = repository_context(fixture.path_str()).unwrap();
        let alias_before = fs::read(fixture.path.join(".git/refs/heads/requested")).unwrap();
        let index_before = fs::read(fixture.path.join(".git/index")).unwrap();
        let token = fixture.token();
        for checkout in [false, true] {
            let error =
                create_branch(fixture.path_str(), "requested", checkout, &token).unwrap_err();
            assert_eq!(error.kind, GitErrorKind::Unsupported);
            assert_eq!(
                fs::read(fixture.path.join(".git/refs/heads/requested")).unwrap(),
                alias_before
            );
            assert!(read_local_branch_head(&context, "unrequested")
                .unwrap()
                .is_none());
            assert_eq!(
                fixture.git_text(&["symbolic-ref", "HEAD"]).trim(),
                "refs/heads/main"
            );
            assert_eq!(fixture.git_text(&["rev-parse", "HEAD"]).trim(), main);
            assert_eq!(
                fs::read(fixture.path.join(".git/index")).unwrap(),
                index_before
            );
            assert_eq!(
                fs::read_to_string(fixture.path.join("file.txt")).unwrap(),
                "base\n"
            );
            let branches = read_branches(&context).unwrap();
            assert_eq!(branches.len(), 1);
            assert_eq!(branches[0].name, "main");
        }
    }

    #[test]
    fn switch_binds_exact_local_ref_and_target_tip_instead_of_tags_or_remote_guessing() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("file.txt", "base\n");
        let base = fixture.commit("base");
        fixture.git_ok(["branch", "side"]);
        fixture.git_ok(["branch", "refs/heads/main"]);
        fixture.git_ok(["tag", "release"]);
        fixture.git_ok(["update-ref", "refs/remotes/origin/ghost", &base]);
        fixture.write("file.txt", "second\n");
        let main = fixture.commit("second");
        let displayed = fixture.snapshot();
        fixture.git_ok(["update-ref", "refs/heads/side", &main]);
        assert_eq!(
            switch_branch(fixture.path_str(), "side", &base, &displayed.mutation_token)
                .unwrap_err()
                .kind,
            GitErrorKind::StaleState
        );
        for name in ["release", "ghost", "missing"] {
            assert_eq!(
                switch_branch(fixture.path_str(), name, &base, &fixture.token())
                    .unwrap_err()
                    .kind,
                GitErrorKind::BranchNotFound
            );
        }
        assert_eq!(
            fixture
                .git_text(&["symbolic-ref", "--short", "HEAD"])
                .trim(),
            "main"
        );
        let nested = switch_branch(
            fixture.path_str(),
            "refs/heads/main",
            &base,
            &fixture.token(),
        )
        .unwrap();
        assert_eq!(nested.snapshot.branch, "refs/heads/main");
        assert_eq!(nested.snapshot.head.as_deref(), Some(base.as_str()));
        assert_eq!(
            fixture.git_text(&["symbolic-ref", "HEAD"]).trim(),
            "refs/heads/refs/heads/main"
        );
    }

    #[test]
    fn occupied_worktree_and_shared_app_or_index_locks_prevent_branch_writes() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("file.txt", "base\n");
        let base = fixture.commit("base");
        fixture.git_ok(["branch", "side"]);
        let linked = fixture.directory.join("linked worktree");
        fixture.git_ok([
            OsString::from("worktree"),
            OsString::from("add"),
            linked.as_os_str().to_owned(),
            OsString::from("side"),
        ]);
        let snapshot = fixture.snapshot();
        let side = snapshot
            .branches
            .iter()
            .find(|branch| branch.name == "side")
            .unwrap();
        assert_eq!(
            fs::canonicalize(side.worktree_path.as_ref().unwrap()).unwrap(),
            fs::canonicalize(&linked).unwrap()
        );
        assert_eq!(
            switch_branch(fixture.path_str(), "side", &base, &snapshot.mutation_token)
                .unwrap_err()
                .kind,
            GitErrorKind::Busy
        );
        let app_lock = MUTATION_LOCK.lock().unwrap();
        assert_eq!(
            create_branch(
                fixture.path_str(),
                "blocked",
                false,
                &snapshot.mutation_token
            )
            .unwrap_err()
            .kind,
            GitErrorKind::Busy
        );
        drop(app_lock);
        fs::write(fixture.path.join(".git/index.lock"), "fixture lock").unwrap();
        assert_eq!(
            create_branch(
                fixture.path_str(),
                "blocked",
                true,
                &snapshot.mutation_token
            )
            .unwrap_err()
            .kind,
            GitErrorKind::Busy
        );
        assert_eq!(
            fixture
                .git_text(&["symbolic-ref", "--short", "HEAD"])
                .trim(),
            "main"
        );
        assert_eq!(fixture.snapshot().branches.len(), 2);
    }

    #[test]
    fn unborn_and_active_operations_are_blocked_but_detached_creation_is_supported() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        assert_eq!(
            create_branch(fixture.path_str(), "unborn", true, &fixture.token())
                .unwrap_err()
                .kind,
            GitErrorKind::Unsupported
        );
        fixture.write("file.txt", "base\n");
        let base = fixture.commit("base");
        fixture.git_ok(["switch", "--detach"]);
        let created =
            create_branch(fixture.path_str(), "rescued", false, &fixture.token()).unwrap();
        assert_eq!(created.snapshot.branch, "(detached)");
        assert_eq!(created.snapshot.head.as_deref(), Some(base.as_str()));
        let selected = create_branch(
            fixture.path_str(),
            "selected",
            true,
            &created.snapshot.mutation_token,
        )
        .unwrap();
        assert_eq!(selected.snapshot.branch, "selected");
        for name in [
            "MERGE_HEAD",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
            "rebase-merge",
            "rebase-apply",
            "sequencer",
        ] {
            let path = fixture.path.join(".git").join(name);
            let directory = matches!(name, "rebase-merge" | "rebase-apply" | "sequencer");
            if directory {
                fs::create_dir(&path).unwrap();
            } else {
                fs::write(&path, format!("{base}\n")).unwrap();
            }
            let token = fixture.token();
            assert_eq!(
                create_branch(fixture.path_str(), "blocked", false, &token)
                    .unwrap_err()
                    .kind,
                GitErrorKind::Unsupported,
                "{name}"
            );
            assert_eq!(
                switch_branch(fixture.path_str(), "main", &base, &token)
                    .unwrap_err()
                    .kind,
                GitErrorKind::Unsupported,
                "{name}"
            );
            if directory {
                fs::remove_dir(&path).unwrap();
            } else {
                fs::remove_file(&path).unwrap();
            }
        }
        assert!(read_local_branch_head(
            &repository_context(fixture.path_str()).unwrap(),
            "blocked"
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn branch_inventory_reads_remote_and_local_upstreams_and_gone_tracking() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("file.txt", "base\n");
        let base = fixture.commit("base");
        fixture.git_ok(["branch", "behind"]);
        fixture.git_ok(["branch", "side"]);
        fixture.write("file.txt", "main\n");
        let main = fixture.commit("main");
        fixture.git_ok(["branch", "--track", "synced", "main"]);
        fixture.git_ok(["branch", "--set-upstream-to=main", "behind"]);
        fixture.git_ok(["switch", "side"]);
        fixture.write("file.txt", "side\n");
        let side = fixture.commit("side");
        fixture.git_ok(["switch", "main"]);
        fixture.git_ok([
            "remote",
            "add",
            "origin",
            "https://example.invalid/enogram-fixture.git",
        ]);
        fixture.git_ok(["update-ref", "refs/remotes/origin/main", &side]);
        fixture.git_ok(["config", "branch.main.remote", "origin"]);
        fixture.git_ok(["config", "branch.main.merge", "refs/heads/main"]);
        fixture.git_ok(["branch", "gone"]);
        fixture.git_ok(["config", "branch.gone.remote", "origin"]);
        fixture.git_ok(["config", "branch.gone.merge", "refs/heads/missing"]);
        let snapshot = fixture.snapshot();
        let branch = |name: &str| {
            snapshot
                .branches
                .iter()
                .find(|branch| branch.name == name)
                .unwrap()
        };
        assert_eq!(branch("main").head, main);
        assert_eq!(
            branch("main").upstream.as_deref(),
            Some("refs/remotes/origin/main")
        );
        assert_eq!(branch("main").upstream_remote.as_deref(), Some("origin"));
        assert_eq!(
            branch("main").upstream_branch.as_deref(),
            Some("refs/heads/main")
        );
        assert_eq!(
            (branch("main").ahead, branch("main").behind),
            (Some(1), Some(1))
        );
        assert_eq!(branch("behind").head, base);
        assert_eq!(branch("behind").upstream_remote.as_deref(), Some("."));
        assert_eq!(
            branch("behind").upstream_branch.as_deref(),
            Some("refs/heads/main")
        );
        assert_eq!(
            (branch("behind").ahead, branch("behind").behind),
            (Some(0), Some(1))
        );
        assert_eq!(
            (branch("synced").ahead, branch("synced").behind),
            (Some(0), Some(0))
        );
        assert_eq!(
            branch("gone").upstream.as_deref(),
            Some("refs/remotes/origin/missing")
        );
        assert_eq!((branch("gone").ahead, branch("gone").behind), (None, None));
        assert_eq!((branch("side").ahead, branch("side").behind), (None, None));
        let json = serde_json::to_value(branch("main")).unwrap();
        assert_eq!(json["upstreamRemote"], "origin");
        assert!(json.get("worktreePath").is_some());
        assert!(json.get("upstream_remote").is_none());
    }

    #[test]
    fn checkout_hook_failure_or_head_movement_is_reported_without_retry_or_rollback() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("file.txt", "base\n");
        let base = fixture.commit("base");
        fixture.git_ok(["switch", "-c", "side"]);
        fixture.write("file.txt", "side\n");
        let side = fixture.commit("side");
        fixture.git_ok(["switch", "main"]);
        fixture.hook("post-checkout", "#!/bin/sh\nprintf x >> .git/hook-runs\nprintf 'fixture rejected checkout\\n' >&2\nexit 1\n");
        let error = switch_branch(fixture.path_str(), "side", &side, &fixture.token()).unwrap_err();
        assert_eq!(error.kind, GitErrorKind::GitCommandFailed);
        assert!(error.message.contains("state may have changed"));
        assert_eq!(
            fixture
                .git_text(&["symbolic-ref", "--short", "HEAD"])
                .trim(),
            "side"
        );
        assert_eq!(fs::read(fixture.path.join(".git/hook-runs")).unwrap(), b"x");
        assert_eq!(
            fs::read_to_string(fixture.path.join("file.txt")).unwrap(),
            "side\n"
        );
        fixture.hook(
            "post-checkout",
            "#!/bin/sh\nprintf x >> .git/hook-runs\ngit symbolic-ref HEAD refs/heads/side\n",
        );
        let error = switch_branch(fixture.path_str(), "main", &base, &fixture.token()).unwrap_err();
        assert_eq!(error.kind, GitErrorKind::StaleState);
        assert!(error.message.contains("do not repeat"));
        assert_eq!(
            fixture
                .git_text(&["symbolic-ref", "--short", "HEAD"])
                .trim(),
            "side"
        );
        assert_eq!(
            fs::read(fixture.path.join(".git/hook-runs")).unwrap(),
            b"xx"
        );
    }

    #[test]
    fn changed_created_ref_is_reported_as_uncertain_after_successful_git_command() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        fixture.write("file.txt", "base\n");
        let base = fixture.commit("base");
        fixture.git_ok(["branch", "other"]);
        fixture.write("file.txt", "main\n");
        let main = fixture.commit("main");
        fixture.hook("reference-transaction", "#!/bin/sh\nif [ \"$1\" = committed ] && [ ! -e .git/branch-hook-ran ]; then\n  while read old new ref; do\n    if [ \"$ref\" = refs/heads/created ]; then\n      printf x > .git/branch-hook-ran\n      git update-ref refs/heads/created refs/heads/other\n    fi\n  done\nfi\n");
        let error =
            create_branch(fixture.path_str(), "created", false, &fixture.token()).unwrap_err();
        assert_eq!(error.kind, GitErrorKind::StaleState);
        assert!(error.message.contains("do not repeat"));
        assert_eq!(fixture.git_text(&["rev-parse", "HEAD"]).trim(), main);
        assert_eq!(
            fixture
                .git_text(&["rev-parse", "refs/heads/created"])
                .trim(),
            base
        );
        assert_eq!(
            fs::read(fixture.path.join(".git/branch-hook-ran")).unwrap(),
            b"x"
        );
    }
}
