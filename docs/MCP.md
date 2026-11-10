# Local MCP server

`enogram-mcp` exposes Enogram's local repository reads and file-level stage, unstage, and commit operations to Codex, Claude Code, and Claude Desktop. Each client starts its own stdio process. The server uses `enogram-core` and can run without the desktop application.

This is a standalone local increment. The roadmap's application-owned IPC bridge, authenticated client registrations, GUI connection/grant controls, durable operation journal, and remote Windows checkout support remain future work. The MCP server does not expose push, branch changes, remote configuration changes, destructive cleanup, or arbitrary shell execution.

## Installed server

The locally built Windows setup includes `enogram-mcp.exe` beside `enogram-desktop.exe`; the default directory is `%LOCALAPPDATA%\Enogram`. No public installer release is available yet. Use the resolved absolute installed path in the client configuration, along with your explicit `--repo` paths. The setup does not edit client settings or enable writes. See [Windows installation](INSTALLATION.md) for building, installation, and updates.

The examples below use a source checkout's development executable. When using the installed package, replace only that executable path with the installed one. Node.js and Rust are not required to run the installed MCP server.

## Build and launch from source

Build from the Enogram checkout with the existing Rust toolchain:

```powershell
cargo build -p enogram-mcp
```

The Windows executable is `target/debug/enogram-mcp.exe`. The shared core detects the installed system Git. Use absolute executable and checkout paths in client configuration. These examples use `C:\example\enogram` for the Enogram source checkout and `C:\example\repository` for the repository exposed to the client.

```powershell
& 'C:\example\enogram\target\debug\enogram-mcp.exe' --repo 'C:\example\repository'
```

The process waits for MCP messages on stdin; it is not an interactive command prompt. It uses newline-delimited JSON-RPC over stdio, writes protocol messages to stdout, and exits when stdin closes. There is no HTTP listener or network authentication setup.

`--repo <absolute checkout root>` is required and repeatable. The startup list defines the accessible repositories. Clients use returned IDs such as `repo-1`; they cannot supply another repository path in a tool call. Query the list after connecting rather than persisting IDs across configuration changes. A linked worktree needs its own `--repo` entry to be callable, even when it appears in another repository's worktree inventory.

The default is read-only. Add `--allow-write` to enable stage, unstage, and commit for the configured repositories:

```powershell
& 'C:\example\enogram\target\debug\enogram-mcp.exe' --repo 'C:\example\repository' --allow-write
```

Changing the allowlist or write mode requires restarting that server process. Client-side tool approval is an additional client policy; it does not replace the server's startup restrictions. Use trusted repositories: the shared core preserves configured Git behavior, including filters, hooks, identity, and signing.

## Updating a running server

Each client owns a separate MCP process. Before replacing its executable on Windows, finish any in-flight write and stop the affected Enogram server through that client's MCP controls, or close the owning client session. A running executable can block the rebuild; an already-running server also keeps its original startup grants and in-memory replay cache.

Rebuild the server, then restart each affected connection. List repositories again and inspect the current state before the next write. Repository IDs follow the new startup list, and operation replay records do not survive the restart. If the previous write was interrupted, reconcile HEAD and the index before considering another operation. Restarting one client's connection does not reload another client's server; avoid closing unrelated active work just to refresh Enogram.

## Codex

Register a read-only server:

```powershell
codex mcp add enogram -- 'C:\example\enogram\target\debug\enogram-mcp.exe' --repo 'C:\example\repository'
codex mcp get enogram --json
```

For write access, append `--allow-write` to the server arguments before registration, or add it to the existing entry's `args` and restart the server. The equivalent entry in the active Codex configuration is:

```toml
[mcp_servers.enogram]
command = 'C:\example\enogram\target\debug\enogram-mcp.exe'
args = ['--repo', 'C:\example\repository']
```

Start a new Codex session and check `/mcp`, then request an Enogram repository inspection. `codex mcp get` verifies saved configuration; a successful tool call verifies runtime access. The [official Codex MCP guide](https://learn.chatgpt.com/docs/extend/mcp?surface=cli) covers configuration locations, restart controls, and optional tool allowlists.

For an explicitly authorized noninteractive write, configure approval for the required Enogram tool in that invocation, for example `-c 'mcp_servers.enogram.tools.enogram_stage_paths.approval_mode="approve"'`. Apply the same per-tool setting to `enogram_commit_index` or `enogram_unstage_paths` only when the run needs that operation. An `approval_policy` of `never` alone does not approve MCP writes: it rejects a call that still needs approval before the server executes it. Keep the server's `--allow-write` and repository allowlist in place; this client setting changes neither restriction.

## Claude Code

Register from the project where the server should be available:

```powershell
Set-Location 'C:\example\repository'
$entry = @{
  type = 'stdio'
  command = 'C:\example\enogram\target\debug\enogram-mcp.exe'
  args = @('--repo', 'C:\example\repository')
} | ConvertTo-Json -Compress
claude mcp add-json --scope local enogram $entry
claude mcp get enogram
```

`--scope local` keeps this registration private to the current project in the user's Claude configuration. `--scope user` makes the registration available across projects; the server's repository allowlist still applies. The structured command avoids PowerShell wrapper ambiguity around the `--` argument separator. Append `'--allow-write'` to `args` when enabling writes. Start a new session, check `/mcp`, and request a repository inspection. See the [official Claude Code MCP guide](https://code.claude.com/docs/en/mcp).

For an isolated automation run, Claude Code accepts `--mcp-config <JSON file>` with `--strict-mcp-config`. The JSON has the same `mcpServers` shape shown below. `--tools ""` removes built-in tools; `--allowedTools` accepts the full MCP tool names, for example `mcp__enogram__enogram_inspect_repository`. Allow only the operations needed for that run. `--permission-mode dontAsk` denies calls that would otherwise require an unanswered permission prompt; it does not enable the server's write mode.

## Claude Desktop

Open Claude Desktop's local MCP server/developer settings and choose **Edit Config**. Use the file opened by the application. The conventional Windows location is `%APPDATA%\Claude\claude_desktop_config.json`; installed Windows packages may relocate application data.

Merge this entry into the existing `mcpServers` object, preserving other settings:

```json
{
  "mcpServers": {
    "enogram": {
      "command": "C:\\example\\enogram\\target\\debug\\enogram-mcp.exe",
      "args": ["--repo", "C:\\example\\repository"]
    }
  }
}
```

For write access, append `"--allow-write"` to `args`. Fully quit and restart Claude Desktop, then check its local MCP server status and tool list. Use a chat tool call to inspect the configured repository. Registration in Claude Code does not register a local MCP server in Claude Desktop's chat interface.

The [MCP local-server guide](https://modelcontextprotocol.io/docs/develop/connect-local-servers) documents the JSON format and restart procedure. [Claude's local MCP help](https://support.claude.com/en/articles/10949351-getting-started-with-local-mcp-servers-on-claude-desktop) identifies the connection-status and log controls in current and older settings layouts.

## Tools and write workflow

Tool arguments use camelCase. All repository-specific tools take a `repoId` returned by `enogram_list_repositories`.

| Tool | Additional arguments | Result or operation |
| --- | --- | --- |
| `enogram_list_repositories` | None | List startup-authorized repositories and their IDs. |
| `enogram_inspect_repository` | None | Read the core repository snapshot, including `mutationToken`, changes, history, branches, worktrees, and remote metadata. |
| `enogram_read_diff` | `filePath`, optional `staged` (default `false`), optional `commitOid` | Read a working, staged, or commit diff; preserve the core `text`, `truncated`, and `binary` fields. |
| `enogram_list_commit_files` | `commitOid` | List the files changed by that commit. |
| `enogram_stage_paths` | `paths`, `expectedToken`, `operationId` | Stage the complete current contents of explicit changed files. Requires `--allow-write`. |
| `enogram_unstage_paths` | `paths`, `expectedToken`, `operationId` | Restore those index entries to HEAD, or remove them from an unborn index, while preserving working files. Requires `--allow-write`. |
| `enogram_commit_index` | `message`, `expectedToken`, `operationId` | Commit the prepared index with normal identity, hooks, and signing. Requires `--allow-write`. |

For a write:

1. List repositories, select the intended ID, and inspect its current state.
2. Review the relevant diff and exact file paths. Copy the snapshot's `mutationToken` into `expectedToken`.
3. Supply a new `operationId` for the intended operation and invoke one write tool.
4. Read the outcome's `snapshot` before the next write. Use its new `mutationToken`. A commit outcome also includes `commitOid` and `commitTreeChanged`; inspect a created commit when the latter reports changed content.

The token binds the core's checkout identity, HEAD, index, and relevant operation state. It does not hash working-file contents. Editing a file after reviewing its diff can therefore change what a subsequent whole-file stage includes. Staging is not hunk staging. Commit consumes the existing index and never stages remaining changes automatically.

Updated Enogram desktop and MCP binaries share a Windows-session mutation mutex. A concurrent Enogram mutation returns `Busy` instead of waiting or retrying. External Git processes, older Enogram binaries, and other Windows sessions remain outside that coordination boundary. Token revalidation reduces stale-state writes but does not make the final Git operation atomic with external processes.

Each MCP process keeps a FIFO cache of up to 128 write operation IDs or 32 MiB of records. While an entry is retained, repeating its ID with the same arguments returns the exact cached outcome, including an error; reusing it with different arguments is rejected. This is not durable or cross-client deduplication. A process restart or cache eviction removes that protection. After an interrupted or failed write, inspect the repository and reconcile the observed state before deciding on a new operation. A Git failure or timeout can occur after a side effect; neither automatic retry nor rollback is promised.

Input frames are limited to 1 MiB, result data to 8 MiB, and encoded responses to 32 MiB. An oversized result is reported explicitly; a write may already have completed, and an available commit ID/content-change warning remains in the bounded response. Repository content is data, not a source of instructions for the client.

## Compatibility and verification

The server implements the legacy MCP `2025-11-25` initialization protocol. It does not implement revision `2026-07-28`. A client probing `server/discover` receives an unsupported-method response and can fall back to `initialize`. Claude Code documents that its newer runtime preserves this fallback. See [MCP lifecycle/version negotiation](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle) and [Claude Code MCP runtimes](https://code.claude.com/docs/en/mcp#mcp-client-runtimes).

Setup syntax was checked on 2026-11-07 against installed Codex CLI `0.167.0-alpha.3.2`, Claude Code `2.1.296`, and Claude Desktop Windows package `4.23118.0.0`. Detecting a version or an authenticated CLI does not establish successful integration.

Keep these verification layers separate:

- **Server:** `cargo test -p enogram-mcp` exercises the protocol and repository restrictions; `cargo test -p enogram-core` exercises shared Git behavior.
- **Client connection:** each actual client launches the configured executable, discovers the tools, and completes a repository inspection.
- **Write acceptance:** each target client completes an authorized inspect → stage → inspect → commit sequence, with the resulting index/tree/commit checked in Git. Also verify read-only rejection, stale-token rejection, wrong-repository rejection, and duplicate-operation behavior at the server layer.

Client configuration, a protocol fixture, and a visible desktop status each provide different evidence. Report which checks ran and any unverified client or GUI boundary. The standalone server's presence does not establish a GUI connection indicator or the roadmap's complete application-owned bridge milestone.
