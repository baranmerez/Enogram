# Windows installation

Enogram's preview can be packaged as a Windows x64 NSIS setup executable. No public installer release is available yet; [build the package from source](#build-the-package) before following the setup steps below. The generated package contains release-mode builds of both the Tauri desktop and the standalone MCP server. It does not depend on the development checkout or its `target/debug` directory.

## Use the setup

1. Run `Enogram_0.1.0_x64-setup.exe` and choose the installation folder. The default is `%LOCALAPPDATA%\Enogram`, for the current Windows user.
2. Launch Enogram from the Start menu. On first launch, enter an existing Git checkout path. The last successfully opened repository is remembered on this device; a moved or unavailable repository leaves an editable path and an error so you can open another one.
3. Install Git for Windows if it is not already available. Enogram uses the installed Git, identity, credentials, hooks, filters, and signing settings.

Node.js, npm, Rust, and Visual C++ build tools are development prerequisites, not prerequisites for using this package. The desktop requires the Microsoft WebView2 runtime. Setup downloads and runs the WebView2 bootstrapper only when the runtime is missing, so that case requires internet access. The MCP server does not require the desktop to be open.

This preview is unsigned. Windows may display an unknown-publisher or SmartScreen notice. Enogram is released under the [MIT License](../LICENSE); code signing and publishing an installer are separate release steps.

## Installed files and MCP clients

The installation directory contains:

- `enogram-desktop.exe`: the desktop application, including its built frontend.
- `enogram-mcp.exe`: the standalone stdio MCP server built from the same checkout.
- `docs/INSTALLATION.md` and `docs/MCP.md`: installation and client setup instructions.

Use the absolute installed MCP path in Codex, Claude Code, or Claude Desktop. For the default directory, PowerShell resolves it with:

```powershell
$mcpPath = Join-Path $env:LOCALAPPDATA 'Enogram\enogram-mcp.exe'
Test-Path -LiteralPath $mcpPath
```

The examples in [MCP setup](MCP.md) show development executable paths. Replace those executable paths with your resolved `$mcpPath`; keep `--repo` pointed at your actual checkout. Client configuration files must contain the expanded absolute path, not the literal `$mcpPath` or `%LOCALAPPDATA%` text.

Add `--allow-write` only for repositories where the client should stage, unstage, and commit. Setup does not register clients, modify their settings, grant repository access, or start an MCP server. Each client owns its connection and process.

## Update and remove

Finish active Git operations, close the installed desktop, and stop the installed Enogram MCP connections through their owning clients before updating or uninstalling. Restart the connections afterward, list repositories again, and inspect current Git state before the next write. An operation interrupted before an update may already have taken effect; never assume that a timeout or restart rolled it back.

Use **Installed apps > Enogram > Uninstall** to remove the package. Repositories are external to the installation directory and are not removed. Keep repositories and other personal files outside the application directory. Remove obsolete MCP client entries separately when permanently uninstalling.

## Build the package

From a Windows x64 checkout with Node.js, Rust MSVC, the Visual C++ build tools, and the Windows SDK:

```powershell
npm ci
npm run installer:build
```

The helper uses the locked Cargo dependencies, builds MCP in release mode, stages its architecture-specific sidecar, and then builds the desktop and NSIS package. Generated inputs and build outputs are ignored by Git. Development builds keep their existing configuration; packaging does not overwrite an MCP executable being used from `target/debug`.

The setup output is:

```text
target/installer/x86_64-pc-windows-msvc/release/bundle/nsis/Enogram_0.1.0_x64-setup.exe
```

The first build may download Tauri's NSIS tooling. Package creation does not install Enogram or change any MCP registration. Preserve the generated package and its SHA-256 together when transferring it to another computer.

Tauri documents the [Windows installer options](https://v2.tauri.app/distribute/windows-installer/) and [external binary packaging](https://v2.tauri.app/develop/sidecar/). Our installer configuration targets x64 and current-user installation; other architectures have not been packaged or verified.
