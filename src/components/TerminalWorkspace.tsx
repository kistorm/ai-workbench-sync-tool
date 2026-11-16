import { useState } from "react";
import { SshTerminal } from "./SshTerminal";
import { Modal } from "./ui";

export interface TermTab {
  id: string;
  alias: string;
  title: string;
}

/**
 * 多标签终端工作区
 * - 每个标签对应一个独立 PTY 会话
 * - 非活动标签仅隐藏，会话与滚动内容都保留
 */
export function TerminalWorkspace({
  tabs,
  activeId,
  onActivate,
  onClose,
  onCloseAll,
  hosts,
  onCreate,
  visible = true,
  onMinimize,
}: {
  tabs: TermTab[];
  activeId: string | null;
  onActivate: (id: string) => void;
  onClose: (id: string) => void;
  onCloseAll: () => void;
  hosts: { alias: string; hostname: string; user: string }[];
  onCreate: (alias: string) => void;
  /** 隐藏时仅 display:none，组件与 PTY 会话保持存活 */
  visible?: boolean;
  onMinimize?: () => void;
}) {
  const [pickerOpen, setPickerOpen] = useState(false);

  return (
    <div
      className="fixed inset-0 z-50 flex flex-col bg-[var(--color-nexus-bg)]"
      style={{ display: visible ? "flex" : "none" }}
    >
      {/* 标签栏 */}
      <header className="flex items-center gap-2 px-2 py-1.5 border-b border-white/10 bg-[var(--color-nexus-bg-elevated)] shrink-0">
        <div className="flex items-center gap-1.5 overflow-x-auto flex-1 min-w-0">
          {tabs.map((t) => {
            const active = t.id === activeId;
            return (
              <div
                key={t.id}
                onClick={() => onActivate(t.id)}
                className={`group flex items-center gap-1.5 px-2.5 py-1 rounded-md border cursor-pointer shrink-0 transition-colors ${
                  active
                    ? "border-[var(--color-neon-purple)]/50 bg-[var(--color-neon-purple)]/10"
                    : "border-white/10 hover:bg-white/[0.04]"
                }`}
              >
                <span className={`dot ${active ? "dot--on" : "dot--idle"}`} />
                <span
                  className={`font-mono text-[11px] ${
                    active
                      ? "text-[var(--color-nexus-text)]"
                      : "text-[var(--color-nexus-text-dim)]"
                  }`}
                >
                  {t.title}
                </span>
                <button
                  className="text-[var(--color-nexus-text-faint)] hover:text-red-400 text-[11px] leading-none px-0.5"
                  onClick={(e) => {
                    e.stopPropagation();
                    onClose(t.id);
                  }}
                  title="关闭此终端"
                >
                  ✕
                </button>
              </div>
            );
          })}
        </div>
        <div className="flex items-center gap-2 shrink-0">
          <button
            className="btn btn--primary"
            onClick={() => setPickerOpen(true)}
            title="再开一个 SSH 终端"
          >
            + 新建
          </button>
          <span className="font-mono text-[10px] text-[var(--color-nexus-text-faint)]">
            {tabs.length} 个会话
          </span>
          {onMinimize && (
            <button className="btn" onClick={onMinimize} title="隐藏终端（连接保持）">
              最小化
            </button>
          )}
          <button className="btn" onClick={onCloseAll}>
            关闭全部
          </button>
        </div>
      </header>

      {/* 终端区：全部实例保持挂载，非活动隐藏 */}
      <div className="flex-1 relative min-h-0">
        {tabs.map((t) => (
          <div
            key={t.id}
            className="absolute inset-0"
            style={{ display: t.id === activeId ? "block" : "none" }}
          >
            <SshTerminal alias={t.alias} visible={t.id === activeId} />
          </div>
        ))}
      </div>

      {/* 新建终端：选择主机 */}
      <Modal
        open={pickerOpen}
        title="新建 SSH 终端"
        onClose={() => setPickerOpen(false)}
        footer={
          <button className="btn" onClick={() => setPickerOpen(false)}>
            取消
          </button>
        }
      >
        <div className="space-y-1.5">
          {hosts.length === 0 && (
            <p className="font-mono text-[11px] text-[var(--color-nexus-text-faint)]">暂无主机</p>
          )}
          {hosts.map((h) => (
            <button
              key={h.alias}
              onClick={() => {
                onCreate(h.alias);
                setPickerOpen(false);
              }}
              className="w-full flex items-center gap-2.5 px-3 py-2 rounded-lg text-left hover:bg-white/[0.05] transition-colors"
            >
              <span className="dot dot--on" />
              <span className="font-mono text-[12.5px] text-[var(--color-nexus-text)]">{h.alias}</span>
              <span className="font-mono text-[10.5px] text-[var(--color-nexus-text-faint)] truncate">
                {h.user}@{h.hostname}
              </span>
            </button>
          ))}
        </div>
      </Modal>
    </div>
  );
}
