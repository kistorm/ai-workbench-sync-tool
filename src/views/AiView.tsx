import { useState } from "react";
import { api } from "../api";
import { useAction, useAsync } from "../hooks";
import { ErrorBox, Loading, useToast } from "../components/ui";

export function AiView() {
  const { push } = useToast();
  const status = useAsync(() => api.aiStatus(), [], { pollMs: 30000 });
  const { busy, run } = useAction();
  const [output, setOutput] = useState<string>("");
  const [lastAction, setLastAction] = useState<string>("");

  async function doSync(action: "push" | "pull" | "status") {
    const r = await run(action, () => api.aiSyncRun(action));
    setLastAction(action);
    if (r.ok) {
      setOutput(r.data ?? "");
      push("ok", `AI 会话 ${action} 完成`);
      void status.reload();
    } else {
      setOutput(r.error ?? "");
      push("err", `AI 会话 ${action} 失败`, r.error);
    }
  }

  const tools = status.data?.tools ?? [];
  const detected = tools.filter((t) => t.detected);
  const totalSessions = tools.reduce((s, t) => s + Number(t.sessions || 0), 0);
  const maxSessions = Math.max(1, ...tools.map((t) => Number(t.sessions || 0)));

  return (
    <div className="page-enter h-full flex flex-col gap-5">
      <header className="flex items-center justify-between shrink-0">
        <div>
          <h1 className="text-lg font-semibold text-[var(--color-nexus-text)]">AI 会话同步</h1>
          <p className="font-mono text-[11px] text-[var(--color-nexus-text-faint)] mt-0.5">
            {status.data?.script_found
              ? `数据目录 ${status.data.repo_dir} · 随统一仓库推送`
              : "未找到 ai-session-sync（配置路径后可启用）"}
          </p>
        </div>
        <div className="flex gap-2">
          <button className="btn" disabled={!!busy} onClick={() => doSync("status")}>
            {busy === "status" ? "查询中…" : "状态"}
          </button>
          <button className="btn" disabled={!!busy} onClick={() => doSync("pull")}>
            {busy === "pull" ? "拉取中…" : "⇣ 拉取"}
          </button>
          <button className="btn btn--primary" disabled={!!busy} onClick={() => doSync("push")}>
            {busy === "push" ? "推送中…" : "⇡ 立即推送"}
          </button>
        </div>
      </header>

      {status.error && <ErrorBox message={status.error} />}

      {status.loading && !status.data && <Loading />}

      {/* 仓库信息 */}
      {status.data && (
        <div className="grid grid-cols-3 gap-4 shrink-0">
          <InfoTile label="检测到的工具" value={`${detected.length}/${tools.length}`} accent="var(--color-neon-cyan)" />
          <InfoTile label="会话总数" value={String(totalSessions)} accent="var(--color-neon-cyan)" />
          <InfoTile label="最近提交" value={status.data.last_commit || "—"} accent="var(--color-nexus-text)" small />
        </div>
      )}

      {/* 工具卡 */}
      <div className="grid grid-cols-2 gap-4 shrink-0 stagger">
        {tools.map((t) => (
          <div key={t.name} className="surface surface-hover px-5 py-4">
            <div className="flex items-center justify-between mb-3">
              <div className="flex items-center gap-2">
                <span className={`dot ${t.detected ? (t.healthy ? "dot--on" : "dot--warn") : "dot--idle"}`} />
                <span className="font-mono text-[13px] text-[var(--color-nexus-text)]">{t.name}</span>
              </div>
              <span className="font-mono text-[10px] text-[var(--color-nexus-text-faint)]">
                {t.detected ? "已检测" : "未安装"}
              </span>
            </div>
            <div className="flex items-baseline gap-1.5">
              <span
                className="font-mono text-2xl font-semibold"
                style={{ color: t.detected ? "var(--color-neon-cyan)" : "var(--color-nexus-text-faint)" }}
              >
                {t.detected ? t.sessions : "—"}
              </span>
              <span className="font-mono text-[11px] text-[var(--color-nexus-text-dim)]">
                {t.detected ? "会话" : ""}
              </span>
            </div>
            <div className="progress-track mt-3">
              <div
                className="progress-fill"
                style={{
                  width: `${t.detected ? Math.min(100, (Number(t.sessions) / maxSessions) * 100) : 0}%`,
                  opacity: t.detected ? 1 : 0.15,
                }}
              />
            </div>
          </div>
        ))}
      </div>

      {/* 输出/日志 */}
      <section className="surface flex-1 min-h-0 flex flex-col">
        <div className="flex items-center justify-between px-5 py-3 border-b border-white/5 shrink-0">
          <h3 className="font-mono text-xs tracking-[0.14em] text-[var(--color-nexus-text-dim)] uppercase">
            {lastAction ? `输出 · ${lastAction}` : "同步输出"}
          </h3>
          <button className="btn" onClick={() => setOutput("")}>
            清空
          </button>
        </div>
        <pre className="flex-1 overflow-auto p-4 font-mono text-[11px] leading-relaxed text-[var(--color-nexus-text-dim)] whitespace-pre-wrap break-words">
          {output || "点击上方按钮执行同步操作，输出会显示在这里。"}
        </pre>
      </section>
    </div>
  );
}

function InfoTile({
  label,
  value,
  accent,
  small,
}: {
  label: string;
  value: string;
  accent: string;
  small?: boolean;
}) {
  return (
    <div className="surface px-5 py-3">
      <div className="font-mono text-[10px] tracking-[0.12em] text-[var(--color-nexus-text-faint)] uppercase mb-1">
        {label}
      </div>
      <div
        className={`font-mono ${small ? "text-[11.5px]" : "text-xl font-semibold"} truncate`}
        style={{ color: accent }}
        title={value}
      >
        {value}
      </div>
    </div>
  );
}
