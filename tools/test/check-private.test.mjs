// tools/check-private.mjs: private information never reaches tracked files.
// The fixtures are assembled at runtime so this file itself holds no user-home path.
import assert from "node:assert/strict";
import { test } from "node:test";

import { findPrivate } from "../check-private.mjs";

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
  const lines = [`%USERPROFILE%${B}OneDrive${B}x`, "~/OneDrive/x", `C:${B}Users${B}dev${B}x`, "os.homedir()", "C:/Users/x/OneDrive - Company/repo"];
  for (const line of lines) assert.deepEqual(findPrivate("f.md", line, who), [], line);
});

test("the git email and the OS user name are refused", () => {
  assert.equal(findPrivate("f.md", `mail ${who.email}`, who).length, 1);
  assert.equal(findPrivate("f.md", `D:${B}work${B}${U}${B}x`, who).length, 1);
  assert.deepEqual(findPrivate("f.md", `${U}not a path`, who), []);
});
