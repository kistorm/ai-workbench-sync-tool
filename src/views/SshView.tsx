import { useEffect, useMemo, useState } from "react";
import { api } from "../api";
import type { SshKeyInfo, TestResult } from "../api";
import { useAction, useAsync } from "../hooks";
import { ErrorBox, Loading, Modal, useToast } from "../components/ui";
import { useAppTasks } from "../store";

export function SshView() {
  const { push } = useToast();
  const hosts = useAsync(() => api.listHosts(), [], { pollMs: 10000 });
  const { busy, run } = useAction();
  const [selected, setSelected] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [testResult, setTestResult] = useState<Record<string, TestResult>>({});
  const [addOpen, setAddOpen] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const [yml, setYml] = useState("");
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  // 终端状态在 App 级（切页不断开），这里只负责发起
  const { openTerminal } = useAppTasks();
  const [pwdOpen, setPwdOpen] = useState(false);
  const [pwdValue, setPwdValue] = useState("");
  const [electermMissing, setElectermMissing] = useState(false);
  const [hostEditOpen, setHostEditOpen] = useState(false);
  const [editAlias, setEditAlias] = useState("");
  const [editHostname, setEditHostname] = useState("");
  const [editUser, setEditUser] = useState("");
  const [editPort, setEditPort] = useState("22");
  const [editTrust, setEditTrust] = useState(true);
  const [pwdVisible, setPwdVisible] = useState(false);
  const [forcePullOpen, setForcePullOpen] = useState(false);
  const [forcePullConfirm, setForcePullConfirm] = useState(false);

  const filtered = useMemo(() => {
    const list = hosts.data ?? [];
    const q = query.trim().toLowerCase();
    if (!q) return list;
    return list.filter(
      (h) =>
        h.alias.toLowerCase().includes(q) ||
        h.hostname.toLowerCase().includes(q) ||
        h.user.toLowerCase().includes(q)
    );
  }, [hosts.data, query]);

  const host = (hosts.data ?? []).find((h) => h.alias === selected) ?? null;

  async function doTest(alias: string) {
    const r = await run("test", () => api.testHost(alias));
    if (r.ok && r.data) {
      setTestResult((prev) => ({ ...prev, [alias]: r.data! }));
      push(r.data.reachable ? "ok" : "err", `${alias}: ${r.data.message}`, `${r.data.latency_ms}ms`);
    } else {
      push("err", `${alias} 测试失败`, r.error);
    }
  }

  async function doConnect(alias: string) {
    const r = await run("connect", () => api.openElecterm(alias));
    if (r.ok) push("ok", "已在 Electerm 中打开", r.data);
    else if (r.error?.includes("NOT_INSTALLED")) setElectermMissing(true);
    else push("err", "打开 Electerm 失败", r.error);
  }

  async function doBuild() {
    const r = await run("build", () => api.buildSshConfig());
    if (r.ok) {
      push("ok", "已生成 ~/.ssh/config", r.data);
      void hosts.reload();
    } else push("err", "生成失败", r.error);
  }

  async function openEditor() {
    const r = await run("read", () => api.readAsshYml());
    if (r.ok && r.data !== undefined) {
      setYml(r.data);
      setEditOpen(true);
    } else push("err", "读取 assh.yml 失败", r.error);
  }

  async function saveEditor() {
    const r = await run("save", () => api.saveAsshYml(yml));
    if (r.ok) {
      push("ok", "assh.yml 已保存");
      setEditOpen(false);
      void hosts.reload();
    } else push("err", "保存失败", r.error);
  }

  async function openPwdModal() {
    if (!host) return;
    setPwdValue("");
    setPwdVisible(false);
    setPwdOpen(true);
    try {
      const pw = await api.revealHostPassword(host.alias);
      setPwdValue(pw);
    } catch {
      /* 没有保存密码，保持空白 */
    }
  }

  function openEditModal() {
    if (!host) return;
    setEditAlias(host.alias);
    setEditHostname(host.hostname);
    setEditUser(host.user);
    setEditPort(String(host.port));
    setEditTrust(host.trust ?? true);
    setHostEditOpen(true);
  }

  async function saveEdit() {
    if (!host) return;
    if (!editAlias.trim() || !editHostname.trim()) {
      push("err", "别名和地址不能为空");
      return;
    }
    const r = await run("edit", () =>
      api.updateHost({
        alias: host.alias,
        newAlias: editAlias.trim(),
        hostname: editHostname.trim(),
        user: editUser.trim() || "root",
        port: Number(editPort) || 22,
        trustHost: editTrust,
      })
    );
    if (r.ok) {
      push("ok", "主机信息已更新");
      setHostEditOpen(false);
      void hosts.reload();
    } else push("err", "更新失败", r.error);
  }

  async function doForcePull() {
    if (!forcePullConfirm) return;
    const r = await run("force-pull", () => api.forcePullSshConfig());
    if (r.ok) {
      push("ok", "SSH 配置已从云端恢复", r.data);
      setForcePullOpen(false);
      setForcePullConfirm(false);
      void hosts.reload();
    } else push("err", "恢复失败", r.error);
  }

  async function doPushSsh() {
    const r = await run("push-ssh", () => api.pushSshConfig());
    if (r.ok) {
      push("ok", "SSH 配置已推送到仓库", r.data);
    } else push("err", "推送失败", r.error);
  }

  async function savePassword() {
    if (!host) return;
    if (!pwdValue) {
      push("err", "密码不能为空");
      return;
    }
    const r = await run("pwd", () => api.setHostPassword(host.alias, pwdValue));
    if (r.ok) {
      push("ok", `已更新 ${host.alias} 的密码`);
      setPwdOpen(false);
      setPwdValue("");
    } else push("err", "保存密码失败", r.error);
  }

  async function doDelete(alias: string) {
    const r = await run("delete", () => api.deleteHost(alias));
    if (r.ok) {
      push("ok", `已删除 ${alias}`);
      setConfirmDelete(null);
      if (selected === alias) setSelected(null);
      void hosts.reload();
    } else push("err", "删除失败", r.error);
  }

  return (
    <div className="page-enter h-full flex flex-col gap-5">
      <header className="flex items-center justify-between shrink-0">
        <div>
          <h1 className="text-lg font-semibold text-[var(--color-nexus-text)]">SSH 连接</h1>
          <p className="font-mono text-[11px] text-[var(--color-nexus-text-faint)] mt-0.5">
            配置由 assh.yml 托管 · 生成标准 ~/.ssh/config
          </p>
        </div>
        <div className="flex gap-2">
          <button className="btn" disabled={!!busy} onClick={doPushSsh} title="把 assh.yml 和主机密码库加密推送到仓库（不影响笔记/AI 会话）">
            {busy === "push-ssh" ? "推送中…" : "⇡ 推送到仓库"}
          </button>
          <button
            className="btn"
            disabled={!!busy}
            onClick={() => {
              setForcePullConfirm(false);
              setForcePullOpen(true);
            }}
            title="只从云端恢复 SSH 配置（不影响笔记/AI 会话）"
          >
            ⇣ 从云端恢复
          </button>
          <button className="btn" disabled={!!busy} onClick={doBuild}>
            {busy === "build" ? "生成中…" : "重新生成 config"}
          </button>
          <button className="btn" disabled={!!busy} onClick={openEditor}>
            编辑 assh.yml
          </button>
          <button className="btn btn--primary" onClick={() => setAddOpen(true)}>
            + 添加主机
          </button>
        </div>
      </header>

      {hosts.error && <ErrorBox message={hosts.error} />}

      <div className="flex-1 min-h-0 grid grid-cols-[1fr_360px] gap-4">
        <div className="surface flex flex-col min-h-0">
          <div className="px-4 py-3 border-b border-white/5 shrink-0">
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="搜索主机名、地址或用户…"
              className="w-full bg-white/[0.03] border border-white/5 rounded-lg px-3 py-2 font-mono text-xs text-[var(--color-nexus-text)] placeholder:text-[var(--color-nexus-text-faint)] focus:border-[var(--color-neon-purple)]/50 focus:outline-none transition"
            />
          </div>
          <div className="flex-1 overflow-y-auto p-2">
            {hosts.loading && <div className="p-3"><Loading /></div>}
            {!hosts.loading && filtered.length === 0 && (
              <div className="p-6 text-center font-mono text-[11px] text-[var(--color-nexus-text-faint)]">
                {query ? "无匹配主机" : "尚未配置主机，点击右上角添加"}
              </div>
            )}
            {filtered.map((h) => {
              const tr = testResult[h.alias];
              return (
                <button
                  key={h.alias}
                  onClick={() => setSelected(h.alias)}
                  className={`w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-left transition-colors ${
                    selected === h.alias ? "bg-white/[0.06]" : "hover:bg-white/[0.03]"
                  }`}
                >
                  <span
                    className={`dot ${
                      tr ? (tr.reachable ? "dot--on" : "dot--off") : "dot--idle"
                    }`}
                  />
                  <div className="flex-1 min-w-0">
                    <div className="font-mono text-[13px] text-[var(--color-nexus-text)] truncate">
                      {h.alias}
                    </div>
                    <div className="font-mono text-[10.5px] text-[var(--color-nexus-text-faint)] truncate">
                      {h.user}@{h.hostname}:{h.port}
                      {h.gateways.length > 0 && (
                        <span className="text-[var(--color-neon-cyan)]"> · via {h.gateways.join("→")}</span>
                      )}
                    </div>
                  </div>
                  <span className="font-mono text-[10px] text-[var(--color-nexus-text-dim)] shrink-0">
                    {tr ? (tr.reachable ? `${tr.latency_ms}ms` : "失败") : ""}
                  </span>
                </button>
              );
            })}
          </div>
        </div>

        <aside className="surface flex flex-col min-h-0 overflow-y-auto">
          {!host ? (
            <div className="flex-1 flex items-center justify-center px-6 text-center">
              <p className="font-mono text-xs text-[var(--color-nexus-text-faint)]">选择左侧主机查看详情</p>
            </div>
          ) : (
            <div className="p-5 space-y-5">
              <div>
                <div className="flex items-center gap-2 mb-1">
                  <span
                    className={`dot ${
                      testResult[host.alias]
                        ? testResult[host.alias].reachable
                          ? "dot--on"
                          : "dot--off"
                        : "dot--idle"
                    }`}
                  />
                  <h2 className="font-mono text-base font-semibold text-[var(--color-nexus-text)]">
                    {host.alias}
                  </h2>
                </div>
                <p className="font-mono text-[11px] text-[var(--color-nexus-text-faint)]">
                  {testResult[host.alias]?.message ?? "未测试"}
                </p>
              </div>

              <dl className="space-y-2.5 font-mono text-xs">
                <Row k="地址" v={host.hostname} />
                <Row k="用户" v={host.user} />
                <Row k="端口" v={String(host.port)} />
                {host.inherit && <Row k="模板" v={host.inherit} accent="var(--color-neon-purple)" />}
                {host.gateways.length > 0 && (
                  <Row k="跳板" v={host.gateways.join(" → ")} accent="var(--color-neon-cyan)" />
                )}
                {host.key_file && <Row k="密钥" v={host.key_file} />}
              </dl>

              {!host.shell_capable && (
                <div className="rounded-lg border border-[var(--color-status-warn)]/30 bg-[var(--color-status-warn)]/[0.06] px-3 py-2">
                  <div className="flex items-center gap-1.5 mb-1">
                    <span className="text-[var(--color-status-warn)] text-[11px]">⚠</span>
                    <span className="font-mono text-[10.5px] text-[var(--color-status-warn)]">Git 服务主机</span>
                  </div>
                  <p className="font-mono text-[10px] text-[var(--color-nexus-text-dim)] leading-relaxed">
                    该主机（{host.user}@{host.hostname}）是代码托管服务，只接受 git 命令，
                    <span className="text-[var(--color-nexus-text)]">不分配伪终端</span>，
                    因此无法开交互式终端或 Electerm 窗口。仅可用于 git 推送/拉取。
                  </p>
                </div>
              )}

              <div className="pt-4 border-t border-white/5 space-y-2">
                <button
                  className="btn btn--primary w-full disabled:opacity-40"
                  disabled={!host.shell_capable}
                  onClick={() => openTerminal(host.alias)}
                  title={host.shell_capable ? "" : "该主机不支持交互式终端"}
                >
                  ▸ 新建终端
                </button>
                <button className="btn w-full" onClick={openEditModal}>
                  编辑连接信息
                </button>
                <button className="btn w-full" disabled={!!busy} onClick={openPwdModal}>
                  查看 / 修改密码
                </button>
                <button
                  className="btn w-full"
                  disabled={!!busy}
                  onClick={() => doTest(host.alias)}
                >
                  {busy === "test" ? "测试中…" : "测试连通性"}
                </button>
                <button
                  className="btn w-full disabled:opacity-40"
                  disabled={!!busy || !host.shell_capable}
                  onClick={() => doConnect(host.alias)}
                  title={host.shell_capable ? "" : "该主机不支持交互式终端"}
                >
                  {busy === "connect" ? "启动中…" : "用 Electerm 打开"}
                </button>
                <button
                  className="btn w-full !text-red-300/80 hover:!border-red-400/40"
                  onClick={() => setConfirmDelete(host.alias)}
                >
                  删除此主机
                </button>
              </div>

              <div className="pt-2">
                <div className="font-mono text-[10px] tracking-[0.12em] text-[var(--color-nexus-text-faint)] uppercase mb-2">
                  assh.yml 片段
                </div>
                <pre className="bg-black/30 border border-white/5 rounded-lg p-3 font-mono text-[10.5px] leading-relaxed text-[var(--color-neon-cyan)] overflow-x-auto">
{`${host.alias}:
  hostname: ${host.hostname}
  user: ${host.user}${host.port !== 22 ? `\n  port: ${host.port}` : ""}${host.inherit ? `\n  inherit: ${host.inherit}` : ""}`}
                </pre>
              </div>
            </div>
          )}
        </aside>
      </div>

      <AddHostModal
        open={addOpen}
        onClose={() => setAddOpen(false)}
        onAdded={() => {
          setAddOpen(false);
          void hosts.reload();
        }}
      />

      <Modal
        open={forcePullOpen}
        title="从云端恢复 SSH 配置"
        onClose={() => setForcePullOpen(false)}
        footer={
          <>
            <button className="btn" onClick={() => setForcePullOpen(false)}>
              取消
            </button>
            <button
              className="btn !text-[var(--color-status-warn)] border-[var(--color-status-warn)]/40"
              disabled={!forcePullConfirm || !!busy}
              onClick={doForcePull}
            >
              {busy === "force-pull" ? "恢复中…" : "确认恢复"}
            </button>
          </>
        }
      >
        <div className="space-y-3">
          <div className="rounded-lg border border-[var(--color-status-warn)]/30 bg-[var(--color-status-warn)]/[0.06] p-3">
            <ul className="font-mono text-[10.5px] leading-relaxed text-[var(--color-nexus-text-dim)] space-y-1">
              <li>· 从云端仓库取最新的 <span className="text-[var(--color-nexus-text)]">assh.yml</span> 与<span className="text-[var(--color-nexus-text)]">主机密码库</span>，覆盖本地</li>
              <li>· <span className="text-[var(--color-status-warn)]">本地对 assh.yml 与密码库的未推送修改会丢失</span></li>
              <li>· <span className="text-[var(--color-nexus-text)]">不影响笔记、AI 会话等其他数据</span></li>
              <li>· 恢复后自动重建 ~/.ssh/config，主机列表立即刷新</li>
            </ul>
          </div>
          <label className="flex items-start gap-2.5 p-3 rounded-lg border border-white/5 bg-white/[0.02] cursor-pointer">
            <input
              type="checkbox"
              checked={forcePullConfirm}
              onChange={(e) => setForcePullConfirm(e.target.checked)}
              className="mt-0.5 accent-[var(--color-status-warn)]"
            />
            <span className="text-[12px] text-[var(--color-nexus-text)]">
              我确认用云端的 assh.yml 覆盖本地
            </span>
          </label>
          <p className="font-mono text-[10px] text-[var(--color-nexus-text-faint)]">
            提示：已设置主密码时，主机密码会加密同步（encrypted/secrets.json.enc），恢复后即可查看密码；SSH 私钥本体不参与同步
          </p>
        </div>
      </Modal>

      <Modal
        open={editOpen}
        title="编辑 ~/.ssh/assh.yml"
        onClose={() => setEditOpen(false)}
        footer={
          <>
            <button className="btn" onClick={() => setEditOpen(false)}>
              取消
            </button>
            <button className="btn btn--primary" disabled={!!busy} onClick={saveEditor}>
              {busy === "save" ? "保存中…" : "保存"}
            </button>
          </>
        }
      >
        <textarea
          value={yml}
          onChange={(e) => setYml(e.target.value)}
          spellCheck={false}
          className="w-full h-[420px] bg-black/40 border border-white/10 rounded-lg p-3 font-mono text-[11.5px] leading-relaxed text-[var(--color-nexus-text)] focus:border-[var(--color-neon-purple)]/50 focus:outline-none resize-none"
        />
        <p className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] mt-2">
          保存时会校验 YAML 语法；保存后建议点「重新生成 config」使配置生效。
        </p>
      </Modal>

      <Modal
        open={electermMissing}
        title="未检测到 Electerm"
        onClose={() => setElectermMissing(false)}
        footer={
          <button className="btn btn--primary" onClick={() => setElectermMissing(false)}>
            知道了
          </button>
        }
      >
        <div className="space-y-3">
          <p className="font-mono text-[11.5px] text-[var(--color-nexus-text-dim)] leading-relaxed">
            本机没有找到 Electerm。请按你的系统下载安装，安装后回到这里重试；
            也可以在「设置 → 路径配置」中手动指定程序路径，或点击自动检测。
          </p>

          <div className="space-y-2">
            {[
              {
                os: "Ubuntu 24.04",
                cmd: "sudo dpkg -i electerm_*.deb",
                note: "下载 .deb 包后执行",
              },
              {
                os: "macOS",
                cmd: "brew install --cask electerm",
                note: "或下载 .dmg 拖入 Applications",
              },
              {
                os: "Windows",
                cmd: "下载 .exe 安装包",
                note: "安装后在设置中自动检测路径",
              },
            ].map((d) => (
              <div
                key={d.os}
                className="flex items-center justify-between gap-3 rounded-lg border border-white/8 bg-white/[0.02] px-3 py-2"
              >
                <div className="min-w-0">
                  <div className="font-mono text-[11.5px] text-[var(--color-nexus-text)]">{d.os}</div>
                  <div className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] truncate">
                    {d.cmd} · {d.note}
                  </div>
                </div>
                <a
                  href="https://github.com/electerm/electerm/releases/latest"
                  target="_blank"
                  rel="noreferrer"
                  className="btn shrink-0"
                >
                  下载
                </a>
              </div>
            ))}
          </div>

          <p className="font-mono text-[10px] text-[var(--color-nexus-text-faint)]">
            官网：electerm.html5beta.com · 或 apt 源：见 GitHub Releases 说明
          </p>
        </div>
      </Modal>

      <Modal
        open={hostEditOpen}
        title={`编辑连接 · ${host?.alias ?? ""}`}
        onClose={() => setHostEditOpen(false)}
        footer={
          <>
            <button className="btn" onClick={() => setHostEditOpen(false)}>
              取消
            </button>
            <button className="btn btn--primary" disabled={!!busy} onClick={saveEdit}>
              {busy === "edit" ? "保存中…" : "保存"}
            </button>
          </>
        }
      >
        <div className="space-y-3">
          <Field label="别名">
            <input value={editAlias} onChange={(e) => setEditAlias(e.target.value)} className={inputCls} />
          </Field>
          <Field label="地址">
            <input
              value={editHostname}
              onChange={(e) => setEditHostname(e.target.value)}
              className={inputCls}
            />
          </Field>
          <div className="grid grid-cols-2 gap-3">
            <Field label="用户">
              <input value={editUser} onChange={(e) => setEditUser(e.target.value)} className={inputCls} />
            </Field>
            <Field label="端口">
              <input
                value={editPort}
                onChange={(e) => setEditPort(e.target.value)}
                type="number"
                className={inputCls}
              />
            </Field>
          </div>
          <label className="flex items-center gap-2.5 p-2.5 rounded-lg border border-white/5 bg-white/[0.02] cursor-pointer">
            <input
              type="checkbox"
              checked={editTrust}
              onChange={(e) => setEditTrust(e.target.checked)}
              className="accent-[var(--color-neon-purple)]"
            />
            <span className="text-[12px] text-[var(--color-nexus-text)]">信任此主机（自动接受指纹）</span>
          </label>
          <p className="font-mono text-[10px] text-[var(--color-nexus-text-faint)]">
            保存后自动重新生成 ~/.ssh/config；修改别名会同步迁移已保存的密码
          </p>
        </div>
      </Modal>

      <Modal
        open={pwdOpen}
        title={`设置密码 · ${host?.alias ?? ""}`}
        onClose={() => setPwdOpen(false)}
        footer={
          <>
            <button className="btn" onClick={() => setPwdOpen(false)}>
              取消
            </button>
            <button className="btn btn--primary" disabled={!!busy} onClick={savePassword}>
              {busy === "pwd" ? "保存中…" : "保存密码"}
            </button>
          </>
        }
      >
        <div className="space-y-3">
          <div>
            <label className="block font-mono text-[11px] text-[var(--color-nexus-text-dim)] mb-1.5">
              登录密码
            </label>
            <div className="flex gap-2">
              <input
                type={pwdVisible ? "text" : "password"}
                value={pwdValue}
                onChange={(e) => setPwdValue(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && savePassword()}
                placeholder="输入该主机的 SSH 登录密码"
                className={`${inputCls} flex-1`}
                autoFocus
              />
              <button className="btn shrink-0" onClick={() => setPwdVisible(!pwdVisible)}>
                {pwdVisible ? "隐藏" : "显示"}
              </button>
            </div>
          </div>
          <p className="font-mono text-[10.5px] text-[var(--color-nexus-text-faint)] leading-relaxed">
            密码仅保存在本地（权限 600，secrets.json），不写入 assh.yml、不同步到 git。
            用于「测试连通性」和「内嵌终端」自动登录。
          </p>
        </div>
      </Modal>

      <Modal
        open={confirmDelete !== null}
        title="确认删除主机"
        onClose={() => setConfirmDelete(null)}
        footer={
          <>
            <button className="btn" onClick={() => setConfirmDelete(null)}>
              取消
            </button>
            <button
              className="btn !text-red-300 border-red-400/30"
              disabled={!!busy}
              onClick={() => confirmDelete && doDelete(confirmDelete)}
            >
              确认删除
            </button>
          </>
        }
      >
        <p className="text-[13px] text-[var(--color-nexus-text)]">
          将从 assh.yml 中删除主机 <span className="font-mono text-[var(--color-neon-purple)]">{confirmDelete}</span>。
        </p>
        <p className="font-mono text-[10.5px] text-[var(--color-nexus-text-faint)] mt-2">
          此操作会直接修改配置文件，删除后可在 git 中恢复（如已提交）。
        </p>
      </Modal>
    </div>
  );
}

function Row({ k, v, accent }: { k: string; v: string; accent?: string }) {
  return (
    <div className="flex justify-between gap-4">
      <dt className="text-[var(--color-nexus-text-faint)] shrink-0">{k}</dt>
      <dd className="text-right break-all" style={{ color: accent ?? "var(--color-nexus-text)" }}>
        {v}
      </dd>
    </div>
  );
}

function AddHostModal({
  open,
  onClose,
  onAdded,
}: {
  open: boolean;
  onClose: () => void;
  onAdded: () => void;
}) {
  const { push } = useToast();
  const { busy, run } = useAction();
  const [alias, setAlias] = useState("");
  const [hostname, setHostname] = useState("");
  const [user, setUser] = useState("root");
  const [port, setPort] = useState("22");
  const [inherit, setInherit] = useState("");
  const [authMethod, setAuthMethod] = useState<"key" | "password" | "none">("key");
  const [keyFile, setKeyFile] = useState("");
  const [password, setPassword] = useState("");
  const [trustHost, setTrustHost] = useState(true);
  const [keys, setKeys] = useState<SshKeyInfo[]>([]);
  const [keysLoaded, setKeysLoaded] = useState(false);

  // 打开时加载本机可用密钥
  useEffect(() => {
    if (!open || keysLoaded) return;
    api
      .listSshKeys()
      .then((list) => {
        setKeys(list);
        if (list.length > 0) setKeyFile(list[0].path);
        setKeysLoaded(true);
      })
      .catch(() => setKeysLoaded(true));
  }, [open, keysLoaded]);

  function reset() {
    setAlias("");
    setHostname("");
    setUser("root");
    setPort("22");
    setInherit("");
    setAuthMethod("key");
    setPassword("");
    setTrustHost(true);
  }

  async function submit() {
    if (!alias.trim() || !hostname.trim()) {
      push("err", "请填写别名和地址");
      return;
    }
    if (authMethod === "key" && !keyFile.trim()) {
      push("err", "请选择或填写密钥路径");
      return;
    }
    if (authMethod === "password" && !password) {
      push("err", "请填写密码");
      return;
    }
    const p = Number(port) || 22;
    const r = await run("add", () =>
      api.addHost({
        alias: alias.trim(),
        hostname: hostname.trim(),
        user: user.trim() || "root",
        port: p,
        inherit: inherit.trim() || null,
        authMethod,
        keyFile: authMethod === "key" ? keyFile.trim() : null,
        password: authMethod === "password" ? password : null,
        trustHost,
      })
    );
    if (r.ok) {
      push("ok", `已添加 ${alias}`);
      reset();
      onAdded();
    } else {
      push("err", "添加失败", r.error);
    }
  }

  return (
    <Modal
      open={open}
      title="添加 SSH 主机"
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>
            取消
          </button>
          <button className="btn btn--primary" disabled={!!busy} onClick={submit}>
            {busy === "add" ? "添加中…" : "添加"}
          </button>
        </>
      }
    >
      <div className="space-y-3">
        <Field label="别名 *">
          <input value={alias} onChange={(e) => setAlias(e.target.value)} placeholder="例如：web-prod" className={inputCls} />
        </Field>
        <Field label="地址 *">
          <input value={hostname} onChange={(e) => setHostname(e.target.value)} placeholder="例如：192.168.1.10 或 host.example.com" className={inputCls} />
        </Field>
        <div className="grid grid-cols-2 gap-3">
          <Field label="用户">
            <input value={user} onChange={(e) => setUser(e.target.value)} className={inputCls} />
          </Field>
          <Field label="端口">
            <input value={port} onChange={(e) => setPort(e.target.value)} type="number" className={inputCls} />
          </Field>
        </div>

        {/* 认证方式 */}
        <Field label="认证方式">
          <div className="flex gap-1.5">
            {(
              [
                { k: "key", label: "密钥" },
                { k: "password", label: "密码" },
                { k: "none", label: "不指定" },
              ] as const
            ).map((o) => (
              <button
                key={o.k}
                onClick={() => setAuthMethod(o.k)}
                className={`flex-1 font-mono text-[11px] py-2 rounded-lg border transition ${
                  authMethod === o.k
                    ? "border-[var(--color-neon-purple)]/60 bg-[var(--color-neon-purple)]/10 text-[var(--color-nexus-text)]"
                    : "border-white/5 bg-white/[0.02] text-[var(--color-nexus-text-dim)] hover:bg-white/[0.05]"
                }`}
              >
                {o.label}
              </button>
            ))}
          </div>
        </Field>

        {authMethod === "key" && (
          <Field label="密钥文件">
            <input
              value={keyFile}
              onChange={(e) => setKeyFile(e.target.value)}
              list="ssh-keys"
              placeholder="~/.ssh/id_ed25519"
              className={inputCls}
            />
            <datalist id="ssh-keys">
              {keys.map((k) => (
                <option key={k.path} value={k.path}>
                  {k.name} ({k.kind})
                </option>
              ))}
            </datalist>
            {keys.length > 0 && (
              <p className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] mt-1">
                检测到 {keys.length} 个密钥，可直接选择
              </p>
            )}
          </Field>
        )}

        {authMethod === "password" && (
          <Field label="密码">
            <input
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              type="password"
              placeholder="仅保存在本地（权限 600），不写入配置、不同步"
              className={inputCls}
            />
            <p className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] mt-1">
              密码存于本地 secrets.json，仅用于「测试连通性」；连接时 Electerm 会另行提示
            </p>
          </Field>
        )}

        <Field label="继承模板（可选）">
          <input value={inherit} onChange={(e) => setInherit(e.target.value)} placeholder="模板名（留空则不继承）" className={inputCls} />
        </Field>

        {/* 信任主机 */}
        <label className="flex items-start gap-2.5 p-3 rounded-lg border border-white/5 bg-white/[0.02] cursor-pointer">
          <input
            type="checkbox"
            checked={trustHost}
            onChange={(e) => setTrustHost(e.target.checked)}
            className="mt-0.5 accent-[var(--color-neon-purple)]"
          />
          <div>
            <div className="text-[12.5px] text-[var(--color-nexus-text)]">信任此主机（首次连接自动接受指纹）</div>
            <div className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] mt-0.5">
              写入 <span className="text-[var(--color-neon-cyan)]">StrictHostKeyChecking accept-new</span>，
              否则新主机首次连接会因指纹确认失败
            </div>
          </div>
        </label>
      </div>
    </Modal>
  );
}

const inputCls =
  "w-full bg-white/[0.03] border border-white/5 rounded-lg px-3 py-2 font-mono text-xs text-[var(--color-nexus-text)] placeholder:text-[var(--color-nexus-text-faint)] focus:border-[var(--color-neon-purple)]/50 focus:outline-none transition";

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <label className="block font-mono text-[11px] text-[var(--color-nexus-text-dim)] mb-1.5">{label}</label>
      {children}
    </div>
  );
}
