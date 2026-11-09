use std::cell::RefCell;
use std::rc::Rc;

use crabhosts_core::{Document, HostEntry, Profile, Storage, parse_hosts, plan_profile};
use crabhosts_system_client::SystemClient;
use slint::{ModelRc, VecModel};

slint::include_modules!();

struct UiState {
    storage: Storage,
    document: Document,
    selected: Option<String>,
}

#[cfg(target_os = "macos")]
fn set_dock_icon() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let main_thread = MainThreadMarker::new().expect("desktop startup runs on the main thread");
    let data = NSData::with_bytes(include_bytes!("../macos/CrabHosts.icns"));
    if let Some(icon) = NSImage::initWithData(NSImage::alloc(), &data) {
        let app = NSApplication::sharedApplication(main_thread);
        // SAFETY: AppKit receives a valid, non-null image on the main thread.
        unsafe { app.setApplicationIconImage(Some(&icon)) };
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let window = MainWindow::new()?;
    #[cfg(target_os = "macos")]
    set_dock_icon();
    let state = Rc::new(RefCell::new(UiState {
        storage: Storage::from_env()?,
        document: Document::default(),
        selected: None,
    }));
    window.set_panel_label("差异预览（只读）".into());
    wire(&window, Rc::clone(&state));

    refresh(&window, &state);
    window.run()?;
    Ok(())
}

fn wire(window: &MainWindow, state: Rc<RefCell<UiState>>) {
    window.set_system_hosts_path(
        state
            .borrow()
            .storage
            .paths()
            .hosts_file
            .display()
            .to_string()
            .into(),
    );
    configure_helper_status(window, &state);

    {
        let state = Rc::clone(&state);
        let weak = window.as_weak();
        window.on_refresh(move || {
            if let Some(window) = weak.upgrade() {
                refresh(&window, &state);
            }
        });
    }
    {
        let state = Rc::clone(&state);
        let weak = window.as_weak();
        window.on_profile_selected(move |index| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let selected = {
                let state = state.borrow();
                state
                    .document
                    .profiles
                    .get(index as usize)
                    .map(|profile| profile.name.clone())
            };
            state.borrow_mut().selected = selected;
            window.set_panel_mode(0);
            rebuild(&window, &state.borrow());
            window.invoke_preview_selected();
        });
    }
    {
        let state = Rc::clone(&state);
        let weak = window.as_weak();
        window.on_create_profile(move |name, import_system| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let name = name.trim().to_owned();
            if name.is_empty() {
                window.set_create_error("请填写 Profile 名称。".into());
                return;
            }
            let mut profile = match Profile::new(name.clone()) {
                Ok(profile) => profile,
                Err(error) => {
                    window.set_create_error(format!("创建失败：{error}").into());
                    return;
                }
            };
            let mut imported = 0;
            if import_system {
                match state.borrow().storage.read_hosts() {
                    Ok(content) => {
                        profile.entries = parse_hosts(&content);
                        imported = profile.entries.len();
                    }
                    Err(error) => {
                        window.set_create_error(format!("读取系统 hosts 失败：{error}").into());
                        return;
                    }
                }
            }
            let failure = {
                let mut guard = state.borrow_mut();
                let state = &mut *guard;
                match state.document.add_profile(profile) {
                    Ok(()) => {
                        state.selected = Some(name.clone());
                        state
                            .storage
                            .save(&mut state.document)
                            .err()
                            .map(|error| format!("保存失败：{error}"))
                    }
                    Err(error) => Some(format!("创建失败：{error}")),
                }
            };
            if let Some(message) = failure {
                window.set_create_error(message.into());
                refresh(&window, &state);
                return;
            }
            window.set_create_error(String::new().into());
            window.set_show_create(false);
            refresh(&window, &state);
            window.set_status(
                format!(
                    "已创建 Profile {name:?}（{imported} 条记录）。Profile 编辑不会自动修改系统 hosts。"
                )
                .into(),
            );
        });
    }
    {
        let state = Rc::clone(&state);
        let weak = window.as_weak();
        window.on_delete_profile(move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let outcome = {
                let mut guard = state.borrow_mut();
                let state = &mut *guard;
                let Some(name) = state.selected.clone() else {
                    window.set_status("请先选择一个 Profile。".into());
                    return;
                };
                if let Err(error) = state.document.remove_profile(&name) {
                    Err(format!("删除失败：{error}"))
                } else {
                    match state.storage.save(&mut state.document) {
                        Ok(()) => {
                            state.selected = None;
                            Ok(name)
                        }
                        Err(error) => Err(format!("删除失败：{error}")),
                    }
                }
            };
            match outcome {
                Ok(name) => {
                    rebuild(&window, &state.borrow());
                    window
                        .set_status(format!("已删除 Profile {name:?}；系统 hosts 未修改。").into());
                }
                Err(message) => {
                    refresh(&window, &state);
                    window.set_status(message.into());
                }
            }
        });
    }
    {
        let state = Rc::clone(&state);
        let weak = window.as_weak();
        window.on_preview_selected(move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let (profile, result) = {
                let state = state.borrow();
                let Some(profile) = state.selected.clone() else {
                    window.set_status("请先选择一个 Profile。".into());
                    return;
                };
                let result = plan_profile(&state.storage, &state.document, &profile);
                (profile, result)
            };
            window.set_panel_label("差异预览（只读）".into());
            match result {
                Ok(plan) => {
                    update_panel_text(&window, &plan.diff);
                    window.set_status(
                        if plan.changed {
                            format!("Profile {profile:?} 有待应用变更，尚未修改系统 hosts。")
                        } else {
                            "系统 hosts 已与该 Profile 一致。".to_owned()
                        }
                        .into(),
                    );
                }
                Err(error) => {
                    update_panel_text(&window, &format!("预览失败：{error}"));
                    window.set_status(format!("预览失败：{error}").into());
                }
            }
        });
    }
    {
        let state = Rc::clone(&state);
        let weak = window.as_weak();
        window.on_apply_selected(move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            if window.get_helper_busy() {
                return;
            }
            let helper_state = window.get_helper_state();
            if helper_state != "ready" && helper_state != "direct" {
                window.set_status("系统写入 Helper 尚未就绪，请先在侧栏安装或修复。".into());
                return;
            }
            let planned = {
                let state = state.borrow();
                let Some(profile) = state.selected.clone() else {
                    window.set_status("请先选择一个 Profile。".into());
                    return;
                };
                plan_profile(&state.storage, &state.document, &profile)
            };
            let plan = match planned {
                Ok(plan) => plan,
                Err(error) => {
                    window.set_status(format!("生成应用计划失败：{error}").into());
                    return;
                }
            };
            if !plan.changed {
                window.set_status("没有需要应用的变更。".into());
                return;
            }

            if helper_state == "direct" {
                let result = {
                    let state = state.borrow();
                    SystemClient::new().apply_plan(&state.storage, &plan)
                };
                refresh(&window, &state);
                finish_apply(&window, result);
            } else {
                let storage = state.borrow().storage.clone();
                window.set_helper_busy(true);
                window.set_status("正在通过系统 Helper 应用，请勿重复提交。".into());
                let weak = window.as_weak();
                std::thread::spawn(move || {
                    let result = SystemClient::new().apply_plan(&storage, &plan);
                    let _ = slint::invoke_from_event_loop(move || {
                        let Some(window) = weak.upgrade() else {
                            return;
                        };
                        window.set_helper_busy(false);
                        window.invoke_refresh();
                        finish_apply(&window, result);
                    });
                });
            }
        });
    }
    {
        let state = Rc::clone(&state);
        let weak = window.as_weak();
        window.on_add_entry(move |ip, hostnames, comment| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let hostnames = hostnames
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if hostnames.is_empty() {
                window.set_status("至少填写一个 hostname。".into());
                return;
            }
            let comment = comment.trim();
            let comment = (!comment.is_empty()).then(|| comment.to_owned());
            let entry = match HostEntry::new(ip.trim(), hostnames, true, comment) {
                Ok(entry) => entry,
                Err(error) => {
                    window.set_status(format!("添加失败：{error}").into());
                    return;
                }
            };
            let outcome = {
                let mut guard = state.borrow_mut();
                let state = &mut *guard;
                let Some(name) = state.selected.clone() else {
                    window.set_status("请先选择一个 Profile。".into());
                    return;
                };
                let added = {
                    let Ok(profile) = state.document.profile_mut(&name) else {
                        window.set_status("Profile 已不存在，请刷新。".into());
                        return;
                    };
                    profile.add_or_merge_entry(entry)
                };
                match added {
                    Ok(_) => match state.storage.save(&mut state.document) {
                        Ok(()) => Ok(name),
                        Err(error) => Err(format!("保存失败：{error}")),
                    },
                    Err(error) => Err(format!("添加失败：{error}")),
                }
            };
            match outcome {
                Ok(name) => {
                    rebuild(&window, &state.borrow());
                    window.set_draft_ip(String::new().into());
                    window.set_draft_hostnames(String::new().into());
                    window.set_draft_comment(String::new().into());
                    window
                        .set_status(format!("已更新 Profile {name:?}；系统 hosts 未修改。").into());
                }
                Err(message) => {
                    refresh(&window, &state);
                    window.set_status(message.into());
                }
            }
        });
    }
    {
        let state = Rc::clone(&state);
        let weak = window.as_weak();
        window.on_toggle_entry(move |index| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let outcome = {
                let mut guard = state.borrow_mut();
                let state = &mut *guard;
                let Some(name) = state.selected.clone() else {
                    window.set_status("请先选择一个 Profile。".into());
                    return;
                };
                let changed = {
                    let Ok(profile) = state.document.profile_mut(&name) else {
                        window.set_status("Profile 已不存在，请刷新。".into());
                        return;
                    };
                    match profile.entries.get_mut(index as usize) {
                        Some(entry) => {
                            entry.enabled = !entry.enabled;
                            Some(entry.enabled)
                        }
                        None => None,
                    }
                };
                let Some(enabled) = changed else {
                    window.set_status("记录已不存在，请刷新。".into());
                    return;
                };
                match state.storage.save(&mut state.document) {
                    Ok(()) => Ok(enabled),
                    Err(error) => Err(format!("保存失败：{error}")),
                }
            };
            match outcome {
                Ok(enabled) => {
                    rebuild(&window, &state.borrow());
                    window.set_status(
                        format!(
                            "已{}该记录；系统 hosts 未修改。",
                            if enabled { "启用" } else { "禁用" }
                        )
                        .into(),
                    );
                }
                Err(message) => {
                    refresh(&window, &state);
                    window.set_status(message.into());
                }
            }
        });
    }
    {
        let state = Rc::clone(&state);
        let weak = window.as_weak();
        window.on_remove_entry(move |index| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let outcome = {
                let mut guard = state.borrow_mut();
                let state = &mut *guard;
                let Some(name) = state.selected.clone() else {
                    window.set_status("请先选择一个 Profile。".into());
                    return;
                };
                let removed = {
                    let Ok(profile) = state.document.profile_mut(&name) else {
                        window.set_status("Profile 已不存在，请刷新。".into());
                        return;
                    };
                    if (index as usize) < profile.entries.len() {
                        profile.entries.remove(index as usize);
                        true
                    } else {
                        false
                    }
                };
                if !removed {
                    window.set_status("记录已不存在，请刷新。".into());
                    return;
                }
                match state.storage.save(&mut state.document) {
                    Ok(()) => Ok(()),
                    Err(error) => Err(format!("保存失败：{error}")),
                }
            };
            match outcome {
                Ok(()) => {
                    rebuild(&window, &state.borrow());
                    window.set_status("已删除该记录；系统 hosts 未修改。".into());
                }
                Err(message) => {
                    refresh(&window, &state);
                    window.set_status(message.into());
                }
            }
        });
    }
    {
        let state = Rc::clone(&state);
        let weak = window.as_weak();
        window.on_import_selected_from_system(move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let content = match state.borrow().storage.read_hosts() {
                Ok(content) => content,
                Err(error) => {
                    window.set_status(format!("读取系统 hosts 失败：{error}").into());
                    return;
                }
            };
            let imported = parse_hosts(&content);
            if imported.is_empty() {
                window.set_status("系统 hosts 中没有可导入的记录。".into());
                return;
            }
            let outcome = {
                let mut guard = state.borrow_mut();
                let state = &mut *guard;
                let Some(name) = state.selected.clone() else {
                    window.set_status("请先选择一个 Profile。".into());
                    return;
                };
                let mut merged = 0;
                {
                    let Ok(profile) = state.document.profile_mut(&name) else {
                        window.set_status("Profile 已不存在，请刷新。".into());
                        return;
                    };
                    for entry in imported {
                        if profile.add_or_merge_entry(entry).is_ok() {
                            merged += 1;
                        }
                    }
                }
                match state.storage.save(&mut state.document) {
                    Ok(()) => Ok((name, merged)),
                    Err(error) => Err(format!("保存失败：{error}")),
                }
            };
            match outcome {
                Ok((name, merged)) => {
                    rebuild(&window, &state.borrow());
                    window.set_status(
                        format!(
                            "已把 {merged} 条系统 hosts 记录合并到 Profile {name:?}；系统 hosts 未修改。"
                        )
                        .into(),
                    );
                }
                Err(message) => {
                    refresh(&window, &state);
                    window.set_status(message.into());
                }
            }
        });
    }
    {
        let state = Rc::clone(&state);
        let weak = window.as_weak();
        window.on_load_system_hosts(move || {
            if let Some(window) = weak.upgrade() {
                load_system_hosts(&window, &state);
            }
        });
    }
    {
        let state = Rc::clone(&state);
        let weak = window.as_weak();
        window.on_open_system_hosts(move || {
            if let Some(window) = weak.upgrade() {
                let text = match state.borrow().storage.read_hosts() {
                    Ok(content) => content,
                    Err(error) => format!("读取系统 hosts 失败：{error}"),
                };
                window.set_system_hosts_text(text.into());
            }
        });
    }
    {
        let weak = window.as_weak();
        window.on_install_helper(move || {
            if let Some(window) = weak.upgrade() {
                start_helper_registration(&window);
            }
        });
    }
}

fn finish_apply(window: &MainWindow, result: crabhosts_core::Result<crabhosts_core::ApplyReport>) {
    match result {
        Ok(report) if report.changed => {
            let message = format!(
                "已应用 {:?}。备份 ID：{}",
                report.source,
                report.backup_id.as_deref().unwrap_or("无")
            );
            window.set_status(message.clone().into());
            window.set_toast_text(message.into());
            window.set_toast_visible(true);
        }
        Ok(_) => window.set_status("没有需要应用的变更。".into()),
        Err(error) => window.set_status(
            format!("应用失败：{error}。请刷新确认系统 hosts 状态，不要自动重试。").into(),
        ),
    }
}

fn uses_direct_storage(storage: &Storage) -> bool {
    if !cfg!(target_os = "macos") {
        return true;
    }
    std::env::var_os(crabhosts_core::HOSTS_FILE_ENV).is_some()
        || storage.paths().hosts_file != std::path::Path::new("/etc/hosts")
}

fn configure_helper_status(window: &MainWindow, state: &Rc<RefCell<UiState>>) {
    if uses_direct_storage(&state.borrow().storage) {
        window.set_helper_state("direct".into());
        window.set_helper_status_text("隔离测试模式 · 直接写入临时 hosts".into());
        return;
    }

    #[cfg(target_os = "macos")]
    {
        if option_env!("CRABHOSTS_TEAM_ID").is_none() {
            window.set_helper_state("unsupported".into());
            window
                .set_helper_status_text("正式系统写入需要签名 .app；请使用 macOS 打包脚本".into());
            return;
        }
        window.set_helper_state("checking".into());
        window.set_helper_status_text("正在检查系统写入 Helper…".into());
        let weak = window.as_weak();
        std::thread::spawn(move || {
            let resolution = resolve_helper_status();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(window) = weak.upgrade() {
                    apply_helper_resolution(&window, resolution);
                }
            });
        });
    }
}

#[cfg(target_os = "macos")]
struct HelperResolution {
    state: &'static str,
    message: String,
    close_modal: bool,
}

#[cfg(target_os = "macos")]
fn resolve_helper_status() -> HelperResolution {
    match crabhosts_system_client::helper_status() {
        Ok(status)
            if status.protocol_version == crabhosts_core::HELPER_PROTOCOL_VERSION
                && status.helper_version == env!("CARGO_PKG_VERSION") =>
        {
            HelperResolution {
                state: "ready",
                message: format!("系统写入 Helper 已就绪 · v{}", status.helper_version),
                close_modal: false,
            }
        }
        Ok(_) => HelperResolution {
            state: "unavailable",
            message: "Helper 版本或协议不匹配，请更新 Helper".into(),
            close_modal: false,
        },
        Err(error) => {
            let registration = crabhosts_system_client::helper_registration_status().ok();
            use crabhosts_system_client::HelperRegistrationStatus as Status;
            let (state, message) = match registration {
                Some(Status::NotRegistered | Status::NotFound) => (
                    "not_installed",
                    "系统写入 Helper 未安装；安装时需管理员批准".to_owned(),
                ),
                Some(Status::RequiresApproval) => (
                    "requires_approval",
                    "请在 macOS 系统设置 → 通用 → 登录项与扩展中允许 CrabHosts Helper".to_owned(),
                ),
                Some(Status::Unsupported) => {
                    ("unsupported", "需要 macOS 13+ 的签名应用包".to_owned())
                }
                _ => ("unavailable", format!("无法连接系统写入 Helper：{error}")),
            };
            HelperResolution {
                state,
                message,
                close_modal: false,
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn apply_helper_resolution(window: &MainWindow, resolution: HelperResolution) {
    window.set_helper_busy(false);
    window.set_helper_state(resolution.state.into());
    window.set_helper_status_text(resolution.message.clone().into());
    if resolution.close_modal {
        window.set_show_helper_install(false);
    }
    window.set_status(resolution.message.into());
}

#[cfg(target_os = "macos")]
fn start_helper_registration(window: &MainWindow) {
    if window.get_helper_busy() {
        return;
    }
    let recheck_only = window.get_helper_state() == "requires_approval";
    window.set_helper_busy(true);
    window.set_helper_status_text(
        if recheck_only {
            "正在重新检查 Helper 授权状态…"
        } else {
            "正在注册 Helper，请按 macOS 提示完成授权…"
        }
        .into(),
    );
    let weak = window.as_weak();
    std::thread::spawn(move || {
        let registration = if recheck_only {
            Ok(crabhosts_system_client::HelperRegistrationStatus::RequiresApproval)
        } else {
            crabhosts_system_client::register_helper()
        };
        let resolution = match registration {
            Ok(crabhosts_system_client::HelperRegistrationStatus::RequiresApproval)
                if recheck_only =>
            {
                let mut resolution = resolve_helper_status();
                resolution.close_modal = resolution.state == "ready";
                resolution
            }
            Ok(crabhosts_system_client::HelperRegistrationStatus::Enabled) => {
                let mut resolution = resolve_helper_status();
                resolution.close_modal = resolution.state == "ready";
                resolution
            }
            Ok(crabhosts_system_client::HelperRegistrationStatus::RequiresApproval) => {
                HelperResolution {
                    state: "requires_approval",
                    message: "请在 macOS 系统设置中允许 CrabHosts Helper，然后点击“重新检查”"
                        .into(),
                    close_modal: false,
                }
            }
            Ok(status) => HelperResolution {
                state: "not_installed",
                message: format!("Helper 尚未启用（{}），请检查签名应用包", status.label()),
                close_modal: false,
            },
            Err(error) => HelperResolution {
                state: "unavailable",
                message: format!("Helper 安装失败：{error}"),
                close_modal: false,
            },
        };
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = weak.upgrade() {
                apply_helper_resolution(&window, resolution);
            }
        });
    });
}

#[cfg(not(target_os = "macos"))]
fn start_helper_registration(window: &MainWindow) {
    window.set_helper_state("unsupported".into());
    window.set_helper_status_text("当前平台不需要 macOS Helper".into());
}

fn refresh(window: &MainWindow, state: &Rc<RefCell<UiState>>) {
    let document = match state.borrow().storage.load() {
        Ok(document) => document,
        Err(error) => {
            window.set_status(format!("加载失败：{error}").into());
            return;
        }
    };
    {
        let mut guard = state.borrow_mut();
        let state = &mut *guard;
        if state
            .selected
            .as_ref()
            .is_some_and(|selected| document.profile(selected).is_err())
        {
            state.selected = None;
        }
        if state.selected.is_none() {
            state.selected = document
                .profiles
                .first()
                .map(|profile| profile.name.clone());
        }
        state.document = document;
    }
    rebuild(window, &state.borrow());
    if window.get_panel_mode() == 1 {
        load_system_hosts(window, state);
    }
    let status = {
        let state = state.borrow();
        format!(
            "已加载 {} 个 Profile；系统 hosts：{}",
            state.document.profiles.len(),
            state.storage.paths().hosts_file.display()
        )
    };
    window.set_status(status.into());
}

fn rebuild(window: &MainWindow, state: &UiState) {
    let profiles = state
        .document
        .profiles
        .iter()
        .map(|profile| ProfileData {
            name: profile.name.clone().into(),
            entry_count: profile.entries.len() as i32,
            selected: state.selected.as_deref() == Some(profile.name.as_str()),
        })
        .collect::<Vec<_>>();
    window.set_profiles(ModelRc::new(VecModel::from(profiles)));
    window.set_selected_name(state.selected.clone().unwrap_or_default().into());
    let entries = state
        .selected
        .as_ref()
        .and_then(|name| state.document.profile(name).ok())
        .map(|profile| {
            profile
                .entries
                .iter()
                .map(|entry| EntryData {
                    address: entry.ip.clone().into(),
                    hostnames: entry.hostnames.join(" ").into(),
                    enabled: entry.enabled,
                    comment: entry.comment.clone().unwrap_or_default().into(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    window.set_selected_enabled_count(entries.iter().filter(|entry| entry.enabled).count() as i32);
    window.set_entries(ModelRc::new(VecModel::from(entries)));
    if window.get_panel_mode() == 0 {
        update_panel_text(window, "");
    }
}

fn update_panel_text(window: &MainWindow, text: &str) {
    window.set_panel_text(text.into());
    let lines = text
        .lines()
        .enumerate()
        .map(|(index, line)| PreviewLineData {
            text: line.into(),
            line_number: index as i32 + 1,
            kind: if line.starts_with('+') && !line.starts_with("+++") {
                "add".into()
            } else if line.starts_with('-') && !line.starts_with("---") {
                "remove".into()
            } else {
                "context".into()
            },
        })
        .collect::<Vec<_>>();
    window.set_preview_lines(ModelRc::new(VecModel::from(lines)));
}

fn load_system_hosts(window: &MainWindow, state: &Rc<RefCell<UiState>>) {
    window.set_panel_label("系统 hosts 原文（只读）".into());
    match state.borrow().storage.read_hosts() {
        Ok(content) => {
            let text = if content.is_empty() {
                "（文件为空）".to_owned()
            } else {
                content
            };
            update_panel_text(window, &text);
        }
        Err(error) => update_panel_text(window, &format!("读取系统 hosts 失败：{error}")),
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use slint::Model;
    use std::fs;

    fn assert_laid_out(window: &MainWindow, label: &str) {
        let handle =
            i_slint_backend_testing::ElementHandle::find_by_accessible_label(window, label)
                .next()
                .unwrap_or_else(|| panic!("element {label:?} must exist"));
        let size = handle.size();
        assert!(
            size.width > 0.0 && size.height > 0.0,
            "element {label:?} must have a layout, got {size:?}"
        );
    }

    #[test]
    fn desktop_workflow_edits_profiles_without_touching_system_hosts() {
        i_slint_backend_testing::init_no_event_loop();
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config");
        let hosts = directory.path().join("hosts");
        let original = "127.0.0.1 localhost\n10.0.0.9 imported.local\n";
        fs::write(&hosts, original).unwrap();

        let storage = Storage::new(crabhosts_core::AppPaths::new(config, hosts.clone()));
        let window = MainWindow::new().unwrap();
        let state = Rc::new(RefCell::new(UiState {
            storage,
            document: Document::default(),
            selected: None,
        }));
        wire(&window, Rc::clone(&state));
        refresh(&window, &state);

        assert_eq!(window.get_profiles().row_count(), 0);
        assert_eq!(window.get_selected_name(), "");
        assert_laid_out(&window, "新建");
        assert_laid_out(&window, "新建 Profile");

        window.invoke_create_profile("dev".into(), true);
        assert_eq!(window.get_profiles().row_count(), 1);
        assert_eq!(window.get_selected_name(), "dev");
        assert_eq!(window.get_entries().row_count(), 2);
        assert_laid_out(&window, "添加记录");
        assert_laid_out(&window, "系统 hosts");
        assert_laid_out(&window, "应用到本机");
        {
            let document = state.borrow().storage.load().unwrap();
            assert_eq!(document.profiles.len(), 1);
            assert_eq!(document.profiles[0].name, "dev");
            assert_eq!(document.profiles[0].entries.len(), 2);
        }

        window.invoke_add_entry(
            "10.0.0.2".into(),
            "api.local web.local".into(),
            "local".into(),
        );
        assert_eq!(window.get_entries().row_count(), 3);
        {
            let document = state.borrow().storage.load().unwrap();
            let entry = document.profiles[0]
                .entries
                .iter()
                .find(|entry| entry.ip == "10.0.0.2")
                .expect("added entry must be persisted");
            assert_eq!(entry.hostnames, ["api.local", "web.local"]);
            assert_eq!(entry.comment.as_deref(), Some("local"));
        }

        let before_invalid = window.get_entries().row_count();
        window.invoke_add_entry("not-an-ip".into(), "bad.local".into(), "".into());
        assert_eq!(window.get_entries().row_count(), before_invalid);
        assert!(window.get_status().contains("添加失败"));

        window.invoke_toggle_entry(2);
        assert!(!window.get_entries().row_data(2).expect("entry row").enabled);
        assert!(
            !state.borrow().storage.load().unwrap().profiles[0].entries[2].enabled,
            "toggle must be persisted"
        );

        window.invoke_remove_entry(2);
        assert_eq!(window.get_entries().row_count(), 2);

        window.invoke_import_selected_from_system();
        assert_eq!(window.get_entries().row_count(), 2);
        assert!(window.get_status().contains("合并"));

        window.invoke_load_system_hosts();
        assert!(window.get_panel_text().contains("localhost"));
        assert_eq!(window.get_panel_label(), "系统 hosts 原文（只读）");

        window.invoke_open_system_hosts();
        assert!(window.get_system_hosts_text().contains("localhost"));
        assert!(window.get_system_hosts_path().ends_with("hosts"));

        window.invoke_preview_selected();
        assert_eq!(window.get_panel_label(), "差异预览（只读）");
        assert!(window.get_panel_text().contains("CrabHosts managed"));

        window.invoke_delete_profile();
        assert_eq!(window.get_profiles().row_count(), 0);
        assert_eq!(window.get_selected_name(), "");
        assert!(state.borrow().storage.load().unwrap().profiles.is_empty());
        assert_eq!(fs::read_to_string(&hosts).unwrap(), original);
    }
}
