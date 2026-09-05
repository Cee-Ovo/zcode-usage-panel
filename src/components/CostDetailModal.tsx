import { useEffect, useState } from "react";
import { AccessibleDialog } from "./AccessibleDialog";
import { FxButton, FxCloseChip } from "./fx";
import { api } from "../lib/ipc";
import type { CostDetailDto, FxInfo } from "../lib/types";
import { RANGE_LABELS } from "../lib/types";
import { formatCny, formatTokens, formatUnitPerM } from "../lib/format";

/**
 * Cost breakdown for one model over the current range (role="dialog").
 * fx / priceUpdatedAt are passed in from the caller's cost_summary snapshot
 * so the footer shows the same rates the estimate was computed with.
 */
export function CostDetailModal({
  model,
  rangeKey,
  fx,
  priceUpdatedAt,
  onClose,
  glass = false,
}: {
  model: string;
  rangeKey: string;
  fx: FxInfo | null | undefined;
  priceUpdatedAt: string | null | undefined;
  onClose: () => void;
  glass?: boolean;
}) {
  const [detail, setDetail] = useState<CostDetailDto | null>(null);
  const [error, setError] = useState(false);
  const [retry, setRetry] = useState(0);

  useEffect(() => {
    let alive = true;
    setDetail(null);
    setError(false);
    api
      .costDetail(rangeKey, model)
      .then((d) => {
        if (alive) setDetail(d);
      })
      .catch(() => { if (alive) setError(true); });
    return () => {
      alive = false;
    };
  }, [model, rangeKey, retry]);

  const rangeLabel = RANGE_LABELS[rangeKey as keyof typeof RANGE_LABELS] ?? rangeKey;

  return (
    <AccessibleDialog glass={glass} label={`成本明细 · ${model}`} onClose={onClose} className="cost-modal">
        <div className="panel-title">
          成本明细 · {model}
          <span className="muted" style={{ fontWeight: 500 }}>
            {rangeLabel}
          </span>
          <span className="right">
            <FxCloseChip onClick={onClose} />
          </span>
        </div>

        {error ? (
          <div role="alert" className="empty-state">
            成本明细加载失败。
            <FxButton variant="quiet" size="small" onClick={() => setRetry((n) => n + 1)}>重试</FxButton>
          </div>
        ) : !detail ? (
          <div className="empty-state">
            <span className="fx-spinner" aria-hidden style={{ marginRight: 8, verticalAlign: -1 }} />
            正在加载成本明细…
          </div>
        ) : !detail.priced ? (
          <div className="empty-state">
            <div style={{ fontSize: 14, color: "var(--zup-text-2)" }}>价格未知</div>
            <div className="muted" style={{ fontSize: 11, marginTop: 6, maxWidth: 420, marginInline: "auto" }}>
              该模型没有官方价格表条目,无法估算 API 花费。可在「设置 → API 价格表 → 价格未知」中手动覆盖单价。
            </div>
            <div style={{ marginTop: 14 }}>
              <FxButton variant="quiet" size="small" onClick={onClose}>
                知道了
              </FxButton>
            </div>
          </div>
        ) : (
          <>
            <div className="cost-lines">
              <div className="cost-line cost-head">
                <span>项目</span>
                <span className="num">Token</span>
                <span className="num">单价</span>
                <span className="num">金额</span>
              </div>
              {detail.lines.length === 0 && (
                <div className="muted" style={{ padding: "10px 4px", fontSize: 11 }}>
                  该模型在本范围内无计费明细
                </div>
              )}
              {detail.lines.map((l) => (
                <div className="cost-line" key={`${l.key}-${l.tier ?? ""}`}>
                  <span>{l.label}</span>
                  <span className="num">{formatTokens(l.tokens)}</span>
                  <span className="num">{formatUnitPerM(l.perM, l.currency)}</span>
                  <span className="num">
                    {l.includedIn ? `已包含在 ${l.includedIn}` : formatCny(l.costCny)}
                  </span>
                </div>
              ))}
            </div>

            {detail.notes.length > 0 && (
              <ul className="cost-notes">
                {detail.notes.map((n, i) => (
                  <li key={i}>{n}</li>
                ))}
              </ul>
            )}

            <div className="cost-total">
              <span>合计</span>
              <span>{formatCny(detail.totalCny)}</span>
            </div>

            <div className="muted" style={{ fontSize: 10.5, marginTop: 8 }}>
              按官方 API 单价估算 · 非实际 Billing
            </div>
            {fx && (
              <div className="muted" style={{ fontSize: 10.5, marginTop: 4 }}>
                价格更新 {priceUpdatedAt || "—"} · 汇率 {fx.updatedAt}（{fx.usdCny.toFixed(4)}）
              </div>
            )}
          </>
        )}
    </AccessibleDialog>
  );
}
