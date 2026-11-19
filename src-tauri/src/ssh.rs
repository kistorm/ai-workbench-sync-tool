use crate::core::{expand_tilde, run_cmd, run_cmd_dir, safe_join, AppState, CmdResult};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
pub struct HostInfo {
    pub alias: String,
    pub hostname: String,
    pub user: String,
    pub port: u16,
    pub inherit: Option<String>,
    pub gateways: Vec<String>,
    pub status: String,
    pub key_file: Option<String>,
    pub trust: bool,
    /// 是否支持交互式终端（Git 托管服务不分配 PTY，不能开终端）
    pub shell_capable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestResult {
    pub reachable: bool,
    pub message: String,
    pub latency_ms: u64,
}

/// 判断主机是否支持交互式终端
/// Git 托管服务（GitHub/GitLab/Bitbucket 等）的 SSH 只接受 git 命令，不分配 PTY
fn shell_capable(user: &str, hostname: &str) -> bool {
    let u = user.to_lowercase();
    let h = hostname.to_lowercase();
    if u == "git" {
        return false;
    }
    for svc in [
        "github.com",
        "gitlab.com",
        "bitbucket.org",
        "gitee.com",
        "codeberg.org",
        "git.sr.ht",
    ] {
        if h == svc || h.ends_with(&format!(".{}", svc)) {
            return false;
        }
    }
    true
}

fn ssh_dir(state: &AppState) -> PathBuf {
    expand_tilde(&state.cfg().ssh_dir)
}

pub(crate) fn ssh_dir_of(state: &AppState) -> PathBuf {
    ssh_dir(state)
}

fn assh_yml_path(state: &AppState) -> PathBuf {
    ssh_dir(state).join("assh.yml")
}

/// 从 assh.yml 读取 alias -> inherit 映射
fn read_inherits(state: &AppState) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let path = assh_yml_path(state);
    let Ok(raw) = fs::read_to_string(&path) else {
        return map;
    };
    let Ok(val) = serde_yaml::from_str::<serde_yaml::Value>(&raw) else {
        return map;
    };
    if let Some(hosts) = val.get("hosts").and_then(|v| v.as_mapping()) {
        for (k, v) in hosts {
            if let Some(alias) = k.as_str() {
                if let Some(inh) = v.get("inherit").and_then(|x| x.as_str()) {
                    map.insert(alias.to_string(), inh.to_string());
                }
            }
        }
    }
    map
}

/// 从 assh.yml 读取 template 的 gateways
fn read_gateways(state: &AppState, template: &str) -> Vec<String> {
    let path = assh_yml_path(state);
    let Ok(raw) = fs::read_to_string(&path) else {
        return vec![];
    };
    let Ok(val) = serde_yaml::from_str::<serde_yaml::Value>(&raw) else {
        return vec![];
    };
    let Some(tmpl) = val.get("templates").and_then(|t| t.get(template)) else {
        return vec![];
    };
    let Some(gw) = tmpl.get("gateways") else {
        return vec![];
    };
    if let Some(arr) = gw.as_sequence() {
        arr.iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect()
    } else if let Some(s) = gw.as_str() {
        vec![s.to_string()]
    } else {
        vec![]
    }
}

/// 从模板读取默认的 user / identities（inherit 时使用）
fn read_template_defaults(state: &AppState, template: &str) -> (Option<String>, Option<String>) {
    let path = assh_yml_path(state);
    let Ok(raw) = fs::read_to_string(&path) else {
        return (None, None);
    };
    let Ok(val) = serde_yaml::from_str::<serde_yaml::Value>(&raw) else {
        return (None, None);
    };
    let Some(tmpl) = val.get("templates").and_then(|t| t.get(template)) else {
        return (None, None);
    };
    let user = tmpl.get("user").and_then(|x| x.as_str()).map(|s| s.to_string());
    let key = tmpl
        .get("identities")
        .and_then(|x| x.as_sequence())
        .and_then(|a| a.first())
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            tmpl.get("identityfile")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        });
    (user, key)
}

/// 列出主机：优先用 assh config json（已解析 inherit/默认值），失败回退 YAML
fn load_hosts(state: &AppState) -> Result<Vec<HostInfo>, String> {
    // mac/Windows：~/.ssh/assh.yml 与仓库文件相互独立；
    // 若本地缺失（例如恢复后尚未重启），先从仓库补一份，避免主机列表为空
    let local_yml = assh_yml_path(state);
    if !local_yml.is_file() || crate::sync::is_bootstrap_yml(&local_yml) {
        let _ = crate::sync::sync_repo_ssh_yml_to_local(state);
    }
    let inherits = read_inherits(state);
    let mut hosts: Vec<HostInfo> = vec![];

    let yml = assh_yml_path(state);
    let yml_arg = yml.to_string_lossy().to_string();
    let json_res: Option<serde_json::Value> = run_cmd("assh", &["-c", &yml_arg, "config", "json"], &[])
        .ok()
        .and_then(|r| {
            if r.ok {
                serde_json::from_str(&r.stdout).ok()
            } else {
                None
            }
        });

    if let Some(v) = json_res {
        if let Some(map) = v.get("hosts").and_then(|h| h.as_object()) {
            for (alias, props) in map {
                let get = |k: &str| props.get(k).map(|x| x.clone());
                let hostname = get("HostName")
                    .and_then(|x| x.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                let inherit = inherits.get(alias).cloned();
                // 模板默认值作为回退
                let (tmpl_user, tmpl_key) = inherit
                    .as_deref()
                    .map(|t| read_template_defaults(state, t))
                    .unwrap_or((None, None));
                let user = get("User")
                    .and_then(|x| x.as_str().map(|s| s.to_string()))
                    .or(tmpl_user)
                    .unwrap_or_else(|| "root".to_string());
                let port = get("Port")
                    .and_then(|x| x.as_str().and_then(|s| s.parse::<u16>().ok()))
                    .unwrap_or(22);
                let key_file = get("IdentityFile")
                    .and_then(|x| {
                        x.as_array()
                            .and_then(|a| a.first())
                            .and_then(|s| s.as_str().map(|v| v.to_string()))
                    })
                    .or(tmpl_key);
                let gateways = inherit
                    .as_deref()
                    .map(|t| read_gateways(state, t))
                    .unwrap_or_default();
                let trust = get("StrictHostKeyChecking")
                    .and_then(|x| x.as_str().map(|v| v.to_string()))
                    .map(|v| v == "accept-new" || v == "no")
                    .unwrap_or(false);
                let capable = shell_capable(&user, &hostname);
                hosts.push(HostInfo {
                    alias: alias.clone(),
                    hostname,
                    user,
                    port,
                    inherit,
                    gateways,
                    status: "idle".into(),
                    key_file,
                    trust,
                    shell_capable: capable,
                });
            }
        }
    } else {
        // 回退：纯 YAML 解析
        let path = assh_yml_path(state);
        let raw = fs::read_to_string(&path).map_err(|e| format!("读取 {} 失败: {}", path.display(), e))?;
        let val: serde_yaml::Value =
            serde_yaml::from_str(&raw).map_err(|e| format!("解析 assh.yml 失败: {}", e))?;
        if let Some(map) = val.get("hosts").and_then(|v| v.as_mapping()) {
            for (k, v) in map {
                let alias = k.as_str().unwrap_or_default().to_string();
                let hostname = v
                    .get("hostname")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let user = v
                    .get("user")
                    .and_then(|x| x.as_str())
                    .unwrap_or("root")
                    .to_string();
                let port = v
                    .get("port")
                    .and_then(|x| x.as_u64())
                    .map(|p| p as u16)
                    .unwrap_or(22);
                let inherit = v
                    .get("inherit")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string());
                let gateways = inherit
                    .as_deref()
                    .map(|t| read_gateways(state, t))
                    .unwrap_or_default();
                let trust = v
                    .get("stricthostkeychecking")
                    .and_then(|x| x.as_str())
                    .map(|s| s == "accept-new" || s == "no")
                    .unwrap_or(false);
                let capable = shell_capable(&user, &hostname);
                hosts.push(HostInfo {
                    alias,
                    hostname,
                    user,
                    port,
                    inherit,
                    gateways,
                    status: "idle".into(),
                    key_file: None,
                    trust,
                    shell_capable: capable,
                });
            }
        }
    }

    hosts.sort_by(|a, b| a.alias.cmp(&b.alias));
    Ok(hosts)
}

/// 在 hosts: 段末尾插入新主机块（纯文本操作，保留原文件注释）
pub fn insert_host_block(content: &str, block: &str) -> Result<String, String> {
    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    if lines.is_empty() {
        lines.push("hosts:".into());
    }
    let mut in_hosts = false;
    let mut insert_at = lines.len();
    let mut found_hosts = false;
    let mut empty_map_line: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_end();
        if !in_hosts {
            if t.trim() == "hosts:" {
                in_hosts = true;
                found_hosts = true;
            } else if t.trim() == "hosts: {}" || t.trim() == "hosts:{}" {
                // 支持空 map 写法：改成 hosts: 后插入
                empty_map_line = Some(i);
                found_hosts = true;
            }
            continue;
        }
        if t.trim().is_empty() || t.trim_start().starts_with('#') {
            continue;
        }
        if !t.starts_with(' ') && !t.starts_with('\t') {
            insert_at = i;
            break;
        }
    }
    if let Some(i) = empty_map_line {
        lines[i] = "hosts:".to_string();
        insert_at = i + 1;
    } else if !found_hosts {
        lines.push("hosts:".into());
        insert_at = lines.len();
    }

    let mut new_lines: Vec<String> = vec![];
    new_lines.extend_from_slice(&lines[..insert_at]);
    if insert_at > 0
        && !new_lines
            .last()
            .map(|s| s.trim().is_empty())
            .unwrap_or(false)
    {
        new_lines.push(String::new());
    }
    for l in block.lines() {
        new_lines.push(l.to_string());
    }
    new_lines.extend_from_slice(&lines[insert_at..]);

    let mut new_content = new_lines.join("\n");
    if !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    serde_yaml::from_str::<serde_yaml::Value>(&new_content)
        .map_err(|e| format!("生成配置后 YAML 校验失败: {}", e))?;
    Ok(new_content)
}

/// 从 hosts: 段删除指定 alias 的块（纯文本操作）
pub fn remove_host_block(content: &str, alias: &str) -> Result<String, String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut in_hosts = false;
    let mut skipping = false;

    for line in lines {
        let trimmed_end = line.trim_end();
        let indent = trimmed_end.len() - trimmed_end.trim_start().len();

        if !in_hosts {
            out.push(line.to_string());
            if trimmed_end.trim() == "hosts:" {
                in_hosts = true;
            }
            continue;
        }

        if !trimmed_end.trim().is_empty()
            && !trimmed_end.trim_start().starts_with('#')
            && indent == 0
        {
            in_hosts = false;
            skipping = false;
            out.push(line.to_string());
            continue;
        }

        if skipping {
            if trimmed_end.trim().is_empty() {
                continue;
            }
            if indent > 2 {
                continue;
            }
            skipping = false;
        }

        let t = trimmed_end.trim_start();
        if indent == 2 {
            if let Some(rest) = t.strip_prefix(alias) {
                if let Some(after) = rest.strip_prefix(':') {
                    if after.trim().is_empty() || after.trim_start().starts_with('#') {
                        skipping = true;
                        continue;
                    }
                }
            }
        }

        out.push(line.to_string());
    }

    let mut new_content = out.join("\n");
    if !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    // 若 hosts 段被删空，显式写成空 map，避免 YAML 中出现 null
    if let Ok(val) = serde_yaml::from_str::<serde_yaml::Value>(&new_content) {
        if val.get("hosts").map(|h| h.is_null()).unwrap_or(false) {
            new_content = if new_content.starts_with("hosts:\n") {
                new_content.replacen("hosts:\n", "hosts: {}\n", 1)
            } else {
                new_content.replacen("\nhosts:\n", "\nhosts: {}\n", 1)
            };
        }
    }
    serde_yaml::from_str::<serde_yaml::Value>(&new_content)
        .map_err(|e| format!("删除后 YAML 校验失败: {}", e))?;
    Ok(new_content)
}

// ==================== 命令 ====================

#[tauri::command]
pub fn list_hosts(state: State<AppState>) -> Result<Vec<HostInfo>, String> {
    load_hosts(&state)
}

#[allow(dead_code)]
pub fn list_hosts_impl(state: &AppState) -> Result<Vec<HostInfo>, String> {
    load_hosts(state)
}

#[tauri::command]
pub fn read_assh_yml(state: State<AppState>) -> Result<String, String> {
    let path = assh_yml_path(&state);
    fs::read_to_string(&path).map_err(|e| format!("读取 {} 失败: {}", path.display(), e))
}

#[tauri::command]
pub fn save_assh_yml(state: State<AppState>, content: String) -> Result<(), String> {
    // 先校验 YAML 合法性
    serde_yaml::from_str::<serde_yaml::Value>(&content)
        .map_err(|e| format!("YAML 语法错误: {}", e))?;
    let path = assh_yml_path(&state);
    fs::write(&path, content).map_err(|e| format!("写入失败: {}", e))?;
    state.log("info", "assh.yml 已保存", &path.to_string_lossy());
    // mac/Windows：同步一份到仓库，保证后续推送的是最新内容
    let _ = crate::sync::sync_local_ssh_yml_to_repo(state.inner());
    Ok(())
}

/// 添加主机（核心逻辑，可测试）
pub fn add_host_impl(
    state: &AppState,
    alias: &str,
    hostname: &str,
    user: &str,
    port: u16,
    inherit: Option<&str>,
    auth_method: &str,
    key_file: Option<&str>,
    password: Option<&str>,
    trust_host: bool,
) -> Result<(), String> {
    let alias = alias.trim();
    if alias.is_empty() {
        return Err("别名不能为空".into());
    }
    if alias.chars().any(|c| c.is_whitespace() || c == ':' || c == '#') {
        return Err("别名不能包含空格、冒号或 #".into());
    }
    let path = assh_yml_path(state);
    let content = fs::read_to_string(&path).unwrap_or_else(|_| "hosts:\n".to_string());

    // 重复检查
    if let Ok(val) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
        if let Some(hosts) = val.get("hosts").and_then(|h| h.as_mapping()) {
            if hosts.get(serde_yaml::Value::String(alias.to_string())).is_some() {
                return Err(format!("主机 {} 已存在", alias));
            }
        }
    }

    let mut block = String::new();
    block.push_str(&format!("  {}:\n", alias));
    block.push_str(&format!("    hostname: {}\n", hostname.trim()));
    if !user.trim().is_empty() {
        block.push_str(&format!("    user: {}\n", user.trim()));
    }
    if port != 22 {
        block.push_str(&format!("    port: {}\n", port));
    }
    if let Some(inh) = inherit {
        if !inh.trim().is_empty() {
            block.push_str(&format!("    inherit: {}\n", inh.trim()));
        }
    }
    if auth_method == "key" {
        if let Some(k) = key_file.filter(|k| !k.trim().is_empty()) {
            block.push_str(&format!("    identityfile: {}\n", k.trim()));
            block.push_str("    identitiesonly: yes\n");
        }
    }
    if trust_host {
        block.push_str("    stricthostkeychecking: accept-new\n");
    }

    let new_content = insert_host_block(&content, &block)?;
    fs::write(&path, new_content).map_err(|e| format!("写入失败: {}", e))?;

    // 密码只存本地 secrets.json（权限 600），不写入 assh.yml
    if auth_method == "password" {
        if let Some(pw) = password.filter(|p| !p.is_empty()) {
            crate::core::save_password(state, alias, pw)?;
        }
    }
    // 立即重新生成 ~/.ssh/config，否则 ssh <别名> 找不到该主机
    let _ = build_ssh_config_impl(state);
    state.log(
        "info",
        "已添加主机",
        &format!(
            "{} → {} ({})",
            alias,
            hostname,
            match auth_method {
                "key" => "密钥认证",
                "password" => "密码认证",
                _ => "默认",
            }
        ),
    );
    Ok(())
}

/// 修改主机连接信息（保留密码不动；别名变更时会迁移密码）
pub fn update_host_impl(
    state: &AppState,
    alias: &str,
    new_alias: &str,
    hostname: &str,
    user: &str,
    port: u16,
    trust_host: bool,
) -> Result<(), String> {
    let new_alias = new_alias.trim();
    if new_alias.is_empty() {
        return Err("别名不能为空".into());
    }
    if new_alias.chars().any(|c| c.is_whitespace() || c == ':' || c == '#') {
        return Err("别名不能包含空格、冒号或 #".into());
    }
    let path = assh_yml_path(state);
    let content = fs::read_to_string(&path).map_err(|e| format!("读取失败: {}", e))?;

    // 读取旧配置（保留 inherit/密钥等未在表单中的字段）
    let old = {
        let val: serde_yaml::Value =
            serde_yaml::from_str(&content).map_err(|e| format!("解析失败: {}", e))?;
        val.get("hosts")
            .and_then(|h| h.get(alias))
            .cloned()
            .ok_or_else(|| format!("主机 {} 不存在", alias))?
    };

    // 别名变更时检查冲突
    if new_alias != alias {
        let val: serde_yaml::Value =
            serde_yaml::from_str(&content).map_err(|e| format!("解析失败: {}", e))?;
        if val.get("hosts").and_then(|h| h.get(new_alias)).is_some() {
            return Err(format!("别名 {} 已存在", new_alias));
        }
    }

    // 删除旧块
    let after_remove = remove_host_block(&content, alias)?;

    // 构造新块（沿用原块中的 inherit / identityfile 等字段）
    let mut block = String::new();
    block.push_str(&format!("  {}:\n", new_alias));
    block.push_str(&format!("    hostname: {}\n", hostname.trim()));
    if !user.trim().is_empty() {
        block.push_str(&format!("    user: {}\n", user.trim()));
    }
    if port != 22 {
        block.push_str(&format!("    port: {}\n", port));
    }
    if let Some(inherit) = old.get("inherit").and_then(|x| x.as_str()) {
        block.push_str(&format!("    inherit: {}\n", inherit));
    }
    if let Some(idf) = old.get("identityfile").and_then(|x| x.as_str()) {
        block.push_str(&format!("    identityfile: {}\n", idf));
        block.push_str("    identitiesonly: yes\n");
    }
    if trust_host {
        block.push_str("    stricthostkeychecking: accept-new\n");
    }

    let final_content = insert_host_block(&after_remove, &block)?;
    fs::write(&path, final_content).map_err(|e| format!("写入失败: {}", e))?;

    // 别名变更：迁移密码
    if new_alias != alias {
        if let Some(pw) = crate::core::get_password(state, alias) {
            let _ = crate::core::save_password(state, new_alias, &pw);
            let _ = crate::core::delete_password(state, alias);
        }
    }

    let _ = build_ssh_config_impl(state);
    state.log("info", "已修改主机配置", &format!("{} → {}@{}:{}", new_alias, user, hostname, port));
    Ok(())
}

#[tauri::command]
pub fn update_host(
    state: State<AppState>,
    alias: String,
    new_alias: String,
    hostname: String,
    user: String,
    port: u16,
    trust_host: Option<bool>,
) -> Result<(), String> {
    update_host_impl(&state, &alias, &new_alias, &hostname, &user, port, trust_host.unwrap_or(true))
}

/// 删除主机（核心逻辑，可测试）
pub fn delete_host_impl(state: &AppState, alias: &str) -> Result<(), String> {
    let path = assh_yml_path(state);
    let content = fs::read_to_string(&path).map_err(|e| format!("读取失败: {}", e))?;
    let new_content = remove_host_block(&content, alias)?;
    fs::write(&path, new_content).map_err(|e| format!("写入失败: {}", e))?;
    // 注意：故意保留本地密码。主机可能通过 git 拉取恢复，
    // 保留密码可避免重新输入；密码仅存本地、不同步，保留无泄露风险。
    let _ = build_ssh_config_impl(state);
    state.log(
        "info",
        "已删除主机",
        &format!("{}（本地密码保留，恢复主机后仍可用）", alias),
    );
    Ok(())
}

#[tauri::command]
pub fn add_host(
    state: State<AppState>,
    alias: String,
    hostname: String,
    user: String,
    port: u16,
    inherit: Option<String>,
    auth_method: Option<String>,
    key_file: Option<String>,
    password: Option<String>,
    trust_host: Option<bool>,
) -> Result<(), String> {
    add_host_impl(
        &state,
        &alias,
        &hostname,
        &user,
        port,
        inherit.as_deref(),
        auth_method.as_deref().unwrap_or("none"),
        key_file.as_deref(),
        password.as_deref(),
        trust_host.unwrap_or(true),
    )
}

/// 从 hosts: 段删除主机块
#[tauri::command]
pub fn delete_host(state: State<AppState>, alias: String) -> Result<(), String> {
    delete_host_impl(&state, &alias)
}

/// 未安装 assh 时的基础生成器：直接从 assh.yml 生成标准 ~/.ssh/config。
/// 支持 hostname/user/port/identityfile/stricthostkeychecking 与模板默认值继承；
/// 跳板机（gateways）需要 assh 的 ProxyCommand，基础模式不支持。
pub(crate) fn build_basic_ssh_config(state: &AppState, dir: &std::path::Path) -> Result<String, String> {
    let yml_path = dir.join("assh.yml");
    let raw = fs::read_to_string(&yml_path)
        .map_err(|e| format!("读取 {} 失败: {}", yml_path.display(), e))?;
    let val: serde_yaml::Value =
        serde_yaml::from_str(&raw).map_err(|e| format!("解析 assh.yml 失败: {}", e))?;

    // YAML 标量统一成字符串：支持字符串 / 整数（port）/ 布尔（trust）
    fn scalar(x: &serde_yaml::Value) -> Option<String> {
        if let Some(s) = x.as_str() {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        } else if let Some(n) = x.as_u64() {
            Some(n.to_string())
        } else if let Some(b) = x.as_bool() {
            Some(if b { "yes".into() } else { "no".into() })
        } else {
            None
        }
    }
    let get_str = |v: &serde_yaml::Value, k: &str| -> Option<String> {
        v.get(k).and_then(scalar)
    };
    let template_of = |name: &str| -> Option<&serde_yaml::Value> {
        val.get("templates")
            .and_then(|t| t.get(name))
    };

    let mut cfg = String::from(
        "# Generated by NEXUS Workbench (basic mode: assh not installed)\n# 基础模式不支持跳板机 ProxyCommand；安装 assh 可获得完整能力\n\n",
    );
    let mut count = 0usize;
    if let Some(hosts) = val.get("hosts").and_then(|h| h.as_mapping()) {
        for (k, v) in hosts {
            let alias = k.as_str().unwrap_or_default();
            if alias.is_empty() {
                continue;
            }
            let tmpl = v
                .get("inherit")
                .and_then(|x| x.as_str())
                .and_then(template_of);
            let pick = |key: &str| -> Option<String> {
                get_str(v, key)
                    .or_else(|| tmpl.and_then(|t| get_str(t, key)))
            };
            let Some(hostname) = pick("hostname") else {
                continue; // 只有模板/无 hostname 的条目跳过
            };
            cfg.push_str(&format!("Host {}\n", alias));
            cfg.push_str(&format!("  HostName {}\n", hostname));
            if let Some(user) = pick("user") {
                cfg.push_str(&format!("  User {}\n", user));
            }
            if let Some(port) = pick("port") {
                cfg.push_str(&format!("  Port {}\n", port));
            }
            if let Some(idf) = pick("identityfile") {
                cfg.push_str(&format!("  IdentityFile {}\n", idf));
                cfg.push_str("  IdentitiesOnly yes\n");
            }
            if let Some(shk) = pick("stricthostkeychecking") {
                cfg.push_str(&format!("  StrictHostKeyChecking {}\n", shk));
            }
            cfg.push_str("\n");
            count += 1;
        }
    }

    // 全局默认
    if let Some(defaults) = val.get("defaults") {
        cfg.push_str("Host *\n");
        for key in ["port", "serveraliveinterval", "serveralivecountmax"] {
            if let Some(v) = defaults.get(key).and_then(|x| x.as_u64()) {
                let ssh_key = match key {
                    "port" => "Port",
                    "serveraliveinterval" => "ServerAliveInterval",
                    "serveralivecountmax" => "ServerAliveCountMax",
                    _ => continue,
                };
                cfg.push_str(&format!("  {} {}\n", ssh_key, v));
            }
        }
        cfg.push_str("\n");
    }

    let config_path = dir.join("config");
    if let Ok(existing) = fs::read_to_string(&config_path) {
        if !existing.contains("Generated by NEXUS Workbench") && !existing.contains("generated by assh") {
            let _ = fs::write(dir.join("config.bak"), existing);
        }
    }
    fs::write(&config_path, &cfg).map_err(|e| format!("写入 config 失败: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600));
    }
    state.log(
        "sync",
        "已生成 ~/.ssh/config（基础模式）",
        &format!("{} 个主机 · {} 字节", count, cfg.len()),
    );
    Ok(format!(
        "已写入 {}（基础模式，{} 个主机；安装 assh 可支持跳板机）",
        config_path.display(),
        count
    ))
}

/// 运行 assh config build 并写入 ~/.ssh/config；assh 缺失时退回基础生成器
pub fn build_ssh_config_impl(state: &AppState) -> Result<String, String> {
    let dir = ssh_dir(state);
    let yml = dir.join("assh.yml");
    // assh 是否可用（macOS 新机器常见未安装）
    let has_assh = run_cmd("assh", &["--version"], &[]).map(|r| r.ok).unwrap_or(false);
    if !has_assh {
        return build_basic_ssh_config(state, &dir);
    }
    let yml_arg = yml.to_string_lossy().to_string();
    let out = run_cmd(
        "assh",
        &["-c", &yml_arg, "config", "build", "--no-automatic-rewrite"],
        &[],
    )?;
    if !out.ok {
        return Err(format!("assh config build 失败: {}", out.combined()));
    }
    let config_path = dir.join("config");
    if let Ok(existing) = fs::read_to_string(&config_path) {
        if !existing.contains("automatically generated by assh") {
            let _ = fs::write(dir.join("config.bak"), existing);
        }
    }
    fs::write(&config_path, &out.stdout).map_err(|e| format!("写入 config 失败: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600));
    }
    state.log(
        "sync",
        "已生成 ~/.ssh/config",
        &format!("{} bytes", out.stdout.len()),
    );
    Ok(format!(
        "已写入 {} ({} 字节)",
        config_path.display(),
        out.stdout.len()
    ))
}

#[tauri::command]
pub fn build_ssh_config(state: State<AppState>) -> Result<String, String> {
    build_ssh_config_impl(&state)
}

/// 设置或修改某主机的登录密码（仅本地保存，不进配置）
#[tauri::command]
pub fn set_host_password(
    state: State<AppState>,
    alias: String,
    password: String,
) -> Result<(), String> {
    if password.is_empty() {
        return Err("密码不能为空".into());
    }
    crate::core::save_password(&state, &alias, &password)?;
    state.log("info", "已更新主机密码", &alias);
    Ok(())
}

/// 查看已保存的主机密码（本地明文，仅用于界面展示/核对）
#[tauri::command]
pub fn reveal_host_password(state: State<AppState>, alias: String) -> Result<String, String> {
    crate::core::get_password(&state, &alias).ok_or_else(|| "该主机未保存密码".to_string())
}

/// 清除某主机的密码
#[tauri::command]
pub fn clear_host_password(state: State<AppState>, alias: String) -> Result<(), String> {
    crate::core::delete_password(&state, &alias)?;
    state.log("info", "已清除主机密码", &alias);
    Ok(())
}

/// SSH 配置单独推送到仓库（加密 assh.yml + 主机密码库，不动笔记/AI 数据）
#[tauri::command]
pub async fn push_ssh_config(state: State<'_, AppState>) -> Result<String, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repo = crate::sync::config_repo_dir();
        let cfg = st.cfg();
        if !repo.join(".git").exists() {
            return Err(format!(
                "{} 不是 git 仓库（请先在设置里初始化仓库）",
                repo.display()
            ));
        }
        let repo_str = repo.to_string_lossy().to_string();
        crate::sync::apply_remote(&repo, &cfg.git_repo)?;

        // 加密 SSH 相关内容；未设置主密码则退回明文模式（assh.yml 直接入库）
        let encrypted_mode = !cfg.encrypt_password.is_empty();
        let mut n = 0usize;
        if encrypted_mode {
            n = crate::sync::encrypt_ssh_only(&st, &repo, &cfg.encrypt_password)?;
        } else {
            crate::sync::setup_gitignore_plain(&repo)?;
        }

        // 只暂存 SSH 相关路径，不把笔记/AI 的本地变更带进这次提交
        let paths: Vec<&str> = if encrypted_mode {
            vec!["encrypted", ".enc_salt", ".gitignore"]
        } else {
            vec!["assh.yml", ".gitignore"]
        };
        for p in &paths {
            let _ = run_cmd_dir("git", &["add", "-A", "--", p], &repo, &[]);
        }
        let commit = run_cmd_dir(
            "git",
            &[
                "commit",
                "-qm",
                &format!(
                    "ssh: sync config ({} files, {})",
                    if encrypted_mode { n } else { 1 },
                    chrono::Local::now().format("%F %T")
                ),
            ],
            &repo,
            &crate::core::git_identity_env(&repo),
        );
        let commit_note = match &commit {
            Ok(r) if r.ok => "已提交".to_string(),
            Ok(r) if r.combined().contains("nothing to commit") => "无变化".to_string(),
            Ok(r) => return Err(format!("提交失败：{}", r.combined())),
            Err(e) => return Err(format!("提交失败: {}", e)),
        };

        let ssh_env = crate::sync::git_ssh_env(&st);
        let branch = run_cmd("git", &["-C", &repo_str, "branch", "--show-current"], &[])?
            .stdout
            .trim()
            .to_string();
        let branch = if branch.is_empty() { "main".to_string() } else { branch };

        // 远端有分支才拉取（首次推送跳过，否则报 couldn't find remote ref）
        if crate::sync::remote_has_branch(&repo, &branch) {
            let pull = run_cmd_dir(
                "git",
                &["pull", "-q", "--rebase", "--autostash", "-X", "ours"],
                &repo,
                &ssh_env,
            )?;
            if !pull.ok {
                let _ = run_cmd_dir("git", &["rebase", "--abort"], &repo, &[]);
                return Err(format!("拉取合并失败：{}", pull.combined()));
            }
        }

        // 推送（失败自动重试 3 次）
        let mut push_err = String::new();
        for attempt in 1..=3usize {
            let p = run_cmd_dir(
                "git",
                &["push", "-q", "-u", "origin", &branch],
                &repo,
                &ssh_env,
            );
            match p {
                Ok(r) if r.ok => {
                    st.log(
                        "sync",
                        "SSH 配置已推送到仓库",
                        &format!("{} · {}", commit_note, branch),
                    );
                    return Ok(format!(
                        "SSH 配置已推送到仓库{}（{} · {}）",
                        if encrypted_mode {
                            format!("，加密 {} 个文件", n)
                        } else {
                            String::new()
                        },
                        commit_note,
                        branch
                    ));
                }
                Ok(r) => push_err = r.combined(),
                Err(e) => push_err = e,
            }
            if attempt < 3 {
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
        }
        Err(format!("推送失败（已重试 3 次）：{}", push_err))
    })
    .await
    .map_err(|e| format!("任务失败: {}", e))?
}

/// 单独从云端强制恢复 SSH 配置（只覆盖 assh.yml，不动笔记/AI 数据）
/// 加密同步下云端没有明文 assh.yml，需要取回 encrypted/assh.yml.enc 并用主密码解密。
#[tauri::command]
pub async fn force_pull_ssh_config(state: State<'_, AppState>) -> Result<String, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repo = crate::sync::config_repo_dir();
        let cfg = st.cfg();
        if !repo.join(".git").exists() {
            return Err(format!("{} 不是 git 仓库", repo.display()));
        }
        let repo_str = repo.to_string_lossy().to_string();

        // 确保远程地址是配置里的私库
        crate::sync::apply_remote(&repo, &cfg.git_repo)?;

        // 网络参数：统一 GIT_SSH_COMMAND（github 走 443），失败自动重试 3 次
        let ssh_env = crate::sync::git_ssh_env(&st);

        // 性能优化：SSH 单独恢复只需要 assh.yml 一个文件。
        // 用 blobless 部分克隆 + 浅拉取，避免把整个数据仓库（数百 MB）都下载下来。
        let _ = run_cmd(
            "git",
            &["-C", &repo_str, "config", "remote.origin.promisor", "true"],
            &[],
        );
        let _ = run_cmd(
            "git",
            &[
                "-C",
                &repo_str,
                "config",
                "remote.origin.partialclonefilter",
                "blob:none",
            ],
            &[],
        );

        let mut fetch_err = String::new();
        let mut fetch_ok = false;
        for attempt in 1..=3usize {
            let f = run_cmd(
                "git",
                &[
                    "-C",
                    &repo_str,
                    "fetch",
                    "-q",
                    "--depth=1",
                    "--filter=blob:none",
                    "--no-tags",
                    "origin",
                ],
                &ssh_env,
            );
            match f {
                Ok(r) if r.ok => {
                    fetch_ok = true;
                    break;
                }
                Ok(r) => fetch_err = r.combined(),
                Err(e) => fetch_err = e,
            }
            if attempt < 3 {
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
        }
        if !fetch_ok {
            // 服务端不支持部分克隆时，退回普通浅拉取
            for attempt in 1..=2usize {
                let f = run_cmd(
                    "git",
                    &["-C", &repo_str, "fetch", "-q", "--depth=1", "origin"],
                    &ssh_env,
                );
                match f {
                    Ok(r) if r.ok => {
                        fetch_ok = true;
                        break;
                    }
                    Ok(r) => fetch_err = r.combined(),
                    Err(e) => fetch_err = e,
                }
                if attempt < 2 {
                    std::thread::sleep(std::time::Duration::from_secs(3));
                }
            }
        }
        if !fetch_ok {
            return Err(format!("拉取远程失败（已重试 3 次）：{}", fetch_err));
        }

        // 当前分支 → 远程分支
        let branch = run_cmd("git", &["-C", &repo_str, "branch", "--show-current"], &[])?
            .stdout
            .trim()
            .to_string();
        let branch = if branch.is_empty() { "main".to_string() } else { branch };
        let target = format!("origin/{}", branch);

        // 云端是否有明文 assh.yml（未加密的旧仓库）
        let has_plain = run_cmd(
            "git",
            &["-C", &repo_str, "cat-file", "-e", &format!("{}:assh.yml", target)],
            &[],
        )
        .map(|r| r.ok)
        .unwrap_or(false);

        let mut note = String::new();
        if has_plain {
            // 旧格式：直接恢复明文
            let restore = run_cmd(
                "git",
                &["-C", &repo_str, "restore", "--source", &target, "--worktree", "assh.yml"],
                &[],
            )?;
            if !restore.ok {
                return Err(format!("恢复 SSH 配置失败: {}", restore.combined()));
            }
        } else {
            // 加密同步：恢复密文 → 用主密码解密 → 写回 assh.yml
            let restore_enc = run_cmd(
                "git",
                &[
                    "-C",
                    &repo_str,
                    "restore",
                    "--source",
                    &target,
                    "--worktree",
                    "encrypted/assh.yml.enc",
                ],
                &[],
            )?;
            if !restore_enc.ok {
                return Err(
                    "远程没有 SSH 配置（assh.yml 与 encrypted/assh.yml.enc 均不存在）".into(),
                );
            }
            if cfg.encrypt_password.is_empty() {
                return Err(
                    "该仓库是加密同步，但设置里没有填写主密码；请先在设置里填好主密码再恢复".into(),
                );
            }
            let blob = fs::read(repo.join("encrypted/assh.yml.enc"))
                .map_err(|e| format!("读取密文失败: {}", e))?;
            let salt = fs::read(repo.join(".enc_salt"))
                .map_err(|_| "仓库缺少 .enc_salt（不是加密同步仓库）".to_string())?;
            let key = crate::crypto::derive_batch_key(&cfg.encrypt_password, &salt);
            let plain = crate::crypto::decrypt_with_key(&key, &blob).map_err(|_| {
                "解密失败：主密码不正确。请在设置里填写加密推送时使用的主密码后重试".to_string()
            })?;
            fs::write(repo.join("assh.yml"), &plain)
                .map_err(|e| format!("写入 assh.yml 失败: {}", e))?;
            note = "（已解密）".to_string();
        }

        // SSH 单独恢复也要把主机密码库一起拉下来（encrypted/secrets.json.enc）
        let mut secrets_note = String::new();
        let has_secrets_enc = run_cmd(
            "git",
            &[
                "-C",
                &repo_str,
                "cat-file",
                "-e",
                &format!("{}:encrypted/secrets.json.enc", target),
            ],
            &[],
        )
        .map(|r| r.ok)
        .unwrap_or(false);
        if has_secrets_enc {
            let restore_secrets = run_cmd(
                "git",
                &[
                    "-C",
                    &repo_str,
                    "restore",
                    "--source",
                    &target,
                    "--worktree",
                    "encrypted/secrets.json.enc",
                ],
                &[],
            )?;
            if !restore_secrets.ok {
                return Err(format!("恢复密码库失败: {}", restore_secrets.combined()));
            }
            let n = crate::sync::restore_secrets_from_repo(&st)
                .map_err(|e| format!("恢复密码库失败：{}", e))?;
            secrets_note = format!("，密码库 {} 条", n);
        }

        // 重建 ~/.ssh/config 使恢复的配置立即生效
        // mac/Windows：仓库与 ~/.ssh 的 assh.yml 是独立文件，恢复后必须同步
        let _ = crate::sync::sync_repo_ssh_yml_to_local(&st);
        let msg = build_ssh_config_impl(&st)?;
        st.log(
            "sync",
            "已从云端恢复 SSH 配置",
            &format!("来源 {} 的 assh.yml{}{}", target, note, secrets_note),
        );
        Ok(format!(
            "已从云端恢复 SSH 配置{}{}（{}）\n{}",
            note, secrets_note, target, msg
        ))
    })
    .await
    .map_err(|e| format!("任务失败: {}", e))?
}

/// 用 PTY 运行 ssh 并在出现密码提示时自动输入密码。
/// 这样 macOS / Windows 无需安装 sshpass 也能测试密码主机。
fn run_ssh_with_password_pty(args: &[String], password: &str) -> Result<(i32, String), String> {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    use std::io::{Read, Write};

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("创建 PTY 失败: {}", e))?;
    let mut cmd = CommandBuilder::new("ssh");
    for a in args {
        cmd.arg(a);
    }
    cmd.env("TERM", "dumb");
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("启动 ssh 失败（确认系统已安装 ssh）: {}", e))?;
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("读取 PTY 失败: {}", e))?;
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("写入 PTY 失败: {}", e))?;

    let mut out = String::new();
    let mut sent = false;
    let start = Instant::now();
    let mut buf = [0u8; 4096];
    loop {
        if start.elapsed() > std::time::Duration::from_secs(20) {
            let _ = child.kill();
            break;
        }
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let s = String::from_utf8_lossy(&buf[..n]).to_string();
                out.push_str(&s);
                if !sent {
                    let low = out.to_lowercase();
                    if low.contains("password:") || low.contains("password for") {
                        let _ = writer.write_all(format!("{}\n", password).as_bytes());
                        let _ = writer.flush();
                        sent = true;
                    }
                }
            }
            Err(_) => break,
        }
    }
    let status = child.wait().map_err(|e| format!("等待 ssh 结束失败: {}", e))?;
    Ok((status.exit_code() as i32, out))
}

/// 测试主机连通性（直接连接或走别名，密码用 PTY 自动输入）
#[tauri::command]
pub async fn test_host(state: State<'_, AppState>, alias: String) -> Result<TestResult, String> {
    let st = state.inner().clone();
    let alias2 = alias.clone();
    // 测试前确保 ~/.ssh/config 与 assh.yml 一致（否则别名可能解析错误）
    let _ = build_ssh_config_impl(&st);
    tauri::async_runtime::spawn_blocking(move || {
        let start = Instant::now();
        let password = crate::core::get_password(&st, &alias2);
        // 用于生成更准确的失败提示
        let host_meta = load_hosts(&st)
            .ok()
            .and_then(|hs| hs.into_iter().find(|h| h.alias == alias2));
        let has_key = host_meta
            .as_ref()
            .map(|h| h.key_file.is_some())
            .unwrap_or(false);
        let has_pw = password.is_some();
        // `true` 比 `exit` 兼容性更好（部分 SSH 服务不允许 exit）
        // 首次连接自动接受 host key（信任主机），避免测试被指纹确认卡住。
        // 常规主机直接 user@host 连接，不依赖 ~/.ssh/config / assh（macOS 可能没装 assh）；
        // 只有带跳板机（gateways）或模板继承的主机才走别名 + config。
        let use_alias = host_meta
            .as_ref()
            .map(|h| !h.gateways.is_empty() || h.inherit.is_some())
            .unwrap_or(true);
        let mut args: Vec<String> = vec![
            "-o".to_string(),
            "ConnectTimeout=8".to_string(),
            "-o".to_string(),
            "StrictHostKeyChecking=accept-new".to_string(),
        ];
        if password.is_some() {
            args.push("-o".to_string());
            args.push("PreferredAuthentications=password,keyboard-interactive".to_string());
            args.push("-o".to_string());
            args.push("NumberOfPasswordPrompts=1".to_string());
        } else {
            args.push("-o".to_string());
            args.push("BatchMode=yes".to_string());
        }
        if !use_alias {
            if let Some(h) = &host_meta {
                if let Some(k) = &h.key_file {
                    let kp = crate::core::expand_tilde(k);
                    args.push("-i".to_string());
                    args.push(kp.to_string_lossy().to_string());
                    args.push("-o".to_string());
                    args.push("IdentitiesOnly=yes".to_string());
                }
                args.push("-p".to_string());
                args.push(h.port.to_string());
                args.push(format!("{}@{}", h.user, h.hostname));
            } else {
                args.push(alias2.clone());
            }
        } else {
            args.push(alias2.clone());
        }
        args.push("true".to_string());

        let res: CmdResult = if let Some(pw) = password {
            // 不依赖 sshpass：在 PTY 里等密码提示后写入
            let (code, out) = run_ssh_with_password_pty(&args, &pw)?;
            CmdResult {
                ok: code == 0,
                code: Some(code),
                stdout: out,
                stderr: String::new(),
            }
        } else {
            let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            run_cmd("ssh", &refs, &[])?
        };
        let ms = start.elapsed().as_millis() as u64;
        let text = res.combined();
        let lower = text.to_lowercase();
        let code = res.code.unwrap_or(-1);

        // ssh 退出码语义：255 表示 ssh 自身错误（连接/认证失败）；其余为远程命令退出码
        let network_ok = code != 255;
        let reachable = network_ok
            || lower.contains("successfully authenticated")
            || lower.contains("permission denied")
            || lower.contains("invalid command");

        let message = if res.code == Some(5) {
            "密码认证失败（密码错误或服务器不允许密码登录）".to_string()
        } else if lower.contains("successfully authenticated") {
            "连接成功 · 认证通过".to_string()
        } else if network_ok {
            format!("连接成功 · 远程命令退出码 {}", code)
        } else if lower.contains("permission denied") {
            // 区分"没有凭据"和"凭据错误"，给出可操作提示
            match (has_pw, has_key) {
                (false, false) => "网络可达 · 未保存密码（请点「查看 / 修改密码」设置密码）".to_string(),
                (false, true) => "网络可达 · 密钥被拒绝（检查公钥是否已添加到服务器）".to_string(),
                (true, _) => "网络可达 · 密码被拒绝（密码可能已变更，请重新设置）".to_string(),
            }
        } else if lower.contains("timed out") || lower.contains("timeout") {
            "连接超时".to_string()
        } else if lower.contains("connection refused") {
            "端口拒绝连接".to_string()
        } else if lower.contains("no route to host") {
            "主机不可达".to_string()
        } else if lower.contains("could not resolve") || lower.contains("name or service not known") {
            "域名解析失败".to_string()
        } else {
            text.lines().next().unwrap_or("未知结果").to_string()
        };

        Ok(TestResult {
            reachable,
            message,
            latency_ms: ms,
        })
    })
    .await
    .map_err(|e| format!("任务失败: {}", e))?
}

/// Electerm 检测结果
#[derive(Debug, Clone, Serialize)]
pub struct ElectermInfo {
    pub found: bool,
    pub path: String,
    pub source: String,
    pub version: String,
}

fn is_exec(p: &std::path::Path) -> bool {
    if !p.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return fs::metadata(p)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// 按优先级收集 Electerm 可能的路径
fn electerm_candidates(state: &AppState) -> Vec<(String, &'static str)> {
    let mut v: Vec<(String, &'static str)> = vec![];
    let cfg = state.cfg();

    // 1. 用户在设置里配置的路径
    if !cfg.electerm_cmd.trim().is_empty() {
        let p = expand_tilde(&cfg.electerm_cmd);
        v.push((p.to_string_lossy().to_string(), "config"));
    }
    // 2. PATH 中的命令（Windows 用 where，可能返回多行，取第一行）
    #[cfg(target_os = "windows")]
    let finder = "where";
    #[cfg(not(target_os = "windows"))]
    let finder = "which";
    if let Ok(r) = run_cmd(finder, &["electerm"], &[]) {
        if r.ok {
            let p = r.stdout.lines().next().unwrap_or("").trim().to_string();
            if !p.is_empty() {
                v.push((p, "path"));
            }
        }
    }
    // 3. 各平台常见安装路径
    #[cfg(target_os = "linux")]
    {
        for p in [
            "/opt/electerm/electerm",
            "/usr/bin/electerm",
            "/usr/local/bin/electerm",
            "/snap/bin/electerm",
            "/var/lib/flatpak/exports/bin/electerm",
        ] {
            v.push((p.to_string(), "common"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        v.push((
            "/Applications/Electerm.app/Contents/MacOS/Electerm".to_string(),
            "common",
        ));
        v.push((
            format!(
                "{}/Applications/Electerm.app/Contents/MacOS/Electerm",
                crate::core::home().display()
            ),
            "common",
        ));
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            v.push((
                format!(r"{}\Programs\electerm\electerm.exe", local),
                "common",
            ));
        }
        v.push((
            r"C:\Program Files\electerm\electerm.exe".to_string(),
            "common",
        ));
    }
    v
}

/// 探测本机 Electerm
pub fn detect_electerm_inner(state: &AppState) -> ElectermInfo {
    for (path, source) in electerm_candidates(state) {
        let p = std::path::Path::new(&path);
        if is_exec(p) {
            let version = electerm_version(&path, source);
            return ElectermInfo {
                found: true,
                path,
                source: source.to_string(),
                version,
            };
        }
    }
    ElectermInfo {
        found: false,
        path: String::new(),
        source: "none".into(),
        version: String::new(),
    }
}

/// 只用「不启动程序」的方式取版本号。
/// 修复的 Bug：Electerm 是 Electron GUI 应用，执行 `electerm --version` 不会打印版本退出，
/// 而是直接把 GUI 拉起来——导致每次打开设置页（自动探测）都会触发一次 Electerm 启动。
fn electerm_version(path: &str, source: &str) -> String {
    #[cfg(target_os = "linux")]
    {
        // deb 安装：dpkg 查询版本，绝不启动程序
        if let Ok(r) = run_cmd("dpkg-query", &["-W", "-f=${Version}", "electerm"], &[]) {
            if r.ok {
                let v = r.stdout.trim().to_string();
                if !v.is_empty() {
                    return v;
                }
            }
        }
        // snap 安装
        if path.starts_with("/snap/") {
            if let Ok(r) = run_cmd("snap", &["list", "electerm"], &[]) {
                if r.ok {
                    if let Some(v) = r
                        .stdout
                        .lines()
                        .nth(1)
                        .and_then(|l| l.split_whitespace().nth(1))
                    {
                        return v.to_string();
                    }
                }
            }
        }
        let _ = source;
        String::new()
    }
    #[cfg(target_os = "macos")]
    {
        // 从 Info.plist 读取，不启动程序
        let _ = (path, source);
        if let Ok(r) = run_cmd(
            "defaults",
            &[
                "read",
                "/Applications/Electerm.app/Contents/Info.plist",
                "CFBundleShortVersionString",
            ],
            &[],
        ) {
            if r.ok {
                let v = r.stdout.trim().to_string();
                if !v.is_empty() {
                    return v;
                }
            }
        }
        String::new()
    }
    #[cfg(target_os = "windows")]
    {
        // 读 exe 的版本元数据，不启动程序
        if let Ok(r) = run_cmd(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                &format!(
                    "(Get-Item '{}').VersionInfo.ProductVersion",
                    path.replace('\'', "''")
                ),
            ],
            &[],
        ) {
            if r.ok {
                return r.stdout.trim().to_string();
            }
        }
        let _ = source;
        String::new()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        String::new()
    }
}

/// 把已运行的 Electerm 窗口带到前台
/// Electerm 是单实例应用：已在运行时命令行连接会作为新标签加入，
/// 但窗口不会自动前置，用户会以为"没打开"。
fn bring_electerm_to_front() {
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        #[cfg(target_os = "linux")]
        {
            let _ = std::process::Command::new("sh")
                .arg("-c")
                .arg(concat!(
                    "for w in $(xdotool search --name electerm 2>/dev/null); do ",
                    "if xwininfo -id $w 2>/dev/null | grep -q IsViewable; then ",
                    "xdotool windowactivate --sync $w 2>/dev/null; ",
                    "xdotool windowraise $w 2>/dev/null; ",
                    "break; fi; done"
                ))
                .spawn();
        }
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("osascript")
                .args(["-e", "tell application \"Electerm\" to activate"])
                .spawn();
        }
        #[cfg(target_os = "windows")]
        {
            let _ = std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    "(New-Object -ComObject WScript.Shell).AppActivate('Electerm')",
                ])
                .spawn();
        }
    });
}

#[tauri::command]
pub fn detect_electerm(state: State<AppState>) -> ElectermInfo {
    detect_electerm_inner(&state)
}

/// 用 Electerm 打开指定主机：把 user/port/hostname/密码作为命令行参数传入
#[tauri::command]
pub fn open_electerm(state: State<AppState>, alias: Option<String>) -> Result<String, String> {
    let info = detect_electerm_inner(&state);
    if !info.found {
        return Err("NOT_INSTALLED".into());
    }

    let Some(a) = alias else {
        std::process::Command::new(&info.path)
            .spawn()
            .map_err(|e| format!("启动 Electerm 失败: {}", e))?;
        bring_electerm_to_front();
        return Ok("Electerm 已启动".into());
    };

    let hosts = load_hosts(&state)?;
    let h = hosts
        .iter()
        .find(|x| x.alias == a)
        .ok_or_else(|| format!("主机 {} 不存在", a))?;

    if !h.shell_capable {
        return Err(format!(
            "该主机是 Git 服务（{}@{}），只支持 git 操作，不支持交互式终端。",
            h.user, h.hostname
        ));
    }

    // electerm -l <user> -P <port> [-pw <password>] -t <title> <hostname>
    let mut cmd = std::process::Command::new(&info.path);
    cmd.arg("-l")
        .arg(&h.user)
        .arg("-P")
        .arg(h.port.to_string())
        .arg("-t")
        .arg(&h.alias);

    if let Some(pw) = crate::core::get_password(&state, &h.alias) {
        cmd.arg("-pw").arg(pw);
    }
    cmd.arg(&h.hostname);

    cmd.spawn()
        .map_err(|e| format!("启动 Electerm 失败（{}）: {}", info.path, e))?;

    // 单实例模式下窗口不会自动前置，这里主动激活
    bring_electerm_to_front();

    state.log(
        "connect",
        "已用 Electerm 连接",
        &format!("{} → {}@{}:{}", h.alias, h.user, h.hostname, h.port),
    );
    Ok(format!(
        "已在 Electerm 中打开 {}（{}@{}:{}）",
        h.alias, h.user, h.hostname, h.port
    ))
}

/// 打开 notes 目录或指定文件（用系统文件管理器）/// 打开 notes 目录或指定文件（用系统文件管理器）
#[tauri::command]
pub fn open_path(state: State<AppState>, path: String) -> Result<(), String> {
    let p = if path.is_empty() {
        expand_tilde(&state.cfg().notes_dir)
    } else {
        expand_tilde(&path)
    };
    #[cfg(target_os = "linux")]
    let r = std::process::Command::new("xdg-open").arg(&p).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&p).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(&p).spawn();
    r.map(|_| ()).map_err(|e| format!("打开失败: {}", e))
}

/// SSH 密钥文件信息
#[derive(Debug, Clone, Serialize)]
pub struct SshKeyInfo {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub has_pub: bool,
}

/// 列出 ~/.ssh 下的私钥（供"读取系统"选择）
#[tauri::command]
pub fn list_ssh_keys(state: State<AppState>) -> Vec<SshKeyInfo> {
    let dir = ssh_dir(&state);
    let mut out = vec![];
    let Ok(entries) = fs::read_dir(&dir) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if !p.is_file()
            || name.ends_with(".pub")
            || name == "known_hosts"
            || name.starts_with("known_hosts.")
            || name == "config"
            || name.starts_with("config.")
            || name.starts_with("assh")
            || name == "authorized_keys"
            || name == "authorized_keys2"
            || name.ends_with(".bak")
        {
            continue;
        }
        // 私钥通常以 id_ 开头，或文件内含私钥头
        let looks_like_key = name.starts_with("id_")
            || fs::read_to_string(&p)
                .map(|c| c.contains("PRIVATE KEY"))
                .unwrap_or(false);
        if !looks_like_key {
            continue;
        }
        let kind = if name.contains("ed25519") {
            "ed25519"
        } else if name.contains("rsa") {
            "rsa"
        } else if name.contains("ecdsa") {
            "ecdsa"
        } else {
            "unknown"
        };
        out.push(SshKeyInfo {
            path: format!("{}/{}", dir.display(), name),
            name: name.clone(),
            kind: kind.to_string(),
            has_pub: dir.join(format!("{}.pub", name)).exists(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// 验证随机的主机名合法性（前端用，可选）
#[tauri::command]
pub fn validate_alias(alias: String) -> Result<bool, String> {
    Ok(!alias.trim().is_empty()
        && !alias.chars().any(|c| c.is_whitespace() || c == ':' || c == '#'))
}

// 让 safe_join 在 SSH 模块也被使用（避免 unused warning）
#[allow(dead_code)]
fn _unused_safe_join(base: &std::path::Path, rel: &str) -> Result<PathBuf, String> {
    safe_join(base, rel)
}
