import assert from "node:assert/strict";
import test from "node:test";
import { localizeBackendText } from "./localizeBackendText.ts";

test("absent reasons stay absent and an empty diagnostic stays empty", () => {
  for (const locale of ["tr", "en"]) {
    assert.equal(localizeBackendText(null, locale), null);
    assert.equal(localizeBackendText(undefined, locale), null);
    assert.equal(localizeBackendText("", locale), "");
  }
});

test("known remote and partial-selection explanations follow the selected language", () => {
  const cases = [
    ["This remote has multiple fetch or push URLs. Edit it in Git to preserve every destination.", "Bu uzak deponun birden fazla Fetch veya Push adresi var. Tüm hedefleri korumak için Git üzerinden düzenleyin."],
    ["The repository configuration is read-only. Make it writable in Git before editing remote settings.", "Depo yapılandırması salt okunur. Uzak depo ayarlarını düzenlemeden önce Git üzerinden yazılabilir duruma getirin."],
    ["Binary files do not support individual-line actions.", "İkili dosyalarda tek tek satır işlemleri desteklenmiyor."],
    ["This file exceeds the 4 MiB partial-selection limit. Use the whole-file action.", "Bu dosya kısmi seçim için 4 MiB sınırını aşıyor. Dosyanın tamamı için işlem yapın."],
  ];
  for (const [original, translated] of cases) {
    assert.equal(localizeBackendText(original, "tr"), translated);
    assert.equal(localizeBackendText(original, "en"), original);
  }
});

test("unknown and decorated diagnostics pass through without substring translation", () => {
  const known = "Binary files do not support individual-line actions.";
  for (const value of [
    "fatal: could not read from remote repository",
    "credential helper: kullanıcı adı alınamadı\r\n",
    `UNSUPPORTED: ${known}`,
    ` ${known}`,
    `${known}\n`,
    `${known}\npath: src/example.ts`,
    known.toLowerCase(),
    "__proto__",
    "constructor",
  ]) {
    assert.equal(localizeBackendText(value, "tr"), value);
    assert.equal(localizeBackendText(value, "en"), value);
  }
});
