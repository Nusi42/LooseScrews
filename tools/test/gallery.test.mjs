// tools/browser-smoke/gallery.mjs: finding the integrated GPU's LUID in chrome://gpu (M4 D5).
import assert from "node:assert/strict";
import { test } from "node:test";

import { integratedLuidFromGpuPage } from "../browser-smoke/gallery.mjs";

// The parts of chrome://gpu the lookup reads (text as the page's shadow DOM walk yields it).
const PAGE = `
GPU0                             : VENDOR= 0x10de, DEVICE=0x2c02 [NVIDIA GeForce RTX 5080], SUBSYS=0x1435196e, REV=161, LUID={0,98334}, DRIVER_VENDOR=NVIDIA, DRIVER_VERSION=32.0.16.1062 *ACTIVE*
GPU1                             : VENDOR= 0x1002, DEVICE=0x13c0 [AMD Radeon(TM) Graphics], SUBSYS=0xd0001458, REV=201, LUID={0,104133}, DRIVER_VERSION=32.0.21036.18
GPU2                             : VENDOR= 0x1414, DEVICE=0x008c [Microsoft Basic Render Driver], LUID={0,104067}, DRIVER_VERSION=10.0.26100.6725
Dawn Info
=========
<Discrete GPU> D3D12 backend - NVIDIA GeForce RTX 5080
-------------------------------------------------------
[WebGPU Status]
<Integrated GPU> D3D12 backend - AMD Radeon(TM) Graphics
---------------------------------------------------------
[WebGPU Status]
* Available
`;

test("the integrated GPU's LUID comes from its GPUn line, matched by Dawn's adapter name", () => {
  assert.equal(integratedLuidFromGpuPage(PAGE), "0,104133");
});

test("no integrated GPU in Dawn's list: no LUID", () => {
  assert.equal(integratedLuidFromGpuPage(PAGE.replace("<Integrated GPU> D3D12 backend - AMD Radeon(TM) Graphics", "")), null);
  assert.equal(integratedLuidFromGpuPage(""), null);
});
