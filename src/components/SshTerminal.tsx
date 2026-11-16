import { useEffect, useRef } from "react";
import { Terminal as XTerm } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { listen } from "@tauri-apps/api/event";
import "@xterm/xterm/css/xterm.css";
import { api } from "../api";

/**
 * 单个 SSH 终端面板（由 TerminalWorkspace 管理多个实例）
 * - visible=false 时仅隐藏，PTY 会话与终端内容仍然保留
 */
export function SshTerminal({ alias, visible }: { alias: string; visible: boolean }) {
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<XTerm | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const sessionRef = useRef<string | null>(null);

  useEffect(() => {
    const el = hostRef.current;
    if (!el) return;

    const cs = getComputedStyle(document.documentElement);
    const read = (name: string, fallback: string) => {
      const v = cs.getPropertyValue(name).trim();
      return v || fallback;
    };

    const term = new XTerm({
      fontFamily:
        '"JetBrains Mono", "DejaVu Sans Mono", "Liberation Mono", "Cascadia Mono", "Consolas", monospace',
      fontSize: 13,
      lineHeight: 1.05,
      cursorBlink: true,
      scrollback: 5000,
      theme: {
        background: read("--color-nexus-bg", "#1a1c29"),
        foreground: read("--color-nexus-text", "#f2f3f9"),
        cursor: read("--color-neon-purple", "#bd9bff"),
        cursorAccent: read("--color-nexus-bg", "#1a1c29"),
        selectionBackground: read("--color-neon-purple", "#bd9bff") + "55",
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(el);
    termRef.current = term;
    fitRef.current = fit;
    try {
      fit.fit();
    } catch {
      /* 布局未就绪 */
    }
    term.write(`\x1b[36m正在连接 ${alias} …\x1b[0m\r\n`);
    term.focus();

    let unOut: (() => void) | undefined;
    let unExit: (() => void) | undefined;

    (async () => {
      try {
        const id = await api.sshTerminalOpen(alias, term.cols, term.rows);
        sessionRef.current = id;
        unOut = await listen<{ id: string; data: string }>("ssh-terminal-output", (e) => {
          if (e.payload.id === id) term.write(e.payload.data);
        });
        unExit = await listen<{ id: string }>("ssh-terminal-exit", (e) => {
          if (e.payload.id === id) term.write("\r\n\x1b[33m[连接已结束]\x1b[0m\r\n");
        });
      } catch (e) {
        term.write(`\x1b[31m连接失败: ${String(e)}\x1b[0m\r\n`);
      }
    })();

    term.onData((data) => {
      if (sessionRef.current) void api.sshTerminalWrite(sessionRef.current, data);
    });

    const onWinResize = () => {
      try {
        fit.fit();
      } catch {
        /* ignore */
      }
      if (sessionRef.current && termRef.current) {
        void api.sshTerminalResize(sessionRef.current, termRef.current.cols, termRef.current.rows);
      }
    };
    window.addEventListener("resize", onWinResize);

    return () => {
      window.removeEventListener("resize", onWinResize);
      unOut?.();
      unExit?.();
      if (sessionRef.current) void api.sshTerminalClose(sessionRef.current);
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
      sessionRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [alias]);

  // 从隐藏切回可见时重新适配尺寸并聚焦
  useEffect(() => {
    if (!visible) return;
    const t = setTimeout(() => {
      try {
        fitRef.current?.fit();
      } catch {
        /* ignore */
      }
      termRef.current?.focus();
      if (termRef.current && sessionRef.current) {
        void api.sshTerminalResize(sessionRef.current, termRef.current.cols, termRef.current.rows);
      }
    }, 60);
    return () => clearTimeout(t);
  }, [visible]);

  return <div ref={hostRef} className="absolute inset-0 h-full w-full px-2 py-1" />;
}
