//! Local stdio MCP adapter. Repository authority is fixed at process startup.
//! Operation replay is finite and process-local: the oldest result is evicted
//! after 128 operations or 32 MiB of serialized records. Restart loses the cache.
//! After an uncertain delivery, inspect the repository; do not blindly retry a
//! write after eviction/restart. The core owns Git validation and coordination.

use enogram_core::{self as core, GitError, GitErrorKind};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

pub const PROTOCOL_VERSION: &str = "2025-11-25";
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_OPERATION_RECORDS: usize = 128;
pub const MAX_OPERATION_CACHE_BYTES: usize = 32 * 1024 * 1024;
// Leave space for the envelope, string escaping, and duplicated text content.
const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;
const MAX_REPOSITORIES: usize = 64;

#[derive(Debug)]
pub struct StartupError(&'static str);

impl fmt::Display for StartupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}
impl std::error::Error for StartupError {}

#[derive(Debug)]
pub struct Config {
    pub repo_paths: Vec<PathBuf>,
    pub allow_write: bool,
}

impl Config {
    pub fn from_args(args: impl IntoIterator<Item = OsString>) -> Result<Self, StartupError> {
        let mut args = args.into_iter();
        let mut config = Self {
            repo_paths: Vec::new(),
            allow_write: false,
        };
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--repo") => {
                    let path = args
                        .next()
                        .ok_or(StartupError("--repo requires an absolute checkout root."))?;
                    let path = PathBuf::from(path);
                    if !path.is_absolute() || config.repo_paths.len() >= MAX_REPOSITORIES {
                        return Err(StartupError("Provide at most 64 absolute checkout roots."));
                    }
                    config.repo_paths.push(path);
                }
                Some("--allow-write") if !config.allow_write => config.allow_write = true,
                _ => return Err(StartupError("Unknown or repeated command-line option.")),
            }
        }
        if config.repo_paths.is_empty() {
            return Err(StartupError(
                "At least one --repo checkout root is required.",
            ));
        }
        Ok(config)
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Repository {
    repo_id: String,
    repo_path: String,
    repo_name: String,
    write_enabled: bool,
    #[serde(skip)]
    canonical_path: PathBuf,
    #[serde(skip)]
    identity: core::RepositoryIdentity,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Fresh,
    AwaitingInitialized,
    Ready,
}

struct CachedOperation {
    operation_id: String,
    fingerprint: Value,
    result: Value,
    bytes: usize,
}

pub struct Server {
    repositories: Vec<Repository>,
    allow_write: bool,
    phase: Phase,
    operations: VecDeque<CachedOperation>,
    operation_bytes: usize,
}

impl Server {
    pub fn new(config: Config) -> Result<Self, StartupError> {
        if config.repo_paths.is_empty() || config.repo_paths.len() > MAX_REPOSITORIES {
            return Err(StartupError(
                "Provide between one and 64 absolute checkout roots.",
            ));
        }
        let mut repositories = Vec::new();
        let mut roots = Vec::new();
        for path in config.repo_paths {
            if !path.is_absolute() {
                return Err(StartupError("Repository roots must be absolute paths."));
            }
            let canonical = fs::canonicalize(path).map_err(|_| {
                StartupError("A configured checkout root does not exist or is inaccessible.")
            })?;
            let path = canonical
                .to_str()
                .ok_or(StartupError("Checkout roots must be valid Unicode."))?;
            let identity = core::repository_identity(path).map_err(|_| {
                StartupError("A configured Git repository identity could not be resolved.")
            })?;
            let snapshot = core::inspect_repository(path).map_err(|_| {
                StartupError("A configured root is not a readable non-bare Git checkout.")
            })?;
            let root = fs::canonicalize(&snapshot.repo_path)
                .map_err(|_| StartupError("The Git checkout root could not be resolved."))?;
            if root != canonical || identity.worktree_root != canonical || roots.contains(&root) {
                return Err(StartupError(
                    "Each --repo must name a distinct checkout root, not a subdirectory.",
                ));
            }
            roots.push(root);
            repositories.push(Repository {
                repo_id: format!("repo-{}", repositories.len() + 1),
                repo_path: path.to_owned(),
                repo_name: snapshot.repo_name,
                write_enabled: config.allow_write,
                canonical_path: canonical.clone(),
                identity,
            });
        }
        Ok(Self {
            repositories,
            allow_write: config.allow_write,
            phase: Phase::Fresh,
            operations: VecDeque::new(),
            operation_bytes: 0,
        })
    }

    /// Process one decoded message. Notifications never invoke a Git operation.
    pub fn handle(&mut self, message: Value) -> Option<Value> {
        let Some(object) = message.as_object() else {
            return Some(rpc_error(
                Value::Null,
                -32600,
                "Expected one JSON-RPC request object.",
            ));
        };
        // No server-to-client requests are issued. Ignore client response objects.
        if object.get("jsonrpc") == Some(&json!("2.0"))
            && !object.contains_key("method")
            && object.contains_key("id")
            && (object.contains_key("result") || object.contains_key("error"))
        {
            return None;
        }
        let id = object.get("id").cloned();
        let valid_id = id.as_ref().map_or(true, |id| match id {
            Value::String(value) => value.len() <= 256,
            Value::Number(value) => value.is_i64() || value.is_u64(),
            _ => false,
        });
        if object.get("jsonrpc") != Some(&json!("2.0"))
            || !valid_id
            || object
                .keys()
                .any(|key| !matches!(key.as_str(), "jsonrpc" | "id" | "method" | "params"))
        {
            return Some(rpc_error(Value::Null, -32600, "Invalid JSON-RPC request."));
        }
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            return Some(rpc_error(
                id.unwrap_or(Value::Null),
                -32600,
                "A request method is required.",
            ));
        };
        let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
        let Some(id) = id else {
            if method == "notifications/initialized"
                && self.phase == Phase::AwaitingInitialized
                && decode::<EmptyParams>(params).is_ok()
            {
                self.phase = Phase::Ready;
            }
            return None;
        };
        // Discovery probes from newer clients must fail promptly so they can
        // fall back to the pinned initialize handshake.
        if !matches!(method, "initialize" | "ping" | "tools/list" | "tools/call") {
            return Some(rpc_error(id, -32601, "Method not found."));
        }
        let result = match method {
            "initialize" => self.initialize(params),
            "ping" => decode::<EmptyParams>(params).map(|_| json!({})),
            _ if self.phase != Phase::Ready => Err(RpcError(
                -32002,
                "Complete initialize and notifications/initialized first.",
            )),
            "tools/list" => decode::<ListParams>(params).and_then(|params| {
                if params.cursor.is_some() {
                    Err(RpcError(
                        -32602,
                        "Pagination is not supported; omit cursor.",
                    ))
                } else {
                    Ok(json!({"tools": tool_definitions(self.allow_write)}))
                }
            }),
            "tools/call" => decode::<CallParams>(params).and_then(|params| self.call(params)),
            _ => unreachable!(),
        };
        Some(match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(error) => rpc_error(id, error.0, error.1),
        })
    }

    fn initialize(&mut self, params: Value) -> Result<Value, RpcError> {
        let params: InitializeParams = decode(params)?;
        if self.phase != Phase::Fresh {
            return Err(RpcError(-32600, "This connection is already initialized."));
        }
        if params.protocol_version.is_empty()
            || params.client_info.name.is_empty()
            || params.client_info.version.is_empty()
        {
            return Err(RpcError(
                -32602,
                "Protocol version and client name/version are required.",
            ));
        }
        self.phase = Phase::AwaitingInitialized;
        Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "enogram-mcp", "version": env!("CARGO_PKG_VERSION")},
            "instructions": "Use only repoId values from enogram_list_repositories. Discovered worktrees do not grant access. Stage stages the entire current file; expectedToken excludes working-file bytes. Writes require --allow-write, a fresh expectedToken, and a unique operationId. Identical operationId/tool/arguments replay the original result, including errors, while cached: FIFO limit 128 operations or 32 MiB per process. Eviction or restart loses that guarantee. After uncertain delivery or any failed write, inspect repository state and do not automatically retry. Configured Git hooks, filters, identity and signing apply."
        }))
    }

    fn call(&mut self, params: CallParams) -> Result<Value, RpcError> {
        let arguments = params.arguments;
        match params.name.as_str() {
            "enogram_list_repositories" => {
                decode::<EmptyArgs>(arguments)?;
                Ok(tool_result(
                    json!({"repositories": self.repositories}),
                    false,
                ))
            }
            "enogram_inspect_repository" => {
                let args: RepoArgs = decode(arguments)?;
                Ok(match self.repository(&args.repo_id, false) {
                    Ok((_, snapshot)) => core_result(Ok(snapshot), false),
                    Err(error) => error,
                })
            }
            "enogram_read_diff" => {
                let args: DiffArgs = decode(arguments)?;
                Ok(match self.repository(&args.repo_id, false) {
                    Ok((repo, _)) => core_result(
                        core::read_diff(
                            &repo.repo_path,
                            &args.file_path,
                            args.staged,
                            args.commit_oid.as_deref(),
                        ),
                        false,
                    ),
                    Err(error) => error,
                })
            }
            "enogram_list_commit_files" => {
                let args: CommitFilesArgs = decode(arguments)?;
                Ok(match self.repository(&args.repo_id, false) {
                    Ok((repo, _)) => core_result(
                        core::list_commit_files(&repo.repo_path, &args.commit_oid)
                            .map(|files| json!({"files": files})),
                        false,
                    ),
                    Err(error) => error,
                })
            }
            "enogram_stage_paths" | "enogram_unstage_paths" => {
                let args: PathsArgs = decode(arguments.clone())?;
                self.write(
                    &params.name,
                    arguments,
                    &args.repo_id,
                    &args.operation_id,
                    |path| {
                        if params.name == "enogram_stage_paths" {
                            core::stage_paths(path, &args.paths, &args.expected_token)
                        } else {
                            core::unstage_paths(path, &args.paths, &args.expected_token)
                        }
                    },
                )
            }
            "enogram_commit_index" => {
                let args: CommitArgs = decode(arguments.clone())?;
                self.write(
                    &params.name,
                    arguments,
                    &args.repo_id,
                    &args.operation_id,
                    |path| core::commit_index(path, &args.message, &args.expected_token),
                )
            }
            _ => Err(RpcError(-32602, "Unknown tool.")),
        }
    }

    fn repository(
        &self,
        repo_id: &str,
        write: bool,
    ) -> Result<(&Repository, core::RepositorySnapshot), Value> {
        let repo = self
            .repositories
            .iter()
            .find(|repo| repo.repo_id == repo_id)
            .ok_or_else(|| {
                tool_error(
                    "accessDenied",
                    "Repository ID is not registered for this process.",
                    false,
                )
            })?;
        let identity_error = || {
            tool_error("accessDenied", "The registered checkout root changed; restart with an explicitly authorized repository configuration.", write)
        };
        if fs::canonicalize(&repo.repo_path).ok().as_ref() != Some(&repo.canonical_path) {
            return Err(identity_error());
        }
        if core::repository_identity(&repo.repo_path).ok().as_ref() != Some(&repo.identity) {
            return Err(identity_error());
        }
        // Git's core.worktree can redirect a checkout without a filesystem
        // symlink. Check Git's resolved root before exposing data or writing.
        // External config/filesystem writers can still race this preflight.
        let snapshot = core::inspect_repository(&repo.repo_path)
            .map_err(|error| core_result::<Value>(Err(error), write))?;
        if fs::canonicalize(&snapshot.repo_path).ok().as_ref() != Some(&repo.canonical_path) {
            return Err(identity_error());
        }
        Ok((repo, snapshot))
    }

    fn write(
        &mut self,
        tool: &str,
        arguments: Value,
        repo_id: &str,
        operation_id: &str,
        action: impl FnOnce(&str) -> Result<core::MutationOutcome, GitError>,
    ) -> Result<Value, RpcError> {
        if operation_id.is_empty()
            || operation_id.len() > 128
            || operation_id.chars().any(char::is_control)
        {
            return Err(RpcError(
                -32602,
                "operationId must contain 1 to 128 bytes without control characters.",
            ));
        }
        let fingerprint = json!({"tool": tool, "arguments": arguments});
        if let Some(cached) = self
            .operations
            .iter()
            .find(|entry| entry.operation_id == operation_id)
        {
            return Ok(if cached.fingerprint == fingerprint {
                cached.result.clone()
            } else {
                tool_error(
                    "operationIdConflict",
                    "operationId was already used with different tool arguments.",
                    true,
                )
            });
        }
        // Dispatch authorization is independent of tool discovery and happens
        // before any core call, including attempted access to another worktree.
        let result = if !self.allow_write {
            tool_error(
                "accessDenied",
                "Writes are disabled; restart with an explicitly authorized --allow-write grant.",
                false,
            )
        } else {
            match self.repository(repo_id, true) {
                Err(error) => error,
                Ok((repo, _)) => core_result(action(&repo.repo_path), true),
            }
        };
        let bytes = serde_json::to_vec(&fingerprint).expect("JSON value").len()
            + serde_json::to_vec(&result).expect("JSON value").len()
            + operation_id.len();
        while self.operations.len() >= MAX_OPERATION_RECORDS
            || self.operation_bytes.saturating_add(bytes) > MAX_OPERATION_CACHE_BYTES
        {
            let Some(oldest) = self.operations.pop_front() else {
                break;
            };
            self.operation_bytes -= oldest.bytes;
        }
        self.operation_bytes += bytes;
        self.operations.push_back(CachedOperation {
            operation_id: operation_id.to_owned(),
            fingerprint,
            result: result.clone(),
            bytes,
        });
        Ok(result)
    }
}

struct RpcError(i64, &'static str);
fn decode<T: DeserializeOwned>(value: Value) -> Result<T, RpcError> {
    serde_json::from_value(value).map_err(|_| {
        RpcError(
            -32602,
            "Arguments do not match the tool or method schema; unknown fields are rejected.",
        )
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyParams {
    #[serde(rename = "_meta")]
    _meta: Option<Map<String, Value>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListParams {
    cursor: Option<String>,
    #[serde(rename = "_meta")]
    _meta: Option<Map<String, Value>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallParams {
    name: String,
    #[serde(default = "empty_object")]
    arguments: Value,
    #[serde(rename = "_meta")]
    _meta: Option<Map<String, Value>>,
}
fn empty_object() -> Value {
    json!({})
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InitializeParams {
    protocol_version: String,
    #[serde(rename = "capabilities")]
    _capabilities: Map<String, Value>,
    client_info: ClientInfo,
    #[serde(rename = "_meta")]
    _meta: Option<Map<String, Value>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClientInfo {
    name: String,
    version: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepoArgs {
    repo_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DiffArgs {
    repo_id: String,
    file_path: String,
    #[serde(default)]
    staged: bool,
    commit_oid: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommitFilesArgs {
    repo_id: String,
    commit_oid: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PathsArgs {
    repo_id: String,
    paths: Vec<String>,
    expected_token: String,
    operation_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommitArgs {
    repo_id: String,
    message: String,
    expected_token: String,
    operation_id: String,
}

fn core_result<T: Serialize>(result: Result<T, GitError>, write: bool) -> Value {
    match result {
        Ok(result) => tool_result(
            serde_json::to_value(result).expect("Core result is serializable"),
            false,
        ),
        Err(error) => {
            let message = match error.kind {
                GitErrorKind::StaleState => "Repository state changed or a previous write was interrupted.",
                GitErrorKind::Busy => "Another Git write is active or the repository is locked.",
                GitErrorKind::Conflicts => "Resolve conflicting files before this operation.",
                GitErrorKind::NothingStaged => "There are no staged changes to commit.",
                GitErrorKind::InvalidCommitMessage => "The commit message is invalid or exceeds the core limit.",
                GitErrorKind::InvalidMutationPath | GitErrorKind::InvalidDiffPath | GitErrorKind::InvalidPath => "Select valid repository-relative files from a current repository inspection.",
                GitErrorKind::InvalidRevision => "Use a full existing commit object ID from repository history.",
                GitErrorKind::InvalidHistorySearch => "The history search query or cursor is invalid.",
                GitErrorKind::NotRepository => "The registered checkout is no longer available as a Git repository.",
                GitErrorKind::GitUnavailable => "Git is unavailable in the server environment.",
                GitErrorKind::TimedOut => "Git exceeded its execution deadline; its effects may already exist.",
                GitErrorKind::OutputLimit => "The Git result exceeded a configured output limit.",
                GitErrorKind::InvalidOutput => "Git returned output that could not be interpreted safely.",
                GitErrorKind::Unsupported => "The current repository operation or format is unsupported.",
                GitErrorKind::GitCommandFailed => "Git or a configured hook, filter, or signing command failed; effects may already exist.",
                GitErrorKind::InvalidBranchName | GitErrorKind::BranchNotFound => "The requested branch state is invalid or unavailable.",
            };
            let kind = serde_json::to_value(error.kind).expect("Error kind is serializable");
            tool_error(
                kind.as_str().expect("Error kind is a string"),
                message,
                write,
            )
        }
    }
}

fn tool_error(kind: &str, message: &str, write: bool) -> Value {
    tool_result(
        json!({"kind": kind, "message": message, "requiresInspection": write,
        "guidance": if write { "Inspect this repository before another write. Do not automatically retry; a failed operation may already have changed Git state." }
        else { "Inspect the registered repository and correct the request or local Git configuration before continuing." }}),
        true,
    )
}

fn tool_result(mut data: Value, mut is_error: bool) -> Value {
    let text = match bounded_json(&data, MAX_RESULT_BYTES) {
        Ok(bytes) => String::from_utf8(bytes).expect("JSON is UTF-8"),
        Err(_) => {
            // A successful write may have an oversized snapshot. Preserve its
            // commit identity/warning without pretending the write was undone.
            let retained: Vec<_> = ["commitOid", "commitTreeChanged", "binary", "truncated"]
                .iter()
                .filter_map(|key| data.get(key).cloned().map(|value| (*key, value)))
                .collect();
            data = json!({"kind": "outputLimit", "message": "The result exceeds the MCP response limit.",
                "requiresInspection": true, "guidance": "Inspect repository state. Do not automatically repeat a write; it may have completed."});
            for (key, value) in retained {
                data[key] = value;
            }
            is_error = true;
            serde_json::to_string(&data).expect("JSON value")
        }
    };
    json!({"content": [{"type": "text", "text": text}], "structuredContent": data, "isError": is_error})
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tool_definitions(write: bool) -> Vec<Value> {
    let mut tools = vec![
        tool("enogram_list_repositories", "List the startup repository allowlist and write grant. Discovered worktrees are not automatically authorized.", json!({}), &[], false),
        tool("enogram_inspect_repository", "Read the registered checkout status, history, worktrees and mutationToken. A listed worktree requires its own startup registration.", json!({"repoId": {"type": "string"}}), &["repoId"], false),
        tool("enogram_read_diff", "Read W (worktree versus index), S (index versus HEAD), or a full commit OID's first-parent diff. Preserve binary and truncated indicators.", json!({"repoId": {"type": "string"}, "filePath": {"type": "string"}, "staged": {"type": "boolean", "default": false}, "commitOid": {"type": "string"}}), &["repoId", "filePath"], false),
        tool("enogram_list_commit_files", "List files changed by a full commit object ID; merges use their first parent.", json!({"repoId": {"type": "string"}, "commitOid": {"type": "string"}}), &["repoId", "commitOid"], false),
    ];
    if write {
        let common = json!({"repoId": {"type": "string"}, "expectedToken": {"type": "string"}, "operationId": {"type": "string", "minLength": 1, "maxLength": 128}});
        for (name, description) in [
            ("enogram_stage_paths", "Stage entire current contents of exact status files. expectedToken excludes working bytes: review current files. No hunk staging."),
            ("enogram_unstage_paths", "Restore exact selected index paths to HEAD while preserving all working-file content."),
        ] {
            let mut properties = common.clone();
            properties["paths"] = json!({"type": "array", "items": {"type": "string"}});
            tools.push(tool(name, &format!("{description} {WRITE_GUIDANCE}"), properties, &["repoId", "paths", "expectedToken", "operationId"], true));
        }
        let mut properties = common;
        properties["message"] = json!({"type": "string"});
        tools.push(tool("enogram_commit_index", &format!("Commit only the prepared index with configured identity, hooks and signing. Check commitOid and commitTreeChanged; a warning means the commit already exists. {WRITE_GUIDANCE}"), properties, &["repoId", "message", "expectedToken", "operationId"], true));
    }
    tools
}

const WRITE_GUIDANCE: &str = "Use a unique operationId. Identical retries replay the cached outcome (including errors) for at most 128 operations / 32 MiB in this process; eviction/restart loses replay protection. After uncertainty, inspect; never automatically retry.";

fn tool(name: &str, description: &str, properties: Value, required: &[&str], write: bool) -> Value {
    json!({"name": name, "description": description,
        "inputSchema": {"type": "object", "properties": properties, "required": required, "additionalProperties": false},
        "annotations": {"readOnlyHint": !write, "destructiveHint": write, "idempotentHint": !write, "openWorldHint": false}})
}

enum Frame {
    Complete(Vec<u8>),
    Oversized,
    Unterminated,
    Eof,
}

fn read_frame(reader: &mut impl BufRead) -> io::Result<Frame> {
    let mut bytes = Vec::new();
    let mut oversized = false;
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return Ok(if oversized {
                Frame::Oversized
            } else if bytes.is_empty() {
                Frame::Eof
            } else {
                Frame::Unterminated
            });
        }
        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let count = newline.unwrap_or(chunk.len());
        if !oversized {
            if count > MAX_REQUEST_BYTES.saturating_sub(bytes.len()) {
                oversized = true;
                bytes.clear();
            } else {
                bytes.extend_from_slice(&chunk[..count]);
            }
        }
        let consumed = count + usize::from(newline.is_some());
        reader.consume(consumed);
        if newline.is_some() {
            if oversized {
                return Ok(Frame::Oversized);
            }
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
            return Ok(Frame::Complete(bytes));
        }
    }
}

struct BoundedBuffer {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("JSON output limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn bounded_json(value: &Value, limit: usize) -> io::Result<Vec<u8>> {
    let mut output = BoundedBuffer {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut output, value).map_err(io::Error::other)?;
    Ok(output.bytes)
}

/// Serve newline-delimited UTF-8 JSON-RPC, with bounded frame allocation and
/// protocol-only stdout. EOF exits; an unterminated final frame never executes.
pub fn serve(
    server: &mut Server,
    mut input: impl BufRead,
    mut output: impl Write,
) -> io::Result<()> {
    loop {
        let response = match read_frame(&mut input)? {
            Frame::Eof => return Ok(()),
            Frame::Oversized => Some(rpc_error(
                Value::Null,
                -32600,
                "Request exceeds the 1 MiB frame limit.",
            )),
            Frame::Unterminated => Some(rpc_error(
                Value::Null,
                -32700,
                "Unterminated JSON-RPC frame at EOF.",
            )),
            Frame::Complete(bytes) => match serde_json::from_slice(&bytes) {
                Ok(message) => server.handle(message),
                Err(_) => Some(rpc_error(Value::Null, -32700, "Invalid JSON.")),
            },
        };
        if let Some(response) = response {
            let bytes = bounded_json(&response, MAX_RESPONSE_BYTES).unwrap_or_else(|_| {
                bounded_json(&rpc_error(response.get("id").cloned().unwrap_or(Value::Null), -32603,
                    "Response exceeded the size limit. Inspect repository state; do not repeat a write automatically."), MAX_RESPONSE_BYTES).expect("Small error response")
            });
            output.write_all(&bytes)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    fn server() -> Server {
        Server {
            repositories: Vec::new(),
            allow_write: false,
            phase: Phase::Fresh,
            operations: VecDeque::new(),
            operation_bytes: 0,
        }
    }
    fn request(method: &str, params: Value) -> Value {
        json!({"jsonrpc":"2.0","id":7,"method":method,"params":params})
    }
    fn initialize(server: &mut Server) {
        let response = server
            .handle(request(
                "initialize",
                json!({"protocolVersion":"future-version",
            "capabilities":{},"clientInfo":{"name":"test","version":"1","title":"Client"}}),
            ))
            .unwrap();
        assert_eq!(response["result"]["protocolVersion"], PROTOCOL_VERSION);
        server.handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    }
    fn call(server: &mut Server, name: &str, arguments: Value) -> Value {
        server
            .handle(request(
                "tools/call",
                json!({"name":name,"arguments":arguments}),
            ))
            .unwrap()
    }

    #[test]
    fn lifecycle_unknown_probe_and_strict_tool_arguments() {
        let mut server = server();
        assert_eq!(
            server
                .handle(request("server/discover", json!({})))
                .unwrap()["error"]["code"],
            -32601
        );
        assert_eq!(
            server.handle(request("ping", json!({}))).unwrap()["result"],
            json!({})
        );
        assert_eq!(
            server.handle(request("tools/list", json!({}))).unwrap()["error"]["code"],
            -32002
        );
        initialize(&mut server);
        assert_eq!(
            server
                .handle(request(
                    "initialize",
                    json!({"protocolVersion":PROTOCOL_VERSION,
            "capabilities":{},"clientInfo":{"name":"test","version":"1"}})
                ))
                .unwrap()["error"]["code"],
            -32600
        );
        assert_eq!(
            server
                .handle(request("tools/list", json!({"cursor":"next"})))
                .unwrap()["error"]["code"],
            -32602
        );
        assert_eq!(
            server.handle(request("tools/list", json!({}))).unwrap()["result"]["tools"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
        for arguments in [Value::Null, json!({"repoPath":"dynamic"})] {
            assert_eq!(
                call(&mut server, "enogram_list_repositories", arguments)["error"]["code"],
                -32602
            );
        }
        assert_eq!(
            call(
                &mut server,
                "enogram_read_diff",
                json!({"repoId":"repo-1","filePath":"file","staged":"true"})
            )["error"]["code"],
            -32602
        );
        assert_eq!(
            call(
                &mut server,
                "enogram_commit_index",
                json!({"repoId":"repo-1","message":"x","expectedToken":"x"})
            )["error"]["code"],
            -32602
        );
        assert_eq!(
            call(&mut server, "unknown", json!({}))["error"]["code"],
            -32602
        );
        assert!(server.handle(json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"enogram_commit_index","arguments":{}}})).is_none());
        assert!(
            server.operations.is_empty(),
            "A notification cannot execute a write"
        );
        assert!(server
            .handle(json!({"jsonrpc":"2.0","id":1,"result":{}}))
            .is_none());
        assert_eq!(server.handle(json!([])).unwrap()["error"]["code"], -32600);
        assert_eq!(
            server
                .handle(json!({"jsonrpc":"2.0","id":null,"method":"ping"}))
                .unwrap()["error"]["code"],
            -32600
        );
    }

    #[test]
    fn bounded_frames_recover_after_invalid_json_oversize_and_eof() {
        let mut input = b"not-json\n".to_vec();
        input.extend(std::iter::repeat_n(b'x', MAX_REQUEST_BYTES + 40));
        input.extend_from_slice(b"\n{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"ping\"}\r\n");
        input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":10,\"method\":\"ping\"}");
        let mut output = Vec::new();
        serve(
            &mut server(),
            BufReader::with_capacity(37, Cursor::new(input)),
            &mut output,
        )
        .unwrap();
        let responses: Vec<Value> = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert_eq!(responses.len(), 4);
        assert_eq!(responses[0]["error"]["code"], -32700);
        assert_eq!(responses[1]["error"]["code"], -32600);
        assert_eq!(responses[2]["id"], 9);
        assert_eq!(responses[2]["result"], json!({}));
        assert_eq!(
            responses[3]["error"]["code"], -32700,
            "An unterminated frame must not execute"
        );
        let mut empty = Vec::new();
        serve(&mut server(), Cursor::new(Vec::<u8>::new()), &mut empty).unwrap();
        assert!(empty.is_empty());
        assert!(matches!(
            read_frame(&mut Cursor::new(vec![b'x'; MAX_REQUEST_BYTES + 1])).unwrap(),
            Frame::Oversized
        ));
    }

    #[test]
    fn bounded_output_retains_commit_and_diff_state_without_raw_diagnostics() {
        let secret = "secret-hook-password";
        let error = core_result::<Value>(
            Err(GitError {
                kind: GitErrorKind::GitCommandFailed,
                message: secret.into(),
            }),
            true,
        );
        assert!(!error.to_string().contains(secret));
        assert_eq!(error["structuredContent"]["requiresInspection"], true);
        let result = tool_result(
            json!({"text":"x".repeat(MAX_RESULT_BYTES), "commitOid":"abc", "commitTreeChanged":true,
            "binary":false,"truncated":true}),
            false,
        );
        assert_eq!(result["isError"], true);
        assert_eq!(result["structuredContent"]["kind"], "outputLimit");
        assert_eq!(result["structuredContent"]["commitOid"], "abc");
        assert_eq!(result["structuredContent"]["commitTreeChanged"], true);
        assert_eq!(result["structuredContent"]["truncated"], true);
        assert_eq!(result["structuredContent"]["binary"], false);
        assert!(bounded_json(&result, MAX_RESPONSE_BYTES).unwrap().len() < 2048);
        assert!(bounded_json(&json!({"large":"x".repeat(200)}), 20).is_err());
        let diff = core_result(
            Ok(core::DiffResult {
                text: "bounded patch".into(),
                binary: false,
                truncated: true,
            }),
            false,
        );
        assert_eq!(diff["structuredContent"]["truncated"], true);
        let decoded: Value =
            serde_json::from_str(diff["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(decoded, diff["structuredContent"]);
    }

    #[test]
    fn operation_cache_replays_errors_and_enforces_count_and_byte_eviction() {
        let mut server = server();
        initialize(&mut server);
        let args =
            json!({"repoId":"repo-1","message":"x","expectedToken":"v1:x","operationId":"same"});
        let first = call(&mut server, "enogram_commit_index", args.clone());
        assert_eq!(first["result"]["structuredContent"]["kind"], "accessDenied");
        assert_eq!(
            call(&mut server, "enogram_commit_index", args.clone()),
            first
        );
        let mut changed = args;
        changed["message"] = json!("changed");
        assert_eq!(
            call(&mut server, "enogram_commit_index", changed.clone())["result"]
                ["structuredContent"]["kind"],
            "operationIdConflict"
        );
        for id in 0..MAX_OPERATION_RECORDS {
            call(
                &mut server,
                "enogram_commit_index",
                json!({"repoId":"repo-1","message":"x","expectedToken":"v1:x","operationId":format!("count-{id}")}),
            );
        }
        assert_eq!(server.operations.len(), MAX_OPERATION_RECORDS);
        assert_eq!(
            call(&mut server, "enogram_commit_index", changed)["result"]["structuredContent"]
                ["kind"],
            "accessDenied",
            "Evicted IDs are no longer replay-protected"
        );
        for id in 0..40 {
            call(
                &mut server,
                "enogram_commit_index",
                json!({"repoId":"repo-1","message":"x".repeat(900_000),"expectedToken":"v1:x","operationId":format!("bytes-{id}")}),
            );
        }
        assert!(server.operation_bytes <= MAX_OPERATION_CACHE_BYTES);
        assert!(
            server.operations.len() < 40,
            "Byte limit must evict before the count limit"
        );
    }

    #[test]
    fn cli_rejects_missing_relative_unknown_and_repeated_options() {
        for args in [
            vec![],
            vec!["--allow-write"],
            vec!["--repo"],
            vec!["--repo", "relative"],
            vec!["--unexpected"],
        ] {
            assert!(Config::from_args(args.into_iter().map(OsString::from)).is_err());
        }
        let root = std::env::temp_dir().into_os_string();
        assert!(
            Config::from_args(vec!["--repo".into(), root.clone(), "--allow-write".into()])
                .unwrap()
                .allow_write
        );
        assert!(Config::from_args(vec![
            "--repo".into(),
            root,
            "--allow-write".into(),
            "--allow-write".into()
        ])
        .is_err());
    }
}
