/** 主题与应用名称管理（持久化到 localStorage） */

export type ThemeKey = "default" | "grass" | "ocean" | "mono" | "light";

export interface ThemeDef {
  key: ThemeKey;
  name: string;
  desc: string;
  /** 预览用色：背景 / 卡片 / 主强调 */
  preview: { bg: string; card: string; accent: string; text: string };
}

export const THEMES: ThemeDef[] = [
  {
    key: "default",
    name: "紫青霓虹",
    desc: "赛博夜色 · 默认",
    preview: { bg: "#1a1c29", card: "#222535", accent: "#bd9bff", text: "#f2f3f9" },
  },
  {
    key: "grass",
    name: "草绿森系",
    desc: "墨绿底 + 嫩芽绿 · 克制自然",
    preview: { bg: "#1a2b22", card: "#22382c", accent: "#96f588", text: "#edf8f0" },
  },
  {
    key: "ocean",
    name: "深蓝深海",
    desc: "海军蓝 + 冰青 · 冷静专业",
    preview: { bg: "#16253a", card: "#1e3050", accent: "#6faeff", text: "#eff5fc" },
  },
  {
    key: "mono",
    name: "黑白极简",
    desc: "纯黑 + 灰度层次 · 零彩色",
    preview: { bg: "#1a1a1a", card: "#222222", accent: "#ffffff", text: "#fbfbfb" },
  },
  {
    key: "light",
    name: "浅绿晨光",
    desc: "浅薄荷底 + 森林绿 · 明亮护眼",
    preview: { bg: "#eef7f0", card: "#ffffff", accent: "#2e9e6b", text: "#17281f" },
  },
];

const THEME_KEY = "nexus.theme";
const NAME_KEY = "nexus.appName";
export const DEFAULT_APP_NAME = "NEXUS";

export function loadTheme(): ThemeKey {
  const v = localStorage.getItem(THEME_KEY);
  if (v && THEMES.some((t) => t.key === v)) return v as ThemeKey;
  return "default";
}

export function applyTheme(theme: ThemeKey) {
  document.documentElement.setAttribute("data-theme", theme);
  localStorage.setItem(THEME_KEY, theme);
}

export function loadAppName(): string {
  return localStorage.getItem(NAME_KEY) || DEFAULT_APP_NAME;
}

export function saveAppName(name: string) {
  const clean = name.trim() || DEFAULT_APP_NAME;
  localStorage.setItem(NAME_KEY, clean);
  document.title = `${clean} Workbench`;
}
