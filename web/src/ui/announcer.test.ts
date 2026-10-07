// Announcer queue (D2): a newer event line replaces an older one still
// waiting, and fired timers are forgotten. Runs on a tiny fake DOM (the
// caption only needs elements, classes and styles) with mocked timers.

import assert from "node:assert/strict";
import { mock, test } from "node:test";
import { fillLine, linesFor } from "../audio/announcer/lines.ts";
import { PACKS } from "../audio/announcer/packs.ts";
import { Announcer } from "./announcer.ts";

class FakeEl {
  className = "";
  innerHTML = "";
  textContent = "";
  offsetWidth = 0;
  readonly dataset: Record<string, string> = {};
  readonly children: FakeEl[] = [];
  readonly classList = {
    set: new Set<string>(),
    add: (...c: string[]) => c.forEach((x) => this.classList.set.add(x)),
    remove: (...c: string[]) => c.forEach((x) => this.classList.set.delete(x)),
    contains: (c: string) => this.classList.set.has(c),
  };
  readonly style = { setProperty: () => {}, animationDelay: "" };
  setAttribute(): void {}
  appendChild<T>(c: T): T {
    if (c instanceof FakeEl) this.children.push(c);
    return c;
  }
  querySelector(): FakeEl {
    return new FakeEl();
  }
  remove(): void {}
}

function withFakeDom(run: () => void): void {
  const g = globalThis as unknown as { document?: unknown };
  const before = g.document;
  g.document = { createElement: () => new FakeEl(), createTextNode: () => new FakeEl() };
  try {
    run();
  } finally {
    g.document = before;
  }
}

test("announcer: a newer event line replaces an older one still waiting; timers do not pile up", () => {
  mock.timers.enable({ apis: ["setTimeout"] });
  try {
    withFakeDom(() => {
      const pack = PACKS[0] ?? assert.fail("no voice pack");
      const a = new Announcer(new FakeEl() as unknown as HTMLElement, null, pack, () => 0);
      a.say("matchStart");
      // While the first line plays, two events are announced back to back.
      a.say("event.duck_parade", { target: "Rivet" });
      a.say("event.hat_rain", { target: "Rivet" });
      for (let k = 0; k < 400; k++) mock.timers.tick(100);
      assert.equal(a.said.length, 2, `said: ${a.said.join(" | ")}`);
      const hat = linesFor(pack, "event.hat_rain").map((l) => fillLine(l, { target: "Rivet" }));
      assert.ok(hat.includes(a.said[1] ?? ""), `the newest event's line: ${a.said[1]}`);
      const timers = (a as unknown as { timers: unknown[] }).timers;
      assert.equal(timers.length, 0, "fired timers are forgotten");
      a.dispose();
    });
  } finally {
    mock.timers.reset();
  }
});
