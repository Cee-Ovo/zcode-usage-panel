import { describe, expect, it } from "vitest";
import { createDetailGate } from "../src/lib/modelDetail";
import type { ModelDetailDto } from "../src/lib/types";

/** 可手动 resolve 的 promise,用来制造乱序返回。 */
function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const detail = (name: string) => ({ name }) as unknown as ModelDetailDto;

describe("createDetailGate", () => {
  it("drops a stale response that resolves after a newer request", async () => {
    const applied: (ModelDetailDto | null)[] = [];
    const gate = createDetailGate((d) => applied.push(d));

    const first = deferred<ModelDetailDto>();
    const second = deferred<ModelDetailDto>();
    gate.open(() => first.promise);
    gate.open(() => second.promise);

    // 后发起的先返回,先发起的后返回 —— 旧响应必须被丢弃。
    second.resolve(detail("B"));
    await second.promise;
    await Promise.resolve();
    first.resolve(detail("A"));
    await first.promise;
    await Promise.resolve();

    expect(applied).toEqual([detail("B")]);
  });

  it("close() invalidates an in-flight request so it cannot reopen the dialog", async () => {
    const applied: (ModelDetailDto | null)[] = [];
    const gate = createDetailGate((d) => applied.push(d));

    const pending = deferred<ModelDetailDto>();
    gate.open(() => pending.promise);
    gate.close();
    pending.resolve(detail("A"));
    await pending.promise;
    await Promise.resolve();

    // 只有 close 写入的 null,迟到的详情不再把弹窗打开。
    expect(applied).toEqual([null]);
  });

  it("reports failures only while the request is still current", async () => {
    const applied: (ModelDetailDto | null)[] = [];
    const errors: number[] = [];
    const gate = createDetailGate((d) => applied.push(d));

    const stale = deferred<ModelDetailDto>();
    gate.open(() => stale.promise, () => errors.push(1));
    const fresh = deferred<ModelDetailDto>();
    gate.open(() => fresh.promise, () => errors.push(2));

    stale.reject(new Error("boom"));
    await stale.promise.catch(() => {});
    await Promise.resolve();
    expect(errors).toEqual([]); // 过期请求的失败不该打扰用户

    fresh.reject(new Error("boom"));
    await fresh.promise.catch(() => {});
    await Promise.resolve();
    expect(errors).toEqual([2]);
    expect(applied).toEqual([]);
  });
});