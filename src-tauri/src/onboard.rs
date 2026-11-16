//! 首次使用引导：恢复模式 / 全新模式
//!
//! - 恢复模式：验证连接 → 全量拉取 → 用主密码解密 → 生成 SSH 配置
//! - 全新模式：验证连接（要求空仓库）→ 用主密码加密当前数据 → 首次推送

use crate::core::{run_cmd_dir, AppState, RunningAction};
use crate::sync::{apply_remote, config_repo_dir, current_branch};
use serde::Serialize;
use std::fs;
use tauri::State;

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

        // 验证连接
        let ls = run_cmd_dir("git", &["ls-remote", "origin"], &repo_dir, &[]);
        match ls {
            Ok(r) if r.ok => {}
            Ok(r) => {
                st.clear_running();
                return Err(format!(
                    "无法连接仓库（请检查地址与 SSH 密钥）：\n{}",
                    r.combined().lines().take(3).collect::<Vec<_>>().join("\n")
                ));
            }
            Err(e) => {
                st.clear_running();
                return Err(format!("连接仓库失败: {}", e));
            }
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
                progress: 40,
                eta: String::new(),
                active: true,
            });
            let fetch = run_cmd_dir("git", &["fetch", "-q", "origin"], &repo_dir, &[])?;
            if !fetch.ok {
                st.clear_running();
                return Err(format!("拉取失败: {}", fetch.combined()));
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

            st.set_running(RunningAction {
                title: "恢复模式".into(),
                subtitle: "解密内容…".into(),
                progress: 70,
                eta: String::new(),
                active: true,
            });
            let n = match crate::sync::decrypt_all(&st, &repo_dir, &pwd) {
                Ok(n) => n,
                Err(e) => {
                    st.clear_running();
                    return Err(format!("解密失败：{}", e));
                }
            };
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
            st.clear_running();
            st.log("sync", "引导完成（恢复模式）", &format!("解密 {} 个文件", n));
            Ok(format!(
                "恢复完成：{} 个文件已解密，SSH 配置已重新生成。",
                n
            ))
        } else {
            // ---------- 全新模式 ----------
            if !remote_empty {
                st.clear_running();
                return Err(
                    "仓库里已有内容。要恢复数据请用「恢复模式」；若确定要用本机数据覆盖云端，请先清空该仓库。".into(),
                );
            }
            st.set_running(RunningAction {
                title: "全新模式".into(),
                subtitle: "加密当前数据…".into(),
                progress: 50,
                eta: "文件较多时需要一点时间".into(),
                active: true,
            });
            let n = match crate::sync::encrypt_all(&st, &repo_dir, &pwd) {
                Ok(n) => n,
                Err(e) => {
                    st.clear_running();
                    return Err(format!("加密失败：{}", e));
                }
            };
            let _ = run_cmd_dir("git", &["add", "-A"], &repo_dir, &[]);
            let _ = run_cmd_dir(
                "git",
                &[
                    "commit",
                    "-qm",
                    &format!("onboard: initial encrypted sync ({} files)", n),
                ],
                &repo_dir,
                &[],
            );

            st.set_running(RunningAction {
                title: "全新模式".into(),
                subtitle: "首次推送到仓库…".into(),
                progress: 80,
                eta: "数据量较大，可能需要几十秒".into(),
                active: true,
            });
            let push = run_cmd_dir("git", &["push", "-q", "-u", "origin", "main"], &repo_dir, &[])?;
            st.clear_running();
            if !push.ok {
                return Err(format!(
                    "推送失败：\n{}",
                    push.combined().lines().take(4).collect::<Vec<_>>().join("\n")
                ));
            }

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
