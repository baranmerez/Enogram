use enogram_mcp::{Config, Server};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};

// Production writes share a Windows-session mutex. Keep these disposable Git
// workflows serial rather than turning expected contention into a flaky test.
static TEST_LOCK: Mutex<()> = Mutex::new(());
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    repo: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "enogram-mcp-test-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let repo = root.join("main");
        fs::create_dir(&repo).unwrap();
        let fixture = Self { root, repo };
        fixture.git(&["init", "-b", "main"]);
        fixture.git(&["config", "user.name", "MCP Fixture"]);
        fixture.git(&["config", "user.email", "mcp-fixture@example.invalid"]);
        fixture.git(&["config", "commit.gpgsign", "false"]);
        fixture.git(&["config", "core.autocrlf", "false"]);
        fixture.git(&["config", "core.attributesFile", ""]);
        fixture.git(&[
            "config",
            "core.hooksPath",
            fixture.repo.join(".git/hooks").to_str().unwrap(),
        ]);
        fixture.write("file.txt", "base\n");
        fixture.git(&["add", "--", "file.txt"]);
        fixture.git(&["commit", "-m", "base"]);
        fixture
    }
    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("--no-optional-locks")
            .args(args)
            .current_dir(&self.repo)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .unwrap()
            .trim_end()
            .to_owned()
    }
    fn write(&self, file: &str, content: &str) {
        fs::write(self.repo.join(file), content).unwrap();
    }
    fn server(&self, write: bool) -> Server {
        let mut server = Server::new(Config {
            repo_paths: vec![self.repo.clone()],
            allow_write: write,
        })
        .unwrap();
        initialize(&mut server);
        server
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let expected = std::env::temp_dir();
        assert!(self.root.starts_with(&expected));
        assert!(self
            .root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("enogram-mcp-test-"));
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn initialize(server: &mut Server) {
    let result = server.handle(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}})).unwrap();
    assert!(result.get("result").is_some(), "{result}");
    assert!(server
        .handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .is_none());
}
fn call(server: &mut Server, name: &str, arguments: Value) -> Value {
    let response = server.handle(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":arguments}})).unwrap();
    if let Some(result) = response.get("result") {
        let text: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text, result["structuredContent"]);
    }
    response
}
fn data(response: &Value) -> &Value {
    assert_eq!(response["result"]["isError"], false, "{response}");
    &response["result"]["structuredContent"]
}
fn token(server: &mut Server) -> String {
    data(&call(
        server,
        "enogram_inspect_repository",
        json!({"repoId":"repo-1"}),
    ))["mutationToken"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn stage_commit_unstage_stale_state_and_duplicate_commit_preserve_git_content() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let fixture = Fixture::new();
    let base = fixture.git(&["rev-parse", "HEAD"]);
    let mut server = fixture.server(true);
    fixture.write("file.txt", "staged\n");
    fixture.write("other.txt", "untracked stays\n");
    let before = token(&mut server);
    let staged = call(
        &mut server,
        "enogram_stage_paths",
        json!({"repoId":"repo-1","paths":["file.txt"],"expectedToken":before,"operationId":"stage-1"}),
    );
    assert!(data(&staged)["commitOid"].is_null());
    let staged_token = data(&staged)["snapshot"]["mutationToken"]
        .as_str()
        .unwrap()
        .to_owned();
    fixture.write("file.txt", "staged\nlater working edit\n");
    assert_eq!(
        token(&mut server),
        staged_token,
        "Token intentionally excludes working bytes"
    );
    let arguments = json!({"repoId":"repo-1","message":"MCP index only","expectedToken":staged_token,"operationId":"commit-1"});
    let committed = call(&mut server, "enogram_commit_index", arguments.clone());
    let oid = data(&committed)["commitOid"].as_str().unwrap();
    assert_eq!(data(&committed)["commitTreeChanged"], false);
    assert_eq!(fixture.git(&["rev-parse", "HEAD"]), oid);
    assert_eq!(fixture.git(&["rev-parse", "HEAD^1"]), base);
    assert_eq!(fixture.git(&["show", "HEAD:file.txt"]), "staged");
    assert_eq!(
        fs::read_to_string(fixture.repo.join("file.txt")).unwrap(),
        "staged\nlater working edit\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.repo.join("other.txt")).unwrap(),
        "untracked stays\n"
    );
    assert_eq!(
        call(&mut server, "enogram_commit_index", arguments.clone()),
        committed
    );
    assert_eq!(
        fixture.git(&["rev-list", "--count", &format!("{base}..HEAD")]),
        "1"
    );
    let mut changed = arguments;
    changed["message"] = json!("different");
    assert_eq!(
        call(&mut server, "enogram_commit_index", changed)["result"]["structuredContent"]["kind"],
        "operationIdConflict"
    );

    let before = token(&mut server);
    let staged = call(
        &mut server,
        "enogram_stage_paths",
        json!({"repoId":"repo-1","paths":["file.txt"],"expectedToken":before,"operationId":"stage-2"}),
    );
    let current = data(&staged)["snapshot"]["mutationToken"].clone();
    let bytes = fs::read(fixture.repo.join("file.txt")).unwrap();
    data(&call(
        &mut server,
        "enogram_unstage_paths",
        json!({"repoId":"repo-1","paths":["file.txt"],"expectedToken":current,"operationId":"unstage-1"}),
    ));
    assert_eq!(fs::read(fixture.repo.join("file.txt")).unwrap(), bytes);
    assert_eq!(fixture.git(&["diff", "--cached", "--name-only"]), "");

    let stale = token(&mut server);
    fixture.git(&["add", "--", "other.txt"]);
    let args = json!({"repoId":"repo-1","paths":["file.txt"],"expectedToken":stale,"operationId":"stale-1"});
    let failed = call(&mut server, "enogram_stage_paths", args.clone());
    assert_eq!(failed["result"]["structuredContent"]["kind"], "staleState");
    assert_eq!(call(&mut server, "enogram_stage_paths", args), failed);
    assert_eq!(
        fixture.git(&["diff", "--cached", "--name-only"]),
        "other.txt"
    );
}

#[test]
fn readonly_and_unregistered_worktrees_cannot_bypass_dispatch() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let fixture = Fixture::new();
    let linked = fixture.root.join("linked");
    fixture.git(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
    let mut server = fixture.server(false);
    let listed = server
        .handle(json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}))
        .unwrap();
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 4);
    fixture.write("file.txt", "dirty\n");
    let snapshot = call(
        &mut server,
        "enogram_inspect_repository",
        json!({"repoId":"repo-1"}),
    );
    assert_eq!(data(&snapshot)["worktrees"].as_array().unwrap().len(), 2);
    let before = fixture.git(&["ls-files", "--stage"]);
    let args = json!({"repoId":"repo-1","paths":["file.txt"],"expectedToken":data(&snapshot)["mutationToken"],"operationId":"bypass"});
    assert_eq!(
        call(&mut server, "enogram_stage_paths", args)["result"]["structuredContent"]["kind"],
        "accessDenied"
    );
    assert_eq!(before, fixture.git(&["ls-files", "--stage"]));
    for repo_id in ["repo-2", linked.to_str().unwrap()] {
        assert_eq!(
            call(
                &mut server,
                "enogram_inspect_repository",
                json!({"repoId":repo_id})
            )["result"]["structuredContent"]["kind"],
            "accessDenied"
        );
    }
    assert_eq!(
        call(
            &mut server,
            "enogram_inspect_repository",
            json!({"repoId":"repo-1","repoPath":linked})
        )["error"]["code"],
        -32602
    );
    let mut granted = Server::new(Config {
        repo_paths: vec![fixture.repo.clone(), linked],
        allow_write: false,
    })
    .unwrap();
    initialize(&mut granted);
    assert_eq!(
        data(&call(&mut granted, "enogram_list_repositories", json!({})))["repositories"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn failed_hook_is_redacted_and_duplicate_failure_does_not_rerun_hook() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let fixture = Fixture::new();
    let mut server = fixture.server(true);
    fixture.write("file.txt", "staged\n");
    let before = token(&mut server);
    let staged = call(
        &mut server,
        "enogram_stage_paths",
        json!({"repoId":"repo-1","paths":["file.txt"],"expectedToken":before,"operationId":"stage"}),
    );
    let hook = fixture.repo.join(".git/hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\nprintf run >> .git/hook-runs\nprintf 'PRIVATE_HOOK_SECRET token=password' >&2\nprintf side-effect > hook.txt\ngit add -- hook.txt\nexit 1\n").unwrap();
    make_executable(&hook);
    let head = fixture.git(&["rev-parse", "HEAD"]);
    let args = json!({"repoId":"repo-1","message":"rejected","expectedToken":data(&staged)["snapshot"]["mutationToken"],"operationId":"failed-commit"});
    let failed = call(&mut server, "enogram_commit_index", args.clone());
    assert_eq!(failed["result"]["isError"], true);
    assert_eq!(
        failed["result"]["structuredContent"]["kind"],
        "gitCommandFailed"
    );
    assert!(!failed.to_string().contains("PRIVATE_HOOK_SECRET"));
    assert!(!failed.to_string().contains("password"));
    assert_eq!(call(&mut server, "enogram_commit_index", args), failed);
    assert_eq!(
        fs::read_to_string(fixture.repo.join(".git/hook-runs")).unwrap(),
        "run"
    );
    assert_eq!(fixture.git(&["rev-parse", "HEAD"]), head);
    assert!(fixture
        .git(&["diff", "--cached", "--name-only"])
        .contains("hook.txt"));
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}
#[cfg(not(unix))]
fn make_executable(_: &Path) {}

#[test]
fn startup_requires_distinct_nonbare_checkout_roots_and_diff_flags_survive() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let fixture = Fixture::new();
    fs::create_dir(fixture.repo.join("nested")).unwrap();
    for paths in [
        vec![fixture.repo.join("nested")],
        vec![fixture.repo.clone(), fixture.repo.clone()],
        vec![fixture.root.clone()],
    ] {
        assert!(Server::new(Config {
            repo_paths: paths,
            allow_write: false
        })
        .is_err());
    }
    let bare = fixture.root.join("bare.git");
    fixture.git(&["init", "--bare", bare.to_str().unwrap()]);
    assert!(Server::new(Config {
        repo_paths: vec![bare],
        allow_write: false
    })
    .is_err());
    let mut server = fixture.server(false);
    fs::write(fixture.repo.join("binary.dat"), b"\0binary\0").unwrap();
    let diff = call(
        &mut server,
        "enogram_read_diff",
        json!({"repoId":"repo-1","filePath":"binary.dat"}),
    );
    assert_eq!(data(&diff)["binary"], true);
    assert_eq!(data(&diff)["truncated"], false);
    let oid = fixture.git(&["rev-parse", "HEAD"]);
    let files = call(
        &mut server,
        "enogram_list_commit_files",
        json!({"repoId":"repo-1","commitOid":oid}),
    );
    assert_eq!(data(&files)["files"][0]["path"], "file.txt");
}

#[test]
fn changed_git_worktree_root_is_rejected_before_reads_or_writes() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let fixture = Fixture::new();
    let mut server = fixture.server(true);
    let before = token(&mut server);
    let outside = fixture.root.join("unregistered");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("file.txt"), "unregistered content\n").unwrap();
    fixture.git(&["config", "core.worktree", outside.to_str().unwrap()]);
    for (tool, args) in [
        ("enogram_inspect_repository", json!({"repoId":"repo-1"})),
        (
            "enogram_read_diff",
            json!({"repoId":"repo-1","filePath":"file.txt"}),
        ),
        (
            "enogram_stage_paths",
            json!({"repoId":"repo-1","paths":["file.txt"],"expectedToken":before,"operationId":"redirect"}),
        ),
    ] {
        let response = call(&mut server, tool, args);
        assert_eq!(
            response["result"]["structuredContent"]["kind"],
            "accessDenied"
        );
        assert!(!response.to_string().contains("unregistered content"));
    }
    assert_eq!(fixture.git(&["diff", "--cached", "--name-only"]), "");
}

#[test]
fn replaced_git_directory_is_not_an_implicit_new_repository_grant() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let fixture = Fixture::new();
    let other = Fixture::new();
    let mut server = fixture.server(true);
    let before = token(&mut server);
    let other_index = other.git(&["ls-files", "--stage"]);
    fs::rename(
        fixture.repo.join(".git"),
        fixture.repo.join(".git-original"),
    )
    .unwrap();
    fs::write(
        fixture.repo.join(".git"),
        format!("gitdir: {}\n", other.repo.join(".git").display()),
    )
    .unwrap();
    // The worktree root is unchanged; the authority over metadata/refs is not.
    assert_eq!(
        enogram_core::repository_identity(fixture.repo.to_str().unwrap())
            .unwrap()
            .worktree_root,
        fs::canonicalize(&fixture.repo).unwrap()
    );
    assert_eq!(
        call(
            &mut server,
            "enogram_inspect_repository",
            json!({"repoId":"repo-1"})
        )["result"]["structuredContent"]["kind"],
        "accessDenied"
    );
    assert_eq!(
        call(
            &mut server,
            "enogram_stage_paths",
            json!({"repoId":"repo-1","paths":["file.txt"],"expectedToken":before,"operationId":"metadata-redirect"})
        )["result"]["structuredContent"]["kind"],
        "accessDenied"
    );
    assert_eq!(other.git(&["ls-files", "--stage"]), other_index);
}

#[test]
fn changed_linked_worktree_common_directory_is_rejected() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let fixture = Fixture::new();
    let other = Fixture::new();
    let linked = fixture.root.join("linked");
    fixture.git(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
    let mut server = Server::new(Config {
        repo_paths: vec![linked.clone()],
        allow_write: true,
    })
    .unwrap();
    initialize(&mut server);
    let identity = enogram_core::repository_identity(linked.to_str().unwrap()).unwrap();
    fs::write(
        identity.git_dir.join("commondir"),
        format!("{}\n", other.repo.join(".git").display()),
    )
    .unwrap();
    let changed = enogram_core::repository_identity(linked.to_str().unwrap()).unwrap();
    assert_eq!(changed.worktree_root, identity.worktree_root);
    assert_eq!(changed.git_dir, identity.git_dir);
    assert_ne!(changed.common_dir, identity.common_dir);
    assert_eq!(
        call(
            &mut server,
            "enogram_inspect_repository",
            json!({"repoId":"repo-1"})
        )["result"]["structuredContent"]["kind"],
        "accessDenied"
    );
}
