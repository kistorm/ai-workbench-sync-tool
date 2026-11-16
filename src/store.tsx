import { createContext, useCallback, useContext, useState } from "react";
import type { ReactNode } from "react";
import { api } from "./api";
import type { YoudaoPullResult } from "./api";
import type { TermTab } from "./components/TerminalWorkspace";
import { useToast } from "./components/ui";

/**
 * 全局任务状态：拉取/同步等长任务放在 App 级，
 * 这样用户可以关闭弹窗、切换页面，任务与结果都不丢失。
 */
interface AppTasks {
  youdaoPulling: boolean;
  youdaoResult: YoudaoPullResult | null;
  startYoudaoPull: (cookies: string, ydnoteDir?: string) => void;
  clearYoudaoResult: () => void;

  /* SSH 终端：提升到 App 级，切换页面不会断开 */
  terminals: TermTab[];
  activeTerm: string | null;
  terminalsVisible: boolean;
  openTerminal: (alias: string) => void;
  closeTerminal: (id: string) => void;
  closeAllTerminals: () => void;
  setActiveTerm: (id: string | null) => void;
  setTerminalsVisible: (v: boolean) => void;
}

const Ctx = createContext<AppTasks>({
  youdaoPulling: false,
  youdaoResult: null,
  startYoudaoPull: () => {},
  clearYoudaoResult: () => {},
  terminals: [],
  activeTerm: null,
  terminalsVisible: false,
  openTerminal: () => {},
  closeTerminal: () => {},
  closeAllTerminals: () => {},
  setActiveTerm: () => {},
  setTerminalsVisible: () => {},
});

export function useAppTasks() {
  return useContext(Ctx);
}

export function AppTasksProvider({ children }: { children: ReactNode }) {
  const { push } = useToast();
  const [youdaoPulling, setYoudaoPulling] = useState(false);
  const [youdaoResult, setYoudaoResult] = useState<YoudaoPullResult | null>(null);

  const startYoudaoPull = useCallback(
    (cookies: string, ydnoteDir?: string) => {
      if (youdaoPulling) return;
      setYoudaoPulling(true);
      setYoudaoResult(null);
      push("info", "已开始从有道云拉取", "可以关闭弹窗或切到其他页面，完成后会通知你");

      // 不阻塞 UI：任务在后台执行，状态提升到 App 级，切页不丢失
      api
        .youdaoPull(cookies, ydnoteDir)
        .then((r) => {
          setYoudaoResult(r);
          if (r.ok) {
            push(
              "ok",
              `有道云拉取完成：新增 ${r.added} · 更新 ${r.updated} · 跳过 ${r.skipped}`,
              "笔记列表已更新，如需同步云端请点「⇡ 推送」"
            );
          } else {
            push("err", "有道云拉取未成功", r.output.slice(-400));
          }
        })
        .catch((e) => {
          push("err", "有道云拉取失败", String(e));
        })
        .finally(() => {
          setYoudaoPulling(false);
        });
    },
    [push, youdaoPulling]
  );

  const clearYoudaoResult = useCallback(() => setYoudaoResult(null), []);

  // ===== SSH 终端（App 级，切页不断开） =====
  const [terminals, setTerminals] = useState<TermTab[]>([]);
  const [activeTerm, setActiveTerm] = useState<string | null>(null);
  const [terminalsVisible, setTerminalsVisible] = useState(false);

  const openTerminal = useCallback((alias: string) => {
    setTerminals((prev) => {
      const same = prev.filter((t) => t.alias === alias).length;
      const title = same > 0 ? `${alias} (${same + 1})` : alias;
      const id = `term-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
      setActiveTerm(id);
      return [...prev, { id, alias, title }];
    });
    setTerminalsVisible(true);
  }, []);

  const closeTerminal = useCallback((id: string) => {
    setTerminals((prev) => {
      const next = prev.filter((t) => t.id !== id);
      setActiveTerm((cur) => {
        if (cur !== id) return cur;
        return next.length > 0 ? next[next.length - 1].id : null;
      });
      if (next.length === 0) setTerminalsVisible(false);
      return next;
    });
  }, []);

  const closeAllTerminals = useCallback(() => {
    setTerminals([]);
    setActiveTerm(null);
    setTerminalsVisible(false);
  }, []);

  return (
    <Ctx.Provider
      value={{
        youdaoPulling,
        youdaoResult,
        startYoudaoPull,
        clearYoudaoResult,
        terminals,
        activeTerm,
        terminalsVisible,
        openTerminal,
        closeTerminal,
        closeAllTerminals,
        setActiveTerm,
        setTerminalsVisible,
      }}
    >
      {children}
    </Ctx.Provider>
  );
}
