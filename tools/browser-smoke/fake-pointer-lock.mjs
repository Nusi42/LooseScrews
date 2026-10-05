// Fake Pointer Lock for automated browsers (smoke test, screenshot gallery).
//
// On Windows even headless Chrome creates hidden native windows, and a real
// pointer lock there hides and clips the user's OS cursor while the test runs.
// This page script (pass it to `page.evaluateOnNewDocument`) replaces the
// Pointer Lock API with an in-page imitation: `requestPointerLock` sets
// `document.pointerLockElement` and fires `pointerlockchange`, `exitPointerLock`
// clears it. Mouse events from puppeteer keep their `movementX/Y`, so the game
// reads mouse look exactly as under a real lock, and the OS cursor stays free.
export function fakePointerLock() {
  let locked = null;
  const change = () => document.dispatchEvent(new Event("pointerlockchange"));
  Object.defineProperty(Document.prototype, "pointerLockElement", {
    configurable: true,
    get: () => locked,
  });
  Element.prototype.requestPointerLock = function () {
    locked = this;
    queueMicrotask(change);
    return Promise.resolve();
  };
  Document.prototype.exitPointerLock = function () {
    if (!locked) return;
    locked = null;
    queueMicrotask(change);
  };
}
