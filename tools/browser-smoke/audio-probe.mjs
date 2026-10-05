// AudioContext probe for automated browsers (smoke test, M4 A6): a page
// script (pass it to `page.evaluateOnNewDocument`) that records every
// AudioContext the page creates and whether the page had seen a user gesture
// by then (`navigator.userActivation.hasBeenActive`). The game must not
// create one before the first click, key or touch.
//
//   window.__audioProbe.created: [{ activated: boolean | null, t: ms }]
export function audioProbe() {
  const probe = { created: [] };
  window.__audioProbe = probe;
  const record = () => probe.created.push({ activated: navigator.userActivation?.hasBeenActive ?? null, t: performance.now() });
  for (const name of ["AudioContext", "webkitAudioContext"]) {
    const Orig = window[name];
    if (typeof Orig !== "function") continue;
    window[name] = class extends Orig {
      constructor(...args) {
        record();
        super(...args);
      }
    };
  }
}
