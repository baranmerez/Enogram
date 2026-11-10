//! Server-owned line selections. The client returns line IDs, never a patch.
//! A native index lock protects a private copy until one atomic publication;
//! Git applies only to that copy and never opens the working file for writing.

use super::*;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_INDEX: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialDiffResult {
    pub text: String,
    pub truncated: bool,
    pub binary: bool,
    pub selection_token: Option<String>,
    /// Zero-based physical line indexes in `text.split('\n')`.
    pub selectable_line_ids: Vec<usize>,
    pub unsupported_reason: Option<String>,
}

struct PreparedDiff {
    result: PartialDiffResult,
    parsed: Option<ParsedPatch>,
    source: Option<IndexEntry>,
    worktree: Option<Vec<u8>>,
}

struct IndexEntry {
    mode: String,
    bytes: Vec<u8>,
}

#[derive(Clone)]
struct PatchLine {
    id: usize,
    kind: u8,
    // Includes the LF, except when Git's following no-newline marker removes it.
    bytes: Vec<u8>,
}

struct Hunk {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    lines: Vec<PatchLine>,
}

struct ParsedPatch {
    old_exists: bool,
    new_exists: bool,
    mode: String,
    hunks: Vec<Hunk>,
}

/// Prepare exactly the displayed bytes and the IDs the backend can select.
/// Unsupported representations still return display text, with no write token.
pub fn prepare_partial_diff(
    repo_path: &str,
    file_path: &str,
    staged: bool,
    expected_token: &str,
) -> Result<PartialDiffResult, GitError> {
    let context = repository_context(repo_path)?;
    Ok(prepare_context(&context, file_path, staged, expected_token)?.result)
}

/// Stage (`staged=false`) or unstage (`staged=true`) exact displayed changed
/// lines. A failure is never retried. Unreachable Git blobs may remain after a
/// failed private-index operation, but the real index is published only once.
pub fn apply_partial_selection(
    repo_path: &str,
    file_path: &str,
    staged: bool,
    expected_token: &str,
    selection_token: &str,
    line_ids: &[usize],
) -> Result<MutationOutcome, GitError> {
    let _guard = acquire_mutation_lock()?;
    let context = repository_context(repo_path)?;
    validate_token(&context, expected_token)?;
    let transaction = IndexTransaction::begin(&context)?;
    // Native index.lock is now held: external Git writers cannot slip between
    // this validation and publication of the selected result.
    let prepared = prepare_context(&context, file_path, staged, expected_token)?;
    if selection_token.is_empty()
        || prepared.result.selection_token.as_deref() != Some(selection_token)
    {
        return Err(stale_selection());
    }
    let parsed = prepared.parsed.as_ref().ok_or_else(|| {
        unsupported("This diff cannot be partially staged. Use the whole-file action.")
    })?;
    let selected: BTreeSet<_> = line_ids.iter().copied().collect();
    if selected.is_empty()
        || selected.len() != line_ids.len()
        || selected.len() > prepared.result.selectable_line_ids.len()
        || selected.iter().any(|id| {
            prepared
                .result
                .selectable_line_ids
                .binary_search(id)
                .is_err()
        })
    {
        return Err(GitError::new(
            GitErrorKind::InvalidMutationPath,
            "Select unique changed lines from the current displayed diff.",
        ));
    }
    let patch = selected_patch(
        file_path,
        staged,
        parsed,
        prepared.source.as_ref(),
        &selected,
    )?;
    let output = partial_git(
        &context,
        [
            "-c",
            "apply.ignoreWhitespace=false",
            "apply",
            "--cached",
            "--whitespace=nowarn",
            "--unidiff-zero",
            "-p1",
            "-",
        ],
        MAX_DIFF_BYTES,
        false,
        Some(&patch),
        Some(&transaction.private_path),
    );
    ensure_mutation_success(output, "apply the selected lines to the private index")?;
    // Working-file bytes are not part of the ordinary mutation token. Check
    // them again before publication, including the missing-file state.
    if read_worktree(&context, file_path)? != prepared.worktree {
        return Err(stale_selection());
    }
    validate_token(&context, expected_token)?;
    transaction.publish()?;
    finish_mutation(
        &context,
        if staged {
            "Unstage selected lines"
        } else {
            "Stage selected lines"
        },
        None,
    )
}

fn prepare_context(
    context: &RepositoryContext,
    file_path: &str,
    staged: bool,
    expected_token: &str,
) -> Result<PreparedDiff, GitError> {
    validate_token(context, expected_token)?;
    validate_relative_path(file_path)?;
    let (_, _, _, changes) = read_status(context)?;
    // A staged deletion with a recreated/untracked working file has two status
    // rows for the same path. Select the row belonging to the requested layer.
    let change = changes.iter().find(|change| {
        change.path == file_path && if staged {
            !matches!(change.index_status.as_str(), "." | " " | "?")
        } else {
            !matches!(change.worktree_status.as_str(), "." | " ")
        }
    }).ok_or_else(|| {
        GitError::new(GitErrorKind::InvalidMutationPath, "The selected file is no longer changed in this layer. Refresh and select it again.")
    })?;
    let mut reason = if is_conflict(change) {
        Some("Resolve the file's merge conflicts before selecting individual lines.".to_owned())
    } else if change.original_path.is_some()
        || matches!(change.index_status.as_str(), "R" | "C" | "T")
        || matches!(change.worktree_status.as_str(), "R" | "C" | "T")
    {
        Some("Renames, copies, and file-type changes require the whole-file action.".to_owned())
    } else {
        None
    };
    let applicable = if staged {
        &change.index_status
    } else {
        &change.worktree_status
    };
    if matches!(applicable.as_str(), "." | " ") || (staged && applicable == "?") {
        return Err(GitError::new(
            GitErrorKind::InvalidMutationPath,
            "This file no longer has changes in the selected layer. Refresh it.",
        ));
    }
    // Directory and symlink representations are display-only. Validate their
    // parent paths using the same exact-file boundary as other index writes.
    match validate_mutation_path(context, file_path, &changes) {
        Ok(()) => {}
        Err(error)
            if fs::symlink_metadata(context.root.join(file_path)).is_ok_and(|m| m.is_dir()) =>
        {
            reason = Some(
                "Submodules and directories do not support individual-line actions.".to_owned(),
            );
            // Still reject traversal/Git metadata before asking Git to read it.
            let prefix = format!("{file_path}/");
            if file_path.split('/').any(|part| {
                part.is_empty() || part == "." || part == ".." || part.eq_ignore_ascii_case(".git")
            }) || changes.iter().any(|other| other.path.starts_with(&prefix))
            {
                return Err(error);
            }
        }
        Err(error) => return Err(error),
    }
    let attributes = read_attributes(context, file_path)?;
    if attributes.chunks_exact(3).any(|row| {
        matches!(
            row[1].as_slice(),
            b"filter" | b"working-tree-encoding" | b"ident"
        ) && !matches!(row[2].as_slice(), b"unspecified" | b"unset")
    }) {
        // Preserve the existing read-only Git diff (including its bounded
        // configured conversion), but never issue a partial-write token.
        reason = Some("Clean/smudge filters, working-tree encoding, and ident conversion require the whole-file action.".to_owned());
    }
    let worktree = match read_worktree(context, file_path) {
        Ok(bytes) => bytes,
        Err(error)
            if matches!(
                error.kind,
                GitErrorKind::Unsupported | GitErrorKind::OutputLimit
            ) =>
        {
            reason = Some(error.message);
            None
        }
        Err(error) => return Err(error),
    };
    let output = canonical_diff(context, file_path, staged, change.worktree_status == "?")?;
    let binary = diff_is_binary(&output.stdout);
    let mut result = PartialDiffResult {
        text: String::from_utf8_lossy(&output.stdout).into_owned(),
        truncated: output.stdout_truncated,
        binary,
        selection_token: None,
        selectable_line_ids: Vec::new(),
        unsupported_reason: None,
    };
    if result.truncated {
        reason = Some("This diff is truncated. Use the whole-file action.".to_owned());
    } else if binary {
        reason = Some("Binary files do not support individual-line actions.".to_owned());
    } else if std::str::from_utf8(&output.stdout).is_err() {
        reason = Some("This diff is not valid UTF-8. Use the whole-file action.".to_owned());
    }
    let mut parsed = None;
    let mut source = None;
    if reason.is_none() {
        match parse_patch(&output.stdout) {
            Ok(patch) => {
                match read_index_entry(context, file_path) {
                    Ok(entry) => source = entry,
                    Err(error)
                        if matches!(
                            error.kind,
                            GitErrorKind::Unsupported | GitErrorKind::OutputLimit
                        ) =>
                    {
                        reason = Some(error.message)
                    }
                    Err(error) => return Err(error),
                }
                let source_exists = if staged {
                    patch.new_exists
                } else {
                    patch.old_exists
                };
                if reason.is_some() {
                    // Preserve the precise unsupported/size reason above.
                } else if source_exists != source.is_some() {
                    reason = Some("Intent-to-add entries and changing file identities require the whole-file action.".to_owned());
                } else if source.as_ref().is_some_and(|entry| {
                    entry.mode != patch.mode || std::str::from_utf8(&entry.bytes).is_err()
                }) {
                    reason = Some("File mode changes and non-UTF-8 index content require the whole-file action.".to_owned());
                } else {
                    result.selectable_line_ids = patch
                        .hunks
                        .iter()
                        .flat_map(|hunk| hunk.lines.iter())
                        .filter(|line| matches!(line.kind, b'+' | b'-'))
                        .map(|line| line.id)
                        .collect();
                    if result.selectable_line_ids.is_empty() {
                        reason = Some(
                            "This change has no selectable text lines. Use the whole-file action."
                                .to_owned(),
                        );
                    } else {
                        parsed = Some(patch);
                    }
                }
            }
            Err(error) => reason = Some(error.message),
        }
    }
    validate_token(context, expected_token)?;
    if reason.is_none() {
        if read_worktree(context, file_path)? != worktree {
            return Err(stale_selection());
        }
        let mut fingerprint = b"enogram-partial-v1\0".to_vec();
        append_fingerprint_field(&mut fingerprint, expected_token.as_bytes());
        append_fingerprint_field(&mut fingerprint, file_path.as_bytes());
        fingerprint.push(u8::from(staged));
        append_fingerprint_field(&mut fingerprint, &output.stdout);
        fingerprint.push(u8::from(worktree.is_some()));
        append_fingerprint_field(&mut fingerprint, worktree.as_deref().unwrap_or_default());
        let hash = partial_git(
            context,
            ["hash-object", "--no-filters", "--stdin"],
            4096,
            false,
            Some(&fingerprint),
            None,
        )?;
        ensure_success(&hash, "fingerprint the displayed partial diff")?;
        let hash = output_text(&hash.stdout, "partial diff token")?.trim();
        if !matches!(hash.len(), 40 | 64) || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(GitError::new(
                GitErrorKind::InvalidOutput,
                "Git returned an invalid partial-diff fingerprint.",
            ));
        }
        result.selection_token = Some(format!("partial-v1:{hash}"));
    } else {
        result.selectable_line_ids.clear();
    }
    result.unsupported_reason = reason;
    Ok(PreparedDiff {
        result,
        parsed,
        source,
        worktree,
    })
}

fn read_attributes(context: &RepositoryContext, file_path: &str) -> Result<Vec<Vec<u8>>, GitError> {
    let output = partial_git(
        context,
        [
            "--literal-pathspecs",
            "check-attr",
            "-z",
            "filter",
            "working-tree-encoding",
            "ident",
            "--",
            file_path,
        ],
        64 * 1024,
        false,
        None,
        None,
    )?;
    ensure_success(&output, "read partial-staging attributes")?;
    let fields = output
        .stdout
        .strip_suffix(b"\0")
        .ok_or_else(|| unsupported("Git did not return complete file attributes."))?
        .split(|byte| *byte == 0)
        .map(<[u8]>::to_vec)
        .collect::<Vec<_>>();
    if fields.len() != 9
        || fields
            .chunks_exact(3)
            .any(|row| row[0] != file_path.as_bytes())
    {
        return Err(unsupported("Git did not return exact file attributes."));
    }
    Ok(fields)
}

fn read_worktree(
    context: &RepositoryContext,
    file_path: &str,
) -> Result<Option<Vec<u8>>, GitError> {
    let path = context.root.join(file_path);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(fingerprint_io_error(error)),
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            return Err(unsupported(
                "Symlinks, submodules, and special files require the whole-file action.",
            ));
        }
        Ok(_) => {}
    }
    let file = fs::File::open(&path).map_err(fingerprint_io_error)?;
    let mut bytes = Vec::new();
    file.take(MAX_WORKTREE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(fingerprint_io_error)?;
    if bytes.len() > MAX_WORKTREE_BYTES {
        return Err(GitError::new(
            GitErrorKind::OutputLimit,
            "This file exceeds the 4 MiB partial-selection limit. Use the whole-file action.",
        ));
    }
    Ok(Some(bytes))
}

fn canonical_diff(
    context: &RepositoryContext,
    file_path: &str,
    staged: bool,
    untracked: bool,
) -> Result<CapturedOutput, GitError> {
    let mut args = vec![
        "-c",
        "core.quotePath=true",
        "-c",
        "diff.suppressBlankEmpty=false",
        "--no-pager",
        "--literal-pathspecs",
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--no-renames",
        "--no-relative",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        "--line-prefix=",
        "--output-indicator-new=+",
        "--output-indicator-old=-",
        "--output-indicator-context= ",
        "--unified=3",
        "--inter-hunk-context=0",
        "--diff-algorithm=myers",
        "--no-indent-heuristic",
        "--ignore-submodules=none",
        "--submodule=short",
    ];
    if staged {
        args.push("--cached");
    }
    if untracked && !staged {
        args.push("--no-index");
    }
    args.push("--");
    if untracked && !staged {
        args.push("/dev/null");
    }
    args.push(file_path);
    let output = partial_git(context, args, MAX_DIFF_BYTES, true, None, None)?;
    if !(output.status.success() || (untracked && !staged && output.status.code() == Some(1))) {
        ensure_success(&output, "read the canonical partial diff")?;
    }
    Ok(output)
}

fn read_index_entry(
    context: &RepositoryContext,
    file_path: &str,
) -> Result<Option<IndexEntry>, GitError> {
    let output = partial_git(
        context,
        [
            "--literal-pathspecs",
            "ls-files",
            "--stage",
            "--full-name",
            "-z",
            "--",
            file_path,
        ],
        64 * 1024,
        false,
        None,
        None,
    )?;
    ensure_success(&output, "read the selected index entry")?;
    if output.stdout.is_empty() {
        return Ok(None);
    }
    let record = output
        .stdout
        .strip_suffix(b"\0")
        .ok_or_else(|| unsupported("Invalid index entry."))?;
    let tab = record
        .iter()
        .position(|byte| *byte == b'\t')
        .ok_or_else(|| unsupported("Invalid index entry."))?;
    if &record[tab + 1..] != file_path.as_bytes() {
        return Err(unsupported(
            "Partial selections require one exact, unmerged index entry.",
        ));
    }
    let header = output_text(&record[..tab], "index entry")?
        .split(' ')
        .collect::<Vec<_>>();
    if header.len() != 3
        || header[2] != "0"
        || !matches!(header[0], "100644" | "100755")
        || !matches!(header[1].len(), 40 | 64)
        || !header[1].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(unsupported(
            "Only ordinary, unmerged text files support individual-line actions.",
        ));
    }
    let blob = partial_git(
        context,
        ["cat-file", "blob", header[1]],
        MAX_WORKTREE_BYTES,
        false,
        None,
        None,
    )?;
    ensure_success(&blob, "read the selected index blob")?;
    Ok(Some(IndexEntry {
        mode: header[0].to_owned(),
        bytes: blob.stdout,
    }))
}

fn partial_git<I, S>(
    context: &RepositoryContext,
    args: I,
    limit: usize,
    truncate: bool,
    input: Option<&[u8]>,
    index: Option<&Path>,
) -> Result<CapturedOutput, GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = git_command(&context.git, &context.root, args);
    command
        .env("LC_ALL", "C")
        .env_remove("GIT_DIFF_OPTS")
        .env("GIT_NO_LAZY_FETCH", "1");
    if let Some(index) = index {
        // Rust canonical paths use Win32's verbatim prefix. Git for Windows
        // accepts normal absolute/UNC paths in this variable, not that prefix.
        let path = path_to_string(index)?;
        #[cfg(windows)]
        let path = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
            format!("//{}", unc.replace('\\', "/"))
        } else {
            path.strip_prefix(r"\\?\")
                .unwrap_or(&path)
                .replace('\\', "/")
        };
        command.env("GIT_INDEX_FILE", path);
    }
    git_process::run(&mut command, limit, truncate, input, COMMAND_TIMEOUT)
}

fn parse_patch(bytes: &[u8]) -> Result<ParsedPatch, GitError> {
    let mut patch = ParsedPatch {
        old_exists: true,
        new_exists: true,
        mode: String::new(),
        hunks: Vec::new(),
    };
    let mut files = 0;
    for (id, raw) in bytes.split_inclusive(|byte| *byte == b'\n').enumerate() {
        if !raw.ends_with(b"\n") {
            return Err(unsupported(
                "Incomplete diff lines require the whole-file action.",
            ));
        }
        let line = &raw[..raw.len() - 1];
        if line.starts_with(b"diff --git ") {
            files += 1;
            if files != 1 {
                return Err(unsupported(
                    "Partial selection requires a single-file diff.",
                ));
            }
        } else if line.starts_with(b"@@ ") {
            if let Some(previous) = patch.hunks.last() {
                validate_hunk(previous)?;
            }
            let text = output_text(line, "hunk header")?;
            let fields = text.split(' ').collect::<Vec<_>>();
            if fields.len() < 4 || fields[0] != "@@" || fields[3] != "@@" {
                return Err(unsupported("Unsupported hunk header."));
            }
            let (old_start, old_count) = parse_range(fields[1], '-')?;
            let (new_start, new_count) = parse_range(fields[2], '+')?;
            patch.hunks.push(Hunk {
                old_start,
                old_count,
                new_start,
                new_count,
                lines: Vec::new(),
            });
        } else if let Some(hunk) = patch.hunks.last_mut() {
            if line == b"\\ No newline at end of file" {
                let previous = hunk
                    .lines
                    .last_mut()
                    .ok_or_else(|| unsupported("Invalid end-of-file marker."))?;
                if previous.bytes.pop() != Some(b'\n') {
                    return Err(unsupported("Invalid duplicate end-of-file marker."));
                }
            } else if matches!(line.first(), Some(b' ' | b'+' | b'-')) {
                hunk.lines.push(PatchLine {
                    id,
                    kind: line[0],
                    bytes: raw[1..].to_vec(),
                });
            } else {
                return Err(unsupported(
                    "Unsupported diff body. Use the whole-file action.",
                ));
            }
        } else if let Some(mode) = line.strip_prefix(b"new file mode ") {
            patch.old_exists = false;
            patch.mode = output_text(mode, "file mode")?.to_owned();
        } else if let Some(mode) = line.strip_prefix(b"deleted file mode ") {
            patch.new_exists = false;
            patch.mode = output_text(mode, "file mode")?.to_owned();
        } else if line.starts_with(b"old mode ")
            || line.starts_with(b"new mode ")
            || line.starts_with(b"rename ")
            || line.starts_with(b"copy ")
            || line.starts_with(b"similarity ")
        {
            return Err(unsupported(
                "File mode changes, renames, and copies require the whole-file action.",
            ));
        } else if let Some(index) = line.strip_prefix(b"index ") {
            if let Some(mode) = index.split(|byte| *byte == b' ').nth(1) {
                patch.mode = output_text(mode, "file mode")?.to_owned();
            }
        } else if !line.starts_with(b"--- ") && !line.starts_with(b"+++ ") {
            return Err(unsupported(
                "Unsupported diff metadata. Use the whole-file action.",
            ));
        }
    }
    if let Some(last) = patch.hunks.last() {
        validate_hunk(last)?;
    }
    if files != 1 || !matches!(patch.mode.as_str(), "100644" | "100755") {
        return Err(unsupported(
            "Only ordinary text-file changes support individual-line actions.",
        ));
    }
    Ok(patch)
}

fn parse_range(value: &str, sign: char) -> Result<(usize, usize), GitError> {
    let value = value
        .strip_prefix(sign)
        .ok_or_else(|| unsupported("Invalid hunk range."))?;
    let (start, count) = value.split_once(',').unwrap_or((value, "1"));
    let start = start
        .parse::<usize>()
        .map_err(|_| unsupported("Invalid hunk range."))?;
    let count = count
        .parse::<usize>()
        .map_err(|_| unsupported("Invalid hunk range."))?;
    if (count > 0 && start == 0) || start > MAX_WORKTREE_BYTES || count > MAX_WORKTREE_BYTES {
        return Err(unsupported(
            "The hunk range exceeds the partial-selection limit.",
        ));
    }
    Ok((start, count))
}

fn validate_hunk(hunk: &Hunk) -> Result<(), GitError> {
    if hunk.lines.iter().filter(|line| line.kind != b'+').count() != hunk.old_count
        || hunk.lines.iter().filter(|line| line.kind != b'-').count() != hunk.new_count
    {
        return Err(unsupported("The diff contains an incomplete hunk."));
    }
    Ok(())
}

fn selected_patch(
    file_path: &str,
    reverse: bool,
    parsed: &ParsedPatch,
    source: Option<&IndexEntry>,
    selected: &BTreeSet<usize>,
) -> Result<Vec<u8>, GitError> {
    let bytes = source
        .map(|entry| entry.bytes.as_slice())
        .unwrap_or_default();
    let source_lines = bytes
        .split_inclusive(|byte| *byte == b'\n')
        .collect::<Vec<_>>();
    let mut projection: Vec<(u8, &[u8])> = Vec::new();
    let mut cursor = 0;
    for hunk in &parsed.hunks {
        let (start, count) = if reverse {
            (hunk.new_start, hunk.new_count)
        } else {
            (hunk.old_start, hunk.old_count)
        };
        let offset = if count == 0 { start } else { start - 1 };
        if offset < cursor || offset > source_lines.len() {
            return Err(stale_selection());
        }
        for line in &source_lines[cursor..offset] {
            projection.push((b' ', line));
        }
        cursor = offset;
        for line in &hunk.lines {
            let kind = if reverse {
                match line.kind {
                    b'+' => b'-',
                    b'-' => b'+',
                    value => value,
                }
            } else {
                line.kind
            };
            if kind != b'+' {
                if source_lines.get(cursor).copied() != Some(line.bytes.as_slice()) {
                    return Err(stale_selection());
                }
                cursor += 1;
            }
            match kind {
                b' ' => projection.push((b' ', &line.bytes)),
                b'-' => projection.push((
                    if selected.contains(&line.id) {
                        b'-'
                    } else {
                        b' '
                    },
                    &line.bytes,
                )),
                b'+' if selected.contains(&line.id) => projection.push((b'+', &line.bytes)),
                b'+' => {}
                _ => unreachable!("parser accepts only canonical hunk lines"),
            }
        }
    }
    for line in &source_lines[cursor..] {
        projection.push((b' ', line));
    }
    let target_lines = projection
        .iter()
        .filter(|(kind, _)| *kind != b'-')
        .collect::<Vec<_>>();
    if target_lines
        .iter()
        .take(target_lines.len().saturating_sub(1))
        .any(|(_, bytes)| !bytes.ends_with(b"\n"))
    {
        return Err(unsupported("This selection would join a line without a final newline to another line. Select the related end-of-file replacement together, or use the whole-file action."));
    }
    let total_changes = parsed
        .hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .filter(|line| line.kind != b' ')
        .count();
    let target_exists = if selected.len() == total_changes {
        if reverse {
            parsed.old_exists
        } else {
            parsed.new_exists
        }
    } else {
        true
    };
    let old_path = quoted_path("a/", file_path);
    let new_path = quoted_path("b/", file_path);
    let mut patch = format!("diff --git {old_path} {new_path}\n").into_bytes();
    if source.is_none() {
        patch.extend_from_slice(format!("new file mode {}\n", parsed.mode).as_bytes());
    } else if !target_exists {
        patch.extend_from_slice(format!("deleted file mode {}\n", parsed.mode).as_bytes());
    }
    patch.extend_from_slice(
        format!(
            "--- {}\n+++ {}\n",
            if source.is_none() {
                "/dev/null"
            } else {
                &old_path
            },
            if target_exists {
                &new_path
            } else {
                "/dev/null"
            }
        )
        .as_bytes(),
    );
    patch.extend_from_slice(
        format!(
            "@@ -{},{} +{},{} @@\n",
            usize::from(!source_lines.is_empty()),
            source_lines.len(),
            usize::from(!target_lines.is_empty()),
            target_lines.len()
        )
        .as_bytes(),
    );
    for (kind, bytes) in projection {
        patch.push(kind);
        patch.extend_from_slice(bytes);
        if !bytes.ends_with(b"\n") {
            patch.extend_from_slice(b"\n\\ No newline at end of file\n");
        }
    }
    Ok(patch)
}

fn quoted_path(prefix: &str, path: &str) -> String {
    let mut quoted = String::from("\"");
    for byte in prefix.bytes().chain(path.bytes()) {
        match byte {
            b'"' => quoted.push_str("\\\""),
            b'\\' => quoted.push_str("\\\\"),
            0x20..=0x7e => quoted.push(byte as char),
            _ => quoted.push_str(&format!("\\{byte:03o}")),
        }
    }
    quoted.push('"');
    quoted
}

struct IndexTransaction {
    index_path: PathBuf,
    lock_path: PathBuf,
    private_path: PathBuf,
    owns_private: bool,
    published: bool,
}

impl IndexTransaction {
    fn begin(context: &RepositoryContext) -> Result<Self, GitError> {
        let lock_path = context.git_dir.join("index.lock");
        let lock = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    GitError::new(
                        GitErrorKind::Busy,
                        "The Git index is locked by another operation. Wait and refresh.",
                    )
                } else {
                    fingerprint_io_error(error)
                }
            })?;
        drop(lock);
        let id = NEXT_INDEX.fetch_add(1, Ordering::Relaxed);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let mut transaction = Self {
            index_path: context.git_dir.join("index"),
            lock_path,
            private_path: context.git_dir.join(format!(
                "enogram-partial-{}-{timestamp}-{id}.index",
                std::process::id()
            )),
            owns_private: false,
            published: false,
        };
        let mut private = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&transaction.private_path)
            .map_err(fingerprint_io_error)?;
        transaction.owns_private = true;
        match fs::File::open(&transaction.index_path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(MAX_INDEX_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(fingerprint_io_error)?;
                if bytes.len() > MAX_INDEX_BYTES {
                    return Err(GitError::new(
                        GitErrorKind::OutputLimit,
                        "The index exceeds the partial-selection size limit.",
                    ));
                }
                private.write_all(&bytes).map_err(fingerprint_io_error)?;
            }
            // Git accepts a missing alternate index, unlike a zero-byte one.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                drop(private);
                fs::remove_file(&transaction.private_path).map_err(fingerprint_io_error)?;
            }
            Err(error) => return Err(fingerprint_io_error(error)),
        }
        Ok(transaction)
    }

    fn publish(mut self) -> Result<(), GitError> {
        // Git has closed and atomically replaced its private index. Close our
        // sync handle before MoveFileEx/rename, including on Windows.
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.private_path)
            .map_err(fingerprint_io_error)?;
        file.sync_all().map_err(fingerprint_io_error)?;
        drop(file);
        fs::rename(&self.private_path, &self.lock_path).map_err(fingerprint_io_error)?;
        fs::rename(&self.lock_path, &self.index_path).map_err(fingerprint_io_error)?;
        self.published = true;
        Ok(())
    }
}

impl Drop for IndexTransaction {
    fn drop(&mut self) {
        // These names were allocated only for this operation; an existing real
        // index.lock is never removed when acquisition failed.
        if self.owns_private {
            let _ = fs::remove_file(&self.private_path);
            let mut private_lock = self.private_path.as_os_str().to_owned();
            private_lock.push(".lock");
            let _ = fs::remove_file(PathBuf::from(private_lock));
        }
        if !self.published {
            let _ = fs::remove_file(&self.lock_path);
        }
    }
}

fn stale_selection() -> GitError {
    GitError::new(GitErrorKind::StaleState, "The file, displayed diff, or repository state changed. Refresh and select the lines again.")
}

fn unsupported(message: impl Into<String>) -> GitError {
    GitError::new(GitErrorKind::Unsupported, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::MUTATION_TEST_LOCK;

    struct Fixture {
        path: PathBuf,
        git: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let id = NEXT_INDEX.fetch_add(1, Ordering::Relaxed);
            let fixture = Self {
                path: std::env::temp_dir().join(format!(
                    "enogram-partial-fixture-{}-{id}",
                    std::process::id()
                )),
                git: resolve_git().unwrap(),
            };
            fs::create_dir(&fixture.path).unwrap();
            fixture.git(&["init", "-b", "main"]);
            for (key, value) in [
                ("user.name", "Enogram Partial Test"),
                ("user.email", "partial@example.invalid"),
                ("commit.gpgsign", "false"),
                ("core.autocrlf", "false"),
                ("core.filemode", "false"),
            ] {
                fixture.git(&["config", key, value]);
            }
            fs::create_dir(fixture.path.join(".git/test-hooks")).unwrap();
            fs::write(fixture.path.join(".git/test-attributes"), b"").unwrap();
            fixture.git(&["config", "core.hooksPath", ".git/test-hooks"]);
            fixture.git(&["config", "core.attributesFile", ".git/test-attributes"]);
            fixture
        }

        fn path_str(&self) -> &str {
            self.path.to_str().unwrap()
        }

        fn command(&self, args: &[&str]) -> std::process::Output {
            Command::new(&self.git)
                .args(args)
                .current_dir(&self.path)
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_AUTHOR_NAME", "Enogram Partial Test")
                .env("GIT_AUTHOR_EMAIL", "partial@example.invalid")
                .env("GIT_COMMITTER_NAME", "Enogram Partial Test")
                .env("GIT_COMMITTER_EMAIL", "partial@example.invalid")
                .output()
                .unwrap()
        }

        fn git(&self, args: &[&str]) -> Vec<u8> {
            let output = self.command(args);
            assert!(
                output.status.success(),
                "{:?}: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
            output.stdout
        }

        fn write(&self, path: &str, bytes: impl AsRef<[u8]>) {
            fs::write(self.path.join(path), bytes).unwrap();
        }

        fn commit(&self) {
            self.git(&["add", "--all"]);
            self.git(&["commit", "-m", "base"]);
        }

        fn token(&self) -> String {
            read_mutation_token(&repository_context(self.path_str()).unwrap()).unwrap()
        }

        fn prepare(&self, path: &str, staged: bool) -> (String, PartialDiffResult) {
            let token = self.token();
            let prepared = prepare_partial_diff(self.path_str(), path, staged, &token).unwrap();
            assert!(
                prepared.unsupported_reason.is_none(),
                "{}\n{}",
                prepared.unsupported_reason.clone().unwrap_or_default(),
                prepared.text
            );
            assert!(prepared.selection_token.is_some());
            (token, prepared)
        }

        fn apply(
            &self,
            path: &str,
            staged: bool,
            token: &str,
            prepared: &PartialDiffResult,
            ids: &[usize],
        ) -> Result<MutationOutcome, GitError> {
            apply_partial_selection(
                self.path_str(),
                path,
                staged,
                token,
                prepared.selection_token.as_deref().unwrap(),
                ids,
            )
        }

        fn select(&self, path: &str, staged: bool, lines: &[&str]) {
            let (token, prepared) = self.prepare(path, staged);
            let ids = lines
                .iter()
                .map(|expected| {
                    let matches = prepared
                        .text
                        .split('\n')
                        .enumerate()
                        .filter(|(id, line)| {
                            line == expected && prepared.selectable_line_ids.contains(id)
                        })
                        .map(|(id, _)| id)
                        .collect::<Vec<_>>();
                    assert_eq!(
                        matches.len(),
                        1,
                        "line {expected:?} is not unique in {}",
                        prepared.text
                    );
                    matches[0]
                })
                .collect::<Vec<_>>();
            self.apply(path, staged, &token, &prepared, &ids).unwrap();
            self.assert_no_artifacts();
        }

        fn select_all(&self, path: &str, staged: bool) {
            let (token, prepared) = self.prepare(path, staged);
            self.apply(
                path,
                staged,
                &token,
                &prepared,
                &prepared.selectable_line_ids,
            )
            .unwrap();
            self.assert_no_artifacts();
        }

        fn index(&self, path: &str) -> Option<Vec<u8>> {
            read_index_entry(&repository_context(self.path_str()).unwrap(), path)
                .unwrap()
                .map(|entry| entry.bytes)
        }

        fn assert_no_artifacts(&self) {
            assert!(!self.path.join(".git/index.lock").exists());
            assert!(!fs::read_dir(self.path.join(".git"))
                .unwrap()
                .any(|entry| entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("enogram-partial-")));
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn partial_replacements_and_inverse_preserve_worktree_and_other_staged_entries() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fixture = Fixture::new();
        fixture.write("file.txt", b"start\nold\nend\n");
        fixture.write("other.txt", b"base\n");
        fixture.commit();
        fixture.write("other.txt", b"already staged\n");
        fixture.git(&["add", "other.txt"]);
        let other_entry = fixture.git(&["ls-files", "--stage", "--", "other.txt"]);
        let worktree = b"start\nnew\nextra\nend\n";
        fixture.write("file.txt", worktree);
        fixture.select("file.txt", false, &["-old", "+new"]);
        assert_eq!(fixture.index("file.txt").unwrap(), b"start\nnew\nend\n");
        assert_eq!(fs::read(fixture.path.join("file.txt")).unwrap(), worktree);
        assert_eq!(
            fixture.git(&["ls-files", "--stage", "--", "other.txt"]),
            other_entry
        );
        fixture.select("file.txt", true, &["+new"]);
        assert_eq!(fixture.index("file.txt").unwrap(), b"start\nend\n");
        fixture.select("file.txt", true, &["-old"]);
        assert_eq!(fixture.index("file.txt").unwrap(), b"start\nold\nend\n");
        assert_eq!(fs::read(fixture.path.join("file.txt")).unwrap(), worktree);
        assert_eq!(
            fixture.git(&["ls-files", "--stage", "--", "other.txt"]),
            other_entry
        );
    }

    #[test]
    fn selected_hunk_leaves_other_hunk_unstaged_with_exact_line_ordinals() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fixture = Fixture::new();
        let base = (1..=30)
            .map(|line| format!("line {line}\n"))
            .collect::<String>();
        fixture.write("file.txt", &base);
        fixture.commit();
        let worktree = base
            .replace("line 2\n", "changed 2\n")
            .replace("line 28\n", "changed 28\n");
        fixture.write("file.txt", &worktree);
        let (token, prepared) = fixture.prepare("file.txt", false);
        assert_eq!(
            prepared
                .text
                .lines()
                .filter(|line| line.starts_with("@@"))
                .count(),
            2
        );
        let second_hunk = prepared
            .text
            .split('\n')
            .enumerate()
            .filter(|(_, line)| line.starts_with("@@"))
            .nth(1)
            .unwrap()
            .0;
        let first_ids = prepared
            .selectable_line_ids
            .iter()
            .copied()
            .filter(|id| *id < second_hunk)
            .collect::<Vec<_>>();
        fixture
            .apply("file.txt", false, &token, &prepared, &first_ids)
            .unwrap();
        assert_eq!(
            fixture.index("file.txt").unwrap(),
            base.replace("line 2\n", "changed 2\n").as_bytes()
        );
        assert_eq!(
            fs::read(fixture.path.join("file.txt")).unwrap(),
            worktree.as_bytes()
        );
        fixture.assert_no_artifacts();
    }

    #[test]
    fn unborn_new_files_can_be_partially_staged_and_unstaged_to_absence() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fixture = Fixture::new();
        fixture.write("new.txt", b"one\ntwo\nthree\n");
        assert!(!fixture.path.join(".git/index").exists());
        fixture.select("new.txt", false, &["+one", "+three"]);
        assert_eq!(fixture.index("new.txt").unwrap(), b"one\nthree\n");
        fixture.select("new.txt", true, &["+one"]);
        assert_eq!(fixture.index("new.txt").unwrap(), b"three\n");
        fixture.select_all("new.txt", true);
        assert_eq!(fixture.index("new.txt"), None);
        assert_eq!(
            fs::read(fixture.path.join("new.txt")).unwrap(),
            b"one\ntwo\nthree\n"
        );
    }

    #[test]
    fn deleted_files_restore_only_selected_lines_without_recreating_worktree() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fixture = Fixture::new();
        fixture.write("file.txt", b"one\ntwo\nthree\n");
        fixture.commit();
        fs::remove_file(fixture.path.join("file.txt")).unwrap();
        fixture.select("file.txt", false, &["-two"]);
        assert_eq!(fixture.index("file.txt").unwrap(), b"one\nthree\n");
        assert!(!fixture.path.join("file.txt").exists());
        fixture.select_all("file.txt", false);
        assert_eq!(fixture.index("file.txt"), None);
        fixture.select("file.txt", true, &["-two"]);
        assert_eq!(fixture.index("file.txt").unwrap(), b"two\n");
        fixture.select_all("file.txt", true);
        assert_eq!(fixture.index("file.txt").unwrap(), b"one\ntwo\nthree\n");
        assert!(!fixture.path.join("file.txt").exists());
    }

    #[test]
    fn deleting_all_text_keeps_an_existing_empty_file_entry() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fixture = Fixture::new();
        fixture.write("file.txt", b"one\n");
        fixture.commit();
        fixture.write("file.txt", b"");
        fixture.select_all("file.txt", false);
        assert_eq!(fixture.index("file.txt"), Some(Vec::new()));
        fixture.select_all("file.txt", true);
        assert_eq!(fixture.index("file.txt").unwrap(), b"one\n");
        assert_eq!(fs::read(fixture.path.join("file.txt")).unwrap(), b"");
    }

    #[test]
    fn canonical_git_line_endings_and_literal_unicode_paths_round_trip() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fixture = Fixture::new();
        fixture.git(&["config", "core.autocrlf", "true"]);
        fixture.git(&["config", "apply.whitespace", "fix"]);
        fixture.git(&["config", "apply.ignoreWhitespace", "change"]);
        fixture.git(&["config", "diff.noprefix", "true"]);
        fixture.git(&["config", "diff.mnemonicPrefix", "true"]);
        fixture.git(&["config", "diff.context", "0"]);
        let path = "--çay [literal] ö.txt";
        let worktree = "bir\r\niki  \r\nüç\r\n".as_bytes();
        fixture.write(path, worktree);
        fixture.select(path, false, &["+iki  "]);
        assert_eq!(fixture.index(path).unwrap(), "iki  \n".as_bytes());
        assert_eq!(fs::read(fixture.path.join(path)).unwrap(), worktree);
        fixture.select_all(path, true);
        assert_eq!(fixture.index(path), None);
        fixture.write(".gitattributes", b"*.txt -text\n");
        fixture.select(path, false, &["+iki  \r"]);
        assert_eq!(fixture.index(path).unwrap(), "iki  \r\n".as_bytes());
        assert_eq!(fs::read(fixture.path.join(path)).unwrap(), worktree);
        fixture.select_all(path, true);
        fixture.write(".gitattributes", b"*.txt text eol=lf\n");
        fixture.select(path, false, &["+bir"]);
        assert_eq!(fixture.index(path).unwrap(), b"bir\n");
        assert_eq!(fs::read(fixture.path.join(path)).unwrap(), worktree);
    }

    #[test]
    fn no_newline_replacements_preserve_eof_and_unsafe_subsets_are_rejected() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fixture = Fixture::new();
        fixture.write("file.txt", b"one");
        fixture.commit();
        fixture.write("file.txt", b"one\nextra\n");
        let (token, prepared) = fixture.prepare("file.txt", false);
        let id = prepared
            .text
            .split('\n')
            .position(|line| line == "+extra")
            .unwrap();
        let before = fs::read(fixture.path.join(".git/index")).unwrap();
        let error = fixture
            .apply("file.txt", false, &token, &prepared, &[id])
            .unwrap_err();
        assert_eq!(error.kind, GitErrorKind::Unsupported);
        assert!(error.message.contains("without a final newline"));
        assert_eq!(fs::read(fixture.path.join(".git/index")).unwrap(), before);
        fixture.assert_no_artifacts();
        fixture.select_all("file.txt", false);
        assert_eq!(fixture.index("file.txt").unwrap(), b"one\nextra\n");
        fixture.select_all("file.txt", true);
        assert_eq!(fixture.index("file.txt").unwrap(), b"one");
        fixture.write("file.txt", b"two");
        fixture.select_all("file.txt", false);
        assert_eq!(fixture.index("file.txt").unwrap(), b"two");
        fixture.select_all("file.txt", true);
        assert_eq!(fixture.index("file.txt").unwrap(), b"one");
        assert_eq!(fs::read(fixture.path.join("file.txt")).unwrap(), b"two");
    }

    #[test]
    fn stale_worktree_snapshot_layer_and_invalid_line_ids_never_write() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fixture = Fixture::new();
        fixture.write("file.txt", b"base\n");
        fixture.commit();
        fixture.write("file.txt", b"new\n");
        let (token, prepared) = fixture.prepare("file.txt", false);
        let before = fs::read(fixture.path.join(".git/index")).unwrap();
        for ids in [
            vec![],
            vec![0],
            vec![usize::MAX],
            vec![prepared.selectable_line_ids[0]; 2],
        ] {
            assert_eq!(
                fixture
                    .apply("file.txt", false, &token, &prepared, &ids)
                    .unwrap_err()
                    .kind,
                GitErrorKind::InvalidMutationPath
            );
        }
        fixture.write("file.txt", b"newer\n");
        assert_eq!(
            fixture.token(),
            token,
            "ordinary token deliberately excludes worktree bytes"
        );
        assert_eq!(
            fixture
                .apply(
                    "file.txt",
                    false,
                    &token,
                    &prepared,
                    &prepared.selectable_line_ids
                )
                .unwrap_err()
                .kind,
            GitErrorKind::StaleState
        );
        fixture.write("file.txt", b"new\n");
        fixture.git(&["add", "file.txt"]);
        let staged_index = fs::read(fixture.path.join(".git/index")).unwrap();
        assert_eq!(
            fixture
                .apply(
                    "file.txt",
                    false,
                    &token,
                    &prepared,
                    &prepared.selectable_line_ids
                )
                .unwrap_err()
                .kind,
            GitErrorKind::StaleState
        );
        let (new_token, staged) = fixture.prepare("file.txt", true);
        assert_eq!(
            apply_partial_selection(
                fixture.path_str(),
                "file.txt",
                true,
                &new_token,
                prepared.selection_token.as_deref().unwrap(),
                &staged.selectable_line_ids
            )
            .unwrap_err()
            .kind,
            GitErrorKind::StaleState
        );
        assert_eq!(
            fs::read(fixture.path.join(".git/index")).unwrap(),
            staged_index
        );
        assert_ne!(staged_index, before);
        fixture.assert_no_artifacts();
    }

    #[test]
    fn worktree_only_crlf_drift_invalidates_selection_even_when_canonical_diff_is_identical() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fixture = Fixture::new();
        fixture.git(&["config", "core.autocrlf", "true"]);
        fixture.write("file.txt", b"base\r\n");
        fixture.commit();
        fixture.write("file.txt", b"new\r\n");
        let (token, first) = fixture.prepare("file.txt", false);
        fixture.write("file.txt", b"new\n");
        let (_, second) = fixture.prepare("file.txt", false);
        assert_eq!(first.text, second.text);
        assert_ne!(first.selection_token, second.selection_token);
        assert_eq!(
            fixture
                .apply(
                    "file.txt",
                    false,
                    &token,
                    &first,
                    &first.selectable_line_ids
                )
                .unwrap_err()
                .kind,
            GitErrorKind::StaleState
        );
        assert_eq!(fixture.index("file.txt").unwrap(), b"base\n");
        fixture.assert_no_artifacts();
    }

    #[test]
    fn native_index_lock_blocks_git_and_preserves_existing_foreign_locks() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fixture = Fixture::new();
        fixture.write("file.txt", b"base\n");
        fixture.commit();
        fixture.write("file.txt", b"new\n");
        let (token, prepared) = fixture.prepare("file.txt", false);
        let before = fs::read(fixture.path.join(".git/index")).unwrap();
        let context = repository_context(fixture.path_str()).unwrap();
        let transaction = IndexTransaction::begin(&context).unwrap();
        let competing = fixture.command(&["add", "file.txt"]);
        assert!(!competing.status.success());
        assert!(String::from_utf8_lossy(&competing.stderr).contains("index.lock"));
        assert_eq!(fs::read(fixture.path.join(".git/index")).unwrap(), before);
        drop(transaction);
        fixture.assert_no_artifacts();
        fixture.write(".git/index.lock", b"foreign lock");
        assert_eq!(
            fixture
                .apply(
                    "file.txt",
                    false,
                    &token,
                    &prepared,
                    &prepared.selectable_line_ids
                )
                .unwrap_err()
                .kind,
            GitErrorKind::Busy
        );
        assert_eq!(
            fs::read(fixture.path.join(".git/index.lock")).unwrap(),
            b"foreign lock"
        );
        assert_eq!(fs::read(fixture.path.join(".git/index")).unwrap(), before);
        fs::remove_file(fixture.path.join(".git/index.lock")).unwrap();
    }

    #[test]
    fn failed_private_apply_leaves_original_index_and_all_worktree_bytes_unchanged() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fixture = Fixture::new();
        fixture.write("file.txt", b"base\n");
        fixture.commit();
        fixture.write("file.txt", b"changed\n");
        let context = repository_context(fixture.path_str()).unwrap();
        let before = fs::read(fixture.path.join(".git/index")).unwrap();
        let transaction = IndexTransaction::begin(&context).unwrap();
        let output = partial_git(&context, ["apply", "--cached", "-"], 4096, false,
            Some(b"diff --git a/file.txt b/file.txt\n--- a/file.txt\n+++ b/file.txt\n@@ -1 +1 @@\n-not the source\n+bad\n"), Some(&transaction.private_path)).unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("patch failed"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        drop(transaction);
        assert_eq!(fs::read(fixture.path.join(".git/index")).unwrap(), before);
        assert_eq!(
            fs::read(fixture.path.join("file.txt")).unwrap(),
            b"changed\n"
        );
        fixture.assert_no_artifacts();
    }

    #[test]
    fn split_index_preserves_an_unrelated_staged_blob() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fixture = Fixture::new();
        fixture.write("file.txt", b"base\n");
        fixture.write("other.txt", b"before\n");
        fixture.commit();
        fixture.write("other.txt", b"after\n");
        fixture.git(&["add", "other.txt"]);
        fixture.git(&["update-index", "--split-index"]);
        let other = fixture.git(&["ls-files", "--stage", "--", "other.txt"]);
        fixture.write("file.txt", b"new\nextra\n");
        fixture.select("file.txt", false, &["-base", "+new"]);
        assert_eq!(fixture.index("file.txt").unwrap(), b"new\n");
        assert_eq!(
            fixture.git(&["ls-files", "--stage", "--", "other.txt"]),
            other
        );
        assert_eq!(
            fs::read(fixture.path.join("file.txt")).unwrap(),
            b"new\nextra\n"
        );
    }

    #[test]
    fn binary_invalid_utf8_attributes_modes_and_empty_files_are_display_only() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fixture = Fixture::new();
        for (path, bytes, needle) in [
            ("binary.bin", b"a\0b".as_slice(), "Binary"),
            ("invalid.txt", b"a\xff\n".as_slice(), "UTF-8"),
            ("empty.txt", b"".as_slice(), "no selectable"),
        ] {
            fixture.write(path, bytes);
            let result =
                prepare_partial_diff(fixture.path_str(), path, false, &fixture.token()).unwrap();
            assert!(result.selection_token.is_none());
            assert!(result.selectable_line_ids.is_empty());
            let reason = result.unsupported_reason.unwrap();
            assert!(reason.contains(needle), "{path}: {reason}");
        }
        fixture.write("converted.txt", b"$Id$\n");
        fixture.write(".gitattributes", b"converted.txt ident\n");
        let result =
            prepare_partial_diff(fixture.path_str(), "converted.txt", false, &fixture.token())
                .unwrap();
        assert!(result.selection_token.is_none());
        assert!(result.unsupported_reason.unwrap().contains("conversion"));
        assert!(
            result.text.contains("+$Id$"),
            "unsupported writes retain diff viewing"
        );
        fixture.git(&["add", "converted.txt"]);
        fixture.git(&["commit", "-m", "base"]);
        fixture.git(&["update-index", "--chmod=+x", "converted.txt"]);
        fixture.write(".gitattributes", b"");
        let result =
            prepare_partial_diff(fixture.path_str(), "converted.txt", true, &fixture.token())
                .unwrap();
        assert!(result.selection_token.is_none());
        assert!(result.unsupported_reason.unwrap().contains("mode"));
        fixture.assert_no_artifacts();
    }

    #[test]
    fn renames_symlinks_submodules_conflicts_and_traversal_cannot_receive_write_tokens() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fixture = Fixture::new();
        fixture.write("file.txt", b"one\ntwo\n");
        fixture.commit();
        fixture.git(&["mv", "file.txt", "renamed.txt"]);
        let rename =
            prepare_partial_diff(fixture.path_str(), "renamed.txt", true, &fixture.token())
                .unwrap();
        assert!(rename.selection_token.is_none());
        assert!(rename.unsupported_reason.unwrap().contains("Renames"));
        let oid = String::from_utf8(fixture.git(&["rev-parse", "HEAD:file.txt"]))
            .unwrap()
            .trim()
            .to_owned();
        fixture.git(&[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("120000,{oid},link"),
        ]);
        let link =
            prepare_partial_diff(fixture.path_str(), "link", true, &fixture.token()).unwrap();
        assert!(link.selection_token.is_none());
        assert!(link.unsupported_reason.is_some());
        let head = String::from_utf8(fixture.git(&["rev-parse", "HEAD"]))
            .unwrap()
            .trim()
            .to_owned();
        fixture.git(&[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{head},submodule"),
        ]);
        let submodule =
            prepare_partial_diff(fixture.path_str(), "submodule", true, &fixture.token()).unwrap();
        assert!(submodule.selection_token.is_none());
        assert!(submodule.unsupported_reason.is_some());
        fixture.write("conflict.txt", b"one\ntwo\n");
        let context = repository_context(fixture.path_str()).unwrap();
        let input = (1..=3)
            .map(|stage| format!("100644 {oid} {stage}\tconflict.txt\0"))
            .collect::<String>();
        let output = run_git_with_input(
            &context.git,
            &context.root,
            ["update-index", "-z", "--index-info"],
            4096,
            false,
            Some(input.as_bytes()),
        )
        .unwrap();
        ensure_success(&output, "create the conflict fixture").unwrap();
        let token = fixture.token();
        let conflict =
            prepare_partial_diff(fixture.path_str(), "conflict.txt", false, &token).unwrap();
        assert!(conflict.selection_token.is_none());
        assert!(conflict.unsupported_reason.unwrap().contains("conflicts"));
        let before = fs::read(fixture.path.join(".git/index")).unwrap();
        for path in [
            "../outside",
            ".git/index",
            "renamed.txt/../file.txt",
            ":(glob)*",
        ] {
            assert!(prepare_partial_diff(fixture.path_str(), path, false, &token).is_err());
        }
        assert_eq!(fs::read(fixture.path.join(".git/index")).unwrap(), before);
        fixture.assert_no_artifacts();
    }

    #[test]
    fn large_index_blobs_keep_readable_diffs_and_truncated_diffs_are_display_only() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fixture = Fixture::new();
        let mut bytes = b"x\n".repeat(MAX_WORKTREE_BYTES / 2 + 1);
        fixture.write("large.txt", &bytes);
        fixture.commit();
        let last = bytes.len() - 2;
        bytes[last] = b'y';
        fixture.write("large.txt", &bytes);
        fixture.git(&["add", "large.txt"]);
        fs::remove_file(fixture.path.join("large.txt")).unwrap();
        let result =
            prepare_partial_diff(fixture.path_str(), "large.txt", true, &fixture.token()).unwrap();
        assert!(!result.truncated);
        assert!(result.text.contains("+y\n"));
        assert!(result.selection_token.is_none());
        assert!(result.unsupported_reason.unwrap().contains("limit"));
        fixture.write("new-large.txt", &bytes);
        let result =
            prepare_partial_diff(fixture.path_str(), "new-large.txt", false, &fixture.token())
                .unwrap();
        assert!(result.truncated);
        assert!(!result.text.is_empty());
        assert!(result.selection_token.is_none());
        assert!(result.selectable_line_ids.is_empty());
        assert!(result.unsupported_reason.unwrap().contains("truncated"));
        fixture.assert_no_artifacts();
    }

    #[test]
    fn staged_deletion_and_untracked_same_path_use_the_requested_layer() {
        let _guard = MUTATION_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fixture = Fixture::new();
        fixture.write("file.txt", b"one\ntwo\n");
        fixture.commit();
        fixture.git(&["rm", "--cached", "file.txt"]);
        let (_, _, _, changes) =
            read_status(&repository_context(fixture.path_str()).unwrap()).unwrap();
        assert_eq!(
            changes
                .iter()
                .filter(|change| change.path == "file.txt")
                .count(),
            2
        );
        let (_, staged) = fixture.prepare("file.txt", true);
        assert!(staged.text.contains("-one"));
        let (_, working) = fixture.prepare("file.txt", false);
        assert!(working.text.contains("+one"));
        fixture.select("file.txt", false, &["+two"]);
        assert_eq!(fixture.index("file.txt").unwrap(), b"two\n");
        assert_eq!(
            fs::read(fixture.path.join("file.txt")).unwrap(),
            b"one\ntwo\n"
        );
    }
}
