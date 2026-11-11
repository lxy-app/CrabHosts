# 桌面端 Tray、设置与本地偏好

## 范围与边界

- 真实工程入口为 `apps/desktop`；`prototypes/desktop-ui` 仅保留设计评审参考。
- Tray 切换 Profile 只改变当前编辑对象；“应用…”只显示主窗口并打开确认层，不直接调用系统写入。
- Helper 安装、更新和授权仍由既有侧栏流程显式发起，不在设置页提供密码或提权入口。
- 桌面偏好是设备本地状态，不加入 `Document`，不参与同步。

## 组件

- `apps/desktop/ui/settings.slint`：通用、外观、备份与安全、关于四个分类；控件只提交期望的新值。
- `apps/desktop/ui/theme.slint`：共享浅色、深色调色板和减少动态效果设置。外观设置即时切换主题，旧偏好默认使用深色。
- `CrabHostsTray`：使用 Slint 1.18 的 `SystemTrayIcon`，由 macOS AppKit 创建 `NSStatusItem` 和原生菜单。
- `apps/desktop/src/preferences.rs`：版本化 `DesktopPreferences`、校验、联动和原子持久化。
- `apps/desktop/src/login_item.rs`：使用 `SMAppService.mainAppService` 显式注册/注销主应用登录项。

## 偏好事务

文件位于 `AppPaths.config_dir/preferences.json`，schema 版本为 1。缺失文件使用默认值；损坏 JSON、未知字段、未知版本、非法值和符号链接路径均报错，不以默认值覆盖原文件。加载失败后，普通偏好修改会被阻止且不得产生系统副作用；只有用户在确认层显式选择“恢复默认”才允许修复写回。

每次更新按以下顺序执行：

1. 克隆当前偏好，在候选副本上校验新值和联动关系。
2. 登录启动变化时，显式调用系统 API；隔离 fixture 模式不调用真实系统服务。
3. 使用核心 `atomic_write` 保存候选偏好。
4. 保存成功后替换 `UiState` 并同步 Slint/Tray；失败则保留旧状态，并对已发生的登录项副作用做 best-effort 回滚。

隐藏 Tray 会把关闭窗口行为联动为退出；重新选择“隐藏到菜单栏”会恢复 Tray。`RequiresApproval` 与 `Enabled` 分开显示，前者表示仍需系统批准。

## 窗口生命周期

- 主窗口与 Tray 均由 `main` 持有到事件循环结束。
- 关闭窗口时，`CloseAction::Tray` 且 Tray 可见则隐藏窗口；否则显式退出事件循环。
- Tray 的打开、设置和 Profile 切换会取消最小化、显示并激活主窗口。
- macOS 原生 Cmd+Q 和应用菜单退出通过 winit delegate 的 `applicationShouldTerminate:` 进入与 Tray 退出相同的 Helper 忙碌保护；不替换已有终止策略。
- 可见 Tray 保持应用事件循环；隐藏 Tray 不允许形成无界面的常驻进程。

## 备份保留

`BackupStore::prune` 只接受 CrabHosts 受管 ID，先校验整批待清理项，再清理最旧的 `.json` / `.hosts` 对。清理在桌面端成功应用后执行，不在修改设置时立即删除文件；失败会明确提示，不否定已经完成的 hosts 写入。

## 验证

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p crabhosts-desktop --features ui-tests
```

UI 测试使用临时 `AppPaths` 和 hosts，覆盖四页导航、真实控件动作、偏好重载、联动、非法输入回滚、损坏偏好原始字节保护、显式恢复默认、确认计划绑定，以及预览后 hosts 变化拒绝写入。设置 `CRABHOSTS_UI_SCREENSHOT_DIR` 可输出四个设置分类的 Slint 快照。

以下必须在真实 macOS 环境另外验收，不能由无头测试替代：

- 签名 `.app` 中 Tray 图标、动态菜单和主窗口恢复。
- 两种关闭策略、隐藏 Tray 后退出、空 Profile 与退出行为；另需覆盖 Helper 忙碌时的原生 Cmd+Q／应用菜单退出，以及最小化后经 Tray 恢复。
- 登录项注册/注销、`RequiresApproval` 的系统设置流程。
- Finder 打开备份目录，以及超过保留上限后的真实备份清理。
- 既有 Helper 的真实写入、macOS 13 和 universal 包边界不变；当前 arm64 包的公证结果不替代这些平台与生命周期验收。

## 2026-11-09 实装验证记录

- 格式、严格 Clippy、全 workspace 测试、桌面 UI 测试与脚本语法检查通过。
- 四个设置分类、主工作区和 900×620 紧凑窗口已完成 Slint 软件渲染目检。
- 独立审查后补强：备份元数据文件名与受管 ID 绑定、回滚失败后的登录项状态校准、Helper 忙碌时禁止退出、主窗口与 Tray 确认均绑定最新 `plan_id`、预览后状态变化拒绝写入、损坏偏好禁止普通修改覆盖；均有回归覆盖。原生 Cmd+Q 保护已接入，其实际快捷键与菜单行为仍列入下方原生验收边界。
- 原生诊断发现 Tray 首次事件循环求值必须具备可转换图标；现改为启动时先解码嵌入 PNG 为 RGBA 并赋值，再进入事件循环，附独立像素解码回归。
- 隔离数据的真实 macOS 主窗口已启动并完成视觉检查，未接触真实 hosts。
- 原生验收已由用户确认：螃蟹图标可见，原生菜单可打开；设置入口、关闭到 Tray、恢复主窗口、“应用… → 确认层 → 取消”和退出均正常。
- macOS 使用专用 48×36 黑白 Tray 资源，按菜单栏明暗外观选择黑色或白色像素；启动时提前解码 RGBA，并有资源与透明度回归。
- 2026-11-10 当时工作区快照的 arm64 包已完成 Developer ID 签名、Apple 公证（`Accepted`）、票据附加、Gatekeeper 检查及最终 ZIP 解压复验；`dist/` 产物保持忽略，不纳入源码提交。后续代码变更必须重新打包和公证，不沿用该快照的验收结论。
