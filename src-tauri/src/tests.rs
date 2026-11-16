//! 核心逻辑单元测试
#![cfg(test)]

use crate::core::{count_files, expand_tilde, home, rel_time, safe_join};
use crate::ssh::{insert_host_block, remove_host_block};
use std::fs;
use std::path::Path;

// ==================== safe_join ====================

#[test]
fn safe_join_allows_relative() {
    assert!(safe_join(Path::new("/tmp/notes"), "a.md").is_ok());
    assert!(safe_join(Path::new("/tmp/notes"), "sub/dir/b.md").is_ok());
}

#[test]
fn safe_join_rejects_parent_dir() {
    assert!(safe_join(Path::new("/tmp/notes"), "../etc/passwd").is_err());
    assert!(safe_join(Path::new("/tmp/notes"), "sub/../../x").is_err());
}

#[test]
fn safe_join_rejects_absolute() {
    assert!(safe_join(Path::new("/tmp/notes"), "/etc/passwd").is_err());
    assert!(safe_join(Path::new("/tmp/notes"), "C:\\Windows\\system32").is_ok()); // 非 Unix 绝对路径视为文件名
}

#[test]
fn safe_join_keeps_inside_base() {
    let base = Path::new("/tmp/notes");
    let p = safe_join(base, "a/b.md").unwrap();
    assert!(p.starts_with(base));
}

// ==================== expand_tilde ====================

#[test]
fn expand_tilde_expands_home() {
    let h = home();
    assert_eq!(expand_tilde("~"), h);
    assert_eq!(expand_tilde("~/notes"), h.join("notes"));
}

#[test]
fn expand_tilde_keeps_absolute() {
    let p = expand_tilde("/var/log");
    assert_eq!(p, std::path::PathBuf::from("/var/log"));
}

#[test]
fn expand_tilde_keeps_relative() {
    let p = expand_tilde("relative/path");
    assert_eq!(p, std::path::PathBuf::from("relative/path"));
}

// ==================== rel_time ====================

#[test]
fn rel_time_buckets() {
    let now = chrono::Local::now().timestamp();
    assert_eq!(rel_time(now), "刚刚");
    assert_eq!(rel_time(now - 120), "2分钟前");
    let yesterday = rel_time(now - 86400 - 3600);
    assert_eq!(yesterday, "昨天");
    assert_eq!(rel_time(now - 86400 * 3), "3天前");
}

// ==================== count_files ====================

#[test]
fn count_files_recursive() {
    let dir = std::env::temp_dir().join(format!("nexus-test-{}", std::process::id()));
    let sub = dir.join("sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(dir.join("a.md"), "1").unwrap();
    fs::write(dir.join("b.txt"), "2").unwrap();
    fs::write(sub.join("c.md"), "3").unwrap();
    assert_eq!(count_files(&dir, ".md"), 2);
    assert_eq!(count_files(&dir, ".txt"), 1);
    assert_eq!(count_files(&dir, ".xyz"), 0);
    let _ = fs::remove_dir_all(&dir);
}

// ==================== insert_host_block ====================

#[test]
fn insert_host_before_next_section() {
    let content = "hosts:\n  a:\n    hostname: 1.1.1.1\n\ndefaults:\n  port: 22\n";
    let block = "  b:\n    hostname: 2.2.2.2\n    user: deploy\n";
    let out = insert_host_block(content, block).unwrap();
    assert!(out.contains("b:"));
    // 必须插在 defaults 之前
    assert!(out.find("b:").unwrap() < out.find("defaults:").unwrap());
    // YAML 合法，两个主机都在
    let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
    let hosts = v.get("hosts").unwrap().as_mapping().unwrap();
    assert!(hosts.get(serde_yaml::Value::String("a".into())).is_some());
    assert!(hosts.get(serde_yaml::Value::String("b".into())).is_some());
    // defaults 未被破坏
    assert!(v.get("defaults").is_some());
}

#[test]
fn insert_host_appends_when_hosts_last() {
    let content = "templates:\n  t:\n    user: x\nhosts:\n  a:\n    hostname: 1.1.1.1\n";
    let block = "  b:\n    hostname: 2.2.2.2\n";
    let out = insert_host_block(content, block).unwrap();
    let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
    let hosts = v.get("hosts").unwrap().as_mapping().unwrap();
    assert_eq!(hosts.len(), 2);
}

#[test]
fn insert_host_creates_hosts_section() {
    let content = "templates:\n  t:\n    user: x\n";
    let block = "  a:\n    hostname: 1.1.1.1\n";
    let out = insert_host_block(content, block).unwrap();
    let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
    assert!(v.get("hosts").is_some());
}

#[test]
fn insert_host_preserves_comments() {
    let content = "# 我的配置\nhosts:\n  # 生产\n  a:\n    hostname: 1.1.1.1\n";
    let block = "  b:\n    hostname: 2.2.2.2\n";
    let out = insert_host_block(content, block).unwrap();
    assert!(out.contains("# 我的配置"));
    assert!(out.contains("# 生产"));
}

// ==================== remove_host_block ====================

#[test]
fn remove_host_removes_only_target() {
    let content = "hosts:\n  a:\n    hostname: 1.1.1.1\n  b:\n    hostname: 2.2.2.2\n\ndefaults:\n  port: 22\n";
    let out = remove_host_block(content, "a").unwrap();
    let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
    let hosts = v.get("hosts").unwrap().as_mapping().unwrap();
    assert!(hosts.get(serde_yaml::Value::String("a".into())).is_none());
    assert!(hosts.get(serde_yaml::Value::String("b".into())).is_some());
    assert!(v.get("defaults").is_some());
}

#[test]
fn remove_host_keeps_other_sections() {
    let content = "templates:\n  t:\n    user: x\nhosts:\n  a:\n    hostname: 1.1.1.1\n";
    let out = remove_host_block(content, "a").unwrap();
    let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
    assert!(v.get("templates").is_some());
    let hosts = v.get("hosts").unwrap().as_mapping().unwrap();
    assert!(hosts.is_empty());
}

#[test]
fn remove_host_prefix_no_false_positive() {
    // 删除 web 不应误删 web-prod
    let content = "hosts:\n  web-prod:\n    hostname: 1.1.1.1\n  web:\n    hostname: 2.2.2.2\n";
    let out = remove_host_block(content, "web").unwrap();
    let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
    let hosts = v.get("hosts").unwrap().as_mapping().unwrap();
    assert!(hosts.get(serde_yaml::Value::String("web-prod".into())).is_some());
    assert!(hosts.get(serde_yaml::Value::String("web".into())).is_none());
}

#[test]
fn remove_host_missing_is_noop() {
    let content = "hosts:\n  a:\n    hostname: 1.1.1.1\n";
    let out = remove_host_block(content, "not-exist").unwrap();
    let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
    assert_eq!(v.get("hosts").unwrap().as_mapping().unwrap().len(), 1);
}

// ==================== 集成测试（真实文件/命令） ====================

use crate::ai::ai_status_impl;
use crate::core::AppState;
use crate::cron::{cron_install_impl, cron_remove_impl};
use crate::notes::{create_note_impl, delete_note_impl, list_notes_impl, read_note_impl, save_note_impl};
use crate::ssh::{add_host_impl, build_ssh_config_impl, delete_host_impl};
use crate::sync::diagnose_git_impl;
use std::path::PathBuf;

/// 构造隔离的测试状态：临时 data/ssh/notes/ai 目录
fn test_state(tag: &str) -> (AppState, PathBuf) {
    let base = std::env::temp_dir().join(format!("nexus-it-{}-{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("ssh")).unwrap();
    fs::create_dir_all(base.join("notes")).unwrap();
    fs::create_dir_all(base.join("ai")).unwrap();
    let state = AppState::init(base.join("data"));
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.ssh_dir = base.join("ssh").to_string_lossy().to_string();
        cfg.notes_dir = base.join("notes").to_string_lossy().to_string();
        cfg.ai_sync_dir = base.join("ai").to_string_lossy().to_string();
        let c = cfg.clone();
        drop(cfg);
        state.save_config(&c).unwrap();
    }
    (state, base)
}

#[test]
fn it_add_and_delete_host_real_file() {
    let (state, base) = test_state("ssh");
    let yml = base.join("ssh").join("assh.yml");
    fs::write(&yml, "hosts:\n  existing:\n    hostname: 1.1.1.1\n\ndefaults:\n  port: 22\n").unwrap();

    // 添加
    add_host_impl(&state, "new-host", "2.2.2.2", "deploy", 2222, Some("prod-bastion"), "key", Some("~/.ssh/id_ed25519"), None, true).unwrap();
    let content = fs::read_to_string(&yml).unwrap();
    assert!(content.contains("new-host:"));
    assert!(content.contains("port: 2222"));
    assert!(content.contains("inherit: prod-bastion"));
    assert!(content.contains("defaults:"), "原有配置必须保留");
    let v: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
    assert_eq!(v.get("hosts").unwrap().as_mapping().unwrap().len(), 2);

    // 重复添加必须报错
    assert!(add_host_impl(&state, "new-host", "3.3.3.3", "", 22, None, "none", None, None, true).is_err());
    // 非法别名必须报错
    assert!(add_host_impl(&state, "bad alias", "3.3.3.3", "", 22, None, "none", None, None, true).is_err());

    // 删除
    delete_host_impl(&state, "new-host").unwrap();
    let content = fs::read_to_string(&yml).unwrap();
    let v: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
    let hosts = v.get("hosts").unwrap().as_mapping().unwrap();
    assert_eq!(hosts.len(), 1);
    assert!(hosts.get(serde_yaml::Value::String("existing".into())).is_some());

    // 活动日志有记录
    let acts = state.activities.lock().unwrap();
    assert!(acts.items.iter().any(|a| a.title.contains("已添加主机")));
    assert!(acts.items.iter().any(|a| a.title.contains("已删除主机")));
}

#[test]
fn it_build_ssh_config_with_isolated_assh_yml() {
    let (state, base) = test_state("build");
    let yml = base.join("ssh").join("assh.yml");
    fs::write(&yml, "hosts:\n  demo:\n    hostname: 10.0.0.1\n    user: root\n").unwrap();

    let out = build_ssh_config_impl(&state).expect("build 应成功");
    assert!(out.contains("已写入"));
    let cfg = fs::read_to_string(base.join("ssh").join("config")).unwrap();
    assert!(cfg.contains("Host demo"), "生成内容: {}", cfg);
    assert!(cfg.contains("ProxyCommand"), "应包含 assh 的 ProxyCommand");
}

#[test]
fn it_notes_full_crud() {
    let (state, base) = test_state("notes");

    // 创建
    let name = create_note_impl(&state, "测试笔记").unwrap();
    assert_eq!(name, "测试笔记.md");
    assert!(base.join("notes").join(&name).exists());

    // 读（初始模板）
    let text = read_note_impl(&state, &name).unwrap();
    assert!(text.contains("# 测试笔记"));

    // 保存 + 读回
    save_note_impl(&state, &name, "# 更新后的内容\n\n正文").unwrap();
    assert_eq!(read_note_impl(&state, &name).unwrap(), "# 更新后的内容\n\n正文");

    // 列表
    let list = list_notes_impl(&state).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "测试笔记");

    // 重名自动加后缀
    let name2 = create_note_impl(&state, "测试笔记").unwrap();
    assert_eq!(name2, "测试笔记-2.md");
    assert_eq!(list_notes_impl(&state).unwrap().len(), 2);

    // 路径逃逸必须被拒绝
    assert!(save_note_impl(&state, "../evil.md", "x").is_err());
    assert!(read_note_impl(&state, "../evil.md").is_err());
    assert!(delete_note_impl(&state, "../evil.md").is_err());

    // 非 Markdown 不允许删除
    fs::write(base.join("notes").join("x.txt"), "x").unwrap();
    assert!(delete_note_impl(&state, "x.txt").is_err());

    // 删除
    delete_note_impl(&state, &name).unwrap();
    assert_eq!(list_notes_impl(&state).unwrap().len(), 1);
}

#[test]
fn it_cron_install_update_remove() {
    // 备份用户 crontab，测试后恢复
    let backup = crate::cron::read_crontab();
    let (state, base) = test_state("cron");

    let r1 = cron_install_impl(&state, 30).expect("安装应成功");
    assert!(r1.installed);
    assert!(r1.line.contains("*/30 * * * *"), "line: {}", r1.line);
    assert!(r1.line.contains("nexus-workbench"));
    // 自动同步脚本已生成
    assert!(std::path::Path::new(&r1.script_path).exists(), "脚本应存在");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&r1.script_path).unwrap().permissions().mode();
        assert!(mode & 0o111 != 0, "脚本应可执行");
    }
    assert!(crate::cron::cron_status_quiet(), "crond 中应能查到");

    // 更新间隔（应替换而非追加）
    let r2 = cron_install_impl(&state, 120).expect("更新应成功");
    assert!(r2.line.contains("0 */2 * * *"), "line: {}", r2.line);
    let crontab = crate::cron::read_crontab();
    assert_eq!(
        crontab.matches("# nexus-workbench").count(),
        1,
        "只应有一条 nexus 任务"
    );

    // 移除
    cron_remove_impl(&state).expect("移除应成功");
    assert!(!crate::cron::cron_status_quiet());
    let crontab_after = crate::cron::read_crontab();
    assert!(!crontab_after.contains("# nexus-workbench"));

    // 恢复备份
    if backup.trim().is_empty() {
        let _ = crate::core::run_cmd("crontab", &["-r"], &[]);
    } else {
        crate::cron::write_crontab(&backup).expect("恢复 crontab");
    }
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn it_diagnose_git_local_repo() {
    // 用本地 bare 仓库测试，不依赖网络
    let (state, base) = test_state("diag");
    let _ = &state;
    let repo_dir = base.join("remote.git");
    fs::create_dir_all(&repo_dir).unwrap();
    crate::core::run_cmd("git", &["init", "--bare", repo_dir.to_str().unwrap()], &[]).unwrap();
    let url = format!("file://{}", repo_dir.display());
    let d = diagnose_git_impl(&url, "~/.ssh/id_ed25519_github").expect("诊断应返回结果");
    assert!(d.ok, "本地裸仓库应可访问: {}", d.message);
    assert!(d.latency_ms < 60_000);
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn it_ai_status_detects_tools() {
    let (state, _base) = test_state("ai");
    let st = ai_status_impl(&state);
    // 五个已知工具的条目必须都在（是否 detected 取决于机器）
    let names: Vec<&str> = st.tools.iter().map(|t| t.name.as_str()).collect();
    for expected in ["ZCode", "Claude Code", "Cursor", "Trae", "DSH"] {
        assert!(names.contains(&expected), "缺少工具条目 {}", expected);
    }
    assert!(!st.script_found, "临时目录不应有 sync.sh");
}

#[test]
fn it_activity_log_persists() {
    let (state, base) = test_state("act");
    state.log("sync", "测试事件", "细节");
    // 内存中有
    let items = state.activities.lock().unwrap().items.clone();
    assert_eq!(items[0].title, "测试事件");
    // 已落盘
    let raw = fs::read_to_string(base.join("data").join("activities.json")).unwrap();
    assert!(raw.contains("测试事件"));
}

#[test]
fn it_config_roundtrip() {
    let (state, base) = test_state("cfg");
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.sync_interval_min = 42;
        cfg.git_repo = "git@example.com:x/y.git".into();
        let c = cfg.clone();
        drop(cfg);
        state.save_config(&c).unwrap();
    }
    // 新实例重新加载
    let state2 = AppState::init(base.join("data"));
    let cfg = state2.cfg();
    assert_eq!(cfg.sync_interval_min, 42);
    assert_eq!(cfg.git_repo, "git@example.com:x/y.git");
}

#[test]
fn it_add_host_with_key_and_trust() {
    let (state, base) = test_state("sshkey");
    let yml = base.join("ssh").join("assh.yml");
    fs::write(&yml, "hosts: {}\n").unwrap();
    add_host_impl(
        &state,
        "k1",
        "10.9.9.9",
        "root",
        22,
        None,
        "key",
        Some("~/.ssh/id_ed25519_github"),
        None,
        true,
    )
    .unwrap();
    let content = fs::read_to_string(&yml).unwrap();
    assert!(content.contains("identityfile: ~/.ssh/id_ed25519_github"));
    assert!(content.contains("stricthostkeychecking: accept-new"));
    // 重新解析应能读出 trust
    let hosts = crate::ssh::list_hosts_impl(&state).unwrap();
    let h = hosts.iter().find(|x| x.alias == "k1").unwrap();
    assert_eq!(h.key_file.as_deref(), Some("~/.ssh/id_ed25519_github"));
    assert!(h.trust, "trust 应为 true");
}

#[test]
fn it_add_host_with_password_saves_secret() {
    let (state, base) = test_state("sshpw");
    let yml = base.join("ssh").join("assh.yml");
    fs::write(&yml, "hosts: {}\n").unwrap();
    add_host_impl(
        &state,
        "p1",
        "10.8.8.8",
        "root",
        22,
        None,
        "password",
        None,
        Some("s3cret"),
        true,
    )
    .unwrap();
    // 密码不写入 assh.yml
    let content = fs::read_to_string(&yml).unwrap();
    assert!(!content.contains("s3cret"), "密码不能出现在配置里");
    // 存到本地 secrets.json 且权限 600
    assert_eq!(crate::core::get_password(&state, "p1").as_deref(), Some("s3cret"));
    let secrets = base.join("data").join("secrets.json");
    assert!(secrets.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&secrets).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "secrets.json 权限应为 600");
    }
    // 删除主机后密码保留（主机可从云端恢复，密码继续可用）
    crate::ssh::delete_host_impl(&state, "p1").unwrap();
    assert!(
        crate::core::get_password(&state, "p1").is_some(),
        "删除主机不应清理本地密码"
    );
}

#[test]
fn it_full_sync_to_local_bare_repo() {
    // 用本地裸仓库模拟远端，验证「SSH 配置 + 笔记」一次推送
    let (state, base) = test_state("fullsync");
    let remote = base.join("remote.git");
    fs::create_dir_all(&remote).unwrap();
    crate::core::run_cmd("git", &["init", "--bare", remote.to_str().unwrap()], &[]).unwrap();

    // 构造数据仓库
    let repo = base.join("data-repo");
    fs::create_dir_all(repo.join("notes")).unwrap();
    crate::core::run_cmd("git", &["init", "-q", repo.to_str().unwrap()], &[]).unwrap();
    crate::core::run_cmd("git", &["-C", repo.to_str().unwrap(), "checkout", "-q", "-b", "main"], &[]).ok();
    fs::write(repo.join("assh.yml"), "hosts:\n  x:\n    hostname: 1.1.1.1\n").unwrap();
    fs::write(repo.join("notes").join("a.md"), "# note\n").unwrap();
    crate::core::run_cmd("git", &["-C", repo.to_str().unwrap(), "add", "-A"], &[]).unwrap();
    crate::core::run_cmd(
        "git",
        &["-C", repo.to_str().unwrap(), "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"],
        &[],
    ).unwrap();

    // 指向本地 remote
    let url = format!("file://{}", remote.display());
    crate::sync::apply_remote(&repo, &url).unwrap();
    let push = crate::core::run_cmd("git", &["-C", repo.to_str().unwrap(), "push", "-q", "-u", "origin", "main"], &[]).unwrap();
    assert!(push.ok, "首次推送应成功: {}", push.combined());

    // 远端应有 main 分支和提交
    let ls = crate::core::run_cmd(
        "git",
        &["--git-dir", remote.to_str().unwrap(), "rev-parse", "--verify", "main"],
        &[],
    )
    .unwrap();
    assert!(ls.ok, "远端应有 main 分支: {}", ls.combined());
}

// ==================== 有道云 Cookie 解析 ====================

#[test]
fn it_parse_cookies_cookie_header() {
    let input = "YNOTE_CSTK=abc123; YNOTE_LOGIN=3||1649054441155; YNOTE_SESS=v2|xyz; other=1";
    let (a, b, c) = crate::youdao::parse_cookies(input).unwrap();
    assert_eq!(a, "abc123");
    assert_eq!(b, "3||1649054441155");
    assert_eq!(c, "v2|xyz");
}

#[test]
fn it_parse_cookies_json() {
    let input = r#"{"YNOTE_CSTK":"a","YNOTE_LOGIN":"b","YNOTE_SESS":"c"}"#;
    let (a, b, c) = crate::youdao::parse_cookies(input).unwrap();
    assert_eq!((a.as_str(), b.as_str(), c.as_str()), ("a", "b", "c"));
}

#[test]
fn it_parse_cookies_missing_is_error() {
    assert!(crate::youdao::parse_cookies("YNOTE_CSTK=abc").is_err());
    assert!(crate::youdao::parse_cookies("").is_err());
    assert!(crate::youdao::parse_cookies("garbage").is_err());
}

// ==================== 清空笔记 ====================

#[test]
fn it_clear_notes_removes_all_but_git() {
    let (state, base) = test_state("clear");
    let notes = base.join("notes");
    fs::create_dir_all(notes.join("sub/deep")).unwrap();
    fs::create_dir_all(notes.join(".git")).unwrap();
    fs::write(notes.join("a.md"), "aaa").unwrap();
    fs::write(notes.join("sub/b.md"), "bb").unwrap();
    fs::write(notes.join("sub/deep/c.md"), "c").unwrap();
    fs::write(notes.join(".git/config"), "keep").unwrap();

    let r = crate::notes::clear_notes_impl(&state).unwrap();
    assert_eq!(r.files, 3, "应删除 3 个文件");
    assert_eq!(r.dirs, 1, "应删除 1 个子目录（.git 保留）");
    assert!(r.bytes >= 6);

    // 内容清空
    assert!(!notes.join("a.md").exists());
    assert!(!notes.join("sub").exists());
    // .git 与目录本身保留
    assert!(notes.join(".git/config").exists(), ".git 必须保留");
    assert!(notes.exists(), "笔记目录本身保留");
    // 再次清空应为 0
    let r2 = crate::notes::clear_notes_impl(&state).unwrap();
    assert_eq!(r2.files, 0);
}

#[test]
fn it_clear_notes_empty_dir_is_ok() {
    let (state, _base) = test_state("clear-empty");
    let r = crate::notes::clear_notes_impl(&state).unwrap();
    assert_eq!(r.files, 0);
    assert_eq!(r.dirs, 0);
}

// ==================== 修改主机信息 ====================

#[test]
fn it_update_host_preserves_and_renames() {
    let (state, base) = test_state("update");
    let yml = base.join("ssh").join("assh.yml");
    fs::write(
        &yml,
        "hosts:\n  old:\n    hostname: 1.1.1.1\n    user: root\n    inherit: tmpl\n",
    )
    .unwrap();

    crate::ssh::update_host_impl(&state, "old", "new", "2.2.2.2", "deploy", 2222, true).unwrap();

    let content = fs::read_to_string(&yml).unwrap();
    assert!(content.contains("new:"), "新别名应存在");
    assert!(!content.contains("  old:"), "旧别名应删除");
    assert!(content.contains("hostname: 2.2.2.2"));
    assert!(content.contains("user: deploy"));
    assert!(content.contains("port: 2222"));
    assert!(content.contains("inherit: tmpl"), "未在表单中的字段应保留");
    assert!(content.contains("stricthostkeychecking: accept-new"));
    // YAML 合法
    let _: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
}

#[test]
fn it_update_host_missing_is_error() {
    let (state, base) = test_state("update-missing");
    let yml = base.join("ssh").join("assh.yml");
    fs::write(&yml, "hosts: {}\n").unwrap();
    assert!(
        crate::ssh::update_host_impl(&state, "nope", "nope2", "1.1.1.1", "root", 22, true).is_err()
    );
}
