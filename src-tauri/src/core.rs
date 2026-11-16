use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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
