import assert from "node:assert/strict";
import test from "node:test";
import { repositoryIdentity } from "./repositoryIdentity.ts";

test("absolute Windows drive aliases share one repository identity", () => {
  const expected = "c:/example/example";
  for (const path of [
    String.raw`C:\example\Example`,
    "c:/EXAMPLE/EXAMPLE/",
    String.raw`\\?\C:\example\Example`,
    "//?/C:/example/Example/",
    "C:\\example/Example\\",
    "C://example///Example//",
  ]) {
    assert.equal(repositoryIdentity(path), expected, path);
    assert.equal(repositoryIdentity(repositoryIdentity(path)), expected, path);
  }
});

test("UNC and verbatim UNC paths share identities across separators and case", () => {
  const expected = "//server/share/team repo";
  for (const path of [
    String.raw`\\Server\Share\Team Repo`,
    "//SERVER/SHARE/Team Repo/",
    String.raw`\\?\UNC\Server\Share\Team Repo`,
    String.raw`\\?\unc\SERVER\SHARE\TEAM REPO`,
    "//?/UNC/Server/Share/Team Repo/",
    "\\\\Server\\Share/Team Repo\\",
    "//Server//Share///Team Repo//",
  ]) {
    assert.equal(repositoryIdentity(path), expected, path);
    assert.equal(repositoryIdentity(repositoryIdentity(path)), expected, path);
  }
});

test("drive and UNC roots retain their absolute identity", () => {
  for (const path of ["C:/", "c:\\", "C:////", "\\\\?\\C:\\"]) {
    assert.equal(repositoryIdentity(path), "c:/", path);
  }
  assert.notEqual(repositoryIdentity("C:/"), repositoryIdentity("c:"));
  for (const path of ["//Server/Share", "\\\\Server\\Share\\", "\\\\?\\UNC\\Server\\Share\\"]) {
    assert.equal(repositoryIdentity(path), "//server/share", path);
  }
});

test("different worktrees and network shares retain separate identities", () => {
  assert.notEqual(repositoryIdentity("C:/projects/repo"), repositoryIdentity("C:/projects/repo-feature"));
  assert.notEqual(repositoryIdentity("C:/projects/repo"), repositoryIdentity("D:/projects/repo"));
  assert.notEqual(repositoryIdentity("//server/main/repo"), repositoryIdentity("//server/feature/repo"));
});

test("POSIX paths preserve case and literal backslashes", () => {
  for (const path of ["/", "/home/Example/Example", "/home/Example/Example/", String.raw`/home/Example/Example\name`]) {
    assert.equal(repositoryIdentity(path), path);
  }
  assert.notEqual(repositoryIdentity("/home/Example/Example"), repositoryIdentity("/home/example/example"));
});

test("identity normalization does not resolve relative paths, dot segments or unknown namespaces", () => {
  for (const path of ["", "C:Repo", String.raw`repo\Child`, String.raw`\Repo`, String.raw`\\.\C:\Repo`, String.raw`\\?\Volume{123}\Repo`]) {
    assert.equal(repositoryIdentity(path), path);
  }
  assert.equal(repositoryIdentity(String.raw`C:\Repo\..\Other`), "c:/repo/../other");
  assert.notEqual(repositoryIdentity(String.raw`C:\Repo\..\Other`), repositoryIdentity("C:/Other"));
  assert.equal(repositoryIdentity("C:/Repo/."), "c:/repo/.");
  assert.equal(repositoryIdentity("C:/Repo/Trailing space "), "c:/repo/trailing space ");
});
