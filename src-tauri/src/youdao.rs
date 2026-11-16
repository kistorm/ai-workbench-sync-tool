//! 有道云笔记拉取（仅拉取，不推送）
//! 复用开源项目 DeppWang/youdaonote-pull 的解析与转换逻辑（XML/JSON → Markdown、图片下载）

use crate::core::{expand_tilde, run_cmd, run_cmd_dir, AppState};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::State;

const REPO_URL: &str = "https://github.com/DeppWang/youdaonote-pull.git";
const MIRROR: &str = "https://pypi.tuna.tsinghua.edu.cn/simple";

#[derive(Debug, Clone, Serialize)]
pub struct YoudaoToolInfo {
    pub project_dir: String,
    pub ready: bool,
    pub python: String,
    pub has_cookie: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct YoudaoPullResult {
    pub ok: bool,
    pub added: u32,
    pub updated: u32,
    pub skipped: u32,
    pub output: String,
}

fn project_dir(state: &AppState) -> PathBuf {
    state.data_dir.join("youdaonote-pull")
}

fn venv_python(dir: &PathBuf) -> PathBuf {
    if cfg!(windows) {
        dir.join("venv").join("Scripts").join("python.exe")
    } else {
        dir.join("venv").join("bin").join("python")
    }
}

fn venv_pip(dir: &PathBuf) -> PathBuf {
    if cfg!(windows) {
        dir.join("venv").join("Scripts").join("pip.exe")
    } else {
        dir.join("venv").join("bin").join("pip")
    }
}

fn cookie_path(state: &AppState) -> PathBuf {
    state.data_dir.join("youdao-cookies.json")
}

/// 解析用户粘贴的 Cookie（支持 `k=v; k=v` 串或 JSON）
pub fn parse_cookies(input: &str) -> Result<(String, String, String), String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Cookie 不能为空".into());
    }

    // JSON 形式：{"YNOTE_CSTK": "...", "YNOTE_LOGIN": "...", "YNOTE_SESS": "..."}
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let get = |k: &str| v.get(k).and_then(|x| x.as_str()).map(|s| s.to_string());
        if let (Some(a), Some(b), Some(c)) =
            (get("YNOTE_CSTK"), get("YNOTE_LOGIN"), get("YNOTE_SESS"))
        {
            return Ok((a, b, c));
        }
    }

    // Cookie 头形式：a=b; c=d; ...
    let mut cstk = None;
    let mut login = None;
    let mut sess = None;
    for part in trimmed.split(';') {
        if let Some((k, val)) = part.split_once('=') {
            let val = val.trim().trim_matches('"');
            match k.trim() {
                "YNOTE_CSTK" => cstk = Some(val.to_string()),
                "YNOTE_LOGIN" => login = Some(val.to_string()),
                "YNOTE_SESS" => sess = Some(val.to_string()),
                _ => {}
            }
        }
    }
    match (cstk, login, sess) {
        (Some(a), Some(b), Some(c)) if !a.is_empty() && !c.is_empty() => Ok((a, b, c)),
        _ => Err(
            "Cookie 解析失败：需要包含 YNOTE_CSTK、YNOTE_LOGIN、YNOTE_SESS 三个值。\n\
             可从浏览器 F12 → Network → 任意 note.youdao.com 请求 → 复制 Cookie 请求头。"
                .into(),
        ),
    }
}

/// 转义为 JSON 字符串（含引号）
fn jstr(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

fn write_secret(path: &PathBuf, content: &str) -> Result<(), String> {
    fs::write(path, content).map_err(|e| format!("写入 {} 失败: {}", path.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// 准备项目：clone → venv → 依赖（首次约需 1 分钟）
fn ensure_project(state: &AppState) -> Result<PathBuf, String> {
    let dir = project_dir(state);

    if !dir.join("pull.py").exists() {
        if let Some(parent) = dir.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::remove_dir_all(&dir);
        let r = run_cmd(
            "git",
            &["clone", "--depth", "1", "-q", REPO_URL, &dir.to_string_lossy()],
            &[],
        )?;
        if !r.ok {
            return Err(format!("克隆 youdaonote-pull 失败: {}", r.combined()));
        }
    }

    let py = venv_python(&dir);
    if !py.exists() {
        let r = run_cmd("python3", &["-m", "venv", &dir.join("venv").to_string_lossy()], &[])?;
        if !r.ok {
            return Err(format!(
                "创建 Python 虚拟环境失败（需要 python3-venv）: {}",
                r.combined()
            ));
        }
    }

    // 依赖检查
    let check = run_cmd(
        &py.to_string_lossy(),
        &["-c", "import requests, markdownify, brotli"],
        &[],
    )?;
    if !check.ok {
        let pip = venv_pip(&dir);
        let r = run_cmd(
            &pip.to_string_lossy(),
            &[
                "install", "-q", "--disable-pip-version-check", "-i", MIRROR, "requests",
                "markdownify", "Brotli", "win32-setctime",
            ],
            &[],
        )?;
        if !r.ok {
            return Err(format!("安装 Python 依赖失败: {}", r.combined()));
        }
    }

    Ok(dir)
}

#[tauri::command]
pub fn youdao_info(state: State<AppState>) -> YoudaoToolInfo {
    let dir = project_dir(&state);
    let py = venv_python(&dir);
    let ready = dir.join("pull.py").exists() && py.exists();
    YoudaoToolInfo {
        project_dir: dir.to_string_lossy().to_string(),
        ready,
        python: py.to_string_lossy().to_string(),
        has_cookie: cookie_path(&state).exists(),
    }
}

/// 从有道云拉取笔记到本地笔记目录（仅拉取）
#[tauri::command]
pub async fn youdao_pull(
    state: State<'_, AppState>,
    cookies: String,
    ydnote_dir: Option<String>,
) -> Result<YoudaoPullResult, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (cstk, login, sess) = parse_cookies(&cookies)?;
        let cfg = st.cfg();
        let notes_dir = expand_tilde(&cfg.notes_dir);
        fs::create_dir_all(&notes_dir).map_err(|e| format!("创建笔记目录失败: {}", e))?;

        st.set_running(crate::core::RunningAction {
            title: "从有道云拉取".into(),
            subtitle: "准备环境 → 下载笔记 → 转换 Markdown".into(),
            progress: 20,
            eta: "首次使用需先安装依赖，约 1 分钟".into(),
            active: true,
        });

        let dir = match ensure_project(&st) {
            Ok(d) => d,
            Err(e) => {
                st.clear_running();
                st.log("error", "有道云拉取失败", &e);
                return Err(e);
            }
        };

        // 写脚本所需的 cookies.json（字符串拼接，保证格式与脚本期望一致）
        let cookie_json = format!(
            "{{\"cookies\":[[\"YNOTE_CSTK\",{},\".note.youdao.com\",\"/\"],[\"YNOTE_LOGIN\",{},\".note.youdao.com\",\"/\"],[\"YNOTE_SESS\",{},\".note.youdao.com\",\"/\"]]}}",
            jstr(&cstk),
            jstr(&login),
            jstr(&sess)
        );
        write_secret(&dir.join("cookies.json"), &cookie_json)?;
        // 本地留一份，便于下次复用
        let _ = write_secret(&cookie_path(&st), &cookie_json);

        // 写 config.json：脚本严格要求 key 顺序为
        // local_dir, ydnote_dir, smms_secret_token, is_relative_path
        // （serde_json::json! 会按字母排序，故必须手工拼接）
        let config_content = format!(
            "{{\"local_dir\":{},\"ydnote_dir\":{},\"smms_secret_token\":\"\",\"is_relative_path\":true}}",
            jstr(notes_dir.to_string_lossy().as_ref()),
            jstr(&ydnote_dir.unwrap_or_default())
        );
        fs::write(dir.join("config.json"), config_content)
            .map_err(|e| format!("写入 config.json 失败: {}", e))?;

        st.set_running(crate::core::RunningAction {
            title: "从有道云拉取".into(),
            subtitle: "正在下载并转换笔记…".into(),
            progress: 60,
            eta: "视笔记数量而定".into(),
            active: true,
        });

        let py = venv_python(&dir);
        let res = run_cmd_dir(&py.to_string_lossy(), &["pull.py"], &dir, &[]);
        st.clear_running();

        match res {
            Ok(r) => {
                let output = r.combined();
                // 解析脚本输出里的统计（"新增 x"、"更新 y"、"跳过 z"）
                let mut added = 0u32;
                let mut updated = 0u32;
                let mut skipped = 0u32;
                for line in output.lines() {
                    if line.contains("新增「") {
                        added += 1;
                    } else if line.contains("更新「") {
                        updated += 1;
                    } else if line.contains("跳过") {
                        skipped += 1;
                    }
                }
                // 脚本首次运行可能因图片下载异常返回非 0，但笔记已写入。
                // 以输出中的「运行完成」或退出码为准确认成功。
                let finished = r.ok || output.contains("运行完成");
                // Cookie 失效的典型表现
                if !finished && (output.contains("fileEntry") || output.contains("KeyError")) {
                    let msg = "Cookie 无效或已过期。请重新登录 note.youdao.com 并复制新的 Cookie（需包含 YNOTE_CSTK / YNOTE_LOGIN / YNOTE_SESS）。";
                    st.log("error", "有道云拉取失败", msg);
                    return Err(msg.to_string());
                }
                let ok = finished;
                if ok {
                    st.log(
                        "note",
                        "有道云拉取完成",
                        &format!("新增 {} · 更新 {} · 跳过 {}", added, updated, skipped),
                    );
                } else {
                    let tail: String = output
                        .lines()
                        .rev()
                        .take(12)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join("\n");
                    st.log("error", "有道云拉取未能正常结束", &tail);
                    return Err(format!(
                        "有道云拉取未能正常结束（可能部分笔记已下载）：\n{}",
                        tail
                    ));
                }
                Ok(YoudaoPullResult {
                    ok,
                    added,
                    updated,
                    skipped,
                    output,
                })
            }
            Err(e) => {
                st.log("error", "有道云拉取失败", &e);
                Err(e)
            }
        }
    })
    .await
    .map_err(|e| format!("任务失败: {}", e))?
}
