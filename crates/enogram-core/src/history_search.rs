//! Bounded, read-only search over locally reachable commit history. Cursors
//! describe consumed commits, not matching rows; an empty page can be partial.

use super::*;
use std::collections::BTreeMap;

const SEARCH_SCAN_LIMIT: usize = 500;
const SEARCH_MATCH_LIMIT: usize = 100;
const MAX_SEARCH_BYTES: usize = 8 * 1024 * 1024;
const MAX_QUERY_BYTES: usize = 1024;
const MAX_CURSOR_OFFSET: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistorySearchCursor {
    pub history_token: String,
    /// Trimmed, Unicode-lowercased query. Callers must return this unchanged.
    pub query: String,
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySearchPage {
    pub commits: Vec<Commit>,
    /// Cumulative number of commits examined, not a total-history estimate.
    pub scanned_count: u64,
    pub next_cursor: Option<HistorySearchCursor>,
    pub complete: bool,
}

struct HistoryRef {
    name: String,
    oid: String,
}

struct HistoryState {
    token: String,
    refs: Vec<HistoryRef>,
    head: Option<String>,
    symbolic_head: Option<String>,
}

struct SearchRecord {
    commit: Commit,
    message: String,
    email: String,
}

/// Search messages (including bodies), raw author names/emails, full object
/// IDs, and ref names using literal Unicode lowercase substring matching.
/// Only local refs and HEAD are visited; this never fetches, changes the index,
/// or writes refs. Every call consumes at most 500 commits and returns at most
/// 100 matches. Continue even after an empty page unless `complete` is true.
pub fn search_repository_history(
    repo_path: &str,
    query: &str,
    cursor: Option<&HistorySearchCursor>,
) -> Result<HistorySearchPage, GitError> {
    let query = normalize_query(query)?;
    let offset = validate_cursor(cursor, &query)?;
    let context = repository_context(repo_path)?;
    ensure_offline_search_supported(&context)?;
    let state = read_history_state(&context)?;
    if cursor.is_some_and(|cursor| cursor.history_token != state.token) {
        return Err(stale_history());
    }
    let (roots, decorations, full_refs) = pinned_commit_roots(&context, &state)?;
    let records = if roots.is_empty() {
        Vec::new()
    } else {
        read_search_batch(&context, &roots, offset)?
    };
    if offset > 0 && records.is_empty() {
        return Err(invalid_search(
            "History search cursor is past the end of this history. Restart the search.",
        ));
    }

    let available = records.len();
    let mut consumed = 0;
    let mut commits = Vec::new();
    for mut record in records.into_iter().take(SEARCH_SCAN_LIMIT) {
        consumed += 1;
        record.commit.refs = decorations
            .get(&record.commit.oid)
            .cloned()
            .unwrap_or_default();
        let reference_names = full_refs
            .get(&record.commit.oid)
            .map(String::as_str)
            .unwrap_or("");
        if [
            record.message.as_str(),
            record.commit.subject.as_str(),
            record.commit.author.as_str(),
            record.email.as_str(),
            record.commit.oid.as_str(),
            record.commit.refs.as_str(),
            reference_names,
        ]
        .iter()
        .any(|value| value.to_lowercase().contains(&query))
        {
            commits.push(record.commit);
            if commits.len() == SEARCH_MATCH_LIMIT {
                break;
            }
        }
    }

    // Pinned OIDs keep a moving branch from silently changing the traversal.
    // Rechecking also invalidates decorations and shallow/graft boundaries.
    // As with snapshots, this detects ordinary concurrent changes, not an ABA
    // change-and-restore performed entirely between the two observations.
    if state.token != read_history_state(&context)?.token {
        return Err(stale_history());
    }
    let scanned_count = offset
        .checked_add(consumed as u64)
        .filter(|value| *value <= MAX_CURSOR_OFFSET)
        .ok_or_else(|| invalid_search("History search cursor exceeds the supported range."))?;
    let complete = consumed == available;
    let next_cursor = (!complete).then(|| HistorySearchCursor {
        history_token: state.token,
        query,
        offset: scanned_count,
    });
    Ok(HistorySearchPage {
        commits,
        scanned_count,
        next_cursor,
        complete,
    })
}

fn normalize_query(query: &str) -> Result<String, GitError> {
    if query.len() > MAX_QUERY_BYTES || query.contains('\0') || query.trim().is_empty() {
        return Err(invalid_search(
            "Enter non-whitespace search text without NUL, at most 1024 UTF-8 bytes.",
        ));
    }
    Ok(query.trim().to_lowercase())
}

fn validate_cursor(cursor: Option<&HistorySearchCursor>, query: &str) -> Result<u64, GitError> {
    let Some(cursor) = cursor else { return Ok(0) };
    if cursor.query != query
        || cursor.offset == 0
        || cursor.offset > MAX_CURSOR_OFFSET
        || !cursor
            .history_token
            .strip_prefix("history-v1:")
            .is_some_and(full_oid)
    {
        return Err(invalid_search(
            "Invalid history search cursor or changed query. Restart the search.",
        ));
    }
    Ok(cursor.offset)
}

fn invalid_search(message: &str) -> GitError {
    GitError::new(GitErrorKind::InvalidHistorySearch, message)
}

fn stale_history() -> GitError {
    GitError::new(GitErrorKind::StaleState, "Repository history changed during this search. Restart the search to use the current branches and tags.")
}

fn full_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn run_search_git<I, S>(
    context: &RepositoryContext,
    args: I,
    limit: usize,
    input: Option<&[u8]>,
) -> Result<CapturedOutput, GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = git_command(&context.git, &context.root, args);
    // A read in a partial clone must report unavailable objects, not fetch them
    // through a promisor remote as a hidden side effect of searching.
    command
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_ALLOW_PROTOCOL", "");
    git_process::run(&mut command, limit, false, input, COMMAND_TIMEOUT)
}

fn ensure_offline_search_supported(context: &RepositoryContext) -> Result<(), GitError> {
    let config = run_search_git(
        context,
        [
            "config",
            "--null",
            "--get-regexp",
            r"^(extensions\.partialclone|remote\..*\.promisor)$",
        ],
        MAX_SEARCH_BYTES,
        None,
    )?;
    if !config.status.success() && config.status.code() != Some(1) {
        ensure_success(&config, "check offline history search support")?;
    }
    let has_promisor = output_text(&config.stdout, "partial clone configuration")?
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            let (key, value) = entry.split_once('\n').unwrap_or((entry, ""));
            if key == "extensions.partialclone" {
                return !value.is_empty();
            }
            !matches!(
                value.to_ascii_lowercase().as_str(),
                "false" | "off" | "no" | "0"
            )
        });
    if !has_promisor {
        return Ok(());
    }
    // Older Git silently ignores GIT_NO_LAZY_FETCH. Probe its command-line
    // equivalent without reading objects before permitting a promisor clone.
    // The empty transport whitelist additionally blocks every protocol on old
    // Git, including when configuration changes after this preflight check.
    let support = run_search_git(context, ["--no-lazy-fetch", "--version"], 4096, None)?;
    if !support.status.success() {
        return Err(GitError::new(GitErrorKind::Unsupported,
            "This Git version cannot search a partial/promisor clone without implicit fetching. Update Git to a version supporting --no-lazy-fetch, or use a complete local clone."));
    }
    Ok(())
}

fn read_history_state(context: &RepositoryContext) -> Result<HistoryState, GitError> {
    let refs_output = run_search_git(
        context,
        [
            "for-each-ref",
            "--sort=refname",
            "--format=%(refname)%00%(objectname)%00%(symref)%00",
        ],
        MAX_SEARCH_BYTES,
        None,
    )?;
    ensure_success(&refs_output, "read references for history search")?;
    let mut refs = Vec::new();
    for line in refs_output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let fields: Vec<_> = line.split(|byte| *byte == 0).collect();
        if fields.len() != 4 || !fields[3].is_empty() {
            return Err(invalid_output(
                "Git returned an invalid history reference record.",
            ));
        }
        let name = utf8(fields[0], "history ref name")?;
        let oid = utf8(fields[1], "history ref object ID")?;
        if !name.starts_with("refs/") || !full_oid(oid) {
            return Err(invalid_output("Git returned an invalid history reference."));
        }
        refs.push(HistoryRef {
            name: name.to_owned(),
            oid: oid.to_owned(),
        });
    }
    let head_output = run_search_git(
        context,
        [
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            "HEAD^{commit}",
        ],
        4096,
        None,
    )?;
    if !head_output.status.success() && head_output.status.code() != Some(1) {
        ensure_success(&head_output, "read HEAD for history search")?;
    }
    let head = if head_output.status.success() {
        let head = output_text(&head_output.stdout, "history HEAD")?.trim();
        if !full_oid(head) {
            return Err(invalid_output("Git returned an invalid history HEAD."));
        }
        Some(head.to_owned())
    } else {
        None
    };
    let common_output =
        run_search_git(context, ["rev-parse", "--git-common-dir"], 64 * 1024, None)?;
    ensure_success(&common_output, "locate shared history metadata")?;
    let common = output_text(&common_output.stdout, "shared history directory")?
        .trim_end_matches(['\r', '\n']);
    let common = fs::canonicalize(context.root.join(common)).map_err(fingerprint_io_error)?;

    let mut fingerprint = b"enogram-history-v1\0".to_vec();
    append_fingerprint_field(&mut fingerprint, path_to_string(&context.root)?.as_bytes());
    append_fingerprint_field(
        &mut fingerprint,
        path_to_string(&context.git_dir)?.as_bytes(),
    );
    append_fingerprint_field(&mut fingerprint, path_to_string(&common)?.as_bytes());
    append_fingerprint_field(&mut fingerprint, &refs_output.stdout);
    append_fingerprint_field(&mut fingerprint, &head_output.stdout);
    append_optional_file(&mut fingerprint, &context.git_dir.join("HEAD"), 64 * 1024)?;
    append_optional_file(&mut fingerprint, &common.join("shallow"), MAX_SEARCH_BYTES)?;
    append_optional_file(
        &mut fingerprint,
        &common.join("info/grafts"),
        MAX_SEARCH_BYTES,
    )?;
    let symbolic_output = run_search_git(
        context,
        ["symbolic-ref", "--quiet", "HEAD"],
        64 * 1024,
        None,
    )?;
    if !symbolic_output.status.success() && symbolic_output.status.code() != Some(1) {
        ensure_success(&symbolic_output, "read symbolic HEAD for history search")?;
    }
    append_fingerprint_field(&mut fingerprint, &symbolic_output.stdout);
    let symbolic_head = symbolic_output
        .status
        .success()
        .then(|| {
            output_text(&symbolic_output.stdout, "symbolic history HEAD")
                .map(|value| value.trim_end_matches(['\r', '\n']).to_owned())
        })
        .transpose()?;
    let hash = run_search_git(
        context,
        ["hash-object", "--no-filters", "--stdin"],
        4096,
        Some(&fingerprint),
    )?;
    ensure_success(&hash, "fingerprint history without writing objects")?;
    let hash = output_text(&hash.stdout, "history fingerprint")?.trim();
    if !full_oid(hash) {
        return Err(invalid_output(
            "Git returned an invalid history fingerprint.",
        ));
    }
    Ok(HistoryState {
        token: format!("history-v1:{hash}"),
        refs,
        head,
        symbolic_head,
    })
}

fn pinned_commit_roots(
    context: &RepositoryContext,
    state: &HistoryState,
) -> Result<
    (
        Vec<String>,
        BTreeMap<String, String>,
        BTreeMap<String, String>,
    ),
    GitError,
> {
    let object_ids: BTreeSet<_> = state
        .refs
        .iter()
        .map(|reference| reference.oid.clone())
        .chain(state.head.iter().cloned())
        .collect();
    if object_ids.is_empty() {
        return Ok((Vec::new(), BTreeMap::new(), BTreeMap::new()));
    }
    // ^{} recursively peels annotated tags. Non-commit tag/ref targets are
    // legal Git objects, but cannot be roots of commit-history traversal.
    let input = object_ids
        .iter()
        .map(|oid| format!("{oid}^{{}}\n"))
        .collect::<String>();
    let output = run_search_git(
        context,
        ["cat-file", "--batch-check=%(objectname) %(objecttype)"],
        MAX_SEARCH_BYTES,
        Some(input.as_bytes()),
    )?;
    ensure_success(&output, "resolve pinned history roots")?;
    let lines: Vec<_> = output_text(&output.stdout, "peeled history roots")?
        .lines()
        .collect();
    if lines.len() != object_ids.len() {
        return Err(invalid_output(
            "Git returned an incomplete list of history roots.",
        ));
    }
    let mut peeled = BTreeMap::new();
    for (oid, line) in object_ids.iter().zip(lines) {
        let fields: Vec<_> = line.split(' ').collect();
        if fields.len() != 2
            || !full_oid(fields[0])
            || !matches!(fields[1], "commit" | "tree" | "blob")
        {
            return Err(invalid_output(
                "A local history object is unavailable or could not be resolved without fetching.",
            ));
        }
        if fields[1] == "commit" {
            peeled.insert(oid, fields[0].to_owned());
        }
    }
    let roots = peeled
        .values()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut decorations = BTreeMap::<String, Vec<String>>::new();
    let mut full_refs = BTreeMap::<String, Vec<String>>::new();
    if let Some(head) = state.head.as_ref().and_then(|head| peeled.get(head)) {
        let label = state
            .symbolic_head
            .as_deref()
            .map(|name| format!("HEAD -> {}", short_ref(name)))
            .unwrap_or_else(|| "HEAD".to_owned());
        decorations.entry(head.clone()).or_default().push(label);
        full_refs
            .entry(head.clone())
            .or_default()
            .push("HEAD".to_owned());
    }
    for reference in &state.refs {
        if let Some(oid) = peeled.get(&reference.oid) {
            full_refs
                .entry(oid.clone())
                .or_default()
                .push(reference.name.clone());
            if state.symbolic_head.as_deref() == Some(&reference.name) {
                continue;
            }
            let label = reference
                .name
                .strip_prefix("refs/tags/")
                .map(|tag| format!("tag: {tag}"))
                .unwrap_or_else(|| short_ref(&reference.name).to_owned());
            decorations.entry(oid.clone()).or_default().push(label);
        }
    }
    Ok((
        roots,
        decorations
            .into_iter()
            .map(|(oid, names)| (oid, names.join(", ")))
            .collect(),
        full_refs
            .into_iter()
            .map(|(oid, names)| (oid, names.join("\n")))
            .collect(),
    ))
}

fn short_ref(name: &str) -> &str {
    name.strip_prefix("refs/heads/")
        .or_else(|| name.strip_prefix("refs/remotes/"))
        .unwrap_or(name)
}

fn read_search_batch(
    context: &RepositoryContext,
    roots: &[String],
    offset: u64,
) -> Result<Vec<SearchRecord>, GitError> {
    let args = vec![
        "--no-pager".to_owned(),
        "log".to_owned(),
        "--topo-order".to_owned(),
        format!("--skip={offset}"),
        format!("--max-count={}", SEARCH_SCAN_LIMIT + 1),
        "--no-color".to_owned(),
        "--no-show-signature".to_owned(),
        "--no-notes".to_owned(),
        "--no-decorate".to_owned(),
        "--encoding=UTF-8".to_owned(),
        "--pretty=tformat:%H%x00%P%x00%s%x00%an%x00%aI%x00%B%x00%ae".to_owned(),
        "-z".to_owned(),
        "--stdin".to_owned(),
        "--".to_owned(),
    ];
    let input = format!("{}\n", roots.join("\n"));
    let output = run_search_git(context, args, MAX_SEARCH_BYTES, Some(input.as_bytes()))?;
    ensure_success(&output, "read the next bounded history search batch")?;
    parse_search_batch(&output.stdout)
}

fn parse_search_batch(bytes: &[u8]) -> Result<Vec<SearchRecord>, GitError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let bytes = bytes
        .strip_suffix(b"\0")
        .ok_or_else(|| invalid_output("Git returned an unterminated history search record."))?;
    let fields: Vec<_> = bytes.split(|byte| *byte == 0).collect();
    if fields.len() % 7 != 0 || fields.len() / 7 > SEARCH_SCAN_LIMIT + 1 {
        return Err(invalid_output(
            "Git returned an invalid or incomplete history search batch.",
        ));
    }
    fields
        .chunks_exact(7)
        .map(|fields| {
            let oid = utf8(fields[0], "search commit ID")?;
            let parents: Vec<_> = utf8(fields[1], "search commit parents")?
                .split_ascii_whitespace()
                .map(str::to_owned)
                .collect();
            if !full_oid(oid)
                || parents
                    .iter()
                    .any(|parent| !full_oid(parent) || parent.len() != oid.len())
            {
                return Err(invalid_output(
                    "Git returned an invalid search commit ID or parent ID.",
                ));
            }
            Ok(SearchRecord {
                commit: Commit {
                    oid: oid.to_owned(),
                    parents,
                    subject: utf8(fields[2], "search commit subject")?.to_owned(),
                    author: utf8(fields[3], "search commit author")?.to_owned(),
                    date: utf8(fields[4], "search commit date")?.to_owned(),
                    refs: String::new(),
                },
                message: utf8(fields[5], "search commit message")?.to_owned(),
                email: utf8(fields[6], "search commit email")?.to_owned(),
            })
        })
        .collect()
}

fn invalid_output(message: &str) -> GitError {
    GitError::new(GitErrorKind::InvalidOutput, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::Stdio;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

    struct Fixture {
        path: PathBuf,
        git: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "enogram-history-fixture-{}-{id}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create a new isolated history fixture");
            let fixture = Self {
                path,
                git: resolve_git().expect("Git is installed"),
            };
            fixture.git(&["init", "-b", "main"], None);
            let hooks = fixture.path.join(".git/test-hooks");
            fs::create_dir(&hooks).unwrap();
            fixture.git(&["config", "core.hooksPath", hooks.to_str().unwrap()], None);
            fixture
        }

        fn git(&self, args: &[&str], input: Option<&[u8]>) -> String {
            let mut command = git_command(&self.git, &self.path, args);
            command
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env(
                    "GIT_CONFIG_GLOBAL",
                    if cfg!(windows) { "NUL" } else { "/dev/null" },
                )
                .env("GIT_AUTHOR_NAME", "History Tester")
                .env("GIT_AUTHOR_EMAIL", "history@example.invalid")
                .env("GIT_COMMITTER_NAME", "History Tester")
                .env("GIT_COMMITTER_EMAIL", "history@example.invalid")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut child = command.spawn().expect("run fixture Git");
            let mut stdin = child.stdin.take().unwrap();
            if let Some(input) = input {
                stdin.write_all(input).unwrap();
            }
            drop(stdin);
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout)
                .unwrap()
                .trim_end_matches(['\r', '\n'])
                .to_owned()
        }

        fn populate(&self, messages: &[String]) -> Vec<String> {
            let mut input = String::new();
            for (index, message) in messages.iter().enumerate() {
                let mark = index + 1;
                let timestamp = 1_700_000_000 + index;
                input.push_str(&format!("commit refs/heads/main\nmark :{mark}\nauthor SÜLEYMAN Öz <Archive+Ops@example.invalid> {timestamp} +0000\ncommitter History Tester <history@example.invalid> {timestamp} +0000\ndata {}\n{message}\n", message.len()));
                if index > 0 {
                    input.push_str(&format!("from :{index}\n"));
                }
                input.push('\n');
            }
            self.git(&["fast-import", "--quiet"], Some(input.as_bytes()));
            self.git(&["rev-list", "--reverse", "HEAD"], None)
                .lines()
                .map(str::to_owned)
                .collect()
        }

        fn search(&self, query: &str, cursor: Option<&HistorySearchCursor>) -> HistorySearchPage {
            search_repository_history(self.path.to_str().unwrap(), query, cursor)
                .expect("search fixture history")
        }

        fn search_error(&self, query: &str, cursor: Option<&HistorySearchCursor>) -> GitError {
            search_repository_history(self.path.to_str().unwrap(), query, cursor)
                .expect_err("reject search")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn history_search_continues_empty_batches_to_an_old_body_beyond_recent_eighty() {
        let fixture = Fixture::new();
        let mut messages = vec!["recent ordinary commit".to_owned(); SEARCH_SCAN_LIMIT + 12];
        messages[0] = "Old subject\n\nA literal --all [x].* marker with ÜNİCODE.\nSecond body line"
            .to_owned();
        let ids = fixture.populate(&messages);
        let first = fixture.search("  --ALL [x].*  ", None);
        assert!(first.commits.is_empty());
        assert!(!first.complete);
        assert_eq!(first.scanned_count, SEARCH_SCAN_LIMIT as u64);
        assert_eq!(first.next_cursor.as_ref().unwrap().query, "--all [x].*");
        let second = fixture.search("--ALL [x].*", first.next_cursor.as_ref());
        assert!(second.complete);
        assert!(second.next_cursor.is_none());
        assert_eq!(second.scanned_count, messages.len() as u64);
        assert_eq!(second.commits.len(), 1);
        assert_eq!(second.commits[0].oid, ids[0]);
        assert_eq!(second.commits[0].subject, "Old subject");
        assert_eq!(second.commits[0].parents, Vec::<String>::new());
        assert!(fixture.search("nonexistent.*", None).commits.is_empty());
    }

    #[test]
    fn history_search_pages_matches_without_skips_duplicates_or_false_completion() {
        let fixture = Fixture::new();
        let ids = fixture.populate(&vec!["shared match".to_owned(); SEARCH_MATCH_LIMIT * 2 + 7]);
        let mut cursor = None;
        let mut found = Vec::new();
        let mut pages = 0;
        loop {
            let page = fixture.search("shared match", cursor.as_ref());
            pages += 1;
            assert!(page.commits.len() <= SEARCH_MATCH_LIMIT);
            found.extend(page.commits.iter().map(|commit| commit.oid.clone()));
            assert_eq!(page.scanned_count as usize, found.len());
            assert_eq!(page.complete, page.next_cursor.is_none());
            cursor = page.next_cursor;
            if page.complete {
                break;
            }
            assert!(pages < 4);
        }
        assert_eq!(pages, 3);
        assert_eq!(found, ids.into_iter().rev().collect::<Vec<_>>());
    }

    #[test]
    fn history_search_matches_unicode_author_email_full_sha_and_literal_text() {
        let fixture = Fixture::new();
        let ids = fixture.populate(&[
            "Subject [literal]+\n\nBody ÇÖZÜM and two\nlines".to_owned(),
            "Subject literal and an unrelated body".to_owned(),
        ]);
        assert_eq!(fixture.search("[literal]+", None).commits[0].oid, ids[0]);
        assert_eq!(fixture.search("çözüm", None).commits[0].oid, ids[0]);
        assert_eq!(fixture.search("two\nlines", None).commits[0].oid, ids[0]);
        assert_eq!(fixture.search("süleyman öz", None).commits.len(), 2);
        assert_eq!(
            fixture
                .search("archive+ops@EXAMPLE.INVALID", None)
                .commits
                .len(),
            2
        );
        assert_eq!(
            fixture.search(&ids[0].to_uppercase(), None).commits[0].oid,
            ids[0]
        );
        assert!(fixture.search("^Subject", None).commits.is_empty());
        let page = fixture.search("[literal]+", None);
        let json = serde_json::to_value(page).unwrap();
        assert_eq!(json["scannedCount"], 2);
        assert!(json["nextCursor"].is_null());
        assert_eq!(json["complete"], true);
    }

    #[test]
    fn history_search_visits_all_refs_tags_detached_head_and_unborn_checkout() {
        let fixture = Fixture::new();
        let ids = fixture.populate(&["ordinary base".to_owned(), "ordinary tip".to_owned()]);
        fixture.git(&["branch", "archive+literal", &ids[0]], None);
        fixture.git(
            &["tag", "-a", "archive-tag", "-m", "tag metadata", &ids[0]],
            None,
        );
        fixture.git(
            &[
                "tag",
                "-a",
                "nested-tag",
                "-m",
                "nested metadata",
                "archive-tag",
            ],
            None,
        );
        let tree = fixture.git(&["rev-parse", "HEAD^{tree}"], None);
        let remote = fixture.git(&["commit-tree", &tree, "-m", "only remote archive"], None);
        fixture.git(&["update-ref", "refs/remotes/origin/old", &remote], None);
        let blob = fixture.git(&["hash-object", "-w", "--stdin"], Some(b"not a commit"));
        fixture.git(&["update-ref", "refs/tags/blob", &blob], None);
        fixture.git(&["update-ref", "refs/tags/tree", &tree], None);
        fixture.git(
            &["tag", "-a", "blob-tag", "-m", "blob metadata", &blob],
            None,
        );

        assert_eq!(
            fixture.search("archive+literal", None).commits[0].oid,
            ids[0]
        );
        assert_eq!(
            fixture.search("refs/heads/archive+literal", None).commits[0].oid,
            ids[0]
        );
        let tagged = fixture.search("nested-tag", None);
        assert_eq!(tagged.commits.len(), 1);
        assert_eq!(tagged.commits[0].oid, ids[0]);
        assert!(tagged.commits[0].refs.contains("tag: nested-tag"));
        assert_eq!(
            fixture.search("only remote archive", None).commits[0].oid,
            remote
        );
        assert!(fixture.search("blob-tag", None).commits.is_empty());

        fixture.git(&["checkout", "--detach", &ids[1]], None);
        fixture.git(
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-m",
                "detached exclusive",
            ],
            None,
        );
        let detached = fixture.git(&["rev-parse", "HEAD"], None);
        let page = fixture.search("detached exclusive", None);
        assert_eq!(page.commits[0].oid, detached);
        assert_eq!(page.commits[0].refs, "HEAD");
        fixture.git(&["symbolic-ref", "HEAD", "refs/heads/unborn"], None);
        assert_eq!(
            fixture.search("only remote archive", None).commits[0].oid,
            remote
        );
        assert!(fixture
            .search("detached exclusive", None)
            .commits
            .is_empty());
        let empty = Fixture::new().search("anything", None);
        assert!(empty.complete && empty.commits.is_empty() && empty.next_cursor.is_none());
        assert_eq!(empty.scanned_count, 0);
    }

    #[test]
    fn history_search_preserves_real_merge_parents_and_topological_order() {
        let fixture = Fixture::new();
        let ids = fixture.populate(&["shared base".to_owned()]);
        fixture.git(&["checkout", "-b", "side"], None);
        fixture.git(
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-m",
                "shared side",
            ],
            None,
        );
        let side = fixture.git(&["rev-parse", "HEAD"], None);
        fixture.git(&["checkout", "main"], None);
        fixture.git(
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-m",
                "shared main",
            ],
            None,
        );
        let main = fixture.git(&["rev-parse", "HEAD"], None);
        fixture.git(
            &[
                "-c",
                "commit.gpgsign=false",
                "merge",
                "--no-ff",
                "side",
                "-m",
                "shared merge",
            ],
            None,
        );
        let merge = fixture.git(&["rev-parse", "HEAD"], None);
        let page = fixture.search("shared", None);
        assert_eq!(page.commits.len(), 4);
        assert_eq!(page.commits[0].oid, merge);
        assert_eq!(page.commits[0].parents, vec![main, side]);
        assert_eq!(page.commits.last().unwrap().oid, ids[0]);
        let filtered = fixture.search("shared merge", None);
        assert_eq!(filtered.commits.len(), 1);
        assert_eq!(filtered.commits[0].parents, page.commits[0].parents);
    }

    #[test]
    fn history_search_rejects_invalid_and_stale_cursors_but_not_index_changes() {
        let fixture = Fixture::new();
        fixture.populate(&vec!["shared match".to_owned(); SEARCH_MATCH_LIMIT + 2]);
        let first = fixture.search("shared match", None);
        let cursor = first.next_cursor.unwrap();
        for query in ["", " \t ", "bad\0query"] {
            assert_eq!(
                fixture.search_error(query, None).kind,
                GitErrorKind::InvalidHistorySearch
            );
        }
        assert_eq!(
            fixture
                .search_error(&"x".repeat(MAX_QUERY_BYTES + 1), None)
                .kind,
            GitErrorKind::InvalidHistorySearch
        );
        assert_eq!(
            fixture.search_error("other", Some(&cursor)).kind,
            GitErrorKind::InvalidHistorySearch
        );
        for offset in [0, MAX_CURSOR_OFFSET + 1] {
            let invalid = HistorySearchCursor {
                offset,
                ..cursor.clone()
            };
            assert_eq!(
                fixture.search_error("shared match", Some(&invalid)).kind,
                GitErrorKind::InvalidHistorySearch
            );
        }
        let invalid = HistorySearchCursor {
            history_token: "--all".to_owned(),
            ..cursor.clone()
        };
        assert_eq!(
            fixture.search_error("shared match", Some(&invalid)).kind,
            GitErrorKind::InvalidHistorySearch
        );
        let beyond_end = HistorySearchCursor {
            offset: 1_000,
            ..cursor.clone()
        };
        assert_eq!(
            fixture.search_error("shared match", Some(&beyond_end)).kind,
            GitErrorKind::InvalidHistorySearch
        );

        fs::write(fixture.path.join("dirty.txt"), "working contents\n").unwrap();
        fixture.git(&["add", "dirty.txt"], None);
        let index = fs::read(fixture.path.join(".git/index")).unwrap();
        let refs = fixture.git(&["show-ref"], None);
        let objects = fixture.git(&["count-objects", "-v"], None);
        assert!(fixture.search("shared match", Some(&cursor)).complete);
        assert_eq!(fs::read(fixture.path.join(".git/index")).unwrap(), index);
        assert_eq!(fixture.git(&["show-ref"], None), refs);
        assert_eq!(fixture.git(&["count-objects", "-v"], None), objects);
        assert_eq!(
            fs::read_to_string(fixture.path.join("dirty.txt")).unwrap(),
            "working contents\n"
        );

        fixture.git(&["branch", "new-ref"], None);
        assert_eq!(
            fixture.search_error("shared match", Some(&cursor)).kind,
            GitErrorKind::StaleState
        );
        let cursor = fixture.search("shared match", None).next_cursor.unwrap();
        fixture.git(&["symbolic-ref", "HEAD", "refs/heads/new-ref"], None);
        assert_eq!(
            fixture.search_error("shared match", Some(&cursor)).kind,
            GitErrorKind::StaleState
        );
        let cursor = fixture.search("shared match", None).next_cursor.unwrap();
        let head = fixture.git(&["rev-parse", "HEAD"], None);
        fs::write(fixture.path.join(".git/shallow"), format!("{head}\n")).unwrap();
        assert_eq!(
            fixture.search_error("shared match", Some(&cursor)).kind,
            GitErrorKind::StaleState
        );
    }

    #[test]
    fn history_search_parser_rejects_incomplete_records_and_invalid_oids() {
        for bytes in [
            b"truncated".as_slice(),
            b"x\0\0subject\0author\0date\0body\0email\0".as_slice(),
            b"\0".as_slice(),
        ] {
            assert_eq!(
                parse_search_batch(bytes).err().unwrap().kind,
                GitErrorKind::InvalidOutput
            );
        }
        assert!(serde_json::from_str::<HistorySearchCursor>(
            r#"{"historyToken":"x","query":"x","offset":-1}"#
        )
        .is_err());
    }

    #[test]
    fn history_search_blocks_all_transport_and_rejects_legacy_promisor_clones() {
        let fixture = Fixture::new();
        fixture.populate(&["local commit".to_owned()]);
        let context = repository_context(fixture.path.to_str().unwrap()).unwrap();
        let transport = run_search_git(
            &context,
            [
                "-c",
                "protocol.file.allow=always",
                "ls-remote",
                fixture.path.to_str().unwrap(),
            ],
            4096,
            None,
        )
        .unwrap();
        assert!(!transport.status.success());
        assert!(stderr_text(&transport).contains("transport 'file' not allowed"));
        fixture.git(&["config", "remote.origin.promisor", "true"], None);
        fixture.git(
            &[
                "config",
                "remote.origin.url",
                "ext::enogram-forbidden-transport",
            ],
            None,
        );
        fixture.git(&["config", "protocol.ext.allow", "always"], None);
        let support =
            run_search_git(&context, ["--no-lazy-fetch", "--version"], 4096, None).unwrap();
        if support.status.success() {
            assert!(fixture.search("local commit", None).complete);
        } else {
            let error = fixture.search_error("local commit", None);
            assert_eq!(error.kind, GitErrorKind::Unsupported);
            assert!(error.message.contains("partial/promisor"));
        }
        assert!(!fixture.path.join(".git/FETCH_HEAD").exists());
    }

    #[test]
    fn history_search_reports_oversized_message_as_an_error_not_a_complete_empty_page() {
        let fixture = Fixture::new();
        fixture.populate(&[format!("Huge message\n\n{}", "x".repeat(MAX_SEARCH_BYTES))]);
        assert_eq!(
            fixture.search_error("unmatched query", None).kind,
            GitErrorKind::OutputLimit
        );
    }
}
