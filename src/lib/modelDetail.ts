/**
 * 模型详情请求的序号保护。
 *
 * 详情写入全局 store 的单个槽位,"谁最后返回谁生效":连点两个模型时旧响应
 * 会覆盖新数据,关闭弹窗后迟到的响应还会把它重新打开。这里用自增序号丢弃
 * 过期响应(与 Sessions.tsx 的会话详情同一模式),并把失败交给调用方呈现,
 * 而不是 `.catch(() => {})` 静默吞掉。
 */

import { store } from "./store";
import type { ModelDetailDto } from "./types";

export interface DetailGate {
  /** 发起一次详情请求;`onError` 只在"仍是最新请求且失败"时调用。 */
  open(fetchDetail: () => Promise<ModelDetailDto | null>, onError?: () => void): void;
  /** 关闭详情,并让在途响应作废(不会再把它打开)。 */
  close(): void;
}

/** 可注入 `apply` 的构造器,便于单测。 */
export function createDetailGate(apply: (d: ModelDetailDto | null) => void): DetailGate {
  let current = 0;
  return {
    open(fetchDetail, onError) {
      const requestId = ++current;
      fetchDetail()
        .then((next) => {
          if (requestId !== current) return; // 已被更新的请求或关闭动作作废
          if (next) apply(next);
          else onError?.();
        })
        .catch(() => {
          if (requestId !== current) return;
          onError?.();
        });
    },
    close() {
      current += 1;
      apply(null);
    },
  };
}

/** 全局共享实例:详情槽位本身就是全局的,一次只可能有一个弹窗。 */
export const modelDetailGate = createDetailGate((d) => store.set({ modelDetail: d }));