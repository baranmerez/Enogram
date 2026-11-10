import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { accessSync, copyFileSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = fileURLToPath(new URL("../", import.meta.url));
const targetTriple = "x86_64-pc-windows-msvc";
const targetDir = path.join(repoRoot, "target", "installer");
const stagingDir = path.join(repoRoot, ".artifacts", "installer");
const stagedMcp = path.join(stagingDir, `enogram-mcp-${targetTriple}.exe`);
const manifestPath = path.join(stagingDir, "build-manifest.json");
const releaseDir = path.join(targetDir, targetTriple, "release");
const tauriCli = path.join(repoRoot, "node_modules", "@tauri-apps", "cli", "tauri.js");
const installerConfig = path.join(repoRoot, "src-tauri", "windows", "tauri.installer.conf.json");
const requiredRustFlags = ["-C", "target-feature=+crt-static"];
const buildEnv = {
  ...process.env,
  CARGO_TARGET_DIR: targetDir,
  RUSTFLAGS: `${process.env.RUSTFLAGS ?? ""} ${requiredRustFlags.join(" ")}`.trim(),
};
// Cargo gives encoded flags precedence over RUSTFLAGS when callers set them.
if (process.env.CARGO_ENCODED_RUSTFLAGS !== undefined) {
  buildEnv.CARGO_ENCODED_RUSTFLAGS = [process.env.CARGO_ENCODED_RUSTFLAGS, ...requiredRustFlags]
    .filter(Boolean).join("\u001f");
}

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    env: buildEnv,
    stdio: "inherit",
    windowsHide: true,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${path.basename(command)} failed (${result.signal ?? `exit ${result.status}`}).`);
  }
}

function describeArtifact(filePath) {
  const contents = readFileSync(filePath);
  return {
    path: path.relative(repoRoot, filePath).split(path.sep).join("/"),
    bytes: contents.length,
    sha256: createHash("sha256").update(contents).digest("hex"),
  };
}

function buildInstaller() {
  if (process.platform !== "win32") {
    throw new Error("Build this installer on Windows with the x86_64-pc-windows-msvc Rust target installed.");
  }
  accessSync(tauriCli);
  accessSync(installerConfig);
  accessSync(path.join(repoRoot, "docs", "MCP.md"));
  accessSync(path.join(repoRoot, "docs", "INSTALLATION.md"));

  const config = JSON.parse(readFileSync(path.join(repoRoot, "src-tauri", "tauri.conf.json"), "utf8"));
  const installerPath = path.join(releaseDir, "bundle", "nsis", `${config.productName}_${config.version}_x64-setup.exe`);
  mkdirSync(stagingDir, { recursive: true });
  // A failed build must not leave an older sidecar or setup looking like fresh output.
  rmSync(stagedMcp, { force: true });
  rmSync(installerPath, { force: true });
  rmSync(manifestPath, { force: true });

  try {
    console.log("Building the current Enogram MCP release with static MSVC runtime...");
    run("cargo", ["build", "--locked", "--release", "--target", targetTriple, "--package", "enogram-mcp"]);
    copyFileSync(path.join(releaseDir, "enogram-mcp.exe"), stagedMcp);

    console.log("Building the Enogram desktop and per-user Windows installer...");
    run(process.execPath, [tauriCli, "build", "--target", targetTriple, "--bundles", "nsis", "--config", installerConfig, "--", "--locked"]);
    accessSync(installerPath);
    writeFileSync(manifestPath, `${JSON.stringify({
      schemaVersion: 1,
      builtAt: new Date().toISOString(),
      productName: config.productName,
      version: config.version,
      target: targetTriple,
      profile: "release",
      staticCrt: true,
      requiredRustFlags,
      artifacts: {
        desktopStandalone: {
          ...describeArtifact(path.join(releaseDir, "enogram-desktop.exe")),
          hashScope: "Standalone release executable restored by Tauri after bundling; the NSIS payload has its own bundle-type marker and hash.",
        },
        mcp: describeArtifact(path.join(releaseDir, "enogram-mcp.exe")),
        stagedMcp: describeArtifact(stagedMcp),
        installer: describeArtifact(installerPath),
      },
    }, null, 2)}\n`);
    console.log(`Installer ready: ${installerPath}`);
    console.log(`Build manifest: ${manifestPath}`);
  } catch (error) {
    for (const generatedPath of [stagedMcp, installerPath, manifestPath]) {
      try {
        rmSync(generatedPath, { force: true });
      } catch (cleanupError) {
        console.error(`Could not remove incomplete build output: ${cleanupError.message}`);
      }
    }
    throw error;
  }
}

if (process.argv.length === 3 && process.argv[2] === "--help") {
  console.log("Usage: npm run installer:build\nBuilds the current Windows x64 desktop and MCP releases into one per-user NSIS installer.");
} else {
  try {
    if (process.argv.length !== 2) throw new Error("This build has no optional arguments. Use --help for usage.");
    buildInstaller();
  } catch (error) {
    console.error(`Installer build failed: ${error.message}`);
    process.exitCode = 1;
  }
}
