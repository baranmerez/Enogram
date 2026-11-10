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
