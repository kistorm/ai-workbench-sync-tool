mod ai;
mod core;
mod cron;
mod notes;
mod ssh;
mod sync;
mod terminal;
mod youdao;
#[cfg(test)]
mod tests;

use core::AppState;
use std::time::{Duration, Instant};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            let state = AppState::init(data_dir);
            // 启动时把配置里的仓库地址同步到本地仓库
            let cfg_start = state.cfg();
            let repo_dir = sync::config_repo_dir();
            if repo_dir.join(".git").exists() && !cfg_start.git_repo.trim().is_empty() {
                let _ = sync::apply_remote(&repo_dir, &cfg_start.git_repo);
            }
            state.log("info", "NEXUS Workbench 已启动", "工作台就绪");
            app.manage(state);
            app.manage(terminal::TerminalManager::default());

            // ===== 应用内定时同步（与系统 cron 互补）=====
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut last_tick = Instant::now();
                let mut last_run: Option<Instant> = None;
                loop {
                    std::thread::sleep(Duration::from_secs(30));
                    let st = app_handle.state::<AppState>().inner().clone();
                    let cfg = st.cfg();
                    if !cfg.auto_sync_enabled || cfg.sync_interval_min == 0 {
                        last_tick = Instant::now();
                        continue;
                    }
                    let cron_installed = cron::cron_status_quiet();
                    let interval = Duration::from_secs(cfg.sync_interval_min as u64 * 60);
                    let due = match last_run {
                        Some(t) => t.elapsed() >= interval,
                        None => last_tick.elapsed() >= interval,
                    };
                    if due {
                        last_run = Some(Instant::now());
                        if !cron_installed {
                            let st2 = st.clone();
                            std::thread::spawn(move || {
                                let _ = sync::run_sync_all_blocking(&st2, "push");
                            });
                        }
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // core
            core::get_sync_config,
            core::save_sync_config,
            core::list_activities,
            core::log_activity,
            core::get_running_action,
            core::app_info,
            core::has_password,
            // ssh
            ssh::list_hosts,
            ssh::read_assh_yml,
            ssh::save_assh_yml,
            ssh::add_host,
            ssh::update_host,
            ssh::delete_host,
            ssh::build_ssh_config,
            ssh::test_host,
            ssh::open_electerm,
            ssh::detect_electerm,
            ssh::force_pull_ssh_config,
            ssh::open_path,
            ssh::validate_alias,
            ssh::list_ssh_keys,
            ssh::set_host_password,
            ssh::reveal_host_password,
            ssh::clear_host_password,
            // terminal
            terminal::ssh_terminal_open,
            terminal::ssh_terminal_write,
            terminal::ssh_terminal_resize,
            terminal::ssh_terminal_close,
            // ai
            ai::ai_status,
            ai::ai_sync_run,
            // notes
            notes::list_notes,
            notes::read_note,
            notes::save_note,
            notes::create_note,
            notes::delete_note,
            notes::notes_git_sync,
            notes::notes_info,
            notes::clear_notes,
            // youdao
            youdao::youdao_info,
            youdao::youdao_pull,
            // sync
            sync::diagnose_git,
            sync::git_sync,
            sync::repo_info,
            sync::sync_all,
            sync::init_data_repo,
            sync::force_restore,
            // cron
            cron::cron_install,
            cron::cron_status,
            cron::cron_remove,
            cron::sync_log_tail,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
