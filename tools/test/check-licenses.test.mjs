// The npm dependency license gate (tools/check-licenses.mjs).
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { LOCKFILES, expressionOk, licenseExpression, lockfileProblems, packageName } from "../check-licenses.mjs";
import { REPO_ROOT } from "../lib/work-folder.mjs";

test("an expression passes when one option avoids GPL, LGPL and AGPL", () => {
  for (const ok of ["MIT", "(MIT OR GPL-3.0)", "GPL-2.0 OR Apache-2.0", "MPL-2.0", "Apache-2.0 WITH LLVM-exception", "(MIT AND Zlib) OR LGPL-2.1-or-later", "mit or gpl-3.0"]) {
    assert.equal(expressionOk(ok), true, ok);
  }
  for (const bad of ["GPL-3.0", "AGPL-3.0-or-later", "LGPL-2.1+", "GPL-2.0 WITH Classpath-exception-2.0", "MIT AND LGPL-3.0-only", "(GPL-2.0 OR LGPL-3.0) AND MIT", "GPLv3"]) {
    assert.equal(expressionOk(bad), false, bad);
  }
});

test("malformed expressions throw", () => {
  for (const bad of ["(MIT", "MIT OR", "OR MIT", "MIT)", "MIT Apache-2.0"]) assert.throws(() => expressionOk(bad), undefined, bad);
});

test("license fields: strings, objects and arrays; missing and unlicensed give null", () => {
  assert.equal(licenseExpression({ license: "MIT" }), "MIT");
  assert.equal(licenseExpression({ license: { type: "ISC" } }), "ISC");
  assert.equal(licenseExpression({ licenses: ["MIT", { type: "GPL-2.0" }] }), "(MIT) OR (GPL-2.0)");
  for (const none of [{}, { license: "" }, { license: "UNLICENSED" }, { license: "SEE LICENSE IN LICENSE.txt" }, { licenses: [] }]) {
    assert.equal(licenseExpression(none), null, JSON.stringify(none));
  }
});

test("package names come from the lockfile path or its name field", () => {
  assert.equal(packageName("node_modules/a/node_modules/@s/b", {}), "@s/b");
  assert.equal(packageName("node_modules/alias", { name: "real" }), "real");
});

test("lockfile problems: copyleft-only and missing fail, the root and reviewed packages are skipped", () => {
  const lock = {
    packages: {
      "": { name: "root", license: "AGPL-3.0-or-later" },
      "node_modules/ok": { version: "1.0.0", license: "(MIT OR GPL-3.0)" },
      "node_modules/gpl": { version: "2.0.0", license: "GPL-3.0" },
      "node_modules/none": { version: "3.0.0" },
      "node_modules/known": { version: "4.0.0" },
      "node_modules/broken": { version: "5.0.0", license: "(MIT" },
      "../linked": { link: true },
    },
  };
  const problems = lockfileProblems("x/package-lock.json", lock, { known: "reviewed: MIT in its LICENSE file" });
  assert.deepEqual(
    problems.map((p) => p.split(":")[1].trim()),
    ["gpl@2.0.0", "none@3.0.0", "broken@5.0.0"],
  );
  assert.match(problems[0], /"GPL-3.0" has no option outside GPL, LGPL and AGPL/);
  assert.match(problems[1], /no license/);
});

test("the repository's lockfiles pass", () => {
  for (const f of LOCKFILES) {
    assert.deepEqual(lockfileProblems(f, JSON.parse(fs.readFileSync(path.join(REPO_ROOT, f), "utf8"))), [], f);
  }
});
