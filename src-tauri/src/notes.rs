use crate::core::{expand_tilde, home, run_cmd_dir, rel_time, safe_join, AppState};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::State;

#[derive(Debug, Clone, Serialize)]
pub struct NoteMeta {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub updated: String,
    pub updated_ts: i64,
}

/// 向上查找包含该目录的 git 仓库根（笔记可能位于同步仓库的子目录中）
fn find_git_root(path: &Path) -> Option<PathBuf> {
    let mut cur = path.to_path_buf();
    loop {
        if cur.join(".git").exists() {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

fn notes_dir(state: &AppState) -> PathBuf {
    let d = expand_tilde(&state.cfg().notes_dir);
    let _ = fs::create_dir_all(&d);
    d
}

fn is_markdown(p: &Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()),
        Some("md") | Some("markdown")
    )
}

/// 递归收集 markdown 文件（跳过 .git 和隐藏目录）
fn collect_notes(base: &Path, dir: &Path, out: &mut Vec<NoteMeta>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect_notes(base, &path, out);
        } else if is_markdown(&path) {
            let meta = fs::metadata(&path).ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let ts = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let rel = path
                .strip_prefix(base)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| name.clone());
            out.push(NoteMeta {
                name: path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or(name),
                path: rel,
                size,
                updated: rel_time(ts),
                updated_ts: ts,
            });
        }
    }
}

#[tauri::command]
pub fn list_notes(state: State<AppState>) -> Result<Vec<NoteMeta>, String> {
    list_notes_impl(&state)
}

pub fn list_notes_impl(state: &AppState) -> Result<Vec<NoteMeta>, String> {
    let base = notes_dir(state);
    let mut out = vec![];
    collect_notes(&base, &base, &mut out);
    out.sort_by(|a, b| b.updated_ts.cmp(&a.updated_ts));
    Ok(out)
}

#[tauri::command]
pub fn read_note(state: State<AppState>, path: String) -> Result<String, String> {
    read_note_impl(&state, &path)
}

pub fn read_note_impl(state: &AppState, path: &str) -> Result<String, String> {
    let base = notes_dir(state);
    let full = safe_join(&base, path)?;
    fs::read_to_string(&full).map_err(|e| format!("读取 {} 失败: {}", full.display(), e))
}

#[tauri::command]
pub fn save_note(state: State<AppState>, path: String, content: String) -> Result<(), String> {
    save_note_impl(&state, &path, &content)
}

pub fn save_note_impl(state: &AppState, path: &str, content: &str) -> Result<(), String> {
    let base = notes_dir(state);
    let full = safe_join(&base, path)?;
    if let Some(parent) = full.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&full, content).map_err(|e| format!("写入失败: {}", e))?;
    state.log("note", "笔记已保存", path);
    Ok(())
}

#[tauri::command]
pub fn create_note(state: State<AppState>, name: String) -> Result<String, String> {
    create_note_impl(&state, &name)
}

pub fn create_note_impl(state: &AppState, name: &str) -> Result<String, String> {
    let base = notes_dir(state);
    let clean = name.trim().replace(['/', '\\'], "-");
    let clean = if clean.is_empty() {
        "未命名笔记".to_string()
    } else {
        clean
    };
    let mut filename = format!("{}.md", clean);
    let mut i = 2;
    while base.join(&filename).exists() {
        filename = format!("{}-{}.md", clean, i);
        i += 1;
    }
    let full = base.join(&filename);
    let content = format!("# {}\n\n", clean);
    fs::write(&full, content).map_err(|e| format!("创建失败: {}", e))?;
    state.log("note", "已新建笔记", &filename);
    Ok(filename)
}

#[tauri::command]
pub fn delete_note(state: State<AppState>, path: String) -> Result<(), String> {
    delete_note_impl(&state, &path)
}

pub fn delete_note_impl(state: &AppState, path: &str) -> Result<(), String> {
    let base = notes_dir(state);
    let full = safe_join(&base, path)?;
    if full
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e != "md" && e != "markdown")
        .unwrap_or(true)
    {
        return Err("只允许删除 Markdown 笔记".into());
    }
    fs::remove_file(&full).map_err(|e| format!("删除失败: {}", e))?;
    state.log("note", "已删除笔记", path);
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct ClearResult {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
}

/// 递归统计文件数与总大小
fn count_all(dir: &Path) -> (u64, u64) {
    let mut files = 0u64;
    let mut bytes = 0u64;
    let Ok(entries) = fs::read_dir(dir) else {
        return (0, 0);
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            let (f, b) = count_all(&p);
            files += f;
            bytes += b;
        } else if p.is_file() {
            files += 1;
            bytes += fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        }
    }
    (files, bytes)
}

/// 清空笔记目录下的全部内容（真删除，不可恢复）
/// 保留目录本身与 .git（避免破坏仓库）
#[tauri::command]
pub fn clear_notes(state: State<AppState>) -> Result<ClearResult, String> {
    clear_notes_impl(&state)
}

pub fn clear_notes_impl(state: &AppState) -> Result<ClearResult, String> {
    let base = notes_dir(state);
    if !base.exists() {
        return Ok(ClearResult {
            files: 0,
            dirs: 0,
            bytes: 0,
        });
    }

    let mut files = 0u64;
    let mut dirs = 0u64;
    let mut bytes = 0u64;

    let entries = fs::read_dir(&base).map_err(|e| format!("读取目录失败: {}", e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        // 保护 git 仓库元数据
        if name == ".git" {
            continue;
        }
        if path.is_dir() {
            let (f, b) = count_all(&path);
            files += f;
            bytes += b;
            dirs += 1;
            fs::remove_dir_all(&path)
                .map_err(|e| format!("删除目录 {} 失败: {}", path.display(), e))?;
        } else if path.is_file() {
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            files += 1;
            bytes += size;
            fs::remove_file(&path)
                .map_err(|e| format!("删除文件 {} 失败: {}", path.display(), e))?;
        }
    }

    state.log(
        "note",
        "已清空全部笔记",
        &format!("删除 {} 个文件、{} 个目录、{:.1} MB", files, dirs, bytes as f64 / 1048576.0),
    );

    Ok(ClearResult { files, dirs, bytes })
}

/// 笔记目录的 git 同步
#[tauri::command]
pub async fn notes_git_sync(state: State<'_, AppState>, action: String) -> Result<String, String> {
    let base = notes_dir(&state);
    let Some(git_root) = find_git_root(&base) else {
        return Err(format!(
            "{} 不在任何 git 仓库内。请在同步配置页初始化数据仓库，或手动 git init。",
            base.display()
        ));
    };
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let action = action.to_lowercase();
        let res = match action.as_str() {
            "push" => {
                let add = run_cmd_dir("git", &["add", "-A"], &git_root, &[])?;
                if !add.ok {
                    return Err(add.combined());
                }
                // 无变化时 commit 会失败，忽略
                let _ = run_cmd_dir(
                    "git",
                    &["commit", "-qm", &format!("notes: {}", chrono::Local::now().format("%F %T"))],
                    &git_root,
                    &[],
                );
                let pull = run_cmd_dir("git", &["pull", "-q", "--rebase", "--autostash", "-X", "ours"], &git_root, &[])?;
                let push = run_cmd_dir("git", &["push", "-q"], &git_root, &[])?;
                format!("{}\n{}", pull.combined(), push.combined())
            }
            "pull" => {
                let r = run_cmd_dir("git", &["pull", "-q", "--rebase", "--autostash", "-X", "ours"], &git_root, &[])?;
                if !r.ok {
                    return Err(r.combined());
                }
                r.combined()
            }
            "status" => {
                let r = run_cmd_dir("git", &["status", "--short"], &git_root, &[])?;
                r.combined()
            }
            _ => return Err(format!("不支持的操作: {}", action)),
        };
        st.log("sync", &format!("笔记 {}", action), "完成");
        Ok(res)
    })
    .await
    .map_err(|e| format!("任务失败: {}", e))?
}

/// 笔记目录信息
#[tauri::command]
pub fn notes_info(state: State<AppState>) -> serde_json::Value {
    let base = notes_dir(&state);
    let git_root = find_git_root(&base);
    let is_git = git_root.is_some();
    let remote = if let Some(root) = &git_root {
        run_cmd_dir("git", &["remote", "get-url", "origin"], root, &[])
            .ok()
            .map(|r| r.stdout.trim().to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };
    serde_json::json!({
        "dir": base.to_string_lossy(),
        "is_git": is_git,
        "remote": remote,
        "home": home().to_string_lossy(),
    })
}
