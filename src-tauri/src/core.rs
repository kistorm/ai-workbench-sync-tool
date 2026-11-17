use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

/// 用户主目录
pub fn home() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(p) = std::env::var("USERPROFILE") {
            return PathBuf::from(p);
        }
    }
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// 展开 ~/ 前缀
pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        home().join(rest)
    } else if p == "~" {
        home()
    } else {
        PathBuf::from(p)
    }
}

/// 安全拼接：拒绝绝对路径和 .. 逃逸
pub fn safe_join(base: &Path, rel: &str) -> Result<PathBuf, String> {
    let p = Path::new(rel);
    if p.is_absolute() {
        return Err("不允许绝对路径".into());
    }
    for c in p.components() {
        if matches!(c, std::path::Component::ParentDir) {
            return Err("不允许 .. 路径".into());
        }
    }
    Ok(base.join(p))
}

// ==================== 配置 ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub git_repo: String,
    pub ssh_key: String,
    pub sync_interval_min: u32,
    pub conflict_strategy: String,
    pub notes_dir: String,
    pub ai_sync_dir: String,
    pub ssh_dir: String,
    pub electerm_cmd: String,
    pub auto_sync_enabled: bool,
    /// 加密主密码：非空时，所有同步内容会逐文件加密后上传
    pub encrypt_password: String,
    /// 是否已完成首次引导
    pub onboarded: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            git_repo: String::new(),
            ssh_key: "~/.ssh/id_ed25519_github".into(),
            sync_interval_min: 60,
            conflict_strategy: "newest_wins".into(),
            notes_dir: "~/ssh-config-sync/notes".into(),
            ai_sync_dir: "~/ai-session-sync".into(),
            ssh_dir: "~/.ssh".into(),
            electerm_cmd: "electerm".into(),
            auto_sync_enabled: true,
            encrypt_password: String::new(),
            onboarded: false,
        }
    }
}

// ==================== 活动日志 ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    pub id: u64,
    pub time: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunningAction {
    pub title: String,
    pub subtitle: String,
    pub progress: u32,
    pub eta: String,
    pub active: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActivityStore {
    pub next_id: u64,
    pub items: Vec<Activity>,
}

// ==================== 应用状态 ====================

/// 可跨线程克隆的应用状态（内部字段都用 Arc 共享）
#[derive(Clone)]
pub struct AppState {
    pub data_dir: PathBuf,
    pub config: Arc<Mutex<AppConfig>>,
    pub running: Arc<Mutex<RunningAction>>,
    pub activities: Arc<Mutex<ActivityStore>>,
}

impl AppState {
    pub fn init(data_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&data_dir);
        let mut config = fs::read_to_string(data_dir.join("config.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<AppConfig>(&s).ok())
            .unwrap_or_default();

        // 一次性迁移：把旧的独立笔记目录并入同步仓库，让笔记能随配置一起推送
        if config.notes_dir == "~/notes" {
            let repo = home().join("ssh-config-sync");
            if repo.join(".git").exists() {
                let target = repo.join("notes");
                let legacy = home().join("notes");
                let _ = fs::create_dir_all(&target);
                let mut migrated = 0;
                if let Ok(entries) = fs::read_dir(&legacy) {
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.is_file() {
                            let dst = target.join(e.file_name());
                            if !dst.exists() && fs::copy(&p, &dst).is_ok() {
                                migrated += 1;
                            }
                        }
                    }
                }
                config.notes_dir = target.to_string_lossy().to_string();
                if migrated > 0 {
                    let _ = fs::write(
                        data_dir.join("migration.log"),
                        format!(
                            "{}: 迁移 {} 篇笔记到 {}\n",
                            chrono::Local::now().format("%F %T"),
                            migrated,
                            target.display()
                        ),
                    );
                }
            }
        }

        let activities = fs::read_to_string(data_dir.join("activities.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<ActivityStore>(&s).ok())
            .unwrap_or_default();

        let config_json = serde_json::to_string_pretty(&config).ok();
        if let Some(s) = config_json {
            let _ = fs::write(data_dir.join("config.json"), s);
        }

        Self {
            data_dir,
            config: Arc::new(Mutex::new(config)),
            running: Arc::new(Mutex::new(RunningAction::default())),
            activities: Arc::new(Mutex::new(activities)),
        }
    }

    pub fn cfg(&self) -> AppConfig {
        self.config.lock().unwrap().clone()
    }

    pub fn save_config(&self, cfg: &AppConfig) -> Result<(), String> {
        let s = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
        fs::write(self.data_dir.join("config.json"), s).map_err(|e| e.to_string())
    }

    pub fn save_activities(&self) {
        if let Ok(store) = self.activities.lock() {
            if let Ok(s) = serde_json::to_string_pretty(&*store) {
                let _ = fs::write(self.data_dir.join("activities.json"), s);
            }
        }
    }

    /// 追加一条活动（最新在前，保留 200 条）
    pub fn log(&self, kind: &str, title: &str, detail: &str) {
        if let Ok(mut store) = self.activities.lock() {
            store.next_id += 1;
            let id = store.next_id;
            store.items.insert(
                0,
                Activity {
                    id,
                    time: chrono::Local::now().format("%H:%M").to_string(),
                    kind: kind.to_string(),
                    title: title.to_string(),
                    detail: detail.to_string(),
                },
            );
            store.items.truncate(200);
        }
        self.save_activities();
    }

    pub fn set_running(&self, action: RunningAction) {
        if let Ok(mut r) = self.running.lock() {
            *r = action;
        }
    }

    pub fn clear_running(&self) {
        if let Ok(mut r) = self.running.lock() {
            *r = RunningAction::default();
        }
    }
}

// ==================== 命令执行 ====================

#[derive(Debug, Clone, Serialize)]
pub struct CmdResult {
    pub ok: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CmdResult {
    pub fn combined(&self) -> String {
        let mut s = self.stdout.clone();
        if !self.stderr.is_empty() {
            if !s.is_empty() {
                s.push('\n');
            }
            s.push_str(&self.stderr);
        }
        s
    }
}

/// 执行外部命令
pub fn run_cmd(program: &str, args: &[&str], envs: &[(&str, String)]) -> Result<CmdResult, String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("执行 {} 失败: {}", program, e))?;
    Ok(CmdResult {
        ok: out.status.success(),
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    })
}

/// 在目录中执行命令
pub fn run_cmd_dir(
    program: &str,
    args: &[&str],
    cwd: &Path,
    envs: &[(&str, String)],
) -> Result<CmdResult, String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.current_dir(cwd);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("执行 {} 失败: {}", program, e))?;
    Ok(CmdResult {
        ok: out.status.success(),
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    })
}

/// 在目录中执行命令，并把 stderr 的每一行（含 `\r` 刷新的进度行）回调出去。
/// 用于解析 git 的 `Writing objects: 42%` / `Receiving objects: 42%` 进度。
pub fn run_cmd_dir_stream(
    program: &str,
    args: &[&str],
    cwd: &Path,
    envs: &[(&str, String)],
    mut on_line: impl FnMut(&str),
) -> Result<CmdResult, String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.current_dir(cwd);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("执行 {} 失败: {}", program, e))?;

    // stdout 在独立线程读取，避免管道写满导致子进程阻塞
    let stdout_handle = child.stdout.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            buf
        })
    });

    let mut err_out = String::new();
    let mut line_buf = String::new();
    if let Some(mut s) = child.stderr.take() {
        let mut chunk = [0u8; 4096];
        loop {
            let n = s
                .read(&mut chunk)
                .map_err(|e| format!("读取 {} 输出失败: {}", program, e))?;
            if n == 0 {
                break;
            }
            let text = String::from_utf8_lossy(&chunk[..n]);
            for ch in text.chars() {
                if ch == '\r' || ch == '\n' {
                    if !line_buf.is_empty() {
                        on_line(&line_buf);
                        err_out.push_str(&line_buf);
                        err_out.push('\n');
                        line_buf.clear();
                    }
                } else {
                    line_buf.push(ch);
                }
            }
        }
    }
    if !line_buf.is_empty() {
        on_line(&line_buf);
        err_out.push_str(&line_buf);
        err_out.push('\n');
    }

    let status = child
        .wait()
        .map_err(|e| format!("等待 {} 结束失败: {}", program, e))?;
    let stdout = stdout_handle
        .and_then(|h| h.join().ok())
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .unwrap_or_default();
    Ok(CmdResult {
        ok: status.success(),
        code: status.code(),
        stdout,
        stderr: err_out,
    })
}

/// 提交时使用的兜底身份：仅当该仓库读不到 user.email 时生效。
/// 不修改用户的全局 git 配置，避免新机器首次提交直接失败。
pub fn git_identity_env(dir: &Path) -> Vec<(&'static str, String)> {
    let configured = run_cmd_dir("git", &["config", "user.email"], dir, &[])
        .map(|r| r.ok && !r.stdout.trim().is_empty())
        .unwrap_or(false);
    if configured {
        Vec::new()
    } else {
        vec![
            ("GIT_AUTHOR_NAME", "NEXUS Workbench".to_string()),
            ("GIT_AUTHOR_EMAIL", "nexus@localhost".to_string()),
            ("GIT_COMMITTER_NAME", "NEXUS Workbench".to_string()),
            ("GIT_COMMITTER_EMAIL", "nexus@localhost".to_string()),
        ]
    }
}

/// 递归统计指定扩展名的文件数
pub fn count_files(dir: &Path, ext: &str) -> u64 {
    let mut n = 0u64;
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            n += count_files(&path, ext);
        } else if path.to_string_lossy().ends_with(ext) {
            n += 1;
        }
    }
    n
}

/// 相对时间描述
pub fn rel_time(ts: i64) -> String {
    let now = chrono::Local::now().timestamp();
    let diff = now.saturating_sub(ts);
    if diff < 60 {
        "刚刚".into()
    } else if diff < 3600 {
        format!("{}分钟前", diff / 60)
    } else if diff < 86400 {
        chrono::DateTime::from_timestamp(ts, 0)
            .map(|d| d.with_timezone(&chrono::Local).format("%H:%M").to_string())
            .unwrap_or_else(|| format!("{}小时前", diff / 3600))
    } else if diff < 86400 * 2 {
        "昨天".into()
    } else {
        format!("{}天前", diff / 86400)
    }
}

// ==================== 密码存储（本地，权限 600） ====================

fn secrets_path(state: &AppState) -> PathBuf {
    state.data_dir.join("secrets.json")
}

fn load_secrets(state: &AppState) -> serde_json::Map<String, serde_json::Value> {
    fs::read_to_string(secrets_path(state))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// 保存某主机的密码（仅存本地 app_data，权限 600，不进 git）
pub fn save_password(state: &AppState, alias: &str, password: &str) -> Result<(), String> {
    let mut map = load_secrets(state);
    map.insert(
        alias.to_string(),
        serde_json::Value::String(password.to_string()),
    );
    let s = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
    let path = secrets_path(state);
    fs::write(&path, s).map_err(|e| format!("保存密码失败: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn get_password(state: &AppState, alias: &str) -> Option<String> {
    load_secrets(state)
        .get(alias)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

pub fn delete_password(state: &AppState, alias: &str) -> Result<(), String> {
    let mut map = load_secrets(state);
    if map.remove(alias).is_some() {
        let s = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
        fs::write(secrets_path(state), s).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn has_password(state: tauri::State<AppState>, alias: String) -> bool {
    get_password(&state, &alias).is_some()
}

// ==================== 前端命令 ====================

#[tauri::command]
pub fn get_sync_config(state: tauri::State<AppState>) -> AppConfig {
    state.cfg()
}

#[tauri::command]
pub fn save_sync_config(state: tauri::State<AppState>, config: AppConfig) -> Result<(), String> {
    // Git 地址严格校验：只允许 git@host:owner/repo.git 格式
    if !config.git_repo.trim().is_empty() {
        crate::sync::validate_git_repo_url(&config.git_repo)?;
    }
    *state.config.lock().unwrap() = config.clone();
    state.save_config(&config)?;
    // 立即把新仓库地址应用到本地仓库，避免"界面填了但推送还去旧仓库"
    let repo_dir = crate::sync::config_repo_dir();
    let mut remote_note = String::new();
    if repo_dir.join(".git").exists() && !config.git_repo.trim().is_empty() {
        match crate::sync::apply_remote(&repo_dir, &config.git_repo) {
            Ok(_) => remote_note = format!("，远程已更新为 {}", config.git_repo),
            Err(e) => remote_note = format!("，但更新远程失败: {}", e),
        }
    }
    state.log(
        "info",
        "同步配置已保存",
        &format!("配置已写入 config.json{}", remote_note),
    );
    Ok(())
}

#[tauri::command]
pub fn list_activities(state: tauri::State<AppState>, limit: Option<usize>) -> Vec<Activity> {
    let store = state.activities.lock().unwrap();
    let n = limit.unwrap_or(50).min(store.items.len());
    store.items[..n].to_vec()
}

#[tauri::command]
pub fn log_activity(state: tauri::State<AppState>, kind: String, title: String, detail: String) {
    state.log(&kind, &title, &detail);
}

#[tauri::command]
pub fn get_running_action(state: tauri::State<AppState>) -> RunningAction {
    state.running.lock().unwrap().clone()
}

#[tauri::command]
pub fn app_info(state: tauri::State<AppState>) -> serde_json::Value {
    serde_json::json!({
        "home": home().to_string_lossy(),
        "data_dir": state.data_dir.to_string_lossy(),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    })
}

// ==================== 系统与升级 ====================

#[derive(Debug, Clone, Serialize)]
pub struct SystemInfo {
    pub os_type: String,     // "Linux" / "macOS" / "Windows"
    pub os_version: String,  // 如 "Ubuntu 24.04.1 LTS"
    pub username: String,    // 当前登录账户
    pub app_version: String, // 应用版本
}

/// 平台版本描述：Linux 读 /etc/os-release 的 PRETTY_NAME，macOS 用 sw_vers，Windows 用 ver
#[cfg(target_os = "linux")]
fn os_version() -> String {
    if let Ok(s) = fs::read_to_string("/etc/os-release") {
        for line in s.lines() {
            if let Some(v) = line.strip_prefix("PRETTY_NAME=") {
                return v.trim_matches('"').to_string();
            }
        }
    }
    "Linux".into()
}

#[cfg(target_os = "macos")]
fn os_version() -> String {
    match run_cmd("sw_vers", &["-productVersion"], &[]) {
        Ok(r) if r.ok && !r.stdout.trim().is_empty() => r.stdout.trim().to_string(),
        _ => "macOS".into(),
    }
}

#[cfg(target_os = "windows")]
fn os_version() -> String {
    match run_cmd("cmd", &["/C", "ver"], &[]) {
        Ok(r) if r.ok && !r.stdout.trim().is_empty() => r.stdout.trim().to_string(),
        _ => "Windows".into(),
    }
}

/// 当前登录账户：Linux/macOS 取 USER（回退 LOGNAME），Windows 取 USERNAME
#[cfg(not(target_os = "windows"))]
fn current_username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "unknown".into())
}

#[cfg(target_os = "windows")]
fn current_username() -> String {
    std::env::var("USERNAME").unwrap_or_else(|_| "unknown".into())
}

/// 汇总系统与应用信息，供设置页展示
#[tauri::command]
pub fn system_info() -> SystemInfo {
    let os_type = match std::env::consts::OS {
        "linux" => "Linux",
        "macos" => "macOS",
        "windows" => "Windows",
        _ => std::env::consts::OS,
    };
    SystemInfo {
        os_type: os_type.to_string(),
        os_version: os_version(),
        username: current_username(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub current: String,        // 当前版本
    pub latest: Option<String>, // 远端最新 tag 版本（无则 None）
    pub has_update: bool,
    pub release_url: String,        // 最新 release 页面
    pub changelog: Option<String>,  // 该版本的更新日志（来自仓库 CHANGELOG.md）
    pub message: String,            // 给用户看的中文结果说明
}

/// 版本号解析成 (major, minor, patch)：不足三段补 0，含非数字段返回 None
fn parse_version_triple(s: &str) -> Option<(u64, u64, u64)> {
    let segs: Vec<&str> = s.split('.').collect();
    if segs.is_empty() || segs.len() > 3 {
        return None;
    }
    let mut n = [0u64; 3];
    for (i, seg) in segs.iter().enumerate() {
        if seg.is_empty() {
            return None;
        }
        n[i] = seg.parse().ok()?;
    }
    Some((n[0], n[1], n[2]))
}

/// 从 ls-remote --tags 的输出里取最大的语义化版本 tag
fn latest_tag_version(output: &str) -> Option<(u64, u64, u64)> {
    let mut best: Option<(u64, u64, u64)> = None;
    for line in output.lines() {
        // 行格式：<sha>\trefs/tags/v1.2.3（附注 tag 会多一行 ...^{} 剥离行）
        let Some((_sha, tag)) = line.split_once('\t') else {
            continue;
        };
        let Some(name) = tag.strip_prefix("refs/tags/") else {
            continue;
        };
        let name = name.strip_suffix("^{}").unwrap_or(name);
        let name = name.strip_prefix('v').unwrap_or(name);
        let Some(v) = parse_version_triple(name) else {
            continue;
        };
        if best.map_or(true, |b| v > b) {
            best = Some(v);
        }
    }
    best
}

/// 检查更新：与私库远端的最新 tag 比对版本（网络失败不报错，前端友好展示）
#[tauri::command]
pub async fn check_update(state: tauri::State<'_, AppState>) -> Result<UpdateInfo, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || check_update_impl(&st))
        .await
        .map_err(|e| format!("任务失败: {}", e))
}

/// 应用自身代码仓库（升级检查/更新日志固定查这里，与用户的数据仓库无关）
const APP_REPO_SLUG: &str = "kistorm/ai-workbench-sync-tool";
const APP_REPO_URL: &str = "git@github.com:kistorm/ai-workbench-sync-tool.git";

/// 从 CHANGELOG.md 文本里提取指定版本（"## v1.2.3"）到下一个 "## " 的段落
pub(crate) fn extract_changelog_section(text: &str, version: &str) -> Option<String> {
    let header = format!("## v{}", version);
    let mut out = String::new();
    let mut in_section = false;
    for line in text.lines() {
        if line.starts_with("## ") {
            if in_section {
                break;
            }
            if line.starts_with(&header) {
                in_section = true;
                out.push_str(line);
                out.push('\n');
            }
        } else if in_section {
            out.push_str(line);
            out.push('\n');
        }
    }
    if in_section && out.trim().len() > header.len() {
        Some(out.trim_end().to_string())
    } else {
        None
    }
}

/// 浅克隆应用仓库到本地缓存，读取 CHANGELOG.md 中指定版本的更新日志
fn fetch_changelog(st: &AppState, version: &str, ssh_env: &[(&str, String)]) -> Option<String> {
    let cache = st.data_dir.join("update-cache").join("app-repo");
    let ok = if cache.join(".git").exists() {
        // 已缓存：fetch 最新 main 并复位
        let fetched = run_cmd_dir(
            "git",
            &["fetch", "--depth", "1", "-q", "origin", "main"],
            &cache,
            ssh_env,
        )
        .map(|r| r.ok)
        .unwrap_or(false);
        fetched
            && run_cmd_dir("git", &["reset", "--hard", "-q", "FETCH_HEAD"], &cache, &[])
                .map(|r| r.ok)
                .unwrap_or(false)
    } else {
        let _ = fs::create_dir_all(st.data_dir.join("update-cache"));
        let _ = fs::remove_dir_all(&cache);
        run_cmd(
            "git",
            &[
                "clone",
                "--depth",
                "1",
                "-q",
                APP_REPO_URL,
                cache.to_str().unwrap_or(""),
            ],
            ssh_env,
        )
        .map(|r| r.ok)
        .unwrap_or(false)
    };
    if !ok {
        return None;
    }
    let text = fs::read_to_string(cache.join("CHANGELOG.md")).ok()?;
    extract_changelog_section(&text, version)
}

fn check_update_impl(st: &AppState) -> UpdateInfo {
    let current = env!("CARGO_PKG_VERSION").to_string();
    // 升级检查固定查应用代码仓库（不随用户的数据仓库变化）
    let release_url = format!("https://github.com/{}/releases/latest", APP_REPO_SLUG);
    let ssh_env = crate::sync::git_ssh_env(st);

    // 直接对应用代码仓库执行 ls-remote
    let result = run_cmd("git", &["ls-remote", "--tags", APP_REPO_URL], &ssh_env);

    let output = match result {
        Ok(r) if r.ok => r.stdout,
        _ => {
            return UpdateInfo {
                current,
                latest: None,
                has_update: false,
                release_url,
                changelog: None,
                message: "无法获取远端版本（请检查网络或仓库配置）".into(),
            };
        }
    };

    // 远端没有可识别的版本 tag：视为最新
    let Some(v) = latest_tag_version(&output) else {
        return UpdateInfo {
            current: current.clone(),
            latest: None,
            has_update: false,
            release_url,
            changelog: None,
            message: "远端还没有可识别的版本 tag".into(),
        };
    };

    let cur = parse_version_triple(&current).unwrap_or((0, 0, 0));
    let has_update = v > cur;
    let latest = format!("{}.{}.{}", v.0, v.1, v.2);
    let message = if has_update {
        format!("发现新版本 v{}，请在设置页点击「前往下载」", latest)
    } else {
        "当前已是最新版本".into()
    };
    // 拉取该版本的更新日志（失败不影响检查结果）
    let changelog = fetch_changelog(st, &latest, &ssh_env);
    UpdateInfo {
        current,
        latest: Some(latest),
        has_update,
        release_url,
        changelog,
        message,
    }
}

/// 用系统默认程序打开网页链接（仅允许 http/https）
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("只允许打开 http/https 链接".into());
    }
    #[cfg(target_os = "linux")]
    let r = run_cmd("xdg-open", &[&url], &[])?;
    #[cfg(target_os = "macos")]
    let r = run_cmd("open", &[&url], &[])?;
    #[cfg(target_os = "windows")]
    let r = run_cmd("cmd", &["/C", "start", "", &url], &[])?;
    if !r.ok {
        return Err("打开链接失败".into());
    }
    Ok(())
}
