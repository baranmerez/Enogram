import assert from "node:assert/strict";
import test from "node:test";
import { diffViewHarness, historyListHarness, textContent } from "../scripts/diffViewHarness.mjs";
import { parseDiffLines } from "./diffLines.ts";

const patch = "--- a/file.txt\n+++ b/file.txt\n@@ -2,2 +2,2 @@\n-İ\t[A]. old  \n+İ\t[a]. new  \n same\n@@ -80 +90 @@\n-old\n+<img src=x> [A].\n";
const result = (text = patch, overrides = {}) => ({ text, binary: false, truncated: false, ...overrides });
const byClass = (name) => (node) => node.props.className?.split(" ").includes(name);
const input = (app) => app.find((node) => node.type === "input" && node.props.type === "text");
const button = (app, label) => app.find((node) => node.type === "button" && node.props["aria-label"] === label);
const marks = (app) => app.all((node) => node.type === "mark");
const count = (app) => textContent(app.find(byClass("diff-search-count")));
const currentIndex = (app) => app.find((node) => node.type === "mark" && node.props["aria-current"] === true).props["data-diff-match"];
const selectionCount = (app) => textContent(app.find(byClass("diff-selection-count")));
const checkbox = (app, label) => app.find((node) => node.type === "input" && node.props.type === "checkbox" && node.props["aria-label"] === label);
const selectionOptions = (overrides = {}) => ({ staged: false, selectionToken: "prepared-1", selectableLineIds: [3, 4, 7, 8], unsupportedReason: null, busy: false, onApply: async () => {}, ...overrides });
function select(app, label, checked = true) { const control = checkbox(app, label); assert.equal(control.props.disabled, false); control.props.onChange({ target: { checked } }); app.flush(); }
const plainRequest = (request) => ({ selectionToken: request.selectionToken, lineIds: Array.from(request.lineIds) });
function search(app, value) { input(app).props.onChange({ target: { value } }); app.flush(); }
function click(app, label) { const control = button(app, label); assert.equal(control.props.disabled, false); const pending = control.props.onClick(); app.flush(); return pending; }
function key(app, keyValue, extra = {}) {
  const event = {
    key: keyValue, target: input(app).surface, nativeEvent: { isComposing: false },
    ctrlKey: false, metaKey: false, altKey: false, shiftKey: false,
    defaultPrevented: false, propagationStopped: false,
    preventDefault() { this.defaultPrevented = true; }, stopPropagation() { this.propagationStopped = true; }, ...extra,
  };
  app.find((node) => node.props.role === "region").props.onKeyDown(event);
  app.flush();
  return event;
}
function mount(t, overrides = {}, options) {
  const app = diffViewHarness({ state: "ready", result: result(), error: null, targetKey: "repo:working:unstaged:file.txt", ...overrides }, options);
  t.after(() => app.unmount());
  app.flush();
  return app;
}

test("search highlights literal occurrences without changing repository text, tabs or line numbers", (t) => {
  const app = mount(t);
  assert.equal(count(app), "0 / 0");
  assert.equal(button(app, "Sonraki eşleşme").props.disabled, true);
  search(app, "[a].");
  assert.equal(count(app), "1 / 3");
  assert.deepEqual(marks(app).map(textContent), ["[A].", "[a].", "[A]."]);
  assert.deepEqual(app.all((node) => node.type === "code").map(textContent), parseDiffLines(patch).map((line) => line.text));
  assert.equal(app.all((node) => node.type === "img").length, 0, "Repository markup remains text");
  const rows = app.all(byClass("diff-view-row"));
  assert.equal(textContent(app.find(byClass("diff-view-old-number"), rows[3])), "2");
  assert.equal(textContent(app.find(byClass("diff-view-new-number"), rows[4])), "2");
  assert.equal(app.scrolls.at(-1).props.className, "diff-scroll");
  search(app, "\t");
  assert.equal(marks(app).length, 2);
  assert.ok(marks(app).every((mark) => textContent(mark) === "\t"));
});

test("Enter, F3 and buttons wrap matches while hunk navigation stays independent", (t) => {
  const app = mount(t);
  search(app, "[a].");
  assert.equal(currentIndex(app), 0);
  key(app, "Enter");
  assert.equal(currentIndex(app), 1);
  key(app, "Enter", { shiftKey: true });
  assert.equal(currentIndex(app), 0);
  key(app, "F3", { shiftKey: true, target: null });
  assert.equal(currentIndex(app), 2);
  key(app, "F3", { target: null });
  assert.equal(currentIndex(app), 0);
  click(app, "Önceki eşleşme");
  assert.equal(currentIndex(app), 2);
  click(app, "Sonraki eşleşme");
  assert.equal(currentIndex(app), 0);
  click(app, "Sonraki değişiklik bölümü");
  assert.equal(app.find(byClass("is-current-hunk")).props["data-diff-line"], 2);
  assert.equal(count(app), "1 / 3");
  click(app, "Önceki değişiklik bölümü");
  assert.equal(app.find(byClass("is-current-hunk")).props["data-diff-line"], 6);
  assert.equal(textContent(app.find(byClass("diff-hunk-count"))), "Bölüm 2 / 2");
  search(app, "missing");
  assert.equal(count(app), "0 / 0");
  assert.equal(button(app, "Sonraki eşleşme").props.disabled, true);
  click(app, "Sonraki değişiklik bölümü");
  assert.equal(app.find(byClass("is-current-hunk")).props["data-diff-line"], 2);
});

test("match and hunk navigation scroll only the code viewport at large two-axis offsets", (t) => {
  const app = mount(t);
  const viewport = app.find(byClass("diff-scroll")).surface;
  const toolbar = app.find(byClass("diff-view-toolbar")).surface;
  viewport.clientTop = 2;
  viewport.clientLeft = 2;
  viewport.clientWidth = 300;
  viewport.clientHeight = 200;
  viewport.scrollWidth = 5_000;
  viewport.scrollHeight = 10_000;
  viewport.rect = () => ({ top: 180, left: 680, width: 320, height: 220, right: 1_000, bottom: 400 });
  toolbar.rect = () => ({ top: 110, left: 680, width: 320, height: 70, right: 1_000, bottom: 180 });
  const beforeToolbar = toolbar.getBoundingClientRect();
  const beforeOuterScroll = { ...app.outerScroll };
  const locate = (surface, top, left, width = 35, height = 23) => {
    surface.rect = () => ({
      top: 182 + top - viewport.scrollTop,
      left: 682 + left - viewport.scrollLeft,
      right: 682 + left - viewport.scrollLeft + width,
      bottom: 182 + top - viewport.scrollTop + height,
      width, height,
    });
  };

  search(app, "[a].");
  locate(marks(app)[1].surface, 7_000, 4_000);
  click(app, "Sonraki eşleşme");
  assert.equal(viewport.scrollTop, 6_911.5, "Center the distant match in the owned viewport");
  assert.equal(viewport.scrollLeft, 3_741, "Reveal its right edge without moving the outer panel");
  let target = marks(app)[1].surface.getBoundingClientRect();
  assert.ok(target.left >= 682 && target.right <= 982 && target.top >= 182 && target.bottom <= 382);

  locate(marks(app)[2].surface, 400, 15);
  key(app, "F3");
  assert.equal(viewport.scrollTop, 311.5);
  assert.equal(viewport.scrollLeft, 9, "Moving back to a left-side match also reveals its start");
  target = marks(app)[2].surface.getBoundingClientRect();
  assert.ok(target.left >= 682 && target.right <= 982 && target.top >= 182 && target.bottom <= 382);

  const hunk = app.find((node) => node.props["data-diff-line"] === 2).surface;
  locate(hunk, 2_300, 0, 5_000);
  click(app, "Sonraki değişiklik bölümü");
  assert.equal(viewport.scrollTop, 2_300);
  assert.equal(viewport.scrollLeft, 0, "Hunk navigation reveals the header's start, not a wide row's far edge");
  assert.deepEqual(app.outerScroll, beforeOuterScroll);
  assert.deepEqual(toolbar.getBoundingClientRect(), beforeToolbar);
  assert.ok(app.scrolls.every((entry) => entry.scope === "element" && entry.props.className === "diff-scroll"));
});

test("diff focus owns Ctrl+F, preserves global modified shortcuts and clears with Escape", (t) => {
  const app = mount(t);
  const event = key(app, "f", { ctrlKey: true, target: null });
  assert.equal(event.defaultPrevented, true);
  assert.equal(event.propagationStopped, true);
  assert.equal(app.activeElement, input(app).surface);
  assert.equal(input(app).surface.selectCount, 1);
  assert.equal(key(app, "f", { ctrlKey: true, shiftKey: true }).propagationStopped, false);
  assert.equal(key(app, "f", { metaKey: true }).propagationStopped, true);
  search(app, "old");
  assert.equal(key(app, "Enter", { nativeEvent: { isComposing: true } }).defaultPrevented, false);
  assert.equal(key(app, "Enter", { target: button(app, "Sonraki eşleşme").surface }).defaultPrevented, false, "Button Enter must not navigate twice");
  key(app, "Escape");
  assert.equal(input(app).props.value, "");
  assert.equal(marks(app).length, 0);
  assert.equal(button(app, "Fark aramasını temizle").props.disabled, true);
  search(app, "old");
  click(app, "Fark aramasını temizle");
  assert.equal(input(app).props.value, "");
  assert.equal(app.activeElement, input(app).surface);
});

test("updated results reconcile offsets immediately; identical text in a new target gets fresh search state", (t) => {
  const app = mount(t);
  search(app, "old");
  click(app, "Sonraki eşleşme");
  click(app, "Sonraki değişiklik bölümü");
  assert.equal(currentIndex(app), 1);
  app.setProps({ result: result("@@ -1 +1 @@\n-old\n+new\n") });
  app.flush();
  assert.equal(input(app).props.value, "old");
  assert.equal(count(app), "1 / 1");
  assert.equal(currentIndex(app), 0);
  assert.equal(app.all(byClass("is-current-hunk")).length, 0);
  app.setProps({ targetKey: "repo:working:staged:file.txt" });
  app.flush();
  assert.equal(input(app).props.value, "");
  assert.equal(marks(app).length, 0);
  search(app, "old");
  app.setProps({ state: "loading" });
  app.flush();
  assert.equal(app.all((node) => node.type === "input").length, 0);
  app.setProps({ state: "ready" });
  app.flush();
  assert.equal(input(app).props.value, "");
});

test("truncated output declares the visible-only boundary and metadata-only output has no hunk targets", (t) => {
  const app = mount(t, { result: result("--- a/[needle].txt\n+++ b/[needle].txt\n", { truncated: true }) });
  assert.match(textContent(app.find(byClass("diff-truncated"))), /yalnızca gösterilen kısımda/);
  search(app, "[needle]");
  assert.equal(count(app), "1 / 2");
  assert.equal(button(app, "Önceki değişiklik bölümü").props.disabled, true);
  assert.equal(button(app, "Sonraki değişiklik bölümü").props.disabled, true);
});

test("excess matches show a visible limit and navigation covers only the retained highlights", (t) => {
  const text = "a".repeat(2_001);
  const app = mount(t, { result: result(text) });
  search(app, "a");
  assert.equal(count(app), "1 / 2.000+");
  assert.equal(marks(app).length, 2_000);
  assert.match(textContent(app.find(byClass("diff-search-limit"))), /İlk 2\.000 eşleşme gösteriliyor/);
  assert.equal(textContent(app.find((node) => node.type === "code")), text);
  click(app, "Önceki eşleşme");
  assert.equal(currentIndex(app), 1_999);
  click(app, "Sonraki eşleşme");
  assert.equal(currentIndex(app), 0);
  search(app, "absent");
  assert.equal(app.all(byClass("diff-search-limit")).length, 0);
});

test("only server-authorized changed rows are selectable, preserving physical row IDs and source rendering", async (t) => {
  const text = "diff --git a/file.txt b/file.txt\n--- a/file.txt\n+++ b/file.txt\n@@ -1,2 +1,2 @@\n-old  \n\\ No newline at end of file\n+new\t  \n same\n";
  const requests = [];
  const app = mount(t, { result: result(text), partialSelection: selectionOptions({
    selectableLineIds: [0, 1, 2, 3, 4, 5, 6, 7, 8, -1, 2.5, 100],
    onApply: async (request) => requests.push(plainRequest(request)),
  }) });
  const rows = app.all(byClass("diff-view-row"));
  assert.deepEqual(rows.map((row) => app.all((node) => node.type === "input" && node.props.type === "checkbox", row).length), [0, 0, 0, 1, 1, 0, 1, 0]);
  assert.deepEqual(app.all((node) => node.type === "code").map(textContent), parseDiffLines(text).map((line) => line.text));
  assert.equal(textContent(app.find(byClass("diff-view-old-number"), rows[4])), "1");
  assert.equal(textContent(app.find(byClass("diff-view-new-number"), rows[6])), "1");
  select(app, "1. bölümün değişen satırlarını seç");
  assert.equal(selectionCount(app), "2 satır seçili");
  await click(app, "Seçili satırları Stage'e al"); app.flush();
  assert.deepEqual(requests, [{ selectionToken: "prepared-1", lineIds: [4, 6] }]);
});

test("mixed selections across hunks submit sorted stable IDs and keep search and hunk navigation intact", async (t) => {
  const requests = [];
  const app = mount(t, { partialSelection: selectionOptions({ onApply: async (request) => requests.push(plainRequest(request)) }) });
  search(app, "[a].");
  select(app, "Eklenen satır 90 seç");
  select(app, "Silinen satır 2 seç");
  assert.equal(selectionCount(app), "2 satır seçili");
  for (const number of [1, 2]) {
    const hunk = checkbox(app, `${number}. bölümün değişen satırlarını seç`);
    assert.equal(hunk.props["aria-checked"], "mixed");
    assert.equal(hunk.surface.indeterminate, true);
  }
  click(app, "Sonraki değişiklik bölümü");
  click(app, "Sonraki eşleşme");
  assert.equal(currentIndex(app), 1);
  assert.equal(selectionCount(app), "2 satır seçili");
  await click(app, "Seçili satırları Stage'e al"); app.flush();
  assert.deepEqual(requests, [{ selectionToken: "prepared-1", lineIds: [3, 8] }]);
  assert.equal(selectionCount(app), "0 satır seçili");
  assert.equal(input(app).props.value, "[a].");
  assert.equal(currentIndex(app), 1);
  assert.deepEqual(app.all((node) => node.type === "code").map(textContent), parseDiffLines(patch).map((line) => line.text));
  assert.ok(app.scrolls.every((entry) => entry.scope === "element" && entry.props.className === "diff-scroll"));
});

test("hunk selection intersects authorized rows; clearing and a direct hunk action have explicit scopes", async (t) => {
  const requests = [];
  const app = mount(t, { partialSelection: selectionOptions({ selectableLineIds: [3, 7, 8], onApply: async (request) => requests.push(plainRequest(request)) }) });
  assert.equal(app.all((node) => node.type === "input" && node.props["aria-label"] === "Eklenen satır 2 seç").length, 0);
  select(app, "1. bölümün değişen satırlarını seç");
  select(app, "2. bölümün değişen satırlarını seç");
  assert.equal(selectionCount(app), "3 satır seçili");
  assert.equal(checkbox(app, "1. bölümün değişen satırlarını seç").props.checked, true);
  select(app, "Silinen satır 80 seç", false);
  assert.equal(checkbox(app, "2. bölümün değişen satırlarını seç").props["aria-checked"], "mixed");
  select(app, "2. bölümün değişen satırlarını seç", false);
  assert.equal(selectionCount(app), "1 satır seçili");
  click(app, "Satır seçimini temizle");
  assert.equal(selectionCount(app), "0 satır seçili");
  assert.equal(button(app, "Seçili satırları Stage'e al").props.disabled, true);
  select(app, "1. bölümün değişen satırlarını seç");
  await click(app, "2. bölümü Stage yap"); app.flush();
  assert.deepEqual(requests, [{ selectionToken: "prepared-1", lineIds: [7, 8] }], "Direct hunk action must not include another hunk's selection");
});

test("staged diffs expose Unstage and send their captured token with selected line IDs", async (t) => {
  const requests = [];
  const app = mount(t, { partialSelection: selectionOptions({ staged: true, selectionToken: "staged-token", onApply: async (request) => requests.push(plainRequest(request)) }) });
  assert.equal(app.all((node) => node.type === "button" && textContent(node) === "Stage").length, 0);
  select(app, "Eklenen satır 2 seç");
  assert.equal(textContent(button(app, "Seçili satırları Unstage yap")), "Unstage");
  await click(app, "Seçili satırları Unstage yap"); app.flush();
  await click(app, "2. bölümü Unstage yap"); app.flush();
  assert.deepEqual(requests, [
    { selectionToken: "staged-token", lineIds: [4] },
    { selectionToken: "staged-token", lineIds: [7, 8] },
  ]);
});

test("selection resets on identical refreshed results, token changes, layer changes and target changes", (t) => {
  let options = selectionOptions();
  const app = mount(t, { partialSelection: options });
  search(app, "old");
  const expectReset = (props) => {
    select(app, "Silinen satır 2 seç");
    assert.equal(selectionCount(app), "1 satır seçili");
    app.setProps(props); app.flush();
    assert.equal(selectionCount(app), "0 satır seçili");
    assert.ok(app.all((node) => node.type === "input" && node.props.type === "checkbox").every((node) => !node.props.checked && !node.surface.indeterminate));
  };
  expectReset({ result: result() });
  assert.equal(input(app).props.value, "old");
  options = { ...options, selectionToken: "prepared-2" };
  expectReset({ partialSelection: options });
  options = { ...options, staged: true };
  expectReset({ partialSelection: options });
  expectReset({ targetKey: "other-repo:working:staged:file.txt" });
  assert.equal(input(app).props.value, "");
});

test("synchronous double-submit guard disables all selection controls until settlement and reports rejection", async (t) => {
  const requests = [];
  let rejectApply;
  const pending = new Promise((_resolve, reject) => { rejectApply = reject; });
  const app = mount(t, { partialSelection: selectionOptions({ onApply: (request) => { requests.push(plainRequest(request)); return pending; } }) });
  select(app, "Silinen satır 2 seç");
  const apply = button(app, "Seçili satırları Stage'e al").props.onClick;
  const clear = button(app, "Satır seçimini temizle").props.onClick;
  const change = checkbox(app, "Eklenen satır 2 seç").props.onChange;
  const firstSubmit = apply(); apply(); clear(); change({ target: { checked: true } });
  app.flush();
  assert.equal(requests.length, 1);
  assert.equal(selectionCount(app), "1 satır seçili");
  assert.equal(button(app, "Satır seçimini temizle").props.disabled, true);
  assert.equal(button(app, "Seçili satırları Stage'e al").props.disabled, true);
  assert.equal(button(app, "2. bölümü Stage yap").props.disabled, true);
  assert.ok(app.all((node) => node.type === "input" && node.props.type === "checkbox").every((node) => node.props.disabled));
  search(app, "old");
  assert.equal(count(app), "1 / 2", "Read-only navigation remains usable during a write");
  rejectApply(new Error("The index changed; refresh the diff."));
  await firstSubmit; app.flush();
  assert.match(textContent(app.find((node) => node.props.role === "alert")), /index changed/);
  assert.equal(selectionCount(app), "1 satır seçili");
  assert.equal(button(app, "Seçili satırları Stage'e al").props.disabled, false);
  assert.equal(requests.length, 1, "A failed selection is never retried automatically");
});

test("parent busy and unsupported states fail closed while preserving normal viewing and search", (t) => {
  const requests = [];
  let options = selectionOptions({ onApply: async (request) => requests.push(plainRequest(request)) });
  const app = mount(t, { partialSelection: options });
  select(app, "Silinen satır 2 seç");
  options = { ...options, busy: true };
  app.setProps({ partialSelection: options }); app.flush();
  button(app, "Seçili satırları Stage'e al").props.onClick();
  button(app, "1. bölümü Stage yap").props.onClick();
  checkbox(app, "Eklenen satır 2 seç").props.onChange({ target: { checked: true } });
  app.flush();
  assert.equal(selectionCount(app), "1 satır seçili");
  assert.equal(requests.length, 0);
  options = { ...options, busy: false, unsupportedReason: "Renamed files require whole-file staging." };
  app.setProps({ partialSelection: options }); app.flush();
  assert.equal(app.all((node) => node.type === "input" && node.props.type === "checkbox").length, 0);
  assert.equal(selectionCount(app), "0 satır seçili");
  assert.equal(button(app, "Seçili satırları Stage'e al").props.disabled, true);
  assert.match(textContent(app.find(byClass("diff-selection-notice"))), /Renamed files/);
  search(app, "old");
  assert.equal(count(app), "1 / 2");
  app.setProps({ partialSelection: undefined }); app.flush();
  assert.equal(app.all(byClass("diff-selection-toolbar")).length, 0, "History or omitted partial-selection metadata stays read-only");
  assert.equal(app.all(byClass("diff-selection-notice")).length, 0);
});

test("checkboxes retain native keyboard behavior and truncated or binary diffs never offer partial actions", (t) => {
  const app = mount(t, { partialSelection: selectionOptions() });
  const control = checkbox(app, "Silinen satır 2 seç");
  assert.equal(control.props.type, "checkbox");
  assert.equal(key(app, " ", { target: control.surface }).defaultPrevented, false);
  assert.equal(key(app, "Enter", { target: control.surface }).defaultPrevented, false);
  app.setProps({ result: result(patch, { truncated: true }) }); app.flush();
  assert.equal(app.all((node) => node.type === "input" && node.props.type === "checkbox").length, 0);
  assert.match(textContent(app.find(byClass("diff-selection-notice"))), /Kısaltılmış/);
  search(app, "old");
  assert.equal(count(app), "1 / 2");
  app.setProps({ result: result("binary", { binary: true }), partialSelection: selectionOptions({ unsupportedReason: "Binary staging is unavailable." }) }); app.flush();
  assert.match(textContent(app.find(byClass("diff-selection-notice"))), /Binary staging/);
  assert.equal(app.all((node) => node.type === "input" || node.type === "button").length, 0);
});

test("search reveals a distant match beyond the sticky selection gutter without moving outer panels", (t) => {
  const app = mount(t, { partialSelection: selectionOptions() });
  search(app, "[a].");
  const viewport = app.find(byClass("diff-scroll")).surface;
  viewport.scrollLeft = 2_000;
  viewport.scrollWidth = 5_000;
  viewport.rect = () => ({ top: 0, left: 0, width: 320, height: 300, right: 320, bottom: 300 });
  marks(app)[1].surface.rect = () => ({ top: 40, left: 1_500 - viewport.scrollLeft, width: 35, height: 23, right: 1_535 - viewport.scrollLeft, bottom: 63 });
  click(app, "Sonraki eşleşme");
  assert.equal(viewport.scrollLeft, 1_470, "Reserve the 24px checkbox gutter and a 6px gap");
  assert.equal(marks(app)[1].surface.getBoundingClientRect().left, 30);
  assert.deepEqual(app.outerScroll, { scrollTop: 11, scrollLeft: 17 });
});

for (const [label, props, message] of [
  ["binary", { result: result("binary", { binary: true }) }, /ikili/],
  ["empty", { result: result("") }, /gösterilecek fark yok/],
  ["unselected", { result: null, state: "idle" }, /bir dosya seçin/],
  ["loading", { state: "loading" }, /okunuyor/],
  ["error", { state: "error", error: "Diff failed" }, /Diff failed/],
]) {
  test(`${label} state does not expose stale search or navigation controls`, (t) => {
    const app = mount(t, props);
    assert.equal(app.all((node) => node.type === "input" || node.type === "button").length, 0);
    assert.match(textContent(app.find(byClass("panel-message"))), message);
  });
}

test("switching diff language preserves search, selected rows, hunk position and viewport", async (t) => {
  const requests = [];
  const app = mount(t, { partialSelection: selectionOptions({ onApply: async (request) => requests.push(plainRequest(request)) }) });
  search(app, "[a].");
  click(app, "Sonraki eşleşme");
  click(app, "Sonraki değişiklik bölümü");
  select(app, "Eklenen satır 90 seç");
  const viewport = app.find(byClass("diff-scroll")).surface;
  viewport.scrollTop = 120;
  viewport.scrollLeft = 42;
  const scrollCount = app.scrolls.length;
  const source = app.all((node) => node.type === "code").map(textContent);

  app.setLocale("en"); app.flush();
  assert.equal(input(app).props["aria-label"], "Search diff");
  assert.equal(input(app).props.placeholder, "Search diff…");
  assert.equal(input(app).props.value, "[a].");
  assert.equal(currentIndex(app), 1);
  assert.equal(app.find(byClass("is-current-hunk")).props["data-diff-line"], 2);
  assert.equal(textContent(app.find(byClass("diff-hunk-count"))), "Hunk 1 / 2");
  assert.equal(selectionCount(app), "1 line selected");
  assert.equal(checkbox(app, "Select added line 90").props.checked, true);
  assert.equal(button(app, "Next match").props.title, "Next match (Enter / F3)");
  assert.equal(app.find(byClass("diff-search-count")).props["aria-label"], "Match 2 of 3");
  assert.equal(textContent(button(app, "Stage hunk 1")), "Stage hunk");
  assert.deepEqual(app.all((node) => node.type === "code").map(textContent), source);
  assert.equal(viewport.scrollTop, 120);
  assert.equal(viewport.scrollLeft, 42);
  assert.equal(app.scrolls.length, scrollCount, "Language changes must not navigate the diff");

  await click(app, "Stage selected lines"); app.flush();
  assert.deepEqual(requests, [{ selectionToken: "prepared-1", lineIds: [8] }]);
  app.setLocale("tr"); app.flush();
  assert.equal(input(app).props["aria-label"], "Farkta ara");
  assert.equal(selectionCount(app), "0 satır seçili");
  assert.equal(input(app).props.value, "[a].");
  assert.equal(currentIndex(app), 1);
});

test("English diff limits, unavailable selections and state messages follow the active language", (t) => {
  const app = mount(t, { result: result("a".repeat(2_001), { truncated: true }), partialSelection: selectionOptions() }, { locale: "en" });
  search(app, "a");
  assert.equal(count(app), "1 / 2,000+");
  assert.equal(app.find(byClass("diff-search-count")).props["aria-label"], "Match 1 of at least 2,000");
  assert.match(textContent(app.find(byClass("diff-search-limit"))), /first 2,000 matches/);
  assert.match(textContent(app.find(byClass("diff-truncated"))), /only the displayed portion/);
  assert.match(textContent(app.find(byClass("diff-selection-notice"))), /truncated diff/);

  for (const [props, message] of [
    [{ result: result("binary", { binary: true }) }, /binary file/],
    [{ result: result("") }, /No changes to show/],
    [{ result: null, state: "idle" }, /Select a file/],
    [{ state: "loading" }, /Loading diff/],
    [{ state: "error", error: "Raw Git diagnostic: İ /tmp/örnek" }, /Raw Git diagnostic: İ \/tmp\/örnek/],
  ]) {
    app.setProps(props); app.flush();
    assert.match(textContent(app.find(byClass("panel-message"))), message);
    assert.equal(app.all((node) => node.type === "input" || node.type === "button").length, 0);
  }
});

test("known backend selection reasons translate on switch while unknown diagnostics stay exact", (t) => {
  const reason = "Binary files do not support individual-line actions.";
  const options = selectionOptions({ unsupportedReason: reason });
  const app = mount(t, { partialSelection: options });
  assert.match(textContent(app.find(byClass("diff-selection-notice"))), /İkili dosyalarda/);
  app.setLocale("en"); app.flush();
  assert.equal(textContent(app.find(byClass("diff-selection-notice"))), reason);
  const diagnostic = "External helper: İĞÜŞ /work/örnek.txt";
  app.setProps({ partialSelection: { ...options, unsupportedReason: diagnostic } }); app.flush();
  app.setLocale("tr"); app.flush();
  assert.equal(textContent(app.find(byClass("diff-selection-notice"))), diagnostic);
});

test("history language changes localize captions and dates while preserving commits, refs and graph selection", (t) => {
  const commits = [
    { oid: "c".repeat(40), parents: ["b".repeat(40), "outside-parent"], author: "İpek Ünal", date: "2026-01-02T10:35:00Z", subject: "Türkçe commit başlığı", refs: "HEAD -> özellik/iş, tag: sürüm-1, origin/özellik" },
    { oid: "b".repeat(40), parents: [], author: "Raw Author", date: "", subject: "", refs: "" },
  ];
  const selections = [];
  const app = historyListHarness({ commits, selectedOid: commits[0].oid, onSelect: (commit) => selections.push(commit) });
  t.after(() => app.unmount());
  app.flush();
  const paths = app.all((node) => node.type === "path").map((node) => node.props.d);
  const trDate = textContent(app.all(byClass("history-graph-date"))[0]);
  assert.equal(app.find(byClass("history-graph-rows")).props["aria-label"], "Commit geçmişi");

  app.setLocale("en"); app.flush();
  assert.equal(app.find(byClass("history-graph-rows")).props["aria-label"], "Commit history");
  assert.deepEqual(app.all(byClass("history-graph-subject")).map(textContent), [commits[0].subject, "(Untitled commit)"]);
  assert.deepEqual(app.all(byClass("history-graph-author")).map(textContent), commits.map((commit) => commit.author));
  assert.equal(app.find(byClass("history-graph-ref")).props.title, commits[0].refs);
  assert.equal(textContent(app.find(byClass("history-graph-ref"))), "HEAD → özellik/iş");
  assert.equal(app.find(byClass("history-graph-ref-more")).props["aria-label"], "Other references: tag: sürüm-1, origin/özellik");
  const dates = app.all(byClass("history-graph-date")).map(textContent);
  assert.equal(dates[0], new Intl.DateTimeFormat("en-US", { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit" }).format(new Date(commits[0].date)));
  assert.notEqual(dates[0], trDate);
  assert.equal(dates[1], "Unknown date");
  const rows = app.all(byClass("history-graph-commit"));
  assert.equal(rows[0].props["aria-pressed"], true);
  assert.match(rows[0].props.title, /Parent commits: b{40}, outside-parent/);
  assert.match(rows[1].props.title, /Root commit/);
  const continuation = app.find(byClass("history-graph-continuation"));
  assert.equal(continuation.props.title, "1 parent commit is outside this view: outside-parent");
  assert.match(textContent(continuation), /History continues beyond this list/);
  assert.deepEqual(app.all((node) => node.type === "path").map((node) => node.props.d), paths);
  rows[0].props.onClick();
  assert.equal(selections[0], commits[0]);

  app.setLocale("tr"); app.flush();
  assert.equal(textContent(app.all(byClass("history-graph-date"))[0]), trDate);
  assert.equal(app.find(byClass("history-graph-rows")).props["aria-label"], "Commit geçmişi");
});
