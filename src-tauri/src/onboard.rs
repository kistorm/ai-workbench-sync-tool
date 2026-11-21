//! 首次使用引导：恢复模式 / 全新模式
//!
//! - 恢复模式：验证连接 → 全量拉取 → 用主密码解密 → 生成 SSH 配置
//! - 全新模式：验证连接（要求空仓库）→ 用主密码加密当前数据 → 首次推送

use crate::core::{run_cmd_dir, run_cmd_dir_stream, AppState, RunningAction};
use crate::sync::{apply_remote, config_repo_dir, current_branch};
use serde::Serialize;
use std::fs;
use tauri::State;

/// 从 git 的进度行里取百分比，例如 "Writing objects: 42% (…)" → 42
fn git_pct(line: &str, label: &str) -> Option<u32> {
    let idx = line.find(label)?;
    let rest = line[idx + label.len()..].trim_start();
    let pct_idx = rest.find('%')?;
    let digits: String = rest[..pct_idx]
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// 取输出的前 n 行并去掉 assh 的用法帮助尾巴，用于错误提示
fn first_lines(s: &str, n: usize) -> String {
    let skip_prefixes = ["Usage:", "Examples:", "Argument is a host.", "  assh connect"];
    let kept: Vec<&str> = s
        .lines()
        .filter(|l| !skip_prefixes.iter().any(|p| l.starts_with(p)))
        .take(n)
        .collect();
    kept.join("\n")
}

#[derive(Debug, Clone, Serialize)]
pub struct OnboardStatus {
    pub onboarded: bool,
    pub git_repo: String,
}

#[tauri::command]
pub fn onboard_status(state: State<AppState>) -> OnboardStatus {
    let cfg = state.cfg();
    OnboardStatus {
        onboarded: cfg.onboarded,
        git_repo: cfg.git_repo,
    }
}

/// 执行首次引导
/// mode: "recover"（恢复）| "fresh"（全新）
#[tauri::command]
pub async fn onboard(
    state: State<'_, AppState>,
    mode: String,
    git_repo: String,
    password: String,
) -> Result<String, String> {
    let repo_url = git_repo.trim().to_string();
    let pwd = password.trim().to_string();
    if repo_url.is_empty() {
        return Err("请填写 Git 仓库地址".into());
    }
    crate::sync::validate_git_repo_url(&repo_url)?;
    if pwd.len() < 6 {
        return Err("主密码至少 6 位".into());
    }
    let mode = mode.to_lowercase();
    if mode != "recover" && mode != "fresh" {
        return Err(format!("未知模式: {}", mode));
    }

    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repo_dir = config_repo_dir();
        fs::create_dir_all(&repo_dir).map_err(|e| format!("创建目录失败: {}", e))?;

        // assh.yml 缺失（全新机器/被清空）时生成最小引导配置，否则 ProxyCommand
        // 里的 assh 会因读不到 yml 而失败，恢复流程自己就联不上网
        let _ = crate::sync::ensure_bootstrap_assh_yml(&st);
        // 重建 ~/.ssh/config（消除 assh 的 "configuration file is outdated" 警告，
        // 并保证 github.com → ssh.github.com:443 等映射是最新的）
        let _ = crate::ssh::build_ssh_config_impl(&st);


        // 本地仓库初始化
        if !repo_dir.join(".git").exists() {
            let init = run_cmd_dir("git", &["init", "-q"], &repo_dir, &[])?;
            if !init.ok {
                return Err(format!("初始化仓库失败: {}", init.combined()));
            }
            let _ = run_cmd_dir("git", &["branch", "-M", "main"], &repo_dir, &[]);
        }
        apply_remote(&repo_dir, &repo_url)?;

        st.set_running(RunningAction {
            title: if mode == "recover" { "恢复模式初始化".into() } else { "全新模式初始化".into() },
            subtitle: "验证仓库连接…".into(),
            progress: 20,
            eta: String::new(),
            active: true,
        });

        // 验证连接（对 GitHub 链路不稳的网络，最多重试 5 次，等待递增）
        let ls = {
            let mut last = None;
            let waits = [2u64, 4, 6, 8];
            for attempt in 1..=5usize {
                let r = run_cmd_dir("git", &["ls-remote", "origin"], &repo_dir, &[]);
                match &r {
                    Ok(res) if res.ok => {
                        last = Some(r);
                        break;
                    }
                    _ => {
                        last = Some(r);
                        if attempt < 5 {
                            std::thread::sleep(std::time::Duration::from_secs(waits[attempt - 1]));
                        }
                    }
                }
            }
            last
        };
        match ls {
            Some(Ok(r)) if r.ok => {}
            Some(Ok(r)) => {
                st.clear_running();
                return Err(format!(
                    "无法连接仓库（已重试 5 次，网络不稳定）：\n{}\n提示：可开启加速服务改善到 GitHub 的连接后重试",
                    first_lines(&r.combined(), 3)
                ));
            }
            Some(Err(e)) => {
                st.clear_running();
                return Err(format!("连接仓库失败: {}", e));
            }
            None => unreachable!(),
        }
        let remote_empty = run_cmd_dir("git", &["ls-remote", "origin"], &repo_dir, &[])
            .map(|r| r.stdout.trim().is_empty())
            .unwrap_or(true);

        if mode == "recover" {
            // ---------- 恢复模式 ----------
            if remote_empty {
                st.clear_running();
                return Err("仓库为空，没有可恢复的内容。请改用「全新模式」。".into());
            }
            st.set_running(RunningAction {
                title: "恢复模式".into(),
                subtitle: "拉取云端内容…".into(),
                progress: 15,
                eta: "正在从 Git 仓库下载".into(),
                active: true,
            });
            // 网络抖动时自动重试（最多 5 次）
            let mut fetch: Result<crate::core::CmdResult, String> = Err("未执行".into());
            let fetch_waits = [3u64, 5, 8, 10];
            for attempt in 1..=5usize {
                st.set_running(RunningAction {
                    title: "恢复模式".into(),
                    subtitle: if attempt == 1 {
                        "拉取云端内容…".into()
                    } else {
                        format!("拉取云端内容…（第 {} 次重试 / 共 5 次）", attempt)
                    },
                    progress: 15,
                    eta: "正在从 Git 仓库下载".into(),
                    active: true,
                });
                let mut fetch_pct = 15u32;
                let r = run_cmd_dir_stream(
                    "git",
                    &["fetch", "--progress", "origin"],
                    &repo_dir,
                    &[],
                    |line| {
                        if let Some(p) = git_pct(line, "Receiving objects:") {
                            let mapped = 15 + p * 40 / 100;
                            if mapped > fetch_pct {
                                fetch_pct = mapped;
                                st.set_running(RunningAction {
                                    title: "恢复模式".into(),
                                    subtitle: format!("拉取云端内容 {}%", p),
                                    progress: mapped,
                                    eta: "正在从 Git 仓库下载".into(),
                                    active: true,
                                });
                            }
                        }
                    },
                );
                let ok_now = matches!(&r, Ok(res) if res.ok);
                fetch = r;
                if ok_now {
                    break;
                }
                if attempt < 5 {
                    std::thread::sleep(std::time::Duration::from_secs(fetch_waits[attempt - 1]));
                }
            }
            let fetch = match fetch {
                Ok(r) => r,
                Err(e) => {
                    st.clear_running();
                    return Err(format!("拉取失败: {}", e));
                }
            };
            if !fetch.ok {
                st.clear_running();
                return Err(format!(
                    "拉取失败（已重试 5 次，网络不稳定）：\n{}\n提示：可开启加速服务改善到 GitHub 的连接后重试",
                    first_lines(&fetch.combined(), 6)
                ));
            }
            let branch = current_branch(&repo_dir);
            let mut target = format!("origin/{}", branch);
            let check = run_cmd_dir("git", &["rev-parse", "--verify", &target], &repo_dir, &[])?;
            if !check.ok {
                target = "origin/main".to_string();
                let check2 = run_cmd_dir("git", &["rev-parse", "--verify", &target], &repo_dir, &[])?;
                if !check2.ok {
                    st.clear_running();
                    return Err("远程没有可恢复的分支（main/master）".into());
                }
            }
            let reset = run_cmd_dir("git", &["reset", "--hard", &target], &repo_dir, &[])?;
            if !reset.ok {
                st.clear_running();
                return Err(format!("重置工作区失败: {}", reset.combined()));
            }
            // 补设 upstream：否则后续 `git pull`（笔记/AI/定时同步）会报「没有跟踪信息」
            let _ = run_cmd_dir(
                "git",
                &["branch", "--set-upstream-to", &target, &branch],
                &repo_dir,
                &[],
            );

            st.set_running(RunningAction {
                title: "恢复模式".into(),
                subtitle: "解密云端内容…".into(),
                progress: 58,
                eta: "准备解密".into(),
                active: true,
            });
            // 先拉取，后解密：encrypted/ 不存在说明是未加密的旧仓库，直接使用明文
            let (n, total) = if repo_dir.join("encrypted").exists() {
                match crate::sync::decrypt_all_with_progress(
                    &st,
                    &repo_dir,
                    &pwd,
                    |done, total| {
                        let p = if total > 0 {
                            58 + (done as u32) * 30 / total as u32
                        } else {
                            88
                        };
                        st.set_running(RunningAction {
                            title: "恢复模式".into(),
                            subtitle: format!("解密 {}/{} 个文件", done, total),
                            progress: p,
                            eta: format!("共 {} 个文件", total),
                            active: true,
                        });
                    },
                ) {
                    Ok(pair) => pair,
                    Err(e) => {
                        st.clear_running();
                        return Err(format!("解密失败：{}", e));
                    }
                }
            } else {
                st.log("sync", "仓库未启用加密同步", "直接使用拉取到的明文内容");
                (0, 0)
            };
            if total > 0 && n < total {
                st.clear_running();
                if n == 0 {
                    // 全部解密失败 = 主密码不对。云端内容已在本地，改对密码重试无需重新下载
                    return Err(
                        "解密失败：主密码不正确。\n请填写加密推送时设置的主密码，再点一次「⇣ 开始恢复」。\n云端内容已拉取到本地，修正密码后无需重新下载，几秒即可完成。"
                            .into(),
                    );
                }
                return Err(format!(
                    "解密失败：仅 {}/{} 个文件解密成功，主密码可能不正确或部分云端数据已损坏。请核对主密码后重试。",
                    n, total
                ));
            }
            st.set_running(RunningAction {
                title: "恢复模式".into(),
                subtitle: "生成 SSH 配置…".into(),
                progress: 92,
                eta: String::new(),
                active: true,
            });
            let _ = crate::ssh::build_ssh_config_impl(&st);

            // 保存配置 + 标记已引导
            {
                let mut cfg = st.config.lock().unwrap();
                cfg.git_repo = repo_url.clone();
                cfg.encrypt_password = pwd.clone();
                cfg.onboarded = true;
                let c = cfg.clone();
                drop(cfg);
                st.save_config(&c)?;
            }
            st.set_running(RunningAction {
                title: "恢复模式".into(),
                subtitle: "完成".into(),
                progress: 100,
                eta: String::new(),
                active: true,
            });
            st.clear_running();
            st.log("sync", "引导完成（恢复模式）", &format!("解密 {} 个文件", n));
            Ok(format!(
                "恢复完成：{} 个文件已解密，SSH 配置已重新生成。",
                n
            ))
        } else {
            // ---------- 全新模式 ----------
            // 注意：GitHub 仓库无法真正做到"无分支"，因此判断标准是远程树里是否有文件
            if !remote_empty {
                let fetch = run_cmd_dir("git", &["fetch", "-q", "origin"], &repo_dir, &[]);
                let fetch_ok = fetch.as_ref().map(|r| r.ok).unwrap_or(false);
                if !fetch_ok {
                    st.clear_running();
                    return Err(
                        "无法确认远程仓库为空（fetch 失败）。为避免覆盖云端已有数据，请检查网络后重试。".into(),
                    );
                }
                let has_files = {
                    let branch = current_branch(&repo_dir);
                    let mut target = format!("origin/{}", branch);
                    if !run_cmd_dir("git", &["rev-parse", "--verify", &target], &repo_dir, &[])
                        .map(|r| r.ok)
                        .unwrap_or(false)
                    {
                        target = "origin/main".to_string();
                    }
                    run_cmd_dir("git", &["ls-tree", "-r", "--name-only", &target], &repo_dir, &[])
                        .map(|r| r.ok && !r.stdout.trim().is_empty())
                        .unwrap_or(false)
                };
                if has_files {
                    st.clear_running();
                    return Err(
                        "仓库里已有文件。要恢复数据请用「恢复模式」；若确定要用本机数据覆盖云端，请先清空该仓库。".into(),
                    );
                }
            }
            // 全新模式：重建本地 git 历史（云端和本地都从头开始，避免旧提交冲突）
            let git_dir = repo_dir.join(".git");
            if git_dir.exists() {
                let _ = fs::remove_dir_all(&git_dir);
            }
            let reinit = run_cmd_dir("git", &["init", "-q"], &repo_dir, &[])?;
            if !reinit.ok {
                st.clear_running();
                return Err(format!("初始化仓库失败: {}", reinit.combined()));
            }
            let _ = run_cmd_dir("git", &["branch", "-M", "main"], &repo_dir, &[]);
            apply_remote(&repo_dir, &repo_url)?;

            st.set_running(RunningAction {
                title: "全新模式".into(),
                subtitle: "加密当前数据…".into(),
                progress: 22,
                eta: "准备加密".into(),
                active: true,
            });
            let n = match crate::sync::encrypt_all_with_progress(
                &st,
                &repo_dir,
                &pwd,
                |done, total| {
                    let p = if total > 0 {
                        22 + (done as u32) * 43 / total as u32
                    } else {
                        65
                    };
                    st.set_running(RunningAction {
                        title: "全新模式".into(),
                        subtitle: format!("加密 {}/{} 个文件", done, total),
                        progress: p,
                        eta: format!("共 {} 个文件", total),
                        active: true,
                    });
                },
            ) {
                Ok(n) => n,
                Err(e) => {
                    st.clear_running();
                    return Err(format!("加密失败：{}", e));
                }
            };
            st.set_running(RunningAction {
                title: "全新模式".into(),
                subtitle: "提交本地变更…".into(),
                progress: 68,
                eta: String::new(),
                active: true,
            });
            let add = run_cmd_dir("git", &["add", "-A"], &repo_dir, &[])?;
            if !add.ok {
                st.clear_running();
                return Err(format!("提交失败（git add）：\n{}", first_lines(&add.combined(), 6)));
            }
            let commit = run_cmd_dir(
                "git",
                &[
                    "commit",
                    "-qm",
                    &format!("onboard: initial encrypted sync ({} files)", n),
                ],
                &repo_dir,
                &crate::core::git_identity_env(&repo_dir),
            )?;
            if !commit.ok {
                st.clear_running();
                return Err(format!(
                    "提交失败（请检查 git 用户配置）：\n{}",
                    first_lines(&commit.combined(), 6)
                ));
            }

            st.set_running(RunningAction {
                title: "全新模式".into(),
                subtitle: "推送到仓库…".into(),
                progress: 72,
                eta: "数据量较大，可能需要 1–3 分钟".into(),
                active: true,
            });
            // 远程已验证为空（无引用或空树），本地重建的历史需要强制覆盖；
            // GitHub 等平台不允许真正零引用的仓库，普通推送会被 (fetch first) 拒绝。
            // 网络抖动（如 assh 通道拨号超时）时自动重试，最多 5 次。
            let mut push_final: Option<Result<crate::core::CmdResult, String>> = None;
            let push_waits = [3u64, 5, 8, 10];
            for attempt in 1..=5usize {
                st.set_running(RunningAction {
                    title: "全新模式".into(),
                    subtitle: if attempt == 1 {
                        "推送到仓库…".into()
                    } else {
                        format!("推送到仓库…（第 {} 次重试）", attempt)
                    },
                    progress: 72,
                    eta: "数据量较大，可能需要 1–3 分钟".into(),
                    active: true,
                });
                let mut push_pct = 72u32;
                let r = run_cmd_dir_stream(
                    "git",
                    &["push", "--progress", "--force", "-u", "origin", "main"],
                    &repo_dir,
                    &[],
                    |line| {
                        if let Some(p) = git_pct(line, "Writing objects:") {
                            let mapped = 72 + p * 26 / 100;
                            if mapped > push_pct {
                                push_pct = mapped;
                                st.set_running(RunningAction {
                                    title: "全新模式".into(),
                                    subtitle: format!("上传对象 {}%", p),
                                    progress: mapped,
                                    eta: "数据量较大，可能需要 1–3 分钟".into(),
                                    active: true,
                                });
                            }
                        }
                    },
                );
                let ok_now = matches!(&r, Ok(res) if res.ok);
                let failed = match &r {
                    Ok(res) if !res.ok => Some(res.combined()),
                    Err(e) => Some(e.clone()),
                    _ => None,
                };
                if ok_now {
                    push_final = Some(r);
                    break;
                }
                if attempt < 5 {
                    let _ = failed; // 记录在最后一次失败时统一报错
                    std::thread::sleep(std::time::Duration::from_secs(push_waits[attempt - 1]));
                } else {
                    push_final = Some(r);
                }
            }
            let push = match push_final {
                Some(Ok(r)) => r,
                Some(Err(e)) => {
                    st.clear_running();
                    return Err(format!("推送失败: {}", e));
                }
                None => unreachable!(),
            };
            if !push.ok {
                st.clear_running();
                return Err(format!(
                    "推送失败（已重试 5 次，网络不稳定）：\n{}\n提示：可开启加速服务改善到 GitHub 的连接后重试",
                    first_lines(&push.combined(), 6)
                ));
            }
            st.set_running(RunningAction {
                title: "全新模式".into(),
                subtitle: "完成".into(),
                progress: 100,
                eta: String::new(),
                active: true,
            });
            st.clear_running();

            // 保存配置 + 标记已引导
            {
                let mut cfg = st.config.lock().unwrap();
                cfg.git_repo = repo_url.clone();
                cfg.encrypt_password = pwd.clone();
                cfg.onboarded = true;
                let c = cfg.clone();
                drop(cfg);
                st.save_config(&c)?;
            }
            st.log("sync", "引导完成（全新模式）", &format!("加密 {} 个文件并推送", n));
            Ok(format!(
                "完成：{} 个文件已用主密码加密并推送到仓库。",
                n
            ))
        }
    })
    .await
    .map_err(|e| format!("任务失败: {}", e))?
}
