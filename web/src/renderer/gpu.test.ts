// Device watching (renderer/gpu.ts): a renderer on a shared device must not
// leave its error listener on the device once destroyed.

import assert from "node:assert/strict";
import { test } from "node:test";
import { watchDevice } from "./gpu.ts";

class FakeDevice extends EventTarget {
  private loseIt: (info: GPUDeviceLostInfo) => void = () => {};
  readonly lost: Promise<GPUDeviceLostInfo> = new Promise((resolve) => {
    this.loseIt = resolve;
  });
  lose(): void {
    this.loseIt({ reason: "destroyed", message: "gone" } as GPUDeviceLostInfo);
  }
  error(message: string): void {
    const e = new Event("uncapturederror") as Event & { error: { message: string } };
    e.error = { message };
    this.dispatchEvent(e);
  }
}

test("a stopped device watch no longer hears the shared device's errors", () => {
  const device = new FakeDevice();
  const game: string[] = [];
  const bay: string[] = [];
  watchDevice(device as unknown as GPUDevice, (m) => game.push(m));
  const bayWatch = watchDevice(device as unknown as GPUDevice, (m) => bay.push(m));
  device.error("first");
  bayWatch.stop(); // the ship bay's renderer was destroyed
  device.error("second");
  assert.deepEqual(game, ["first", "second"]);
  assert.deepEqual(bay, ["first"]);
});

test("a device watch reports the loss", async () => {
  const device = new FakeDevice();
  const watch = watchDevice(device as unknown as GPUDevice, () => {});
  assert.equal(watch.lost, false);
  device.lose();
  await device.lost;
  await Promise.resolve();
  assert.equal(watch.lost, true);
});
