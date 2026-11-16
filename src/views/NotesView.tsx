import { useEffect, useMemo, useRef, useState } from "react";
import { marked } from "marked";
import TurndownService from "turndown";
import { api } from "../api";
import type { ClearResult, NoteMeta } from "../api";
import { useAction, useAsync } from "../hooks";
import { ErrorBox, Loading, Modal, useToast } from "../components/ui";
import { useAppTasks } from "../store";

const inputCls =
  "w-full bg-white/[0.03] border border-white/5 rounded-lg px-3 py-2 font-mono text-xs text-[var(--color-nexus-text)] placeholder:text-[var(--color-nexus-text-faint)] focus:border-[var(--color-neon-purple)]/50 focus:outline-none transition";

const turndown = new TurndownService({
  headingStyle: "atx",
  codeBlockStyle: "fenced",
  bulletListMarker: "-",
});

export function NotesView() {
  const { push } = useToast();
  const notes = useAsync(() => api.listNotes(), []);
  const info = useAsync(() => api.notesInfo(), []);
  const { busy, run } = useAction();

  const [selected, setSelected] = useState<string | null>(null);
  const [content, setContent] = useState("");
  const [original, setOriginal] = useState("");
  const [loadedPath, setLoadedPath] = useState<string | null>(null);
  const [mode, setMode] = useState<"md" | "rich">("md");
  const [newOpen, setNewOpen] = useState(false);
  const [newName, setNewName] = useState("");
  const [confirmDelete, setConfirmDelete] = useState<NoteMeta | null>(null);
  const [youdaoOpen, setYoudaoOpen] = useState(false);
  const [youdaoCookie, setYoudaoCookie] = useState("");
  const [youdaoDir, setYoudaoDir] = useState("");
  // 拉取属于长任务：状态放在 App 级，关弹窗/切页都不打断
  const { youdaoPulling, youdaoResult, startYoudaoPull, clearYoudaoResult } = useAppTasks();
  const [clearOpen, setClearOpen] = useState(false);
  const [clearConfirm, setClearConfirm] = useState(false);
  const [clearBusy, setClearBusy] = useState(false);
  const richRef = useRef<HTMLDivElement>(null);

  const list = notes.data ?? [];
  const dirty = content !== original;

  // 默认选中第一篇
  useEffect(() => {
    if (!selected && list.length > 0) setSelected(list[0].path);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [list.length]);

  // 载入选中笔记
  useEffect(() => {
    if (!selected || selected === loadedPath) return;
    let alive = true;
    (async () => {
      try {
        const text = await api.readNote(selected);
        if (!alive) return;
        setContent(text);
        setOriginal(text);
        setLoadedPath(selected);
      } catch (e) {
        if (alive) push("err", "读取笔记失败", String(e));
      }
    })();
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected]);

  // 切换到富文本时，把 Markdown 渲染进编辑器
  useEffect(() => {
    if (mode === "rich" && richRef.current) {
      richRef.current.innerHTML = marked.parse(content, { async: false }) as string;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, loadedPath]);

  const previewHtml = useMemo(() => {
    if (mode !== "rich") return "";
    return marked.parse(content, { async: false }) as string;
  }, [content, mode]);

  /** 富文本编辑：HTML → Markdown */
  function syncFromRich() {
    if (!richRef.current) return;
    const html = richRef.current.innerHTML;
    setContent(turndown.turndown(html));
  }

  /** 富文本工具栏命令（保住焦点） */
  function exec(cmd: string, val?: string) {
    document.execCommand(cmd, false, val);
    richRef.current?.focus();
    syncFromRich();
  }

  async function saveCurrent() {
    if (!selected) return;
    const r = await run("save", () => api.saveNote(selected, content));
    if (r.ok) {
      push("ok", "笔记已保存");
      setOriginal(content);
      void notes.reload();
    } else push("err", "保存失败", r.error);
  }

  async function createNew() {
    const r = await run("create", () => api.createNote(newName));
    if (r.ok && r.data) {
      push("ok", "已新建笔记", r.data);
      setNewOpen(false);
      setNewName("");
      await notes.reload();
      setSelected(r.data);
      setLoadedPath(null);
    } else push("err", "新建失败", r.error);
  }

  async function doDelete(n: NoteMeta) {
    const r = await run("delete", () => api.deleteNote(n.path));
    if (!r.ok) {
      push("err", "删除失败", r.error);
      return;
    }
    push("ok", `已删除「${n.name}」`);
    setConfirmDelete(null);
    if (selected === n.path) {
      setSelected(null);
      setContent("");
      setOriginal("");
      setLoadedPath(null);
    }
    void notes.reload();

    // 删除后自动推送到云端（让删除操作也同步）
    const pr = await run("push", () => api.notesGitSync("push"));
    if (pr.ok) push("ok", "删除已同步到云端");
    else push("err", "删除已在本地生效，但推送失败", pr.error);
  }

  function doYoudaoPull() {
    if (!youdaoCookie.trim()) {
      push("err", "请粘贴有道云 Cookie");
      return;
    }
    // 弹窗立即关闭，任务在后台继续，不阻塞其它操作
    setYoudaoOpen(false);
    startYoudaoPull(youdaoCookie.trim(), youdaoDir.trim() || undefined);
  }

  // 拉取结束后自动刷新笔记列表
  const prevPulling = useRef(false);
  useEffect(() => {
    if (prevPulling.current && !youdaoPulling) {
      void notes.reload();
    }
    prevPulling.current = youdaoPulling;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [youdaoPulling]);

  async function doClear() {
    if (!clearConfirm) return;
    setClearBusy(true);
    try {
      const r: ClearResult = await api.clearNotes();
      push(
        "ok",
        `已清空笔记：${r.files} 个文件、${r.dirs} 个目录`,
        `释放 ${(r.bytes / 1048576).toFixed(1)} MB。删除仅发生在本地，未推送云端。`
      );
      setSelected(null);
      setContent("");
      setOriginal("");
      setLoadedPath(null);
      await notes.reload();
      setClearOpen(false);
      setClearConfirm(false);
    } catch (e) {
      push("err", "清空失败", String(e));
    } finally {
      setClearBusy(false);
    }
  }

  async function gitSync(action: "push" | "pull") {
    const r = await run(action, () => api.notesGitSync(action));
    if (r.ok) push("ok", `笔记 ${action} 完成`, r.data?.trim().slice(0, 200));
    else push("err", `笔记 ${action} 失败`, r.error);
  }

  return (
    <div className="page-enter h-full flex flex-col gap-5 relative">
      {/* 拉取期间锁定本页，避免并发修改笔记 */}
      {youdaoPulling && (
        <div className="absolute inset-0 z-30 rounded-xl bg-[var(--color-nexus-bg)]/78 backdrop-blur-[2px] flex flex-col items-center justify-center gap-3">
          <span className="dot dot--half" />
          <div className="font-mono text-[13px] text-[var(--color-nexus-text)]">正在从有道云拉取笔记…</div>
          <div className="font-mono text-[10.5px] text-[var(--color-nexus-text-faint)]">
            此页面暂不可操作，可切换到其他页面继续使用
          </div>
          <div className="font-mono text-[10px] text-[var(--color-nexus-text-faint)]">
            完成后会自动刷新列表并通知
          </div>
        </div>
      )}
      <header className="flex items-center justify-between shrink-0">
        <div>
          <h1 className="text-lg font-semibold text-[var(--color-nexus-text)]">笔记同步</h1>
          <p className="font-mono text-[11px] text-[var(--color-nexus-text-faint)] mt-0.5">
            本地 Markdown · {info.data?.dir ?? "~/ssh-config-sync/notes"}
            {info.data && !info.data.is_git && " · 未绑定 git"}
          </p>
        </div>
        <div className="flex gap-2">
          <button className="btn" disabled={!!busy} onClick={() => gitSync("pull")}>
            {busy === "pull" ? "拉取中…" : "⇣ 拉取"}
          </button>
          <button className="btn" disabled={!!busy} onClick={() => gitSync("push")}>
            {busy === "push" ? "推送中…" : "⇡ 推送"}
          </button>
          <button className="btn" onClick={() => setYoudaoOpen(true)}>
            ⇣ 有道云拉取
          </button>
          <button className="btn btn--primary" onClick={() => setNewOpen(true)}>
            + 新建笔记
          </button>
        </div>
      </header>

      {notes.error && <ErrorBox message={notes.error} />}

      <div className="surface flex items-start gap-2.5 px-4 py-3 shrink-0">
        <span className="text-[var(--color-status-warn)] text-sm leading-none mt-0.5">⚠</span>
        <p className="font-mono text-[11px] text-[var(--color-nexus-text-dim)] leading-relaxed">
          有道云笔记官方 OpenAPI 已停止新增申请，因此通过
          <span className="text-[var(--color-neon-pink)]"> Cookie 方式 </span>
          从有道云<span className="text-[var(--color-neon-cyan)]"> 拉取</span>笔记（仅拉取，不推送）。
          点击右上「⇣ 有道云拉取」，粘贴浏览器 Cookie 即可导入到
          <span className="text-[var(--color-nexus-text)]"> {info.data?.dir ?? "~/ssh-config-sync/notes"} </span>
          目录，之后可随 git 同步到多机。
        </p>
      </div>

      <div className="flex-1 min-h-0 grid grid-cols-[300px_1fr] gap-4">
        {/* 列表 */}
        <div className="surface flex flex-col min-h-0">
          <div className="flex items-center justify-between px-3 py-2 border-b border-white/5 shrink-0">
            <span className="font-mono text-[10px] tracking-[0.1em] text-[var(--color-nexus-text-faint)] uppercase">
              全部笔记 · {list.length}
            </span>
            <button
              className="font-mono text-[10px] text-red-400/60 hover:text-red-400 transition"
              onClick={() => {
                setClearConfirm(false);
                setClearOpen(true);
              }}
            >
              清空
            </button>
          </div>
          <div className="flex-1 overflow-y-auto p-2">
            {notes.loading && <div className="p-3"><Loading /></div>}
            {!notes.loading && list.length === 0 && (
              <div className="p-5 text-center font-mono text-[11px] text-[var(--color-nexus-text-faint)]">
                还没有笔记，点击「新建笔记」开始
              </div>
            )}
            {list.map((n) => (
              <div
                key={n.path}
                className={`group w-full text-left px-3 py-2.5 rounded-lg transition-colors cursor-pointer ${
                  selected === n.path ? "bg-white/[0.06]" : "hover:bg-white/[0.03]"
                }`}
                onClick={() => setSelected(n.path)}
              >
                <div className="flex items-center gap-2">
                  <span className="text-[13px] text-[var(--color-nexus-text)] truncate flex-1">{n.name}</span>
                  <button
                    className="opacity-0 group-hover:opacity-100 transition text-[var(--color-nexus-text-faint)] hover:text-red-400 text-xs shrink-0"
                    title="删除并推送"
                    onClick={(e) => {
                      e.stopPropagation();
                      setConfirmDelete(n);
                    }}
                  >
                    ✕
                  </button>
                </div>
                <div className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] mt-0.5">
                  {n.updated} · {(n.size / 1024).toFixed(1)} KB
                </div>
              </div>
            ))}
          </div>
        </div>

        {/* 编辑区 */}
        <div className="surface flex flex-col min-h-0">
          {!selected ? (
            <div className="flex-1 flex items-center justify-center">
              <p className="font-mono text-xs text-[var(--color-nexus-text-faint)]">选择或新建一篇笔记</p>
            </div>
          ) : (
            <>
              <div className="flex items-center justify-between px-5 py-2.5 border-b border-white/5 shrink-0">
                <div className="min-w-0">
                  <div className="text-sm text-[var(--color-nexus-text)] truncate">
                    {list.find((n) => n.path === selected)?.name ?? selected}
                  </div>
                  <div className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] truncate">
                    {info.data?.dir}/{selected}
                  </div>
                </div>
                <div className="flex items-center gap-2 shrink-0">
                  {/* 模式切换 */}
                  <div className="flex rounded-lg border border-white/5 overflow-hidden">
                    <button
                      onClick={() => setMode("md")}
                      className={`font-mono text-[10.5px] px-2.5 py-1.5 transition ${
                        mode === "md"
                          ? "bg-white/[0.08] text-[var(--color-nexus-text)]"
                          : "text-[var(--color-nexus-text-dim)] hover:bg-white/[0.04]"
                      }`}
                    >
                      Markdown
                    </button>
                    <button
                      onClick={() => setMode("rich")}
                      className={`font-mono text-[10.5px] px-2.5 py-1.5 transition ${
                        mode === "rich"
                          ? "bg-white/[0.08] text-[var(--color-nexus-text)]"
                          : "text-[var(--color-nexus-text-dim)] hover:bg-white/[0.04]"
                      }`}
                    >
                      富文本
                    </button>
                  </div>
                  {dirty && (
                    <span className="font-mono text-[10px] text-[var(--color-status-warn)]">未保存</span>
                  )}
                  <button
                    className="btn btn--primary"
                    disabled={!!busy || !dirty}
                    onClick={saveCurrent}
                  >
                    {busy === "save" ? "保存中…" : "保存"}
                  </button>
                </div>
              </div>

              {/* 富文本工具栏 */}
              {mode === "rich" && (
                <div className="flex items-center gap-1 px-4 py-1.5 border-b border-white/5 shrink-0 flex-wrap">
                  {[
                    { cmd: "bold", label: "B", title: "加粗" },
                    { cmd: "italic", label: "I", title: "斜体" },
                    { cmd: "strikeThrough", label: "S", title: "删除线" },
                  ].map((b) => (
                    <button
                      key={b.cmd}
                      title={b.title}
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={() => exec(b.cmd)}
                      className="w-7 h-7 rounded font-mono text-[11px] text-[var(--color-nexus-text-dim)] hover:bg-white/[0.06] hover:text-[var(--color-nexus-text)] transition"
                    >
                      {b.label}
                    </button>
                  ))}
                  <span className="w-px h-4 bg-white/10 mx-1" />
                  <button
                    title="标题"
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={() => exec("formatBlock", "h2")}
                    className="px-2 h-7 rounded font-mono text-[11px] text-[var(--color-nexus-text-dim)] hover:bg-white/[0.06] hover:text-[var(--color-nexus-text)] transition"
                  >
                    H2
                  </button>
                  <button
                    title="无序列表"
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={() => exec("insertUnorderedList")}
                    className="px-2 h-7 rounded font-mono text-[11px] text-[var(--color-nexus-text-dim)] hover:bg-white/[0.06] hover:text-[var(--color-nexus-text)] transition"
                  >
                    • 列表
                  </button>
                  <button
                    title="有序列表"
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={() => exec("insertOrderedList")}
                    className="px-2 h-7 rounded font-mono text-[11px] text-[var(--color-nexus-text-dim)] hover:bg-white/[0.06] hover:text-[var(--color-nexus-text)] transition"
                  >
                    1. 列表
                  </button>
                  <button
                    title="引用"
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={() => exec("formatBlock", "blockquote")}
                    className="px-2 h-7 rounded font-mono text-[11px] text-[var(--color-nexus-text-dim)] hover:bg-white/[0.06] hover:text-[var(--color-nexus-text)] transition"
                  >
                    ❝ 引用
                  </button>
                  <span className="ml-auto font-mono text-[9.5px] text-[var(--color-nexus-text-faint)]">
                    保存时自动转回 Markdown
                  </span>
                </div>
              )}

              {mode === "md" ? (
                <textarea
                  value={content}
                  onChange={(e) => setContent(e.target.value)}
                  spellCheck={false}
                  className="flex-1 w-full bg-transparent p-5 font-mono text-[12.5px] leading-relaxed text-[var(--color-nexus-text)] resize-none focus:outline-none"
                  placeholder="开始写 Markdown…"
                />
              ) : (
                <div
                  ref={richRef}
                  contentEditable
                  suppressContentEditableWarning
                  onInput={syncFromRich}
                  onBlur={syncFromRich}
                  className="flex-1 overflow-y-auto p-5 text-[13.5px] leading-relaxed text-[var(--color-nexus-text)] focus:outline-none prose-nexus"
                  dangerouslySetInnerHTML={{ __html: previewHtml }}
                />
              )}
            </>
          )}
        </div>
      </div>

      <Modal
        open={newOpen}
        title="新建笔记"
        onClose={() => setNewOpen(false)}
        footer={
          <>
            <button className="btn" onClick={() => setNewOpen(false)}>
              取消
            </button>
            <button className="btn btn--primary" disabled={!!busy} onClick={createNew}>
              {busy === "create" ? "创建中…" : "创建"}
            </button>
          </>
        }
      >
        <label className="block font-mono text-[11px] text-[var(--color-nexus-text-dim)] mb-1.5">
          笔记名称
        </label>
        <input
          value={newName}
          onChange={(e) => setNewName(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && createNew()}
          placeholder="例如：服务器巡检记录"
          className={inputCls}
          autoFocus
        />
      </Modal>

      <Modal
        open={clearOpen}
        title="确认清空全部笔记"
        onClose={() => setClearOpen(false)}
        footer={
          <>
            <button className="btn" onClick={() => setClearOpen(false)}>
              取消
            </button>
            <button
              className="btn !text-red-300 border-red-400/40 bg-red-500/10"
              disabled={!clearConfirm || clearBusy}
              onClick={doClear}
            >
              {clearBusy ? "删除中…" : "确认清空"}
            </button>
          </>
        }
      >
        <div className="space-y-3">
          <div className="rounded-lg border border-red-500/30 bg-red-500/[0.07] p-3">
            <div className="flex items-center gap-1.5 mb-2">
              <span className="text-red-400 text-[13px]">⚠</span>
              <span className="font-mono text-[11.5px] text-red-300">不可恢复的危险操作</span>
            </div>
            <ul className="font-mono text-[10.5px] leading-relaxed text-[var(--color-nexus-text-dim)] space-y-1">
              <li>· 将<span className="text-red-300"> 永久删除 </span>笔记目录下的全部内容：
                <span className="text-[var(--color-nexus-text)]"> {list.length} 篇笔记</span>
                {list.length > 0 && " 及其所有子目录、图片等资源"}
              </li>
              <li>· 删除的是真实文件（不进入回收站），文件中已无备份</li>
              <li>· 不会自动推送到云端；如需同步删除，删除后需手动点「⇡ 推送」</li>
              <li>· <span className="text-[var(--color-nexus-text)]">.git 仓库元数据会保留</span>，若之前提交过，可从 git 历史恢复</li>
            </ul>
          </div>

          <label className="flex items-start gap-2.5 p-3 rounded-lg border border-white/5 bg-white/[0.02] cursor-pointer">
            <input
              type="checkbox"
              checked={clearConfirm}
              onChange={(e) => setClearConfirm(e.target.checked)}
              className="mt-0.5 accent-red-500"
            />
            <div>
              <div className="text-[12.5px] text-[var(--color-nexus-text)]">
                我已确认：了解此操作不可恢复，并要删除全部笔记
              </div>
              <div className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] mt-0.5">
                目录：{info.data?.dir ?? "~/ssh-config-sync/notes"}
              </div>
            </div>
          </label>
        </div>
      </Modal>

      <Modal
        open={youdaoOpen}
        title="从有道云拉取笔记"
        onClose={() => setYoudaoOpen(false)}
        footer={
          <>
            <button className="btn" onClick={() => setYoudaoOpen(false)}>
              关闭
            </button>
            <button className="btn btn--primary" disabled={youdaoPulling} onClick={doYoudaoPull}>
              {youdaoPulling ? "拉取中…（可关闭本窗口）" : "开始拉取"}
            </button>
          </>
        }
      >
        <div className="space-y-3">
          <div className="rounded-lg border border-white/5 bg-white/[0.02] p-3 font-mono text-[10.5px] leading-relaxed text-[var(--color-nexus-text-dim)]">
            <div className="text-[var(--color-nexus-text)] mb-1.5">获取 Cookie 步骤：</div>
            1. 浏览器登录 note.youdao.com<br />
            2. F12 打开开发者工具 → Network<br />
            3. 刷新页面，点任意请求，复制 Request Headers 里的 <span className="text-[var(--color-neon-cyan)]">Cookie</span><br />
            4. 粘贴到下方（需包含 YNOTE_CSTK / YNOTE_LOGIN / YNOTE_SESS）
          </div>

          {/* 注意事项 */}
          <div className="rounded-lg border border-[var(--color-status-warn)]/30 bg-[var(--color-status-warn)]/[0.06] p-3">
            <div className="flex items-center gap-1.5 mb-1.5">
              <span className="text-[var(--color-status-warn)] text-[12px]">⚠</span>
              <span className="font-mono text-[11px] text-[var(--color-status-warn)]">注意事项</span>
            </div>
            <ul className="font-mono text-[10.5px] leading-relaxed text-[var(--color-nexus-text-dim)] space-y-1">
              <li>
                <span className="text-[var(--color-nexus-text)]">同名直接覆盖：</span>
                拉取时会与本地笔记比对，若已存在<span className="text-[var(--color-status-warn)]">相同名称</span>的笔记，
                云端较新的版本会<span className="text-[var(--color-status-warn)]">直接覆盖</span>本地文件，本地修改将丢失。
              </li>
              <li>
                <span className="text-[var(--color-nexus-text)]">仅拉取：</span>
                不会向有道云写入、修改或删除任何内容。
              </li>
              <li>
                <span className="text-[var(--color-nexus-text)]">建议：</span>
                拉取前先点「⇡ 推送」或备份本地笔记，避免未同步的修改被覆盖。
              </li>
            </ul>
          </div>

          <div>
            <label className="block font-mono text-[11px] text-[var(--color-nexus-text-dim)] mb-1.5">
              Cookie *
            </label>
            <textarea
              value={youdaoCookie}
              onChange={(e) => setYoudaoCookie(e.target.value)}
              placeholder="YNOTE_CSTK=xxx; YNOTE_LOGIN=yyy; YNOTE_SESS=zzz"
              spellCheck={false}
              className="w-full h-24 bg-black/30 border border-white/10 rounded-lg p-3 font-mono text-[10.5px] leading-relaxed text-[var(--color-nexus-text)] placeholder:text-[var(--color-nexus-text-faint)] focus:border-[var(--color-neon-pink)]/50 focus:outline-none resize-none"
            />
            <p className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] mt-1">
              Cookie 仅保存在本地（权限 600），不会同步到 git
            </p>
          </div>

          <div>
            <label className="block font-mono text-[11px] text-[var(--color-nexus-text-dim)] mb-1.5">
              有道云目录名（可选，留空=全部）
            </label>
            <input
              value={youdaoDir}
              onChange={(e) => setYoudaoDir(e.target.value)}
              placeholder="例如：工作/运维"
              className={inputCls}
            />
          </div>

          {youdaoPulling && (
            <div className="flex items-center gap-2 font-mono text-[11px] text-[var(--color-neon-pink)]">
              <span className="dot dot--half" />
              已开始拉取：可以关闭本窗口，任务会在后台继续
            </div>
          )}
          {!youdaoPulling && youdaoResult && (
            <button
              className="font-mono text-[10px] text-[var(--color-nexus-text-faint)] hover:text-[var(--color-nexus-text)] self-start"
              onClick={clearYoudaoResult}
            >
              清除上次结果
            </button>
          )}

          {youdaoResult && (
            <div className="space-y-2">
              <div className="flex gap-4 font-mono text-[11px]">
                <span className="text-[var(--color-status-on)]">新增 {youdaoResult.added}</span>
                <span className="text-[var(--color-neon-cyan)]">更新 {youdaoResult.updated}</span>
                <span className="text-[var(--color-nexus-text-dim)]">跳过 {youdaoResult.skipped}</span>
              </div>
              <pre className="max-h-48 overflow-auto rounded-lg bg-black/40 p-3 font-mono text-[10px] leading-relaxed text-[var(--color-nexus-text-dim)] whitespace-pre-wrap">
                {youdaoResult.output || "（无输出）"}
              </pre>
            </div>
          )}
        </div>
      </Modal>

      <Modal
        open={confirmDelete !== null}
        title="确认删除笔记"
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
              {busy === "delete" ? "删除中…" : "删除并同步"}
            </button>
          </>
        }
      >
        <p className="text-[13px] text-[var(--color-nexus-text)]">
          将删除「{confirmDelete?.name}」（{confirmDelete?.path}）。
        </p>
        <p className="font-mono text-[10.5px] text-[var(--color-nexus-text-faint)] mt-2">
          删除后会自动执行一次 git push，让云端也同步删除。若已提交，仍可从 git 历史恢复。
        </p>
      </Modal>
    </div>
  );
}
