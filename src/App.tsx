import { useEffect, useState } from "react";
import { Dashboard } from "./views/Dashboard";
import { SshView } from "./views/SshView";
import { AiView } from "./views/AiView";
import { NotesView } from "./views/NotesView";
import { SyncView } from "./views/SyncView";
import { ToastProvider } from "./components/ui";
import { AppTasksProvider, useAppTasks } from "./store";
import { TerminalWorkspace } from "./components/TerminalWorkspace";
import { Onboarding } from "./views/Onboarding";
import { api } from "./api";
import { useAsync } from "./hooks";
import type { TabKey } from "./data";
import { applyTheme, loadAppName, loadTheme, saveAppName } from "./theme";
import type { ThemeKey } from "./theme";

const TABS: { key: TabKey; label: string }[] = [
  { key: "dashboard", label: "总览" },
  { key: "ssh", label: "SSH" },
  { key: "ai", label: "AI 会话" },
  { key: "notes", label: "笔记" },
  { key: "sync", label: "设置" },
];

function useClock() {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const id = setInterval(() => setNow(new Date()), 1000);
    return () => clearInterval(id);
  }, []);
  return now.toLocaleTimeString("zh-CN", { hour12: false });
}

export default function App() {
  return (
    <ToastProvider>
      <AppTasksProvider>
        <Shell />
      </AppTasksProvider>
    </ToastProvider>
  );
}

function Shell() {
  const onboard = useAsync(() => api.onboardStatus(), []);
  const [tab, setTab] = useState<TabKey>("dashboard");
  const [theme, setTheme] = useState<ThemeKey>(loadTheme);
  const [appName, setAppName] = useState(loadAppName);
  const clock = useClock();

  // 主题：切换 data-theme 并持久化
  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  // 工作台名称：持久化并同步窗口标题
  useEffect(() => {
    saveAppName(appName);
  }, [appName]);
  const hosts = useAsync(() => api.listHosts(), [], { pollMs: 20000 });
  const {
    terminals,
    activeTerm,
    terminalsVisible,
    openTerminal,
    closeTerminal,
    closeAllTerminals,
    setActiveTerm,
    setTerminalsVisible,
  } = useAppTasks();
  const running = useAsync(() => api.getRunningAction(), [], { pollMs: 3000 });
  const cron = useAsync(() => api.cronStatus(), [], { pollMs: 30000 });

  // 键盘快捷键：Ctrl+Tab 切换页面（保留 Tab 原生焦点导航）
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === "Tab") {
        e.preventDefault();
        setTab((cur) => {
          const i = TABS.findIndex((t) => t.key === cur);
          return TABS[(i + 1) % TABS.length].key;
        });
      }
      // Ctrl+1..5 直达
      if (e.ctrlKey && /^[1-5]$/.test(e.key)) {
        const idx = Number(e.key) - 1;
        if (idx >= 0 && idx < TABS.length) {
          e.preventDefault();
          setTab(TABS[idx].key);
        }
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  const hostCount = hosts.data?.length ?? 0;
  const isRunning = running.data?.active ?? false;
  const cronInterval = cron.data?.installed ? cron.data?.interval : null;

  const termHosts = (hosts.data ?? []).map((h) => ({
    alias: h.alias,
    hostname: h.hostname,
    user: h.user,
  }));

  // 首次使用引导
  if (onboard.loading) {
    return (
      <div className="h-screen flex items-center justify-center bg-[var(--color-nexus-bg)]">
        <span className="font-mono text-xs text-[var(--color-nexus-text-faint)]">加载中…</span>
      </div>
    );
  }
  if (!onboard.data?.onboarded) {
    return <Onboarding onDone={() => void onboard.reload()} />;
  }

  return (
    <>
      <div className="nexus-bg-glow h-screen flex flex-col relative">
        {/* ================= 顶栏 ================= */}
        <header className="relative z-10 flex items-center justify-between px-6 h-14 border-b border-white/5 bg-[var(--color-nexus-bg)]/70 backdrop-blur-md shrink-0">
          <div className="flex items-center gap-2.5">
            <span
              className="text-[var(--color-neon-purple)] text-base leading-none"
              style={{ textShadow: "0 0 14px rgba(157,107,255,0.7)" }}
            >
              ◆
            </span>
            <span
              className="font-mono text-[13px] font-semibold tracking-[0.16em] text-[var(--color-nexus-text)] max-w-[220px] truncate"
              title={appName || "NEXUS"}
            >
              {appName.trim() || "NEXUS"}
            </span>
            <span className="font-mono text-[10px] tracking-[0.1em] text-[var(--color-nexus-text-faint)] mt-0.5">
              WORKBENCH
            </span>
          </div>

          <div className="flex items-center gap-5 font-mono text-[11px]">
            <span className="flex items-center gap-1.5">
              <span className={`dot ${isRunning ? "dot--pulse" : "dot--on"}`} />
              <span className={isRunning ? "text-[var(--color-neon-purple)]" : "text-[var(--color-status-on)]"}>
                {isRunning ? "同步中" : "已就绪"}
              </span>
            </span>
            <span className="text-[var(--color-nexus-text-faint)]">{hostCount} hosts</span>
            <span className="text-[var(--color-nexus-text-dim)] tabular-nums">{clock}</span>
            <button
              onClick={() => setTab("sync")}
              className="text-[var(--color-nexus-text-faint)] hover:text-[var(--color-nexus-text)] transition text-sm"
              title="设置"
            >
              ⚙
            </button>
          </div>
        </header>

        {/* ================= 标签页导航 ================= */}
        <nav className="relative z-10 flex items-center gap-0.5 px-4 border-b border-white/5 bg-[var(--color-nexus-bg)]/40 backdrop-blur-sm shrink-0">
          {TABS.map((t) => (
            <button
              key={t.key}
              onClick={() => setTab(t.key)}
              className={`tab ${tab === t.key ? "tab--active" : ""}`}
            >
              {t.label}
            </button>
          ))}
        </nav>

        {/* ================= 内容区 ================= */}
        <main className="relative z-10 flex-1 min-h-0 p-6">
          <div className="h-full max-w-[1240px] mx-auto">
            {tab === "dashboard" && <Dashboard onGoto={(t) => setTab(t)} />}
            {tab === "ssh" && <SshView />}
            {tab === "ai" && <AiView />}
            {tab === "notes" && <NotesView />}
            {tab === "sync" && (
              <SyncView
                theme={theme}
                onThemeChange={setTheme}
                appName={appName}
                onAppNameChange={setAppName}
              />
            )}
          </div>
        </main>

        {/* ================= 状态栏 ================= */}
        <footer className="relative z-10 h-7 flex items-center justify-between px-6 border-t border-white/5 bg-[var(--color-nexus-bg)]/70 backdrop-blur font-mono text-[10px] text-[var(--color-nexus-text-faint)] shrink-0">
          <div className="flex items-center gap-4">
            <span className="flex items-center gap-1.5">
              <span className={`dot ${hostCount > 0 ? "dot--on" : "dot--idle"}`} />
              {hostCount} SSH 主机
            </span>
            <span className="flex items-center gap-1.5">
              <span className={`dot ${isRunning ? "dot--pulse" : "dot--idle"}`} />
              {isRunning ? "同步中" : "空闲"}
            </span>
            <span className={cronInterval ? "text-[var(--color-neon-lime)]" : ""}>
              {cronInterval ? `cron ${cronInterval}min` : "cron 未安装"}
            </span>
            {terminals.length > 0 && (
              <button
                onClick={() => setTerminalsVisible(!terminalsVisible)}
                className="flex items-center gap-1 px-1.5 py-0.5 rounded border border-[var(--color-neon-purple)]/40 text-[var(--color-neon-purple)] hover:bg-[var(--color-neon-purple)]/10 transition"
                title={terminalsVisible ? "隐藏终端（连接保持）" : "显示后台终端"}
              >
                {terminalsVisible ? "▾" : "▸"} 终端 {terminals.length}
              </button>
            )}
          </div>
          <div>
            <span className="text-[var(--color-nexus-text-dim)]">Ctrl+Tab</span> 切换 ·{" "}
            <span className="text-[var(--color-nexus-text-dim)]">Esc</span> 关闭弹窗
          </div>
        </footer>
      </div>

      {/* 全局终端工作区：位于页面切换之外，切页不会断开 */}
      {terminals.length > 0 && (
        <TerminalWorkspace
          tabs={terminals}
          activeId={activeTerm}
          onActivate={setActiveTerm}
          onClose={closeTerminal}
          onCloseAll={closeAllTerminals}
          hosts={termHosts}
          onCreate={openTerminal}
          visible={terminalsVisible}
          onMinimize={() => setTerminalsVisible(false)}
        />
      )}
    </>
  );
}
