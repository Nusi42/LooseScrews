#!/usr/bin/env node
// Renders the repository's social preview image (docs/media/social-preview.png, 1280x640, the
// size GitHub asks for) from the README banner's still frame over a gameplay still. Run it after
// the banner or the stills change, then upload the PNG by hand: repository Settings, General,
// Social preview.
//
//   cd tools/readme-media && npm ci && cd ../..
//   node tools/readme-media/social.mjs [--still <docs/media PNG>] [--out <path>] [--browser <path>]
//
// Headless only, with the fake pointer lock like every automated browser here.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

import puppeteer from "puppeteer-core";

import { fakePointerLock } from "../browser-smoke/fake-pointer-lock.mjs";
import { findBrowser, repoRoot } from "../browser-smoke/harness.mjs";

const MEDIA = path.join(repoRoot, "docs", "media");
const WIDTH = 1280;
const HEIGHT = 640;

/** The preview page: `still` (a docs/media PNG) under a shade, the banner in GitHub's safe zone, the URL. */
export function previewHtml(banner, still) {
  return `<!doctype html><html><head><meta charset="utf-8"><style>
  html,body{margin:0;width:${WIDTH}px;height:${HEIGHT}px;overflow:hidden;background:#0b0d12}
  .bg{position:absolute;inset:-6%;background:url(${still}) center/cover}
  .shade{position:absolute;inset:0;background:linear-gradient(180deg,rgba(4,5,10,.78) 0%,rgba(4,5,10,.35) 62%,rgba(4,5,10,.75) 100%)}
  .banner{position:absolute;left:80px;top:48px;width:1120px;height:350px;filter:drop-shadow(0 10px 30px rgba(0,0,0,.6))}
  .url{position:absolute;left:0;right:0;bottom:44px;text-align:center;color:#e6e8ee;
       font:600 24px/1 "Segoe UI",system-ui,sans-serif;letter-spacing:.4px;text-shadow:0 2px 10px #000}
</style></head><body>
  <div class="bg"></div><div class="shade"></div>
  <img class="banner" src="${banner}" alt="">
  <div class="url">github.com/Nusi42/LooseScrews</div>
</body></html>`;
}

const dataUrl = (file, type) => `data:${type};base64,${fs.readFileSync(file).toString("base64")}`;

async function main() {
  const { values } = parseArgs({
    options: {
      still: { type: "string", default: "open-sky-dark.png" },
      out: { type: "string", default: path.join(MEDIA, "social-preview.png") },
      browser: { type: "string" },
    },
  });
  const executablePath = findBrowser(values.browser ?? process.env.LS_BROWSER);
  if (!executablePath) throw new Error("no Chrome or Edge found (set LS_BROWSER or pass --browser)");
  const html = previewHtml(dataUrl(path.join(MEDIA, "banner.svg"), "image/svg+xml"), dataUrl(path.join(MEDIA, values.still), "image/png"));
  const browser = await puppeteer.launch({ executablePath, headless: true, args: ["--hide-scrollbars"] });
  try {
    const page = await browser.newPage();
    await page.evaluateOnNewDocument(fakePointerLock);
    // The banner's still frame: its animations stop under reduced motion.
    await page.emulateMediaFeatures([{ name: "prefers-reduced-motion", value: "reduce" }]);
    await page.setViewport({ width: WIDTH, height: HEIGHT, deviceScaleFactor: 1 });
    await page.setContent(html, { waitUntil: "load" });
    await page.screenshot({ path: values.out, type: "png" });
    console.log(`social preview: ${values.out} (${WIDTH}x${HEIGHT}, ${fs.statSync(values.out).size} bytes)`);
  } finally {
    await browser.close();
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}
