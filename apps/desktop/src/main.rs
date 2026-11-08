use std::cell::RefCell;
use std::rc::Rc;

use crabhosts_core::{Document, Storage, apply_plan, plan_profile};
use slint::{ModelRc, VecModel};

slint::include_modules!();

struct UiState {
    storage: Storage,
    document: Document,
    selected: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let window = MainWindow::new()?;
    let state = Rc::new(RefCell::new(UiState {
        storage: Storage::from_env()?,
        document: Document::default(),
        selected: None,
    }));

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
            if let Some(window) = weak.upgrade() {
                let mut state = state.borrow_mut();
                let selected = state
                    .document
                    .profiles
                    .get(index as usize)
                    .map(|profile| profile.name.clone());
                state.selected = selected;
                update_selection(&window, &state);
            }
        });
    }
    {
        let state = Rc::clone(&state);
        let weak = window.as_weak();
        window.on_preview_selected(move || {
            if let Some(window) = weak.upgrade() {
                let state = state.borrow();
                let Some(profile) = state.selected.clone() else {
                    window.set_status("请先选择一个 Profile。".into());
                    return;
                };
                match plan_profile(&state.storage, &state.document, &profile) {
                    Ok(plan) => {
                        window.set_diff(plan.diff.into());
                        window.set_status(
                            if plan.changed {
                                format!("Profile {profile:?} 有待应用变更，尚未修改系统 hosts。")
                            } else {
                                "系统 hosts 已与该 Profile 一致。".to_owned()
                            }
                            .into(),
                        );
                    }
                    Err(error) => window.set_status(format!("预览失败：{error}").into()),
                }
            }
        });
    }
    {
        let state = Rc::clone(&state);
        let weak = window.as_weak();
        window.on_apply_selected(move || {
            if let Some(window) = weak.upgrade() {
                let result = {
                    let state = state.borrow();
                    let Some(profile) = state.selected.clone() else {
                        window.set_status("请先选择一个 Profile。".into());
                        return;
                    };
                    plan_profile(&state.storage, &state.document, &profile)
                        .and_then(|plan| apply_plan(&state.storage, &plan))
                };
                match result {
                    Ok(report) if report.changed => {
                        refresh(&window, &state);
                        window.set_status(
                            format!(
                                "已应用。备份 ID：{}",
                                report.backup_id.as_deref().unwrap_or("无")
                            )
                            .into(),
                        );
                    }
                    Ok(_) => window.set_status("没有需要应用的变更。".into()),
                    Err(error) => window.set_status(
                        format!("应用失败：{error}。请检查文件权限；CrabHosts 不会自动提权。")
                            .into(),
                    ),
                }
            }
        });
    }

    refresh(&window, &state);
    window.run()?;
    Ok(())
}

fn refresh(window: &MainWindow, state: &Rc<RefCell<UiState>>) {
    let result = state.borrow().storage.load();
    match result {
        Ok(document) => {
            let mut state = state.borrow_mut();
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
            update_selection(window, &state);
            window.set_status(
                format!(
                    "已加载 {} 个 Profile；系统 hosts：{}",
                    state.document.profiles.len(),
                    state.storage.paths().hosts_file.display()
                )
                .into(),
            );
        }
        Err(error) => window.set_status(format!("加载失败：{error}").into()),
    }
}

fn update_selection(window: &MainWindow, state: &UiState) {
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
    window.set_entries(ModelRc::new(VecModel::from(entries)));
}
