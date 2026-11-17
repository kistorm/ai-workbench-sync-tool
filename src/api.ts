import { invoke } from "@tauri-apps/api/core";

// ==================== 类型 ====================

export interface HostInfo {
  alias: string;
  hostname: string;
  user: string;
  port: number;
  inherit: string | null;
  gateways: string[];
  status: string;
  key_file: string | null;
  trust: boolean;
  shell_capable: boolean;
}

export interface TestResult {
  reachable: boolean;
  message: string;
  latency_ms: number;
}

export interface Activity {
  id: number;
  time: string;
  kind: string;
  title: string;
  detail: string;
}

export interface RunningAction {
  title: string;
  subtitle: string;
  progress: number;
  eta: string;
  active: boolean;
}

export interface AiTool {
  name: string;
  detected: boolean;
  sessions: number;
  last_sync: string;
  healthy: boolean;
}

export interface AiStatus {
  tools: AiTool[];
  repo_dir: string;
  script_found: boolean;
  last_commit: string;
  remote: string;
}

export interface NoteMeta {
  name: string;
  path: string;
  size: number;
  updated: string;
  updated_ts: number;
}

export interface Diagnosis {
  ok: boolean;
  message: string;
  latency_ms: number;
  raw: string;
}

export interface RepoInfo {
  dir: string;
  exists: boolean;
  is_git: boolean;
  remote: string;
  branch: string;
  dirty: number;
  last_commit: string;
}

export interface ClearResult {
  files: number;
  dirs: number;
  bytes: number;
}

export interface OnboardStatus {
  onboarded: boolean;
  git_repo: string;
}

export interface ElectermInfo {
  found: boolean;
  path: string;
  source: string;
  version: string;
}

export interface YoudaoToolInfo {
  project_dir: string;
  ready: boolean;
  python: string;
  has_cookie: boolean;
}

export interface YoudaoPullResult {
  ok: boolean;
  added: number;
  updated: number;
  skipped: number;
  output: string;
}

export interface SyncItemResult {
  name: string;
  ok: boolean;
  detail: string;
}

export interface CronStatus {
  installed: boolean;
  interval: number | null;
  line: string;
  script_path: string;
}

export interface AppConfig {
  git_repo: string;
  ssh_key: string;
  sync_interval_min: number;
  conflict_strategy: string;
  notes_dir: string;
  ai_sync_dir: string;
  ssh_dir: string;
  electerm_cmd: string;
  auto_sync_enabled: boolean;
  encrypt_password: string;
}

export interface SshKeyInfo {
  path: string;
  name: string;
  kind: string;
  has_pub: boolean;
}

export interface NotesInfo {
  dir: string;
  is_git: boolean;
  remote: string;
  home: string;
}

export interface AppInfo {
  home: string;
  data_dir: string;
  os: string;
  arch: string;
}

export interface SystemInfo {
  os_type: string;
  os_version: string;
  username: string;
  app_version: string;
}

export interface UpdateInfo {
  current: string;
  latest: string | null;
  has_update: boolean;
  release_url: string;
  changelog: string | null;
  message: string;
}

// ==================== API ====================

export const api = {
  // onboarding
  onboardStatus: () => invoke<OnboardStatus>("onboard_status"),
  onboard: (mode: "recover" | "fresh", gitRepo: string, password: string) =>
    invoke<string>("onboard", { mode, gitRepo, password }),

  // core
  getSyncConfig: () => invoke<AppConfig>("get_sync_config"),
  saveSyncConfig: (config: AppConfig) => invoke<void>("save_sync_config", { config }),
  listActivities: (limit = 50) => invoke<Activity[]>("list_activities", { limit }),
  logActivity: (kind: string, title: string, detail: string) =>
    invoke<void>("log_activity", { kind, title, detail }),
  getRunningAction: () => invoke<RunningAction>("get_running_action"),
  appInfo: () => invoke<AppInfo>("app_info"),
  getSystemInfo: () => invoke<SystemInfo>("system_info"),
  checkUpdate: () => invoke<UpdateInfo>("check_update"),
  openUrl: (url: string) => invoke<void>("open_url", { url }),

  // ssh
  listHosts: () => invoke<HostInfo[]>("list_hosts"),
  readAsshYml: () => invoke<string>("read_assh_yml"),
  saveAsshYml: (content: string) => invoke<void>("save_assh_yml", { content }),
  addHost: (p: {
    alias: string;
    hostname: string;
    user: string;
    port: number;
    inherit?: string | null;
    authMethod?: "key" | "password" | "none";
    keyFile?: string | null;
    password?: string | null;
    trustHost?: boolean;
  }) =>
    invoke<void>("add_host", {
      alias: p.alias,
      hostname: p.hostname,
      user: p.user,
      port: p.port,
      inherit: p.inherit ?? null,
      authMethod: p.authMethod ?? "none",
      keyFile: p.keyFile ?? null,
      password: p.password ?? null,
      trustHost: p.trustHost ?? true,
    }),
  deleteHost: (alias: string) => invoke<void>("delete_host", { alias }),
  buildSshConfig: () => invoke<string>("build_ssh_config"),
  testHost: (alias: string) => invoke<TestResult>("test_host", { alias }),
  openElecterm: (alias?: string) => invoke<string>("open_electerm", { alias: alias ?? null }),
  detectElecterm: () => invoke<ElectermInfo>("detect_electerm"),
  openPath: (path = "") => invoke<void>("open_path", { path }),
  validateAlias: (alias: string) => invoke<boolean>("validate_alias", { alias }),
  listSshKeys: () => invoke<SshKeyInfo[]>("list_ssh_keys"),
  setHostPassword: (alias: string, password: string) =>
    invoke<void>("set_host_password", { alias, password }),
  revealHostPassword: (alias: string) => invoke<string>("reveal_host_password", { alias }),
  updateHost: (p: {
    alias: string;
    newAlias: string;
    hostname: string;
    user: string;
    port: number;
    trustHost?: boolean;
  }) => invoke<void>("update_host", p),
  clearHostPassword: (alias: string) => invoke<void>("clear_host_password", { alias }),
  sshTerminalOpen: (alias: string, cols: number, rows: number) =>
    invoke<string>("ssh_terminal_open", { alias, cols, rows }),
  sshTerminalWrite: (id: string, data: string) =>
    invoke<void>("ssh_terminal_write", { id, data }),
  sshTerminalResize: (id: string, cols: number, rows: number) =>
    invoke<void>("ssh_terminal_resize", { id, cols, rows }),
  sshTerminalClose: (id: string) => invoke<void>("ssh_terminal_close", { id }),

  // ai
  aiStatus: () => invoke<AiStatus>("ai_status"),
  aiSyncRun: (action: "push" | "pull" | "status") => invoke<string>("ai_sync_run", { action }),

  // notes
  listNotes: () => invoke<NoteMeta[]>("list_notes"),
  readNote: (path: string) => invoke<string>("read_note", { path }),
  saveNote: (path: string, content: string) => invoke<void>("save_note", { path, content }),
  createNote: (name: string) => invoke<string>("create_note", { name }),
  deleteNote: (path: string) => invoke<void>("delete_note", { path }),
  notesGitSync: (action: "push" | "pull" | "status") =>
    invoke<string>("notes_git_sync", { action }),
  notesInfo: () => invoke<NotesInfo>("notes_info"),
  clearNotes: () => invoke<ClearResult>("clear_notes"),
  youdaoInfo: () => invoke<YoudaoToolInfo>("youdao_info"),
  youdaoPull: (cookies: string, ydnoteDir?: string) =>
    invoke<YoudaoPullResult>("youdao_pull", { cookies, ydnoteDir: ydnoteDir ?? null }),

  // sync
  diagnoseGit: (repo?: string, key?: string) =>
    invoke<Diagnosis>("diagnose_git", { repo: repo ?? null, key: key ?? null }),
  gitSync: (action: "push" | "pull" | "status", dir?: string) =>
    invoke<string>("git_sync", { action, dir: dir ?? null }),
  repoInfo: (dir?: string) => invoke<RepoInfo>("repo_info", { dir: dir ?? null }),
  syncAll: (action: "push" | "pull" = "push") =>
    invoke<SyncItemResult[]>("sync_all", { action }),
  initDataRepo: () => invoke<string>("init_data_repo"),
  forceRestore: () => invoke<string>("force_restore"),
  forcePullSshConfig: () => invoke<string>("force_pull_ssh_config"),
  pushSshConfig: () => invoke<string>("push_ssh_config"),
  hasPassword: (alias: string) => invoke<boolean>("has_password", { alias }),

  // cron
  cronInstall: (intervalMin: number) => invoke<CronStatus>("cron_install", { intervalMin }),
  cronStatus: () => invoke<CronStatus>("cron_status"),
  cronRemove: () => invoke<CronStatus>("cron_remove"),
  syncLogTail: (lines = 50) => invoke<string[]>("sync_log_tail", { lines }),
};

/** 统一错误转字符串 */
export function errMsg(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  if (e && typeof e === "object") {
    try {
      return JSON.stringify(e);
    } catch {
      return String(e);
    }
  }
  return String(e);
}
