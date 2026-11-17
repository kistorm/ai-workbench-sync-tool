import { useEffect, useState } from "react";
import { api } from "../api";
import type { AppConfig, CronStatus, Diagnosis, ElectermInfo, RepoInfo, SshKeyInfo, SyncItemResult, SystemInfo, UpdateInfo } from "../api";
import { useAction, useAsync } from "../hooks";
import { ErrorBox, Loading, Modal, useToast } from "../components/ui";
import { THEMES, DEFAULT_APP_NAME } from "../theme";
import type { ThemeKey } from "../theme";

interface SyncViewProps {
  theme: ThemeKey;
  onThemeChange: (t: ThemeKey) => void;
  appName: string;
  onAppNameChange: (n: string) => void;
}

export function SyncView({ theme, onThemeChange, appName, onAppNameChange }: SyncViewProps) {
  const { push } = useToast();
  const cfgQ = useAsync(() => api.getSyncConfig(), []);
  const repo = useAsync(() => api.repoInfo(), [], { pollMs: 15000 });
  const cronQ = useAsync(() => api.cronStatus(), []);
  const { busy, run } = useAction();

  const [cfg, setCfg] = useState<AppConfig | null>(null);
  const [diagnosis, setDiagnosis] = useState<Diagnosis | null>(null);
  const [interval, setInterval] = useState(60);
  const [confirmRemoveCron, setConfirmRemoveCron] = useState(false);
  const [showLog, setShowLog] = useState(false);
  const [logLines, setLogLines] = useState<string[]>([]);
  const [keysOpen, setKeysOpen] = useState(false);
  const [keys, setKeys] = useState<SshKeyInfo[]>([]);
  const [lastSync, setLastSync] = useState<SyncItemResult[] | null>(null);
  const [electermInfo, setElectermInfo] = useState<ElectermInfo | null>(null);
  const [restoreOpen, setRestoreOpen] = useState(false);
  const [restoreConfirm, setRestoreConfirm] = useState(false);
  const [syncKeyVisible, setSyncKeyVisible] = useState(false);
  const [sysInfo, setSysInfo] = useState<SystemInfo | null>(null);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);

  useEffect(() => {
    if (cfgQ.data) {
      setCfg(cfgQ.data);
      setInterval(cfgQ.data.sync_interval_min || 60);
    }
  }, [cfgQ.data]);

  // 打开设置页时自动探测一次 Electerm
  useEffect(() => {
    void api.detectElecterm().then(setElectermInfo).catch(() => {});
  }, []);

  // 挂载时读取一次系统信息
  useEffect(() => {
    void api.getSystemInfo().then(setSysInfo).catch(() => {});
  }, []);

  async function checkUpdate() {
    setChecking(true);
    try {
      const info = await api.checkUpdate();
      setUpdateInfo(info);
    } catch (e) {
      push("err", "检查更新失败", String(e));
    } finally {
      setChecking(false);
    }
  }

  async function openRelease(url: string) {
    try {
      await api.openUrl(url);
    } catch (e) {
      push("err", "打开链接失败", String(e));
    }
  }

  function update<K extends keyof AppConfig>(key: K, value: AppConfig[K]) {
    setCfg((prev) => (prev ? { ...prev, [key]: value } : prev));
  }

  async function detectElecterm() {
    try {
      const info = await api.detectElecterm();
      setElectermInfo(info);
      if (info.found) {
        update("electerm_cmd", info.path);
        push("ok", "已找到 Electerm", `${info.path}${info.version ? " · " + info.version : ""}`);
      } else {
        push("err", "未检测到 Electerm", "请安装，或手动填写程序路径");
      }
    } catch (e) {
      push("err", "检测失败", String(e));
    }
  }

  async function openKeyPicker() {
    try {
      const list = await api.listSshKeys();
      setKeys(list);
      setKeysOpen(true);
      if (list.length === 0) {
        push("info", "未找到密钥", "~/.ssh 下没有检测到私钥文件");
      }
    } catch (e) {
      push("err", "读取密钥失败", String(e));
    }
  }

  async function saveConfig() {
    if (!cfg) return;
    // Git 地址严格校验：仅允许 git@host:owner/repo.git
    const gr = cfg.git_repo.trim();
    if (gr && !/^git@[^\s:/]+\.[^\s:/]+:[^\s:/]+\/[^\s:/]+\.git$/.test(gr)) {
      push(
        "err",
        "仓库地址格式不正确",
        "仅支持 SSH 格式 git@host:用户名/仓库.git，例如 git@github.com:you/your-repo.git；不支持 http(s) 等其他格式。"
      );
      return;
    }
    const oldPwd = cfgQ.data?.encrypt_password ?? "";
    const pwdChanged = oldPwd !== cfg.encrypt_password;
    const r = await run("save", () => api.saveSyncConfig(cfg));
    if (r.ok) {
      push("ok", "设置已保存");
      if (pwdChanged && cfg.encrypt_password) {
        push(
          "info",
          "主密码已变更",
          "请立即执行一次「立即全量推送」，用新密码重新加密云端内容；否则下次拉取会因密码不匹配而解密失败。"
        );
      }
      void cfgQ.reload();
      void repo.reload();
    } else push("err", "保存失败", r.error);
  }

  async function diagnose() {
    const r = await run("diag", () =>
      api.diagnoseGit(cfg?.git_repo ?? undefined, cfg?.ssh_key ?? undefined)
    );
    if (r.ok && r.data) {
      setDiagnosis(r.data);
      push(r.data.ok ? "ok" : "err", "通信诊断", r.data.message);
    } else push("err", "诊断失败", r.error);
  }

  async function doForceRestore() {
    if (!restoreConfirm) return;
    const r = await run("restore", () => api.forceRestore());
    if (r.ok) {
      push("ok", "已从云端强制恢复", r.data);
      setRestoreOpen(false);
      setRestoreConfirm(false);
      void repo.reload();
      void cfgQ.reload();
    } else push("err", "恢复失败", r.error);
  }

  async function doSyncAll(action: "push" | "pull") {
    const label = action === "push" ? "推送" : "拉取";
    const r = await run("all", () => api.syncAll(action));
    if (r.ok && r.data) {
      setLastSync(r.data);
      const failed = r.data.filter((x) => !x.ok);
      const detail = r.data
        .map((x) => `${x.ok ? "✓" : "✗"} ${x.name}: ${x.detail}`)
        .join("\n");
      if (failed.length === 0) push("ok", `全量${label}成功`, detail);
      else push("err", `全量${label}：${failed.length} 项失败`, detail);
      void repo.reload();
    } else push("err", `全量${label}失败`, r.error);
  }

  async function initRepo() {
    const r = await run("init", () => api.initDataRepo());
    if (r.ok) {
      push("ok", "数据仓库已初始化", r.data);
      void repo.reload();
    } else push("err", "初始化失败", r.error);
  }

  async function installCron() {
    const r = await run("cron", () => api.cronInstall(interval));
    if (r.ok && r.data) {
      push("ok", `定时同步已安装（每 ${interval} 分钟）`, r.data.line);
      void cronQ.reload();
      void cfgQ.reload();
    } else push("err", "安装定时任务失败", r.error);
  }

  async function removeCron() {
    const r = await run("cronrm", () => api.cronRemove());
    if (r.ok) {
      push("ok", "定时同步已移除");
      setConfirmRemoveCron(false);
      void cronQ.reload();
    } else push("err", "移除定时任务失败", r.error);
  }

  async function loadLog() {
    try {
      const lines = await api.syncLogTail(80);
      setLogLines(lines);
      setShowLog(true);
    } catch (e) {
      push("err", "读取日志失败", String(e));
    }
  }

  const cron: CronStatus | undefined = cronQ.data;
  const repository: RepoInfo | undefined = repo.data;

  return (
    <div className="page-enter h-full flex flex-col gap-5">
      <header className="flex items-center justify-between shrink-0">
        <div>
          <h1 className="text-lg font-semibold text-[var(--color-nexus-text)]">设置</h1>
          <p className="font-mono text-[11px] text-[var(--color-nexus-text-faint)] mt-0.5">
            外观 · 工作台名称 · Git 同步 · 定时任务
          </p>
        </div>
        <div className="flex gap-2">
          <button className="btn" disabled={!!busy} onClick={loadLog}>
            同步日志
          </button>
          <button className="btn btn--primary" disabled={!!busy} onClick={() => doSyncAll("push")}>
            {busy === "all" ? "同步中…" : "立即全量推送"}
          </button>
        </div>
      </header>

      {(cfgQ.error || repo.error || cronQ.error) && (
        <ErrorBox message={cfgQ.error || repo.error || cronQ.error || ""} />
      )}
      {cfgQ.loading && !cfg && <Loading />}

      <div className="flex-1 min-h-0 overflow-y-auto space-y-4 pb-2">
        {cfg && (
          <>
            {/* 外观 */}
            <section className="surface p-5">
              <h3 className="font-mono text-[10px] tracking-[0.14em] text-[var(--color-nexus-text-faint)] uppercase mb-3">
                外观
              </h3>

              {/* 主题选择 */}
              <div className="grid grid-cols-5 gap-2.5">
                {THEMES.map((t) => (
                  <button
                    key={t.key}
                    onClick={() => onThemeChange(t.key)}
                    className={`theme-card text-left ${theme === t.key ? "theme-card--active" : ""}`}
                  >
                    {/* 色板预览：固定色值，不随当前主题变化 */}
                    <div className="flex gap-1 mb-2.5">
                      <span
                        className="h-5 flex-1 rounded-l-md border border-white/10"
                        style={{ background: t.preview.bg }}
                      />
                      <span
                        className="h-5 flex-1 border-y border-white/10"
                        style={{ background: t.preview.card }}
                      />
                      <span
                        className="h-5 flex-1 rounded-r-md border border-white/10"
                        style={{ background: t.preview.accent }}
                      />
                    </div>
                    <div className="text-[12px] text-[var(--color-nexus-text)]">{t.name}</div>
                    <div className="font-mono text-[9.5px] text-[var(--color-nexus-text-faint)] mt-0.5 leading-snug">
                      {t.desc}
                    </div>
                    {theme === t.key && (
                      <div className="font-mono text-[9px] text-[var(--color-neon-purple)] mt-1.5">● 当前</div>
                    )}
                  </button>
                ))}
              </div>

              <div className="h-px bg-white/5 my-4" />

              {/* 工作台名称 */}
              <div>
                <label className="block font-mono text-[11px] text-[var(--color-nexus-text-dim)] mb-1.5">
                  工作台名称
                </label>
                <div className="flex gap-2">
                  <input
                    value={appName}
                    onChange={(e) => onAppNameChange(e.target.value)}
                    maxLength={20}
                    placeholder={DEFAULT_APP_NAME}
                    className={`${inputCls} flex-1`}
                  />
                  <button
                    className="btn shrink-0"
                    onClick={() => onAppNameChange(DEFAULT_APP_NAME)}
                    disabled={appName === DEFAULT_APP_NAME}
                  >
                    恢复默认
                  </button>
                </div>
                <p className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] mt-1.5">
                  显示在左上角与窗口标题，修改即时生效；留空自动恢复「{DEFAULT_APP_NAME}」
                </p>
              </div>
            </section>

            {/* 系统与升级 */}
            <section className="surface p-5">
              <div className="flex items-center justify-between mb-3">
                <h3 className="font-mono text-[10px] tracking-[0.14em] text-[var(--color-nexus-text-faint)] uppercase">
                  系统与升级
                </h3>
                <button className="btn" disabled={checking} onClick={checkUpdate}>
                  {checking ? "检查中…" : "检查更新"}
                </button>
              </div>
              <div className="grid grid-cols-2 gap-x-6 gap-y-2 font-mono text-xs">
                <KV k="操作系统" v={sysInfo?.os_type ?? "—"} />
                <KV k="系统版本" v={sysInfo?.os_version ?? "—"} />
                <KV k="登录账户" v={sysInfo?.username ?? "—"} />
                <KV k="应用版本" v={sysInfo?.app_version ?? "—"} />
              </div>
              {updateInfo && (
                <div className="mt-3 pt-3 border-t border-white/5 flex items-center justify-between gap-3">
                  <span className="font-mono text-[11px] text-[var(--color-nexus-text-dim)]">
                    {updateInfo.message}
                  </span>
                  {updateInfo.has_update && updateInfo.latest && (
                    <button className="btn shrink-0" onClick={() => openRelease(updateInfo.release_url)}>
                      ⇣ 前往下载 v{updateInfo.latest}
                    </button>
                  )}
                </div>
              )}
            </section>

            {/* 网络加速推荐 */}
            <section className="surface px-4 py-3 flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-[12.5px] text-[var(--color-nexus-text)]">网络加速推荐</div>
                <p className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] mt-0.5">
                  稳定访问 GitHub / 加速 clone 与推送，推荐 xx 云加速服务
                </p>
              </div>
              <div className="shrink-0 flex flex-col items-end gap-1">
                <button
                  className="btn"
                  onClick={() => openRelease("https://www.xx-yun.com/?code=HxaXgLvi")}
                >
                  打开 xx-yun
                </button>
                <span className="font-mono text-[9px] text-[var(--color-nexus-text-faint)]">
                  推广支持 · 感谢使用
                </span>
              </div>
            </section>

            {/* Git 仓库 */}
            <section className="surface p-5">
              <h3 className="font-mono text-[10px] tracking-[0.14em] text-[var(--color-nexus-text-faint)] uppercase mb-3">
                Git 私库
              </h3>
              <div className="space-y-3">
                <div>
                  <label className="block font-mono text-[11px] text-[var(--color-nexus-text-dim)] mb-1.5">
                    仓库地址
                  </label>
                  <input
                    value={cfg.git_repo}
                    onChange={(e) => update("git_repo", e.target.value)}
                    placeholder="git@github.com:you/your-repo.git"
                    className={inputCls}
                  />
                  <p className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] mt-1">
                    仅支持 SSH 格式：git@host:用户名/仓库.git
                  </p>
                </div>
                <div>
                  <label className="block font-mono text-[11px] text-[var(--color-nexus-text-dim)] mb-1.5">
                    SSH 密钥
                  </label>
                  <div className="flex gap-2">
                    <input
                      value={cfg.ssh_key}
                      onChange={(e) => update("ssh_key", e.target.value)}
                      className={`${inputCls} flex-1`}
                    />
                    <button className="btn shrink-0" onClick={openKeyPicker}>
                      读取系统
                    </button>
                  </div>
                </div>
                <div>
                  <label className="block font-mono text-[11px] text-[var(--color-nexus-text-dim)] mb-1.5">
                    加密主密码（可选 · 全量加密同步内容）
                  </label>
                  <div className="flex gap-2">
                    <input
                      type={syncKeyVisible ? "text" : "password"}
                      value={cfg.encrypt_password}
                      onChange={(e) => update("encrypt_password", e.target.value)}
                      placeholder="留空则明文同步；设置后全部内容加密"
                      className={`${inputCls} flex-1`}
                    />
                    <button className="btn shrink-0" onClick={() => setSyncKeyVisible(!syncKeyVisible)}>
                      {syncKeyVisible ? "隐藏" : "显示"}
                    </button>
                  </div>
                  <p className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] mt-1.5">
                    设置后，<span className="text-[var(--color-nexus-text)]">SSH 配置、笔记、AI 会话</span>会逐文件加密为
                    <span className="text-[var(--color-neon-cyan)]"> encrypted/**/*.enc </span>
                    再推送；拉取时用相同主密码解密还原。主密码本身不入库，请务必牢记。
                  </p>
                </div>
                <div className="flex justify-end">
                  <button className="btn" disabled={!!busy} onClick={saveConfig}>
                    {busy === "save" ? "保存中…" : "保存配置"}
                  </button>
                </div>
              </div>
            </section>

            {/* 通信诊断 */}
            <section className="surface p-5">
              <div className="flex items-center justify-between mb-3">
                <h3 className="font-mono text-[10px] tracking-[0.14em] text-[var(--color-nexus-text-faint)] uppercase">
                  通信诊断
                </h3>
                <button className="btn" disabled={!!busy} onClick={diagnose}>
                  {busy === "diag" ? "诊断中…" : "诊断"}
                </button>
              </div>
              <div className="flex items-center gap-2.5">
                <span
                  className={`dot ${
                    diagnosis ? (diagnosis.ok ? "dot--on" : "dot--off") : "dot--half"
                  }`}
                />
                <span className="font-mono text-xs text-[var(--color-nexus-text)]">
                  {diagnosis ? diagnosis.message : "尚未诊断"}
                </span>
              </div>
              {diagnosis && !diagnosis.ok && diagnosis.raw && (
                <pre className="mt-2 max-h-24 overflow-auto rounded-lg bg-black/30 p-2 font-mono text-[10px] text-[var(--color-nexus-text-faint)] whitespace-pre-wrap">
                  {diagnosis.raw}
                </pre>
              )}
            </section>

            {/* 仓库状态 */}
            <section className="surface p-5">
              <div className="flex items-center justify-between mb-3">
                <h3 className="font-mono text-[10px] tracking-[0.14em] text-[var(--color-nexus-text-faint)] uppercase">
                  同步仓库
                </h3>
                <div className="flex gap-2">
                  <button className="btn" disabled={!!busy} onClick={() => doSyncAll("pull")}>
                    {busy === "all" ? "拉取中…" : "⇣ 全量拉取"}
                  </button>
                  <button
                    className="btn !text-[var(--color-status-warn)] border-[var(--color-status-warn)]/40"
                    disabled={!!busy}
                    onClick={() => {
                      setRestoreConfirm(false);
                      setRestoreOpen(true);
                    }}
                    title="丢弃本地未推送修改，用云端版本覆盖"
                  >
                    从云端强制恢复
                  </button>
                  <button className="btn" disabled={!!busy} onClick={() => doSyncAll("push")}>
                    {busy === "all" ? "推送中…" : "⇡ 全量推送"}
                  </button>
                </div>
              </div>
              {repository ? (
                <>
                  <div className="grid grid-cols-2 gap-x-6 gap-y-2 font-mono text-xs">
                    <KV k="目录" v={repository.dir} />
                    <KV k="分支" v={repository.branch || "—"} />
                    <KV k="远程" v={repository.remote || "—"} />
                    <KV k="未提交" v={`${repository.dirty} 个文件`} />
                    <KV k="最近提交" v={repository.last_commit || "—"} className="col-span-2" />
                  </div>
                  {!repository.is_git && (
                    <div className="mt-3 flex items-center justify-between gap-3 rounded-lg border border-[var(--color-status-warn)]/30 bg-[var(--color-status-warn)]/5 px-3 py-2">
                      <span className="font-mono text-[11px] text-[var(--color-status-warn)]">
                        数据仓库尚未初始化，无法推送
                      </span>
                      <button className="btn shrink-0" disabled={!!busy} onClick={initRepo}>
                        {busy === "init" ? "初始化中…" : "初始化仓库"}
                      </button>
                    </div>
                  )}
                  {repository.is_git && !repository.remote && (
                    <p className="mt-2 font-mono text-[10.5px] text-[var(--color-status-warn)]">
                      ⚠ 尚未绑定远程地址，请填写上方「仓库地址」并保存
                    </p>
                  )}
                </>
              ) : (
                <Loading text="读取仓库信息…" />
              )}

              {lastSync && (
                <div className="mt-4 pt-3 border-t border-white/5 space-y-1.5">
                  <div className="font-mono text-[10px] tracking-[0.12em] text-[var(--color-nexus-text-faint)] uppercase mb-1">
                    上次同步结果
                  </div>
                  {lastSync.map((item) => (
                    <div key={item.name} className="flex items-start gap-2 font-mono text-[10.5px]">
                      <span className={item.ok ? "text-[var(--color-status-on)]" : "text-[var(--color-status-off)]"}>
                        {item.ok ? "✓" : "✗"}
                      </span>
                      <span className="text-[var(--color-nexus-text)] shrink-0 w-32">{item.name}</span>
                      <span className="text-[var(--color-nexus-text-dim)] break-all">{item.detail}</span>
                    </div>
                  ))}
                </div>
              )}

              <p className="mt-3 font-mono text-[10px] text-[var(--color-nexus-text-faint)] leading-relaxed">
                说明：SSH 配置与笔记同属一个数据仓库（一次推送全部上传）；AI 会话由其自身的
                sync.sh 管理独立仓库，全量操作时也会一并执行。
              </p>
            </section>

            {/* 同步策略 */}
            <section className="surface p-5">
              <h3 className="font-mono text-[10px] tracking-[0.14em] text-[var(--color-nexus-text-faint)] uppercase mb-3">
                同步策略
              </h3>
              <div className="space-y-4">
                <div className="flex items-center justify-between">
                  <div>
                    <div className="text-[13px] text-[var(--color-nexus-text)]">自动同步间隔</div>
                    <div className="font-mono text-[10.5px] text-[var(--color-nexus-text-faint)] mt-0.5">
                      每隔 N 分钟向云端推送一次（写入 crontab）
                    </div>
                  </div>
                  <div className="flex items-center gap-2">
                    <input
                      type="number"
                      min={1}
                      value={interval}
                      onChange={(e) => setInterval(Number(e.target.value))}
                      className="w-20 bg-white/[0.03] border border-white/5 rounded-lg px-3 py-1.5 font-mono text-xs text-[var(--color-nexus-text)] focus:border-[var(--color-neon-purple)]/50 focus:outline-none transition"
                    />
                    <span className="font-mono text-[11px] text-[var(--color-nexus-text-dim)]">min</span>
                  </div>
                </div>
                <div className="h-px bg-white/5" />
                <div className="flex items-center justify-between">
                  <div>
                    <div className="text-[13px] text-[var(--color-nexus-text)]">冲突策略</div>
                    <div className="font-mono text-[10.5px] text-[var(--color-nexus-text-faint)] mt-0.5">
                      文本按修改时间取新 · SQLite 整文件本机优先
                    </div>
                  </div>
                  <span className="font-mono text-[11.5px] text-[var(--color-neon-lime)]">最新覆盖旧</span>
                </div>
                <div className="h-px bg-white/5" />
                <div className="flex items-center justify-between">
                  <div>
                    <div className="text-[13px] text-[var(--color-nexus-text)]">Cron 任务</div>
                    <div className="font-mono text-[10.5px] text-[var(--color-nexus-text-faint)] mt-0.5">
                      {cron?.installed ? cron.line : "未安装 · 应用内定时器仍生效"}
                    </div>
                  </div>
                  <div className="flex items-center gap-2">
                    <span className={`dot ${cron?.installed ? "dot--on" : "dot--idle"}`} />
                    <span
                      className={`font-mono text-[11px] ${
                        cron?.installed ? "text-[var(--color-status-on)]" : "text-[var(--color-nexus-text-dim)]"
                      }`}
                    >
                      {cron?.installed ? "已安装" : "未安装"}
                    </span>
                    <button className="btn ml-2" disabled={!!busy} onClick={installCron}>
                      {busy === "cron" ? "安装中…" : "安装/更新"}
                    </button>
                    {cron?.installed && (
                      <button className="btn" onClick={() => setConfirmRemoveCron(true)}>
                        移除
                      </button>
                    )}
                  </div>
                </div>
                <div className="h-px bg-white/5" />
                <div className="flex items-center justify-between">
                  <div>
                    <div className="text-[13px] text-[var(--color-nexus-text)]">运行状态</div>
                    <div className="font-mono text-[10.5px] text-[var(--color-nexus-text-faint)] mt-0.5">
                      上次同步 {repository?.last_commit || "—"}
                    </div>
                  </div>
                  <div className="flex items-center gap-2">
                    <span className={`dot ${cfg.auto_sync_enabled ? "dot--on" : "dot--idle"}`} />
                    <span className="font-mono text-[11px] text-[var(--color-nexus-text-dim)]">
                      {cfg.auto_sync_enabled ? "自动同步开启" : "自动同步关闭"}
                    </span>
                  </div>
                </div>
              </div>
            </section>

            {/* 路径配置 */}
            <section className="surface p-5">
              <h3 className="font-mono text-[10px] tracking-[0.14em] text-[var(--color-nexus-text-faint)] uppercase mb-3">
                路径配置
              </h3>
              <div className="space-y-3">
                <PathField
                  label="SSH 配置目录"
                  value={cfg.ssh_dir}
                  onChange={(v) => update("ssh_dir", v)}
                />
                <PathField
                  label="AI 会话仓库"
                  value={cfg.ai_sync_dir}
                  onChange={(v) => update("ai_sync_dir", v)}
                />
                <PathField
                  label="笔记目录"
                  value={cfg.notes_dir}
                  onChange={(v) => update("notes_dir", v)}
                />
                <div className="flex items-center gap-3">
                  <label className="w-28 shrink-0 font-mono text-[11px] text-[var(--color-nexus-text-dim)]">
                    Electerm 命令
                  </label>
                  <input
                    value={cfg.electerm_cmd}
                    onChange={(e) => update("electerm_cmd", e.target.value)}
                    placeholder="/opt/electerm/electerm"
                    className={`${inputCls} flex-1`}
                  />
                  <button className="btn shrink-0" onClick={detectElecterm}>
                    自动检测
                  </button>
                </div>
                <div className="flex items-center gap-1.5 ml-[124px] font-mono text-[10px]">
                  <span className={`dot ${electermInfo?.found ? "dot--on" : "dot--warn"}`} />
                  <span className="text-[var(--color-nexus-text-dim)] truncate">
                    {electermInfo
                      ? electermInfo.found
                        ? `已找到：${electermInfo.path}${electermInfo.version ? " · " + electermInfo.version : ""}（来源：${electermInfo.source}）`
                        : "未检测到 Electerm（可从 github.com/electerm/electerm/releases 下载）"
                      : "检测中…"}
                  </span>
                </div>
              </div>
              <div className="flex justify-end mt-3">
                <button className="btn" disabled={!!busy} onClick={saveConfig}>
                  保存配置
                </button>
              </div>
            </section>
          </>
        )}
      </div>

      <Modal
        open={keysOpen}
        title="选择 SSH 密钥"
        onClose={() => setKeysOpen(false)}
        footer={
          <button className="btn" onClick={() => setKeysOpen(false)}>
            关闭
          </button>
        }
      >
        <div className="space-y-2">
          <p className="font-mono text-[10.5px] text-[var(--color-nexus-text-faint)] mb-3">
            从 {cfg?.ssh_dir ?? "~/.ssh"} 中检测到的私钥：
          </p>
          {keys.length === 0 && (
            <p className="font-mono text-[11px] text-[var(--color-nexus-text-dim)]">未找到私钥文件</p>
          )}
          {keys.map((k) => (
            <button
              key={k.path}
              onClick={() => {
                update("ssh_key", k.path);
                setKeysOpen(false);
                push("ok", "已选择密钥", k.name);
              }}
              className={`w-full flex items-center justify-between px-3 py-2.5 rounded-lg text-left transition-colors ${
                cfg?.ssh_key === k.path ? "bg-white/[0.06]" : "hover:bg-white/[0.03]"
              }`}
            >
              <div className="min-w-0">
                <div className="font-mono text-[12.5px] text-[var(--color-nexus-text)] truncate">{k.name}</div>
                <div className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] truncate">{k.path}</div>
              </div>
              <div className="flex items-center gap-2 shrink-0 ml-3">
                <span className="font-mono text-[10px] text-[var(--color-neon-cyan)]">{k.kind}</span>
                {k.has_pub && <span className="font-mono text-[10px] text-[var(--color-status-on)]">.pub</span>}
              </div>
            </button>
          ))}
        </div>
      </Modal>

      <Modal
        open={confirmRemoveCron}
        title="移除定时同步"
        onClose={() => setConfirmRemoveCron(false)}
        footer={
          <>
            <button className="btn" onClick={() => setConfirmRemoveCron(false)}>
              取消
            </button>
            <button className="btn !text-red-300 border-red-400/30" disabled={!!busy} onClick={removeCron}>
              确认移除
            </button>
          </>
        }
      >
        <p className="text-[13px] text-[var(--color-nexus-text)]">
          将从 crontab 中删除 NEXUS Workbench 的定时同步任务。
        </p>
        <p className="font-mono text-[10.5px] text-[var(--color-nexus-text-faint)] mt-2">
          应用内定时器仍会在应用运行时生效（除非关闭自动同步）。
        </p>
      </Modal>

      <Modal
        open={restoreOpen}
        title="从云端强制恢复"
        onClose={() => setRestoreOpen(false)}
        footer={
          <>
            <button className="btn" onClick={() => setRestoreOpen(false)}>
              取消
            </button>
            <button
              className="btn !text-red-300 border-red-400/40 bg-red-500/10"
              disabled={!restoreConfirm || !!busy}
              onClick={doForceRestore}
            >
              {busy === "restore" ? "恢复中…" : "确认强制恢复"}
            </button>
          </>
        }
      >
        <div className="space-y-3">
          <div className="rounded-lg border border-[var(--color-status-warn)]/30 bg-[var(--color-status-warn)]/[0.06] p-3">
            <ul className="font-mono text-[10.5px] leading-relaxed text-[var(--color-nexus-text-dim)] space-y-1">
              <li>· 将执行 <span className="text-[var(--color-nexus-text)]">git reset --hard origin/main</span></li>
              <li>· <span className="text-[var(--color-status-warn)]">丢弃本地未推送的所有修改</span>（SSH 配置、笔记、AI 会话数据）</li>
              <li>· 用云端的版本完整覆盖本地，并自动重建 ~/.ssh/config</li>
              <li>· 适用于：本地配置误删、换机器后想强制对齐云端</li>
            </ul>
          </div>
          <label className="flex items-start gap-2.5 p-3 rounded-lg border border-white/5 bg-white/[0.02] cursor-pointer">
            <input
              type="checkbox"
              checked={restoreConfirm}
              onChange={(e) => setRestoreConfirm(e.target.checked)}
              className="mt-0.5 accent-red-500"
            />
            <span className="text-[12px] text-[var(--color-nexus-text)]">
              我确认本地未推送的修改可以丢弃，要用云端版本覆盖
            </span>
          </label>
        </div>
      </Modal>

      <Modal
        open={showLog}
        title="同步日志（尾部）"
        onClose={() => setShowLog(false)}
        footer={
          <button className="btn" onClick={() => setShowLog(false)}>
            关闭
          </button>
        }
      >
        <pre className="max-h-[50vh] overflow-auto rounded-lg bg-black/40 p-3 font-mono text-[10.5px] leading-relaxed text-[var(--color-nexus-text-dim)] whitespace-pre-wrap">
          {logLines.length > 0 ? logLines.join("\n") : "（暂无日志，执行一次同步后生成）"}
        </pre>
      </Modal>
    </div>
  );
}

const inputCls =
  "w-full bg-white/[0.03] border border-white/5 rounded-lg px-3 py-2 font-mono text-xs text-[var(--color-nexus-text)] placeholder:text-[var(--color-nexus-text-faint)] focus:border-[var(--color-neon-purple)]/50 focus:outline-none transition";

function KV({ k, v, className = "" }: { k: string; v: string; className?: string }) {
  return (
    <div className={`flex justify-between gap-4 ${className}`}>
      <span className="text-[var(--color-nexus-text-faint)] shrink-0">{k}</span>
      <span className="text-[var(--color-nexus-text)] text-right truncate" title={v}>
        {v}
      </span>
    </div>
  );
}

function PathField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <div className="flex items-center gap-3">
      <label className="w-28 shrink-0 font-mono text-[11px] text-[var(--color-nexus-text-dim)]">{label}</label>
      <input value={value} onChange={(e) => onChange(e.target.value)} className={`${inputCls} flex-1`} />
    </div>
  );
}
