//! Local remote settings. Git edits a private copy while config.lock excludes
//! normal Git writers; one rename publishes all requested keys together.

use super::*;
use std::fs::{File, OpenOptions, Permissions};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_INPUT_URL_BYTES: usize = 64 * 1024;
static NEXT_CONFIG_COPY: AtomicU64 = AtomicU64::new(1);

pub(super) struct RemoteSettingsMetadata {
    pub configured_fetch_url: Option<String>,
    pub configured_push_url: Option<String>,
    pub has_explicit_push_url: bool,
    pub edit_blocked_reason: Option<String>,
}

pub(super) fn remote_settings_metadata(
    context: &RepositoryContext,
    config: &EffectiveConfig,
    name: &str,
) -> Result<RemoteSettingsMetadata, GitError> {
    let fetch_values = config_values(config, &format!("remote.{name}.url"));
    let push_values = config_values(config, &format!("remote.{name}.pushurl"));
    let path = common_config_path(context)?;
    Ok(RemoteSettingsMetadata {
        configured_fetch_url: editable_configured_value(&fetch_values),
        configured_push_url: editable_configured_value(&push_values),
        has_explicit_push_url: !push_values.is_empty(),
        edit_blocked_reason: edit_blocked_reason(context, config, name, &path),
    })
}

fn editable_configured_value(values: &[&str]) -> Option<String> {
    let [value] = values else { return None };
    (remote_url_is_safe(value) && display_url(value) == *value).then(|| (*value).to_owned())
}

/// Add a local remote and its canonical fetch refspec without contacting it.
pub fn add_remote(
    repo_path: &str,
    name: &str,
    fetch_url: &str,
    push_url: Option<&str>,
    expected_token: &str,
) -> Result<MutationOutcome, GitError> {
    let _guard = acquire_mutation_lock()?;
    let context = repository_context(repo_path)?;
    validate_token(&context, expected_token)?;
    validate_settings_name(&context, name)?;
    validate_settings_url(fetch_url)?;
    if let Some(url) = push_url {
        validate_settings_url(url)?;
    }
    let config = read_effective_config(&context)?;
    if config.remote_names().contains(name) {
        return Err(settings_error(
            "A remote with this name already exists. Choose a different name or edit the existing remote.",
        ));
    }
    if has_url_conditional_include(&config) {
        return Err(settings_error(url_include_message()));
    }
    let mut edits = vec![
        (format!("remote.{name}.url"), Some(fetch_url.to_owned())),
        (
            format!("remote.{name}.fetch"),
            Some(format!("+refs/heads/*:refs/remotes/{name}/*")),
        ),
    ];
    if let Some(url) = push_url {
        edits.push((format!("remote.{name}.pushurl"), Some(url.to_owned())));
    }
    save_remote_settings(&context, &config, &edits, expected_token)
}

/// The remote name is immutable. None preserves fetch; update_push_url controls
/// whether None preserves push or removes its explicit URL to inherit fetch.
pub fn update_remote(
    repo_path: &str,
    remote_name: &str,
    fetch_url: Option<&str>,
    push_url: Option<&str>,
    update_push_url: bool,
    expected_remote_token: &str,
    expected_token: &str,
) -> Result<MutationOutcome, GitError> {
    let _guard = acquire_mutation_lock()?;
    let context = repository_context(repo_path)?;
    validate_token(&context, expected_token)?;
    validate_settings_name(&context, remote_name)?;
    if !update_push_url && push_url.is_some() {
        return Err(settings_error(
            "A push URL was supplied without requesting a push URL change.",
        ));
    }
    if fetch_url.is_none() && !update_push_url {
        return Err(settings_error(
            "Choose a fetch or push URL change before saving remote settings.",
        ));
    }
    if let Some(url) = fetch_url {
        validate_settings_url(url)?;
    }
    if let Some(url) = push_url {
        validate_settings_url(url)?;
    }
    let config = read_effective_config(&context)?;
    if !config.remote_names().contains(remote_name) {
        return Err(stale_settings(
            "The selected remote no longer exists. Refresh before editing its settings.",
        ));
    }
    if expected_remote_token.is_empty()
        || remote_token(&context, &config, remote_name)? != expected_remote_token
    {
        return Err(stale_settings("The selected remote or Git configuration changed since this form was opened. Refresh before editing its settings."));
    }
    let path = common_config_path(&context)?;
    if let Some(reason) = edit_blocked_reason(&context, &config, remote_name, &path) {
        return Err(settings_error(reason));
    }
    let mut edits = Vec::new();
    if let Some(url) = fetch_url {
        edits.push((format!("remote.{remote_name}.url"), Some(url.to_owned())));
    }
    if update_push_url {
        edits.push((
            format!("remote.{remote_name}.pushurl"),
            push_url.map(str::to_owned),
        ));
    }
    save_remote_settings(&context, &config, &edits, expected_token)
}

fn validate_settings_name(context: &RepositoryContext, name: &str) -> Result<(), GitError> {
    if !remote_name_shape_is_safe(name) {
        return Err(settings_error(
            "Enter a valid remote name without whitespace, a URL, or command options.",
        ));
    }
    check_ref_name(context, &format!("refs/remotes/{name}/enogram-check"))
}

fn validate_settings_url(url: &str) -> Result<(), GitError> {
    let mut valid =
        url.len() <= MAX_INPUT_URL_BYTES && url.trim() == url && remote_url_is_safe(url);
    if let Some((scheme, rest)) = url.split_once("://") {
        // Git accepts local paths with spaces; network authorities cannot be
        // empty or whitespace. Leave credential parsing and transport to Git.
        let authority = rest.split('/').next().unwrap_or("");
        valid &= !rest.is_empty()
            && (scheme.eq_ignore_ascii_case("file")
                || (!authority.is_empty() && !authority.chars().any(char::is_whitespace)));
    }
    if !valid {
        return Err(settings_error("Enter a standard Git URL or local repository path without surrounding whitespace, control characters, command options, or a custom transport helper."));
    }
    Ok(())
}

fn config_values<'a>(config: &'a EffectiveConfig, key: &str) -> Vec<&'a str> {
    config
        .entries
        .iter()
        .filter_map(|(candidate, value)| (candidate == key).then_some(value.as_str()))
        .collect()
}

fn has_url_conditional_include(config: &EffectiveConfig) -> bool {
    config.entries.iter().any(|(key, _)| {
        key.strip_prefix("includeif.").is_some_and(|condition| {
            condition
                .to_ascii_lowercase()
                .starts_with("hasconfig:remote.*.url:")
                && condition.ends_with(".path")
        })
    })
}

fn url_include_message() -> &'static str {
    "This repository uses URL-dependent conditional includes. Edit its remote settings in Git to preserve those configuration rules."
}

fn edit_blocked_reason(
    context: &RepositoryContext,
    config: &EffectiveConfig,
    name: &str,
    config_path: &Path,
) -> Option<String> {
    if let Err(error) = regular_config_metadata(config_path) {
        return Some(error.message);
    }
    if !remote_name_shape_is_safe(name) {
        return Some("This remote has an unsupported name. Edit its settings in Git.".to_owned());
    }
    if has_url_conditional_include(config) {
        return Some(url_include_message().to_owned());
    }
    let prefix = format!("remote.{name}.");
    if config
        .entries
        .iter()
        .zip(&config.origins)
        .any(|((key, _), origin)| {
            key.strip_prefix(&prefix)
                .is_some_and(|suffix| !suffix.contains('.'))
                && !origin_is_direct_local(context, origin, config_path)
        })
    {
        return Some("This remote uses inherited, included, or worktree-specific configuration. Edit it in Git so those scopes remain intact.".to_owned());
    }
    if config_values(config, &format!("remote.{name}.url")).len() > 1
        || config_values(config, &format!("remote.{name}.pushurl")).len() > 1
    {
        return Some("This remote has multiple fetch or push URLs. Edit it in Git to preserve every destination.".to_owned());
    }
    if config
        .last(&format!("remote.{name}.vcs"))
        .is_some_and(|value| !value.is_empty())
    {
        return Some("This remote uses a custom transport. Edit its settings in Git.".to_owned());
    }
    None
}

fn origin_is_direct_local(context: &RepositoryContext, origin: &ConfigOrigin, path: &Path) -> bool {
    if origin.scope != "local" {
        return false;
    }
    let Some(source) = origin.source.strip_prefix("file:") else {
        return false;
    };
    let source = Path::new(source);
    let source = if source.is_absolute() {
        source.to_path_buf()
    } else {
        context.root.join(source)
    };
    fs::canonicalize(source).is_ok_and(|source| source == path)
}

fn common_config_path(context: &RepositoryContext) -> Result<PathBuf, GitError> {
    let output = run_git(
        &context.git,
        &context.root,
        ["rev-parse", "--path-format=absolute", "--git-common-dir"],
        64 * 1024,
        false,
    )?;
    if !output.status.success() {
        return Err(settings_io_error(
            "Git could not locate the shared repository configuration.",
        ));
    }
    let path = output_text(&output.stdout, "shared Git directory")?.trim_end_matches(['\r', '\n']);
    let common = fs::canonicalize(path)
        .map_err(|_| settings_io_error("The shared Git directory is inaccessible."))?;
    Ok(common.join("config"))
}

fn regular_config_metadata(path: &Path) -> Result<fs::Metadata, GitError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| settings_io_error("The repository configuration is missing or inaccessible. Edit it in Git before using remote settings."))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(settings_error("The repository configuration is a symlink or a non-regular file. Edit its remote settings in Git."));
    }
    if metadata.permissions().readonly() {
        return Err(settings_error("The repository configuration is read-only. Make it writable in Git before editing remote settings."));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() > 1 {
            return Err(settings_error("The repository configuration has multiple hard links. Edit its remote settings in Git."));
        }
    }
    Ok(metadata)
}

type ConfigEdit = (String, Option<String>);

fn save_remote_settings(
    context: &RepositoryContext,
    config: &EffectiveConfig,
    edits: &[ConfigEdit],
    expected_token: &str,
) -> Result<MutationOutcome, GitError> {
    let mut transaction = ConfigTransaction::begin(context)?;
    ensure_config_unchanged(context, config)?;
    let original_entries = read_file_config(context, &transaction.config_path)?;
    for (key, value) in edits {
        // Removing an absent pushurl already expresses inheritance. Do not ask
        // Git to unset it, because that deliberately returns a failure status.
        if value.is_none()
            && !original_entries
                .iter()
                .any(|(candidate, _)| candidate == key)
        {
            continue;
        }
        transaction.edit(context, key, value.as_deref())?;
    }
    let staged_entries = read_file_config(context, transaction.staged_path.as_ref().unwrap())?;
    verify_entries_change(&original_entries, &staged_entries, edits)?;
    let staged_bytes = read_config_bytes(transaction.staged_path.as_ref().unwrap())?;
    // Git writers honor config.lock. Other files and external editors do not:
    // recheck both the effective configuration and source bytes before publish.
    validate_token(context, expected_token)?;
    ensure_config_unchanged(context, config)?;
    transaction.ensure_source_unchanged()?;
    transaction.commit(&staged_bytes)?;
    let config_path = transaction.config_path.clone();
    drop(transaction);

    validate_token(context, expected_token).map_err(after_settings_write)?;
    let saved_config = read_effective_config(context).map_err(after_settings_write)?;
    verify_effective_change(config, &saved_config, edits).map_err(after_settings_write)?;
    if read_config_bytes(&config_path).map_err(after_settings_write)? != staged_bytes {
        return Err(after_settings_write(stale_settings(
            "Git configuration changed after saving remote settings.",
        )));
    }
    let outcome = finish_mutation(context, "Remote settings save", None)?;
    ensure_config_unchanged(context, &saved_config).map_err(after_settings_write)?;
    if outcome.snapshot.mutation_token != expected_token {
        return Err(after_settings_write(stale_settings(
            "Repository state changed after saving remote settings.",
        )));
    }
    Ok(outcome)
}

fn read_file_config(
    context: &RepositoryContext,
    path: &Path,
) -> Result<Vec<(String, String)>, GitError> {
    let output = run_git(
        &context.git,
        path.parent().unwrap(),
        [
            OsString::from("config"),
            OsString::from("--file"),
            path.file_name().unwrap().to_owned(),
            OsString::from("--no-includes"),
            OsString::from("--null"),
            OsString::from("--list"),
        ],
        MAX_CONFIG_BYTES,
        false,
    )?;
    if !output.status.success() {
        return Err(settings_io_error("Git could not parse the staged remote settings. The repository configuration was not changed."));
    }
    let mut entries = Vec::new();
    for field in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
    {
        let entry = output_text(field, "Git configuration")?;
        let (key, value) = entry.split_once('\n').unwrap_or((entry, ""));
        entries.push((key.to_owned(), value.to_owned()));
    }
    Ok(entries)
}

fn verify_entries_change(
    before: &[(String, String)],
    after: &[(String, String)],
    edits: &[ConfigEdit],
) -> Result<(), GitError> {
    let untouched = |key: &str| !edits.iter().any(|(edited, _)| edited == key);
    let unchanged_before = before
        .iter()
        .filter(|(key, _)| untouched(key))
        .collect::<Vec<_>>();
    let unchanged_after = after
        .iter()
        .filter(|(key, _)| untouched(key))
        .collect::<Vec<_>>();
    if unchanged_before != unchanged_after
        || edits.iter().any(|(key, value)| {
            let actual = after
                .iter()
                .filter_map(|(candidate, value)| (candidate == key).then_some(value.as_str()))
                .collect::<Vec<_>>();
            let expected = value.as_deref().into_iter().collect::<Vec<_>>();
            actual != expected
        })
    {
        return Err(stale_settings("Git configuration did not match the requested remote settings. Refresh and inspect it before another action."));
    }
    Ok(())
}

fn verify_effective_change(
    before: &EffectiveConfig,
    after: &EffectiveConfig,
    edits: &[ConfigEdit],
) -> Result<(), GitError> {
    verify_entries_change(&before.entries, &after.entries, edits)?;
    let untouched =
        |entry: &((String, String), ConfigOrigin)| !edits.iter().any(|(key, _)| key == &entry.0 .0);
    let before = before
        .entries
        .iter()
        .cloned()
        .zip(before.origins.iter().cloned())
        .filter(untouched)
        .collect::<Vec<_>>();
    let after = after
        .entries
        .iter()
        .cloned()
        .zip(after.origins.iter().cloned())
        .filter(untouched)
        .collect::<Vec<_>>();
    if before != after {
        return Err(stale_settings(
            "Effective Git configuration changed while remote settings were being saved.",
        ));
    }
    Ok(())
}

struct ConfigTransaction {
    config_path: PathBuf,
    lock_path: PathBuf,
    lock_file: Option<File>,
    owns_lock: bool,
    staged_path: Option<PathBuf>,
    original_bytes: Vec<u8>,
    permissions: Option<Permissions>,
}

impl ConfigTransaction {
    fn begin(context: &RepositoryContext) -> Result<Self, GitError> {
        let config_path = common_config_path(context)?;
        regular_config_metadata(&config_path)?;
        let lock_path = config_path.with_file_name("config.lock");
        let lock_file = private_new_file(&lock_path).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                GitError::new(GitErrorKind::Busy, "The Git configuration is locked by another operation. Wait for it to finish and refresh.")
            } else {
                settings_io_error("Could not lock the repository configuration. No remote settings were changed.")
            }
        })?;
        let mut transaction = Self {
            config_path,
            lock_path,
            lock_file: Some(lock_file),
            owns_lock: true,
            staged_path: None,
            original_bytes: Vec::new(),
            permissions: None,
        };
        // Read only after holding Git's common-config lock, including worktrees.
        transaction.permissions =
            Some(regular_config_metadata(&transaction.config_path)?.permissions());
        transaction.original_bytes = read_config_bytes(&transaction.config_path)?;
        for _ in 0..16 {
            let id = NEXT_CONFIG_COPY.fetch_add(1, Ordering::Relaxed);
            let path = transaction
                .config_path
                .with_file_name(format!("config.enogram-{}-{id}", std::process::id()));
            match private_new_file(&path) {
                Ok(mut file) => {
                    transaction.staged_path = Some(path);
                    file.write_all(&transaction.original_bytes).map_err(|_| settings_io_error("Could not stage remote settings. The repository configuration was not changed."))?;
                    return Ok(transaction);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err(settings_io_error("Could not stage remote settings. The repository configuration was not changed.")),
            }
        }
        Err(settings_io_error(
            "Could not allocate a private configuration copy. No remote settings were changed.",
        ))
    }

    fn edit(
        &self,
        context: &RepositoryContext,
        key: &str,
        value: Option<&str>,
    ) -> Result<(), GitError> {
        let mut args = vec![
            OsString::from("config"),
            OsString::from("--file"),
            self.staged_path
                .as_ref()
                .unwrap()
                .file_name()
                .unwrap()
                .to_owned(),
            OsString::from("--no-includes"),
            OsString::from(if value.is_some() {
                "--replace-all"
            } else {
                "--unset-all"
            }),
            OsString::from(key),
        ];
        if let Some(value) = value {
            args.push(OsString::from(value));
        }
        // Git for Windows does not accept Rust's canonical \\?\ path prefix
        // for --file. A filename relative to this directory also keeps UNC and
        // linked-worktree config paths literal, without rewriting path strings.
        let output = run_git(
            &context.git,
            self.config_path.parent().unwrap(),
            args,
            MAX_CONFIG_BYTES,
            false,
        )?;
        if !output.status.success() {
            // Git diagnostics can quote a credential-bearing value or path.
            return Err(settings_io_error("Git could not stage the remote URL change. The repository configuration was not changed."));
        }
        Ok(())
    }

    fn ensure_source_unchanged(&self) -> Result<(), GitError> {
        regular_config_metadata(&self.config_path)?;
        if read_config_bytes(&self.config_path)? != self.original_bytes {
            return Err(stale_settings("The repository configuration changed while editing remote settings. Refresh before saving."));
        }
        Ok(())
    }

    fn commit(&mut self, bytes: &[u8]) -> Result<(), GitError> {
        let mut file = self.lock_file.take().unwrap();
        file.write_all(bytes).map_err(|_| settings_io_error("Could not write the remote settings transaction. The repository configuration was not changed."))?;
        if let Some(permissions) = &self.permissions {
            file.set_permissions(permissions.clone()).map_err(|_| settings_io_error("Could not preserve repository configuration permissions. No remote settings were changed."))?;
        }
        file.sync_all().map_err(|_| settings_io_error("Could not flush the remote settings transaction. The repository configuration was not changed."))?;
        drop(file);
        // Same-directory rename replaces the regular config file in one step on
        // Unix and Windows. Never delete the destination or retry a failed save.
        fs::rename(&self.lock_path, &self.config_path).map_err(|_| {
            settings_io_error(
                "Could not publish remote settings. The repository configuration was not changed.",
            )
        })?;
        self.owns_lock = false;
        Ok(())
    }
}

impl Drop for ConfigTransaction {
    fn drop(&mut self) {
        drop(self.lock_file.take());
        if self.owns_lock {
            let _ = fs::remove_file(&self.lock_path);
        }
        if let Some(path) = &self.staged_path {
            let _ = fs::remove_file(path);
            // This private path was created uniquely by this transaction. Git
            // can leave its child lock behind if its bounded process is killed.
            let mut lock = path.as_os_str().to_owned();
            lock.push(".lock");
            let _ = fs::remove_file(PathBuf::from(lock));
        }
    }
}

fn private_new_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn read_config_bytes(path: &Path) -> Result<Vec<u8>, GitError> {
    let file = File::open(path).map_err(|_| {
        settings_io_error(
            "Could not read repository configuration. No remote settings were changed.",
        )
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            settings_io_error(
                "Could not read repository configuration. No remote settings were changed.",
            )
        })?;
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(GitError::new(
            GitErrorKind::OutputLimit,
            "The repository configuration exceeds the remote settings size limit.",
        ));
    }
    Ok(bytes)
}

fn settings_error(message: impl Into<String>) -> GitError {
    GitError::new(GitErrorKind::Unsupported, message)
}
fn settings_io_error(message: impl Into<String>) -> GitError {
    GitError::new(GitErrorKind::GitCommandFailed, message)
}
fn stale_settings(message: impl Into<String>) -> GitError {
    GitError::new(GitErrorKind::StaleState, message)
}
fn after_settings_write(error: GitError) -> GitError {
    GitError::new(error.kind, format!("Remote settings were saved, but the resulting state could not be verified. {} Refresh and inspect the settings; do not automatically repeat the save.", error.message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Output;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

    struct Fixture {
        root: PathBuf,
        git: PathBuf,
        _serial: MutexGuard<'static, ()>,
    }

    impl Fixture {
        fn new() -> Self {
            let serial = crate::tests::MUTATION_TEST_LOCK
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "enogram-remote-settings-{}-{id}",
                std::process::id()
            ));
            fs::create_dir(&root).unwrap();
            let fixture = Self {
                root,
                git: resolve_git().unwrap(),
                _serial: serial,
            };
            fixture.git(&["init", "-b", "main"]);
            fixture.git(&["config", "user.name", "Enogram Remote Settings Test"]);
            fixture.git(&["config", "user.email", "enogram@example.invalid"]);
            fixture.git(&["config", "commit.gpgsign", "false"]);
            fixture.git(&["config", "core.autocrlf", "false"]);
            let hooks = fixture.root.join(".git/disabled-hooks");
            fs::create_dir(&hooks).unwrap();
            fixture.git(&["config", "core.hooksPath", hooks.to_str().unwrap()]);
            fs::write(fixture.root.join("tracked.txt"), "initial\n").unwrap();
            fixture.git(&["add", "tracked.txt"]);
            fixture.git(&["commit", "-m", "fixture"]);
            fixture
        }

        fn output(&self, path: &Path, args: &[&str]) -> Output {
            Command::new(&self.git)
                .args(args)
                .current_dir(path)
                .env("GIT_OPTIONAL_LOCKS", "0")
                .env("GIT_AUTHOR_NAME", "Enogram Remote Settings Test")
                .env("GIT_AUTHOR_EMAIL", "enogram@example.invalid")
                .env("GIT_COMMITTER_NAME", "Enogram Remote Settings Test")
                .env("GIT_COMMITTER_EMAIL", "enogram@example.invalid")
                .output()
                .unwrap()
        }

        fn git_at(&self, path: &Path, args: &[&str]) -> String {
            let output = self.output(path, args);
            assert!(
                output.status.success(),
                "fixture Git failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout)
                .unwrap()
                .trim_end_matches(['\r', '\n'])
                .to_owned()
        }

        fn git(&self, args: &[&str]) -> String {
            self.git_at(&self.root, args)
        }
        fn context(&self) -> RepositoryContext {
            repository_context(self.root.to_str().unwrap()).unwrap()
        }
        fn snapshot(&self) -> RepositorySnapshot {
            inspect_repository(self.root.to_str().unwrap()).unwrap()
        }
        fn config_path(&self) -> PathBuf {
            self.root.join(".git/config")
        }
        fn config_bytes(&self) -> Vec<u8> {
            fs::read(self.config_path()).unwrap()
        }

        fn origin(&self) {
            self.git(&[
                "remote",
                "add",
                "origin",
                "https://example.invalid/original.git",
            ]);
            self.git(&["config", "branch.main.remote", "origin"]);
            self.git(&["config", "branch.main.merge", "refs/heads/main"]);
        }

        fn remote(&self, snapshot: &RepositorySnapshot) -> Remote {
            snapshot
                .remotes
                .iter()
                .find(|remote| remote.name == "origin")
                .unwrap()
                .clone()
        }

        fn add(
            &self,
            name: &str,
            fetch: &str,
            push: Option<&str>,
            snapshot: &RepositorySnapshot,
        ) -> Result<MutationOutcome, GitError> {
            add_remote(
                self.root.to_str().unwrap(),
                name,
                fetch,
                push,
                &snapshot.mutation_token,
            )
        }

        fn update(
            &self,
            fetch: Option<&str>,
            push: Option<&str>,
            update_push: bool,
            snapshot: &RepositorySnapshot,
        ) -> Result<MutationOutcome, GitError> {
            update_remote(
                self.root.to_str().unwrap(),
                "origin",
                fetch,
                push,
                update_push,
                &self.remote(snapshot).config_token,
                &snapshot.mutation_token,
            )
        }

        fn checkout_state(&self) -> (Vec<u8>, Vec<u8>, Vec<u8>, String, String) {
            (
                fs::read(self.root.join(".git/index")).unwrap(),
                fs::read(self.root.join(".git/HEAD")).unwrap(),
                fs::read(self.root.join("tracked.txt")).unwrap(),
                self.git(&[
                    "for-each-ref",
                    "--format=%(refname)%00%(objectname)%00%(symref)",
                ]),
                self.git(&["status", "--porcelain=v2", "--untracked-files=all"]),
            )
        }

        fn no_transaction_files(&self) {
            for entry in fs::read_dir(self.root.join(".git")).unwrap() {
                let name = entry.unwrap().file_name().to_string_lossy().into_owned();
                assert!(
                    name != "config.lock" && !name.starts_with("config.enogram-"),
                    "left transaction file: {name}"
                );
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if self.root.is_absolute() && self.root.parent() == Some(std::env::temp_dir().as_path())
            {
                let _ = fs::remove_dir_all(&self.root);
            }
        }
    }

    #[test]
    fn add_remote_settings_is_local_and_preserves_checkout_and_unrelated_bytes() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("tracked.txt"), "staged\n").unwrap();
        fixture.git(&["add", "tracked.txt"]);
        fs::write(fixture.root.join("tracked.txt"), "unstaged\n").unwrap();
        fs::write(fixture.root.join("untracked.txt"), "keep\n").unwrap();
        let include = fixture.root.join(".git/unrelated.cfg");
        let include_bytes = b"# included comment\n[enogram]\n\tkept = included\n";
        fs::write(&include, include_bytes).unwrap();
        let mut original = fixture.config_bytes();
        original.extend_from_slice(
            b"\n[include]\n\tpath = unrelated.cfg\n# final comment without newline",
        );
        fs::write(fixture.config_path(), &original).unwrap();
        let before = fixture.snapshot();
        let state = fixture.checkout_state();
        let missing = fixture.root.join("missing remote A.git");
        let push = fixture.root.join("missing remote B.git");
        let result = fixture
            .add(
                "team/origin",
                missing.to_str().unwrap(),
                Some(push.to_str().unwrap()),
                &before,
            )
            .unwrap();
        assert!(
            !missing.exists() && !push.exists(),
            "settings must never contact or create the destination"
        );
        assert_eq!(fixture.checkout_state(), state);
        assert_eq!(result.snapshot.mutation_token, before.mutation_token);
        assert!(
            fixture.config_bytes().starts_with(&original),
            "existing comments and bytes are retained"
        );
        assert_eq!(fs::read(include).unwrap(), include_bytes);
        assert_eq!(
            fixture.git(&["config", "--get", "enogram.kept"]),
            "included"
        );
        assert_eq!(
            fixture.git(&["config", "--get", "remote.team/origin.fetch"]),
            "+refs/heads/*:refs/remotes/team/origin/*"
        );
        let remote = result
            .snapshot
            .remotes
            .iter()
            .find(|remote| remote.name == "team/origin")
            .unwrap();
        assert_eq!(remote.configured_fetch_url.as_deref(), missing.to_str());
        assert_eq!(remote.configured_push_url.as_deref(), push.to_str());
        assert!(remote.has_explicit_push_url && remote.edit_blocked_reason.is_none());
        fixture.no_transaction_files();
    }

    #[test]
    fn update_remote_settings_preserves_fetch_rules_upstream_and_inheritance() {
        let fixture = Fixture::new();
        let added = fixture
            .add(
                "origin",
                "https://example.invalid/one.git",
                None,
                &fixture.snapshot(),
            )
            .unwrap();
        assert!(!fixture.remote(&added.snapshot).has_explicit_push_url);
        fixture.git(&["config", "branch.main.remote", "origin"]);
        fixture.git(&["config", "branch.main.merge", "refs/heads/main"]);
        fixture.git(&[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/special/*:refs/archive/*",
        ]);
        fixture.git(&["config", "remote.origin.tagopt", "--no-tags"]);
        let mappings = fixture.git(&["config", "--get-all", "remote.origin.fetch"]);
        let state = fixture.checkout_state();
        let updated = fixture
            .update(
                Some("ssh://example.invalid/fetch.git"),
                Some("git@example.invalid:push.git"),
                true,
                &fixture.snapshot(),
            )
            .unwrap();
        let updated_remote = fixture.remote(&updated.snapshot);
        assert_eq!(
            updated_remote.configured_fetch_url.as_deref(),
            Some("ssh://example.invalid/fetch.git")
        );
        // SCP userinfo remains display-only and is never an editable original.
        assert!(
            updated_remote.has_explicit_push_url && updated_remote.configured_push_url.is_none()
        );
        let kept = fixture
            .update(
                Some("https://example.invalid/new-fetch.git"),
                None,
                false,
                &updated.snapshot,
            )
            .unwrap();
        assert_eq!(
            fixture.git(&["config", "--get", "remote.origin.pushurl"]),
            "git@example.invalid:push.git"
        );
        let cleared = fixture.update(None, None, true, &kept.snapshot).unwrap();
        let remote = fixture.remote(&cleared.snapshot);
        assert!(!remote.has_explicit_push_url && remote.configured_push_url.is_none());
        assert_eq!(remote.fetch_url, remote.push_url);
        assert_eq!(
            remote.configured_fetch_url.as_deref(),
            Some("https://example.invalid/new-fetch.git")
        );
        assert_eq!(
            fixture.git(&["config", "--get-all", "remote.origin.fetch"]),
            mappings
        );
        assert_eq!(
            fixture.git(&["config", "--get", "remote.origin.tagopt"]),
            "--no-tags"
        );
        assert_eq!(
            fixture.git(&["config", "--get", "branch.main.remote"]),
            "origin"
        );
        assert_eq!(
            fixture.git(&["config", "--get", "branch.main.merge"]),
            "refs/heads/main"
        );
        assert_eq!(fixture.checkout_state(), state);
        fixture.no_transaction_files();
    }

    #[test]
    fn remote_settings_redact_originals_and_preserve_hidden_values_and_aliases() {
        let fixture = Fixture::new();
        fixture.origin();
        let secret_fetch =
            "https://fetch-user:fetch-secret@example.invalid/repo.git?fetch-query#fetch-fragment";
        let secret_push =
            "https://push-user:push-secret@example.invalid/repo.git?push-query#push-fragment";
        fixture.git(&["config", "remote.origin.url", secret_fetch]);
        fixture.git(&["config", "remote.origin.pushurl", secret_push]);
        let before = fixture.snapshot();
        let serialized = serde_json::to_string(&before).unwrap();
        for hidden in [
            "fetch-user",
            "fetch-secret",
            "fetch-query",
            "fetch-fragment",
            "push-user",
            "push-secret",
            "push-query",
            "push-fragment",
        ] {
            assert!(!serialized.contains(hidden), "snapshot exposed {hidden}");
        }
        let remote = fixture.remote(&before);
        assert!(remote.configured_fetch_url.is_none() && remote.configured_push_url.is_none());
        assert!(remote.has_explicit_push_url && remote.edit_blocked_reason.is_none());
        let changed = fixture
            .update(
                None,
                Some("https://example.invalid/push.git"),
                true,
                &before,
            )
            .unwrap();
        assert_eq!(
            fixture.git(&["config", "--get", "remote.origin.url"]),
            secret_fetch
        );
        fixture
            .update(Some("corp:repo.git"), None, false, &changed.snapshot)
            .unwrap();
        fixture.git(&[
            "config",
            "url.https://rewrite-user:rewrite-secret@example.invalid/.insteadOf",
            "corp:",
        ]);
        let aliased = fixture.snapshot();
        let remote = fixture.remote(&aliased);
        assert_eq!(
            remote.configured_fetch_url.as_deref(),
            Some("corp:repo.git")
        );
        assert_eq!(remote.fetch_url, "https://example.invalid/repo.git");
        assert!(!serde_json::to_string(&aliased)
            .unwrap()
            .contains("rewrite-secret"));
        fixture.update(None, None, true, &aliased).unwrap();
        assert_eq!(
            fixture.git(&["config", "--get", "remote.origin.url"]),
            "corp:repo.git"
        );
        fixture.no_transaction_files();
    }

    #[test]
    fn remote_settings_reject_invalid_duplicate_and_inconsistent_payloads_without_writes() {
        let fixture = Fixture::new();
        fixture.origin();
        let before = fixture.snapshot();
        let bytes = fixture.config_bytes();
        for name in [
            "",
            "--all",
            "has space",
            "/leading",
            "a:b",
            "a..b",
            "a.lock",
            "a\\b",
            "a\nb",
        ] {
            assert_eq!(
                fixture
                    .add(name, "https://example.invalid/repo.git", None, &before)
                    .unwrap_err()
                    .kind,
                GitErrorKind::Unsupported
            );
        }
        for url in [
            "",
            " ",
            "https://example.invalid/repo.git ",
            "-x",
            "https://",
            "https://host with space/repo",
            "ext::secret-helper",
            "custom://secret-helper",
            "https://secret\nfragment/repo",
        ] {
            let error = fixture.add("new", url, None, &before).unwrap_err();
            assert_eq!(error.kind, GitErrorKind::Unsupported);
            assert!(!error.message.contains("secret"));
        }
        assert_eq!(
            fixture
                .add("origin", "https://example.invalid/repo.git", None, &before)
                .unwrap_err()
                .kind,
            GitErrorKind::Unsupported
        );
        assert_eq!(
            fixture
                .update(
                    None,
                    Some("https://example.invalid/push.git"),
                    false,
                    &before
                )
                .unwrap_err()
                .kind,
            GitErrorKind::Unsupported
        );
        assert_eq!(
            fixture.update(None, None, false, &before).unwrap_err().kind,
            GitErrorKind::Unsupported
        );
        assert_eq!(
            fixture
                .update(
                    Some("https://example.invalid/new.git"),
                    Some("ext::hidden"),
                    true,
                    &before
                )
                .unwrap_err()
                .kind,
            GitErrorKind::Unsupported
        );
        assert_eq!(fixture.config_bytes(), bytes);
        fixture.no_transaction_files();
    }

    #[test]
    fn remote_settings_reject_stale_effective_config_and_checkout_tokens() {
        let fixture = Fixture::new();
        fixture.origin();
        let before = fixture.snapshot();
        fixture.git(&["config", "enogram.unrelated", "external-change"]);
        let bytes = fixture.config_bytes();
        assert_eq!(
            fixture
                .update(
                    Some("https://example.invalid/new.git"),
                    None,
                    false,
                    &before
                )
                .unwrap_err()
                .kind,
            GitErrorKind::StaleState
        );
        assert_eq!(fixture.config_bytes(), bytes);
        let fresh = fixture.snapshot();
        fs::write(fixture.root.join("tracked.txt"), "external staged change\n").unwrap();
        fixture.git(&["add", "tracked.txt"]);
        assert_eq!(
            fixture
                .add("new", "https://example.invalid/new.git", None, &fresh)
                .unwrap_err()
                .kind,
            GitErrorKind::StaleState
        );
        assert_eq!(
            fixture
                .update(Some("https://example.invalid/new.git"), None, false, &fresh)
                .unwrap_err()
                .kind,
            GitErrorKind::StaleState
        );
        assert_eq!(fixture.config_bytes(), bytes);
        fixture.no_transaction_files();
    }

    #[test]
    fn remote_settings_block_included_push_values_and_preserve_other_includes() {
        let fixture = Fixture::new();
        fixture.origin();
        let include = fixture.root.join(".git/remotes.cfg");
        let included = b"[remote \"origin\"]\n\tpushurl = https://example.invalid/included-push.git\n[remote \"inherited\"]\n\turl = https://example.invalid/included.git\n";
        fs::write(&include, included).unwrap();
        fixture.git(&["config", "include.path", "remotes.cfg"]);
        let before = fixture.snapshot();
        assert!(fixture
            .remote(&before)
            .edit_blocked_reason
            .unwrap()
            .contains("included"));
        assert!(before
            .remotes
            .iter()
            .any(|remote| remote.name == "inherited"));
        let bytes = fixture.config_bytes();
        assert_eq!(
            fixture
                .update(
                    Some("https://example.invalid/new.git"),
                    None,
                    false,
                    &before
                )
                .unwrap_err()
                .kind,
            GitErrorKind::Unsupported
        );
        assert_eq!(
            fixture
                .add(
                    "inherited",
                    "https://example.invalid/new.git",
                    None,
                    &before
                )
                .unwrap_err()
                .kind,
            GitErrorKind::Unsupported
        );
        assert_eq!(fixture.config_bytes(), bytes);
        // Inherited settings on another remote do not disable new local ones.
        fixture
            .add("local", "https://example.invalid/local.git", None, &before)
            .unwrap();
        assert_eq!(fs::read(include).unwrap(), included);
        fixture.no_transaction_files();
    }

    #[test]
    fn remote_settings_block_global_and_worktree_url_sources() {
        let fixture = Fixture::new();
        fixture.origin();
        let context = fixture.context();
        let direct = ConfigOrigin {
            scope: "local".to_owned(),
            source: format!("file:{}", fixture.config_path().display()),
        };
        let mut global = direct.clone();
        global.scope = "global".to_owned();
        let path = common_config_path(&context).unwrap();
        assert!(origin_is_direct_local(&context, &direct, &path));
        assert!(
            !origin_is_direct_local(&context, &global, &path),
            "identical values/paths never grant local ownership to a global source"
        );
        let mut config = read_effective_config(&context).unwrap();
        let index = config
            .entries
            .iter()
            .position(|(key, _)| key == "remote.origin.url")
            .unwrap();
        config.origins[index].scope = "global".to_owned();
        assert!(edit_blocked_reason(&context, &config, "origin", &path)
            .unwrap()
            .contains("inherited"));
        fixture.git(&["config", "extensions.worktreeConfig", "true"]);
        fixture.git(&[
            "config",
            "--worktree",
            "remote.origin.pushurl",
            "https://example.invalid/worktree-push.git",
        ]);
        let before = fixture.snapshot();
        let bytes = fixture.config_bytes();
        let worktree_bytes = fs::read(fixture.root.join(".git/config.worktree")).unwrap();
        assert!(fixture
            .remote(&before)
            .edit_blocked_reason
            .unwrap()
            .contains("worktree-specific"));
        assert_eq!(
            fixture.update(None, None, true, &before).unwrap_err().kind,
            GitErrorKind::Unsupported
        );
        assert_eq!(fixture.config_bytes(), bytes);
        assert_eq!(
            fs::read(fixture.root.join(".git/config.worktree")).unwrap(),
            worktree_bytes
        );
        fixture.no_transaction_files();
    }

    #[test]
    fn remote_settings_block_multiple_urls_and_url_conditional_includes() {
        let fixture = Fixture::new();
        fixture.origin();
        for key in ["remote.origin.url", "remote.origin.pushurl"] {
            if key.ends_with("pushurl") {
                fixture.git(&["config", "--unset-all", "remote.origin.url"]);
                fixture.git(&[
                    "config",
                    "remote.origin.url",
                    "https://example.invalid/original.git",
                ]);
                fixture.git(&[
                    "config",
                    "remote.origin.pushurl",
                    "https://example.invalid/first-push.git",
                ]);
            }
            fixture.git(&["config", "--add", key, "https://example.invalid/second.git"]);
            let before = fixture.snapshot();
            let bytes = fixture.config_bytes();
            assert!(fixture
                .remote(&before)
                .edit_blocked_reason
                .unwrap()
                .contains("multiple"));
            assert_eq!(
                fixture
                    .update(
                        Some("https://example.invalid/new.git"),
                        None,
                        false,
                        &before
                    )
                    .unwrap_err()
                    .kind,
                GitErrorKind::Unsupported
            );
            assert_eq!(fixture.config_bytes(), bytes);
        }
        fixture.git(&["config", "--unset-all", "remote.origin.pushurl"]);
        fixture.git(&[
            "config",
            "includeIf.hasconfig:remote.*.url:https://conditional.invalid/**.path",
            "not-active.cfg",
        ]);
        let before = fixture.snapshot();
        let bytes = fixture.config_bytes();
        assert!(fixture
            .remote(&before)
            .edit_blocked_reason
            .unwrap()
            .contains("URL-dependent"));
        assert_eq!(
            fixture
                .update(
                    Some("https://conditional.invalid/repo.git"),
                    None,
                    false,
                    &before
                )
                .unwrap_err()
                .kind,
            GitErrorKind::Unsupported
        );
        assert_eq!(
            fixture
                .add("new", "https://conditional.invalid/repo.git", None, &before)
                .unwrap_err()
                .kind,
            GitErrorKind::Unsupported
        );
        assert_eq!(fixture.config_bytes(), bytes);
        fixture.no_transaction_files();
    }

    #[test]
    fn remote_settings_linked_worktree_uses_only_shared_config() {
        let fixture = Fixture::new();
        fixture.origin();
        let linked = fixture.root.join("linked worktree");
        fixture.git(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
        let linked_context = repository_context(linked.to_str().unwrap()).unwrap();
        let before = inspect_context(&linked_context).unwrap();
        let main_state = fixture.checkout_state();
        let linked_index = fs::read(linked_context.git_dir.join("index")).unwrap();
        let linked_head = fs::read(linked_context.git_dir.join("HEAD")).unwrap();
        let origin = before
            .remotes
            .iter()
            .find(|remote| remote.name == "origin")
            .unwrap();
        assert!(origin.edit_blocked_reason.is_none());
        let result = update_remote(
            linked.to_str().unwrap(),
            "origin",
            Some("ssh://example.invalid/new.git"),
            None,
            false,
            &origin.config_token,
            &before.mutation_token,
        )
        .unwrap();
        add_remote(
            linked.to_str().unwrap(),
            "backup",
            "relative remote.git",
            None,
            &result.snapshot.mutation_token,
        )
        .unwrap();
        assert_eq!(
            fixture.git(&["config", "--local", "--get", "remote.origin.url"]),
            "ssh://example.invalid/new.git"
        );
        assert_eq!(
            fixture.git(&["config", "--local", "--get", "remote.backup.url"]),
            "relative remote.git"
        );
        assert!(!linked_context.git_dir.join("config").exists());
        assert!(!linked_context.git_dir.join("config.lock").exists());
        assert_eq!(
            fs::read(linked_context.git_dir.join("index")).unwrap(),
            linked_index
        );
        assert_eq!(
            fs::read(linked_context.git_dir.join("HEAD")).unwrap(),
            linked_head
        );
        assert_eq!(fixture.checkout_state(), main_state);
        fixture.no_transaction_files();
    }

    #[test]
    fn remote_settings_respect_existing_config_lock_and_readonly_config() {
        let fixture = Fixture::new();
        fixture.origin();
        let before = fixture.snapshot();
        let bytes = fixture.config_bytes();
        let lock = fixture.root.join(".git/config.lock");
        fs::write(&lock, "external lock\n").unwrap();
        assert_eq!(
            fixture
                .add("new", "https://example.invalid/new.git", None, &before)
                .unwrap_err()
                .kind,
            GitErrorKind::Busy
        );
        assert_eq!(fs::read(&lock).unwrap(), b"external lock\n");
        assert_eq!(fixture.config_bytes(), bytes);
        fs::remove_file(&lock).unwrap();
        let original_permissions = fs::metadata(fixture.config_path()).unwrap().permissions();
        let mut readonly = original_permissions.clone();
        readonly.set_readonly(true);
        fs::set_permissions(fixture.config_path(), readonly).unwrap();
        let blocked = fixture.snapshot();
        assert!(fixture
            .remote(&blocked)
            .edit_blocked_reason
            .unwrap()
            .contains("read-only"));
        let error = fixture
            .update(
                Some("https://example.invalid/new.git"),
                None,
                false,
                &blocked,
            )
            .unwrap_err();
        let add_error = fixture
            .add("new", "https://example.invalid/new.git", None, &blocked)
            .unwrap_err();
        fs::set_permissions(fixture.config_path(), original_permissions).unwrap();
        assert_eq!(error.kind, GitErrorKind::Unsupported);
        assert_eq!(add_error.kind, GitErrorKind::Unsupported);
        assert_eq!(fixture.config_bytes(), bytes);
        fixture.no_transaction_files();
    }

    #[test]
    fn remote_settings_partial_staging_failure_and_external_edit_never_publish() {
        let fixture = Fixture::new();
        fixture.origin();
        let context = fixture.context();
        let before = fixture.config_bytes();
        let transaction = ConfigTransaction::begin(&context).unwrap();
        transaction
            .edit(
                &context,
                "remote.origin.url",
                Some("https://example.invalid/staged.git"),
            )
            .unwrap();
        let error = transaction
            .edit(
                &context,
                "invalid-key-without-section",
                Some("https://user:secret@example.invalid/repo"),
            )
            .unwrap_err();
        assert!(!error.message.contains("secret") && !error.message.contains("user:"));
        drop(transaction);
        assert_eq!(fixture.config_bytes(), before);
        fixture.no_transaction_files();
        let transaction = ConfigTransaction::begin(&context).unwrap();
        transaction
            .edit(
                &context,
                "remote.origin.url",
                Some("https://example.invalid/staged.git"),
            )
            .unwrap();
        let mut external = before;
        external.extend_from_slice(b"\n# external editor retained\n");
        fs::write(fixture.config_path(), &external).unwrap();
        assert_eq!(
            transaction.ensure_source_unchanged().unwrap_err().kind,
            GitErrorKind::StaleState
        );
        drop(transaction);
        assert_eq!(fixture.config_bytes(), external);
        fixture.no_transaction_files();
    }

    #[cfg(windows)]
    #[test]
    fn remote_settings_failed_windows_rename_keeps_original_and_cleans_lock() {
        use std::os::windows::fs::OpenOptionsExt;
        let fixture = Fixture::new();
        fixture.origin();
        let context = fixture.context();
        let before = fixture.config_bytes();
        let mut transaction = ConfigTransaction::begin(&context).unwrap();
        transaction
            .edit(
                &context,
                "remote.origin.url",
                Some("https://example.invalid/staged.git"),
            )
            .unwrap();
        let staged = read_config_bytes(transaction.staged_path.as_ref().unwrap()).unwrap();
        let hold = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(fixture.config_path())
            .unwrap();
        assert_eq!(
            transaction.commit(&staged).unwrap_err().kind,
            GitErrorKind::GitCommandFailed
        );
        drop(hold);
        drop(transaction);
        assert_eq!(fixture.config_bytes(), before);
        fixture.no_transaction_files();
    }

    #[cfg(unix)]
    #[test]
    fn remote_settings_reject_linked_config_files_without_replacing_them() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        fixture.origin();
        let original = fixture.config_bytes();
        let actual = fixture.root.join(".git/config.actual");
        fs::rename(fixture.config_path(), &actual).unwrap();
        symlink(&actual, fixture.config_path()).unwrap();
        let before = fixture.snapshot();
        assert!(fixture
            .remote(&before)
            .edit_blocked_reason
            .unwrap()
            .contains("symlink"));
        assert_eq!(
            fixture
                .update(
                    Some("https://example.invalid/new.git"),
                    None,
                    false,
                    &before
                )
                .unwrap_err()
                .kind,
            GitErrorKind::Unsupported
        );
        assert_eq!(
            fixture
                .add("new", "https://example.invalid/new.git", None, &before)
                .unwrap_err()
                .kind,
            GitErrorKind::Unsupported
        );
        assert!(fs::symlink_metadata(fixture.config_path())
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(actual).unwrap(), original);
        fixture.no_transaction_files();
    }
}
