# 桌面端 Tray、设置与本地偏好

## 范围与边界

- 真实工程入口为 `apps/desktop`；`prototypes/desktop-ui` 仅保留设计评审参考。
- Tray 切换 Profile 只改变当前编辑对象；“应用…”只显示主窗口并打开完整启用组合的确认层，不直接调用系统写入。
- Helper 安装、更新和授权仍由既有侧栏流程显式发起，不在设置页提供密码或提权入口。
- 桌面偏好与 Profile 启用集合都是设备本地状态，不加入 `Document`，不参与同步。

## Profile 组合启用

- `selected` 只表示当前编辑对象；`activation.json` 中的有序 `active_profile_ids` 才决定系统合成内容。Profile 名称仅用于展示，重命名不改变启用身份。
- 侧栏开关可同时启用多个 Profile。开关通过 `activation.lock` 跨进程事务原子保存本机启用状态，不修改 `Document` revision 或系统 hosts；桌面端与 CLI 的读改写不会互相覆盖。取消应用确认不会回滚开关，未应用的差异明确显示为待应用。
- 主窗口与 Tray 的预览、确认、plan 绑定和应用都使用完整 active ID 集合。空集合是合法计划，只移除 CrabHosts 托管区块并保留非托管内容。
- 同一 hostname 对应不同 IP 时预览报错并列出冲突来源，不静默按顺序覆盖；相同 IP 的重复映射只渲染一次。
- activation 缺失时从现有托管区块的 Profile ID 分节恢复；旧版单 Profile 名称区块也可迁移。activation 损坏、缺字段、含未知字段或版本不支持时禁止应用，不能按空集合降级写入。

## 组件

- `apps/desktop/ui/settings.slint`：通用、外观、备份与安全、关于四个分类；控件只提交期望的新值。
- `apps/desktop/ui/theme.slint`：共享浅色、深色调色板和减少动态效果设置。外观设置提供“跟随系统 / 浅色 / 深色”三态主题，以及“跟随系统 / 开启 / 关闭”三态减少动态效果；默认均跟随系统。主题的系统模式读取 `NSApplication.effectiveAppearance` 并通过 KVO 监听 macOS 外观变化，减少动态效果读取 `NSWorkspace.accessibilityDisplayShouldReduceMotion` 并监听辅助功能显示选项通知。旧版布尔 `reduce_motion` 偏好仍可读取（`true` 映射为开启，`false` 映射为关闭）。
- `CrabHostsTray`：使用 Slint 1.18 的 `SystemTrayIcon`，由 macOS AppKit 创建 `NSStatusItem` 和原生菜单。
- `apps/desktop/src/preferences.rs`：版本化 `DesktopPreferences`、校验、联动和原子持久化。
- `apps/desktop/src/login_item.rs`：使用 `SMAppService.mainAppService` 显式注册/注销主应用登录项。
- `apps/desktop/src/cli_install.rs`：签名应用包内 `crab` 的发现、用户级符号链接安装与 zsh PATH 托管块。

## 偏好事务

文件位于 `AppPaths.config_dir/preferences.json`，schema 版本为 1。缺失文件使用默认值；损坏 JSON、未知字段、未知版本、非法值和符号链接路径均报错，不以默认值覆盖原文件。加载失败后，普通偏好修改会被阻止且不得产生系统副作用；只有用户在确认层显式选择“恢复默认”才允许修复写回。

每次更新按以下顺序执行：

1. 克隆当前偏好，在候选副本上校验新值和联动关系。
2. 登录启动变化时，显式调用系统 API；隔离 fixture 模式不调用真实系统服务。
3. 使用核心 `atomic_write` 保存候选偏好。
4. 保存成功后替换 `UiState` 并同步 Slint/Tray；失败则保留旧状态，并对已发生的登录项副作用做 best-effort 回滚。

隐藏 Tray 会把关闭窗口行为联动为退出；重新选择“隐藏到菜单栏”会恢复 Tray。`RequiresApproval` 与 `Enabled` 分开显示，前者表示仍需系统批准。

## CLI PATH 安装

「设置 → 通用 → crab 命令行工具」提供显式“安装到 PATH”按钮，仅当当前进程来自包含 `Contents/SharedSupport/crab` 的签名应用包时可用。安装器在 `~/.local/bin` 创建指向当前应用内 CLI 的绝对路径符号链接，并在 `~/.zprofile` 写入独立 CrabHosts 托管块，使后续 macOS 默认 zsh 登录 shell 把 `~/.local/bin` 加入 PATH。不会调用 `sudo`、AppleScript 或 root Helper，也不修改系统 hosts。

已有终端和已经运行的 Harness/GUI 进程不会自动刷新 PATH，成功后需打开新的终端窗口。链接依赖当前 `.app` 位置，建议先把应用固定到 `/Applications`；移动或删除应用会使旧链接失效。安装不覆盖同名普通文件、目录或指向其他目标的链接，也不跟随 `.zprofile` 符号链接写入。托管 marker 按整行匹配，残缺、重复或倒序块会报错并保留原文件。

PATH 配置先原子保存，再创建链接；若链接创建失败，错误会明确说明 PATH 已保存，重试保持幂等，不通过删除链接回滚。此安装状态不写入 `DesktopPreferences`，恢复默认偏好不会卸载 CLI。

## 窗口生命周期

- 主窗口与 Tray 均由 `main` 持有到事件循环结束。
- 关闭窗口时，`CloseAction::Tray` 且 Tray 可见则隐藏窗口；否则显式退出事件循环。
- Tray 的打开、设置和 Profile 切换会取消最小化、显示并激活主窗口。
- macOS 原生 Cmd+Q 和应用菜单退出通过 winit delegate 的 `applicationShouldTerminate:` 进入与 Tray 退出相同的 Helper 忙碌保护；不替换已有终止策略。
- 可见 Tray 保持应用事件循环；隐藏 Tray 不允许形成无界面的常驻进程。

## 删除与清理确认

- 所有危险操作复用 `ModalFrame`，提供关闭按钮、取消、`Esc` 和遮罩关闭路径；弹窗打开期间，主窗口菜单、Tray 入口及 Rust 业务写回调共同阻止后台流程叠开或写入。
- Profile 删除冻结打开时的目标名称、记录数和启用数。只有输入完全一致（不 trim、区分大小写）的名称才能通过按钮或 Enter 提交，Rust 回调再次校验名称并按目标删除，而不是读取已可能变化的当前选择。保存使用候选 `Document`，失败不破坏内存中的原配置。
- 单条记录确认展示 Profile、IP、Hostname 与启用状态；批量确认列出所选明细，并在弹窗期间冻结复选、全选和刷新。实际删除继续按稳定 entry ID 执行。Profile 删除会同时清理本机 active ID 引用，记录删除只修改所属 Profile；两者都不立即修改系统 hosts，也不删除已有备份。删除已启用 Profile 后，其旧贡献要等到用户重新预览并确认应用剩余组合时才移除。
- 降低备份保留上限时，Rust 暂存原偏好与完整候选偏好并打开确认层；取消清空候选，确认时再次比对当前偏好，过期候选直接丢弃。确认后仍通过原有校验、原子保存与失败回滚事务，不能绕过损坏偏好防覆盖保护。
- 恢复默认偏好若会把保留上限从 50 降为 20，现有确认层会升级为明确的备份清理确认；未降低时保持普通重置确认。

## 备份保留

`BackupStore::prune` 只接受 CrabHosts 受管 ID，先校验整批待清理项，再清理最旧的 `.json` / `.hosts` 对。清理在桌面端成功应用后执行，不在修改设置时立即删除文件；失败会明确提示，不否定已经完成的 hosts 写入。

## 验证

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p crabhosts-desktop --features ui-tests
```

UI 测试使用临时 `AppPaths` 和 hosts，覆盖四页导航、真实控件动作、偏好重载、联动、非法输入回滚、损坏偏好原始字节保护、显式恢复默认、确认计划绑定，以及预览后 hosts 变化拒绝写入。删除回归通过真实可访问控件覆盖名称错误、按钮禁用、Enter/Esc、单条/批量删除、目标绑定、备份下调取消与确认、陈旧候选、保存失败和危险重置；同时断言系统 hosts 与备份内容不变。设置 `CRABHOSTS_UI_SCREENSHOT_DIR` 可输出设置分类及删除/清理确认层的 Slint 快照。

CLI 安装器单元测试只使用临时 home 和应用包 fixture，覆盖首次/重复安装、已有普通文件及异目标链接冲突、完整/残缺/重复/倒序 marker、CRLF、`.zprofile` 符号链接拒绝，以及链接失败后的明确部分状态与幂等重试；不会读取或改写真实用户配置。

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

## 2026-11-14 删除与清理确认实装验证

- 已实装 Profile 名称复核、单条/批量记录明细、备份保留下调确认、危险重置文案、目标绑定与完整候选偏好事务；保留最后 Profile 可为空及成功应用后才 prune 的既有语义。
- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 全部通过。
- `cargo test -p crabhosts-desktop --features ui-tests` 共 11 项测试全部通过，覆盖真实控件动作、名称错误与按钮禁用、Enter/Esc、目标绑定、取消/关闭、未确认不保存、陈旧候选、保存失败、损坏偏好保护，以及系统 hosts 和备份不变。
- 已目检 6 张 Slint 软件渲染截图：1080 窗口下的 Profile、单条、批量、备份下调和危险重置，以及 900×620 的最后 Profile 名称确认；深浅色危险按钮和小窗口底部操作区均完整可见。
- 本轮未执行真实 Helper/系统 hosts 写入，未以软件渲染替代原生 Tab 顺序、VoiceOver 或实际备份清理验收；这些仍按上文原生验收边界执行。

## 2026-11-20 多 Profile 组合启用实装验证

- 当前编辑与 active 集合已分离；侧栏可独立启用多个 Profile，主窗口与 Tray 统一预览、确认和应用完整组合，支持空组合移除。确认前会重新加载 Profile 文档与 activation，外部变化使旧批准失效。
- activation 使用稳定 Profile ID、严格磁盘 schema 和 `activation.lock` 跨进程事务；缺字段或损坏文件失败关闭。核心补充 DNS/IP 规范化冲突、旧区块迁移、混合换行字节保真、no-op 过期检查及最终替换前并发修改拒绝。
- `cargo test --locked --workspace`、`cargo clippy --locked --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check` 全部通过。
- `cargo test --locked -p crabhosts-desktop --features ui-tests` 共 22 项测试全部通过，覆盖开关不冒泡、A+B 组合、关闭单项、取消不回滚 activation、空组合、并发更新、保存失败、外部 activation/Profile 变化使计划失效及加载失败禁止应用。
- 已目检组合启用、组合确认和空组合确认三张 Slint 软件渲染截图，开关、完整组合名称、危险提示及底部按钮均完整可见；测试截图输出目录已在验证后清理，不纳入源码。
- 本轮所有写入均使用临时 AppPaths 和 hosts fixture，未执行真实 `/etc/hosts` 写入、签名包更新或协议 2 Helper 安装。新协议的真实 Helper 更新、桌面/CLI 端到端及原生 Tray/VoiceOver 仍需另行验收，不能沿用协议 1 的历史结果。

## 2026-11-20 主题与动态效果跟随系统实装验证

- 主题和减少动态效果均改为设备本地三态偏好，默认跟随系统；旧版布尔 `reduce_motion` 配置可读取。主题通过 `NSApplication.effectiveAppearance` KVO 更新，Tray 图标独立按系统菜单栏外观选择。
- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 全部通过。
- `cargo test -p crabhosts-desktop --features ui-tests` 共 23 项测试通过，覆盖三态切换、系统浅色/深色解析、系统减弱动态效果解析、旧配置兼容及 900×620 紧凑窗口布局。
- 已目检外观设置的软件渲染截图（普通与紧凑窗口），并使用临时 `CRABHOSTS_HOME` / `CRABHOSTS_HOSTS_FILE` 启动原生隔离实例，确认默认主题为 `system`、当前深色系统下正常启动；未执行真实系统 hosts 写入。原生手动切换 macOS 外观和“减弱动态效果”仍需在签名应用中复核。
