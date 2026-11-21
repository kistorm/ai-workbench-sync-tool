use crate::core::{expand_tilde, home, run_cmd, run_cmd_dir, AppState};
use rand::RngCore;
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
/// - 指定了密钥：只用该密钥（IdentitiesOnly）
/// - 未指定密钥：交给系统 SSH 配置 / ssh-agent（首次引导时还没有保存密钥）
fn ssh_command(key: &str) -> String {
    // -F /dev/null：不读用户 ~/.ssh/config。
    // 用户的 config 可能带 assh ProxyCommand / 过期条目，会让应用的网络操作
    // 超时或失败（例如清理过 assh.yml 后 ProxyCommand 悬空）。应用所需的
    // 主机覆盖（HostName/Port）由 git_ssh_env 显式给出，完全自包含。
    let common = "-F /dev/null -o ConnectTimeout=15 -o StrictHostKeyChecking=accept-new";
    let key = key.trim();
    if key.is_empty() {
        format!("ssh {}", common)
    } else {
        // Git 在 Windows 上通过 sh 解析 GIT_SSH_COMMAND：反斜杠会被当转义符吃掉，
        // 路径含空格也会被拆开。统一转正斜杠并加引号，macOS/Linux 加引号同样安全。
        let p = expand_tilde(key).to_string_lossy().replace('\\', "/");
        format!("ssh -F /dev/null -i \"{}\" -o IdentitiesOnly=yes {}", p, common)
    }
}

/// 从 git@host:owner/repo.git 里提取主机名
fn repo_host(url: &str) -> String {
    url.split('@')
        .nth(1)
        .and_then(|s| s.split(':').next())
        .unwrap_or("")
        .to_string()
}

/// github.com 强制覆盖为 ssh.github.com:443 —— 不依赖 ~/.ssh/config 或 assh.yml
/// 里的映射条目（用户删除 github.com 主机后同步通道依然可用）。
fn apply_github_override(mut val: String, repo_host: &str) -> String {
    if repo_host == "github.com" {
        val.push_str(" -o HostName=ssh.github.com -o Port=443");
    }
    val
}

/// 供引导等网络 git 操作统一使用：按配置的密钥构造 GIT_SSH_COMMAND 环境变量。
pub(crate) fn git_ssh_env(st: &AppState) -> Vec<(&'static str, String)> {
    let cfg = st.cfg();
    let val = apply_github_override(ssh_command(&cfg.ssh_key), &repo_host(&cfg.git_repo));
    vec![("GIT_SSH_COMMAND", val)]
}

/// assh.yml 缺失时的最小引导配置：只包含 Git 同步所需的传输通道，不含用户主机数据。
/// 场景：全新机器首次恢复，或本地 assh.yml 被清空——没有它，
/// ~/.ssh/config（assh 生成）里的 ProxyCommand 会因 assh 读不到 yml 而失败，
/// 恢复模式自身就无法联网拉取（鸡生蛋问题）。
/// 恢复完成后，云端解密回来的完整 assh.yml 会覆盖这份引导文件。
pub(crate) fn ensure_bootstrap_assh_yml(st: &AppState) -> Result<(), String> {
    let dir = crate::ssh::ssh_dir_of(st);
    let yml = dir.join("assh.yml");
    if yml.exists() {
        return Ok(());
    }
    let cfg = st.cfg();
    // 从 git 仓库地址里取主机名（已通过 validate_git_repo_url 校验过格式）
    let host = cfg
        .git_repo
        .split('@')
        .nth(1)
        .and_then(|s| s.split(':').next())
        .unwrap_or("")
        .to_string();
    let content = if host == "github.com" {
        // GitHub:SSH 走 ssh.github.com:443，规避 22 端口被干扰的网络环境
        let idfile = cfg.ssh_key.trim();
        let id = if idfile.is_empty() {
            String::new()
        } else {
            format!(
                "    identityfile: {}\n    identitiesonly: yes\n",
                idfile
            )
        };
        format!(
            "# 由 NEXUS Workbench 引导生成：仅含 git 同步传输通道，不含用户主机。\nhosts:\n  github.com:\n    hostname: ssh.github.com\n    user: git\n    port: 443\n{}",
            id
        )
    } else {
        "# 由 NEXUS Workbench 引导生成：仅含 git 同步传输通道，不含用户主机。\nhosts: {}\n"
            .to_string()
    };
    fs::create_dir_all(&dir).map_err(|e| format!("创建 SSH 目录失败: {}", e))?;
    fs::write(&yml, content).map_err(|e| format!("写入引导 assh.yml 失败: {}", e))
}

/// 严格校验 Git 仓库地址：只接受 git@主机:用户/仓库.git 的 SSH（git）格式。
/// 拒绝 https://、http://、ssh://、git://、file:// 以及本地路径等一切其他格式。
pub fn validate_git_repo_url(repo: &str) -> Result<(), String> {
    let repo = repo.trim();
    if repo.is_empty() {
        return Err("Git 仓库地址不能为空".into());
    }
    let lower = repo.to_lowercase();
    for scheme in ["https://", "http://", "ssh://", "git://", "file://"] {
        if lower.starts_with(scheme) {
            return Err(format!(
                "不支持 {} 格式：请使用 git@ 开头的 SSH 格式，例如 git@github.com:you/your-repo.git",
                scheme.trim_end_matches("//")
            ));
        }
    }
    if !lower.starts_with("git@") {
        return Err("地址必须以 git@ 开头，例如 git@github.com:you/your-repo.git".into());
    }
    let rest = &repo[4..];
    let Some(ci) = rest.find(':') else {
        return Err("地址缺少冒号，应为 git@github.com:you/your-repo.git".into());
    };
    let host = &rest[..ci];
    let path = &rest[ci + 1..];
    if host.is_empty() || !host.contains('.') || host.contains('/') {
        return Err("主机名无效，应为 git@github.com:you/your-repo.git".into());
    }
    if path.contains(':') || path.contains(char::is_whitespace) {
        return Err("仓库路径中不能包含冒号或空白字符".into());
    }
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() < 2 {
        return Err("路径应为 用户名/仓库.git 两段，例如 git@github.com:you/your-repo.git".into());
    }
    if !lower.ends_with(".git") {
        return Err("仓库路径必须以 .git 结尾，例如 git@github.com:you/your-repo.git".into());
    }
    Ok(())
}

/// 诊断 git 远程连通性（核心逻辑，可测试）
pub fn diagnose_git_impl(repo: &str, key: &str) -> Result<Diagnosis, String> {
    if repo.trim().is_empty() {
        return Err("未配置 Git 仓库地址".into());
    }
    validate_git_repo_url(repo)?;
    // 检测的仓库同样应用 github 443 覆盖（否则 -F /dev/null 后直连 22 端口被墙）
    let env = [(
        "GIT_SSH_COMMAND",
        apply_github_override(ssh_command(key), &repo_host(repo)),
    )];
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
    // assh.yml 缺失时生成引导配置，避免 ProxyCommand 断网导致检测误报
    let _ = ensure_bootstrap_assh_yml(&state);
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
pub(crate) fn current_branch(dir: &Path) -> String {
    run_cmd_dir("git", &["branch", "--show-current"], dir, &[])
        .ok()
        .map(|r| r.stdout.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "main".into())
}

/// 拉取合并：显式指定 origin 分支。
/// 恢复模式 `reset --hard` 后本地分支可能没有 upstream 跟踪信息，
/// 不带参数的 `git pull` 会报「当前分支没有跟踪信息」。
pub(crate) fn git_pull_merge(
    dir: &Path,
    branch: &str,
    envs: &[(&str, String)],
) -> Result<crate::core::CmdResult, String> {
    run_cmd_dir(
        "git",
        &[
            "pull",
            "-q",
            "--rebase",
            "--autostash",
            "-X",
            "ours",
            "origin",
            branch,
        ],
        dir,
        envs,
    )
}

/// 远端是否已有该分支（空仓库首次推送时没有）
pub(crate) fn remote_has_branch(dir: &Path, branch: &str) -> bool {
    run_cmd_dir("git", &["ls-remote", "--heads", "origin", branch], dir, &[])
        .map(|r| r.ok && !r.stdout.trim().is_empty())
        .unwrap_or(false)
}

/// 推送（首次自动设置 upstream，兼容空仓库）
fn push_branch(dir: &Path, branch: &str, envs: &[(&str, String)]) -> CmdResultResult {
    // 网络抖动时重试 5 次（3/5/8/10 秒递增），避免自动同步只提交不推送
    let waits = [3u64, 5, 8, 10];
    let mut last: CmdResultResult = Err("未执行".into());
    for attempt in 1..=5usize {
        let r = run_cmd_dir("git", &["push", "-q", "-u", "origin", branch], dir, envs);
        match &r {
            Ok(res) if res.ok => return r,
            _ => last = r,
        }
        if attempt < 5 {
            std::thread::sleep(std::time::Duration::from_secs(waits[attempt - 1]));
        }
    }
    last
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
        let ssh_env = git_ssh_env(&st);
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
                    &crate::core::git_identity_env(&target),
                )?;
                log_buf.push_str(&commit.combined());
                let branch = current_branch(&target);
                let pull = git_pull_merge(&target, &branch, &ssh_env)?;
                log_buf.push_str(&pull.combined());
                if !pull.ok {
                    let _ = run_cmd_dir("git", &["rebase", "--abort"], &target, &[]);
                    return Err(format!("拉取合并冲突，已中止。\n{}", log_buf));
                }
                let push = run_cmd_dir("git", &["push", "-q", "-u", "origin", &branch], &target, &ssh_env)?;
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
                let pull = git_pull_merge(&target, &branch, &ssh_env)?;
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

// ==================== 全量加密同步 ====================
// 加密模式下：assh.yml / notes / ai-sessions 逐文件加密到 encrypted/**/*.enc，
// 明文不入库；拉取时解密回原位置。SSH 密码（secrets.json）不参与同步。

const ENC_DIR: &str = "encrypted";
const SALT_FILE: &str = ".enc_salt";

/// 生成/读取仓库固定 salt（salt 明文无害，用于批量派生一次密钥）
fn ensure_salt(repo: &Path) -> Result<Vec<u8>, String> {
    let path = repo.join(SALT_FILE);
    if path.exists() {
        return fs::read(&path).map_err(|e| format!("读取 salt 失败: {}", e));
    }
    let mut salt = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    fs::write(&path, salt).map_err(|e| format!("写入 salt 失败: {}", e))?;
    Ok(salt.to_vec())
}

/// 递归收集明文文件（跳过 .git / encrypted / 加密产物）
fn collect_plain(
    dir: &Path,
    base: &Path,
    out: &mut Vec<(std::path::PathBuf, std::path::PathBuf)>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name == ".git" || name == ENC_DIR || name.starts_with(".enc") || name == ".migrated" {
            continue;
        }
        if p.is_dir() {
            collect_plain(&p, base, out);
        } else if p.is_file() {
            if let Ok(rel) = p.strip_prefix(base) {
                out.push((p.clone(), rel.to_path_buf()));
            }
        }
    }
}

/// 递归收集密文文件
fn collect_enc(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_enc(&p, out);
        } else if p.extension().and_then(|x| x.to_str()) == Some("enc") {
            out.push(p);
        }
    }
}

/// 加密模式的 .gitignore：排除明文，只提交 encrypted/ 与 salt
/// 未启用加密时：明文同步模式的 gitignore（assh.yml 入库，密码库仍排除）
pub(crate) fn setup_gitignore_plain(repo: &Path) -> Result<(), String> {
    setup_gitignore(repo, false)
}

fn setup_gitignore(repo: &Path, encrypted: bool) -> Result<(), String> {
    let path = repo.join(".gitignore");
    let content = if encrypted {
        "# 加密同步：明文不入库（内容见 encrypted/）\n/assh.yml\n/notes/\n/ai-sessions/\n/.migrated\n/secrets.json\n"
    } else {
        "# 明文同步\n*.sqlite-wal\n*.sqlite-shm\n/secrets.json\n"
    };
    fs::write(&path, content).map_err(|e| format!("写入 .gitignore 失败: {}", e))
}

/// 推送前：把全部明文逐文件加密到 encrypted/
pub(crate) fn encrypt_all(st: &AppState, repo: &Path, passphrase: &str) -> Result<usize, String> {
    encrypt_all_with_progress(st, repo, passphrase, |_, _| {})
}

/// SSH 单独推送：只加密 assh.yml + 主机密码库（不动笔记/AI 会话）
pub(crate) fn encrypt_ssh_only(
    st: &AppState,
    repo: &Path,
    passphrase: &str,
) -> Result<usize, String> {
    // mac/Windows：先把 ~/.ssh/assh.yml 同步进仓库
    let _ = sync_local_ssh_yml_to_repo(st);
    setup_gitignore(repo, true)?;
    let _ = run_cmd_dir(
        "git",
        &[
            "rm",
            "--cached",
            "-r",
            "--quiet",
            "--ignore-unmatch",
            "assh.yml",
            "secrets.json",
        ],
        repo,
        &[],
    );
    let salt = ensure_salt(repo)?;
    let key = crate::crypto::derive_batch_key(passphrase, &salt);
    let enc_root = repo.join(ENC_DIR);

    let mut items: Vec<(std::path::PathBuf, std::path::PathBuf)> = vec![];
    let yml = repo.join("assh.yml");
    if yml.is_file() {
        items.push((yml, std::path::PathBuf::from("assh.yml")));
    }
    let secrets = st.data_dir.join("secrets.json");
    if secrets.is_file() {
        items.push((secrets, std::path::PathBuf::from("secrets.json")));
    }

    let mut count = 0usize;
    for (src, rel) in items {
        let dst = enc_root.join(format!("{}.enc", rel.display()));
        if let Some(parent) = dst.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let Ok(plain) = fs::read(&src) else {
            continue;
        };
        if let Ok(blob) = crate::crypto::encrypt_with_key(&key, &plain) {
            if fs::write(&dst, blob).is_ok() {
                count += 1;
            }
        }
    }
    st.log(
        "sync",
        "SSH 配置已加密",
        &format!("{} 个文件 → encrypted/", count),
    );
    Ok(count)
}

/// 判断两个路径是否指向同一个文件（含符号链接解析）。
/// Linux 上 ~/.ssh/assh.yml 常是仓库文件的软链接，此时应跳过同步避免自我覆盖。
fn same_path(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// 是否为引导生成的占位 assh.yml（仅含 git 传输通道）。
/// 用于修复旧版本在 mac/Windows 上恢复后本地仍是引导文件的问题。
pub(crate) fn is_bootstrap_yml(p: &Path) -> bool {
    fs::read_to_string(p)
        .map(|s| s.contains("由 NEXUS Workbench 引导生成"))
        .unwrap_or(false)
}

/// 是否为符号链接（包括目标已删除的悬空链接）。
/// 悬空软链写入会穿透到目标文件，必须跳过复制，否则可能自我截断。
fn is_symlink(p: &Path) -> bool {
    fs::symlink_metadata(p)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// 从仓库的 encrypted/secrets.json.enc 解密密码库并写回 app 数据目录（权限 600）。
/// 返回恢复的条目数。
pub(crate) fn restore_secrets_from(
    repo: &Path,
    data_dir: &Path,
    password: &str,
) -> Result<usize, String> {
    if password.is_empty() {
        return Err("未设置主密码，无法解密密码库".into());
    }
    let enc = repo.join(ENC_DIR).join("secrets.json.enc");
    if !enc.is_file() {
        return Err("云端没有密码库（encrypted/secrets.json.enc）".into());
    }
    let salt = fs::read(repo.join(SALT_FILE))
        .map_err(|_| "仓库缺少 .enc_salt（不是加密同步仓库）".to_string())?;
    let key = crate::crypto::derive_batch_key(password, &salt);
    let blob = fs::read(&enc).map_err(|e| format!("读取密码库密文失败: {}", e))?;
    let plain = crate::crypto::decrypt_with_key(&key, &blob)
        .map_err(|_| "密码库解密失败：主密码不正确".to_string())?;
    let count = serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(&plain)
        .map(|m| m.len())
        .unwrap_or(0);
    let path = data_dir.join("secrets.json");
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&path, &plain).map_err(|e| format!("写入密码库失败: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(count)
}

/// 用当前配置从默认仓库恢复密码库
pub(crate) fn restore_secrets_from_repo(st: &AppState) -> Result<usize, String> {
    let cfg = st.cfg();
    let n = restore_secrets_from(&config_repo_dir(), &st.data_dir, &cfg.encrypt_password)?;
    st.log("sync", "已恢复主机密码库", &format!("{} 条", n));
    Ok(n)
}

/// 拉取/恢复后：把仓库里的 assh.yml 同步到 ~/.ssh/assh.yml。
/// macOS / Windows 上这两个文件是独立的（没有软链接），不同步会导致
/// 恢复完成后 SSH 页读取 ~/.ssh/assh.yml 仍是空/引导配置，看不到云端主机。
pub(crate) fn sync_repo_ssh_yml_to_local(st: &AppState) -> Result<(), String> {
    let repo_yml = config_repo_dir().join("assh.yml");
    if !repo_yml.is_file() {
        return Ok(());
    }
    let ssh_yml = crate::ssh::ssh_dir_of(st).join("assh.yml");
    if is_symlink(&ssh_yml) || same_path(&repo_yml, &ssh_yml) {
        return Ok(());
    }
    if let Some(parent) = ssh_yml.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::copy(&repo_yml, &ssh_yml)
        .map_err(|e| format!("同步 assh.yml 到 ~/.ssh 失败: {}", e))?;
    Ok(())
}

/// 推送前：把 ~/.ssh/assh.yml 同步回仓库。
/// mac/Windows 上用户编辑的是 ~/.ssh/assh.yml，不同步会导致推送的还是旧仓库文件。
pub(crate) fn sync_local_ssh_yml_to_repo(st: &AppState) -> Result<(), String> {
    let ssh_yml = crate::ssh::ssh_dir_of(st).join("assh.yml");
    if !ssh_yml.is_file() {
        return Ok(());
    }
    let repo_dir = config_repo_dir();
    if !repo_dir.exists() {
        return Ok(());
    }
    let repo_yml = repo_dir.join("assh.yml");
    if is_symlink(&ssh_yml) || same_path(&ssh_yml, &repo_yml) {
        return Ok(());
    }
    fs::copy(&ssh_yml, &repo_yml)
        .map_err(|e| format!("同步 assh.yml 到仓库失败: {}", e))?;
    Ok(())
}

/// 同 encrypt_all，但每处理一个文件回调 (已完成, 总数)，用于界面进度条
pub(crate) fn encrypt_all_with_progress(
    st: &AppState,
    repo: &Path,
    passphrase: &str,
    mut on_progress: impl FnMut(usize, usize),
) -> Result<usize, String> {
    // mac/Windows：先把 ~/.ssh/assh.yml 同步进仓库，保证推送的是最新主机配置
    let _ = sync_local_ssh_yml_to_repo(st);
    setup_gitignore(repo, true)?;
    // 把已跟踪的明文移出索引，避免继续提交明文
    let _ = run_cmd_dir(
        "git",
        &[
            "rm",
            "--cached",
            "-r",
            "--quiet",
            "--ignore-unmatch",
            "assh.yml",
            "notes",
            "ai-sessions",
        ],
        repo,
        &[],
    );

    let salt = ensure_salt(repo)?;
    let key = crate::crypto::derive_batch_key(passphrase, &salt);
    let enc_root = repo.join(ENC_DIR);

    let mut items: Vec<(std::path::PathBuf, std::path::PathBuf)> = vec![];
    for name in ["assh.yml", "notes", "ai-sessions"] {
        let p = repo.join(name);
        if p.is_file() {
            items.push((p, std::path::PathBuf::from(name)));
        } else if p.is_dir() {
            collect_plain(&p, &repo, &mut items);
        }
    }
    // 主机密码库：随仓库加密同步（恢复后可直接查看密码）。
    // 源文件在 app 数据目录（不进仓库），密文落到 encrypted/secrets.json.enc
    let secrets_src = st.data_dir.join("secrets.json");
    if secrets_src.is_file() {
        items.push((secrets_src, std::path::PathBuf::from("secrets.json")));
    }

    let total = items.len();
    on_progress(0, total);
    let mut count = 0usize;
    let mut done = 0usize;
    for (src, rel) in items {
        let dst = enc_root.join(format!("{}.enc", rel.display()));
        if let Some(parent) = dst.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(plain) = fs::read(&src) {
            if let Ok(blob) = crate::crypto::encrypt_with_key(&key, &plain) {
                if fs::write(&dst, blob).is_ok() {
                    count += 1;
                }
            }
        }
        done += 1;
        on_progress(done, total);
    }
    st.log(
        "sync",
        "已加密同步内容",
        &format!("{} 个文件 → encrypted/", count),
    );
    Ok(count)
}

/// 拉取后：把 encrypted/ 下全部密文解密回原位置
pub(crate) fn decrypt_all(st: &AppState, repo: &Path, passphrase: &str) -> Result<usize, String> {
    decrypt_all_with_progress(st, repo, passphrase, |_, _| {}).map(|(c, _)| c)
}

/// 同 decrypt_all，但每处理一个文件回调 (已完成, 总数)。
/// 返回 (成功解密数, 总数)：主密码错误时每个文件都会失败（成功 0 个），
/// 调用方据此区分「密码不对」和「真的没有内容」。
pub(crate) fn decrypt_all_with_progress(
    st: &AppState,
    repo: &Path,
    passphrase: &str,
    mut on_progress: impl FnMut(usize, usize),
) -> Result<(usize, usize), String> {
    let salt_path = repo.join(SALT_FILE);
    if !salt_path.exists() {
        return Err("仓库缺少加密 salt：该仓库不是加密模式".into());
    }
    let salt = fs::read(&salt_path).map_err(|e| format!("读取 salt 失败: {}", e))?;
    let key = crate::crypto::derive_batch_key(passphrase, &salt);
    let enc_root = repo.join(ENC_DIR);
    if !enc_root.exists() {
        return Err("仓库中没有 encrypted/ 目录".into());
    }

    let mut files = vec![];
    collect_enc(&enc_root, &mut files);
    let total = files.len();
    on_progress(0, total);
    let mut count = 0usize;
    let mut done = 0usize;
    for enc in files {
        let Ok(rel) = enc.strip_prefix(&enc_root) else {
            continue;
        };
        let rel_str = rel.to_string_lossy().trim_end_matches(".enc").to_string();
        // 密码库解密回写到 app 数据目录（mode 600），绝不落进仓库工作区
        let is_secrets = rel_str == "secrets.json";
        let target = if is_secrets {
            st.data_dir.join("secrets.json")
        } else {
            repo.join(&rel_str)
        };
        let Ok(blob) = fs::read(&enc) else {
            continue;
        };
        match crate::crypto::decrypt_with_key(&key, &blob) {
            Ok(plain) => {
                if let Some(parent) = target.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if fs::write(&target, plain).is_ok() {
                    if is_secrets {
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let _ =
                                fs::set_permissions(&target, fs::Permissions::from_mode(0o600));
                        }
                    }
                    count += 1;
                }
            }
            Err(e) => {
                st.log("error", "解密失败", &format!("{}: {}", rel_str, e));
            }
        }
        done += 1;
        on_progress(done, total);
    }
    st.log(
        "sync",
        "已解密云端内容",
        &format!("{}/{} 个文件", count, total),
    );
    Ok((count, total))
}

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
    // 加密模式：配置了主密码时，全部内容加密后入库
    let encrypted_mode = !cfg.encrypt_password.is_empty();
    if push_like {
        if encrypted_mode {
            if let Err(e) = encrypt_all(st, &repo_dir, &cfg.encrypt_password) {
                return SyncItemResult {
                    name,
                    ok: false,
                    detail: format!("加密失败: {}", e),
                };
            }
        } else {
            // 明文模式同样需要把 ~/.ssh/assh.yml 同步进仓库（mac/Windows 独立文件）
            let _ = sync_local_ssh_yml_to_repo(st);
            let _ = setup_gitignore(&repo_dir, false);
        }
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
            &crate::core::git_identity_env(&repo_dir),
        );
    }

    let branch = current_branch(&repo_dir);
    let ssh_env = git_ssh_env(st);

    // 远端有分支才拉取（空仓库首次推送时跳过，否则会报 couldn't find remote ref）
    if remote_has_branch(&repo_dir, &branch) {
        let pull = git_pull_merge(&repo_dir, &branch, &ssh_env);
        if !pull.as_ref().map(|p| p.ok).unwrap_or(false) {
            let _ = run_cmd_dir("git", &["rebase", "--abort"], &repo_dir, &[]);
            return SyncItemResult {
                name,
                ok: false,
                detail: "拉取失败或合并冲突，已中止（本地数据未改动）".into(),
            };
        }
        // 拉取后：加密模式下解密全部内容回原位置
        if encrypted_mode {
            if let Err(e) = decrypt_all(st, &repo_dir, &cfg.encrypt_password) {
                st.log("error", "解密云端内容失败", &e);
            }
        }
    }

    if !push_like {
        // 拉取会更新 assh.yml：先同步到 ~/.ssh（mac/Windows 独立文件），再重建 config
        let _ = sync_repo_ssh_yml_to_local(st);
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

    match push_branch(&repo_dir, &branch, &ssh_env) {
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
        // 强制恢复：加密模式下解密全部内容
        if !cfg.encrypt_password.is_empty() {
            let _ = decrypt_all(&st, &repo, &cfg.encrypt_password);
        }
        // 同步到 ~/.ssh（mac/Windows 上 assh.yml 是独立文件），再重建 config
        let _ = sync_repo_ssh_yml_to_local(&st);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_command_quotes_and_normalizes_key_path() {
        let s = ssh_command("~/.ssh/id_ed25519_github");
        assert!(s.contains("-F /dev/null"), "应绕过用户 ssh 配置: {}", s);
        assert!(s.contains("-i \""), "密钥路径应加引号: {}", s);
        assert!(s.contains("IdentitiesOnly=yes"));
        assert!(s.contains("ConnectTimeout=15"));
        assert!(!s.contains('\\'), "不应包含反斜杠（Windows 会被 sh 吃掉）: {}", s);
        let empty = ssh_command("");
        assert!(empty.starts_with("ssh -F /dev/null"));
    }
}
