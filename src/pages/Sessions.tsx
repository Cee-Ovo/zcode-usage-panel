import { useCallback, useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { Glass, SearchField } from "open-glass-ui";
import { TrendChart } from "../components/TrendChart";
import { AccessibleDialog } from "../components/AccessibleDialog";
import { FxButton, FxCloseChip } from "../components/fx";
import { api, onEvent } from "../lib/ipc";
import { store, useStore } from "../lib/store";
import type { SessionSort, SessionSummary, SessionsPageDto } from "../lib/types";
import { cacheHitRate, totalTokens } from "../lib/types";
import { formatDateTime, formatRelative, formatTokens, shortSessionId, truncateHead } from "../lib/format";
import { listItemVariants, rowGestures, softSpring } from "../lib/motion";

const DEFAULT_PAGE_SIZE = 50;

async function loadSessionsPage(query: string, sort: SessionSort, page: number, pageSize: number): Promise<SessionsPageDto> {
  return api.sessionsPage(query, sort, page, pageSize);
}

export function SessionsPage() {
  const detail = useStore((s) => s.sessionDetail);
  const [inputQuery, setInputQuery] = useState("");
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<SessionSort>("recent");
  const [page, setPage] = useState(0);
  const [pageSize, setPageSize] = useState(DEFAULT_PAGE_SIZE);
  const [result, setResult] = useState<SessionsPageDto | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState(false);
  const [refreshTick, setRefreshTick] = useState(0);
  const requestRef = useRef(0);
  const detailRequestRef = useRef(0);
  const aliveRef = useRef(true);

  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
      detailRequestRef.current += 1;
    };
  }, []);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      setQuery(inputQuery.trim());
      setPage(0);
    }, 250);
    return () => window.clearTimeout(timer);
  }, [inputQuery]);

  const refresh = useCallback(() => {
    if (!document.hidden) setRefreshTick((tick) => tick + 1);
  }, []);

  useEffect(() => {
    let disposed = false;
    const requestId = ++requestRef.current;
    setLoading(true);
    setError(null);
    loadSessionsPage(query, sort, page, pageSize)
      .then((next) => {
        if (disposed || requestId !== requestRef.current) return;
        setResult(next);
        setLoading(false);
      })
      .catch(() => {
        if (disposed || requestId !== requestRef.current) return;
        setLoading(false);
        setError("无法加载 Session 列表，请稍后重试。");
      });
    return () => { disposed = true; };
  }, [query, sort, page, pageSize, refreshTick]);

  useEffect(() => {
    let disposed = false;
    const onPageRefresh = () => refresh();
    window.addEventListener("zup-page-sessions", onPageRefresh);
    document.addEventListener("visibilitychange", onPageRefresh);
    const timer = window.setInterval(refresh, 15_000);
    const unsubs: Array<() => void> = [];
    if ("__TAURI_INTERNALS__" in window) {
      onEvent("usage-update", onPageRefresh)
        .then((unlisten) => { if (disposed) unlisten(); else unsubs.push(unlisten); })
        .catch(() => {});
    }
    return () => {
      disposed = true;
      window.removeEventListener("zup-page-sessions", onPageRefresh);
      document.removeEventListener("visibilitychange", onPageRefresh);
      window.clearInterval(timer);
      unsubs.forEach((unlisten) => unlisten());
    };
  }, [refresh]);

  const pageCount = result ? Math.max(1, Math.ceil(result.total / pageSize)) : 1;
  const canPrevious = page > 0;
  const canNext = page + 1 < pageCount;
  const openDetail = useCallback((sessionId: string) => {
    const requestId = ++detailRequestRef.current;
    setSelectedSessionId(sessionId);
    setDetailLoading(true);
    setDetailError(false);
    store.set({ sessionDetail: null });
    api.sessionDetail(sessionId)
      .then((next) => {
        if (!aliveRef.current || requestId !== detailRequestRef.current) return;
        if (next) {
          store.set({ sessionDetail: next });
          setDetailError(false);
        } else {
          setDetailError(true);
        }
        setDetailLoading(false);
      })
      .catch(() => {
        if (!aliveRef.current || requestId !== detailRequestRef.current) return;
        setDetailLoading(false);
        setDetailError(true);
      });
  }, []);
  const closeDetail = useCallback(() => {
    detailRequestRef.current += 1;
    setSelectedSessionId(null);
    setDetailLoading(false);
    setDetailError(false);
    store.set({ sessionDetail: null });
  }, []);
  const retryDetail = useCallback(() => {
    if (selectedSessionId) openDetail(selectedSessionId);
  }, [openDetail, selectedSessionId]);

  return (
    <div className="sessions-page" style={{ paddingTop: 6 }}>
      <header className="page-heading page-heading--compact">
        <div>
          <span className="page-eyebrow">SESSION HISTORY</span>
          <h1>Sessions</h1>
          <p>浏览本地会话的 Token 用量与最近活动。</p>
        </div>
      </header>
      <div className="page-toolbar sessions-toolbar" style={{ display: "flex", gap: 10, alignItems: "center", marginBottom: 12, flexWrap: "wrap" }}>
        <SearchField label="搜索 session / 会话名 / 项目 / 模型" value={inputQuery} onValueChange={setInputQuery} placeholder="搜索 session / 会话名 / 项目 / 模型…" />
        <span className="muted" style={{ fontSize: 11 }}>{result ? `${result.total} sessions` : "加载 sessions…"}</span>
        <label className="muted" style={{ fontSize: 11, marginLeft: "auto" }}>
          排序{" "}
          <select value={sort} onChange={(event) => { setSort(event.target.value as SessionSort); setPage(0); }} aria-label="Session 排序">
            <option value="recent">最近活动</option><option value="tokens">总 Token</option>
          </select>
        </label>
        <label className="muted" style={{ fontSize: 11 }}>
          每页{" "}
          <select value={pageSize} onChange={(event) => { setPageSize(Number(event.target.value)); setPage(0); }} aria-label="每页 Session 数">
            <option value={25}>25</option><option value={50}>50</option><option value={100}>100</option>
          </select>
        </label>
      </div>

      <Glass className="panel sample-glass page-surface sessions-surface" material="regular" renderer="css" interactive={false}>
        <div className="session-row table-head" style={{ cursor: "default" }}>
          <span title="完整 Session ID 见单元格悬停">Session</span>
          <span title="真实会话标题(ZCode 本地记录),完整内容见悬停">会话名</span>
          <span title="项目文件夹名,完整路径见悬停">项目</span>
          <span title="完整模型名见悬停">模型</span>
          <span style={{ textAlign: "right" }}>输入</span><span style={{ textAlign: "right" }}>输出</span>
          <span style={{ textAlign: "right" }}>推理</span><span style={{ textAlign: "right" }}>缓存</span>
          <span style={{ textAlign: "right" }}>总量</span><span style={{ textAlign: "right" }}>命中率 / 活动</span>
        </div>
        {loading && !result && <div className="empty-state">正在加载 Session…</div>}
        {error && <div className="empty-state" role="alert">加载 Session 失败：{error}<br /><FxButton size="small" onClick={refresh}>重试</FxButton></div>}
        {!loading && !error && result?.items.length === 0 && (
          <div className="empty-state">没有匹配的 Session。<br /><span style={{ fontSize: 11 }}>数据来源为 ZCode 本地记录;若列表为空,请到「设置 → 数据源详情」检查目录。</span></div>
        )}
        <AnimatePresence initial={false}>
          {result?.items.map((s) => (
            <SessionLine key={s.id} s={s} onOpen={openDetail} />
          ))}
        </AnimatePresence>
        {result && result.total > 0 && (
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", padding: "10px 12px" }}>
            <span className="muted" style={{ fontSize: 11 }}>第 {page + 1} / {pageCount} 页 · 显示 {page * pageSize + 1}–{Math.min((page + 1) * pageSize, result.total)} / {result.total}</span>
            <div style={{ display: "flex", gap: 6 }}>
              <FxButton size="small" disabled={!canPrevious || loading} onClick={() => setPage((p) => p - 1)}>上一页</FxButton>
              <FxButton size="small" disabled={!canNext || loading} onClick={() => setPage((p) => p + 1)}>下一页</FxButton>
            </div>
          </div>
        )}
      </Glass>

      {(detail || detailLoading || detailError) && (
        <AccessibleDialog label="Session 详情" onClose={closeDetail} glass>
          <div className="panel-title">
            Session 详情 · {detail ? shortSessionId(detail.summary.id) : "加载中"}
            <span className="right"><FxCloseChip onClick={closeDetail} /></span>
          </div>
          {detailLoading && <div className="empty-state">正在加载 Session 详情…</div>}
          {detailError && !detailLoading && (
            <div className="empty-state" role="alert">
              无法加载 Session 详情，请稍后重试。
              <br />
              <FxButton size="small" onClick={retryDetail}>重试</FxButton>
            </div>
          )}
          {detail && !detailLoading && !detailError && (
            <>
              <div className="kv" style={{ marginBottom: 12 }}>
                <span className="k">会话名</span><span>{detail.summary.title ?? "—（无真实标题）"}</span>
                <span className="k">项目</span><span>{detail.summary.projectPath ?? detail.summary.project ?? "—"}</span>
                <span className="k">Session ID</span><span style={{ wordBreak: "break-all" }}>{detail.summary.id}</span>
                <span className="k">模型</span><span>{detail.summary.models.join(", ")}</span>
                <span className="k">开始 → 最后活动</span>
                <span>
                  {formatDateTime(detail.summary.agg.firstTsMs)} → {" "}
                  {formatDateTime(detail.summary.agg.lastTsMs)}
                </span>
                <span className="k">总 Token</span>
                <span>{formatTokens(totalTokens(detail.summary.agg))}</span>
              </div>
              <TrendChart
                trend={{
                  rangeKey: "session",
                  fromMs: detail.summary.agg.firstTsMs ?? 0,
                  toMs: detail.summary.agg.lastTsMs ?? 0,
                  buckets: detail.buckets,
                  restored: false,
                }}
                visibleModels={null}
              />
            </>
          )}
        </AccessibleDialog>
      )}
    </div>
  );
}

function SessionLine({ s, onOpen }: { s: SessionSummary; onOpen: (sessionId: string) => void }) {
  const hit = cacheHitRate(s.agg);
  const open = () => onOpen(s.id);
  return (
    <motion.div
      className="session-row"
      layout="position"
      variants={listItemVariants}
      initial="initial"
      animate="enter"
      exit="exit"
      {...rowGestures}
      transition={softSpring}
      role="button"
      tabIndex={0}
      aria-label={`查看 Session ${s.id}`}
      onClick={open}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          open();
        }
      }}
      title="点击查看 Session 趋势"
    >
      <span title={s.id}>{truncateHead(s.id, 10)}</span>
      <span title={s.title ?? "无真实会话标题"} className={s.title ? "clip-cell" : "muted clip-cell"}>
        {s.title ? truncateHead(s.title, 12) : "—"}
      </span>
      <span title={s.projectPath ?? s.project ?? "无项目记录"} className="muted clip-cell">
        {s.project ? truncateHead(s.project, 14) : "—"}
      </span>
      <span title={s.models.join(", ") || "无模型记录"} className="muted clip-cell">
        {s.models.length > 0 ? truncateHead(s.models.join(", "), 14) : "—"}
      </span>
      <span className="num">{formatTokens(s.agg.input)}</span><span className="num">{formatTokens(s.agg.output)}</span>
      <span className="num">{s.agg.reasoning.present > 0 ? formatTokens(s.agg.reasoning.sum) : "—"}</span>
      <span className="num">{s.agg.cacheRead.present > 0 ? formatTokens(s.agg.cacheRead.sum) : "—"}</span>
      <span className="num" style={{ fontWeight: 600 }}>{formatTokens(totalTokens(s.agg))}</span>
      <span className="num">{hit === null ? "—" : `${(hit * 100).toFixed(0)}%`}<div className="muted" style={{ fontSize: 10 }}>{formatRelative(s.agg.lastTsMs)}</div></span>
    </motion.div>
  );
}
