import assert from "node:assert/strict";
import test from "node:test";
import {
  branchNameError,
  createRemoteConfigurationDraft,
  defaultRemoteTarget,
  networkActionBlockReason,
  remoteConfigurationAction,
  remoteConfigurationDraftError,
  remoteConfigurationHasChanges,
  remoteConfigurationNameError,
  remoteConfigurationStaleReason,
  remoteConfigurationUrlError,
  repositoryActionCommand,
  repositoryActionContext,
  repositoryActionDeadline,
  repositoryActionProgress,
  repositoryActionSuccess,
} from "./repositoryActionModel.ts";

const branch = (overrides = {}) => ({
  name: "main", head: "a".repeat(40), current: true, upstream: null,
  upstreamRemote: null, upstreamBranch: null, ahead: null, behind: null,
  worktreePath: "C:\\repo", ...overrides,
});
const remote = (name, overrides = {}) => ({
  name, fetchUrl: `${name}:fetch`, pushUrl: `${name}:push`, configToken: `${name}:configuration`,
  configuredFetchUrl: `${name}:fetch`, configuredPushUrl: `${name}:push`, hasExplicitPushUrl: true, editBlockedReason: null,
  ...overrides,
});
const snapshot = (overrides = {}) => ({
  repoPath: "C:\\repo", repoName: "repo", mutationToken: "current-repository-token",
  branch: "main", head: "a".repeat(40), gitVersion: "git test", isUnborn: false,
  mergeInProgress: false, changes: [], commits: [], worktrees: [],
  branches: [branch()], remotes: [remote("origin"), remote("upstream")], ...overrides,
});

test("default network target uses the configured upstream and short literal remote branch", () => {
  const current = branch({
    upstream: "refs/remotes/upstream/release/mobile", upstreamRemote: "upstream",
    upstreamBranch: "refs/heads/release/mobile",
  });
  assert.deepEqual(defaultRemoteTarget(snapshot({ branches: [current] })), {
    remoteName: "upstream", branchName: "release/mobile",
  });
  assert.deepEqual(defaultRemoteTarget(snapshot({ branches: [branch({ upstreamRemote: "removed", upstreamBranch: "refs/heads/old" })] })), {
    remoteName: "origin", branchName: "main",
  });
  assert.deepEqual(defaultRemoteTarget(snapshot({ remotes: [] })), { remoteName: "", branchName: "main" });
});

test("detached HEAD allows explicit push without inventing a default branch; Pull is blocked", () => {
  const detached = snapshot({ branch: "(detached)", branches: [branch({ current: false })] });
  assert.deepEqual(defaultRemoteTarget(detached), { remoteName: "origin", branchName: "" });
  assert.equal(networkActionBlockReason("fetch", detached), null);
  assert.equal(networkActionBlockReason("push", detached), null);
  assert.match(networkActionBlockReason("pull", detached), /yerel bir dala/);
});

test("Pull guards clean checkout state while Fetch and Push can retain local edits", () => {
  for (const changes of [
    [{ path: "staged.txt", originalPath: null, indexStatus: "M", worktreeStatus: "." }],
    [{ path: "untracked.txt", originalPath: null, indexStatus: "?", worktreeStatus: "?" }],
  ]) {
    const dirty = snapshot({ changes });
    assert.match(networkActionBlockReason("pull", dirty), /temiz/);
    assert.equal(networkActionBlockReason("fetch", dirty), null);
    assert.equal(networkActionBlockReason("push", dirty), null);
  }
  assert.match(networkActionBlockReason("pull", snapshot({ mergeInProgress: true })), /merge/);
  assert.equal(networkActionBlockReason("pull", snapshot()), null);
});

test("missing remotes and unborn HEAD have distinct actionable disabled reasons", () => {
  for (const kind of ["fetch", "pull", "push"]) {
    assert.match(networkActionBlockReason(kind, snapshot({ remotes: [] })), /Uzak depo tanımlı değil/);
  }
  const unborn = snapshot({ isUnborn: true, head: null, branches: [] });
  assert.equal(networkActionBlockReason("fetch", unborn), null);
  assert.match(networkActionBlockReason("pull", unborn), /ilk commit/);
  assert.match(networkActionBlockReason("push", unborn), /ilk commit/);
});

test("mutation commands retain snapshot, branch-head and remote configuration identities", () => {
  const target = { repoPath: "C:\\repo with spaces", expectedToken: "shown-token" };
  assert.deepEqual(repositoryActionCommand({ ...target, kind: "switchBranch", name: "feature/çalışma", expectedHead: "branch-head" }), {
    command: "switch_branch", args: { ...target, name: "feature/çalışma", expectedHead: "branch-head" },
  });
  assert.deepEqual(repositoryActionCommand({ ...target, kind: "createBranch", name: "feature/new", checkout: false }), {
    command: "create_branch", args: { ...target, name: "feature/new", checkout: false },
  });
  const remoteTarget = { ...target, remoteName: "upstream", expectedRemoteToken: "shown-remote-config" };
  assert.deepEqual(repositoryActionCommand({ ...remoteTarget, kind: "fetch" }), {
    command: "fetch_remote", args: remoteTarget,
  });
  for (const kind of ["pull", "push"]) {
    assert.deepEqual(repositoryActionCommand({ ...remoteTarget, kind, branchName: "release/mobile", localBranch: "feature/mobile" }), {
      command: `${kind}_remote`, args: { ...remoteTarget, branchName: "release/mobile" },
    });
  }
});

test("branch form preserves literal spelling and does not silently trim names", () => {
  assert.match(branchNameError("  feature/work"), /boşluk/);
  assert.match(branchNameError("main", [branch()]), /zaten var/);
  assert.equal(branchNameError("Main", [branch()]), null);
  assert.equal(branchNameError("feature/çalışma"), null);
});

test("a first remote defaults to origin and inherits Fetch for Push without a network operation", () => {
  const current = snapshot({ remotes: [] });
  const draft = createRemoteConfigurationDraft(current);
  assert.equal(draft.name, "origin");
  assert.equal(draft.fetchUrl, "");
  assert.equal(draft.pushUrl, "");
  assert.equal(draft.separatePushUrl, false);
  assert.match(remoteConfigurationDraftError(draft), /adres/);
  draft.fetchUrl = "C:\\repos with spaces\\remote.git";
  assert.equal(remoteConfigurationDraftError(draft), null);
  assert.deepEqual(repositoryActionCommand(remoteConfigurationAction(draft)), {
    command: "add_remote",
    args: { repoPath: current.repoPath, expectedToken: current.mutationToken, name: "origin", fetchUrl: draft.fetchUrl, pushUrl: null },
  });
  assert.match(networkActionBlockReason("fetch", current), /Uzak depolar penceresinden/);
});

test("editing uses configured literal addresses and never writes their resolved display replacements", () => {
  const entry = remote("origin", {
    configuredFetchUrl: "work:team/repo.git", fetchUrl: "ssh://[redacted]@host/team/repo.git",
    configuredPushUrl: "publish:team/repo.git", pushUrl: "ssh://[redacted]@write-host/team/repo.git",
  });
  const draft = createRemoteConfigurationDraft(snapshot(), entry);
  assert.equal(draft.fetchUrl, "work:team/repo.git");
  assert.equal(draft.pushUrl, "publish:team/repo.git");
  assert.equal(remoteConfigurationHasChanges(draft), false);
  assert.deepEqual(repositoryActionCommand(remoteConfigurationAction(draft)), {
    command: "update_remote",
    args: {
      repoPath: draft.repoPath, expectedToken: draft.expectedToken, remoteName: "origin", expectedRemoteToken: entry.configToken,
      fetchUrl: null, pushUrl: null, updatePushUrl: false,
    },
  });
});

test("masked configured addresses remain blank and preserve the original unless deliberately replaced", () => {
  const entry = remote("origin", {
    configuredFetchUrl: null, fetchUrl: "https://[redacted]@host/repo.git",
    configuredPushUrl: null, pushUrl: "ssh://[redacted]@push-host/repo.git",
  });
  const draft = createRemoteConfigurationDraft(snapshot(), entry);
  assert.equal(draft.fetchUrl, "");
  assert.equal(draft.pushUrl, "");
  assert.equal(remoteConfigurationDraftError(draft), null);
  assert.equal(remoteConfigurationHasChanges(draft), false);
  assert.equal(remoteConfigurationAction(draft).fetchUrl, null);
  assert.equal(remoteConfigurationAction(draft).updatePushUrl, false);

  draft.fetchUrl = "git@new-host:team/repo.git";
  assert.deepEqual(remoteConfigurationAction(draft), {
    kind: "updateRemote", repoPath: draft.repoPath, expectedToken: draft.expectedToken,
    remoteName: "origin", expectedRemoteToken: entry.configToken,
    fetchUrl: "git@new-host:team/repo.git", pushUrl: null, updatePushUrl: false,
  });
  draft.fetchUrl = "";
  draft.pushUrl = "ssh://git@new-push-host/team/repo.git";
  assert.equal(remoteConfigurationAction(draft).fetchUrl, null);
  assert.equal(remoteConfigurationAction(draft).pushUrl, draft.pushUrl);
  assert.equal(remoteConfigurationAction(draft).updatePushUrl, true);
});

test("separate Push can be added, preserved, and deliberately changed back to Fetch inheritance", () => {
  const inherited = remote("origin", { configuredPushUrl: null, hasExplicitPushUrl: false });
  const draft = createRemoteConfigurationDraft(snapshot(), inherited);
  assert.equal(remoteConfigurationHasChanges(draft), false);
  draft.separatePushUrl = true;
  assert.match(remoteConfigurationDraftError(draft), /adres/);
  draft.pushUrl = "git@host:write/repo.git";
  assert.equal(remoteConfigurationDraftError(draft), null);
  assert.equal(remoteConfigurationAction(draft).pushUrl, draft.pushUrl);
  assert.equal(remoteConfigurationAction(draft).updatePushUrl, true);
  draft.separatePushUrl = false;
  assert.equal(remoteConfigurationHasChanges(draft), false);

  const explicit = createRemoteConfigurationDraft(snapshot(), remote("origin", { configuredPushUrl: null }));
  explicit.separatePushUrl = false;
  assert.equal(remoteConfigurationDraftError(explicit), null);
  assert.equal(remoteConfigurationAction(explicit).updatePushUrl, true);
  assert.equal(remoteConfigurationAction(explicit).pushUrl, null);
  assert.equal(remoteConfigurationAction(explicit).fetchUrl, null);
  explicit.separatePushUrl = true;
  assert.equal(remoteConfigurationHasChanges(explicit), false);
});

test("remote drafts keep the captured identities when snapshots change and edit names are immutable", () => {
  const entry = remote("origin");
  const current = snapshot({ remotes: [entry] });
  const draft = createRemoteConfigurationDraft(current, entry);
  draft.name = "attempted-rename";
  draft.fetchUrl = "C:\\new remote";
  entry.configToken = "later-config";
  current.mutationToken = "later-snapshot";
  const action = remoteConfigurationAction(draft);
  assert.equal(action.remoteName, "origin");
  assert.equal(action.expectedRemoteToken, "origin:configuration");
  assert.equal(action.expectedToken, "current-repository-token");
  assert.match(remoteConfigurationStaleReason(draft, current), /Uzak depo ayarı/);
  assert.equal(draft.fetchUrl, "C:\\new remote");
  assert.match(remoteConfigurationStaleReason(draft, snapshot({ remotes: [] })), /kaldırılmış/);
  assert.match(remoteConfigurationStaleReason(draft, snapshot({ mutationToken: "new-checkout" })), /Depo durumu/);
  assert.match(remoteConfigurationStaleReason(draft, snapshot({ repoPath: "C:\\other" })), /Açık depo/);
  assert.equal(remoteConfigurationStaleReason(draft, snapshot()), null);
});

test("remote validation preserves literal input, accepts real Git transports and rejects unsafe form syntax", () => {
  assert.match(remoteConfigurationNameError(" origin"), /boşluk/);
  assert.match(remoteConfigurationNameError("up stream"), /boşluk/);
  assert.ok(remoteConfigurationNameError("-origin"));
  assert.ok(remoteConfigurationNameError("bad:name"));
  assert.match(remoteConfigurationNameError("origin", [remote("origin")]), /zaten var/);
  assert.equal(remoteConfigurationNameError("Origin", [remote("origin")]), null);
  assert.equal(remoteConfigurationNameError("ekip/çalışma"), null);
  for (const address of [
    "https://host/repo.git", "ssh://git@host/team/repo.git", "git@host:team/repo.git",
    "https://[2001:db8::1]/repo.git", "ssh://git@[::1]/repo.git", "C:\\repos with spaces\\repo.git", "../local repo.git",
  ]) assert.equal(remoteConfigurationUrlError(address), null, address);
  for (const address of ["", "  https://host/repo.git", "https://host/repo.git ", "host/repo\nnext", "-bad", "ext::git shell", "unknown://host/repo.git"]) {
    assert.ok(remoteConfigurationUrlError(address), address);
  }
  const draft = createRemoteConfigurationDraft(snapshot(), remote("origin"));
  draft.fetchUrl = "";
  assert.match(remoteConfigurationDraftError(draft), /adres/);
});

test("unsupported configuration stays read-only even if an input is changed", () => {
  const reason = "Birden fazla Fetch adresi düzenlenemez.";
  const draft = createRemoteConfigurationDraft(snapshot(), remote("origin", { editBlockedReason: reason, configuredFetchUrl: null }));
  draft.fetchUrl = "https://host/new.git";
  assert.equal(remoteConfigurationDraftError(draft), reason);
});

test("configuration action feedback names the remote without including submitted addresses", () => {
  const draft = createRemoteConfigurationDraft(snapshot({ remotes: [] }));
  draft.fetchUrl = "https://private-user:private-password@host/repo.git?private-token";
  const add = remoteConfigurationAction(draft);
  const update = remoteConfigurationAction({ ...createRemoteConfigurationDraft(snapshot(), remote("origin")), fetchUrl: draft.fetchUrl });
  for (const action of [add, update]) {
    const text = [repositoryActionContext(action), repositoryActionProgress(action), repositoryActionSuccess(action), repositoryActionDeadline(action.kind)].join(" ");
    assert.match(text, /origin/);
    assert.doesNotMatch(text, /private|host|https/);
    assert.match(repositoryActionDeadline(action.kind), /Yerel komut/);
  }
});

test("explicit English locale reaches nested validators without changing Git commands", () => {
  assert.equal(branchNameError("", [], "en"), "Enter a branch name.");
  assert.match(networkActionBlockReason("pull", snapshot({ changes: [{ path: "changed.txt" }] }), "en"), /clean working tree and index/);
  assert.match(networkActionBlockReason("fetch", snapshot({ remotes: [] }), "en"), /Remotes dialog/);
  const draft = createRemoteConfigurationDraft(snapshot({ remotes: [] }));
  draft.name = "";
  assert.equal(remoteConfigurationDraftError(draft, [], "en"), "Enter a remote name.");
  draft.name = "origin";
  assert.equal(remoteConfigurationDraftError(draft, [], "en"), "Enter a repository address.");
  draft.fetchUrl = "C:\\depolar\\çalışma.git";
  draft.separatePushUrl = true;
  assert.equal(remoteConfigurationDraftError(draft, [], "en"), "Enter a repository address.");
  draft.pushUrl = "git@host:ekip/çalışma.git";
  assert.equal(remoteConfigurationDraftError(draft, [], "en"), null);
  assert.match(remoteConfigurationStaleReason(draft, snapshot({ mutationToken: "later" }), "en"), /repository state changed/);
  const action = remoteConfigurationAction(draft);
  const command = repositoryActionCommand(action);
  assert.match(repositoryActionSuccess(action, "en"), /Added remote “origin”/);
  assert.match(repositoryActionContext(action, "en"), /Add remote/);
  assert.match(repositoryActionProgress(action, "en"), /Adding remote/);
  assert.match(repositoryActionDeadline(action.kind, "en"), /Local command limit/);
  assert.deepEqual(repositoryActionCommand(action), command);
  assert.equal(command.args.fetchUrl, draft.fetchUrl);
  assert.equal(command.args.pushUrl, draft.pushUrl);
});

test("operation feedback localizes every action while preserving target names", () => {
  const target = { repoPath: "C:\\repo", expectedToken: "token" };
  const remoteTarget = { ...target, remoteName: "ekip", expectedRemoteToken: "remote-token" };
  const actions = [
    { ...target, kind: "createBranch", name: "özellik", checkout: false },
    { ...target, kind: "createBranch", name: "özellik", checkout: true },
    { ...target, kind: "switchBranch", name: "özellik", expectedHead: "a".repeat(40) },
    { ...remoteTarget, kind: "updateRemote", fetchUrl: null, pushUrl: null, updatePushUrl: false },
    { ...remoteTarget, kind: "fetch" },
    { ...remoteTarget, kind: "pull", branchName: "özellik", localBranch: "çalışma" },
    { ...remoteTarget, kind: "push", branchName: "özellik", localBranch: "çalışma" },
  ];
  for (const action of actions) {
    assert.ok(repositoryActionContext(action, "en"));
    assert.notEqual(repositoryActionProgress(action, "en"), repositoryActionProgress(action, "tr"));
    assert.notEqual(repositoryActionSuccess(action, "en"), repositoryActionSuccess(action, "tr"));
    assert.notEqual(repositoryActionDeadline(action.kind, "en"), repositoryActionDeadline(action.kind, "tr"));
  }
  assert.match(repositoryActionSuccess(actions.at(-1), "en"), /çalışma → ekip\/özellik/);
});
