import assert from "node:assert/strict";
import test from "node:test";
import { createLocaleStore, defaultLocale, intlLocale, localeStorageKey, translate } from "./i18n.ts";
import { appMessages } from "./locales/app.ts";
import { controlsMessages } from "./locales/controls.ts";
import { diffHistoryMessages } from "./locales/diffHistory.ts";

function memoryStorage(initial) {
  const values = new Map(initial === undefined ? [] : [[localeStorageKey, initial]]);
  return { getItem: (key) => values.get(key) ?? null, setItem: (key, value) => values.set(key, value) };
}

test("language preference survives restart and updates the document language", () => {
  const storage = memoryStorage();
  const root = { lang: "" };
  const store = createLocaleStore();
  store.initialize(storage, root);
  assert.equal(store.getSnapshot().locale, defaultLocale);
  assert.equal(root.lang, "tr");
  assert.equal(store.setLocale("en"), true);
  assert.equal(storage.getItem(localeStorageKey), "en");
  assert.equal(root.lang, "en");
  const restarted = createLocaleStore();
  restarted.initialize(storage, root);
  assert.deepEqual(restarted.getSnapshot(), { locale: "en", saved: true });
});

test("unsupported or malformed saved preferences use Turkish without rewriting storage", () => {
  for (const value of ["de", "EN", '{"locale":"en"}', "", null]) {
    const storage = memoryStorage(value);
    const store = createLocaleStore();
    store.initialize(storage);
    assert.equal(store.getSnapshot().locale, "tr");
    assert.equal(storage.getItem(localeStorageKey), value);
    assert.equal(store.setLocale("unsupported"), false);
    assert.equal(storage.getItem(localeStorageKey), value);
  }
});

test("blocked storage preserves language switching for this session", () => {
  const store = createLocaleStore();
  store.initialize({ getItem() { throw new Error("blocked"); }, setItem() { throw new Error("blocked"); } });
  assert.deepEqual(store.getSnapshot(), { locale: "tr", saved: false });
  assert.equal(store.setLocale("en"), false);
  assert.deepEqual(store.getSnapshot(), { locale: "en", saved: false });
});

test("external-store snapshots remain stable between changes and unsubscribe cleanly", () => {
  const store = createLocaleStore();
  store.initialize(memoryStorage());
  const before = store.getSnapshot();
  let notifications = 0;
  const unsubscribe = store.subscribe(() => { notifications++; });
  store.setLocale("tr");
  assert.equal(store.getSnapshot(), before);
  assert.equal(notifications, 0);
  store.setLocale("en");
  assert.equal(notifications, 1);
  unsubscribe();
  store.setLocale("tr");
  assert.equal(notifications, 1);
});

test("translations interpolate data literally and locale formatting follows the selection", () => {
  const catalog = { result: { tr: "{count} dosya · {path}", en: "{count} files · {path}" } };
  const params = { count: 2, path: "example/{name}<$&>" };
  assert.equal(translate(catalog, "en", "result", params), "2 files · example/{name}<$&>");
  assert.equal(translate(catalog, "tr", "result", params), "2 dosya · example/{name}<$&>");
  assert.equal(intlLocale("tr"), "tr-TR");
  assert.equal(intlLocale("en"), "en-US");
});

test("all interface messages provide both languages with matching interpolation parameters", () => {
  const parameters = (text) => [...new Set([...text.matchAll(/\{([a-zA-Z][a-zA-Z0-9]*)\}/g)].map((match) => match[1]))].sort();
  for (const [section, catalog] of Object.entries({ appMessages, controlsMessages, diffHistoryMessages })) {
    for (const [key, message] of Object.entries(catalog)) {
      assert.ok(message.tr.trim(), `${section}.${key}: Turkish missing`);
      assert.ok(message.en.trim(), `${section}.${key}: English missing`);
      assert.deepEqual(parameters(message.tr), parameters(message.en), `${section}.${key}: interpolation differs`);
    }
  }
});
