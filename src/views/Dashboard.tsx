import { api } from "../api";
import type { Activity } from "../api";
import { useAsync } from "../hooks";
import { ErrorBox, Loading } from "../components/ui";

const kindIcon: Record<string, { icon: string; color: string }> = {
  sync: { icon: "⇅", color: "var(--color-neon-purple)" },
  connect: { icon: "→", color: "var(--color-neon-cyan)" },
  note: { icon: "✎", color: "var(--color-neon-pink)" },
  error: { icon: "!", color: "var(--color-status-off)" },
  info: { icon: "·", color: "var(--color-nexus-text-faint)" },
};

export function Dashboard({ onGoto }: { onGoto: (tab: "ssh" | "ai" | "notes" | "sync") => void }) {
  const running = useAsync(() => api.getRunningAction(), [], { pollMs: 3000 });
  const activities = useAsync(() => api.listActivities(30), [], { pollMs: 5000 });
  const hosts = useAsync(() => api.listHosts(), [], { pollMs: 15000 });
  const ai = useAsync(() => api.aiStatus(), [], { pollMs: 15000 });
  const notes = useAsync(() => api.listNotes(), [], { pollMs: 20000 });

  const upHosts = (hosts.data ?? []).filter((h) => h.alias && h.hostname).length;
  const detectedTools = (ai.data?.tools ?? []).filter((t) => t.detected).length;
  const totalSessions = (ai.data?.tools ?? []).reduce((s, t) => s + Number(t.sessions || 0), 0);
  const noteCount = notes.data?.length ?? 0;
  const ra = running.data;

  return (
    <div className="page-enter flex flex-col gap-5 h-full">
      {/* ================= 主视觉：正在执行 ================= */}
      <section className={`surface-active p-6 shrink-0 ${ra?.active ? "breathing" : ""}`}>
        <div className="relative z-10">
          <div className="flex items-start justify-between mb-4">
            <div className="min-w-0">
              <div className="flex items-center gap-2 mb-1.5">
                <span className={`dot ${ra?.active ? "dot--pulse" : "dot--idle"}`} />
                <span className="font-mono text-[11px] tracking-[0.18em] text-[var(--color-neon-purple)] uppercase">
                  {ra?.active ? "执行中" : "空闲"}
                </span>
              </div>
              <h2 className="text-xl font-semibold text-[var(--color-nexus-text)] mb-1 truncate">
                {ra?.active ? ra.title : "当前无进行中的任务"}
              </h2>
              <p className="font-mono text-xs text-[var(--color-nexus-text-dim)] truncate">
                {ra?.active ? ra.subtitle : `上次活动：${activities.data?.[0]?.title ?? "—"}`}
              </p>
            </div>
            <div className="text-right shrink-0 ml-4">
              <div className="font-mono text-3xl font-semibold text-[var(--color-nexus-text)] leading-none">
                {ra?.active ? ra.progress : "—"}
                {ra?.active && <span className="text-sm text-[var(--color-nexus-text-dim)] ml-0.5">%</span>}
              </div>
              {ra?.active && ra.eta && (
                <div className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] mt-1">{ra.eta}</div>
              )}
            </div>
          </div>
          <div className="progress-track">
            <div
              className="progress-fill"
              style={{ width: `${ra?.active ? ra.progress : 0}%`, opacity: ra?.active ? 1 : 0.25 }}
            />
          </div>
        </div>
      </section>

      {/* ================= 健康度三卡 ================= */}
      <section className="grid grid-cols-3 gap-4 shrink-0 stagger">
        <HealthCard
          label="SSH 主机"
          value={hosts.loading ? "…" : `${upHosts}`}
          hint={`已配置 · ${hosts.data?.length ?? 0} 总计`}
          accent="var(--color-neon-purple)"
          ok={!hosts.error && upHosts > 0}
          onClick={() => onGoto("ssh")}
        />
        <HealthCard
          label="AI 工具"
          value={ai.loading ? "…" : `${detectedTools}`}
          hint={`已检测 · ${totalSessions} 会话`}
          accent="var(--color-neon-cyan)"
          ok={!ai.error && detectedTools > 0}
          onClick={() => onGoto("ai")}
        />
        <HealthCard
          label="笔记"
          value={notes.loading ? "…" : `${noteCount}`}
          hint="篇 Markdown"
          accent="var(--color-neon-pink)"
          ok={!notes.error}
          onClick={() => onGoto("notes")}
        />
      </section>

      {/* ================= 活动流 ================= */}
      <section className="surface flex-1 min-h-0 flex flex-col">
        <div className="flex items-center justify-between px-5 py-3 border-b border-white/5 shrink-0">
          <h3 className="font-mono text-xs tracking-[0.14em] text-[var(--color-nexus-text-dim)] uppercase">
            活动流
          </h3>
          <button className="btn" onClick={() => activities.reload()}>
            刷新
          </button>
        </div>
        <div className="flex-1 overflow-y-auto px-2 py-2">
          {activities.error && <div className="p-3"><ErrorBox message={activities.error} /></div>}
          {activities.loading && !activities.data && <div className="p-4"><Loading /></div>}
          {!activities.loading && (activities.data ?? []).length === 0 && (
            <div className="p-6 text-center font-mono text-[11px] text-[var(--color-nexus-text-faint)]">
              暂无活动记录
            </div>
          )}
          {(activities.data ?? []).map((a, i) => (
            <ActivityRow key={a.id} activity={a} delay={Math.min(i * 0.03, 0.3)} />
          ))}
        </div>
      </section>
    </div>
  );
}

function HealthCard({
  label,
  value,
  hint,
  accent,
  ok,
  onClick,
}: {
  label: string;
  value: string;
  hint: string;
  accent: string;
  ok: boolean;
  onClick: () => void;
}) {
  return (
    <button onClick={onClick} className="surface surface-hover px-5 py-4 text-left w-full">
      <div className="flex items-center justify-between mb-2">
        <span className="font-mono text-[10px] tracking-[0.12em] text-[var(--color-nexus-text-faint)] uppercase">
          {label}
        </span>
        <span className={`dot ${ok ? "dot--on" : "dot--warn"}`} />
      </div>
      <div className="flex items-baseline gap-1.5">
        <span className="font-mono text-2xl font-semibold" style={{ color: accent }}>
          {value}
        </span>
        <span className="font-mono text-[11px] text-[var(--color-nexus-text-dim)] truncate">{hint}</span>
      </div>
    </button>
  );
}

function ActivityRow({ activity, delay }: { activity: Activity; delay: number }) {
  const k = kindIcon[activity.kind] ?? kindIcon.info;
  return (
    <div
      className="log-item flex items-start gap-3 px-3 py-2.5 rounded-lg hover:bg-white/[0.03] transition-colors"
      style={{ animationDelay: `${delay}s` }}
    >
      <span className="font-mono text-[11px] text-[var(--color-nexus-text-faint)] w-10 shrink-0 pt-0.5">
        {activity.time}
      </span>
      <span className="text-sm shrink-0 w-4 text-center pt-0.5" style={{ color: k.color }}>
        {k.icon}
      </span>
      <div className="flex-1 min-w-0">
        <div className="text-[13px] text-[var(--color-nexus-text)] truncate">{activity.title}</div>
        {activity.detail && (
          <div className="font-mono text-[10.5px] text-[var(--color-nexus-text-faint)] truncate mt-0.5">
            {activity.detail}
          </div>
        )}
      </div>
    </div>
  );
}
