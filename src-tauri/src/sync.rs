use crate::core::{expand_tilde, home, run_cmd, run_cmd_dir, AppState};
use std::fs;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
pub struct Diagnosis {
    pub ok: bool,
    pub message: String,
    pub latency_ms: u64,
    pub raw: String,
}

/// 构造 GIT_SSH_COMMAND
fn ssh_command(key: &str) -> String {
    let key_path = expand_tilde(key);
    format!(
        "ssh -i {} -o IdentitiesOnly=yes -o ConnectTimeout=8 -o StrictHostKeyChecking=accept-new",
        key_path.display()
    )
}

/// 诊断 git 远程连通性（核心逻辑，可测试）
pub fn diagnose_git_impl(repo: &str, key: &str) -> Result<Diagnosis, String> {
    if repo.trim().is_empty() {
        return Err("未配置 Git 仓库地址".into());
    }
    let env = [("GIT_SSH_COMMAND", ssh_command(key))];
    let start = Instant::now();
    let res = run_cmd("git", &["ls-remote", repo], &env);
    let ms = start.elapsed().as_millis() as u64;
    match res {
        Ok(r) => {
            let raw = r.combined();
            if r.ok {
                let count = r.stdout.lines().count();
                Ok(Diagnosis {
                    ok: true,
                    message: format!("连接正常 · {}ms · {} 个引用", ms, count),
                    latency_ms: ms,
                    raw,
                })
            } else {
                let lower = raw.to_lowercase();
                let msg = if lower.contains("permission denied") || lower.contains("publickey") {
                    "认证失败：SSH 密钥未授权给该仓库".to_string()
                } else if lower.contains("could not resolve")
                    || lower.contains("name or service")
                {
                    "域名解析失败".to_string()
                } else if lower.contains("timed out") || lower.contains("timeout") {
                    "连接超时".to_string()
                } else if lower.contains("repository not found")
                    || lower.contains("does not appear")
                {
                    "仓库不存在或无权限".to_string()
                } else {
                    raw.lines().next().unwrap_or("未知错误").to_string()
                };
                Ok(Diagnosis {
                    ok: false,
                    message: msg,
                    latency_ms: ms,
                    raw,
                })
            }
        }
        Err(e) => Err(e),
    }
}

/// 诊断 git 远程连通性（git ls-remote）
#[tauri::command]
pub async fn diagnose_git(
    state: State<'_, AppState>,
    repo: Option<String>,
    key: Option<String>,
) -> Result<Diagnosis, String> {
    let cfg = state.cfg();
    let repo = repo.filter(|s| !s.trim().is_empty()).unwrap_or(cfg.git_repo);
    let key = key.filter(|s| !s.trim().is_empty()).unwrap_or(cfg.ssh_key);
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = diagnose_git_impl(&repo, &key);
        if let Ok(d) = &result {
            if d.ok {
                st.log("sync", "Git 通信诊断成功", &d.message);
            } else {
                st.log("error", "Git 通信诊断失败", &d.message);
            }
        } else if let Err(e) = &result {
            st.log("error", "Git 通信诊断失败", e);
        }
        result
    })
    .await
    .map_err(|e| format!("任务失败: {}", e))?
}

/// 同步仓库目录：SSH 配置 + 笔记都在这里，一次 push 全部上传
pub fn config_repo_dir() -> PathBuf {
    home().join("ssh-config-sync")
}

/// 把配置里的仓库地址应用到本地仓库的 origin（用户改地址后能立即生效）
pub fn apply_remote(dir: &Path, repo: &str) -> Result<(), String> {
    let repo = repo.trim();
    if repo.is_empty() {
        return Ok(());
    }
    let current = run_cmd_dir("git", &["remote", "get-url", "origin"], dir, &[]);
    match current {
        Ok(c) if c.ok && c.stdout.trim() == repo => Ok(()),
        Ok(c) if c.ok => {
            let r = run_cmd_dir("git", &["remote", "set-url", "origin", repo], dir, &[])?;
            if r.ok {
                Ok(())
            } else {
                Err(format!("更新远程地址失败: {}", r.combined()))
            }
        }
        _ => {
            let r = run_cmd_dir("git", &["remote", "add", "origin", repo], dir, &[])?;
            if r.ok {
                Ok(())
            } else {
                Err(format!("添加远程地址失败: {}", r.combined()))
            }
        }
    }
}

/// 当前分支名
fn current_branch(dir: &Path) -> String {
    run_cmd_dir("git", &["branch", "--show-current"], dir, &[])
        .ok()
        .map(|r| r.stdout.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "main".into())
}

/// 远端是否已有该分支（空仓库首次推送时没有）
fn remote_has_branch(dir: &Path, branch: &str) -> bool {
    run_cmd_dir("git", &["ls-remote", "--heads", "origin", branch], dir, &[])
        .map(|r| r.ok && !r.stdout.trim().is_empty())
        .unwrap_or(false)
}

/// 推送（首次自动设置 upstream，兼容空仓库）
fn push_branch(dir: &Path, branch: &str) -> CmdResultResult {
    run_cmd_dir("git", &["push", "-q", "-u", "origin", branch], dir, &[])
}

type CmdResultResult = Result<crate::core::CmdResult, String>;

/// 对同步仓库执行 add/commit/pull/push
#[tauri::command]
pub async fn git_sync(
    state: State<'_, AppState>,
    action: String,
    dir: Option<String>,
) -> Result<String, String> {
    let target = match dir.filter(|s| !s.trim().is_empty()) {
        Some(d) => expand_tilde(&d),
        None => config_repo_dir(),
    };
    if !target.join(".git").exists() {
        return Err(format!(
            "{} 不是 git 仓库。请先 clone 同步仓库。",
            target.display()
        ));
    }
    let st = state.inner().clone();
    let repo = state.cfg().git_repo;
    tauri::async_runtime::spawn_blocking(move || {
        // 让配置里的仓库地址生效
        apply_remote(&target, &repo)?;
        let action = action.to_lowercase();
        let mut log_buf = String::new();
        match action.as_str() {
            "push" => {
                let add = run_cmd_dir("git", &["add", "-A"], &target, &[])?;
                log_buf.push_str(&add.combined());
                if !add.ok {
                    return Err(log_buf);
                }
                // 无变化时 commit 失败属正常
                let commit = run_cmd_dir(
                    "git",
                    &["commit", "-qm", &format!("auto sync: {}", chrono::Local::now().format("%F %T"))],
                    &target,
                    &[],
                )?;
                log_buf.push_str(&commit.combined());
                let pull = run_cmd_dir(
                    "git",
                    &["pull", "-q", "--rebase", "--autostash", "-X", "ours"],
                    &target,
                    &[],
                )?;
                log_buf.push_str(&pull.combined());
                if !pull.ok {
                    let _ = run_cmd_dir("git", &["rebase", "--abort"], &target, &[]);
                    return Err(format!("拉取合并冲突，已中止。\n{}", log_buf));
                }
                let branch = current_branch(&target);
                let push = run_cmd_dir("git", &["push", "-q", "-u", "origin", &branch], &target, &[])?;
                log_buf.push_str(&push.combined());
                if !push.ok {
                    return Err(log_buf);
                }
                st.log("sync", "配置仓库已推送", "git push 完成");
            }
            "pull" => {
                let branch = current_branch(&target);
                if !remote_has_branch(&target, &branch) {
                    return Ok("远端为空，无需拉取".to_string());
                }
                let pull = run_cmd_dir(
                    "git",
                    &["pull", "-q", "--rebase", "--autostash", "-X", "ours"],
                    &target,
                    &[],
                )?;
                log_buf.push_str(&pull.combined());
                if !pull.ok {
                    let _ = run_cmd_dir("git", &["rebase", "--abort"], &target, &[]);
                    return Err(log_buf);
                }
                st.log("sync", "配置仓库已拉取", "git pull 完成");
            }
            "status" => {
                let r = run_cmd_dir("git", &["status", "--short", "--branch"], &target, &[])?;
                log_buf.push_str(&r.combined());
            }
            _ => return Err(format!("不支持的操作: {}", action)),
        }
        Ok(log_buf)
    })
    .await
    .map_err(|e| format!("任务失败: {}", e))?
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoInfo {
    pub dir: String,
    pub exists: bool,
    pub is_git: bool,
    pub remote: String,
    pub branch: String,
    pub dirty: u32,
    pub last_commit: String,
}

/// 同步仓库概况
#[tauri::command]
pub fn repo_info(_state: State<AppState>, dir: Option<String>) -> RepoInfo {
    let target = match dir.filter(|s| !s.trim().is_empty()) {
        Some(d) => expand_tilde(&d),
        None => config_repo_dir(),
    };
    let exists = target.exists();
    let is_git = target.join(".git").exists();
    let mut remote = String::new();
    let mut branch = String::new();
    let mut dirty = 0u32;
    let mut last_commit = String::new();
    if is_git {
        remote = run_cmd_dir("git", &["remote", "get-url", "origin"], &target, &[])
            .ok()
            .map(|r| r.stdout.trim().to_string())
            .unwrap_or_default();
        branch = run_cmd_dir("git", &["branch", "--show-current"], &target, &[])
            .ok()
            .map(|r| r.stdout.trim().to_string())
            .unwrap_or_default();
        dirty = run_cmd_dir("git", &["status", "--porcelain"], &target, &[])
            .ok()
            .map(|r| r.stdout.lines().filter(|l| !l.trim().is_empty()).count() as u32)
            .unwrap_or(0);
        last_commit = run_cmd_dir("git", &["log", "-1", "--format=%cr · %s"], &target, &[])
            .ok()
            .map(|r| r.stdout.trim().to_string())
            .unwrap_or_default();
    }
    RepoInfo {
        dir: target.to_string_lossy().to_string(),
        exists,
        is_git,
        remote,
        branch,
        dirty,
        last_commit,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncItemResult {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

/// 同步「SSH 配置 + 笔记」所在的数据仓库
pub fn sync_data_repo(st: &AppState, action: &str) -> SyncItemResult {
    let name = "SSH 配置 + 笔记".to_string();
    let repo_dir = config_repo_dir();
    let cfg = st.cfg();

    if !repo_dir.join(".git").exists() {
        return SyncItemResult {
            name,
            ok: false,
            detail: format!("{} 不是 git 仓库（可在同步配置页初始化）", repo_dir.display()),
        };
    }
    if let Err(e) = apply_remote(&repo_dir, &cfg.git_repo) {
        return SyncItemResult {
            name,
            ok: false,
            detail: e,
        };
    }

    let push_like = action == "push";
    if push_like {
        let add = run_cmd_dir("git", &["add", "-A"], &repo_dir, &[]);
        if !add.as_ref().map(|a| a.ok).unwrap_or(false) {
            return SyncItemResult {
                name,
                ok: false,
                detail: format!(
                    "git add 失败: {}",
                    add.map(|a| a.combined().lines().next().unwrap_or("").to_string())
                        .unwrap_or_else(|e| e)
                ),
            };
        }
        // 无变化时 commit 会失败，忽略
        let _ = run_cmd_dir(
            "git",
            &[
                "commit",
                "-qm",
                &format!("auto: {}", chrono::Local::now().format("%F %T")),
            ],
            &repo_dir,
            &[],
        );
    }

    let branch = current_branch(&repo_dir);

    // 远端有分支才拉取（空仓库首次推送时跳过，否则会报 couldn't find remote ref）
    if remote_has_branch(&repo_dir, &branch) {
        let pull = run_cmd_dir(
            "git",
            &["pull", "-q", "--rebase", "--autostash", "-X", "ours"],
            &repo_dir,
            &[],
        );
        if !pull.as_ref().map(|p| p.ok).unwrap_or(false) {
            let _ = run_cmd_dir("git", &["rebase", "--abort"], &repo_dir, &[]);
            return SyncItemResult {
                name,
                ok: false,
                detail: "拉取失败或合并冲突，已中止（本地数据未改动）".into(),
            };
        }
    }

    if !push_like {
        // 拉取会更新 assh.yml，这里同步重建 ~/.ssh/config，否则 ssh 别名仍是旧的
        let build_note = match crate::ssh::build_ssh_config_impl(st) {
            Ok(_) => "，已重建 ~/.ssh/config".to_string(),
            Err(e) => format!("，但重建 ssh config 失败: {}", e),
        };
        return SyncItemResult {
            name,
            ok: true,
            detail: if remote_has_branch(&repo_dir, &branch) {
                format!("已从远程拉取（分支 {}）{}", branch, build_note)
            } else {
                format!("远端为空，无需拉取{}", build_note)
            },
        };
    }

    match push_branch(&repo_dir, &branch) {
        Ok(p) if p.ok => SyncItemResult {
            name,
            ok: true,
            detail: format!("已推送 → {} ({})", cfg.git_repo, branch),
        },
        Ok(p) => SyncItemResult {
            name,
            ok: false,
            detail: format!(
                "推送失败: {}",
                p.combined().lines().next().unwrap_or("未知错误")
            ),
        },
        Err(e) => SyncItemResult {
            name,
            ok: false,
            detail: e,
        },
    }
}

/// 同步 AI 会话：收集到统一仓库的 ai-sessions/，随配置一起推送
fn sync_ai_repo(st: &AppState, action: &str) -> SyncItemResult {
    let cfg = st.cfg();
    let name = "AI 会话".to_string();
    let script_dir = expand_tilde(&cfg.ai_sync_dir);
    if !script_dir.join("sync.sh").exists() {
        return SyncItemResult {
            name,
            ok: false,
            detail: format!("未找到 {}/sync.sh", script_dir.display()),
        };
    }
    let data_dir = match crate::ai::ensure_ai_data(st) {
        Ok(d) => d,
        Err(e) => {
            return SyncItemResult {
                name,
                ok: false,
                detail: e,
            }
        }
    };
    let act = if action == "pull" { "pull" } else { "push" };
    // AI_SYNC_REPO 让脚本收集到统一仓库的 ai-sessions/，
    // 其内部 git 操作会自动落到上级数据仓库（不需要独立 remote）
    match run_cmd_dir(
        "bash",
        &["sync.sh", act],
        &script_dir,
        &[("AI_SYNC_REPO", data_dir.to_string_lossy().to_string())],
    ) {
        Ok(r) if r.ok => SyncItemResult {
            name,
            ok: true,
            detail: format!(
                "已{}到统一仓库 ai-sessions/",
                if act == "pull" { "拉取" } else { "收集并推送" }
            ),
        },
        Ok(r) => SyncItemResult {
            name,
            ok: false,
            detail: format!(
                "失败: {}",
                r.combined().lines().next().unwrap_or("未知错误")
            ),
        },
        Err(e) => SyncItemResult {
            name,
            ok: false,
            detail: e,
        },
    }
}

/// 一键同步全部（阻塞版本，供命令和后台定时器共用）
pub fn run_sync_all_blocking(st: &AppState, action: &str) -> Vec<SyncItemResult> {
    st.set_running(crate::core::RunningAction {
        title: if action == "pull" { "全量拉取" } else { "全量推送" }.into(),
        subtitle: "SSH 配置 + 笔记 · AI 会话".into(),
        progress: 40,
        eta: "执行中".into(),
        active: true,
    });

    let mut results = Vec::new();
    // 先让 AI 脚本把会话收集进 ai-sessions/（它自己会提交/推送一次），
    // 再统一同步数据仓库，确保 SSH 配置/笔记/会话都在同一次推送里保持一致
    results.push(sync_ai_repo(st, action));
    results.push(sync_data_repo(st, action));

    st.clear_running();
    let ok_count = results.iter().filter(|r| r.ok).count();
    let summary = results
        .iter()
        .map(|r| format!("{}: {}", r.name, if r.ok { "成功" } else { "失败" }))
        .collect::<Vec<_>>()
        .join(" | ");
    st.log(
        if ok_count == results.len() { "sync" } else { "error" },
        &format!(
            "全量{}完成 ({}/{})",
            if action == "pull" { "拉取" } else { "推送" },
            ok_count,
            results.len()
        ),
        &summary,
    );
    results
}

/// 一键同步全部（action: push / pull）
#[tauri::command]
pub async fn sync_all(
    state: State<'_, AppState>,
    action: Option<String>,
) -> Result<Vec<SyncItemResult>, String> {
    let st = state.inner().clone();
    let act = action.unwrap_or_else(|| "push".into());
    let act2 = if act == "pull" { "pull" } else { "push" }.to_string();
    tauri::async_runtime::spawn_blocking(move || run_sync_all_blocking(&st, &act2))
        .await
        .map_err(|e| format!("任务失败: {}", e))
}

/// 从云端强制恢复：丢弃本地未推送的修改，用远程版本覆盖
#[tauri::command]
pub async fn force_restore(state: State<'_, AppState>) -> Result<String, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repo = config_repo_dir();
        let cfg = st.cfg();
        if !repo.join(".git").exists() {
            return Err(format!("{} 不是 git 仓库", repo.display()));
        }
        apply_remote(&repo, &cfg.git_repo)?;

        let branch = current_branch(&repo);
        let fetch = run_cmd_dir("git", &["fetch", "-q", "origin"], &repo, &[])?;
        if !fetch.ok {
            return Err(format!("拉取远程失败: {}", fetch.combined()));
        }
        let target = format!("origin/{}", branch);
        let check = run_cmd_dir("git", &["rev-parse", "--verify", &target], &repo, &[])?;
        if !check.ok {
            return Err(format!("远程分支 {} 不存在", target));
        }
        let reset = run_cmd_dir("git", &["reset", "--hard", &target], &repo, &[])?;
        if !reset.ok {
            return Err(format!("重置失败: {}", reset.combined()));
        }
        // 重建 ~/.ssh/config，让恢复后的 assh.yml 立即生效
        let cfg_msg = crate::ssh::build_ssh_config_impl(&st)
            .unwrap_or_else(|e| format!("（重建 ssh config 失败: {}）", e));
        st.log(
            "sync",
            "已从云端强制恢复",
            &format!("重置到 {}，本地未推送修改已丢弃", target),
        );
        Ok(format!("已从云端恢复（{}）\n{}", target, cfg_msg))
    })
    .await
    .map_err(|e| format!("任务失败: {}", e))?
}

/// 初始化数据仓库（当前不是 git 仓库时使用）
#[tauri::command]
pub fn init_data_repo(state: State<AppState>) -> Result<String, String> {
    let repo_dir = config_repo_dir();
    if repo_dir.join(".git").exists() {
        return Ok(format!("{} 已是 git 仓库", repo_dir.display()));
    }
    fs::create_dir_all(&repo_dir).map_err(|e| e.to_string())?;
    let init = run_cmd_dir("git", &["init", "-q"], &repo_dir, &[])?;
    if !init.ok {
        return Err(init.combined());
    }
    let _ = run_cmd_dir("git", &["branch", "-M", "main"], &repo_dir, &[]);
    let cfg = state.cfg();
    apply_remote(&repo_dir, &cfg.git_repo)?;
    state.log("sync", "已初始化数据仓库", &repo_dir.to_string_lossy());
    Ok(format!("已初始化 {} 并绑定 {}", repo_dir.display(), cfg.git_repo))
}
