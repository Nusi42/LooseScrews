// Checkout roles and work-folder names (docs/adr/0001).
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

import {
  DEFAULT_PREFIX, branchOf, checkoutRole, claudeLocalSettings, envValue, folderOf, folderPrefix, isMaster,
  masterCargoConfig, masterRefusal, MASTER_RUSTC_WRAPPER, parseFolderNumber, withEnvValue, workFolderNumber,
} from "../lib/work-folder.mjs";

function checkout(envLocal) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "role-"));
  if (envLocal !== undefined) fs.writeFileSync(path.join(dir, ".env.local"), envLocal);
  return dir;
}

test("the role comes from .env.local; the environment variable overrides it", () => {
  const master = checkout("# local\nLS_CHECKOUT_ROLE=master\n");
  const plain = checkout();
  assert.equal(checkoutRole(master, {}), "master");
  assert.ok(isMaster(master, {}));
  assert.ok(!isMaster(plain, {}));
  assert.ok(!isMaster(checkout("OTHER=1\n"), {}));
  assert.ok(!isMaster(master, { LS_CHECKOUT_ROLE: "work" }));
  assert.ok(isMaster(plain, { LS_CHECKOUT_ROLE: "master" }));
  assert.ok(isMaster(checkout('LS_CHECKOUT_ROLE="master"\r\n'), {}));
});

test("the refusal is neutral and only in the master", () => {
  const msg = masterRefusal(checkout("LS_CHECKOUT_ROLE=master\n"), "npm run dev", {});
  assert.match(msg, /npm run dev: refused/);
  assert.match(msg, /<work root>\/loosescrews\.win1 \(branch WIN1/);
  assert.doesNotMatch(`${msg}\n${MASTER_RUSTC_WRAPPER}`, /onedrive|users|home/i);
  assert.equal(masterRefusal(checkout(), "x", {}), undefined);
});

test(".env.local values are read and replaced in place", () => {
  assert.equal(envValue("A=1\nexport B = 'two'\nC=3 # note\n", "B"), "two");
  assert.equal(envValue("C=3 # note", "C"), "3");
  assert.equal(envValue("A=1", "B"), undefined);
  assert.equal(withEnvValue("", "K", "v"), "K=v\n");
  assert.equal(withEnvValue("A=1\nK=old\n", "K", "v"), "A=1\nK=v\n");
  assert.equal(withEnvValue("A=1", "K", "v"), "A=1\nK=v\n");
});

test("the folder prefix comes from the checkout's name", () => {
  assert.equal(folderPrefix(path.join("w", "oldname.win1")), "oldname");
  assert.equal(folderPrefix(path.join("w", "loosescrews.win12")), "loosescrews");
  assert.equal(folderPrefix(path.join("w", "LooseScrews")), DEFAULT_PREFIX);
  assert.equal(workFolderNumber("oldname.win10", "oldname"), 10);
  assert.equal(workFolderNumber("other.win1", "oldname"), undefined);
  assert.equal(workFolderNumber("oldname.master-backup", "oldname"), undefined);
});

test("names and numbers", () => {
  assert.equal(branchOf(3), "WIN3");
  assert.equal(folderOf(3), "loosescrews.win3");
  assert.equal(folderOf(3, "x"), "x.win3");
  assert.equal(parseFolderNumber("12"), 12);
  for (const bad of ["0", "100", "x", undefined]) assert.throws(() => parseFolderNumber(bad));
});

test("master cargo config and local Claude settings", () => {
  assert.match(masterCargoConfig(".cargo\\refuse-build.cmd"), /rustc-wrapper = "\.cargo\/refuse-build\.cmd"/);
  const s = JSON.parse(claudeLocalSettings("C:\\w\\loosescrews.win2"));
  assert.equal(s.autoMemoryDirectory, "C:/w/loosescrews.win2/.claude/memory");
});
