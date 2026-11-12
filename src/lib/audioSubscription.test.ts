import { afterEach, beforeEach, expect, test, vi } from "vitest";

const callMock = vi.fn();

vi.mock("./ipc", () => ({
  inTauri: () => true,
  call: (...args: unknown[]) => callMock(...args),
}));

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage: (message: unknown) => void = () => {};
  },
}));

beforeEach(() => {
  vi.resetModules();
  callMock.mockReset();
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

test("a failed channel subscription is retried and then kept", async () => {
  callMock.mockRejectedValueOnce(new Error("not ready")).mockResolvedValue(undefined);
  const { onPlaybackEvent, SUBSCRIBE_RETRY_MS } = await import("./audio");

  const stop = onPlaybackEvent(() => {});
  await vi.advanceTimersByTimeAsync(0);
  expect(callMock).toHaveBeenCalledTimes(1);

  await vi.advanceTimersByTimeAsync(SUBSCRIBE_RETRY_MS);
  expect(callMock).toHaveBeenCalledTimes(2);

  // Subscribed now: more listeners share the channel, no further calls.
  onPlaybackEvent(() => {});
  await vi.advanceTimersByTimeAsync(SUBSCRIBE_RETRY_MS * 3);
  expect(callMock).toHaveBeenCalledTimes(2);
  stop();
});

test("retries stop once every listener is gone", async () => {
  callMock.mockRejectedValue(new Error("down"));
  const { onPlaybackEvent, SUBSCRIBE_RETRY_MS } = await import("./audio");

  const stop = onPlaybackEvent(() => {});
  await vi.advanceTimersByTimeAsync(0);
  stop();
  await vi.advanceTimersByTimeAsync(SUBSCRIBE_RETRY_MS * 5);
  expect(callMock).toHaveBeenCalledTimes(1);
});

test("subscribeAsync releases a listener that resolves after stop", async () => {
  const { subscribeAsync } = await import("./audio");
  let resolve: (unlisten: () => void) => void = () => {};
  const unlisten = vi.fn();
  const stop = subscribeAsync(() => new Promise((done) => (resolve = done)));

  stop();
  resolve(unlisten);
  await vi.advanceTimersByTimeAsync(0);
  expect(unlisten).toHaveBeenCalledTimes(1);
});

test("subscribeAsync releases on stop and reports failures", async () => {
  const { subscribeAsync } = await import("./audio");
  const unlisten = vi.fn();
  const stop = subscribeAsync(() => Promise.resolve(unlisten));
  await vi.advanceTimersByTimeAsync(0);
  expect(unlisten).not.toHaveBeenCalled();
  stop();
  expect(unlisten).toHaveBeenCalledTimes(1);

  const onError = vi.fn();
  subscribeAsync(() => Promise.reject(new Error("no ipc")), onError);
  await vi.advanceTimersByTimeAsync(0);
  expect(onError).toHaveBeenCalledTimes(1);
});
