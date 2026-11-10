import assert from "node:assert/strict";
import test from "node:test";
import { repositoryActionsHarness, textContent } from "../scripts/repositoryActionsHarness.mjs";

const repoA = String.raw`\\?\C:\work\repo-a`;
const repoB = String.raw`\\?\C:\work\repo-b`;
const remote = (name, host = "a") => ({
  name, fetchUrl: `https://${host}.example/${name}.git`, pushUrl: `https://${host}.example/${name}.git`,
  configToken: `${name}-config`, configuredFetchUrl: `https://${host}.example/${name}.git`,
  configuredPushUrl: null, hasExplicitPushUrl: false, editBlockedReason: null,
});
const snapshot = (repoPath = repoA, overrides = {}) => ({
  repoPath, repoName: "fixture", mutationToken: "repository-token", branch: "main", head: "a".repeat(40),
  gitVersion: "fixture", isUnborn: false, mergeInProgress: false, changes: [], commits: [], worktrees: [], branches: [],
  remotes: [remote("origin"), remote("upstream")], ...overrides,
});
const byClass = (name) => (node) => node.props.className?.split(" ").includes(name);
const dialog = (app) => app.find(byClass("remote-manager-dialog"));
const field = (app, labelText) => {
  const label = app.find((node) => node.type === "label" && textContent(node).startsWith(labelText), dialog(app));
  return app.find((node) => node.type === "input" && node.props.id === label.props.htmlFor, dialog(app));
};
const saveButton = (app) => app.find((node) => node.type === "button" && node.props.type === "submit", dialog(app));
const submit = (app) => app.find((node) => node.type === "form", dialog(app)).props.onSubmit({ preventDefault() {} });

async function mount(t) {
  const requests = [];
  const app = repositoryActionsHarness({
    snapshot: snapshot(), targetsLocked: false, canMutate: true, refreshing: false, activeAction: null, feedback: null,
    onExecute: (action) => new Promise((resolve) => { requests.push({ action, resolve }); }),
    onDismissFeedback() {}, onRefresh() {},
  });
  t.after(() => app.unmount());
  await app.flush();
  return { app, requests };
}

async function open(app) {
  app.find((node) => node.props["aria-label"] === "Uzak depolar").props.onClick();
  await app.flush();
  assert.equal(dialog(app).props.ref.current.open, true, "The retained dialog must actually reopen");
}

async function edit(app, value) {
  field(app, "Fetch adresi").props.onChange({ target: { value } });
  await app.flush();
}

async function select(app, name) {
  app.find((node) => byClass("remote-manager-row")(node) && node.props.title === name, dialog(app)).props.onClick();
  await app.flush();
}

async function failSaveAndRecovery(app, requests) {
  submit(app);
  await app.flush();
  assert.equal(requests.length, 1);
  app.setProps({ snapshot: null, canMutate: false, targetsLocked: false });
  await app.flush();
  assert.equal(dialog(app).props.ref.current.open, false, "Unavailable repositories close the modal");
  requests[0].resolve({ kind: "error", message: "Save failed; recovery inspection failed", repositoryAction: requests[0].action });
  await app.flush();
}

test("failed save and failed recovery retain every remote draft and reopen the native dialog", async (t) => {
  const { app, requests } = await mount(t);
  await open(app);
  await edit(app, "https://typed.example/origin.git");
  await select(app, "upstream");
  await edit(app, "https://typed.example/upstream.git");
  await select(app, "origin");
  await failSaveAndRecovery(app, requests);
  app.setProps({ snapshot: snapshot(), canMutate: true });
  await app.flush();
  assert.equal(requests.length, 1, "Recovery must not retry the write");
  await open(app);
  assert.equal(field(app, "Fetch adresi").props.value, "https://typed.example/origin.git");
  assert.equal(saveButton(app).props.disabled, false);
  await select(app, "upstream");
  assert.equal(field(app, "Fetch adresi").props.value, "https://typed.example/upstream.git");
});

test("canonical aliases retain draft values and submit only to the restored snapshot path", async (t) => {
  const { app, requests } = await mount(t);
  await open(app);
  await edit(app, "https://typed.example/origin.git");
  await failSaveAndRecovery(app, requests);
  const alias = "C:/WORK/repo-a/";
  app.setProps({ snapshot: snapshot(alias), canMutate: true });
  await app.flush();
  await open(app);
  assert.equal(field(app, "Fetch adresi").props.value, "https://typed.example/origin.git");
  assert.equal(saveButton(app).props.disabled, false);
  submit(app);
  await app.flush();
  assert.equal(requests.length, 2);
  assert.equal(requests[1].action.repoPath, alias);
  assert.equal(requests[1].action.expectedToken, "repository-token");
  assert.equal(requests[1].action.expectedRemoteToken, "origin-config");
});

for (const drift of ["repository", "remote"]) {
  test(`restored ${drift} drift keeps the draft but requires explicit reload before saving`, async (t) => {
    const { app, requests } = await mount(t);
    await open(app);
    await edit(app, "https://typed.example/origin.git");
    await failSaveAndRecovery(app, requests);
    const restored = drift === "repository"
      ? snapshot(repoA, { mutationToken: "new-repository-token" })
      : snapshot(repoA, { remotes: [{ ...remote("origin"), configToken: "new-origin-config" }, remote("upstream")] });
    app.setProps({ snapshot: restored, canMutate: true });
    await app.flush();
    await open(app);
    assert.equal(field(app, "Fetch adresi").props.value, "https://typed.example/origin.git");
    assert.equal(saveButton(app).props.disabled, true);
    assert.match(textContent(dialog(app)), /güncel ayarı yükleyin/);
    submit(app);
    await app.flush();
    assert.equal(requests.length, 1, "An old draft token must never be silently refreshed or submitted");
  });
}

test("a different repository with identical remote names and tokens never inherits the old draft", async (t) => {
  const { app, requests } = await mount(t);
  await open(app);
  await edit(app, "https://typed-a.example/origin.git");
  await failSaveAndRecovery(app, requests);
  app.setProps({ snapshot: snapshot(repoB, { remotes: [remote("origin", "b")] }), canMutate: true });
  await app.flush();
  await open(app);
  assert.equal(field(app, "Fetch adresi").props.value, "https://b.example/origin.git");
  assert.equal(saveButton(app).props.disabled, true);
  await edit(app, "https://typed-b.example/origin.git");
  submit(app);
  await app.flush();
  assert.equal(requests.length, 2);
  assert.equal(requests[1].action.repoPath, repoB);
  assert.equal(requests[1].action.fetchUrl, "https://typed-b.example/origin.git");
});

test("language switching updates a retained remote form and validation without changing its draft", async (t) => {
  const { app, requests } = await mount(t);
  await open(app);
  const surface = dialog(app).props.ref.current;
  const value = " https://host/ekip/çalışma.git";
  await edit(app, value);
  submit(app);
  await app.flush();
  assert.equal(requests.length, 0);
  assert.match(textContent(dialog(app)), /Adresin başında veya sonunda/);
  app.setLocale("en");
  await app.flush();
  assert.equal(dialog(app).props.ref.current, surface);
  assert.equal(surface.open, true);
  assert.equal(surface.showCount, 1, "Changing language must not reopen the modal");
  assert.equal(field(app, "Fetch address").props.value, value);
  assert.equal(saveButton(app).props.disabled, true);
  assert.match(textContent(dialog(app)), /An address cannot start or end with whitespace/);
  assert.doesNotMatch(textContent(dialog(app)), /Adresin başında veya sonunda/);
  field(app, "Fetch address").props.onChange({ target: { value: value.trim() } });
  await app.flush();
  submit(app);
  await app.flush();
  assert.equal(requests.length, 1);
  assert.equal(requests[0].action.fetchUrl, value.trim());
  assert.equal(requests[0].action.expectedRemoteToken, "origin-config");
  app.setLocale("tr");
  await app.flush();
  assert.equal(requests.length, 1, "Changing language must not retry a pending action");
  assert.equal(field(app, "Fetch adresi").props.value, value.trim());
});

test("stored local remote failure and returned application feedback relocalize with the open dialog", async (t) => {
  const { app, requests } = await mount(t);
  await open(app);
  await edit(app, "https://typed.example/origin.git");
  submit(app);
  await app.flush();
  requests[0].resolve(null);
  await app.flush();
  assert.match(textContent(dialog(app)), /Depo durumu değişti/);
  app.setLocale("en");
  await app.flush();
  assert.match(textContent(dialog(app)), /The repository state changed/);
  assert.equal(field(app, "Fetch address").props.value, "https://typed.example/origin.git");
  submit(app);
  await app.flush();
  requests[1].resolve({
    kind: "error", message: "State refresh failed: fatal: raw diagnostic", repositoryAction: requests[1].action,
    localizedMessage: { tr: "Durum yenilenemedi: fatal: raw diagnostic", en: "State refresh failed: fatal: raw diagnostic" },
  });
  await app.flush();
  app.setLocale("tr");
  await app.flush();
  assert.match(textContent(dialog(app)), /Durum yenilenemedi: fatal: raw diagnostic/);
  assert.equal(requests.length, 2);
});

test("raw remote diagnostics stay unchanged when the language switches", async (t) => {
  const { app, requests } = await mount(t);
  await open(app);
  await edit(app, "https://typed.example/origin.git");
  submit(app);
  await app.flush();
  const diagnostic = "fatal: remote 'özgün' already exists";
  requests[0].resolve({ kind: "error", message: diagnostic, repositoryAction: requests[0].action });
  await app.flush();
  app.setLocale("en");
  await app.flush();
  assert.ok(textContent(dialog(app)).includes(diagnostic));
});

test("branch drafts and success feedback survive language switching", async (t) => {
  const { app, requests } = await mount(t);
  const branchDialog = () => app.find((node) => node.type === "dialog" && !byClass("remote-manager-dialog")(node) && byClass("repository-dialog")(node));
  app.find(byClass("repository-branch-trigger")).props.onClick();
  await app.flush();
  app.find((node) => node.type === "button" && textContent(node) === "Yeni dal", branchDialog()).props.onClick();
  await app.flush();
  app.find((node) => node.type === "input" && node.props.placeholder === "feature/example", branchDialog()).props.onChange({ target: { value: "feature/çalışma" } });
  await app.flush();
  const surface = branchDialog().props.ref.current;
  app.setLocale("en");
  await app.flush();
  assert.equal(branchDialog().props.ref.current, surface);
  assert.equal(surface.showCount, 1);
  assert.equal(app.find((node) => node.type === "input" && node.props.placeholder === "feature/example", branchDialog()).props.value, "feature/çalışma");
  app.find((node) => node.type === "form", branchDialog()).props.onSubmit({ preventDefault() {} });
  await app.flush();
  assert.equal(requests.length, 1);
  assert.equal(requests[0].action.name, "feature/çalışma");
  const result = { kind: "success", message: "Created and switched", repositoryAction: requests[0].action };
  requests[0].resolve(result);
  app.setProps({ feedback: result });
  await app.flush();
  const notice = () => app.find(byClass("repository-notice"));
  assert.match(textContent(notice()), /Created and switched to branch “feature\/çalışma”/);
  app.setLocale("tr");
  await app.flush();
  assert.match(textContent(notice()), /“feature\/çalışma” dalı oluşturuldu ve bu dala geçildi/);
});
