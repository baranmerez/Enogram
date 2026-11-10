import assert from "node:assert/strict";
import test from "node:test";
import { repositoryAppHarness, textContent } from "../scripts/repositoryAppHarness.mjs";
import { workspacePreferencesKey } from "./workspacePreferences.ts";

const repoA = String.raw`\\?\C:\work\repo-a`;
const repoB = String.raw`\\?\C:\work\repo-b`;
const oidA = "a".repeat(40);
const oidB = "b".repeat(40);
const file = (path, indexStatus = "M", worktreeStatus = "M") => ({
  path, originalPath: null, indexStatus, worktreeStatus,
});
const commit = (oid) => ({ oid, parents: [], subject: oid, author: "Test", date: "2026-11-07T12:00:00Z", refs: "" });
const snapshot = (repoPath = repoA, overrides = {}) => ({
  repoPath, repoName: "fixture", mutationToken: "token", branch: "main", head: oidA,
  gitVersion: "git fixture", isUnborn: false, mergeInProgress: false,
  changes: [file("working.txt")], commits: [commit(oidA), commit(oidB)],
  worktrees: [], branches: [], remotes: [], ...overrides,
});
const byClass = (className) => (node) => node.props.className?.split(" ").includes(className);
const composer = (app) => app.find((node) => node.props.id === "commit-message");
const diffView = (app) => app.find((node) => node.type === "DiffView");
const fileRows = (app) => app.all((node) => node.type?.name === "FileRow");
const selectedFile = (app) => fileRows(app).find((node) => node.props.selected)?.props.file.path;
const preventDefault = () => {};
const lastRepositoryStorageKey = "enogram.lastRepository.v1";
const partialDiff = (overrides = {}) => ({
  text: "diff --git a/working.txt b/working.txt\n--- a/working.txt\n+++ b/working.txt\n@@ -1,2 +1,3 @@\n-old\n+new\n context\n+extra\n",
  binary: false, truncated: false, selectionToken: "selection-v1", selectableLineIds: [4, 5, 7],
  unsupportedReason: null, ...overrides,
});

async function resolvePartialDiff(app, result = partialDiff()) {
  app.take("prepare_partial_diff").resolve(result);
  await app.flush();
  return diffView(app).props.partialSelection;
}

function memoryStorage(savedPath) {
  const values = new Map(savedPath === undefined ? [] : [[lastRepositoryStorageKey, savedPath]]);
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
  };
}

async function mount(t, initial = snapshot()) {
  const app = repositoryAppHarness();
  t.after(() => app.unmount());
  await app.flush();
  await open(app, initial.repoPath, initial);
  return app;
}

async function editDraft(app, value) {
  composer(app).props.onChange({ target: { value } });
  await app.flush();
}

async function open(app, path, response) {
  app.find((node) => node.props["aria-label"] === "Depo yolu").props.onChange({ target: { value: path } });
  await app.flush();
  app.find(byClass("repo-open-form")).props.onSubmit({ preventDefault });
  await app.flush();
  const request = app.take("inspect_repository");
  if (response instanceof Error) request.reject(response);
  else request.resolve(response);
  await app.flush();
}

async function refresh(app, response) {
  app.find((node) => node.props["aria-label"] === "Geçmişi ve durumu yenile").props.onClick();
  await app.flush();
  const request = app.take("inspect_repository");
  if (response instanceof Error) request.reject(response);
  else request.resolve(response);
  await app.flush();
}

async function selectCommit(app, oid, files) {
  const history = app.find((node) => node.type === "HistoryList");
  history.props.onSelect(history.props.commits.find((entry) => entry.oid === oid));
  await app.flush();
  app.take("get_commit_files").resolve(files);
  await app.flush();
}

async function selectFile(app, path) {
  app.find((node) => node.type?.name === "FileRow" && node.props.file.path === path).props.onSelect();
  await app.flush();
}

test("first run waits for an explicit repository path and remembers only its accepted canonical path", async (t) => {
  const localStorage = memoryStorage();
  const app = repositoryAppHarness({ localStorage });
  t.after(() => app.unmount());
  await app.flush();
  assert.equal(app.requests.length, 0);
  const input = app.find((node) => node.props["aria-label"] === "Depo yolu");
  assert.equal(input.props.value, "");
  assert.equal(input.props.disabled, false);
  assert.match(textContent(app.find(byClass("history-empty"))), /bir depo açın/);
  assert.equal(composer(app).props.disabled, true);

  await open(app, "C:/work/repo-a", new Error("Repository missing"));
  assert.equal(localStorage.getItem(lastRepositoryStorageKey), null);
  await open(app, "C:/work/repo-a", snapshot());
  assert.equal(localStorage.getItem(lastRepositoryStorageKey), repoA);
  await open(app, "C:/missing", new Error("Repository missing"));
  assert.equal(localStorage.getItem(lastRepositoryStorageKey), repoA);

  const restarted = repositoryAppHarness({ localStorage });
  t.after(() => restarted.unmount());
  await restarted.flush();
  const request = restarted.take("inspect_repository");
  assert.equal(request.args.path, String.raw`C:\work\repo-a`);
  request.resolve(snapshot());
  await restarted.flush();
  assert.equal(restarted.find((node) => node.props["aria-label"] === "Depo yolu").props.value, String.raw`C:\work\repo-a`);
});

test("a failed saved repository keeps retry and another repository available", async (t) => {
  const localStorage = memoryStorage(repoA);
  const app = repositoryAppHarness({ localStorage });
  t.after(() => app.unmount());
  await app.flush();
  app.take("inspect_repository").reject("Saved repository moved");
  await app.flush();
  const input = app.find((node) => node.props["aria-label"] === "Depo yolu");
  assert.equal(input.props.value, String.raw`C:\work\repo-a`);
  assert.equal(input.props.disabled, false);
  assert.equal(composer(app).props.disabled, true);
  const error = app.find((node) => node.type?.name === "InlineError");
  assert.equal(error.props.message, "Saved repository moved");
  error.props.onRetry();
  await app.flush();
  const retry = app.take("inspect_repository");
  assert.equal(retry.args.path, String.raw`C:\work\repo-a`);
  retry.reject("Still missing");
  await app.flush();
  assert.equal(localStorage.getItem(lastRepositoryStorageKey), repoA);
  await open(app, "C:/work/repo-b", snapshot(repoB));
  assert.equal(localStorage.getItem(lastRepositoryStorageKey), repoB);
  assert.equal(composer(app).props.disabled, false);
});

test("invalid saved values do not invoke Git or turn first run into an error", async (t) => {
  for (const savedPath of ["", "   ", "{}", "null", "repo-a", "C:repo-a", String.raw`\repo-a`, "//server", String.raw`\\.\pipe\repo`, "C:\\work\\repo\n", "C:\\work\\repo\0"]) {
    const app = repositoryAppHarness({ localStorage: memoryStorage(savedPath) });
    t.after(() => app.unmount());
    await app.flush();
    assert.equal(app.requests.length, 0, savedPath);
    assert.equal(app.find((node) => node.props["aria-label"] === "Depo yolu").props.value, "", savedPath);
    assert.equal(app.all((node) => node.type?.name === "InlineError").length, 0, savedPath);
  }
});

test("saved absolute paths support Windows drives, UNC shares, and Unix roots", async (t) => {
  for (const [savedPath, expectedPath] of [
    ["C:/work/repo-a", "C:/work/repo-a"],
    [String.raw`\\?\UNC\server\share\repo-a`, String.raw`\\server\share\repo-a`],
    ["/work/repo-a", "/work/repo-a"],
  ]) {
    const app = repositoryAppHarness({ localStorage: memoryStorage(savedPath) });
    t.after(() => app.unmount());
    await app.flush();
    assert.equal(app.take("inspect_repository").args.path, expectedPath);
  }
});

test("unavailable storage cannot prevent opening a repository", async (t) => {
  const app = repositoryAppHarness({ localStorage: {
    getItem() { throw new Error("Storage access denied"); },
    setItem() { throw new Error("Storage quota exceeded"); },
  } });
  t.after(() => app.unmount());
  await app.flush();
  assert.equal(app.requests.length, 0);
  await open(app, "C:/work/repo-a", snapshot());
  assert.equal(composer(app).props.disabled, false);
  assert.equal(app.all((node) => node.type?.name === "InlineError").length, 0);
});

test("an obsolete startup response cannot replace the saved repository", async () => {
  const localStorage = memoryStorage(repoA);
  const app = repositoryAppHarness({ localStorage });
  await app.flush();
  const request = app.take("inspect_repository");
  app.unmount();
  request.resolve(snapshot(repoB));
  await app.flush();
  assert.equal(localStorage.getItem(lastRepositoryStorageKey), repoA);
});

test("session drafts survive A to B, failed open, canonical A return, and clear only the committed worktree", async (t) => {
  const app = await mount(t);
  await editDraft(app, "draft A");
  await open(app, "C:/work/repo-b", snapshot(repoB));
  assert.equal(composer(app).props.value, "");
  await editDraft(app, "draft B");
  await open(app, "C:/missing", new Error("Repository missing"));
  assert.equal(composer(app).props.value, "draft B");
  assert.equal(composer(app).props.disabled, true);
  await open(app, "C:/WORK/REPO-A/", snapshot("C:/work/repo-a/"));
  assert.equal(composer(app).props.value, "draft A");
  await open(app, "C:/work/repo-b", snapshot(repoB));
  assert.equal(composer(app).props.value, "draft B");
  app.find(byClass("commit-composer")).props.onSubmit({ preventDefault });
  await app.flush();
  const request = app.take("commit_index");
  assert.equal(request.args.repoPath, repoB);
  assert.equal(request.args.message, "draft B");
  request.resolve({ snapshot: snapshot(repoB), commitOid: oidB, commitTreeChanged: false });
  await app.flush();
  assert.equal(composer(app).props.value, "");
  await open(app, "C:/work/repo-a", snapshot(repoA));
  assert.equal(composer(app).props.value, "draft A");
});

test("failed refresh and failed commit recovery retain the target draft", async (t) => {
  const app = await mount(t);
  await editDraft(app, "retry after inspecting Git state");
  await refresh(app, new Error("Refresh failed"));
  assert.equal(composer(app).props.value, "retry after inspecting Git state");
  await open(app, "C:/work/repo-a", snapshot(repoA));
  app.find(byClass("commit-composer")).props.onSubmit({ preventDefault });
  await app.flush();
  app.take("commit_index").reject(new Error("Hook failed"));
  await app.flush();
  const recovery = app.take("inspect_repository");
  assert.equal(recovery.args.path, repoA);
  recovery.reject(new Error("Recovery failed"));
  await app.flush();
  assert.equal(composer(app).props.value, "retry after inspecting Git state");
  await open(app, "C:/work/repo-b", snapshot(repoB));
  await editDraft(app, "B stays separate");
  await open(app, "C:/work/repo-a", snapshot(repoA));
  assert.equal(composer(app).props.value, "retry after inspecting Git state");
  assert.equal(app.requests.filter((request) => request.command === "commit_index").length, 1);
});

test("stale composer handlers and in-flight writes cannot edit or commit another repository's draft", async (t) => {
  const app = await mount(t);
  await editDraft(app, "draft A");
  const oldChange = composer(app).props.onChange;
  const oldSubmit = app.find(byClass("commit-composer")).props.onSubmit;
  const oldOpen = app.find(byClass("repo-open-form")).props.onSubmit;
  oldSubmit({ preventDefault });
  oldChange({ target: { value: "during commit" } });
  oldOpen({ preventDefault });
  oldSubmit({ preventDefault });
  await app.flush();
  assert.equal(app.requests.filter((request) => request.command === "commit_index").length, 1);
  assert.equal(app.requests.filter((request) => request.command === "inspect_repository").length, 1);
  app.take("commit_index").resolve({ snapshot: snapshot(), commitOid: oidB, commitTreeChanged: false });
  await app.flush();
  await open(app, "C:/work/repo-b", snapshot(repoB));
  await editDraft(app, "draft B");
  oldChange({ target: { value: "stale A event" } });
  oldSubmit({ preventDefault });
  await app.flush();
  assert.equal(composer(app).props.value, "draft B");
  assert.equal(app.requests.filter((request) => request.command === "commit_index").length, 1);
  await open(app, "C:/work/repo-a", snapshot());
  assert.equal(composer(app).props.value, "");
});

for (const [label, staged] of [["Çalışma ağacı farkını göster", false], ["Stage edilen farkı göster", true]]) {
  test(`re-clicking active ${staged ? "S" : "W"} accepts the pending diff response`, async (t) => {
    const app = await mount(t);
    let pending = app.take("prepare_partial_diff");
    if (staged) {
      app.find((node) => node.props["aria-label"] === label).props.onClick();
      await app.flush();
      pending = app.take("prepare_partial_diff");
    }
    assert.equal(pending.args.staged, staged);
    assert.equal(pending.args.expectedToken, "token");
    assert.equal(diffView(app).props.state, "loading");
    const count = app.requests.length;
    app.find((node) => node.props["aria-label"] === label).props.onClick();
    await app.flush();
    assert.equal(app.requests.length, count);
    pending.resolve({ text: "the requested diff", binary: false, truncated: false });
    await app.flush();
    assert.equal(diffView(app).props.state, "ready");
    assert.equal(diffView(app).props.result.text, "the requested diff");
  });
}

for (const staged of [false, true]) {
  test(`partial ${staged ? "unstage" : "stage"} sends only the current preview IDs once and preserves the commit draft`, async (t) => {
    const app = await mount(t);
    await resolvePartialDiff(app);
    if (staged) {
      app.find((node) => node.props["aria-label"] === "Stage edilen farkı göster").props.onClick();
      await app.flush();
      await resolvePartialDiff(app);
    }
    await editDraft(app, "keep this draft");
    const controls = diffView(app).props.partialSelection;
    assert.equal(controls.staged, staged);
    assert.equal(controls.busy, false);
    const completion = controls.onApply({ selectionToken: "selection-v1", lineIds: [7, 4, 7] });
    void controls.onApply({ selectionToken: "selection-v1", lineIds: [5] });
    await app.flush();
    const request = app.take("apply_partial_selection");
    assert.deepEqual(JSON.parse(JSON.stringify(request.args)), {
      repoPath: repoA, filePath: "working.txt", staged, expectedToken: "token",
      selectionToken: "selection-v1", lineIds: [4, 7],
    });
    assert.equal(app.requests.filter((entry) => entry.command === "apply_partial_selection").length, 1);
    assert.equal(composer(app).props.disabled, true);
    request.resolve({ snapshot: snapshot(repoA, { mutationToken: "updated-index" }), commitOid: null, commitTreeChanged: false });
    await completion;
    await app.flush();
    assert.equal(composer(app).props.value, "keep this draft");
    assert.match(textContent(app.find(byClass("mutation-feedback"))), /2 değişiklik satırı/);
    await controls.onApply({ selectionToken: "selection-v1", lineIds: [5] });
    assert.equal(app.requests.filter((entry) => entry.command === "apply_partial_selection").length, 1);
    assert.equal(app.take("prepare_partial_diff").args.expectedToken, "updated-index");
  });
}

test("partial selection rejects invalid rows, stale tokens, and unsupported previews", async (t) => {
  for (const overrides of [{}, { binary: true }, { truncated: true }, { selectionToken: null }, { unsupportedReason: "Dosyanın tamamını kullanın." }]) {
    const app = await mount(t);
    const controls = await resolvePartialDiff(app, partialDiff(overrides));
    for (const selection of [
      { selectionToken: "selection-v1", lineIds: [] },
      { selectionToken: "selection-v1", lineIds: [3] },
      { selectionToken: "selection-v1", lineIds: [4, -1] },
      { selectionToken: "selection-v1", lineIds: [4.5] },
      { selectionToken: "old-preview", lineIds: [4] },
    ]) await controls.onApply(selection);
    if (Object.keys(overrides).length) await controls.onApply({ selectionToken: "selection-v1", lineIds: [4] });
    assert.equal(app.requests.filter((entry) => entry.command === "apply_partial_selection").length, 0);
  }
});

test("partial selection callbacks cannot follow a changed file, layer, repository, history, or refreshed preview", async (t) => {
  for (const target of ["file", "layer", "repository", "history", "refresh"]) {
    const initial = snapshot(repoA, { changes: [file("working.txt"), file("other.txt")] });
    const app = await mount(t, initial);
    const controls = await resolvePartialDiff(app);
    if (target === "file") await selectFile(app, "other.txt");
    if (target === "layer") {
      app.find((node) => node.props["aria-label"] === "Stage edilen farkı göster").props.onClick();
      await app.flush();
    }
    if (target === "repository") await open(app, repoB, snapshot(repoB));
    if (target === "history") {
      await selectCommit(app, oidA, [file("working.txt")]);
      app.take("read_diff").resolve(partialDiff());
      await app.flush();
      assert.equal(diffView(app).props.partialSelection, undefined);
    }
    if (target === "refresh") {
      await refresh(app, snapshot(repoA));
      await resolvePartialDiff(app, partialDiff({ selectionToken: "selection-v2" }));
    }
    await controls.onApply({ selectionToken: "selection-v1", lineIds: [4] });
    assert.equal(app.requests.filter((entry) => entry.command === "apply_partial_selection").length, 0, target);
  }
});

test("a failed partial write refreshes its repository without retry and retains the draft", async (t) => {
  const app = await mount(t);
  const controls = await resolvePartialDiff(app);
  await editDraft(app, "inspect before retrying");
  const completion = controls.onApply({ selectionToken: "selection-v1", lineIds: [4] });
  await app.flush();
  app.take("apply_partial_selection").reject("Dosya seçimden sonra değişti.");
  await app.flush();
  const recovery = app.take("inspect_repository");
  assert.equal(recovery.args.path, repoA);
  assert.equal(composer(app).props.value, "inspect before retrying");
  recovery.resolve(snapshot(repoA, { mutationToken: "fresh-token" }));
  await completion;
  await app.flush();
  assert.match(textContent(app.find(byClass("mutation-feedback"))), /seçimden sonra değişti/);
  assert.equal(composer(app).props.value, "inspect before retrying");
  assert.equal(app.requests.filter((entry) => entry.command === "apply_partial_selection").length, 1);
  assert.equal(app.take("prepare_partial_diff").args.expectedToken, "fresh-token");
});

test("history refresh retains a valid selected path for canonical repo and commit, then resets missing or changed targets", async (t) => {
  const app = await mount(t);
  const files = [file("a.txt"), file("b.txt")];
  await selectCommit(app, oidA, files);
  await selectFile(app, "b.txt");
  assert.equal(selectedFile(app), "b.txt");
  await refresh(app, snapshot("C:/WORK/repo-a/"));
  const request = app.take("get_commit_files");
  assert.equal(request.args.commitOid, oidA);
  request.resolve(files);
  await app.flush();
  assert.equal(selectedFile(app), "b.txt");
  const latestDiff = app.requests.filter((entry) => entry.command === "read_diff").at(-1);
  assert.equal(latestDiff.args.filePath, "b.txt");
  assert.equal(latestDiff.args.commitOid, oidA);
  await refresh(app, snapshot(repoA));
  app.take("get_commit_files").resolve([files[0]]);
  await app.flush();
  assert.equal(selectedFile(app), "a.txt");
  await refresh(app, snapshot(repoA));
  app.take("get_commit_files").resolve(files);
  await app.flush();
  await selectFile(app, "b.txt");
  await selectCommit(app, oidB, files);
  assert.equal(selectedFile(app), "a.txt");
  await selectFile(app, "b.txt");
  await open(app, "C:/work/repo-b", snapshot(repoB));
  await selectCommit(app, oidB, files);
  assert.equal(selectedFile(app), "a.txt");
});

test("obsolete history and diff responses cannot overwrite another repository with the same OID", async (t) => {
  const app = await mount(t);
  const files = [file("a.txt"), file("b.txt")];
  const oldDiff = app.take("prepare_partial_diff");
  await selectCommit(app, oidA, files);
  await selectFile(app, "b.txt");
  await refresh(app, snapshot(repoA));
  const oldFiles = app.take("get_commit_files");
  await open(app, "C:/work/repo-b", snapshot(repoB));
  await selectCommit(app, oidA, [file("c.txt"), file("d.txt")]);
  await selectFile(app, "d.txt");
  const currentDiff = app.requests.filter((entry) => entry.command === "read_diff").at(-1);
  currentDiff.resolve({ text: "B diff", binary: false, truncated: false });
  await app.flush();
  oldFiles.resolve(files);
  oldDiff.resolve({ text: "stale A diff", binary: false, truncated: false });
  await app.flush();
  assert.equal(selectedFile(app), "d.txt");
  assert.equal(fileRows(app).map((node) => node.props.file.path).join(","), "c.txt,d.txt");
  assert.equal(diffView(app).props.result.text, "B diff");
});

test("current worktree marker recognizes canonical Windows path aliases", async (t) => {
  const app = await mount(t, snapshot(repoA, {
    worktrees: [
      { path: "C:/work/repo-a/", branch: "main", head: oidA, isBare: false, isDetached: false },
      { path: "C:/work/repo-b", branch: "other", head: oidB, isBare: false, isDetached: false },
    ],
  }));
  const current = app.find((node) => byClass("worktree-item")(node) && byClass("current")(node));
  assert.equal(current.props.title, "C:/work/repo-a/");
  assert.equal(app.all((node) => node.props["aria-label"] === "Açık depo").length, 1);
});

test("a successful commit with changed staged contents warns with the real OID and never retries", async (t) => {
  const app = await mount(t);
  await editDraft(app, "hook changes the staged tree");
  app.find(byClass("commit-composer")).props.onSubmit({ preventDefault });
  await app.flush();
  app.take("commit_index").resolve({ snapshot: snapshot(), commitOid: oidB, commitTreeChanged: true });
  await app.flush();
  const feedback = app.find(byClass("mutation-feedback"));
  assert.ok(byClass("warning")(feedback));
  assert.equal(feedback.props.role, "alert");
  assert.match(textContent(feedback), /commit sırasında.*stage.*değişti/i);
  assert.match(textContent(feedback), /kontrol edin/i);
  assert.equal(app.find((node) => node.type === "code").props.title, oidB);
  assert.equal(composer(app).props.value, "");
  assert.equal(app.requests.filter((request) => request.command === "commit_index").length, 1);
});

for (const [name, lineEnding] of [["LF", "\n"], ["CRLF", "\r\n"]]) {
  test(`diff footer excludes the terminal empty ${name} element from its displayed row count`, async (t) => {
    const app = await mount(t);
    const text = [
      "diff --git a/working.txt b/working.txt",
      "index 3367afd..3e75765 100644",
      "--- a/working.txt",
      "+++ b/working.txt",
      "@@ -1 +1 @@",
      "-old",
      "+new",
      "",
    ].join(lineEnding);
    app.take("prepare_partial_diff").resolve({ text, binary: false, truncated: false });
    await app.flush();
    assert.equal(textContent(app.find(byClass("diff-line-total"))), "7 satır");
  });
}

test("file filters scope bulk stage to visible paths and invalidate hidden diff responses", async (t) => {
  const initial = snapshot(repoA, { changes: [file("src/a.ts", " ", "M"), file("docs/b.md", " ", "M")] });
  const app = await mount(t, initial);
  const oldDiff = app.take("prepare_partial_diff");
  app.find((node) => node.props["aria-label"] === "Dosya ara").props.onChange({ target: { value: "docs/" } });
  await app.flush();
  assert.equal(fileRows(app).map((row) => row.props.file.path).join(","), "docs/b.md");
  assert.equal(selectedFile(app), "docs/b.md");
  const currentDiff = app.requests.filter((request) => request.command === "prepare_partial_diff").at(-1);
  currentDiff.resolve({ text: "docs diff", binary: false, truncated: false });
  oldDiff.resolve({ text: "obsolete src diff", binary: false, truncated: false });
  await app.flush();
  assert.equal(diffView(app).props.result.text, "docs diff");
  const stage = app.find((node) => node.type === "button" && textContent(node).startsWith("Görünenleri stage et"));
  stage.props.onClick();
  await app.flush();
  const request = app.take("stage_paths");
  assert.equal(Array.from(request.args.paths).join(","), "docs/b.md");
  assert.equal(app.find((node) => node.props["aria-label"] === "Dosya ara").props.disabled, true);
  request.resolve({ snapshot: snapshot(repoA, { changes: [initial.changes[0], file("docs/b.md", "M", " ")] }), commitOid: null, commitTreeChanged: false });
  await app.flush();
  app.find((node) => node.props["aria-label"] === "Dosya durumu").props.onChange({ target: { value: "working" } });
  await app.flush();
  assert.equal(fileRows(app).length, 0);
  assert.equal(app.all((node) => node.type === "DiffView").length, 0);
  assert.match(textContent(app.find(byClass("panel-message"))), /Filtreye uyan dosya yok/);
  assert.match(textContent(app.find((node) => byClass("file-empty")(node))), /filtreyle eşleşen dosya yok/);
});

test("filtered commit still commits the whole prepared index and hidden conflicts block it", async (t) => {
  const changes = [file("hidden.txt", "M", " "), file("visible.txt", " ", "M")];
  const app = await mount(t, snapshot(repoA, { changes }));
  app.find((node) => node.props["aria-label"] === "Dosya ara").props.onChange({ target: { value: "visible" } });
  await editDraft(app, "entire prepared index");
  assert.equal(app.find(byClass("commit-button")).props.disabled, false);
  assert.match(textContent(app.find((node) => node.props.id === "commit-scope-hint")), /Filtre dışındakiler dahil/);
  app.find(byClass("commit-composer")).props.onSubmit({ preventDefault });
  await app.flush();
  const request = app.take("commit_index");
  assert.equal(request.args.message, "entire prepared index");
  assert.equal("paths" in request.args, false);
  request.resolve({ snapshot: snapshot(repoA, { changes: [changes[1]] }), commitOid: oidB, commitTreeChanged: false });
  await app.flush();
  await refresh(app, snapshot(repoA, { changes: [...changes, file("hidden-conflict.txt", "U", "U")] }));
  await editDraft(app, "blocked by conflict");
  assert.equal(app.find(byClass("commit-button")).props.disabled, true);
  app.find(byClass("commit-composer")).props.onSubmit({ preventDefault });
  await app.flush();
  assert.equal(app.requests.filter((entry) => entry.command === "commit_index").length, 1);
});

test("all-history search selects an older commit outside the graph and preserves it across refresh", async (t) => {
  const initial = snapshot();
  const oldCommit = { ...commit("c".repeat(40)), subject: "needle feature", parents: ["d".repeat(40)] };
  const app = await mount(t, initial);
  app.find((node) => node.props["aria-label"] === "Commit ara").props.onChange({ target: { value: "needle" } });
  await app.flush();
  const history = app.find((node) => node.type === "HistoryList");
  assert.equal(history.props.commits.length, 2);
  app.find((node) => node.props["aria-label"] === "Tüm geçmişte ara").props.onClick();
  await app.flush();
  const search = app.take("search_repository_history");
  assert.equal(search.args.query, "needle");
  assert.equal(search.args.cursor, null);
  search.resolve({ commits: [oldCommit], scannedCount: 250, nextCursor: null, complete: true });
  await app.flush();
  assert.equal(app.all((node) => node.type === "HistoryList").length, 0, "Sparse matches must not be drawn as a graph");
  app.find((node) => node.props["aria-label"] === "Sonraki commit eşleşmesi").props.onClick();
  await app.flush();
  const request = app.take("get_commit_files");
  assert.equal(request.args.commitOid, oldCommit.oid);
  request.resolve([file("commit-file.txt")]);
  await app.flush();
  assert.equal(textContent(app.find(byClass("commit-file-oid"))), oldCommit.oid.slice(0, 8));
  assert.match(textContent(app.find(byClass("diff-source"))), /İLK EBEVEYN/);
  await refresh(app, snapshot());
  const refreshedFiles = app.take("get_commit_files");
  assert.equal(refreshedFiles.args.commitOid, oldCommit.oid);
  refreshedFiles.resolve([file("commit-file.txt")]);
  await app.flush();
  app.find((node) => node.props["aria-label"] === "Dosya ara").props.onChange({ target: { value: "missing-file" } });
  await app.flush();
  assert.match(textContent(app.find(byClass("panel-message"))), /Filtreye uyan dosya yok/);
  assert.equal(app.find((node) => node.type === "HistoryList").props.selectedOid, oldCommit.oid);
  assert.equal(app.find((node) => node.type === "HistoryList").props.commits.length, 2);
});

async function beginHistorySearch(app, query) {
  app.find((node) => node.props["aria-label"] === "Commit ara").props.onChange({ target: { value: query } });
  await app.flush();
  app.find((node) => node.props["aria-label"] === "Tüm geçmişte ara").props.onClick();
  await app.flush();
  return app.take("search_repository_history");
}

test("history search continues empty batches, exposes paging, and revisits the previous page", async (t) => {
  const app = await mount(t);
  const first = await beginHistorySearch(app, "Test");
  const cursor = { historyToken: "token", query: "test", offset: 500 };
  first.resolve({ commits: [], scannedCount: 500, nextCursor: cursor, complete: false });
  await app.flush();
  const second = app.take("search_repository_history");
  assert.equal(second.args.cursor.offset, 500);
  const many = Array.from({ length: 100 }, (_, index) => commit(index.toString(16).padStart(40, "0")));
  second.resolve({ commits: many, scannedCount: 600, nextCursor: { ...cursor, offset: 600 }, complete: false });
  await app.flush();
  assert.equal(app.all(byClass("history-search-result")).length, 100);
  assert.match(textContent(app.find(byClass("history-search-progress"))), /Diğer sonuçlar var/);
  app.find((node) => node.props["aria-label"] === "Sonraki sonuç sayfası").props.onClick();
  await app.flush();
  const next = app.take("search_repository_history");
  assert.equal(next.args.cursor.offset, 600);
  next.resolve({ commits: [commit("f".repeat(40))], scannedCount: 601, nextCursor: null, complete: true });
  await app.flush();
  assert.equal(app.all(byClass("history-search-result")).length, 1);
  app.find((node) => node.props["aria-label"] === "Önceki sonuç sayfası").props.onClick();
  await app.flush();
  assert.equal(app.take("search_repository_history").args.cursor, null);
});

test("history search stop, query changes and repo changes reject delayed responses", async (t) => {
  const app = await mount(t);
  const stopped = await beginHistorySearch(app, "first");
  app.find((node) => node.type === "button" && textContent(node) === "Durdur").props.onClick();
  await app.flush();
  stopped.resolve({ commits: [commit("c".repeat(40))], scannedCount: 20, nextCursor: null, complete: true });
  await app.flush();
  assert.equal(app.all(byClass("history-search-result")).length, 0);
  app.find((node) => node.props["aria-label"] === "Aramayı sürdür").props.onClick();
  await app.flush();
  const obsolete = app.take("search_repository_history");
  const current = await beginHistorySearch(app, "second");
  current.resolve({ commits: [commit("d".repeat(40))], scannedCount: 20, nextCursor: null, complete: true });
  obsolete.resolve({ commits: [commit("e".repeat(40))], scannedCount: 20, nextCursor: null, complete: true });
  await app.flush();
  assert.equal(app.all(byClass("history-search-result")).length, 1);
  assert.match(textContent(app.find(byClass("history-search-result"))), /^dddd/);
  const oldRepo = await beginHistorySearch(app, "third");
  await open(app, repoB, snapshot(repoB));
  oldRepo.resolve({ commits: [commit("c".repeat(40))], scannedCount: 20, nextCursor: null, complete: true });
  await app.flush();
  assert.equal(app.all(byClass("history-search-result")).length, 0);
  assert.equal(app.find((node) => node.props["aria-label"] === "Commit ara").props.value, "");
});

test("history failures stay explicit without automatic retries and writes invalidate pending searches", async (t) => {
  const app = await mount(t);
  const request = await beginHistorySearch(app, "needle");
  request.reject("History changed; restart search");
  await app.flush();
  assert.match(textContent(app.find((node) => node.props.role === "alert" && byClass("history-search-message")(node))), /History changed/);
  assert.equal(app.requests.filter((entry) => entry.command === "search_repository_history").length, 1);
  const pending = await beginHistorySearch(app, "Test");
  app.find((node) => node.type === "button" && textContent(node).startsWith("Tümünü stage et")).props.onClick();
  await app.flush();
  const stage = app.take("stage_paths");
  pending.resolve({ commits: [commit("f".repeat(40))], scannedCount: 3, nextCursor: null, complete: true });
  stage.resolve({ snapshot: snapshot(), commitOid: null, commitTreeChanged: false });
  await app.flush();
  assert.equal(app.all(byClass("history-search-result")).length, 0);
});

test("diff navigation identity distinguishes file, layer, commit and repository", async (t) => {
  const app = await mount(t);
  const workingKey = diffView(app).props.targetKey;
  app.find((node) => node.props["aria-label"] === "Stage edilen farkı göster").props.onClick();
  await app.flush();
  assert.notEqual(diffView(app).props.targetKey, workingKey);
  await selectCommit(app, oidB, [file("working.txt")]);
  assert.notEqual(diffView(app).props.targetKey, workingKey);
  assert.ok(diffView(app).props.targetKey.includes(oidB));
  const historyKey = diffView(app).props.targetKey;
  await open(app, repoB, snapshot(repoB));
  assert.notEqual(diffView(app).props.targetKey, historyKey);
});

test("workspace density and collapse persist, reset independently, and collapsed rail opens worktrees", async (t) => {
  const storage = memoryStorage();
  const app = repositoryAppHarness({ localStorage: storage });
  t.after(() => app.unmount());
  await app.flush();
  const actions = () => app.find((node) => node.type === "RepositoryActions");
  actions().props.onDensityChange("comfortable");
  app.find((node) => node.props["aria-label"] === "Kenar çubuğunu daralt").props.onClick();
  await app.flush();
  assert.equal(JSON.parse(storage.getItem(workspacePreferencesKey)).density, "comfortable");
  assert.equal(JSON.parse(storage.getItem(workspacePreferencesKey)).sidebarCollapsed, true);
  assert.equal(typeof app.find((node) => node.props["aria-label"] === "Depo yoluna git").props.onClick, "function");
  app.find((node) => node.props["aria-label"] === "Çalışma ağaçlarını göster").props.onClick();
  await app.flush();
  assert.equal(app.find(byClass("sidebar")).props.className.includes("collapsed"), false);
  const restarted = repositoryAppHarness({ localStorage: storage });
  t.after(() => restarted.unmount());
  await restarted.flush();
  assert.equal(restarted.find(byClass("app-shell")).props["data-density"], "comfortable");
  actions().props.onResetLayout();
  await app.flush();
  assert.equal(app.find(byClass("app-shell")).props["data-density"], "compact");
});

test("worktree navigation excludes bare/current targets and preserves per-worktree drafts", async (t) => {
  const worktrees = [
    { path: repoA, branch: "main", head: oidA, isBare: false, isDetached: false },
    { path: repoB, branch: "other", head: oidB, isBare: false, isDetached: false },
    { path: "C:/work/bare.git", branch: "", head: oidA, isBare: true, isDetached: false },
  ];
  const app = await mount(t, snapshot(repoA, { worktrees }));
  await editDraft(app, "draft A");
  const items = () => app.all(byClass("worktree-item"));
  assert.equal(items()[0].props.disabled, true);
  assert.equal(items()[2].props.disabled, true);
  items()[1].props.onClick();
  await app.flush();
  const openB = app.take("inspect_repository");
  assert.equal(openB.args.path, repoB);
  openB.resolve(snapshot(repoB, { worktrees }));
  await app.flush();
  await editDraft(app, "draft B");
  items()[0].props.onClick();
  await app.flush();
  app.take("inspect_repository").resolve(snapshot(repoA, { worktrees }));
  await app.flush();
  assert.equal(composer(app).props.value, "draft A");
});
