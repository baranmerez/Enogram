import assert from "node:assert/strict";
import test from "node:test";
import {
  applyTheme,
  contrastRatio,
  createThemeTokens,
  defaultThemeSettings,
  initializeTheme,
  loadThemeSettings,
  normalizeHexColor,
  normalizeThemeSettings,
  relativeLuminance,
  saveThemeSettings,
  settingsForPreset,
  themeLimits,
  themePresets,
  themeStorageKey,
} from "./theme.ts";

function memoryStorage(initial) {
  const values = new Map(initial === undefined ? [] : [[themeStorageKey, initial]]);
  return {
    getItem(key) { return values.get(key) ?? null; },
    setItem(key, value) { values.set(key, value); },
  };
}

function themeTarget() {
  const properties = new Map();
  const attributes = new Map();
  return {
    properties,
    attributes,
    style: { setProperty(name, value) { properties.set(name, value); } },
    setAttribute(name, value) { attributes.set(name, value); },
  };
}

function assertContrast(foreground, background, minimum, description) {
  const ratio = contrastRatio(foreground, background);
  assert.ok(ratio >= minimum, `${description}: ${foreground} on ${background} has contrast ${ratio.toFixed(3)} < ${minimum}`);
}

test("hex input permits shorthand and rejects color functions or CSS injection", () => {
  assert.equal(normalizeHexColor(" #aBc "), "#AABBCC");
  assert.equal(normalizeHexColor("75aef2"), "#75AEF2");
  assert.equal(normalizeHexColor("#123456"), "#123456");
  for (const value of [null, 123, {}, "", "#1234", "#12345678", "transparent", "red", "rgb(1, 2, 3)", "#123456; color:red", "#１２３４５６"]) {
    assert.equal(normalizeHexColor(value), null, String(value));
  }
});

test("contrast math matches black, white and the WCAG middle gray boundary", () => {
  assert.equal(relativeLuminance("#000000"), 0);
  assert.equal(relativeLuminance("#FFFFFF"), 1);
  assert.equal(contrastRatio("#000000", "#FFFFFF"), 21);
  assert.equal(contrastRatio("#AABBCC", "#AABBCC"), 1);
  assert.ok(contrastRatio("#767676", "#FFFFFF") > 4.5);
  assert.ok(contrastRatio("#777777", "#FFFFFF") < 4.5);
  assert.throws(() => relativeLuminance("invalid"), TypeError);
});

test("settings constrain numbers, enum values, versions and custom palette identity", () => {
  for (const invalid of [null, [], "dark", 7, {}, { version: 2, seed: "#FF0000" }]) {
    assert.deepEqual(normalizeThemeSettings(invalid), defaultThemeSettings);
  }
  const extreme = normalizeThemeSettings({ ...defaultThemeSettings, opacity: -10, blur: 999, seed: "#f00", mode: "light" });
  assert.equal(extreme.opacity, themeLimits.minOpacity);
  assert.equal(extreme.blur, themeLimits.maxBlur);
  assert.equal(extreme.seed, "#FF0000");
  assert.equal(extreme.preset, "custom");
  const malformed = normalizeThemeSettings({ ...defaultThemeSettings, opacity: Number.NaN, blur: Infinity, buttonStyle: "bad", surface: "bad", mode: "bad", seed: "url(evil)" });
  assert.deepEqual(malformed, defaultThemeSettings);
  const rounded = normalizeThemeSettings({ ...defaultThemeSettings, opacity: 90.6, blur: 15.3 });
  assert.equal(rounded.opacity, 91);
  assert.equal(rounded.blur, 15);
});

test("curated presets restore complete independent settings", () => {
  for (const preset of themePresets) {
    const settings = settingsForPreset(preset.id);
    assert.deepEqual(normalizeThemeSettings(settings), settings);
    settings.opacity = themeLimits.minOpacity;
    assert.equal(settingsForPreset(preset.id).opacity, preset.settings.opacity);
  }
  assert.ok(themePresets.some((preset) => preset.settings.mode === "light"));
  assert.ok(themePresets.some((preset) => preset.settings.surface === "glass"));
  assert.ok(themePresets.some((preset) => preset.settings.surface === "solid"));
});

test("every preset has accessible text, semantic states, buttons and graph lanes", () => {
  for (const preset of themePresets) verifyPalette(preset.settings);
});

function verifyPalette(settings) {
  const tokens = createThemeTokens(settings);
  const identity = `${settings.mode} ${settings.seed}`;
  const surfaces = ["--canvas", "--panel", "--panel-raised", "--panel-low", "--selection", "--blue-soft"];
  for (const surface of surfaces) {
    assertContrast(tokens["--text"], tokens[surface], 7, `${identity} text ${surface}`);
    assertContrast(tokens["--muted"], tokens[surface], 4.5, `${identity} muted ${surface}`);
    assertContrast(tokens["--dim"], tokens[surface], 4.5, `${identity} dim ${surface}`);
    for (let lane = 1; lane <= 8; lane += 1) assertContrast(tokens[`--graph-${lane}`], tokens[surface], 3, `${identity} graph ${lane} ${surface}`);
  }
  assertContrast(tokens["--accent-contrast"], tokens["--blue"], 4.5, `${identity} solid button`);
  for (const semantic of ["blue", "green", "red", "amber"]) {
    assertContrast(tokens[`--${semantic}`], tokens[`--${semantic}-soft`], 4.5, `${identity} ${semantic} soft`);
    for (const surface of surfaces.slice(0, 4)) assertContrast(tokens[`--${semantic}`], tokens[surface], 4.5, `${identity} ${semantic} ${surface}`);
  }
  assert.ok(Object.values(tokens).every((value) => !value.includes("NaN") && !value.includes("undefined")));
}

test("seed generation stays deterministic and readable over a full RGB grid in both modes", () => {
  const channels = [0, 51, 102, 153, 204, 255];
  for (const red of channels) for (const green of channels) for (const blue of channels) {
    const seed = `#${[red, green, blue].map((value) => value.toString(16).padStart(2, "0")).join("")}`;
    for (const mode of ["dark", "light"]) {
      const settings = { ...defaultThemeSettings, preset: "custom", seed, mode };
      verifyPalette(settings);
      assert.deepEqual(createThemeTokens(settings), createThemeTokens(settings));
    }
  }
});

test("valid settings persist and restore without writing during initialization", () => {
  const storage = memoryStorage();
  assert.deepEqual(loadThemeSettings(storage), { settings: defaultThemeSettings, storageStatus: "available" });
  const settings = { ...settingsForPreset("glacier"), buttonStyle: "outline", opacity: 73, blur: 28 };
  assert.equal(saveThemeSettings(settings, storage), true);
  assert.deepEqual(loadThemeSettings(storage), { settings, storageStatus: "available" });
  const target = themeTarget();
  const initialized = initializeTheme(target, { getItem: storage.getItem, setItem() { assert.fail("startup must not write preferences"); } });
  assert.deepEqual(initialized.settings, settings);
  assert.equal(target.attributes.get("data-theme"), "light");
  assert.equal(target.attributes.get("data-surface"), "glass");
  assert.equal(target.attributes.get("data-button-style"), "outline");
  assert.equal(target.properties.get("--glass-opacity"), "0.73");
  assert.equal(target.properties.get("--glass-blur"), "28px");
});

test("corrupt or obsolete saved values recover and storage failures remain explicit", () => {
  for (const raw of ["{broken", "null", "[]", '{"version":2,"mode":"light"}']) {
    assert.deepEqual(loadThemeSettings(memoryStorage(raw)), { settings: defaultThemeSettings, storageStatus: "recovered" });
  }
  const repaired = loadThemeSettings(memoryStorage(JSON.stringify({ ...settingsForPreset("aurora"), opacity: 1000, blur: -4 })));
  assert.equal(repaired.storageStatus, "recovered");
  assert.equal(repaired.settings.opacity, themeLimits.maxOpacity);
  assert.equal(repaired.settings.blur, themeLimits.minBlur);
  const denied = { getItem() { throw new Error("denied"); }, setItem() { throw new Error("quota"); } };
  assert.equal(loadThemeSettings(denied).storageStatus, "unavailable");
  assert.equal(saveThemeSettings(defaultThemeSettings, denied), false);
  assert.equal(loadThemeSettings(null).storageStatus, "unavailable");
  assert.equal(saveThemeSettings(defaultThemeSettings, null), false);
});

test("root application emits only validated colors and attributes and is safe without a DOM", () => {
  const target = themeTarget();
  const unsafe = { ...defaultThemeSettings, seed: "#123; background:url(x)", mode: "bad", surface: "bad", buttonStyle: "bad", opacity: -99, blur: Infinity };
  applyTheme(unsafe, target);
  assert.equal(target.attributes.get("data-theme"), "dark");
  assert.equal(target.attributes.get("data-surface"), "solid");
  assert.equal(target.attributes.get("data-button-style"), "soft");
  assert.equal(target.properties.get("color-scheme"), "dark");
  assert.equal(target.properties.get("--glass-opacity"), "0.68");
  assert.equal(target.properties.get("--glass-blur"), `${defaultThemeSettings.blur}px`);
  assert.equal(target.properties.get("--graph-8"), createThemeTokens(defaultThemeSettings)["--graph-8"]);
  assert.ok([...target.properties.values()].every((value) => !value.includes("url(")));
  assert.doesNotThrow(() => applyTheme(defaultThemeSettings));
  assert.doesNotThrow(() => initializeTheme(undefined, null));
});
