// Guards of the master / work-folder setup (docs/adr/0001).
import assert from "node:assert/strict";
import { test } from "node:test";

import {
  branchOf, claudeLocalSettings, folderOf, isInOneDrive, masterCargoConfig, masterRefusal, parseFolderNumber,
} from "../lib/work-folder.mjs";

test("OneDrive paths are recognised by segment", () => {
  assert.ok(isInOneDrive("C:\\Users\\dev\\<master>"));
  assert.ok(isInOneDrive("C:/Users/x/OneDrive - Company/repo"));
  assert.ok(!isInOneDrive("C:\\Users\\dev\\<work root>/loosescrews.win1"));
  assert.ok(!isInOneDrive("C:\\work\\NotOneDrive\\repo"));
});

test("the refusal names the work folders, only in OneDrive", () => {
  const msg = masterRefusal("C:\\Users\\dev\\<master>", "npm run dev");
  assert.match(msg, /npm run dev: refused/);
  assert.match(msg, /colent\.win1 \(branch WIN1/);
  assert.equal(masterRefusal("C:\\Users\\dev\\<work root>/loosescrews.win2", "x"), undefined);
});

test("names and numbers", () => {
  assert.equal(branchOf(3), "WIN3");
  assert.equal(folderOf(3), "loosescrews.win3");
  assert.equal(parseFolderNumber("12"), 12);
  for (const bad of ["0", "100", "x", undefined]) assert.throws(() => parseFolderNumber(bad));
});

test("master cargo config and local Claude settings", () => {
  assert.match(masterCargoConfig(".cargo\\refuse-build.cmd"), /rustc-wrapper = "\.cargo\/refuse-build\.cmd"/);
  const s = JSON.parse(claudeLocalSettings("C:\\w\\loosescrews.win2"));
  assert.equal(s.autoMemoryDirectory, "C:/w/loosescrews.win2/.claude/memory");
});
