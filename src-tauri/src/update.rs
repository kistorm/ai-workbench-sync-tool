//! 应用自更新：下载 GitHub Release 安装包并自动安装。
//!
//! - macOS：下载 `*_macos-universal.zip`，解压后由后台脚本等待应用退出，
//!   替换 /Applications 下的 app、清除隔离标记并重启
//! - Windows：下载 `*_x64-setup.exe`，以 `/S` 静默安装后重启
//! - Linux：下载 `*_amd64.AppImage`，覆盖当前 AppImage 并重启
//!   （deb 安装方式需要 sudo，下载后交给系统安装器，不自动强装）

use crate::core::{run_cmd, AppState};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::State;

const APP_REPO_SLUG: &str = "kistorm/ai-workbench-sync-tool";
const APP_REPO_URL: &str = "git@github.com:kistorm/ai-workbench-sync-tool.git";

static DOWNLOADED: AtomicU64 = AtomicU64::new(0);
static TOTAL: AtomicU64 = AtomicU64::new(0);
static PHASE: Mutex<String> = Mutex::new(String::new());

#[derive(Debug, Clone, Serialize)]
pub struct UpdateProgress {
    pub phase: String, // idle | downloading | installing
    pub downloaded: u64,
    pub total: u64,
    pub percent: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadedUpdate {
    pub version: String,
    pub asset: String,
    pub path: String,
}

#[tauri::command]
pub fn update_progress() -> UpdateProgress {
    let downloaded = DOWNLOADED.load(Ordering::Relaxed);
    let total = TOTAL.load(Ordering::Relaxed);
    let percent = if total > 0 {
        ((downloaded.min(total) * 100) / total) as u32
    } else {
        0
    };
    let phase = PHASE.lock().map(|s| s.clone()).unwrap_or_default();
    UpdateProgress {
        phase,
        downloaded,
        total,
        percent,
    }
}

fn set_phase(p: &str) {
    if let Ok(mut s) = PHASE.lock() {
        *s = p.to_string();
    }
}

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

fn latest_remote_version() -> Result<String, String> {
    let out = run_cmd("git", &["ls-remote", "--tags", APP_REPO_URL], &[])?;
    if !out.ok {
        return Err("无法获取远端版本（请检查网络）".into());
    }
    let mut best: Option<(u64, u64, u64)> = None;
    for line in out.stdout.lines() {
        let Some((_sha, tag)) = line.split_once('\t') else {
            continue;
        };
        let Some(name) = tag.strip_prefix("refs/tags/") else {
            continue;
        };
        let name = name.strip_suffix("^{}").unwrap_or(name);
        let name = name.strip_prefix('v').unwrap_or(name);
        if let Some(v) = parse_version_triple(name) {
            if best.map_or(true, |b| v > b) {
                best = Some(v);
            }
        }
    }
    best.map(|(a, b, c)| format!("{}.{}.{}", a, b, c))
        .ok_or_else(|| "远端还没有可识别的版本 tag".to_string())
}

/// 当前平台对应的 Release 资源名
fn asset_name(version: &str) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        Some(format!("nexus-workbench_{}_macos-universal.zip", version))
    }
    #[cfg(target_os = "windows")]
    {
        Some(format!("nexus-workbench_{}_x64-setup.exe", version))
    }
    #[cfg(target_os = "linux")]
    {
        Some(format!("nexus-workbench_{}_amd64.AppImage", version))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = version;
        None
    }
}

fn download_url(version: &str, asset: &str) -> String {
    format!(
        "https://github.com/{}/releases/download/v{}/{}",
        APP_REPO_SLUG, version, asset
    )
}

fn content_length(url: &str) -> u64 {
    let Ok(r) = run_cmd("curl", &["-sIL", url], &[]) else {
        return 0;
    };
    let mut total = 0u64;
    for line in r.stdout.lines() {
        let lower = line.to_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            if let Ok(n) = rest.trim().parse::<u64>() {
                total = n;
            }
        }
    }
    total
}

/// 下载最新版本安装包到临时目录
#[tauri::command]
pub async fn download_update(_state: State<'_, AppState>) -> Result<DownloadedUpdate, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let version = latest_remote_version()?;
        let asset = asset_name(&version)
            .ok_or_else(|| "当前平台不支持自动更新，请前往 Releases 手动下载".to_string())?;
        let url = download_url(&version, &asset);
        let dir = std::env::temp_dir().join("nexus-workbench-update");
        let _ = fs::create_dir_all(&dir);
        let dest: PathBuf = dir.join(&asset);
        let _ = fs::remove_file(&dest);

        DOWNLOADED.store(0, Ordering::Relaxed);
        TOTAL.store(content_length(&url), Ordering::Relaxed);
        set_phase("downloading");

        let curl_spawn = Command::new("curl")
            .args([
                "-L",
                "--fail",
                "--retry",
                "2",
                "-s",
                "-o",
                dest.to_str().unwrap_or(""),
                &url,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match curl_spawn {
            Ok(c) => c,
            Err(_) => {
                // 没有 curl（旧版 Windows）：用 PowerShell 下载
                #[cfg(target_os = "windows")]
                {
                    let ps = format!(
                        "Invoke-WebRequest -Uri '{}' -OutFile '{}' -UseBasicParsing",
                        url,
                        dest.to_string_lossy()
                    );
                    let out = Command::new("powershell")
                        .args(["-NoProfile", "-Command", &ps])
                        .output()
                        .map_err(|e| format!("启动下载失败: {}", e))?;
                    if !out.status.success() {
                        set_phase("idle");
                        return Err(format!(
                            "下载失败: {}",
                            String::from_utf8_lossy(&out.stderr)
                        ));
                    }
                    if let Ok(md) = fs::metadata(&dest) {
                        DOWNLOADED.store(md.len(), Ordering::Relaxed);
                        TOTAL.store(md.len(), Ordering::Relaxed);
                    }
                    set_phase("idle");
                    return Ok(DownloadedUpdate {
                        version,
                        asset,
                        path: dest.to_string_lossy().to_string(),
                    });
                }
                #[cfg(not(target_os = "windows"))]
                return Err("未找到 curl，无法下载更新".to_string());
            }
        };

        loop {
            if let Ok(Some(status)) = child.try_wait() {
                if !status.success() {
                    set_phase("idle");
                    return Err(format!(
                        "下载失败（HTTP/网络错误）。请确认仓库为公开（自动更新需要匿名访问 Release），或前往 Releases 手动下载：\n{}",
                        url
                    ));
                }
                break;
            }
            if let Ok(md) = fs::metadata(&dest) {
                DOWNLOADED.store(md.len(), Ordering::Relaxed);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        if let Ok(md) = fs::metadata(&dest) {
            DOWNLOADED.store(md.len(), Ordering::Relaxed);
        }
        set_phase("idle");
        Ok(DownloadedUpdate {
            version,
            asset,
            path: dest.to_string_lossy().to_string(),
        })
    })
    .await
    .map_err(|e| format!("任务失败: {}", e))?
}

#[allow(dead_code)]
fn app_exe_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "nexus-workbench"
    }
    #[cfg(target_os = "windows")]
    {
        "nexus-workbench.exe"
    }
    #[cfg(target_os = "linux")]
    {
        "nexus-workbench"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        "nexus-workbench"
    }
}

/// 应用退出后执行替换并重启（各平台不同）
fn schedule_install(path: &Path, version: &str) -> Result<String, String> {
    #[cfg(not(target_os = "macos"))]
    let _ = version;
    #[cfg(target_os = "macos")]
    {
        let stage = std::env::temp_dir().join(format!("nexus-update-{}", version));
        let _ = fs::remove_dir_all(&stage);
        fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
        let unzip = Command::new("ditto")
            .args([
                "-x",
                "-k",
                path.to_str().unwrap_or(""),
                stage.to_str().unwrap_or(""),
            ])
            .output()
            .map_err(|e| format!("解压失败: {}", e))?;
        if !unzip.status.success() {
            return Err(format!(
                "解压安装包失败: {}",
                String::from_utf8_lossy(&unzip.stderr)
            ));
        }
        let new_app = stage.join("nexus-workbench").join("nexus-workbench.app");
        if !new_app.exists() {
            return Err("安装包内未找到 nexus-workbench.app".into());
        }
        let script = stage.join("apply-update.sh");
        let body = format!(
            r#"#!/bin/bash
set -e
DIR="{stage}"
APP="$DIR/nexus-workbench/nexus-workbench.app"
TARGET="/Applications/nexus-workbench.app"
# 等旧进程退出，避免替换正在运行的应用
while pgrep -f "$TARGET/Contents/MacOS/{exe}" >/dev/null 2>&1; do sleep 0.5; done
rm -rf "$TARGET"
cp -R "$APP" "$TARGET"
xattr -cr "$TARGET" 2>/dev/null || true
open "$TARGET"
"#,
            stage = stage.display(),
            exe = app_exe_name()
        );
        fs::write(&script, body).map_err(|e| e.to_string())?;
        let _ = Command::new("chmod").arg("+x").arg(&script).status();
        Command::new("/bin/bash")
            .arg(&script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("启动更新脚本失败: {}", e))?;
        // 命令返回后由后台线程退出应用，让脚本完成替换
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(1200));
            std::process::exit(0);
        });
        Ok("安装包已就绪，应用即将退出并自动重启".into())
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // 用批处理：等应用退出 → 静默安装（currentUser，无需管理员）→ 重新启动
        let bat = std::env::temp_dir().join("nexus-workbench-update.bat");
        let body = format!(
            "@echo off\r\ntimeout /t 2 /nobreak >nul\r\nstart /wait \"\" \"{setup}\" /S\r\ntimeout /t 3 /nobreak >nul\r\nif exist \"%LOCALAPPDATA%\\nexus-workbench\\nexus-workbench.exe\" start \"\" \"%LOCALAPPDATA%\\nexus-workbench\\nexus-workbench.exe\"\r\n",
            setup = path.display()
        );
        fs::write(&bat, body).map_err(|e| format!("写入更新脚本失败: {}", e))?;
        Command::new("cmd")
            .arg("/C")
            .arg(&bat)
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("启动安装程序失败: {}", e))?;
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(1200));
            std::process::exit(0);
        });
        Ok("安装程序已启动（静默安装），应用即将退出并自动重启".into())
    }
    #[cfg(target_os = "linux")]
    {
        let appimage = std::env::var("APPIMAGE").ok();
        match appimage {
            Some(current) if !current.is_empty() => {
                let cur = PathBuf::from(&current);
                fs::copy(path, &cur)
                    .map_err(|e| format!("替换 AppImage 失败（可能无写权限）: {}", e))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(&cur, fs::Permissions::from_mode(0o755));
                }
                let _ = Command::new(&cur)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
                std::thread::spawn(|| {
                    std::thread::sleep(Duration::from_millis(800));
                    std::process::exit(0);
                });
                Ok("已替换 AppImage，应用即将重启".into())
            }
            _ => {
                // deb / 开发模式：交给系统安装器（需要用户确认管理员权限）
                let _ = Command::new("xdg-open").arg(path).spawn();
                Ok("安装包已下载并打开，请按系统提示完成安装".into())
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (path, version);
        Err("当前平台不支持自动安装，请前往 Releases 手动下载".into())
    }
}

/// 安装已下载的更新
#[tauri::command]
pub async fn install_update(path: String, version: String) -> Result<String, String> {
    let p = PathBuf::from(path);
    if !p.is_file() {
        return Err("安装包不存在，请重新下载".into());
    }
    set_phase("installing");
    let r = schedule_install(&p, &version);
    if r.is_err() {
        set_phase("idle");
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_name_matches_platform() {
        let name = asset_name("0.2.6").expect("当前平台应有资源名");
        assert!(name.contains("0.2.6"), "资源名应包含版本号: {}", name);
        #[cfg(target_os = "macos")]
        assert!(name.ends_with("macos-universal.zip"));
        #[cfg(target_os = "windows")]
        assert!(name.ends_with("x64-setup.exe"));
        #[cfg(target_os = "linux")]
        assert!(name.ends_with("amd64.AppImage"));
    }

    #[test]
    fn download_url_shape() {
        let u = download_url("0.2.6", "nexus-workbench_0.2.6_amd64.AppImage");
        assert!(u.starts_with("https://github.com/kistorm/ai-workbench-sync-tool/releases/download/v0.2.6/"));
    }

    #[test]
    fn version_parse() {
        assert_eq!(parse_version_triple("0.2.6"), Some((0, 2, 6)));
        assert_eq!(parse_version_triple("0.2"), Some((0, 2, 0)));
        assert_eq!(parse_version_triple("x.y.z"), None);
    }
}
