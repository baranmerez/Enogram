import assert from "node:assert/strict";
import test from "node:test";
import { defaultWorkspacePreferences, fitPanelWidths, loadWorkspacePreferences, normalizeWorkspacePreferences, saveWorkspacePreferences, workspacePreferencesKey } from "./workspacePreferences.ts";

test("a narrow viewport fits without losing preferred widths when expanded", () => {
  const preferred = { sidebar: 248, center: 470 };
  assert.deepEqual(fitPanelWidths(preferred, 986, false), { sidebar: 248, center: 406 });
  assert.deepEqual(fitPanelWidths(preferred, 1542, false), preferred);
  assert.deepEqual(fitPanelWidths(preferred, 986, true), preferred);
});
test("workspace preferences persist independently of appearance and restore safely", () => {
  const values = new Map([["enogram.theme.v1", "untouched"]]);
  const storage = { getItem: (key) => values.get(key), setItem: (key, value) => values.set(key, value) };
  const desired = { density: "comfortable", sidebarCollapsed: true, panelWidths: { sidebar: 300, center: 600 } };
  assert.equal(saveWorkspacePreferences(desired, storage), true);
  assert.deepEqual(loadWorkspacePreferences(storage), desired);
  assert.equal(values.get("enogram.theme.v1"), "untouched");
  values.set(workspacePreferencesKey, "invalid json");
  assert.deepEqual(loadWorkspacePreferences(storage), defaultWorkspacePreferences);
  assert.deepEqual(loadWorkspacePreferences({ getItem() { throw new Error("unavailable"); } }), defaultWorkspacePreferences);
  assert.equal(saveWorkspacePreferences(desired, { setItem() { throw new Error("unavailable"); } }), false);
});
test("restored dimensions reject invalid values and constrain extreme settings", () => {
  assert.deepEqual(normalizeWorkspacePreferences({ density: "invalid", sidebarCollapsed: "yes", panelWidths: { sidebar: Infinity, center: "500px" } }), defaultWorkspacePreferences);
  assert.deepEqual(normalizeWorkspacePreferences({ panelWidths: { sidebar: -1, center: 1e12 } }).panelWidths, { sidebar: 196, center: 1600 });
});
