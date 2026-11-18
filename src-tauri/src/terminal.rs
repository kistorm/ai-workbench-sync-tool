//! 内嵌 SSH 终端：用 PTY 执行系统 ssh，通过 Tauri 事件把输入输出转发给前端 xterm.js
//!
//! 设计要点：
//! - 复用 ~/.ssh/config（assh 生成），别名、跳板机、密钥全部生效
//! - 主机若保存了密码，用 sshpass 自动登录；否则 ssh 会在终端里提示用户输入
//! - 每个会话一个 PTY + 读取线程，输出以事件推送（id 区分多标签）

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

pub struct TermSession {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
}

#[derive(Default)]
pub struct TerminalManager(pub Arc<Mutex<HashMap<String, TermSession>>>);

#[derive(Clone, Serialize)]
struct TermOutput {
    id: String,
    data: String,
}

#[derive(Clone, Serialize)]
struct TermExit {
    id: String,
}

/// 打开一个 SSH 终端会话
#[tauri::command]
pub fn ssh_terminal_open(
    app: AppHandle,
    state: State<crate::core::AppState>,
    mgr: State<TerminalManager>,
    alias: String,
    cols: u16,
    rows: u16,
) -> Result<String, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: rows.max(10),
            cols: cols.max(20),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("创建 PTY 失败: {}", e))?;

    // 有保存密码时由 PTY 读取线程在出现密码提示时自动输入；
    // 不再依赖 sshpass（macOS / Windows 通常没有该工具）
    let password = crate::core::get_password(&state, &alias);
    let mut cmd = CommandBuilder::new("ssh");
    // 首次连接自动接受 host key（与添加主机时的"信任主机"一致）
    cmd.arg("-o");
    cmd.arg("StrictHostKeyChecking=accept-new");
    cmd.arg(&alias);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("启动 ssh 失败（确认系统已安装 ssh）: {}", e))?;
    // slave 句柄必须释放，否则 PTY 不会收到 EOF
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("读取 PTY 失败: {}", e))?;
    let writer: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(
        pair.master
            .take_writer()
            .map_err(|e| format!("写入 PTY 失败: {}", e))?,
    ));

    let id = format!("term-{}", chrono::Local::now().timestamp_millis());
    let id_thread = id.clone();
    let app_thread = app.clone();
    let writer_thread = writer.clone();

    // 读取线程：PTY 输出 → 前端事件；首次出现密码提示时自动输入保存的密码
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        let mut pw_sent = false;
        let mut seen = String::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).to_string();
                    if let Some(pw) = password.as_deref() {
                        if !pw_sent {
                            seen.push_str(&data);
                            let low = seen.to_lowercase();
                            if low.contains("password:") || low.contains("password for") {
                                if let Ok(mut w) = writer_thread.lock() {
                                    let _ = w.write_all(format!("{}\n", pw).as_bytes());
                                    let _ = w.flush();
                                }
                                pw_sent = true;
                            }
                        }
                    }
                    let _ = app_thread.emit(
                        "ssh-terminal-output",
                        TermOutput {
                            id: id_thread.clone(),
                            data,
                        },
                    );
                }
                Err(_) => break,
            }
        }
        let _ = app_thread.emit(
            "ssh-terminal-exit",
            TermExit {
                id: id_thread.clone(),
            },
        );
    });

    mgr.0.lock().unwrap().insert(
        id.clone(),
        TermSession {
            writer,
            master: pair.master,
            child,
        },
    );

    state.log("connect", "已打开 SSH 终端", &alias);
    Ok(id)
}

/// 键盘输入 → PTY
#[tauri::command]
pub fn ssh_terminal_write(
    mgr: State<TerminalManager>,
    id: String,
    data: String,
) -> Result<(), String> {
    let map = mgr.0.lock().unwrap();
    let s = map.get(&id).ok_or("终端会话不存在")?;
    let mut w = s.writer.lock().map_err(|e| e.to_string())?;
    w.write_all(data.as_bytes())
        .map_err(|e| format!("写入失败: {}", e))?;
    w.flush().map_err(|e| e.to_string())
}

/// 窗口尺寸变化 → PTY 调整（让 vim/htop 等全屏程序正确渲染）
#[tauri::command]
pub fn ssh_terminal_resize(
    mgr: State<TerminalManager>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let map = mgr.0.lock().unwrap();
    let s = map.get(&id).ok_or("终端会话不存在")?;
    s.master
        .resize(PtySize {
            rows: rows.max(5),
            cols: cols.max(10),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())
}

/// 关闭终端会话
#[tauri::command]
pub fn ssh_terminal_close(mgr: State<TerminalManager>, id: String) -> Result<(), String> {
    let mut map = mgr.0.lock().unwrap();
    if let Some(mut s) = map.remove(&id) {
        let _ = s.child.kill();
    }
    Ok(())
}
