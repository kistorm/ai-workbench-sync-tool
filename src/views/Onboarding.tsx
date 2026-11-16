import { useState } from "react";
import { api } from "../api";

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

  async function run() {
    if (!mode) return;
    if (!repo.trim()) {
      setError("请填写 Git 仓库地址");
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
    try {
      const msg = await api.onboard(mode, repo.trim(), pwd);
      setResult(msg);
      setTimeout(onDone, 1800);
    } catch (e) {
      setError(String(e));
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

            <Field label="Git 仓库地址">
              <input
                value={repo}
                onChange={(e) => setRepo(e.target.value)}
                placeholder="git@github.com:you/your-private-repo.git"
                disabled={busy}
                className={inputCls}
              />
            </Field>
            <Field label="加密主密码（至少 6 位）">
              <input
                type="password"
                value={pwd}
                onChange={(e) => setPwd(e.target.value)}
                placeholder="用于加解密所有同步内容"
                disabled={busy}
                className={inputCls}
              />
            </Field>
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

            {error && (
              <pre className="rounded-lg border border-red-500/30 bg-red-500/5 px-3 py-2 font-mono text-[10.5px] text-red-300 whitespace-pre-wrap break-words max-h-40 overflow-y-auto">
                {error}
              </pre>
            )}
            {result && (
              <div className="rounded-lg border border-[var(--color-status-on)]/30 bg-[var(--color-status-on)]/5 px-3 py-2 font-mono text-[11px] text-[var(--color-status-on)]">
                ✓ {result}
              </div>
            )}

            <button
              className="btn btn--primary w-full py-2.5"
              disabled={busy || !!result}
              onClick={run}
            >
              {busy
                ? "处理中（文件较多时需要一点时间）…"
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
