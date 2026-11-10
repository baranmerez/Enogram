//! Explicit remote operations. Remote configuration and checkout state are
//! checked before each write; external Git processes are still outside our lock.

use super::*;

#[path = "remote_settings.rs"]
mod remote_settings;

pub use remote_settings::{add_remote, update_remote};

const MAX_CONFIG_BYTES: usize = 4 * 1024 * 1024;
const MAX_REMOTE_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Remote {
    pub name: String,
    /// Display text only. Never pass a displayed URL back to Git.
    pub fetch_url: String,
    pub push_url: String,
    /// Original configured value, only when no display redaction is needed.
    /// None must preserve the original value unless the user replaces it.
    pub configured_fetch_url: Option<String>,
    pub configured_push_url: Option<String>,
    pub has_explicit_push_url: bool,
    pub edit_blocked_reason: Option<String>,
    pub config_token: String,
}

struct RemoteState {
    fetch_urls: Vec<String>,
    push_urls: Vec<String>,
    config: EffectiveConfig,
}

struct EffectiveConfig {
    bytes: Vec<u8>,
    entries: Vec<(String, String)>,
    origins: Vec<ConfigOrigin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigOrigin {
    scope: String,
    source: String,
}

impl EffectiveConfig {
    fn remote_names(&self) -> BTreeSet<String> {
        self.entries
            .iter()
            .filter_map(|(key, _)| {
                key.strip_prefix("remote.")?
                    .rsplit_once('.')
                    .map(|(name, _)| name)
            })
            .map(str::to_owned)
            .collect()
    }

    fn last(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .rev()
            .find_map(|(candidate, value)| (candidate == key).then_some(value.as_str()))
    }
}

pub(super) fn read_remotes(context: &RepositoryContext) -> Result<Vec<Remote>, GitError> {
    let config = read_effective_config(context)?;
    let mut remotes = Vec::new();
    for name in config.remote_names() {
        // Unusual/malformed remote names are still visible, but cannot be used
        // as command arguments. Reading a repository must not run a URL helper.
        let (fetch_urls, push_urls) = if remote_name_shape_is_safe(&name)
            && config
                .last(&format!("remote.{name}.url"))
                .is_some_and(|url| !url.is_empty())
        {
            (
                read_remote_urls(context, &name, false)?,
                read_remote_urls(context, &name, true)?,
            )
        } else {
            (Vec::new(), Vec::new())
        };
        remotes.push(remote_display(
            context,
            &config,
            &name,
            &fetch_urls,
            &push_urls,
        )?);
    }
    ensure_config_unchanged(context, &config)?;
    Ok(remotes)
}

/// Fetch heads only into this remote's canonical tracking namespace. Configured
/// fetch mappings, tag following, pruning and submodule recursion are ignored.
pub fn fetch_remote(
    repo_path: &str,
    remote_name: &str,
    expected_remote_token: &str,
    expected_token: &str,
) -> Result<MutationOutcome, GitError> {
    let _guard = acquire_mutation_lock()?;
    let context = repository_context(repo_path)?;
    validate_token(&context, expected_token)?;
    let remote = checked_remote(&context, remote_name, expected_remote_token, false)?;
    ensure_tracking_namespace_safe(&context, remote_name, None)?;
    validate_token(&context, expected_token)?;
    ensure_config_unchanged(&context, &remote.config)?;
    let refspec = format!("+refs/heads/*:refs/remotes/{remote_name}/*");
    fetch_refspec(&context, remote_name, &refspec)?;
    validate_token(&context, expected_token).map_err(after_remote_write_error)?;
    ensure_config_unchanged(&context, &remote.config).map_err(after_remote_write_error)?;
    let outcome = finish_mutation(&context, "Fetch", None)?;
    ensure_config_unchanged(&context, &remote.config).map_err(after_remote_write_error)?;
    if outcome.snapshot.mutation_token != expected_token {
        return Err(after_remote_write_error(stale_checkout()));
    }
    Ok(outcome)
}

/// Fast-forward the current branch from one explicit remote branch. No merge
/// commit, rebase, autostash, upstream configuration or automatic retry is used.
pub fn pull_remote(
    repo_path: &str,
    remote_name: &str,
    branch_name: &str,
    expected_remote_token: &str,
    expected_token: &str,
) -> Result<MutationOutcome, GitError> {
    let _guard = acquire_mutation_lock()?;
    let context = repository_context(repo_path)?;
    validate_token(&context, expected_token)?;
    validate_target_branch(&context, branch_name)?;
    let remote = checked_remote(&context, remote_name, expected_remote_token, false)?;
    let (current_branch, previous_head) = clean_pull_checkout(&context)?;
    ensure_tracking_namespace_safe(&context, remote_name, Some(branch_name))?;
    ensure_index_unlocked(&context)?;
    validate_token(&context, expected_token)?;
    ensure_config_unchanged(&context, &remote.config)?;

    let tracking_ref = format!("refs/remotes/{remote_name}/{branch_name}");
    let refspec = format!("+refs/heads/{branch_name}:{tracking_ref}");
    let output = fetch_refspec(&context, remote_name, &refspec)?;
    let fetched_oid = fetched_oid(&output, &tracking_ref).map_err(after_fetch_error)?;

    // A credential helper, hook or external Git process may have changed the
    // checkout/config during the fetch. Never merge into that changed state.
    validate_token(&context, expected_token).map_err(after_fetch_error)?;
    ensure_config_unchanged(&context, &remote.config).map_err(after_fetch_error)?;
    let (branch_after_fetch, head_after_fetch) =
        clean_pull_checkout(&context).map_err(after_fetch_error)?;
    if branch_after_fetch != current_branch || head_after_fetch != previous_head {
        return Err(after_fetch_error(stale_checkout()));
    }
    ensure_index_unlocked(&context).map_err(after_fetch_error)?;

    let expected_head = if is_ancestor(&context, &fetched_oid, &previous_head)
        .map_err(after_fetch_error)?
    {
        // Includes an already-current checkout and a checkout ahead of remote.
        previous_head.clone()
    } else if is_ancestor(&context, &previous_head, &fetched_oid).map_err(after_fetch_error)? {
        fetched_oid.clone()
    } else {
        return Err(after_fetch_error(GitError::new(
            GitErrorKind::Unsupported,
            "The local and remote branches have diverged. Pull only permits a fast-forward; integrate the branches explicitly in Git.",
        )));
    };

    ensure_config_unchanged(&context, &remote.config).map_err(after_fetch_error)?;
    let (final_branch, final_head) = clean_pull_checkout(&context).map_err(after_fetch_error)?;
    if final_branch != current_branch || final_head != previous_head {
        return Err(after_fetch_error(stale_checkout()));
    }
    ensure_index_unlocked(&context).map_err(after_fetch_error)?;
    validate_token(&context, expected_token).map_err(after_fetch_error)?;
    ensure_mutation_success(
        run_git(
            &context.git,
            &context.root,
            [
                "-c",
                "submodule.recurse=false",
                "merge",
                "--ff-only",
                "--no-squash",
                "--no-edit",
                "--no-autostash",
                "--no-overwrite-ignore",
                "--",
                &fetched_oid,
            ],
            MAX_REMOTE_OUTPUT_BYTES,
            false,
        )
        .map(redact_remote_output),
        "fast-forward the current branch",
    )?;
    let outcome = finish_mutation(&context, "Pull", None)?;
    ensure_config_unchanged(&context, &remote.config).map_err(after_remote_write_error)?;
    if outcome.snapshot.branch != current_branch
        || outcome.snapshot.head.as_deref() != Some(expected_head.as_str())
    {
        return Err(GitError::new(
            GitErrorKind::StaleState,
            "Pull completed, but HEAD moved unexpectedly afterward. A hook or external Git process may have changed it. Refresh and inspect history; do not automatically repeat the pull.",
        ));
    }
    Ok(outcome)
}

/// Push the displayed HEAD commit to one explicit remote branch. Neither
/// default refspecs nor matching, force, mirror, tag or submodule pushes are used.
pub fn push_remote(
    repo_path: &str,
    remote_name: &str,
    branch_name: &str,
    expected_remote_token: &str,
    expected_token: &str,
) -> Result<MutationOutcome, GitError> {
    let _guard = acquire_mutation_lock()?;
    let context = repository_context(repo_path)?;
    validate_token(&context, expected_token)?;
    validate_target_branch(&context, branch_name)?;
    let remote = checked_remote(&context, remote_name, expected_remote_token, true)?;
    ensure_push_tracking_config(&remote.config, remote_name)?;
    ensure_tracking_namespace_safe(&context, remote_name, Some(branch_name))?;
    let (_, head, _, _) = read_status(&context)?;
    let source_oid = head.filter(|oid| valid_oid(oid)).ok_or_else(|| {
        GitError::new(
            GitErrorKind::Unsupported,
            "Create a commit before pushing this checkout.",
        )
    })?;
    validate_token(&context, expected_token)?;
    ensure_config_unchanged(&context, &remote.config)?;
    let refspec = format!("{source_oid}:refs/heads/{branch_name}");
    remote_write(
        &context,
        [
            "-c",
            "credential.interactive=false",
            "-c",
            "push.autoSetupRemote=false",
            "push",
            "--porcelain",
            "--no-force",
            "--no-mirror",
            "--no-follow-tags",
            "--no-prune",
            "--recurse-submodules=no",
            "--",
            remote_name,
            &refspec,
        ],
        "push the selected branch",
    )?;
    validate_token(&context, expected_token).map_err(after_remote_write_error)?;
    ensure_config_unchanged(&context, &remote.config).map_err(after_remote_write_error)?;
    let outcome = finish_mutation(&context, "Push", None)?;
    ensure_config_unchanged(&context, &remote.config).map_err(after_remote_write_error)?;
    if outcome.snapshot.mutation_token != expected_token {
        return Err(after_remote_write_error(stale_checkout()));
    }
    Ok(outcome)
}

fn read_effective_config(context: &RepositoryContext) -> Result<EffectiveConfig, GitError> {
    let output = run_git(
        &context.git,
        &context.root,
        [
            "config",
            "--null",
            "--list",
            "--includes",
            "--show-origin",
            "--show-scope",
        ],
        MAX_CONFIG_BYTES,
        false,
    )?;
    // Config parsing errors can contain a credential-bearing URL or include
    // path. Keep the original config and diagnostics out of serialized errors.
    if !output.status.success() {
        return Err(GitError::new(
            GitErrorKind::GitCommandFailed,
            "Git could not read the effective remote configuration. Check the repository's Git configuration.",
        ));
    }
    let mut entries = Vec::new();
    let mut origins = Vec::new();
    let fields = output
        .stdout
        .strip_suffix(&[0])
        .unwrap_or(&output.stdout)
        .split(|byte| *byte == 0)
        .collect::<Vec<_>>();
    let fields = if output.stdout.is_empty() {
        Vec::new()
    } else {
        fields
    };
    if fields.len() % 3 != 0 || (!output.stdout.is_empty() && !output.stdout.ends_with(&[0])) {
        return Err(GitError::new(
            GitErrorKind::InvalidOutput,
            "Git returned invalid configuration origin information.",
        ));
    }
    for fields in fields.chunks_exact(3) {
        let scope = output_text(fields[0], "Git configuration scope")?;
        let source = output_text(fields[1], "Git configuration origin")?;
        let entry = output_text(fields[2], "Git configuration")?;
        let (key, value) = entry.split_once('\n').unwrap_or((entry, ""));
        entries.push((key.to_owned(), value.to_owned()));
        origins.push(ConfigOrigin {
            scope: scope.to_owned(),
            source: source.to_owned(),
        });
    }
    Ok(EffectiveConfig {
        bytes: output.stdout,
        entries,
        origins,
    })
}

fn ensure_config_unchanged(
    context: &RepositoryContext,
    expected: &EffectiveConfig,
) -> Result<(), GitError> {
    if read_effective_config(context)?.bytes != expected.bytes {
        return Err(GitError::new(
            GitErrorKind::StaleState,
            "Git remote configuration changed. Refresh and choose the remote again before writing.",
        ));
    }
    Ok(())
}

fn remote_token(
    context: &RepositoryContext,
    config: &EffectiveConfig,
    name: &str,
) -> Result<String, GitError> {
    let mut bytes = b"enogram-remote-v1\0".to_vec();
    append_fingerprint_field(&mut bytes, path_to_string(&context.root)?.as_bytes());
    append_fingerprint_field(&mut bytes, path_to_string(&context.git_dir)?.as_bytes());
    append_fingerprint_field(&mut bytes, name.as_bytes());
    append_fingerprint_field(&mut bytes, &config.bytes);
    let output = run_git_with_input(
        &context.git,
        &context.root,
        ["hash-object", "--no-filters", "--stdin"],
        4096,
        false,
        Some(&bytes),
    )?;
    ensure_success(&output, "fingerprint remote configuration")?;
    let oid = output_text(&output.stdout, "remote configuration token")?.trim();
    if !valid_oid(oid) {
        return Err(GitError::new(
            GitErrorKind::InvalidOutput,
            "Git returned an invalid remote configuration fingerprint.",
        ));
    }
    Ok(format!("remote-v1:{oid}"))
}

fn remote_display(
    context: &RepositoryContext,
    config: &EffectiveConfig,
    name: &str,
    fetch_urls: &[String],
    push_urls: &[String],
) -> Result<Remote, GitError> {
    let settings = remote_settings::remote_settings_metadata(context, config, name)?;
    Ok(Remote {
        name: name.to_owned(),
        fetch_url: fetch_urls
            .first()
            .map_or_else(String::new, |url| display_url(url)),
        push_url: push_urls
            .iter()
            .map(|url| display_url(url))
            .collect::<Vec<_>>()
            .join(", "),
        configured_fetch_url: settings.configured_fetch_url,
        configured_push_url: settings.configured_push_url,
        has_explicit_push_url: settings.has_explicit_push_url,
        edit_blocked_reason: settings.edit_blocked_reason,
        config_token: remote_token(context, config, name)?,
    })
}

fn read_remote_urls(
    context: &RepositoryContext,
    name: &str,
    push: bool,
) -> Result<Vec<String>, GitError> {
    let mut args = vec!["remote", "get-url"];
    if push {
        args.push("--push");
    }
    args.extend(["--", name]);
    let output = run_git(&context.git, &context.root, &args, MAX_CONFIG_BYTES, false)?;
    if !output.status.success() {
        // An invalid remote may be displayed, but checked_remote will reject it.
        return Ok(Vec::new());
    }
    let text = output_text(&output.stdout, "remote URLs")?;
    // get-url without --all returns one URL followed by LF. Never split URL
    // data on lines: raw URLs and insteadOf/pushInsteadOf rewrite bases can
    // contain newlines, exposing partial userinfo if treated as separate URLs.
    let first = text.strip_suffix('\n').unwrap_or(text);
    if first.chars().any(char::is_control) {
        // The private sentinel cannot be displayed as a URL or passed to Git.
        return Ok(vec!["\0".to_owned()]);
    }
    let mut urls = vec![first.to_owned()];
    if push {
        args.insert(2, "--all");
        let all = run_git(&context.git, &context.root, args, MAX_CONFIG_BYTES, false)?;
        if !all.status.success() {
            return Ok(Vec::new());
        }
        if all.stdout != output.stdout {
            // Only the first destination is display data. Remaining output
            // may contain multiline secrets, so expose a marker, not fragments.
            urls.push("[additional push destinations]".to_owned());
        }
    }
    Ok(urls)
}

fn checked_remote(
    context: &RepositoryContext,
    name: &str,
    expected_token: &str,
    push: bool,
) -> Result<RemoteState, GitError> {
    if !remote_name_shape_is_safe(name) {
        return Err(GitError::new(
            GitErrorKind::Unsupported,
            "Choose an existing remote with a valid name; URLs and command options are not accepted as remote names.",
        ));
    }
    check_ref_name(context, &format!("refs/remotes/{name}/enogram-check"))?;
    let config = read_effective_config(context)?;
    if !config.remote_names().contains(name) {
        return Err(GitError::new(
            GitErrorKind::StaleState,
            "The selected remote no longer exists. Refresh and choose an existing remote.",
        ));
    }
    if config
        .last(&format!("remote.{name}.url"))
        .is_none_or(|url| url.is_empty())
    {
        return Err(GitError::new(
            GitErrorKind::Unsupported,
            "The selected remote has no configured URL. Configure its URL in Git before using this action.",
        ));
    }
    let fetch_urls = read_remote_urls(context, name, false)?;
    let push_urls = read_remote_urls(context, name, true)?;
    let remote = remote_display(context, &config, name, &fetch_urls, &push_urls)?;
    if expected_token.is_empty() || expected_token != remote.config_token {
        return Err(GitError::new(
            GitErrorKind::StaleState,
            "The selected remote or Git configuration changed since the displayed snapshot. Refresh and choose the remote again.",
        ));
    }
    let state = RemoteState {
        fetch_urls,
        push_urls,
        config,
    };
    let urls = if push {
        &state.push_urls
    } else {
        &state.fetch_urls
    };
    if urls.is_empty() || urls.iter().any(|url| !remote_url_is_safe(url)) {
        return Err(GitError::new(
            GitErrorKind::Unsupported,
            "The selected remote has a missing or unsupported URL. Use a standard Git URL or local repository path.",
        ));
    }
    // Checking the raw values also catches newlines that get-url would split
    // into apparently distinct URLs, and helper protocols hidden by rewrites.
    let url_key = format!("remote.{name}.url");
    let push_url_key = format!("remote.{name}.pushurl");
    if state.config.entries.iter().any(|(key, value)| {
        (key == &url_key || key == &push_url_key)
            && (value.is_empty() || value.chars().any(char::is_control))
    }) || state
        .config
        .last(&format!("remote.{name}.vcs"))
        .is_some_and(|value| !value.is_empty())
    {
        return Err(GitError::new(
            GitErrorKind::Unsupported,
            "The selected remote uses a malformed URL or an unsupported custom transport.",
        ));
    }
    if push && state.push_urls.len() != 1 {
        return Err(GitError::new(
            GitErrorKind::Unsupported,
            "This remote has multiple push URLs. Choose a remote with one push destination to avoid publishing to additional repositories.",
        ));
    }
    if push {
        if let Some(value) = state.config.last(&format!("remote.{name}.mirror")) {
            if !matches!(
                value.to_ascii_lowercase().as_str(),
                "false" | "no" | "off" | "0"
            ) {
                return Err(GitError::new(
                    GitErrorKind::Unsupported,
                    "Mirror remotes are not supported for this branch push. Choose a remote configured for normal branch pushes.",
                ));
            }
        }
    }
    ensure_config_unchanged(context, &state.config)?;
    Ok(state)
}

fn ensure_push_tracking_config(config: &EffectiveConfig, remote: &str) -> Result<(), GitError> {
    // Git also updates local tracking refs after a push. Unlike fetch, push
    // has no --refmap override; reject custom mappings rather than permitting
    // a successful publication to unexpectedly move local branches or tags.
    let key = format!("remote.{remote}.fetch");
    let canonical = format!("refs/heads/*:refs/remotes/{remote}/*");
    if config.entries.iter().any(|(candidate, value)| {
        candidate == &key && value.strip_prefix('+').unwrap_or(value) != canonical
    }) {
        return Err(GitError::new(
            GitErrorKind::Unsupported,
            "This remote uses custom fetch mappings. Push requires canonical remote-tracking mappings so it cannot update other local references.",
        ));
    }
    Ok(())
}

fn remote_name_shape_is_safe(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 1024
        && name != "."
        && !name.starts_with('-')
        && !name.starts_with('/')
        && !name.contains([':', '\\', '*', '?', '[', '^', '~'])
        && !name
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
}

fn remote_url_is_safe(url: &str) -> bool {
    if url.is_empty() || url.starts_with('-') || url.chars().any(char::is_control) {
        return false;
    }
    if let Some((scheme, _)) = url.split_once("://") {
        return matches!(
            scheme.to_ascii_lowercase().as_str(),
            "http" | "https" | "ssh" | "git" | "file" | "ftp" | "ftps"
        );
    }
    !url.contains("::")
}

/// Remove URL userinfo, SCP usernames, query/fragment data and control text.
/// Unknown helper syntax may contain shell commands and is never displayed.
fn display_url(url: &str) -> String {
    if !remote_url_is_safe(url) {
        return "[configured URL hidden]".to_owned();
    }
    let clean = url.split(['?', '#']).next().unwrap_or("");
    if let Some((scheme, rest)) = clean.split_once("://") {
        let (authority, path) = rest
            .split_once('/')
            .map_or((rest, ""), |(host, tail)| (host, tail));
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        if path.is_empty() {
            format!("{scheme}://{host}")
        } else {
            format!("{scheme}://{host}/{path}")
        }
    } else if let Some((host, path)) = clean.split_once(':') {
        // C:\... / C:/... are filesystem paths, not SCP addresses.
        if host.len() == 1 && host.as_bytes()[0].is_ascii_alphabetic() {
            return clean.to_owned();
        }
        let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
        format!("{host}:{path}")
    } else {
        clean.to_owned()
    }
}

fn validate_target_branch(context: &RepositoryContext, branch: &str) -> Result<(), GitError> {
    if branch.is_empty()
        || branch.len() > 1024
        || branch == "HEAD"
        || branch.starts_with('-')
        || branch.starts_with("refs/")
        || branch.chars().any(char::is_control)
    {
        return Err(GitError::new(
            GitErrorKind::Unsupported,
            "Enter a short branch name, such as main or feature/work, without a refspec, revision expression or command option.",
        ));
    }
    check_ref_name(context, &format!("refs/heads/{branch}"))
}

fn check_ref_name(context: &RepositoryContext, reference: &str) -> Result<(), GitError> {
    let output = run_git(
        &context.git,
        &context.root,
        ["check-ref-format", reference],
        4096,
        false,
    )?;
    if !output.status.success() {
        return Err(GitError::new(
            GitErrorKind::Unsupported,
            "The remote or branch name is not a valid Git reference name.",
        ));
    }
    Ok(())
}

fn clean_pull_checkout(context: &RepositoryContext) -> Result<(String, String), GitError> {
    branches::ensure_no_active_operation(context)?;
    let (branch, head, unborn, changes) = read_status(context)?;
    if !changes.is_empty() {
        return Err(GitError::new(
            GitErrorKind::Unsupported,
            "Pull requires a clean working tree and index, including untracked files. Commit or stash changes explicitly before pulling.",
        ));
    }
    let symbolic = run_git(
        &context.git,
        &context.root,
        ["symbolic-ref", "--quiet", "HEAD"],
        4096,
        false,
    )?;
    if unborn || !symbolic.status.success() || head.is_none() {
        return Err(GitError::new(
            GitErrorKind::Unsupported,
            "Pull requires a checked-out local branch with an existing commit.",
        ));
    }
    let head = head.unwrap();
    if !valid_oid(&head) {
        return Err(GitError::new(
            GitErrorKind::InvalidOutput,
            "Git returned an invalid HEAD commit.",
        ));
    }
    Ok((branch, head))
}

fn ensure_tracking_namespace_safe(
    context: &RepositoryContext,
    remote: &str,
    branch: Option<&str>,
) -> Result<(), GitError> {
    let prefix = format!("refs/remotes/{remote}/");
    if let Some(branch) = branch {
        let selected = format!("{prefix}{branch}");
        // for-each-ref omits dangling symrefs. Explicit pull destinations and
        // post-push tracking updates still dereference them, so check the exact
        // destination without following its target, including an absent target.
        let symbolic = run_git(
            &context.git,
            &context.root,
            ["symbolic-ref", "--quiet", "--no-recurse", &selected],
            4096,
            false,
        )?;
        match symbolic.status.code() {
            Some(1) => {}
            Some(0) => {
                return Err(GitError::new(
                    GitErrorKind::Unsupported,
                    "The selected remote-tracking branch is a symbolic reference. Resolve that configuration in Git before pulling or pushing here.",
                ));
            }
            _ => {
                return Err(GitError::new(
                    GitErrorKind::GitCommandFailed,
                    "Git could not verify the selected remote-tracking branch.",
                ));
            }
        }
    }
    let output = run_git(
        &context.git,
        &context.root,
        ["for-each-ref", "--format=%(refname)%00%(symref)", &prefix],
        MAX_REMOTE_OUTPUT_BYTES,
        false,
    )?;
    ensure_success(&output, "check remote-tracking references")?;
    let text = output_text(&output.stdout, "remote-tracking references")?;
    let selected = branch.map(|branch| format!("{prefix}{branch}"));
    for line in text.lines() {
        let (reference, target) = line.split_once('\0').ok_or_else(|| {
            GitError::new(
                GitErrorKind::InvalidOutput,
                "Git returned invalid remote-tracking reference information.",
            )
        })?;
        // A tracking symref could redirect fetch to a local branch. Ordinary
        // origin/HEAD is harmless for heads other than HEAD and is left intact.
        if !target.is_empty()
            && (!target.starts_with(&prefix)
                || selected.as_ref().map_or_else(
                    || reference != format!("{prefix}HEAD"),
                    |selected| reference == selected,
                ))
        {
            return Err(GitError::new(
                GitErrorKind::Unsupported,
                "A remote-tracking branch is a symbolic reference. Resolve that configuration in Git before fetching here.",
            ));
        }
    }
    Ok(())
}

fn fetch_refspec(
    context: &RepositoryContext,
    remote: &str,
    refspec: &str,
) -> Result<Vec<u8>, GitError> {
    remote_write(
        context,
        [
            "-c",
            "credential.interactive=false",
            "fetch",
            "--porcelain",
            "--verbose",
            "--atomic",
            "--refmap=",
            "--no-tags",
            "--no-prune",
            "--no-prune-tags",
            "--no-recurse-submodules",
            "--no-write-fetch-head",
            "--no-auto-maintenance",
            "--no-write-commit-graph",
            "--",
            remote,
            refspec,
        ],
        "fetch the selected remote",
    )
}

fn remote_write<I, S>(
    context: &RepositoryContext,
    args: I,
    operation: &str,
) -> Result<Vec<u8>, GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let result = run_git_network(
        &context.git,
        &context.root,
        args,
        MAX_REMOTE_OUTPUT_BYTES,
        false,
    );
    let stdout = result
        .as_ref()
        .ok()
        .map(|output| output.stdout.clone())
        .unwrap_or_default();
    ensure_mutation_success(result.map(redact_remote_output), operation)?;
    Ok(stdout)
}

fn redact_remote_output(mut output: CapturedOutput) -> CapturedOutput {
    if !output.status.success() {
        let diagnostic = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        )
        .to_ascii_lowercase();
        let message = if diagnostic.contains("non-fast-forward")
            || diagnostic.contains("fetch first")
        {
            "The remote branch rejected a non-fast-forward update. Fetch and integrate its changes before pushing."
        } else if diagnostic.contains("authentication failed")
            || diagnostic.contains("permission denied")
            || diagnostic.contains("could not read username")
            || diagnostic.contains("could not read password")
            || diagnostic.contains("terminal prompts disabled")
            || diagnostic.contains("could not read from remote repository")
        {
            "Remote authentication or access failed. Check the selected remote and configure credentials in your Git client; interactive prompts are disabled."
        } else if diagnostic.contains("could not resolve")
            || diagnostic.contains("failed to connect")
            || diagnostic.contains("connection refused")
            || diagnostic.contains("unable to access")
        {
            "Git could not connect to the selected remote. Check its URL and network access."
        } else if diagnostic.contains(".lock")
            && (diagnostic.contains("file exists") || diagnostic.contains("another git process"))
        {
            "A Git .lock file exists because another git process may be running."
        } else if diagnostic.contains("remote rejected") || diagnostic.contains("hook declined") {
            "The remote rejected the branch update. Check remote branch permissions and hooks."
        } else if diagnostic.contains("does not have a gpg signature")
            || diagnostic.contains("does not have a good signature")
            || diagnostic.contains("bad signature")
            || diagnostic.contains("failed to sign")
        {
            "Git could not satisfy the configured signature requirements. Check the commit signature or signing configuration in your Git client."
        } else if diagnostic.contains("would be overwritten") {
            "Git refused to overwrite local files. Inspect the working tree, including ignored files, before pulling."
        } else {
            "The remote Git operation failed. Check the selected remote, branch, credentials and hooks in your Git client."
        };
        output.stderr = message.as_bytes().to_vec();
        output.stderr_truncated = false;
    }
    output
}

fn fetched_oid(output: &[u8], tracking_ref: &str) -> Result<String, GitError> {
    let text = output_text(output, "fetch result")?;
    let mut result = None;
    for line in text.lines() {
        // The fast-forward flag itself is a space, so split the flag before
        // splitting fields. --verbose includes an '=' record for a no-op.
        if line.len() < 2
            || !matches!(
                line.as_bytes()[0],
                b' ' | b'+' | b'*' | b'=' | b'!' | b'-' | b't'
            )
            || line.as_bytes()[1] != b' '
        {
            continue;
        }
        let flag = line.as_bytes()[0];
        let fields = line[1..].split_whitespace().collect::<Vec<_>>();
        if fields.len() == 3 && fields[2] == tracking_ref {
            if !matches!(flag, b' ' | b'+' | b'*' | b'=')
                || !valid_oid(fields[1])
                || result.is_some()
            {
                return Err(GitError::new(
                    GitErrorKind::InvalidOutput,
                    "Git returned an ambiguous fetched branch result.",
                ));
            }
            result = Some(fields[1].to_owned());
        }
    }
    result.ok_or_else(|| GitError::new(
        GitErrorKind::InvalidOutput,
        "Fetch completed without an exact commit result for the selected branch. Refresh and inspect the remote branch before pulling again.",
    ))
}

fn is_ancestor(
    context: &RepositoryContext,
    ancestor: &str,
    commit: &str,
) -> Result<bool, GitError> {
    let output = run_git(
        &context.git,
        &context.root,
        ["merge-base", "--is-ancestor", ancestor, commit],
        4096,
        false,
    )?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(GitError::new(
            GitErrorKind::GitCommandFailed,
            "Git could not compare the fetched branch with HEAD.",
        )),
    }
}

fn valid_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn stale_checkout() -> GitError {
    GitError::new(
        GitErrorKind::StaleState,
        "The current checkout changed during fetch.",
    )
}

fn after_fetch_error(error: GitError) -> GitError {
    GitError::new(
        error.kind,
        format!("{} Fetch has already updated remote-tracking references; the fast-forward was not performed. Refresh before another action; do not automatically retry.", error.message),
    )
}

fn after_remote_write_error(error: GitError) -> GitError {
    GitError::new(
        error.kind,
        format!("{} The remote operation already ran and local or remote references may have changed. Refresh and inspect the result before another action; do not automatically retry.", error.message),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Output;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

    struct Fixture {
        base: PathBuf,
        client: PathBuf,
        seed: PathBuf,
        bare: PathBuf,
        git: PathBuf,
        _serial: MutexGuard<'static, ()>,
    }

    impl Fixture {
        fn new() -> Self {
            let serial = crate::tests::MUTATION_TEST_LOCK
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let base = std::env::temp_dir().join(format!(
                "enogram-remote-fixture-{}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&base).unwrap();
            let fixture = Self {
                client: base.join("client"),
                seed: base.join("seed"),
                bare: base.join("remote.git"),
                git: resolve_git().expect("Git installed for remote integration tests"),
                base,
                _serial: serial,
            };
            fixture.ok_at(
                &fixture.base,
                &[
                    "init",
                    "--bare",
                    "-b",
                    "main",
                    fixture.bare.to_str().unwrap(),
                ],
            );
            let disabled_hooks = fixture.base.join("disabled-hooks");
            fs::create_dir_all(&disabled_hooks).unwrap();
            fixture.ok_at(
                &fixture.bare,
                &["config", "core.hooksPath", disabled_hooks.to_str().unwrap()],
            );
            fs::create_dir_all(&fixture.seed).unwrap();
            fixture.ok_at(&fixture.seed, &["init", "-b", "main"]);
            fixture.configure(&fixture.seed);
            fs::write(fixture.seed.join("tracked.txt"), "initial\n").unwrap();
            fixture.commit_at(&fixture.seed, "initial");
            fixture.ok_at(
                &fixture.seed,
                &["remote", "add", "origin", fixture.bare.to_str().unwrap()],
            );
            fixture.publish_seed();
            fixture.ok_at(
                &fixture.base,
                &[
                    "clone",
                    "-c",
                    "core.autocrlf=false",
                    "-c",
                    &format!("core.hooksPath={}", disabled_hooks.to_string_lossy()),
                    fixture.bare.to_str().unwrap(),
                    fixture.client.to_str().unwrap(),
                ],
            );
            fixture.configure(&fixture.client);
            assert!(
                fixture.git(&["status", "--porcelain"]).is_empty(),
                "fixture clone must start clean"
            );
            fixture
        }

        fn configure(&self, path: &Path) {
            for (key, value) in [
                ("user.name", "Enogram Remote Test"),
                ("user.email", "enogram@example.invalid"),
                ("commit.gpgsign", "false"),
                ("push.gpgsign", "false"),
                ("core.autocrlf", "false"),
            ] {
                self.ok_at(path, &["config", key, value]);
            }
            let hooks = path.join(".git/test-hooks");
            fs::create_dir_all(&hooks).unwrap();
            self.ok_at(path, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
        }

        fn output_at(&self, path: &Path, args: &[&str]) -> Output {
            Command::new(&self.git)
                .args(args)
                .current_dir(path)
                .env("GIT_AUTHOR_NAME", "Enogram Remote Test")
                .env("GIT_AUTHOR_EMAIL", "enogram@example.invalid")
                .env("GIT_COMMITTER_NAME", "Enogram Remote Test")
                .env("GIT_COMMITTER_EMAIL", "enogram@example.invalid")
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GCM_INTERACTIVE", "Never")
                .env("GIT_ASKPASS", "")
                .output()
                .expect("run disposable Git fixture command")
        }

        fn ok_at(&self, path: &Path, args: &[&str]) -> String {
            let output = self.output_at(path, args);
            assert!(
                output.status.success(),
                "fixture command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        }

        fn git(&self, args: &[&str]) -> String {
            self.ok_at(&self.client, args)
        }

        fn config(&self, key: &str, value: &str) {
            self.git(&["config", key, value]);
        }

        fn commit_at(&self, path: &Path, message: &str) -> String {
            self.ok_at(path, &["add", "--all"]);
            self.ok_at(path, &["commit", "-m", message]);
            self.ok_at(path, &["rev-parse", "HEAD"])
        }

        fn client_commit(&self, file: &str, content: &str) -> String {
            fs::write(self.client.join(file), content).unwrap();
            self.commit_at(&self.client, "client change")
        }

        fn seed_commit(&self, file: &str, content: &str) -> String {
            fs::write(self.seed.join(file), content).unwrap();
            self.commit_at(&self.seed, "remote change")
        }

        fn publish_seed(&self) {
            self.ok_at(
                &self.seed,
                &["push", "origin", "refs/heads/main:refs/heads/main"],
            );
        }

        fn snapshot(&self) -> RepositorySnapshot {
            inspect_repository(self.client.to_str().unwrap()).expect("inspect disposable client")
        }

        fn remote(&self, snapshot: &RepositorySnapshot) -> Remote {
            snapshot
                .remotes
                .iter()
                .find(|remote| remote.name == "origin")
                .unwrap()
                .clone()
        }

        fn fetch(&self, snapshot: &RepositorySnapshot) -> Result<MutationOutcome, GitError> {
            fetch_remote(
                self.client.to_str().unwrap(),
                "origin",
                &self.remote(snapshot).config_token,
                &snapshot.mutation_token,
            )
        }

        fn pull(&self, snapshot: &RepositorySnapshot) -> Result<MutationOutcome, GitError> {
            pull_remote(
                self.client.to_str().unwrap(),
                "origin",
                "main",
                &self.remote(snapshot).config_token,
                &snapshot.mutation_token,
            )
        }

        fn push(
            &self,
            branch: &str,
            snapshot: &RepositorySnapshot,
        ) -> Result<MutationOutcome, GitError> {
            push_remote(
                self.client.to_str().unwrap(),
                "origin",
                branch,
                &self.remote(snapshot).config_token,
                &snapshot.mutation_token,
            )
        }

        fn bare_ref(&self, reference: &str) -> String {
            self.ok_at(&self.bare, &["rev-parse", "--verify", reference])
        }

        fn hook(&self, name: &str, content: &str) {
            let path = self.client.join(".git/test-hooks").join(name);
            fs::write(&path, content).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }

        fn upload_pack_script(&self, body: &str) {
            self.hook(
                "test-upload-pack",
                &format!("#!/bin/sh\n{body}\nexec git-upload-pack \"$@\"\n"),
            );
            let script = self.client.join(".git/test-hooks/test-upload-pack");
            self.config(
                "remote.origin.uploadpack",
                &shell_quote(&script.to_string_lossy().replace('\\', "/")),
            );
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            // Only remove the uniquely created fixture beneath the temp root.
            if self.base.is_absolute() && self.base.parent() == Some(std::env::temp_dir().as_path())
            {
                let _ = fs::remove_dir_all(&self.base);
            }
        }
    }

    fn shell_quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\\''"))
    }

    #[test]
    fn remote_urls_are_display_only_and_do_not_expose_credentials() {
        for (url, expected) in [
            (
                "https://user:secret@example.invalid/team/repo.git?token=secret#fragment",
                "https://example.invalid/team/repo.git",
            ),
            (
                "ssh://git:secret@[::1]:22/team/repo",
                "ssh://[::1]:22/team/repo",
            ),
            (
                "git@example.invalid:team/repo.git",
                "example.invalid:team/repo.git",
            ),
            ("C:/repos/repo.git", "C:/repos/repo.git"),
            ("file:///C:/repos/repo.git", "file:///C:/repos/repo.git"),
            ("ext::echo password", "[configured URL hidden]"),
            (
                "https://example.invalid/repo\npassword",
                "[configured URL hidden]",
            ),
        ] {
            assert_eq!(display_url(url), expected);
        }
    }

    #[test]
    fn remote_list_includes_unusable_remotes_without_exposing_urls() {
        let fixture = Fixture::new();
        fixture.config("remote.empty.fetch", "+refs/heads/*:refs/remotes/empty/*");
        fixture.config(
            "remote.secret.url",
            "https://user:secret@example.invalid/repo?token=hidden",
        );
        fixture.config("remote.origin.mirror", "true");
        fixture.git(&[
            "config",
            "--add",
            "remote.origin.pushurl",
            fixture.bare.to_str().unwrap(),
        ]);
        fixture.git(&[
            "config",
            "--add",
            "remote.origin.pushurl",
            fixture.bare.to_str().unwrap(),
        ]);
        let snapshot = fixture.snapshot();
        assert_eq!(snapshot.remotes.len(), 3);
        let empty = snapshot
            .remotes
            .iter()
            .find(|remote| remote.name == "empty")
            .unwrap();
        assert!(empty.fetch_url.is_empty());
        assert!(empty.push_url.is_empty());
        let json = serde_json::to_string(&snapshot.remotes).unwrap();
        assert!(!json.contains("user:"));
        assert!(!json.contains("token=hidden"));
        assert!(json.contains("fetchUrl"));
        assert!(json.contains("configToken"));
        assert_eq!(
            fixture.push("main", &snapshot).unwrap_err().kind,
            GitErrorKind::Unsupported
        );
    }

    #[test]
    fn multiline_urls_and_rewrite_bases_are_hidden_and_rejected_before_network() {
        let fixture = Fixture::new();
        fixture.config(
            "remote.raw-control.url",
            "https://alice:top\nsecret@example.invalid/repo",
        );
        fixture.config("remote.rewritten-fetch.url", "fetch-alias:repo");
        fixture.config(
            "url.https://alice:top\tsecret@example.invalid/.insteadOf",
            "fetch-alias:",
        );
        fixture.config("remote.rewritten-push.url", "push-alias:repo");
        fixture.config(
            "url.https://bob:top\tsecret@example.invalid/.pushInsteadOf",
            "push-alias:",
        );
        let snapshot = fixture.snapshot();
        let json = serde_json::to_string(&snapshot.remotes).unwrap();
        assert!(!json.contains("alice"));
        assert!(!json.contains("bob"));
        assert!(!json.contains("secret@example"));
        for name in ["raw-control", "rewritten-fetch", "rewritten-push"] {
            let remote = snapshot
                .remotes
                .iter()
                .find(|remote| remote.name == name)
                .unwrap();
            assert_eq!(remote.push_url, "[configured URL hidden]");
            let error = push_remote(
                fixture.client.to_str().unwrap(),
                name,
                "published",
                &remote.config_token,
                &snapshot.mutation_token,
            )
            .unwrap_err();
            assert_eq!(error.kind, GitErrorKind::Unsupported);
            if name != "rewritten-push" {
                assert_eq!(remote.fetch_url, "[configured URL hidden]");
                assert_eq!(
                    fetch_remote(
                        fixture.client.to_str().unwrap(),
                        name,
                        &remote.config_token,
                        &snapshot.mutation_token
                    )
                    .unwrap_err()
                    .kind,
                    GitErrorKind::Unsupported
                );
            }
        }
    }

    #[test]
    fn fetch_updates_only_canonical_tracking_heads_and_preserves_tags_and_checkout() {
        let fixture = Fixture::new();
        let initial = fixture.git(&["rev-parse", "HEAD"]);
        let next = fixture.seed_commit("remote.txt", "remote\n");
        fixture.ok_at(&fixture.seed, &["branch", "topic"]);
        fixture.ok_at(
            &fixture.seed,
            &["tag", "-a", "remote-tag", "-m", "remote tag"],
        );
        fixture.ok_at(
            &fixture.seed,
            &["push", "origin", "main", "topic", "refs/tags/remote-tag"],
        );
        fixture.config("remote.origin.fetch", "+refs/heads/*:refs/heads/unsafe/*");
        fixture.config("fetch.prune", "true");
        fixture.config("fetch.pruneTags", "true");
        fixture.config("remote.origin.prune", "true");
        fixture.config("remote.origin.pruneTags", "true");
        fixture.config("remote.origin.tagOpt", "--tags");
        fixture.config("fetch.recurseSubmodules", "true");
        fixture.git(&["tag", "local-tag"]);
        fixture.git(&["update-ref", "refs/remotes/origin/stale", &initial]);
        fs::write(fixture.client.join(".git/FETCH_HEAD"), "keep fetch head\n").unwrap();
        let before = fixture.snapshot();
        let outcome = fixture.fetch(&before).expect("explicit safe fetch");
        assert_eq!(outcome.snapshot.head.as_deref(), Some(initial.as_str()));
        assert_eq!(outcome.snapshot.mutation_token, before.mutation_token);
        assert_eq!(
            fixture.git(&["rev-parse", "refs/remotes/origin/main"]),
            next
        );
        assert_eq!(
            fixture.git(&["rev-parse", "refs/remotes/origin/topic"]),
            next
        );
        assert_eq!(
            fixture.git(&["rev-parse", "refs/remotes/origin/stale"]),
            initial
        );
        assert_eq!(
            fixture.git(&["for-each-ref", "--format=%(refname)", "refs/heads/"]),
            "refs/heads/main"
        );
        assert_eq!(fixture.git(&["tag", "--list"]), "local-tag");
        assert_eq!(
            fs::read_to_string(fixture.client.join(".git/FETCH_HEAD")).unwrap(),
            "keep fetch head\n"
        );
    }

    #[test]
    fn pull_fast_forwards_and_noops_without_rebase_merge_commit_or_upstream_changes() {
        let fixture = Fixture::new();
        let next = fixture.seed_commit("tracked.txt", "updated remote\n");
        fixture.publish_seed();
        fixture.config("pull.rebase", "true");
        fixture.config("merge.ff", "false");
        fixture.config("merge.autoStash", "true");
        fixture.config("branch.main.mergeOptions", "--squash --no-ff");
        fixture.config("submodule.recurse", "true");
        let outcome = fixture
            .pull(&fixture.snapshot())
            .expect("fast-forward pull");
        assert_eq!(outcome.snapshot.head.as_deref(), Some(next.as_str()));
        assert!(outcome.commit_oid.is_none());
        assert!(outcome.snapshot.changes.is_empty());
        assert!(!outcome.snapshot.merge_in_progress);
        assert_eq!(
            fs::read_to_string(fixture.client.join("tracked.txt")).unwrap(),
            "updated remote\n"
        );
        assert_eq!(
            fixture.git(&["config", "branch.main.merge"]),
            "refs/heads/main"
        );
        let noop = fixture.pull(&outcome.snapshot).expect("up-to-date pull");
        assert_eq!(noop.snapshot.head, outcome.snapshot.head);
        let ahead = fixture.client_commit("client.txt", "ahead\n");
        let noop = fixture
            .pull(&fixture.snapshot())
            .expect("ahead checkout remains unchanged");
        assert_eq!(noop.snapshot.head.as_deref(), Some(ahead.as_str()));
    }

    #[test]
    fn pull_preserves_branch_signature_verification_requirements() {
        let fixture = Fixture::new();
        let initial = fixture.git(&["rev-parse", "HEAD"]);
        let unsigned = fixture.seed_commit("remote.txt", "unsigned tip\n");
        fixture.publish_seed();
        fixture.config("branch.main.mergeOptions", "--verify-signatures");
        let error = fixture.pull(&fixture.snapshot()).unwrap_err();
        assert_eq!(error.kind, GitErrorKind::GitCommandFailed);
        assert!(error.message.contains("signature requirements"));
        assert_eq!(fixture.git(&["rev-parse", "HEAD"]), initial);
        assert_eq!(
            fixture.git(&["rev-parse", "refs/remotes/origin/main"]),
            unsigned
        );
        assert!(fixture.git(&["status", "--porcelain"]).is_empty());
    }

    #[test]
    fn pull_divergence_keeps_local_history_and_reports_fetch_side_effects() {
        let fixture = Fixture::new();
        let local = fixture.client_commit("local.txt", "local\n");
        let remote = fixture.seed_commit("remote.txt", "remote\n");
        fixture.publish_seed();
        let error = fixture.pull(&fixture.snapshot()).unwrap_err();
        assert_eq!(error.kind, GitErrorKind::Unsupported);
        assert!(error.message.contains("diverged"));
        assert!(error.message.contains("Fetch has already"));
        assert_eq!(fixture.git(&["rev-parse", "HEAD"]), local);
        assert_eq!(
            fixture.git(&["rev-parse", "refs/remotes/origin/main"]),
            remote
        );
        assert!(fixture.git(&["status", "--porcelain"]).is_empty());
        assert!(!fixture.client.join(".git/MERGE_HEAD").exists());
    }

    #[test]
    fn pull_blocks_dirty_untracked_and_active_operation_before_fetch() {
        let fixture = Fixture::new();
        let initial = fixture.git(&["rev-parse", "HEAD"]);
        fixture.seed_commit("remote.txt", "remote\n");
        fixture.publish_seed();
        fs::write(fixture.client.join("untracked.txt"), "keep\n").unwrap();
        assert_eq!(
            fixture.pull(&fixture.snapshot()).unwrap_err().kind,
            GitErrorKind::Unsupported
        );
        fs::remove_file(fixture.client.join("untracked.txt")).unwrap();
        fs::write(fixture.client.join("tracked.txt"), "dirty\n").unwrap();
        assert_eq!(
            fixture.pull(&fixture.snapshot()).unwrap_err().kind,
            GitErrorKind::Unsupported
        );
        fixture.git(&["add", "tracked.txt"]);
        assert_eq!(
            fixture.pull(&fixture.snapshot()).unwrap_err().kind,
            GitErrorKind::Unsupported
        );
        fixture.git(&[
            "restore",
            "--source=HEAD",
            "--staged",
            "--worktree",
            "tracked.txt",
        ]);
        fs::write(
            fixture.client.join(".git/MERGE_HEAD"),
            format!("{initial}\n"),
        )
        .unwrap();
        assert!(fixture.pull(&fixture.snapshot()).is_err());
        assert_eq!(
            fixture.git(&["rev-parse", "refs/remotes/origin/main"]),
            initial
        );
    }

    #[test]
    fn push_first_publication_and_noop_affect_only_explicit_branch() {
        let fixture = Fixture::new();
        let initial = fixture.bare_ref("refs/heads/main");
        fixture.git(&["switch", "-c", "local-only"]);
        let local = fixture.client_commit("local.txt", "publish this\n");
        fixture.git(&["branch", "other-local"]);
        fixture.git(&["tag", "-a", "private-tag", "-m", "do not publish"]);
        fixture.config("push.default", "matching");
        fixture.config("push.followTags", "true");
        fixture.config("push.autoSetupRemote", "true");
        fixture.config("push.recurseSubmodules", "on-demand");
        fixture.config("remote.origin.push", "+refs/heads/*:refs/heads/*");
        fs::write(fixture.client.join("tracked.txt"), "uncommitted remains\n").unwrap();
        let outcome = fixture
            .push("published", &fixture.snapshot())
            .expect("publish one branch");
        assert_eq!(fixture.bare_ref("refs/heads/published"), local);
        assert_eq!(fixture.bare_ref("refs/heads/main"), initial);
        assert_eq!(
            fixture.ok_at(&fixture.bare, &["for-each-ref", "--format=%(refname)"]),
            "refs/heads/main\nrefs/heads/published"
        );
        assert!(!fixture
            .output_at(
                &fixture.client,
                &["config", "--get", "branch.local-only.remote"]
            )
            .status
            .success());
        assert_eq!(
            fs::read_to_string(fixture.client.join("tracked.txt")).unwrap(),
            "uncommitted remains\n"
        );
        let noop = fixture
            .push("published", &outcome.snapshot)
            .expect("up-to-date push");
        assert_eq!(noop.snapshot.head, outcome.snapshot.head);
    }

    #[test]
    fn push_rejects_non_fast_forward_without_force() {
        let fixture = Fixture::new();
        fixture.client_commit("local.txt", "local\n");
        let remote = fixture.seed_commit("remote.txt", "remote\n");
        fixture.publish_seed();
        let error = fixture.push("main", &fixture.snapshot()).unwrap_err();
        assert_eq!(error.kind, GitErrorKind::GitCommandFailed);
        assert!(error.message.contains("non-fast-forward"));
        assert!(error.message.contains("do not automatically retry"));
        assert_eq!(fixture.bare_ref("refs/heads/main"), remote);
    }

    #[test]
    fn stale_checkout_remote_config_and_remote_identity_are_rejected() {
        let fixture = Fixture::new();
        let before = fixture.snapshot();
        fs::write(fixture.client.join("new.txt"), "staged\n").unwrap();
        fixture.git(&["add", "new.txt"]);
        assert_eq!(
            fixture.fetch(&before).unwrap_err().kind,
            GitErrorKind::StaleState
        );
        let before = fixture.snapshot();
        fixture.config(
            "remote.origin.url",
            fixture.base.join("missing.git").to_str().unwrap(),
        );
        assert_eq!(
            fixture.push("published", &before).unwrap_err().kind,
            GitErrorKind::StaleState
        );
        fixture.config("remote.origin.url", fixture.bare.to_str().unwrap());
        fixture.git(&["remote", "add", "second", fixture.bare.to_str().unwrap()]);
        let before = fixture.snapshot();
        let error = fetch_remote(
            fixture.client.to_str().unwrap(),
            "second",
            &fixture.remote(&before).config_token,
            &before.mutation_token,
        )
        .unwrap_err();
        assert_eq!(error.kind, GitErrorKind::StaleState);
        assert!(!fixture
            .output_at(
                &fixture.bare,
                &["show-ref", "--verify", "refs/heads/published"]
            )
            .status
            .success());
    }

    #[test]
    fn invalid_arguments_do_not_become_options_urls_or_refspecs() {
        let fixture = Fixture::new();
        let before = fixture.snapshot();
        let token = fixture.remote(&before).config_token;
        let refs_before = fixture.ok_at(&fixture.bare, &["show-ref"]);
        for remote in [
            "--all",
            "https://example.invalid/repo",
            ".",
            "../elsewhere",
            "origin\nsecond",
        ] {
            assert!(fetch_remote(
                fixture.client.to_str().unwrap(),
                remote,
                &token,
                &before.mutation_token
            )
            .is_err());
        }
        for branch in [
            "",
            "--all",
            "main:refs/heads/other",
            "@{-1}",
            "../main",
            "refs/heads/main",
            "HEAD",
            "*",
        ] {
            assert!(push_remote(
                fixture.client.to_str().unwrap(),
                "origin",
                branch,
                &token,
                &before.mutation_token
            )
            .is_err());
        }
        assert_eq!(fixture.ok_at(&fixture.bare, &["show-ref"]), refs_before);
    }

    #[test]
    fn push_rejects_multiple_destinations_mirror_and_unsafe_tracking_mappings() {
        let fixture = Fixture::new();
        let refs_before = fixture.ok_at(&fixture.bare, &["show-ref"]);
        fixture.git(&[
            "config",
            "--add",
            "remote.origin.pushurl",
            fixture.bare.to_str().unwrap(),
        ]);
        fixture.git(&[
            "config",
            "--add",
            "remote.origin.pushurl",
            fixture.bare.to_str().unwrap(),
        ]);
        assert_eq!(
            fixture
                .push("published", &fixture.snapshot())
                .unwrap_err()
                .kind,
            GitErrorKind::Unsupported
        );
        fixture.git(&["config", "--unset-all", "remote.origin.pushurl"]);
        fixture.config("remote.origin.mirror", "true");
        assert_eq!(
            fixture
                .push("published", &fixture.snapshot())
                .unwrap_err()
                .kind,
            GitErrorKind::Unsupported
        );
        fixture.config("remote.origin.mirror", "false");
        fixture.config("remote.origin.fetch", "+refs/heads/*:refs/heads/*");
        assert_eq!(
            fixture
                .push("published", &fixture.snapshot())
                .unwrap_err()
                .kind,
            GitErrorKind::Unsupported
        );
        fixture.config("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*");
        fixture.git(&[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/heads/main",
        ]);
        assert_eq!(
            fixture.fetch(&fixture.snapshot()).unwrap_err().kind,
            GitErrorKind::Unsupported
        );
        assert_eq!(
            fixture
                .push("published", &fixture.snapshot())
                .unwrap_err()
                .kind,
            GitErrorKind::Unsupported
        );
        assert_eq!(fixture.ok_at(&fixture.bare, &["show-ref"]), refs_before);
    }

    #[test]
    fn pull_and_push_reject_dangling_tracking_symrefs_before_writes() {
        let fixture = Fixture::new();
        let initial = fixture.git(&["rev-parse", "HEAD"]);
        fixture.seed_commit("remote.txt", "remote\n");
        fixture.publish_seed();
        fixture.git(&[
            "symbolic-ref",
            "refs/remotes/origin/main",
            "refs/heads/unrequested",
        ]);
        fixture.git(&[
            "symbolic-ref",
            "refs/remotes/origin/published",
            "refs/heads/unrequested",
        ]);
        let snapshot = fixture.snapshot();
        assert_eq!(
            fixture.pull(&snapshot).unwrap_err().kind,
            GitErrorKind::Unsupported
        );
        assert_eq!(
            fixture.push("published", &snapshot).unwrap_err().kind,
            GitErrorKind::Unsupported
        );
        assert_eq!(fixture.git(&["rev-parse", "HEAD"]), initial);
        assert!(!fixture
            .output_at(
                &fixture.client,
                &["show-ref", "--verify", "refs/heads/unrequested"]
            )
            .status
            .success());
        assert!(!fixture
            .output_at(
                &fixture.bare,
                &["show-ref", "--verify", "refs/heads/published"]
            )
            .status
            .success());
    }

    #[test]
    fn push_preserves_hook_rejection_and_redacts_hook_diagnostics() {
        let fixture = Fixture::new();
        fixture.hook("pre-push", "#!/bin/sh\nprintf ran > .git/hook-ran\necho 'Authentication failed https://user:fake-secret@example.invalid/repo?token=fake-secret' >&2\nexit 1\n");
        let error = fixture.push("published", &fixture.snapshot()).unwrap_err();
        assert!(fixture.client.join(".git/hook-ran").exists());
        assert_eq!(error.kind, GitErrorKind::GitCommandFailed);
        assert!(error.message.contains("authentication"));
        assert!(!error.message.contains("fake-secret"));
        assert!(!fixture
            .output_at(
                &fixture.bare,
                &["show-ref", "--verify", "refs/heads/published"]
            )
            .status
            .success());
    }

    #[test]
    fn push_reports_uncertainty_when_hook_changes_checkout_after_source_capture() {
        let fixture = Fixture::new();
        let before = fixture.snapshot();
        fixture.hook(
            "pre-push",
            "#!/bin/sh\nprintf changed > drift.txt\ngit add -- drift.txt\n",
        );
        let error = fixture.push("published", &before).unwrap_err();
        assert_eq!(error.kind, GitErrorKind::StaleState);
        assert!(error.message.contains("remote operation already ran"));
        assert_eq!(
            fixture.bare_ref("refs/heads/published"),
            before.head.unwrap()
        );
        assert!(fixture
            .git(&["status", "--porcelain"])
            .contains("drift.txt"));
    }

    #[test]
    fn pull_rechecks_config_after_fetch_and_keeps_checkout_unchanged() {
        let fixture = Fixture::new();
        let initial = fixture.git(&["rev-parse", "HEAD"]);
        let remote = fixture.seed_commit("remote.txt", "remote\n");
        fixture.publish_seed();
        let client = shell_quote(&fixture.client.to_string_lossy().replace('\\', "/"));
        fixture.upload_pack_script(&format!("git -C {client} config enogram.test changed"));
        let error = fixture.pull(&fixture.snapshot()).unwrap_err();
        assert_eq!(error.kind, GitErrorKind::StaleState);
        assert!(error.message.contains("Fetch has already"));
        assert_eq!(fixture.git(&["rev-parse", "HEAD"]), initial);
        assert_eq!(
            fixture.git(&["rev-parse", "refs/remotes/origin/main"]),
            remote
        );
    }

    #[test]
    fn pull_rechecks_worktree_after_fetch_without_autostash() {
        let fixture = Fixture::new();
        let initial = fixture.git(&["rev-parse", "HEAD"]);
        fixture.seed_commit("remote.txt", "remote\n");
        fixture.publish_seed();
        let drift = shell_quote(
            &fixture
                .client
                .join("drift.txt")
                .to_string_lossy()
                .replace('\\', "/"),
        );
        fixture.upload_pack_script(&format!("printf keep > {drift}"));
        let error = fixture.pull(&fixture.snapshot()).unwrap_err();
        assert_eq!(error.kind, GitErrorKind::Unsupported);
        assert!(error.message.contains("clean working tree"));
        assert_eq!(fixture.git(&["rev-parse", "HEAD"]), initial);
        assert_eq!(
            fs::read_to_string(fixture.client.join("drift.txt")).unwrap(),
            "keep"
        );
        assert!(fixture.git(&["stash", "list"]).is_empty());
    }

    #[test]
    fn pull_preserves_ignored_files_that_remote_would_overwrite() {
        let fixture = Fixture::new();
        let initial = fixture.git(&["rev-parse", "HEAD"]);
        fixture.seed_commit("ignored.txt", "remote version\n");
        fixture.publish_seed();
        fs::write(fixture.client.join(".git/info/exclude"), "ignored.txt\n").unwrap();
        fs::write(
            fixture.client.join("ignored.txt"),
            "local ignored version\n",
        )
        .unwrap();
        assert!(fixture.snapshot().changes.is_empty());
        let error = fixture.pull(&fixture.snapshot()).unwrap_err();
        assert_eq!(error.kind, GitErrorKind::GitCommandFailed);
        assert!(error.message.contains("overwrite local files"));
        assert_eq!(fixture.git(&["rev-parse", "HEAD"]), initial);
        assert_eq!(
            fs::read_to_string(fixture.client.join("ignored.txt")).unwrap(),
            "local ignored version\n"
        );
    }

    #[test]
    fn push_reports_config_drift_after_successful_publication() {
        let fixture = Fixture::new();
        fixture.hook("pre-push", "#!/bin/sh\ngit config enogram.test changed\n");
        let before = fixture.snapshot();
        let error = fixture.push("published", &before).unwrap_err();
        assert_eq!(error.kind, GitErrorKind::StaleState);
        assert!(error.message.contains("remote operation already ran"));
        assert_eq!(
            fixture.bare_ref("refs/heads/published"),
            before.head.unwrap()
        );
    }

    #[test]
    fn unavailable_local_remote_fails_promptly_with_bounded_diagnostics() {
        let fixture = Fixture::new();
        fixture.config(
            "remote.origin.url",
            fixture.base.join("missing.git").to_str().unwrap(),
        );
        let before = fixture.snapshot();
        let started = Instant::now();
        let error = fixture.fetch(&before).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(15));
        assert_eq!(error.kind, GitErrorKind::GitCommandFailed);
        assert!(error.message.len() < 1000);
        assert!(!error.message.contains("missing.git"));
        assert!(error.message.contains("Refresh"));
    }
}
