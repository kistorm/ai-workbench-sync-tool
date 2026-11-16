import { createContext, useCallback, useContext, useEffect, useState } from "react";
import { createPortal } from "react-dom";
import type { ReactNode } from "react";

type ToastKind = "ok" | "err" | "info";
interface ToastItem {
  id: number;
  kind: ToastKind;
  title: string;
  detail?: string;
}

const ToastCtx = createContext<{
  push: (kind: ToastKind, title: string, detail?: string) => void;
}>({ push: () => {} });

export function useToast() {
  return useContext(ToastCtx);
}

let nextId = 1;

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);

  const push = useCallback((kind: ToastKind, title: string, detail?: string) => {
    const id = nextId++;
    setItems((prev) => [...prev, { id, kind, title, detail }]);
    setTimeout(() => {
      setItems((prev) => prev.filter((t) => t.id !== id));
    }, 4200);
  }, []);

  return (
    <ToastCtx.Provider value={{ push }}>
      {children}
      <div className="fixed bottom-12 right-5 z-50 flex flex-col gap-2 w-[380px]">
        {items.map((t) => (
          <Toast key={t.id} item={t} />
        ))}
      </div>
    </ToastCtx.Provider>
  );
}

function Toast({ item }: { item: ToastItem }) {
  const accent =
    item.kind === "ok"
      ? "var(--color-status-on)"
      : item.kind === "err"
        ? "var(--color-status-off)"
        : "var(--color-neon-cyan)";
  return (
    <div
      className="page-enter rounded-xl px-4 py-3 border backdrop-blur-md"
      style={{
        background: "var(--color-nexus-bg-elevated)",
        borderColor: "color-mix(in srgb, var(--color-nexus-text) 12%, transparent)",
        boxShadow: `0 0 0 1px ${accent}22, 0 8px 32px -8px rgba(0,0,0,0.8)`,
      }}
    >
      <div className="flex items-start gap-2.5">
        <span className="text-sm leading-5 shrink-0" style={{ color: accent }}>
          {item.kind === "ok" ? "✓" : item.kind === "err" ? "✕" : "·"}
        </span>
        <div className="min-w-0">
          <div className="text-[13px] text-[var(--color-nexus-text)]">{item.title}</div>
          {item.detail && (
            <div className="font-mono text-[10.5px] text-[var(--color-nexus-text-faint)] mt-1 whitespace-pre-wrap break-words max-h-32 overflow-y-auto">
              {item.detail}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

/** 模态框 */
export function Modal({
  open,
  title,
  onClose,
  children,
  footer,
}: {
  open: boolean;
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
}) {
  useEffect(() => {
    if (!open) return;
    const h = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [open, onClose]);

  if (!open) return null;
  // 用 Portal 挂到 body，避免父级 transform（如 page-enter 动画）破坏 fixed 定位
  return createPortal(
    <div className="fixed inset-0 z-50 flex items-center justify-center p-6" onMouseDown={onClose}>
      <div className="absolute inset-0 bg-black/60 backdrop-blur-sm" />
      <div
        className="page-enter relative w-[520px] max-w-full rounded-2xl border border-white/10 bg-[var(--color-nexus-bg-elevated)] shadow-2xl"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-5 py-4 border-b border-white/5">
          <h3 className="font-mono text-sm text-[var(--color-nexus-text)]">{title}</h3>
          <button
            onClick={onClose}
            className="text-[var(--color-nexus-text-faint)] hover:text-[var(--color-nexus-text)] transition"
          >
            ✕
          </button>
        </div>
        <div className="px-5 py-4 max-h-[60vh] overflow-y-auto">{children}</div>
        {footer && <div className="flex justify-end gap-2 px-5 py-3 border-t border-white/5">{footer}</div>}
      </div>
    </div>,
    document.body
  );
}

/** 加载/错误占位 */
export function Loading({ text = "加载中…" }: { text?: string }) {
  return (
    <div className="flex items-center gap-2 font-mono text-[11px] text-[var(--color-nexus-text-faint)]">
      <span className="dot dot--half" />
      {text}
    </div>
  );
}

export function ErrorBox({ message }: { message: string }) {
  return (
    <div className="rounded-xl border border-red-500/20 bg-red-500/5 px-4 py-3 font-mono text-[11px] text-red-300/90 whitespace-pre-wrap break-words">
      {message}
    </div>
  );
}
