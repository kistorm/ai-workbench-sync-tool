import { useCallback, useEffect, useRef, useState } from "react";

/** 异步数据加载 hook：手动 reload + 自动轮询 */
export function useAsync<T>(
  loader: () => Promise<T>,
  deps: unknown[] = [],
  opts: { pollMs?: number; initial?: T } = {}
) {
  const [data, setData] = useState<T | undefined>(opts.initial);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(true);

  const reload = useCallback(async () => {
    try {
      const d = await loader();
      if (alive.current) {
        setData(d);
        setError(null);
      }
    } catch (e) {
      if (alive.current) setError(String(e));
    } finally {
      if (alive.current) setLoading(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  useEffect(() => {
    alive.current = true;
    setLoading(true);
    void reload();
    return () => {
      alive.current = false;
    };
  }, [reload]);

  useEffect(() => {
    if (!opts.pollMs) return;
    const id = setInterval(() => void reload(), opts.pollMs);
    return () => clearInterval(id);
  }, [opts.pollMs, reload]);

  return { data, loading, error, reload, setData };
}

/** 操作执行 hook：管理 busy 状态 */
export function useAction() {
  const [busy, setBusy] = useState<string | null>(null);
  const run = useCallback(
    async <R,>(label: string, fn: () => Promise<R>): Promise<{ ok: boolean; data?: R; error?: string }> => {
      setBusy(label);
      try {
        const data = await fn();
        return { ok: true, data };
      } catch (e) {
        return { ok: false, error: String(e) };
      } finally {
        setBusy(null);
      }
    },
    []
  );
  return { busy, run };
}
