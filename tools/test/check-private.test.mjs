// tools/check-private.mjs: private information never reaches tracked files.
// The fixtures are assembled at runtime so this file itself holds no user-home
// path, session link or session line; the private list is a made-up fixture.
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

import { HANDLE_KIND, LIST_KINDS, findListValues, findPrivate, loadPrivateList, mask } from "../check-private.mjs";
import { FIXTURE_LIST } from "./private-list-fixture.mjs";

const U = ["j", "doe"].join("");
const who = { email: `someone@${"example"}.org`, user: U };
const B = "\\";


test("absolute user-home paths are refused in every spelling", () => {
  const lines = [
    `C:${B}Users${B}${U}${B}repo`,
    `C:/Users/${U}/repo`,
    `cd /c/Users/${U}/x`,
    `p = "C:${B}${B}Users${B}${B}${U}${B}${B}x"`,
    `x /home/${U}/y`,
  ];
  for (const line of lines) assert.equal(findPrivate("f.md", line, who).length, 1, line);
});

test("placeholders and portable paths pass", () => {
  const lines = [`%USERPROFILE%${B}work${B}x`, "~/work/x", `C:${B}Users${B}dev${B}x`, "os.homedir()", "C:/Users/x/Some Folder/repo"];
  for (const line of lines) assert.deepEqual(findPrivate("f.md", line, who), [], line);
});

test("the git email and the OS user name are refused", () => {
  assert.equal(findPrivate("f.md", `mail ${who.email}`, who).length, 1);
  assert.equal(findPrivate("f.md", `D:${B}work${B}${U}${B}x`, who).length, 1);
  assert.deepEqual(findPrivate("f.md", `${U}not a path`, who), []);
});

test("session links and session lines are refused without a private list", () => {
  const ai = ["claude", "ai"].join(".");
  const lines = [
    `see https://${ai}/code/session_x`,
    `doc ${ai}/artifact/abc`,
    `${["Claude", "Session"].join("-")}: https://example.org/s`,
    `${["origin", "SessionId"].join("")}: 1234`,
  ];
  for (const line of lines) assert.equal(findPrivate("f.md", line, {}).length, 1, line);
  assert.deepEqual(findPrivate("f.md", `the ${ai} web app and anthropic.com`, {}), []);
});

test("private-list values are found as whole words, masked, with their kind", () => {
  const found = (text) => findListValues(text, FIXTURE_LIST);
  assert.deepEqual(found("by jane roe today"), [
    { kind: LIST_KINDS.names, masked: "j******e" },
    { kind: LIST_KINDS.nameParts, masked: "r*e" },
  ]);
  assert.deepEqual(found("Roebuck and Quillfeathers"), []);
  assert.ok(found("mail JANE.ROE@example.org").some((f) => f.kind === LIST_KINDS.emails));
  assert.equal(found("user jroe77")[0].kind, LIST_KINDS.accountNames);
  assert.equal(found("quillfeather notes")[0].kind, LIST_KINDS.projects);
  assert.equal(found("ZAP!")[0].kind, LIST_KINDS.caseSensitive);
  assert.deepEqual(found("zap it"), []);
  const findings = findPrivate("f.md", "by Jane Roe", { list: FIXTURE_LIST });
  assert.equal(findings.length, 2);
  for (const f of findings) assert.doesNotMatch(f, /Jane|Roe/);
  assert.match(findings[0], /^f\.md:1: contains a name from the private list \(J\*{6}e\)$/);
});

test("the handle is allowed only in its GitHub forms", () => {
  const ok = [
    "https://github.com/Octo99/LooseScrews/issues",
    "github.com/octo99",
    "12345+Octo99@users.noreply.github.com",
  ];
  for (const line of ok) assert.deepEqual(findListValues(line, FIXTURE_LIST), [], line);
  for (const line of ["made by Octo99", "octo99@example.org", "gitlab.com/Octo99/x"]) {
    assert.deepEqual(findListValues(line, FIXTURE_LIST).map((f) => f.kind), [HANDLE_KIND], line);
  }
  assert.deepEqual(findListValues("made by Octo99", FIXTURE_LIST, { handle: false }), []);
  assert.deepEqual(findListValues("Octo999", FIXTURE_LIST), []);
});

test("the public commit address is not the git email finding", () => {
  const email = FIXTURE_LIST.publicIdentity.email;
  assert.deepEqual(findPrivate("f.md", `mail ${email}`, { email, list: FIXTURE_LIST }), []);
});

test("masking keeps only the first and last character", () => {
  assert.equal(mask("abcdef"), "a****f");
  assert.equal(mask("ab"), "**");
  assert.equal(mask(""), "*");
});

test("the private list loads from $LS_PRIVATE_LIST or the home folder; missing means none", () => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "private-list-"));
  assert.equal(loadPrivateList({}, home), null);
  const dir = path.join(home, ".config", "loose-screws");
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "private-list.json"), JSON.stringify(FIXTURE_LIST));
  assert.deepEqual(loadPrivateList({}, home), FIXTURE_LIST);
  const other = path.join(home, "other.json");
  fs.writeFileSync(other, JSON.stringify({ names: ["X Y"] }));
  assert.deepEqual(loadPrivateList({ LS_PRIVATE_LIST: other }, home), { names: ["X Y"] });
  assert.equal(loadPrivateList({ LS_PRIVATE_LIST: path.join(home, "missing.json") }, home), null);
  fs.writeFileSync(other, "{ not json");
  assert.throws(() => loadPrivateList({ LS_PRIVATE_LIST: other }, home), /not valid JSON/);
  fs.rmSync(home, { recursive: true, force: true });
});
