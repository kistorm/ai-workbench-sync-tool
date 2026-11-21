use crate::core::{count_files, expand_tilde, home, run_cmd_dir, AppState, RunningAction};
use serde::Serialize;
use std::path::PathBuf;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
pub struct AiTool {
    pub name: String,
    pub detected: bool,
    pub sessions: u64,
    pub last_sync: String,
    pub healthy: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AiStatus {
    pub tools: Vec<AiTool>,
    pub repo_dir: String,
    pub script_found: bool,
    pub last_commit: String,
    pub remote: String,
}

fn ai_dir(state: &AppState) -> PathBuf {
    expand_tilde(&state.cfg().ai_sync_dir)
}

/// AI 会话数据的收集目录：并入统一的配置仓库，随配置一起推送
pub fn ai_data_dir() -> PathBuf {
    crate::sync::config_repo_dir().join("ai-sessions")
}

/// 首次迁移：把旧独立仓库里的会话数据搬到统一仓库的 ai-sessions/
pub fn ensure_ai_data(state: &AppState) -> Result<PathBuf, String> {
    let target = ai_data_dir();
    let legacy = ai_dir(state);
    std::fs::create_dir_all(&target).map_err(|e| format!("创建 {} 失败: {}", target.display(), e))?;

    // 旧目录里已有数据时迁移（排除脚本本身与 .git）
    if legacy.exists() && legacy != target {
        let marker = target.join(".migrated");
        if !marker.exists() {
            for sub in ["dsh", "zcode", "claude", "cursor", "trae"] {
                let src = legacy.join(sub);
                if !src.is_dir() {
                    continue;
                }
                let dst = target.join(sub);
                // 用 rsync 快速镜像（无 rsync 时回退 cp）
                let r = crate::core::run_cmd(
                    "rsync",
                    &[
                        "-a",
                        "--ignore-existing",
                        &format!("{}/", src.display()),
                        &format!("{}/", dst.display()),
                    ],
                    &[],
                );
                if !r.map(|x| x.ok).unwrap_or(false) {
                    let _ = std::fs::create_dir_all(&dst);
                    copy_tree(&src, &dst);
                }
            }
            let _ = std::fs::write(&marker, format!("migrated at {}\n", chrono::Local::now()));
            state.log(
                "sync",
                "AI 会话已迁移到统一仓库",
                &target.to_string_lossy(),
            );
        }
    }
    Ok(target)
}

fn copy_tree(src: &std::path::Path, dst: &std::path::Path) {
    if let Ok(entries) = std::fs::read_dir(src) {
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name();
            if name == ".git" {
                continue;
            }
            let d = dst.join(&name);
            if p.is_dir() {
                let _ = std::fs::create_dir_all(&d);
                copy_tree(&p, &d);
            } else if p.is_file() {
                let _ = std::fs::copy(&p, &d);
            }
        }
    }
}

/// 最后一条 git commit 的描述
fn last_commit(dir: &PathBuf) -> String {
    if !dir.join(".git").exists() {
        return "未初始化".into();
    }
    run_cmd_dir("git", &["log", "-1", "--format=%cr · %s"], dir, &[])
        .ok()
        .map(|r| r.stdout.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "无提交".into())
}

/// ZCode 的真实会话数：db.sqlite 的 session 表（rollout 只是模型 IO 日志，数量少且不准）
pub(crate) fn zcode_session_count(zcode_dir: &std::path::Path) -> u64 {
    let db = zcode_dir.join("cli").join("db").join("db.sqlite");
    if db.is_file() {
        if let Ok(con) = rusqlite::Connection::open_with_flags(
            &db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            if let Ok(n) = con.query_row("select count(*) from session", [], |r| {
                r.get::<_, i64>(0)
            }) {
                if n >= 0 {
                    return n as u64;
                }
            }
        }
    }
    // 数据库不可用时回退到 rollout 日志数
    count_files(&zcode_dir.join("cli").join("rollout"), ".jsonl")
}

fn detect_tools(overrides: &crate::core::AiPaths) -> Vec<AiTool> {
    let h = home();
    let ov = |base: String, custom: &str| -> std::path::PathBuf {
        if custom.trim().is_empty() { base.into() } else { crate::core::expand_tilde(custom) }
    };
    let mut tools = vec![];

    // ZCode
    let zcode_dir = ov(h.join(".zcode").to_string_lossy().to_string(), &overrides.zcode);
    let zcode_detected = zcode_dir.exists();
    tools.push(AiTool {
        name: "ZCode".into(),
        detected: zcode_detected,
        sessions: if zcode_detected {
            zcode_session_count(&zcode_dir)
        } else {
            0
        },
        last_sync: String::new(),
        healthy: zcode_detected,
    });

    // Claude Code
    let claude_dir = ov(h.join(".claude").to_string_lossy().to_string(), &overrides.claude).join("projects");
    let claude_detected = claude_dir.exists();
    tools.push(AiTool {
        name: "Claude Code".into(),
        detected: claude_detected,
        sessions: if claude_detected {
            count_files(&claude_dir, ".jsonl")
        } else {
            0
        },
        last_sync: String::new(),
        healthy: claude_detected,
    });

    // Cursor
    let cursor_candidates = [
        ov(h.join(".config/Cursor").to_string_lossy().to_string(), &overrides.cursor)
            .join("User/globalStorage/state.vscdb"),
        h.join("Library/Application Support/Cursor/User/globalStorage/state.vscdb"),
        h.join("AppData/Roaming/Cursor/User/globalStorage/state.vscdb"),
    ];
    let cursor_detected = cursor_candidates.iter().any(|p| p.exists());
    tools.push(AiTool {
        name: "Cursor".into(),
        detected: cursor_detected,
        sessions: if cursor_detected { 1 } else { 0 },
        last_sync: String::new(),
        healthy: cursor_detected,
    });

    // Trae
    let trae_candidates = [
        ov(h.join(".config/Trae").to_string_lossy().to_string(), &overrides.trae)
            .join("User/globalStorage/state.vscdb"),
        h.join("Library/Application Support/Trae/User/globalStorage/state.vscdb"),
        h.join("AppData/Roaming/Trae/User/globalStorage/state.vscdb"),
    ];
    let trae_detected = trae_candidates.iter().any(|p| p.exists());
    tools.push(AiTool {
        name: "Trae".into(),
        detected: trae_detected,
        sessions: if trae_detected { 1 } else { 0 },
        last_sync: String::new(),
        healthy: trae_detected,
    });

    // 豆包桌面版（Electron 应用；聊天记录主要在厂商云端，本地同步其配置与数据目录做备份）
    let doubao_candidates = [
        ov(
            h.join(".config/Doubao").to_string_lossy().to_string(),
            &overrides.doubao,
        ),
        h.join("Library/Application Support/Doubao"),
        {
            let app_data = std::env::var("APPDATA")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| h.join("AppData/Roaming"));
            app_data.join("Doubao")
        },
    ];
    let doubao_dir = doubao_candidates.into_iter().find(|p| p.is_dir());
    let doubao_detected = doubao_dir.is_some();
    tools.push(AiTool {
        name: "豆包".into(),
        detected: doubao_detected,
        sessions: if doubao_detected {
            let d = doubao_dir.as_ref().unwrap();
            count_files(d, ".json") + count_files(d, ".jsonl")
        } else {
            0
        },
        last_sync: String::new(),
        healthy: doubao_detected,
    });

    // WorkBuddy（Electron 应用，目录名大小写不一）
    let wb_candidates = [
        ov(
            h.join(".config/WorkBuddy").to_string_lossy().to_string(),
            &overrides.workbuddy,
        ),
        ov(
            h.join(".config/workbuddy").to_string_lossy().to_string(),
            &overrides.workbuddy,
        ),
        h.join("Library/Application Support/WorkBuddy"),
        h.join("Library/Application Support/workbuddy"),
        {
            let app_data = std::env::var("APPDATA")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| h.join("AppData/Roaming"));
            app_data.join("WorkBuddy")
        },
        {
            let app_data = std::env::var("APPDATA")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| h.join("AppData/Roaming"));
            app_data.join("workbuddy")
        },
    ];
    let workbuddy_dir = wb_candidates.into_iter().find(|p| p.is_dir());
    let workbuddy_detected = workbuddy_dir.is_some();
    tools.push(AiTool {
        name: "WorkBuddy".into(),
        detected: workbuddy_detected,
        sessions: if workbuddy_detected {
            let d = workbuddy_dir.as_ref().unwrap();
            count_files(d, ".json") + count_files(d, ".jsonl")
        } else {
            0
        },
        last_sync: String::new(),
        healthy: workbuddy_detected,
    });

    // DSH (DeepSeek Harness)
    let dsh_dir = ov(h.join(".dsh").to_string_lossy().to_string(), &overrides.dsh).join("sessions");
    let dsh_detected = dsh_dir.exists();
    tools.push(AiTool {
        name: "DSH".into(),
        detected: dsh_detected,
        sessions: if dsh_detected {
            // 会话文件为 session.v4.jsonl.zstd
            count_files(&dsh_dir, ".jsonl.zstd") + count_files(&dsh_dir, ".zst")
        } else {
            0
        },
        last_sync: String::new(),
        healthy: dsh_detected,
    });

    tools
}

#[tauri::command]
pub fn ai_status(state: State<AppState>) -> AiStatus {
    ai_status_impl(&state)
}

pub fn ai_status_impl(state: &AppState) -> AiStatus {
    // 数据收集到统一仓库；脚本仍在 ai_sync_dir
    let data_dir = ai_data_dir();
    let script_dir = ai_dir(state);
    let script_found = script_dir.join("sync.sh").exists();

    // 远程与最近提交取统一仓库
    let repo = crate::sync::config_repo_dir();
    let remote = run_cmd_dir("git", &["remote", "get-url", "origin"], &repo, &[])
        .ok()
        .filter(|r| r.ok)
        .map(|r| r.stdout.trim().to_string())
        .unwrap_or_default();
    let commit = last_commit(&repo);

    let mut tools = detect_tools(&state.cfg().ai_paths);
    for t in tools.iter_mut() {
        t.last_sync = commit.clone();
    }
    AiStatus {
        tools,
        repo_dir: data_dir.to_string_lossy().to_string(),
        script_found,
        last_commit: commit,
        remote,
    }
}

/// 执行 ai-session-sync 的 push/pull/status
#[tauri::command]
pub async fn ai_sync_run(state: State<'_, AppState>, action: String) -> Result<String, String> {
    let dir = ai_dir(&state);
    let script = dir.join("sync.sh");
    if !script.exists() {
        return Err(format!(
            "未找到 {}。请先 git clone ai-session-sync-public 到 {}",
            script.display(),
            dir.display()
        ));
    }
    let action = action.trim().to_lowercase();
    if !["push", "pull", "status"].contains(&action.as_str()) {
        return Err(format!("不支持的操作: {}", action));
    }
    let st = state.inner().clone();
    let data_dir = match ensure_ai_data(&st) {
        Ok(d) => d,
        Err(e) => return Err(e),
    };
    tauri::async_runtime::spawn_blocking(move || {
        st.set_running(RunningAction {
            title: format!("AI 会话 {}", action),
            subtitle: format!("收集到 {} 后统一推送", data_dir.display()),
            progress: 40,
            eta: "执行中".into(),
            active: true,
        });
        // pull 前先从统一仓库拉取（数据在 ai-sessions/ 内）
        if action == "pull" {
            let _ = crate::sync::sync_data_repo(&st, "pull");
        }

        // Windows 依赖 Git for Windows 提供的 bash 执行 sync.sh
        #[cfg(target_os = "windows")]
        {
            let has_bash = crate::core::run_cmd("bash", &["--version"], &[])
                .map(|r| r.ok)
                .unwrap_or(false);
            if !has_bash {
                st.clear_running();
                return Err(
                    "Windows 上 AI 会话同步需要 Git for Windows 提供的 bash，请安装 Git 后重启应用"
                        .into(),
                );
            }
        }
        // AI_SYNC_REPO 指定收集目录；统一模式下脚本只收集，不自推
        let res = run_cmd_dir(
            "bash",
            &["sync.sh", &action],
            &dir,
            &[("AI_SYNC_REPO", data_dir.to_string_lossy().to_string())],
        );
        st.clear_running();

        // push 后再把统一仓库提交并推送，确保会话数据真正上云
        if action == "push" && res.as_ref().map(|r| r.ok).unwrap_or(false) {
            let pushed = crate::sync::sync_data_repo(&st, "push");
            if !pushed.ok {
                st.log("error", "AI 会话推送失败", &pushed.detail);
                return Err(format!("AI 数据已收集，但统一仓库推送失败: {}", pushed.detail));
            }
        }

        match res {
            Ok(r) => {
                let combined = r.combined();
                if r.ok {
                    st.log(
                        "sync",
                        &format!("AI 会话 {} 完成", action),
                        &first_lines(&combined, 3),
                    );
                    Ok(combined)
                } else {
                    st.log(
                        "error",
                        &format!("AI 会话 {} 失败", action),
                        &first_lines(&combined, 3),
                    );
                    Err(combined)
                }
            }
            Err(e) => {
                st.log("error", &format!("AI 会话 {} 失败", action), &e);
                Err(e)
            }
        }
    })
    .await
    .map_err(|e| format!("任务失败: {}", e))?
}

fn first_lines(s: &str, n: usize) -> String {
    s.lines().take(n).collect::<Vec<_>>().join(" | ")
}
