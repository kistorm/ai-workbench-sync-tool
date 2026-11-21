import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { AiPaths, Diagnosis, RunningAction } from "../api";

/**
 * 首次使用引导
 * - 恢复模式：连接仓库 → 全量拉取 → 主密码解密 → 生成 SSH 配置
 * - 全新模式：连接空仓库 → 主密码加密本机数据 → 首次推送
 */
export function Onboarding({ onDone }: { onDone: () => void }) {
  const [mode, setMode] = useState<"recover" | "fresh" | null>(null);
  const [repo, setRepo] = useState("");
  const [pwd, setPwd] = useState("");
  const [pwd2, setPwd2] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<string | null>(null);
  const [checkBusy, setCheckBusy] = useState(false);
  const [checkRes, setCheckRes] = useState<Diagnosis | null>(null);
  const [checkErr, setCheckErr] = useState<string | null>(null);
  const [prog, setProg] = useState<RunningAction | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const [countdown, setCountdown] = useState(0);
  const pwdRef = useRef<HTMLInputElement>(null);
  const [aiDefaults, setAiDefaults] = useState<AiPaths | null>(null);
  const [aiPaths, setAiPaths] = useState<Record<string, string>>({});

  // 加载当前平台的 AI 工具默认路径（作为 placeholder；可自行覆盖）
  useEffect(() => {
    api.aiDefaultPaths().then(setAiDefaults).catch(() => {});
  }, []);

  // Shell 会因轮询重新渲染，用 ref 固定 onDone，避免倒计时被重置
  const onDoneRef = useRef(onDone);
  onDoneRef.current = onDone;

  // 执行期间轮询后端进度（RunningAction）
  useEffect(() => {
    if (!busy) return;
    let alive = true;
    const tick = () => {
      api
        .getRunningAction()
        .then((r) => {
          if (alive) setProg(r);
        })
        .catch(() => {});
    };
    tick();
    const id = setInterval(tick, 400);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [busy]);

  // 已用时
  useEffect(() => {
    if (!busy) return;
    const start = Date.now();
    setElapsed(0);
    const id = setInterval(() => setElapsed(Math.floor((Date.now() - start) / 1000)), 1000);
    return () => clearInterval(id);
  }, [busy]);

  // 成功后 5 秒倒计时进入主界面
  useEffect(() => {
    if (!result) return;
    const start = Date.now();
    setCountdown(5);
    const id = setInterval(() => {
      const left = 5 - Math.floor((Date.now() - start) / 1000);
      if (left <= 0) {
        clearInterval(id);
        setCountdown(0);
        onDoneRef.current();
      } else {
        setCountdown(left);
      }
    }, 200);
    return () => clearInterval(id);
  }, [result]);

  const pct = Math.max(0, Math.min(100, prog?.progress ?? 0));

  /** Git 地址严格格式校验：只允许 git@host:owner/repo.git */
  const REPO_RE = /^git@[^\s:/]+\.[^\s:/]+:[^\s:/]+\/[^\s:/]+\.git$/;
  const repoTrimmed = repo.trim();
  const repoValid = REPO_RE.test(repoTrimmed);
  function repoFormatError(): string | null {
    if (!repoTrimmed) return null;
    const s = repoTrimmed;
    if (/^(https?|ssh|git|file):\/\//i.test(s)) return "不支持 http/ssh 等格式，请使用 git@ 开头的 SSH 格式";
    if (!s.toLowerCase().startsWith("git@")) return "地址必须以 git@ 开头";
    if (!s.includes(":")) return "地址缺少冒号，应为 git@github.com:you/your-repo.git";
    if (!/\.[A-Za-z]{2,}:/i.test(s)) return "主机名无效，应为 git@github.com:you/your-repo.git";
    const path = s.slice(s.indexOf(":") + 1);
    if (!path.includes("/")) return "路径应为 用户名/仓库.git 两段";
    if (!s.toLowerCase().endsWith(".git")) return "仓库路径必须以 .git 结尾";
    return null;
  }
  const repoErr = repoFormatError();

  /** 检测 Git 仓库连通性（全新 / 恢复模式共用） */
  async function checkRepo() {
    if (!repoTrimmed) {
      setCheckErr("请先填写 Git 仓库地址");
      setCheckRes(null);
      return;
    }
    if (!repoValid) {
      setCheckErr(repoErr ?? "地址格式不正确");
      setCheckRes(null);
      return;
    }
    setCheckBusy(true);
    setCheckErr(null);
    setCheckRes(null);
    try {
      const d = await api.diagnoseGit(repoTrimmed);
      setCheckRes(d);
    } catch (e) {
      setCheckErr(String(e));
    } finally {
      setCheckBusy(false);
    }
  }

  async function run() {
    if (!mode || busy || result) return;
    if (!repo.trim()) {
      setError("请填写 Git 仓库地址");
      return;
    }
    if (!repoValid) {
      setError(repoErr ?? "Git 地址格式不正确：必须为 git@host:owner/repo.git");
      return;
    }
    if (!checkRes?.ok) {
      setError("请先点击「检测连通性」，确认仓库可以访问");
      return;
    }
    if (pwd.length < 6) {
      setError("主密码至少 6 位");
      return;
    }
    if (pwd !== pwd2) {
      setError("两次输入的密码不一致");
      return;
    }
    setBusy(true);
    setError(null);
    setResult(null);
    setProg(null);
    try {
      const msg = await api.onboard(mode, repo.trim(), pwd);
      // 持久化 AI 工具路径覆盖（收集/恢复时按这些目录读写）
      try {
        const c = await api.getSyncConfig();
        await api.saveSyncConfig({ ...c, ai_paths: aiPaths });
      } catch {}
      setResult(msg);
    } catch (e) {
      const msg = String(e);
      setError(msg);
      // 主密码不对：聚焦密码框，用户改完直接重试（无需重新下载）
      if (msg.includes("主密码不正确")) {
        pwdRef.current?.focus();
        pwdRef.current?.select();
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="h-screen flex items-center justify-center bg-[var(--color-nexus-bg)] p-6 overflow-y-auto">
      <div className="w-full max-w-3xl py-8">
        {/* Logo */}
        <div className="text-center mb-8">
          <div
            className="text-4xl mb-3 text-[var(--color-neon-purple)]"
            style={{ textShadow: "0 0 20px rgba(157,107,255,0.6)" }}
          >
            ◆
          </div>
          <h1 className="text-2xl font-semibold text-[var(--color-nexus-text)]">NEXUS Workbench</h1>
          <p className="font-mono text-[11px] text-[var(--color-nexus-text-faint)] mt-2 tracking-widest">
            首次使用设置
          </p>
        </div>

        {!mode ? (
          <>
            <p className="text-center font-mono text-xs text-[var(--color-nexus-text-dim)] mb-6">
              选择你的使用方式
            </p>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <ModeCard
                icon="⇣"
                title="恢复模式"
                desc="已有私有仓库，从云端恢复数据"
                detail="连接仓库 → 全量拉取 → 用主密码解密 → 生成 SSH 配置"
                accent="var(--color-neon-cyan)"
                onClick={() => setMode("recover")}
              />
              <ModeCard
                icon="⇡"
                title="全新模式"
                desc="第一次使用，把本机数据加密上传"
                detail="连接空仓库 → 用主密码加密本机数据 → 首次推送"
                accent="var(--color-neon-purple)"
                onClick={() => setMode("fresh")}
              />
            </div>
            <p className="text-center font-mono text-[10px] text-[var(--color-nexus-text-faint)] mt-6 leading-relaxed">
              两种模式都需要配置 Git 仓库地址与加密主密码。
              <br />
              主密码只保存在本机，用于加解密同步内容，请务必牢记。
            </p>
          </>
        ) : (
          <div className="surface p-6 space-y-4 max-w-xl mx-auto">
            <button
              className="font-mono text-[11px] text-[var(--color-nexus-text-dim)] hover:text-[var(--color-nexus-text)]"
              onClick={() => {
                setMode(null);
                setError(null);
                setResult(null);
              }}
              disabled={busy}
            >
              ← 返回选择
            </button>
            <div>
              <h2 className="text-lg font-semibold text-[var(--color-nexus-text)]">
                {mode === "recover" ? "恢复模式" : "全新模式"}
              </h2>
              <p className="font-mono text-[10.5px] text-[var(--color-nexus-text-faint)] mt-1">
                {mode === "recover"
                  ? "从已有仓库拉取并用主密码解密"
                  : "把本机数据用主密码加密后首次推送"}
              </p>
            </div>

            <Field label="Git 仓库地址（仅支持 git@host:owner/repo.git 格式）">
              <div className="flex gap-2">
                <input
                  value={repo}
                  onChange={(e) => {
                    setRepo(e.target.value);
                    setCheckRes(null);
                    setCheckErr(null);
                    setError(null);
                  }}
                  placeholder="git@github.com:you/your-private-repo.git"
                  disabled={busy}
                  className={`${inputCls} flex-1`}
                  spellCheck={false}
                  autoComplete="off"
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && repoValid) void checkRepo();
                  }}
                />
                <button
                  className="btn shrink-0"
                  onClick={checkRepo}
                  disabled={busy || checkBusy || !repoValid}
                  title="用 git ls-remote 检测仓库是否可访问"
                >
                  {checkBusy ? "检测中…" : "检测连通性"}
                </button>
              </div>
              {repoErr && (
                <p className="font-mono text-[10.5px] mt-1.5 text-red-300 break-words">
                  ✕ {repoErr}
                </p>
              )}
              {!repoErr && checkRes && (
                <p
                  className={`font-mono text-[10.5px] mt-1.5 break-words ${
                    checkRes.ok ? "text-[var(--color-status-on)]" : "text-red-300"
                  }`}
                >
                  {checkRes.ok ? "✓" : "✕"} {checkRes.message}
                </p>
              )}
              {!repoErr && checkErr && (
                <p className="font-mono text-[10.5px] mt-1.5 text-red-300 break-words">{checkErr}</p>
              )}
              <p className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] mt-1.5">
                仅支持 SSH 格式（git@host:用户名/仓库.git），不支持 http(s) 等其他格式；中文输入法可能吞掉 . 和 -
              </p>
            </Field>
            <Field label="加密主密码（至少 6 位）">
              <input
                ref={pwdRef}
                type="password"
                value={pwd}
                onChange={(e) => setPwd(e.target.value)}
                placeholder="用于加解密所有同步内容"
                disabled={busy}
                className={inputCls}
              />
            </Field>

            <div className="rounded-xl border border-white/5 bg-white/[0.02] px-4 py-3">
              <div className="font-mono text-[10px] tracking-[0.14em] text-[var(--color-nexus-text-faint)] uppercase mb-1">
                AI 会话数据路径（可选）
              </div>
              <p className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] mb-3 leading-relaxed">
                按当前系统已自动填入默认路径；如你的 AI 工具装在其他位置可修改，留空 = 系统默认。恢复时会写入这些目录
              </p>
              <div className="space-y-2.5">
                {([
                  ["zcode", "ZCode"],
                  ["workbuddy", "WorkBuddy"],
                  ["doubao", "豆包 (Doubao)"],
                  ["claude", "Claude Code"],
                  ["dsh", "DeepSeek Harness"],
                  ["cursor", "Cursor"],
                  ["trae", "Trae"],
                ] as const).map(([key, label]) => (
                  <div key={key} className="flex items-center gap-3">
                    <span className="font-mono text-[10.5px] text-[var(--color-nexus-text-dim)] w-40 shrink-0">{label}</span>
                    <input
                      value={aiPaths[key] ?? ""}
                      onChange={(e) => setAiPaths((prev) => ({ ...prev, [key]: e.target.value }))}
                      placeholder={aiDefaults?.[key as keyof AiPaths] ?? "系统默认"}
                      disabled={busy}
                      spellCheck={false}
                      className="flex-1 bg-white/[0.03] border border-white/5 rounded-lg px-2.5 py-1.5 font-mono text-[11px] text-[var(--color-nexus-text)] placeholder:text-[var(--color-nexus-text-faint)] focus:border-[var(--color-neon-purple)]/50 focus:outline-none transition"
                    />
                  </div>
                ))}
              </div>
            </div>

            <Field label="确认主密码">
              <input
                type="password"
                value={pwd2}
                onChange={(e) => setPwd2(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && run()}
                disabled={busy}
                className={inputCls}
              />
            </Field>

            {busy && (
              <div className="rounded-xl border border-[var(--color-neon-purple)]/25 bg-[var(--color-neon-purple)]/[0.06] px-3.5 py-3 space-y-2">
                <div className="flex items-center justify-between gap-3 font-mono text-[11px]">
                  <span className="text-[var(--color-nexus-text)] truncate">
                    {prog?.title || (mode === "recover" ? "恢复模式" : "全新模式")}
                    <span className="text-[var(--color-nexus-text-faint)]"> · </span>
                    <span className="text-[var(--color-nexus-text-dim)]">
                      {prog?.subtitle || "正在准备…"}
                    </span>
                  </span>
                  <span className="text-[var(--color-neon-purple)] tabular-nums shrink-0">{pct}%</span>
                </div>
                <div className="progress-track">
                  <div className="progress-fill" style={{ width: `${pct}%` }} />
                </div>
                <div className="flex items-center justify-between font-mono text-[10px] text-[var(--color-nexus-text-faint)]">
                  <span className="truncate">{prog?.eta || "请勿关闭窗口"}</span>
                  <span className="tabular-nums shrink-0">已用 {elapsed}s</span>
                </div>
              </div>
            )}

            {error && (
              <div className="rounded-lg border border-red-500/30 bg-red-500/5 px-3 py-2">
                <div className="font-mono text-[10.5px] text-red-300 mb-1">✕ 操作失败</div>
                <pre className="font-mono text-[10.5px] text-red-300/90 whitespace-pre-wrap break-words max-h-40 overflow-y-auto">
                  {error}
                </pre>
              </div>
            )}

            {result && (
              <div className="rounded-lg border border-[var(--color-status-on)]/30 bg-[var(--color-status-on)]/5 px-3 py-2.5">
                <div className="font-mono text-[11px] text-[var(--color-status-on)] break-words">
                  ✓ {result}
                </div>
                <div className="flex items-center justify-between gap-3 mt-2">
                  <span className="font-mono text-[10.5px] text-[var(--color-nexus-text-dim)]">
                    {countdown > 0 ? `${countdown} 秒后进入主界面…` : "正在进入主界面…"}
                  </span>
                  <button className="btn py-1 px-3 text-[11px]" onClick={() => onDoneRef.current()}>
                    立即进入
                  </button>
                </div>
              </div>
            )}

            <button
              className="btn btn--primary w-full py-2.5"
              disabled={busy || !!result}
              onClick={run}
            >
              {busy
                ? `处理中… ${pct}%`
                : mode === "recover"
                  ? "⇣ 开始恢复"
                  : "⇡ 加密并推送"}
            </button>

            <p className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] leading-relaxed">
              {mode === "recover"
                ? "将执行：验证仓库连接 → 拉取 → 解密到本机 → 重新生成 ~/.ssh/config"
                : "将执行：验证仓库连接（需为空仓库）→ 加密 assh.yml/笔记/AI 会话 → 首次推送"}
            </p>
          </div>
        )}
      </div>
    </div>
  );
}

function ModeCard({
  icon,
  title,
  desc,
  detail,
  accent,
  onClick,
}: {
  icon: string;
  title: string;
  desc: string;
  detail: string;
  accent: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className="surface surface-hover p-6 text-left transition-all"
      style={{ borderColor: `color-mix(in srgb, ${accent} 25%, transparent)` }}
    >
      <div className="text-2xl mb-3" style={{ color: accent }}>
        {icon}
      </div>
      <h3 className="text-base font-semibold text-[var(--color-nexus-text)] mb-1">{title}</h3>
      <p className="text-[12.5px] text-[var(--color-nexus-text-dim)] mb-2">{desc}</p>
      <p className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] leading-relaxed">
        {detail}
      </p>
    </button>
  );
}

const inputCls =
  "w-full bg-white/[0.03] border border-white/5 rounded-lg px-3 py-2 font-mono text-xs text-[var(--color-nexus-text)] placeholder:text-[var(--color-nexus-text-faint)] focus:border-[var(--color-neon-purple)]/50 focus:outline-none transition disabled:opacity-50";

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <label className="block font-mono text-[11px] text-[var(--color-nexus-text-dim)] mb-1.5">
        {label}
      </label>
      {children}
    </div>
  );
}
