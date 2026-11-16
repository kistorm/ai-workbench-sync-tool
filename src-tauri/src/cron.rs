use crate::core::{expand_tilde, home, run_cmd, AppState};
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tauri::State;

const MARKER: &str = "# nexus-workbench";

#[derive(Debug, Clone, Serialize)]
pub struct CronStatus {
    pub installed: bool,
    pub interval: Option<u32>,
    pub line: String,
    pub script_path: String,
}

/// 生成自动同步脚本（按当前配置的路径）
fn write_auto_sync_script(state: &AppState) -> Result<PathBuf, String> {
    let cfg = state.cfg();
    let data_dir = state.data_dir.clone();
    let _ = fs::create_dir_all(&data_dir);
    let script_path = data_dir.join("auto-sync.sh");

    let ssh_repo = home().join("ssh-config-sync");
    let ai_dir = expand_tilde(&cfg.ai_sync_dir);
    let notes_dir = expand_tilde(&cfg.notes_dir);
    let log_path = home().join(".local/state/nexus-workbench-sync.log");

    let script = format!(
        r#"#!/usr/bin/env bash
# 由 NEXUS Workbench 自动生成 —— 请勿手改（改配置请在应用内操作）
set -u
LOG="{log}"
mkdir -p "$(dirname "$LOG")"
echo "[$(date -Is)] ===== auto sync start =====" >> "$LOG"

# 1. SSH 配置仓库
if [ -d "{ssh_repo}/.git" ]; then
  (
    cd "{ssh_repo}" || exit 0
    git add -A
    git commit -qm "auto: $(date +%F_%T)" 2>/dev/null || true
    git pull -q --rebase --autostash -X ours 2>>"$LOG" || git rebase --abort 2>/dev/null || true
    git push -q 2>>"$LOG"
  ) && echo "[$(date -Is)] ssh-config ok" >> "$LOG" || echo "[$(date -Is)] ssh-config failed" >> "$LOG"
else
  echo "[$(date -Is)] ssh-config skipped (no repo)" >> "$LOG"
fi

# 2. AI 会话（收集到统一仓库 ai-sessions/，随数据仓库一起推送）
if [ -f "{ai_dir}/sync.sh" ]; then
  AI_SYNC_REPO="{ssh_repo}/ai-sessions" bash "{ai_dir}/sync.sh" push >> "$LOG" 2>&1 && echo "[$(date -Is)] ai-sync ok" >> "$LOG" || echo "[$(date -Is)] ai-sync failed" >> "$LOG"
else
  echo "[$(date -Is)] ai-sync skipped (no sync.sh)" >> "$LOG"
fi

# 3. 笔记
if [ -d "{notes_dir}/.git" ]; then
  (
    cd "{notes_dir}" || exit 0
    git add -A
    git commit -qm "auto notes: $(date +%F_%T)" 2>/dev/null || true
    git pull -q --rebase --autostash 2>>"$LOG" || git rebase --abort 2>/dev/null || true
    git push -q 2>>"$LOG"
  ) && echo "[$(date -Is)] notes ok" >> "$LOG" || echo "[$(date -Is)] notes failed" >> "$LOG"
else
  echo "[$(date -Is)] notes skipped (no repo)" >> "$LOG"
fi

echo "[$(date -Is)] ===== auto sync end =====" >> "$LOG"
"#,
        log = log_path.display(),
        ssh_repo = ssh_repo.display(),
        ai_dir = ai_dir.display(),
        notes_dir = notes_dir.display(),
    );

    fs::write(&script_path, script).map_err(|e| format!("写入脚本失败: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755));
    }
    Ok(script_path)
}

pub(crate) fn read_crontab() -> String {
    run_cmd("crontab", &["-l"], &[])
        .ok()
        .filter(|r| r.ok)
        .map(|r| r.stdout)
        .unwrap_or_default()
}

pub(crate) fn write_crontab(content: &str) -> Result<(), String> {
    let mut child = Command::new("crontab")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法执行 crontab: {}", e))?;
    {
        let stdin = child.stdin.as_mut().ok_or("无法写入 crontab stdin")?;
        stdin
            .write_all(content.as_bytes())
            .map_err(|e| format!("写入 crontab 失败: {}", e))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("crontab 执行失败: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "crontab 更新失败: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

pub(crate) fn strip_nexus_lines(content: &str) -> String {
    content
        .lines()
        .filter(|l| !l.contains(MARKER))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 安装/更新定时同步任务（核心逻辑，可测试）
pub fn cron_install_impl(state: &AppState, interval_min: u32) -> Result<CronStatus, String> {
    if interval_min == 0 {
        return Err("间隔必须大于 0 分钟".into());
    }
    let script = write_auto_sync_script(state)?;
    let existing = read_crontab();
    let cleaned = strip_nexus_lines(&existing);

    let schedule = if interval_min < 60 {
        format!("*/{} * * * *", interval_min.max(1))
    } else if interval_min == 60 {
        "0 * * * *".to_string()
    } else if interval_min % 60 == 0 {
        format!("0 */{} * * *", interval_min / 60)
    } else {
        format!("*/{} * * * *", interval_min.min(59))
    };

    let line = format!(
        "{} /bin/bash {} >/dev/null 2>&1 {}",
        schedule,
        script.display(),
        MARKER
    );
    let mut new_content = cleaned.trim_end().to_string();
    if !new_content.is_empty() {
        new_content.push('\n');
    }
    new_content.push_str(&line);
    new_content.push('\n');

    write_crontab(&new_content)?;

    {
        let mut cfg = state.config.lock().unwrap();
        cfg.sync_interval_min = interval_min;
        cfg.auto_sync_enabled = true;
        let cfg2 = cfg.clone();
        drop(cfg);
        state.save_config(&cfg2)?;
    }
    state.log(
        "info",
        "定时同步已安装",
        &format!("每 {} 分钟 · {}", interval_min, schedule),
    );

    Ok(CronStatus {
        installed: true,
        interval: Some(interval_min),
        line,
        script_path: script.to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub fn cron_install(state: State<AppState>, interval_min: u32) -> Result<CronStatus, String> {
    cron_install_impl(&state, interval_min)
}

/// 查询定时任务状态
#[tauri::command]
pub fn cron_status(state: State<AppState>) -> CronStatus {
    let content = read_crontab();
    let line = content
        .lines()
        .find(|l| l.contains(MARKER))
        .unwrap_or("")
        .to_string();
    let script_path = state
        .data_dir
        .join("auto-sync.sh")
        .to_string_lossy()
        .to_string();
    let interval = if line.is_empty() {
        None
    } else {
        // 从 cron 表达式里尽力解析
        let first = line.split_whitespace().next().unwrap_or("");
        if let Some(rest) = first.strip_prefix("*/") {
            rest.trim_end_matches(|c: char| !c.is_ascii_digit())
                .parse::<u32>()
                .ok()
        } else if first == "0" {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() > 1 {
                if let Some(rest) = fields[1].strip_prefix("*/") {
                    rest.parse::<u32>().ok().map(|h| h * 60)
                } else {
                    Some(60)
                }
            } else {
                Some(60)
            }
        } else {
            None
        }
    };
    CronStatus {
        installed: !line.is_empty(),
        interval,
        line,
        script_path,
    }
}

/// 移除定时任务（核心逻辑，可测试）
pub fn cron_remove_impl(state: &AppState) -> Result<CronStatus, String> {
    let existing = read_crontab();
    let cleaned = strip_nexus_lines(&existing);
    let mut new_content = cleaned.trim_end().to_string();
    if !new_content.is_empty() {
        new_content.push('\n');
    }
    write_crontab(&new_content)?;
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.auto_sync_enabled = false;
        let cfg2 = cfg.clone();
        drop(cfg);
        state.save_config(&cfg2)?;
    }
    state.log("info", "定时同步已移除", "crontab 条目已删除");
    Ok(CronStatus {
        installed: false,
        interval: None,
        line: String::new(),
        script_path: state
            .data_dir
            .join("auto-sync.sh")
            .to_string_lossy()
            .to_string(),
    })
}

#[tauri::command]
pub fn cron_remove(state: State<AppState>) -> Result<CronStatus, String> {
    cron_remove_impl(&state)
}

/// 静默查询：系统 crontab 是否已安装 nexus 任务（供后台定时器使用）
pub fn cron_status_quiet() -> bool {
    read_crontab().lines().any(|l| l.contains(MARKER))
}

/// 读取同步日志尾部（供前端展示）
#[tauri::command]
pub fn sync_log_tail(lines: Option<usize>) -> Vec<String> {
    let path = home().join(".local/state/nexus-workbench-sync.log");
    let Ok(content) = fs::read_to_string(&path) else {
        return vec![];
    };
    let all: Vec<&str> = content.lines().collect();
    let n = lines.unwrap_or(50).min(all.len());
    all[all.len() - n..].iter().map(|s| s.to_string()).collect()
}
