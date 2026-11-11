mod login_item;
mod preferences;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crabhosts_core::{
    Document, HostEntry, Profile, Storage, SystemPlan, parse_hosts, plan_profile,
};
use crabhosts_system_client::SystemClient;
use login_item::LoginItemStatus;
use preferences::{CloseAction, DesktopPreferences};
use slint::{ModelRc, VecModel};

slint::include_modules!();

type ApplyResult = crabhosts_core::Result<crabhosts_core::ApplyReport>;

struct UiState {
    storage: Storage,
    document: Document,
    selected: Option<String>,
    preferences: DesktopPreferences,
    preference_load_error: Option<String>,
    last_applied: Option<String>,
    login_item_status: LoginItemStatus,
    confirmed_plan: Option<SystemPlan>,
    pending_apply: Arc<Mutex<Option<ApplyResult>>>,
    tray: Option<slint::Weak<CrabHostsTray>>,
}

fn load_tray_images() -> (slint::Image, slint::Image) {
    #[cfg(target_os = "macos")]
    let asset = include_bytes!("../ui/assets/tray-macos.png").as_slice();
    #[cfg(not(target_os = "macos"))]
    let asset = include_bytes!("../ui/assets/app-icon.png").as_slice();
    let decoded = image::load_from_memory(asset)
        .expect("embedded Tray icon must decode")
        .to_rgba8();
    let (width, height) = decoded.dimensions();
    let to_image = |pixels: &image::RgbaImage| {
        slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
            pixels.as_raw(),
            width,
            height,
        ))
    };
    let normal = to_image(&decoded);
    #[cfg(target_os = "macos")]
    let on_dark = {
        let mut mask = decoded;
        for pixel in mask.pixels_mut() {
            pixel.0[..3].fill(255);
        }
        to_image(&mask)
    };
    #[cfg(not(target_os = "macos"))]
    let on_dark = normal.clone();
    (normal, on_dark)
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

#[cfg(target_os = "macos")]
thread_local! {
    static NATIVE_QUIT_WINDOW: RefCell<Option<slint::Weak<MainWindow>>> = const { RefCell::new(None) };
}

#[cfg(target_os = "macos")]
fn install_native_quit_guard(window: &MainWindow) -> Result<(), Box<dyn std::error::Error>> {
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::sel;
    use objc2_app_kit::NSApplicationTerminateReply;

    unsafe extern "C-unwind" fn should_terminate(
        _this: *mut AnyObject,
        _cmd: Sel,
        _app: *mut AnyObject,
    ) -> NSApplicationTerminateReply {
        let allowed = NATIVE_QUIT_WINDOW.with(|slot| {
            slot.borrow()
                .as_ref()
                .and_then(|window| window.upgrade())
                .is_none_or(|window| request_quit(&window))
        });
        if allowed {
            NSApplicationTerminateReply::TerminateNow
        } else {
            NSApplicationTerminateReply::TerminateCancel
        }
    }

    NATIVE_QUIT_WINDOW.with(|slot| *slot.borrow_mut() = Some(window.as_weak()));
    let selector = sel!(applicationShouldTerminate:);
    // Slint 1.18 uses this same winit delegate-class injection because winit 0.30 does not
    // allow replacing its NSApplication delegate. Never replace an existing termination policy.
    let Some(class) = AnyClass::get(c"WinitApplicationDelegate") else {
        return Err("macOS application delegate is unavailable; refusing an unguarded Quit".into());
    };
    if class.instance_method(selector).is_some() {
        return Err(
            "macOS already has an application termination policy; refusing to replace it".into(),
        );
    }
    let added = unsafe {
        // SAFETY: the signature is `NSUInteger applicationShouldTerminate:(id)app`, the selector
        // has one argument, and the C-unwind callback cannot unwind across the AppKit boundary.
        objc2::ffi::class_addMethod(
            (class as *const AnyClass).cast_mut(),
            selector,
            core::mem::transmute::<
                unsafe extern "C-unwind" fn(
                    *mut AnyObject,
                    Sel,
                    *mut AnyObject,
                ) -> NSApplicationTerminateReply,
                Imp,
            >(should_terminate),
            c"Q@:@".as_ptr(),
        )
    };
    if !added.as_bool() {
        return Err("macOS rejected the guarded application termination callback".into());
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let window = MainWindow::new()?;
    let tray = CrabHostsTray::new()?;
    // The native Tray is created on the first event-loop evaluation; provide decoded pixels now.
    let (normal, on_dark) = load_tray_images();
    tray.set_tray_icon(normal);
    tray.set_tray_icon_on_dark(on_dark);
    #[cfg(target_os = "macos")]
    {
        set_dock_icon();
        install_native_quit_guard(&window)?;
    }

    let storage = Storage::from_env()?;
    let mut preference_warning = None;
    let mut preference_load_error = None;
    let mut preferences = DesktopPreferences::load(storage.paths()).unwrap_or_else(|error| {
        preference_load_error = Some(error.to_string());
        preference_warning = Some(format!(
            "偏好加载失败，已使用默认值且禁止普通修改覆盖原文件；显式恢复默认可修复：{error}"
        ));
        DesktopPreferences::default()
    });
    let mut login_item_status = LoginItemStatus::NotInstalled;
    if !uses_direct_storage(&storage) {
        match login_item::status() {
            Ok(status) => {
                preferences.launch_at_login = status.is_enabled();
                login_item_status = status;
            }
            Err(_) => login_item_status = LoginItemStatus::Unavailable,
        }
    }
    let state = Rc::new(RefCell::new(UiState {
        storage,
        document: Document::default(),
        selected: None,
        preferences,
        preference_load_error,
        last_applied: None,
        login_item_status,
        confirmed_plan: None,
        pending_apply: Arc::new(Mutex::new(None)),
        tray: Some(tray.as_weak()),
    }));
    window.set_panel_label("差异预览（只读）".into());
    window.set_app_version(env!("CARGO_PKG_VERSION").into());
    window.set_backup_path(
        state
            .borrow()
            .storage
            .paths()
            .backups_dir
            .display()
            .to_string()
            .into(),
    );
    wire(&window, Rc::clone(&state));
    wire_tray(&window, &tray, Rc::clone(&state));
    configure_window_close(&window, Rc::clone(&state));

    refresh(&window, &state);
    if let Some(warning) = preference_warning {
        window.set_status(warning.into());
    }
    window.show()?;
    if state.borrow().preferences.show_tray {
        tray.show()?;
    } else {
        tray.hide()?;
    }
    slint::run_event_loop()?;
    window.hide()?;
    tray.hide()?;
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
    window.on_valid_entry(|ip, hosts, comment| draft_entry(&ip, &hosts, &comment).is_ok());
    configure_helper_status(window, &state);
    wire_preferences(window, Rc::clone(&state));

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
            if let Some(window) = weak.upgrade() {
                preview_selected(&window, &state);
            }
        });
    }
    {
        let state = Rc::clone(&state);
        let weak = window.as_weak();
        window.on_request_apply(move || {
            if let Some(window) = weak.upgrade() {
                request_apply(&window, &state);
            }
        });
    }
    {
        let state = Rc::clone(&state);
        window.on_apply_cancelled(move || {
            state.borrow_mut().confirmed_plan = None;
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
            let Some(plan) = confirmed_plan_for_apply(&window, &state) else {
                return;
            };

            if helper_state == "direct" {
                let result = {
                    let state = state.borrow();
                    SystemClient::new().apply_plan(&state.storage, &plan)
                };
                refresh(&window, &state);
                finish_apply(&window, &state, result);
            } else {
                let storage = state.borrow().storage.clone();
                window.set_helper_busy(true);
                window.set_status("正在通过系统 Helper 应用，请勿重复提交。".into());
                let weak = window.as_weak();
                let pending = state.borrow().pending_apply.clone();
                std::thread::spawn(move || {
                    let result = SystemClient::new().apply_plan(&storage, &plan);
                    *pending.lock().expect("apply result mutex poisoned") = Some(result);
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = weak.upgrade() {
                            window.invoke_apply_finished();
                        }
                    });
                });
            }
        });
    }
    {
        let state = Rc::clone(&state);
        let weak = window.as_weak();
        window.on_apply_finished(move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            window.set_helper_busy(false);
            window.invoke_refresh();
            let pending = state.borrow().pending_apply.clone();
            let result = pending
                .lock()
                .map(|mut result| result.take())
                .unwrap_or(None);
            if let Some(result) = result {
                finish_apply(&window, &state, result);
            } else {
                window.set_status("应用结果不可用；请刷新核对系统 hosts，不要自动重试。".into());
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
            let entry = match draft_entry(&ip, &hostnames, &comment) {
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
        let weak = window.as_weak();
        window.on_copy_preview(move || {
            if let Some(window) = weak.upgrade() {
                let text = window.get_panel_text().to_string();
                let copied =
                    arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text));
                match copied {
                    Ok(()) => toast(&window, "已复制预览内容".into()),
                    Err(error) => toast(&window, format!("复制预览内容失败：{error}")),
                }
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

fn wire_preferences(window: &MainWindow, state: Rc<RefCell<UiState>>) {
    {
        let state = Rc::clone(&state);
        let weak = window.as_weak();
        window.on_preference_changed(move |key, value| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let (previous, mut candidate) = {
                let state = state.borrow();
                (state.preferences.clone(), state.preferences.clone())
            };
            if let Err(error) = candidate.apply(&key, &value) {
                rebuild(&window, &state.borrow());
                notify(&window, format!("偏好未保存：{error}"));
                return;
            }
            commit_preferences(&window, &state, previous, candidate, false);
        });
    }
    {
        let state = Rc::clone(&state);
        let weak = window.as_weak();
        window.on_reset_preferences(move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let previous = state.borrow().preferences.clone();
            commit_preferences(
                &window,
                &state,
                previous,
                DesktopPreferences::default(),
                true,
            );
        });
    }
    {
        let state = Rc::clone(&state);
        let weak = window.as_weak();
        window.on_open_backups(move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            open_backups(&window, &state);
        });
    }
}

fn commit_preferences(
    window: &MainWindow,
    state: &Rc<RefCell<UiState>>,
    previous: DesktopPreferences,
    candidate: DesktopPreferences,
    explicit_reset: bool,
) {
    if !explicit_reset && state.borrow().preference_load_error.is_some() {
        rebuild(window, &state.borrow());
        notify(
            window,
            "偏好未保存：原偏好文件加载失败，普通修改不会覆盖它；请显式选择“恢复默认偏好设置”进行修复。".into(),
        );
        return;
    }
    let direct = uses_direct_storage(&state.borrow().storage);
    let login_changed = previous.launch_at_login != candidate.launch_at_login;
    if login_changed
        && !direct
        && let Err(error) = login_item::set_enabled(candidate.launch_at_login)
    {
        rebuild(window, &state.borrow());
        notify(window, format!("登录启动设置失败：{error}"));
        return;
    }

    let paths = state.borrow().storage.paths().clone();
    if let Err(error) = candidate.save(&paths) {
        let rollback = if login_changed && !direct {
            login_item::set_enabled(previous.launch_at_login).err()
        } else {
            None
        };
        let rollback_failed = rollback.is_some();
        let mut message = match rollback {
            Some(rollback) => {
                format!("偏好保存失败：{error}；登录启动回滚也失败：{rollback}。请检查系统状态。")
            }
            None => format!("偏好保存失败，已保持原设置：{error}"),
        };
        sync_login_item_status(window, state);
        if rollback_failed {
            let status = state.borrow().login_item_status;
            if reconcile_login_preference(&mut state.borrow_mut().preferences, status) {
                message.push_str("；开关已按实际系统状态校准");
            } else {
                message.push_str("；当前无法确认登录项实际状态");
            }
        }
        rebuild(window, &state.borrow());
        notify(window, message);
        return;
    }

    {
        let mut state = state.borrow_mut();
        state.preferences = candidate;
        state.preference_load_error = None;
    }
    sync_login_item_status(window, state);
    rebuild(window, &state.borrow());
    let message = if state.borrow().login_item_status == LoginItemStatus::RequiresApproval {
        "偏好已保存；登录启动仍需在 macOS 系统设置中批准。".to_owned()
    } else {
        "偏好已保存到此设备。".to_owned()
    };
    notify(window, message);
}

/// Reconciles the visible switch after a rollback failure when the OS state is known.
/// Returns false for an unavailable/unknown state, leaving the stored preference untouched.
fn reconcile_login_preference(
    preferences: &mut DesktopPreferences,
    status: LoginItemStatus,
) -> bool {
    if status == LoginItemStatus::Unavailable {
        return false;
    }
    preferences.launch_at_login = status.is_enabled();
    true
}

fn sync_login_item_status(window: &MainWindow, state: &Rc<RefCell<UiState>>) {
    let direct = uses_direct_storage(&state.borrow().storage);
    let status = if direct {
        if state.borrow().preferences.launch_at_login {
            LoginItemStatus::Enabled
        } else {
            LoginItemStatus::NotInstalled
        }
    } else {
        login_item::status().unwrap_or(LoginItemStatus::Unavailable)
    };
    state.borrow_mut().login_item_status = status;
    window.set_login_item_status(status.as_str().into());
}

fn toast(window: &MainWindow, message: String) {
    window.set_toast_text(message.into());
    window.set_toast_visible(true);
}

fn notify(window: &MainWindow, message: String) {
    window.set_status(message.clone().into());
    toast(window, message);
}

fn open_backups(window: &MainWindow, state: &Rc<RefCell<UiState>>) {
    let directory = state.borrow().storage.paths().backups_dir.clone();
    if let Err(error) = std::fs::create_dir_all(&directory) {
        notify(window, format!("无法创建备份目录：{error}"));
        return;
    }
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(&directory).status();
    #[cfg(not(target_os = "macos"))]
    let result = Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "opening the backup directory is currently supported only on macOS",
    ));
    match result {
        Ok(status) if status.success() => {
            window.set_status("已打开备份目录。".into());
        }
        Ok(status) => notify(window, format!("打开备份目录失败：{status}")),
        Err(error) => notify(window, format!("打开备份目录失败：{error}")),
    }
}

fn wire_tray(window: &MainWindow, tray: &CrabHostsTray, state: Rc<RefCell<UiState>>) {
    {
        let weak = window.as_weak();
        tray.on_open_main(move || {
            if let Some(window) = weak.upgrade() {
                show_main_window(&window);
            }
        });
    }
    {
        let weak = window.as_weak();
        tray.on_open_settings(move || {
            if let Some(window) = weak.upgrade() {
                show_main_window(&window);
                window.set_current_view("settings".into());
            }
        });
    }
    {
        let weak = window.as_weak();
        tray.on_profile_selected(move |index| {
            if let Some(window) = weak.upgrade() {
                window.invoke_profile_selected(index);
                window.set_current_view("profiles".into());
                show_main_window(&window);
            }
        });
    }
    {
        let weak = window.as_weak();
        tray.on_apply_current(move || {
            if let Some(window) = weak.upgrade() {
                request_apply_from_tray(&window, &state);
            }
        });
    }
    {
        let weak = window.as_weak();
        tray.on_quit(move || {
            if let Some(window) = weak.upgrade() {
                request_quit(&window);
            } else {
                let _ = slint::quit_event_loop();
            }
        });
    }
}

fn draft_entry(ip: &str, hosts: &str, comment: &str) -> crabhosts_core::Result<HostEntry> {
    let hosts = hosts.split_whitespace().map(str::to_owned).collect();
    let comment = comment.trim();
    HostEntry::new(
        ip.trim(),
        hosts,
        true,
        (!comment.is_empty()).then(|| comment.to_owned()),
    )
}

fn selected_plan(
    window: &MainWindow,
    state: &Rc<RefCell<UiState>>,
) -> Option<(String, crabhosts_core::Result<SystemPlan>)> {
    let state = state.borrow();
    let Some(profile) = state.selected.clone() else {
        window.set_status("请先选择一个 Profile。".into());
        return None;
    };
    let result = plan_profile(&state.storage, &state.document, &profile);
    Some((profile, result))
}

fn present_preview(
    window: &MainWindow,
    profile: &str,
    result: crabhosts_core::Result<SystemPlan>,
) -> Option<SystemPlan> {
    window.set_selected_in_sync(result.as_ref().is_ok_and(|plan| !plan.changed));
    window.set_panel_label("差异预览（只读）".into());
    match result {
        Ok(plan) => {
            update_panel_text(window, &plan.diff);
            window.set_status(
                if plan.changed {
                    format!("Profile {profile:?} 有待应用变更，尚未修改系统 hosts。")
                } else {
                    "系统 hosts 已与该 Profile 一致。".to_owned()
                }
                .into(),
            );
            Some(plan)
        }
        Err(error) => {
            update_panel_text(window, &format!("预览失败：{error}"));
            window.set_status(format!("预览失败：{error}").into());
            None
        }
    }
}

fn preview_selected(window: &MainWindow, state: &Rc<RefCell<UiState>>) -> bool {
    // A new preview revokes any older confirmation before presenting new content.
    window.set_show_confirm(false);
    state.borrow_mut().confirmed_plan = None;
    let Some((profile, result)) = selected_plan(window, state) else {
        return false;
    };
    present_preview(window, &profile, result).is_some()
}

fn request_apply(window: &MainWindow, state: &Rc<RefCell<UiState>>) {
    window.set_current_view("profiles".into());
    // Both the main window and Tray must approve a fresh, exact plan.
    window.set_show_confirm(false);
    state.borrow_mut().confirmed_plan = None;
    show_main_window(window);
    let Some((profile, result)) = selected_plan(window, state) else {
        return;
    };
    let Some(plan) = present_preview(window, &profile, result) else {
        return;
    };
    if !plan.changed {
        return;
    }
    state.borrow_mut().confirmed_plan = Some(plan);
    window.set_show_confirm(true);
}

fn request_apply_from_tray(window: &MainWindow, state: &Rc<RefCell<UiState>>) {
    request_apply(window, state);
}

fn confirmed_plan_for_apply(
    window: &MainWindow,
    state: &Rc<RefCell<UiState>>,
) -> Option<SystemPlan> {
    window.set_show_confirm(false);
    let approved = state.borrow_mut().confirmed_plan.take();
    let Some(approved) = approved else {
        window.set_status("请先通过“应用到本机”生成最新预览并确认。".into());
        return None;
    };
    let (profile, result) = selected_plan(window, state)?;
    match result {
        Ok(fresh) if fresh.plan_id == approved.plan_id => Some(approved),
        Ok(fresh) => {
            // Show the new state, but require another explicit request and confirmation for it.
            present_preview(window, &profile, Ok(fresh));
            window.set_status(
                "系统 hosts 或 Profile 在预览后已变化，已取消本次应用；请重新预览并确认。".into(),
            );
            None
        }
        Err(error) => {
            window.set_status(format!("生成应用计划失败：{error}").into());
            None
        }
    }
}

fn request_quit(window: &MainWindow) -> bool {
    if window.get_helper_busy() {
        show_main_window(window);
        notify(
            window,
            "系统写入仍在进行，完成前无法退出；请稍后核对结果。".to_owned(),
        );
        return false;
    }
    let _ = slint::quit_event_loop();
    true
}

#[allow(deprecated)] // activateIgnoringOtherApps is the compatible activation API for macOS 13.
fn show_main_window(window: &MainWindow) {
    window.window().set_minimized(false);
    if let Err(error) = window.show() {
        window.set_status(format!("无法显示主窗口：{error}").into());
    }
    #[cfg(target_os = "macos")]
    {
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSApplication;

        if let Some(main_thread) = MainThreadMarker::new() {
            NSApplication::sharedApplication(main_thread).activateIgnoringOtherApps(true);
        }
    }
}

fn configure_window_close(window: &MainWindow, state: Rc<RefCell<UiState>>) {
    let weak = window.as_weak();
    window.window().on_close_requested(move || {
        if state.borrow().preferences.show_tray
            && state.borrow().preferences.close_action == CloseAction::Tray
        {
            return slint::CloseRequestResponse::HideWindow;
        }
        if weak.upgrade().is_some_and(|window| !request_quit(&window)) {
            return slint::CloseRequestResponse::KeepWindowShown;
        }
        slint::CloseRequestResponse::HideWindow
    });
}

fn finish_apply(
    window: &MainWindow,
    state: &Rc<RefCell<UiState>>,
    result: crabhosts_core::Result<crabhosts_core::ApplyReport>,
) {
    match result {
        Ok(report) if report.changed => {
            let retention = state.borrow().preferences.backup_retention;
            let prune = state.borrow().storage.backups().prune(retention);
            if report.operation == "apply_profile" {
                state.borrow_mut().last_applied = Some(report.source.clone());
            }
            rebuild(window, &state.borrow());
            let mut message = format!(
                "已应用 {:?}。备份 ID：{}",
                report.source,
                report.backup_id.as_deref().unwrap_or("无")
            );
            match prune {
                Ok(0) => {}
                Ok(count) => message.push_str(&format!("；已清理 {count} 份最旧备份")),
                Err(error) => {
                    message.push_str(&format!("；系统 hosts 已更新，但旧备份清理失败：{error}"))
                }
            }
            notify(window, message);
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
    state.borrow_mut().confirmed_plan = None;
    window.set_show_confirm(false);
    let document = match state.borrow().storage.load() {
        Ok(document) => document,
        Err(error) => {
            window.set_selected_in_sync(false);
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

fn sync_preferences(window: &MainWindow, state: &UiState) {
    let preferences = &state.preferences;
    window.set_pref_launch_at_login(preferences.launch_at_login);
    window.set_pref_show_tray(preferences.show_tray);
    window.set_pref_close_action(preferences.close_action.as_str().into());
    window.set_pref_light(preferences.theme == preferences::ColorTheme::Light);
    window.set_pref_density(preferences.density.as_str().into());
    window.set_pref_show_profile_counts(preferences.show_profile_counts);
    window.set_pref_reduce_motion(preferences.reduce_motion);
    window.set_pref_backup_retention(preferences.backup_retention as i32);
    window.set_login_item_status(state.login_item_status.as_str().into());
    window.set_backup_path(
        state
            .storage
            .paths()
            .backups_dir
            .display()
            .to_string()
            .into(),
    );
    window.set_app_version(env!("CARGO_PKG_VERSION").into());
}

fn rebuild(window: &MainWindow, state: &UiState) {
    sync_preferences(window, state);
    window.set_selected_in_sync(state.selected.as_ref().is_some_and(|name| {
        plan_profile(&state.storage, &state.document, name).is_ok_and(|plan| !plan.changed)
    }));
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
    if let Some(tray) = state.tray.as_ref().and_then(|tray| tray.upgrade()) {
        tray.set_profiles(window.get_profiles());
        tray.set_selected_name(window.get_selected_name());
        tray.set_applied_name(state.last_applied.clone().unwrap_or_default().into());
        tray.set_show_tray(state.preferences.show_tray);
    }
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
    set_panel_content(window, text, false);
}

fn update_panel_raw_text(window: &MainWindow, text: &str) {
    set_panel_content(window, text, true);
}

fn set_panel_content(window: &MainWindow, text: &str, raw: bool) {
    window.set_panel_text(text.into());
    let mut added = 0;
    let mut removed = 0;
    let lines = text
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let kind = if line.starts_with('+') && !line.starts_with("+++") {
                added += 1;
                "add"
            } else if line.starts_with('-') && !line.starts_with("---") {
                removed += 1;
                "remove"
            } else {
                "context"
            };
            PreviewLineData {
                text: line.into(),
                line_number: index as i32 + 1,
                kind: kind.into(),
            }
        })
        .collect::<Vec<_>>();
    let line_count = lines.len();
    window.set_panel_line_count(line_count as i32);
    window.set_panel_stats(
        if raw {
            format!("共 {line_count} 行")
        } else {
            format!("共 {line_count} 行 · +{added} −{removed}")
        }
        .into(),
    );
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
            update_panel_raw_text(window, &text);
        }
        Err(error) => update_panel_raw_text(window, &format!("读取系统 hosts 失败：{error}")),
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

    fn activate(window: &MainWindow, label: &str) {
        let handle =
            i_slint_backend_testing::ElementHandle::find_by_accessible_label(window, label)
                .find(|handle| {
                    matches!(
                        handle.accessible_role(),
                        Some(i_slint_backend_testing::AccessibleRole::Button)
                            | Some(i_slint_backend_testing::AccessibleRole::Checkbox)
                    )
                })
                .unwrap_or_else(|| panic!("button or switch {label:?} must exist"));
        assert_ne!(
            handle.accessible_enabled(),
            Some(false),
            "element {label:?} must be enabled"
        );
        let size = handle.size();
        assert!(
            size.width > 0.0 && size.height > 0.0,
            "element {label:?} must have a layout, got {size:?}"
        );
        handle.invoke_accessible_default_action();
    }

    fn capture_if_requested(window: &MainWindow, name: &str) {
        let Ok(directory) = std::env::var("CRABHOSTS_UI_SCREENSHOT_DIR") else {
            return;
        };
        fs::create_dir_all(&directory).unwrap();
        window.set_toast_visible(false);
        // Start any pending transitions, then capture their settled colors.
        window.window().take_snapshot().unwrap();
        i_slint_backend_testing::testing_backend::mock_elapsed_time(200);
        let snapshot = window.window().take_snapshot().unwrap();
        let file = fs::File::create(format!("{directory}/{name}.png")).unwrap();
        let writer = std::io::BufWriter::new(file);
        let mut encoder = png::Encoder::new(writer, snapshot.width(), snapshot.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(snapshot.as_bytes())
            .unwrap();
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
            preferences: DesktopPreferences::default(),
            preference_load_error: None,
            last_applied: None,
            login_item_status: LoginItemStatus::NotInstalled,
            confirmed_plan: None,
            pending_apply: Arc::new(Mutex::new(None)),
            tray: None,
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

        assert!(!window.get_draft_valid());
        window.set_draft_ip("10.0.0.2".into());
        window.set_draft_hostnames("   ".into());
        assert!(!window.get_draft_valid());
        window.set_draft_hostnames("api.local web.local".into());
        assert!(window.get_draft_valid());
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

        // 导入必须经过确认：点击只打开确认层，取消不会改数据。
        assert!(!window.get_show_import_confirm());
        activate(&window, "从系统 hosts 导入");
        assert!(window.get_show_import_confirm());
        assert_laid_out(&window, "确认导入");
        assert_eq!(window.get_entries().row_count(), 2);
        assert_eq!(
            state.borrow().storage.load().unwrap().profiles[0]
                .entries
                .len(),
            2,
            "取消导入不得写入 Profile"
        );
        assert!(!window.get_status().contains("合并"));
        window.set_show_import_confirm(false);
        assert_eq!(window.get_entries().row_count(), 2);

        activate(&window, "从系统 hosts 导入");
        assert!(window.get_show_import_confirm());
        activate(&window, "确认导入");
        assert!(!window.get_show_import_confirm());
        assert_eq!(window.get_entries().row_count(), 2);
        assert!(window.get_status().contains("合并"));

        window.invoke_load_system_hosts();
        assert!(window.get_panel_text().contains("localhost"));
        assert_eq!(window.get_panel_label(), "系统 hosts 原文（只读）");
        assert!(window.get_panel_stats().starts_with("共 "));
        assert!(!window.get_panel_stats().contains('+'));
        assert_eq!(
            window.get_panel_line_count() as usize,
            window.get_panel_text().lines().count()
        );

        window
            .window()
            .set_size(slint::LogicalSize::new(1080.0, 1000.0));
        window.invoke_preview_selected();
        assert_eq!(window.get_panel_label(), "差异预览（只读）");
        assert!(window.get_panel_text().contains("CrabHosts managed"));
        assert!(window.get_panel_stats().contains("· +"));
        assert!(window.get_panel_line_count() > 0);
        assert_laid_out(&window, "全屏查看差异预览");
        assert_laid_out(&window, "全屏查看完整预览");

        request_apply_from_tray(&window, &state);
        assert!(window.get_show_confirm());
        assert_eq!(window.get_current_view(), "profiles");
        assert_eq!(fs::read_to_string(&hosts).unwrap(), original);
        assert!(state.borrow().confirmed_plan.is_some());

        let concurrent = format!("{original}10.0.0.99 concurrent.local\n");
        fs::write(&hosts, &concurrent).unwrap();
        window.invoke_apply_selected();
        assert!(!window.get_show_confirm());
        assert!(window.get_status().contains("预览后已变化"));
        assert_eq!(fs::read_to_string(&hosts).unwrap(), concurrent);
        assert!(state.borrow().confirmed_plan.is_none());
        assert!(state.borrow().storage.backups().list().unwrap().is_empty());
        fs::write(&hosts, original).unwrap();

        window.invoke_request_apply();
        assert!(window.get_show_confirm());
        assert!(state.borrow().confirmed_plan.is_some());
        window.set_show_confirm(false);
        window.invoke_apply_cancelled();
        assert!(state.borrow().confirmed_plan.is_none());

        let moved_hosts = directory.path().join("hosts-unavailable");
        fs::rename(&hosts, &moved_hosts).unwrap();
        request_apply_from_tray(&window, &state);
        assert!(!window.get_show_confirm());
        assert!(window.get_status().contains("预览失败"));
        fs::rename(moved_hosts, &hosts).unwrap();
        let synced = {
            let state = state.borrow();
            plan_profile(&state.storage, &state.document, "dev")
                .unwrap()
                .proposed
        };
        fs::write(&hosts, synced).unwrap();
        window.invoke_refresh();
        assert!(window.get_selected_in_sync());
        request_apply_from_tray(&window, &state);
        assert!(!window.get_show_confirm());
        fs::write(&hosts, original).unwrap();
        window.invoke_preview_selected();
        assert!(!window.get_selected_in_sync());

        window.invoke_delete_profile();
        assert_eq!(window.get_profiles().row_count(), 0);
        assert_eq!(window.get_selected_name(), "");
        assert!(state.borrow().storage.load().unwrap().profiles.is_empty());
        assert_eq!(fs::read_to_string(&hosts).unwrap(), original);
    }

    #[test]
    fn settings_are_persistent_linked_and_isolated_from_business_data() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions {
                mock_time: true,
                threading: false,
                renderer_name: Some("software".into()),
            },
        )))
        .expect("software testing platform must initialize");
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config");
        let hosts = directory.path().join("hosts");
        let original = "127.0.0.1 localhost\n";
        fs::write(&hosts, original).unwrap();
        let storage = Storage::new(crabhosts_core::AppPaths::new(config, hosts.clone()));
        let window = MainWindow::new().unwrap();
        window.show().unwrap();
        let state = Rc::new(RefCell::new(UiState {
            storage,
            document: Document::default(),
            selected: None,
            preferences: DesktopPreferences::default(),
            preference_load_error: None,
            last_applied: None,
            login_item_status: LoginItemStatus::NotInstalled,
            confirmed_plan: None,
            pending_apply: Arc::new(Mutex::new(None)),
            tray: None,
        }));
        wire(&window, Rc::clone(&state));
        refresh(&window, &state);
        window.invoke_create_profile("dev".into(), false);

        window.set_current_view("settings".into());
        assert_laid_out(&window, "返回 Profiles");
        assert_laid_out(&window, "设置-通用");
        assert_laid_out(&window, "设置-外观");
        assert_laid_out(&window, "设置-备份与安全");
        assert_laid_out(&window, "设置-关于");
        capture_if_requested(&window, "settings-general");

        activate(&window, "登录时启动 CrabHosts");
        assert!(
            window.get_pref_launch_at_login(),
            "login preference did not change; status: {}",
            window.get_status()
        );
        assert_eq!(window.get_login_item_status(), "enabled");
        activate(&window, "显示菜单栏图标");
        assert!(!window.get_pref_show_tray());
        assert_eq!(window.get_pref_close_action(), "quit");
        activate(&window, "关闭主窗口时-隐藏到菜单栏");
        assert!(window.get_pref_show_tray());
        assert_eq!(window.get_pref_close_action(), "tray");

        activate(&window, "设置-外观");
        capture_if_requested(&window, "settings-appearance");
        activate(&window, "主题-浅色");
        assert!(window.get_pref_light());
        capture_if_requested(&window, "settings-light");
        activate(&window, "主题-深色");
        assert!(!window.get_pref_light());
        activate(&window, "主题-浅色");
        activate(&window, "界面密度-紧凑");
        assert_eq!(window.get_pref_density(), "compact");
        activate(&window, "在侧栏显示记录数");
        assert!(!window.get_pref_show_profile_counts());
        activate(&window, "减少动态效果");
        assert!(window.get_pref_reduce_motion());

        activate(&window, "设置-备份与安全");
        capture_if_requested(&window, "settings-backup");
        activate(&window, "备份保留-50");
        assert_eq!(window.get_pref_backup_retention(), 50);
        assert_laid_out(&window, "在 Finder 中显示");
        assert!(window.get_backup_path().ends_with("backups"));

        let saved = DesktopPreferences::load(state.borrow().storage.paths()).unwrap();
        assert_eq!(saved.theme, preferences::ColorTheme::Light);
        assert!(saved.launch_at_login);
        assert!(saved.show_tray);
        assert_eq!(saved.density, preferences::Density::Compact);
        assert!(!saved.show_profile_counts);
        assert!(saved.reduce_motion);
        assert_eq!(saved.backup_retention, 50);

        window.invoke_preference_changed("backup-retention".into(), "7".into());
        window.invoke_preference_changed("density".into(), "invalid".into());
        assert_eq!(window.get_pref_backup_retention(), 50);
        assert_eq!(window.get_pref_density(), "compact");
        assert_eq!(
            DesktopPreferences::load(state.borrow().storage.paths())
                .unwrap()
                .backup_retention,
            50
        );

        activate(&window, "设置-关于");
        capture_if_requested(&window, "settings-about");
        assert_eq!(window.get_app_version(), env!("CARGO_PKG_VERSION"));
        activate(&window, "查看安全说明");
        assert!(window.get_show_safety());
        window.set_show_safety(false);
        activate(&window, "恢复默认偏好设置");
        assert!(window.get_show_reset_preferences());
        activate(&window, "恢复默认");
        assert_eq!(state.borrow().preferences, DesktopPreferences::default());
        assert_eq!(
            DesktopPreferences::load(state.borrow().storage.paths()).unwrap(),
            DesktopPreferences::default()
        );

        window.set_helper_busy(true);
        assert!(!request_quit(&window));
        assert!(window.get_status().contains("完成前无法退出"));
        window.set_helper_busy(false);
        let mut reconciled = DesktopPreferences::default();
        assert!(reconcile_login_preference(
            &mut reconciled,
            LoginItemStatus::RequiresApproval
        ));
        assert!(reconciled.launch_at_login);
        assert!(!reconcile_login_preference(
            &mut reconciled,
            LoginItemStatus::Unavailable
        ));
        assert!(reconciled.launch_at_login);

        activate(&window, "返回 Profiles");
        assert_eq!(window.get_current_view(), "profiles");
        assert_eq!(window.get_profiles().row_count(), 1);
        assert_eq!(state.borrow().storage.load().unwrap().profiles.len(), 1);
        assert_eq!(fs::read_to_string(&hosts).unwrap(), original);

        window.invoke_add_entry(
            "127.0.0.1".into(),
            "api.local web.local".into(),
            "本地开发".into(),
        );
        assert!(!window.get_pref_light());
        capture_if_requested(&window, "profiles-main");
        window
            .window()
            .set_size(slint::LogicalSize::new(1080.0, 1000.0));

        window.invoke_preview_selected();
        assert_laid_out(&window, "全屏查看差异预览");
        assert_laid_out(&window, "全屏查看完整预览");
        capture_if_requested(&window, "profiles-preview-inline");
        activate(&window, "从系统 hosts 导入");
        assert!(window.get_show_import_confirm());
        capture_if_requested(&window, "import-confirm");
        window.set_show_import_confirm(false);
        activate(&window, "全屏查看差异预览");
        assert!(window.get_show_preview_viewer());
        assert_laid_out(&window, "关闭全屏预览");
        assert_laid_out(&window, "自动换行");
        assert_laid_out(&window, "显示行号");
        assert_laid_out(&window, "复制预览内容");
        capture_if_requested(&window, "preview-viewer-diff");
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                text: slint::platform::Key::Escape.into(),
            });
        assert!(
            !window.get_show_preview_viewer(),
            "Escape must close the fullscreen preview"
        );
        activate(&window, "全屏查看完整预览");
        assert!(window.get_show_preview_viewer());
        window.set_preview_wrap(true);
        window.set_preview_gutter(false);
        capture_if_requested(&window, "preview-viewer-wrap");
        window.set_preview_wrap(false);
        window.set_preview_gutter(true);
        window.set_panel_mode(1);
        window.invoke_load_system_hosts();
        assert!(window.get_panel_stats().starts_with("共 "));
        assert!(!window.get_panel_stats().contains('+'));
        assert!(window.get_panel_text().contains("localhost"));
        capture_if_requested(&window, "preview-viewer-system");

        // Synthetic long preview for scroll/wrap visuals only; the real plan is restored below.
        let long_lines = (1..=40)
            .map(|index| {
                let aliases = if index == 3 {
                    "alias-a.company.local alias-b.company.local alias-c.company.local alias-d.company.local alias-e.company.local alias-f.company.local alias-g.company.local alias-h.company.local"
                } else {
                    "short.local"
                };
                PreviewLineData {
                    text: format!("+10.0.0.{index} host-{index}.internal.company.local {aliases}").into(),
                    line_number: index,
                    kind: if index % 8 == 0 { "remove".into() } else { "add".into() },
                }
            })
            .collect::<Vec<_>>();
        window.set_panel_mode(0);
        window.set_panel_stats(format!("共 40 行 · +35 −5").into());
        window.set_panel_line_count(40);
        window.set_preview_lines(ModelRc::new(VecModel::from(long_lines)));
        window.set_preview_wrap(false);
        capture_if_requested(&window, "preview-viewer-long");
        window.set_preview_wrap(true);
        capture_if_requested(&window, "preview-viewer-long-wrap");
        window.set_show_preview_viewer(false);
        assert!(!window.get_show_preview_viewer());
        window.set_preview_wrap(false);
        window.set_panel_mode(0);
        window.invoke_preview_selected();
        assert_eq!(fs::read_to_string(&hosts).unwrap(), original);

        window
            .window()
            .set_size(slint::LogicalSize::new(900.0, 620.0));
        window.set_show_preview_viewer(true);
        capture_if_requested(&window, "preview-viewer-compact");
        assert_laid_out(&window, "关闭全屏预览");
        window.set_show_preview_viewer(false);
        capture_if_requested(&window, "profiles-compact");

        window
            .window()
            .set_size(slint::LogicalSize::new(1080.0, 760.0));
        window.invoke_preference_changed("theme".into(), "light".into());
        capture_if_requested(&window, "profiles-light");
        window.invoke_preview_selected();
        window.set_show_preview_viewer(true);
        capture_if_requested(&window, "preview-viewer-light");
        window.set_show_preview_viewer(false);
        window.set_current_view("settings".into());
        window.set_settings_section(0);
        window
            .window()
            .set_size(slint::LogicalSize::new(900.0, 620.0));
        assert_laid_out(&window, "关闭主窗口时-隐藏到菜单栏");
        capture_if_requested(&window, "settings-compact");
        assert_eq!(fs::read_to_string(&hosts).unwrap(), original);

        let preferences_path = DesktopPreferences::path(state.borrow().storage.paths());
        let broken = "{\"schema_version\":99,\"theme\":\"light\"}\n";
        fs::write(&preferences_path, broken).unwrap();
        assert!(DesktopPreferences::load(state.borrow().storage.paths()).is_err());
        {
            let mut state = state.borrow_mut();
            state.preferences = DesktopPreferences::default();
            state.preference_load_error = Some("invalid fixture preferences".into());
        }
        window.invoke_preference_changed("density".into(), "compact".into());
        assert_eq!(fs::read_to_string(&preferences_path).unwrap(), broken);
        assert!(window.get_status().contains("普通修改不会覆盖"));
        window.invoke_reset_preferences();
        assert!(state.borrow().preference_load_error.is_none());
        assert_eq!(
            DesktopPreferences::load(state.borrow().storage.paths()).unwrap(),
            DesktopPreferences::default()
        );
        assert_eq!(fs::read_to_string(&hosts).unwrap(), original);
    }
}

#[cfg(test)]
mod tray_image_tests {
    #[test]
    fn embedded_tray_icon_decodes_to_rgba_pixels() {
        let (image, _on_dark) = super::load_tray_images();
        let pixels = image
            .to_rgba8()
            .expect("embedded Tray icon must expose RGBA pixels");
        assert!(pixels.width() > 0 && pixels.height() > 0);
        assert!(!pixels.as_bytes().is_empty());
        #[cfg(target_os = "macos")]
        {
            let inverse = _on_dark.to_rgba8().unwrap();
            assert_eq!((pixels.width(), pixels.height()), (48, 36));
            assert!(pixels.as_slice().iter().any(|pixel| pixel.a == 255));
            assert!(pixels.as_slice().iter().any(|pixel| pixel.a == 0));
            for (black, white) in pixels.as_slice().iter().zip(inverse.as_slice()) {
                assert_eq!(black.a, white.a);
                if black.a > 0 {
                    assert_eq!((black.r, black.g, black.b), (0, 0, 0));
                    assert_eq!((white.r, white.g, white.b), (255, 255, 255));
                }
            }
        }
    }
}
