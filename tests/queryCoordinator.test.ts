import { describe, expect, it, vi } from "vitest";
import { createQueryCoordinator, type QueryRequest } from "../src/lib/queryCoordinator";

type Page = "dashboard" | "models";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const request = (page: Page, rangeKey: string, visible = true): QueryRequest<Page> => ({
  page,
  rangeKey,
  visible,
});

describe("QueryCoordinator", () => {
  it("applies only the newest range when responses resolve out of order", async () => {
    const flights: Array<ReturnType<typeof deferred<string>>> = [];
    const applied: string[] = [];
    const coordinator = createQueryCoordinator<Page, string>({
      fetch: vi.fn(() => {
        const flight = deferred<string>();
        flights.push(flight);
        return flight.promise;
      }),
      apply: (value) => applied.push(value),
    });

    coordinator.request(request("dashboard", "today"));
    coordinator.request(request("dashboard", "30d"));
    flights[0].resolve("today-result");
    await Promise.resolve();
    expect(flights).toHaveLength(2);
    flights[1].resolve("30d-result");
    await coordinator.whenIdle();

    expect(applied).toEqual(["30d-result"]);
    expect(coordinator.getState().lastSuccessRequest?.rangeKey).toBe("30d");
  });

  it("coalesces a burst to one pending request and preserves single-flight", async () => {
    const flights: Array<ReturnType<typeof deferred<number>>> = [];
    const fetch = vi.fn(() => {
      const flight = deferred<number>();
      flights.push(flight);
      return flight.promise;
    });
    const applied: number[] = [];
    const coordinator = createQueryCoordinator<Page, number>({ fetch, apply: (n) => applied.push(n) });

    coordinator.request(request("dashboard", "today"));
    coordinator.request(request("dashboard", "7d"));
    coordinator.request(request("dashboard", "30d"));
    coordinator.request(request("dashboard", "all"));
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(coordinator.getState().pending).toBe(true);

    flights[0].resolve(1);
    await Promise.resolve();
    expect(fetch).toHaveBeenCalledTimes(2);
    flights[1].resolve(2);
    await coordinator.whenIdle();
    expect(applied).toEqual([2]);
  });

  it("reports current errors, supports retry, and does not clear existing data", async () => {
    const first = deferred<string>();
    const second = deferred<string>();
    const fetch = vi
      .fn<(_: QueryRequest<Page>) => Promise<string>>()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);
    const applied: string[] = [];
    const coordinator = createQueryCoordinator<Page, string>({ fetch, apply: (v) => applied.push(v) });

    coordinator.request(request("dashboard", "today"));
    first.reject(new Error("backend unavailable"));
    await coordinator.whenIdle();
    expect(coordinator.getState().error).toBe("刷新失败，请稍后重试");
    expect(applied).toEqual([]);

    coordinator.request(request("dashboard", "today"));
    second.resolve("fresh");
    await coordinator.whenIdle();
    expect(applied).toEqual(["fresh"]);
    expect(coordinator.getState().error).toBeNull();
    expect(coordinator.getState().lastSuccessMs).not.toBeNull();
  });

  it("does not apply a late result after disposal", async () => {
    const flight = deferred<string>();
    const apply = vi.fn();
    const coordinator = createQueryCoordinator<Page, string>({
      fetch: () => flight.promise,
      apply,
    });

    coordinator.request(request("dashboard", "today"));
    coordinator.dispose();
    flight.resolve("late");
    await coordinator.whenIdle();
    await Promise.resolve();
    expect(apply).not.toHaveBeenCalled();
    expect(coordinator.getState().loading).toBe(false);
  });

  it("allows a fresh coordinator after a disposed StrictMode-like lifecycle", async () => {
    const first = deferred<string>();
    const second = deferred<string>();
    const apply = vi.fn();
    const make = (flight: ReturnType<typeof deferred<string>>) =>
      createQueryCoordinator<Page, string>({
        fetch: () => flight.promise,
        apply,
      });

    const firstCoordinator = make(first);
    firstCoordinator.request(request("dashboard", "today"));
    firstCoordinator.dispose();

    const secondCoordinator = make(second);
    secondCoordinator.request(request("dashboard", "today"));
    second.resolve("fresh lifecycle");
    await secondCoordinator.whenIdle();
    first.resolve("stale lifecycle");
    await Promise.resolve();

    expect(apply).toHaveBeenCalledTimes(1);
    expect(apply).toHaveBeenCalledWith("fresh lifecycle", expect.objectContaining({ rangeKey: "today" }));
  });

  it("coalesces same-range refreshes without starving completed snapshots", async () => {
    const flights: Array<ReturnType<typeof deferred<number>>> = [];
    const fetch = vi.fn(() => {
      const flight = deferred<number>();
      flights.push(flight);
      return flight.promise;
    });
    const applied: number[] = [];
    const coordinator = createQueryCoordinator<Page, number>({ fetch, apply: (n) => applied.push(n) });

    coordinator.request(request("dashboard", "today"));
    coordinator.request(request("dashboard", "today"));
    coordinator.request(request("dashboard", "today"));
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(coordinator.getState().pending).toBe(true);
    flights[0].resolve(1);
    await Promise.resolve();
    expect(fetch).toHaveBeenCalledTimes(2);
    expect(applied).toEqual([1]);
    flights[1].resolve(2);
    await coordinator.whenIdle();
    expect(applied).toEqual([1, 2]);
  });

  it("keeps page selection and avoids hidden-window fetches", async () => {
    const fetch = vi.fn(async (req: QueryRequest<Page>) => req.page);
    const applied: string[] = [];
    const coordinator = createQueryCoordinator<Page, Page>({
      fetch,
      apply: (page) => applied.push(page),
    });

    await coordinator.request(request("dashboard", "today", false));
    expect(fetch).not.toHaveBeenCalled();
    coordinator.request(request("models", "today"));
    await coordinator.whenIdle();

    expect(fetch).toHaveBeenCalledTimes(1);
    expect(fetch.mock.calls[0][0].page).toBe("models");
    expect(applied).toEqual(["models"]);
  });
});
