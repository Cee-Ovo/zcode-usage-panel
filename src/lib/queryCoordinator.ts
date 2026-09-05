/**
 * Serializes refreshes while retaining only the newest request.  The
 * coordinator is deliberately UI-agnostic so its race/disposal behaviour can
 * be exercised without mounting the Tauri application.
 */

export interface QueryRequest<Page extends string = string> {
  page: Page;
  rangeKey: string;
  visible: boolean;
}

export interface QueryCoordinatorState<Page extends string = string> {
  loading: boolean;
  pending: boolean;
  error: string | null;
  lastSuccessMs: number | null;
  lastSuccessRequest: QueryRequest<Page> | null;
  request: QueryRequest<Page> | null;
}

export interface QueryCoordinatorOptions<Page extends string, Result> {
  fetch: (request: QueryRequest<Page>) => Promise<Result>;
  apply: (result: Result, request: QueryRequest<Page>) => void;
  onStateChange?: (state: QueryCoordinatorState<Page>) => void;
  now?: () => number;
}

function errorMessage(_error: unknown): string {
  // Do not expose backend paths, query details, or other implementation
  // strings in the always-visible application status area.
  return "刷新失败，请稍后重试";
}

export class QueryCoordinator<Page extends string, Result> {
  private readonly options: QueryCoordinatorOptions<Page, Result>;
  private readonly now: () => number;
  private state: QueryCoordinatorState<Page> = {
    loading: false,
    pending: false,
    error: null,
    lastSuccessMs: null,
    lastSuccessRequest: null,
    request: null,
  };
  private active: QueryRequest<Page> | null = null;
  private pending: QueryRequest<Page> | null = null;
  private latest: QueryRequest<Page> | null = null;
  private disposed = false;
  private idlePromise: Promise<void> = Promise.resolve();
  private resolveIdle: (() => void) | null = null;

  constructor(options: QueryCoordinatorOptions<Page, Result>) {
    this.options = options;
    this.now = options.now ?? (() => Date.now());
  }

  getState(): QueryCoordinatorState<Page> {
    return this.state;
  }

  /** Resolves when the current flight and any coalesced follow-up are done. */
  whenIdle(): Promise<void> {
    return this.idlePromise;
  }

  /** Queue a request.  A hidden request is remembered but never fetched. */
  request(request: QueryRequest<Page>): Promise<void> {
    if (this.disposed) return Promise.resolve();

    this.latest = request;
    this.state = { ...this.state, request, error: request.visible ? null : this.state.error };

    if (!request.visible) {
      if (this.active) this.pending = request;
      this.publish();
      return this.idlePromise;
    }

    if (this.active) {
      // Repeated refreshes for the in-flight key are single-flight.  Different
      // keys replace the one pending request, so only the latest range wins.
      // Even the same key gets a follow-up: the backend may have changed
      // between the start of the active read and this refresh signal.
      this.pending = request;
      this.state = { ...this.state, pending: this.pending !== null };
      this.publish();
      return this.idlePromise;
    }

    this.pending = null;
    this.start(request);
    return this.idlePromise;
  }

  /** Mark the window hidden/visible without inventing a new page or range. */
  setVisible(visible: boolean): void {
    if (this.disposed) return;
    if (!this.latest) return;

    const request = { ...this.latest, visible };
    this.latest = request;
    this.state = { ...this.state, request, error: visible ? null : this.state.error };
    if (!visible) {
      if (this.active) this.pending = request;
      this.publish();
      return;
    }

    if (this.active) {
      this.pending = request;
      this.state = { ...this.state, pending: true };
      this.publish();
      return;
    }

    this.pending = null;
    this.start(request);
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.active = null;
    this.pending = null;
    this.latest = null;
    this.state = {
      ...this.state,
      loading: false,
      pending: false,
      request: null,
    };
    this.publish();
    this.finishIdle();
  }

  private start(request: QueryRequest<Page>): void {
    if (this.disposed || !request.visible) return;
    if (!this.active) {
      this.idlePromise = new Promise<void>((resolve) => {
        this.resolveIdle = resolve;
      });
    }
    this.active = request;
    this.state = { ...this.state, loading: true, pending: false, request, error: null };
    this.publish();
    void this.execute(request);
  }

  private async execute(request: QueryRequest<Page>): Promise<void> {
    try {
      const result = await this.options.fetch(request);
      // Same-range refresh signals must not starve rendering on a busy source.
      // Show the completed snapshot, then consume the coalesced follow-up.
      // A different range/page or hidden view still rejects the old response.
      if (!this.disposed && this.latest?.visible && request.visible &&
          this.latest.page === request.page && this.latest.rangeKey === request.rangeKey) {
        this.options.apply(result, request);
        this.state = {
          ...this.state,
          error: null,
          lastSuccessMs: this.now(),
          lastSuccessRequest: request,
        };
        this.publish();
      }
    } catch (error) {
      // An old range failing must not overwrite the status of the newest one.
      if (!this.disposed && this.latest === request && request.visible) {
        this.state = { ...this.state, error: errorMessage(error) };
        this.publish();
      }
    } finally {
      if (this.active === request) this.active = null;
      if (this.disposed) {
        this.finishIdle();
        return;
      }

      const next = this.pending;
      this.pending = null;
      if (next && next.visible && this.latest === next) {
        // Keep the same idle promise across the serial follow-up.
        this.startWithoutReset(next);
      } else {
        this.state = { ...this.state, loading: false, pending: false };
        this.publish();
        this.finishIdle();
      }
    }
  }

  private startWithoutReset(request: QueryRequest<Page>): void {
    if (this.disposed || !request.visible) return;
    this.active = request;
    this.state = { ...this.state, loading: true, pending: false, request, error: null };
    this.publish();
    void this.execute(request);
  }

  private finishIdle(): void {
    const resolve = this.resolveIdle;
    this.resolveIdle = null;
    if (resolve) resolve();
  }

  private publish(): void {
    this.options.onStateChange?.(this.state);
  }
}

export function createQueryCoordinator<Page extends string, Result>(
  options: QueryCoordinatorOptions<Page, Result>,
): QueryCoordinator<Page, Result> {
  return new QueryCoordinator(options);
}
