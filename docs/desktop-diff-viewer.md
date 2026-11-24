# 桌面端差异预览：截断预览与全屏查看

## 背景与问题

短窗口下，主窗口底部差异预览只露出约 6–7 行，卡片底边被窗口裁切；内联面板自身又嵌了一层 `ScrollView`，内外两层滚动区域的滚轮归属不明确，用户看到的现象是“滚不动、也看不全”。

现状常量：

| 项 | 舒适 | 紧凑（窗口高 ≤ 760px） |
| --- | --- | --- |
| `preview-height` | 206px | 178px |
| 可视线数（约） | 6–7 | 5 |

结论：内联面板不再承担长内容阅读，只做“一眼预览 + 明确截断提示”；完整内容交给独立的全屏查看器。这样同时消除嵌套滚动，不再依赖内层滚动条。

## 实装状态（2026-11-11）

- 已实装于 `apps/desktop/ui/main.slint` 与 `apps/desktop/src/main.rs`，并新增 `expand.svg`、`wrap.svg`、`hash.svg`、`copy.svg`。
- 原型保留在 `prototypes/desktop-ui/index.html` 供设计对照；尺寸与视觉以原型为准。
- 已通过桌面 UI 测试（9 项）、`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings` 和 `cargo fmt --all -- --check`。
- 关闭查看器后通过一次性 focus 钩子把焦点送回面板头「全屏」按钮；该行为尚无自动化焦点断言，需在真实窗口中手工确认。真实 macOS 窗口的剪贴板与触控板滚动仍需手工验收。

## 目标与非目标

目标：

- 内联面板稳定显示 8 行（紧凑 6 行），并显示总行数、增删统计和“仅显示前 N 行”。
- 全屏查看器在 900×620 至 1440×960 窗口下一次可见约 17–30 行，内容独立滚动，最长 1180×860，短窗口自适应。
- 全屏严格只读，不修改 Profile、差异计划、备份或系统 hosts。
- 键盘可达：`Esc` 关闭；`⌘⇧F` 通过菜单项「全屏预览」打开；`Tab` 可到达查看器控件。

非目标（本轮不做）：

- 不改 diff 生成、plan 校验、确认层、Helper 或备份逻辑。
- 不做语法高亮、搜索、行跳转、并排 diff 或系统 hosts 编辑。

## 交互入口

1. 面板头右侧「全屏」按钮（expand 图标，small quiet）。
2. 面板底部右侧「全屏查看」文本按钮（靠近被截断内容，短窗口下也容易发现）。
3. 侧栏「查看系统 hosts」直接打开查看器的系统 hosts 模式；原先的 720px 只读弹层已移除。
4. `⌘⇧F`（仅 Profiles 视图）通过菜单项「全屏预览」注册，快捷键与菜单项同时生效。
5. 关闭：`Esc`、右上角关闭按钮、点击遮罩；关闭后通过预览卡内的一次性 focus 钩子把焦点送回「全屏」按钮。

入口位置见标注图：[preview-fullscreen-entry.png](../prototypes/desktop-ui/preview-fullscreen-entry.png)。

## 内联面板规格

| 项 | 舒适 | 紧凑（高 ≤ 760px） |
| --- | --- | --- |
| 卡片高度 | 272px | 232px |
| 头部高度 | 50px | 50px |
| 正文高度 | 192px | 152px |
| 底部条高度 | 30px | 30px |
| 可见行数 | 8 | 6 |
| 行高 / 字号 | 22px / 12px mono | 同左 |
| 行号列宽 | 46px | 46px |
| 水平内边距 | 18px | 18px |
| 头部分段控件 | 高 29px，内边距 3px，圆角 9px | 同左 |

结构：

- 头部：`差异预览 / 系统 hosts` 分段 + 统计（`共 37 行 · +4 −22`，11px `faint`，等宽数字）+ 只读锁 + 从系统 hosts 导入 + 全屏。紧凑宽度下统计可隐藏，但分段、只读与全屏必须保留。
- 正文：只渲染前 N 行并 `clip`，**不使用内部滚动**；最后一行完整显示，不做半行截断。正文底部 10px 渐隐只压住留白，不遮挡文字。
- 底部条：左侧 `共 37 行 · 仅显示前 8 行`（全部显示时为 `已全部显示`），右侧「全屏查看」按钮（24px 高，hover 出现边框与底色）。

颜色（深色主题）：

| 元素 | 值 |
| --- | --- |
| 正文背景 | `#151613`（`Theme.field`） |
| 普通文本 | `#BBBCB2` |
| 行号 | `#5F6059` |
| 新增行 | 文本 `#A8B7A1`，背景 `rgba(128,151,120,.08)` |
| 删除行 | 文本 `#D2AAA3`，背景 `rgba(166,108,99,.08)` |
| Hunk 头 | 文本 `#93A291`，背景 `rgba(147,162,145,.07)` |
| 文件头（`---/+++`） | `Theme.text-soft`，字重 500 |
| 系统 hosts 注释 | `#85877E` |
| 底部条 | 背景 `#171815`，上边框 `Theme.border-soft` |

边界：

- 侧栏入口打开后默认进入“系统 hosts”分段，关闭时内联面板保持该分段。
- 空 Profile / 空差异：显示 `# 暂无启用记录`，面板高度不变，统计为 `共 N 行 · +0 −0`。
- 系统 hosts 为空：显示 `（文件为空）`。
- 超长单行（≥200 字符）不换行、横向裁切，由全屏的“自动换行”处理。

## 全屏查看器规格

遮罩：全窗口 `rgba(5,6,5,.74)` + 6px 背景模糊，点击空白关闭。

面板：

| 项 | 值 |
| --- | --- |
| 尺寸 | `min(1180px, 100vw − 48px) × min(860px, 100vh − 48px)` |
| 圆角 / 边框 | 16px / 1px `#3F4039` |
| 背景 | `Theme.surface`，阴影 `0 28px 80px rgba(0,0,0,.5)` |
| 头部 | 68px；左：11px kicker、19px/650 标题、Profile 胶囊、只读锁；右：`Esc` 键帽（34×22，与关闭按钮同宽、同行垂直居中）+ 34px 关闭 |
| 工具栏 | 52px；左：分段控件；右：自动换行 / 行号 / 复制，随行高垂直居中 |
| 正文 | 自适应剩余高度，**唯一滚动容器**；上下内边距 10/20px |
| 行高 / 字号 / 行号列 | 22px / 12px mono / 56px |
| 底部 | 46px；状态点 + 只读说明 + 统计 + `系统 hosts：/etc/hosts` |
| 滚动条 | 11px，轨道 `#151613`，滑块 `#3D3E37`（Slint 用默认滚动条） |
| 渐隐 | 顶部/底部 22px，仅在可继续滚动的一侧显示 |

可见行数（行高 22px，已扣除正文留白）：

| 窗口 | 面板 | 正文 | 约可见行 |
| --- | --- | --- | --- |
| 1440×960 | 1180×860 | 694px | 30 |
| 1024×720 | 992×688 | 540px | 23 |
| 900×620（最小） | 852×572 | 424px | 17 |

内容与行为：

- 差异 tab 展示原始 unified diff（保留 `---`、`+++`、`@@`、`+`、`-` 前缀）；行号沿用当前 model 的 diff 行序号。
- 系统 hosts tab 展示原文；托管区块起止行用正文色，普通注释用 `#85877E`。
- 自动换行：关→单行 + 横向滚动；开→`word-wrap`，行高自适应，最小 22px。
- 行号：关→行号列宽 0；开关状态只在本次会话内保留，不写入偏好、不参与同步。
- 复制：复制当前 tab 的完整原文并 toast 提示。Slint 若无法可靠访问剪贴板，应隐藏复制按钮，而不是给出假成功。
- 打开时滚动到顶部；切换 tab 也回到顶部，避免在差异已变化后停留在过期位置。
- 查看器不提供“应用/写入”按钮，也不改变主窗口的待确认状态。

## 键盘与可访问性

- `Esc` 关闭；打开后焦点由查看器 `FocusScope` 接管，`Tab` 可到达工具栏与关闭按钮；关闭后焦点回到面板头「全屏」按钮。Slint 没有内建焦点陷阱，背景控件不在 Tab 顺序中被显式屏蔽（列为已知限制）。
- 语义：`role=dialog`、`aria-modal=true`、标题与说明关联；分段用 `tablist/tab`，开关按钮用 `aria-pressed`。
- 减少动态效果下渐隐与 hover 过渡为 0ms。
- 正文对比度约 8:1；行号属于辅助信息，不承载操作。

## 响应式

- 宽 ≤1120px：遮罩内边距 16px，面板 `100vw − 32px × 100vh − 32px`；头部/工具栏/底部左右内边距 16/14px。
- 高 ≤760px：头部 60px、工具栏 46px、底部 42px，正文仍保留约 23 行。
- 头部右侧三个操作按钮在 1024 宽下必须单行；不足时优先保留“全屏”，其次行号、复制，最后自动换行。

## 安全边界

- 查看器是只读视图：不改 `Document`、不改 Profile、不生成新 plan、不触发备份、不写 hosts。
- 关闭查看器后，应用仍需在主窗口确认层显式确认；查看器不提供绕过路径。
- 打开/关闭查看器不得改变 `plan_id` 或使已有 plan 失效。

## Slint 实装映射（已落地）

### `apps/desktop/ui/main.slint`

1. `preview-height` 已改为 272px，紧凑窗口 232px；`preview-inline-lines` 为 8/6，`content-height`、`entries-height` 公式沿用。
2. 内联预览移除内部 `ScrollView`：`for line[index] in root.preview-lines` 渲染，超出 `preview-inline-lines` 的行 `height: 0px` 且 `visible: false`，最后一行完整；底部 30px 条显示 `共 N 行 · …` 与「全屏查看」。
3. 头部新增「全屏」`AppButton`（`expand.svg`）；统计文案来自 Rust 下发的 `panel-stats`，行数来自 `panel-line-count`。
4. 全屏查看器直接内联在 `MainWindow` 根层（没有拆成独立组件），与 `ModalFrame` 同级，避免被 `main-scroll` 嵌套；正文只有一个 `ScrollView`。
5. 状态：`show-preview-viewer`、`preview-wrap`、`preview-gutter`；`panel-mode` 同时驱动内联与查看器 tab；尺寸为 `min(1180px, root.width - 48px) × min(860px, root.height - 48px)`。
6. 工具栏：自动换行、行号、复制；正文行高 22px（换行时按 `viewer-line.preferred-height` 增高），行号列 56px，行号与正文之间留 8px。
7. 键盘：`FocusScope` 的 `key-pressed` 处理 `Esc`；菜单项「全屏预览」注册 `@keys(Control + Shift + F)`，仅 Profiles 视图生效。
8. 图标 `expand.svg`、`wrap.svg`、`hash.svg`、`copy.svg` 均为 24 viewBox、1.7px 线性风格。
9. 颜色全部使用 `Theme.*`，深浅主题沿用现有令牌。
10. 焦点：打开后由查看器 `FocusScope` 接管；关闭时，预览卡内一个仅在 `viewer-opened && !show-preview-viewer` 时创建的零尺寸元素在 `init` 中聚焦「全屏」按钮，实现焦点归还（无自动化焦点断言）。

### `apps/desktop/src/main.rs`

- `set_panel_content` 统一设置 `panel-text`、`preview-lines`、`panel-line-count`、`panel-stats`；diff 统计 `+A −R`，系统 hosts 只显示总行数。
- `load_system_hosts` 使用 raw 版本，不把 hosts 中的 `-`/`+` 行误计为增删。
- 新增 `copy-preview` 回调，使用 `arboard`（仅文本、`default-features = false`）写入剪贴板，成功/失败都通过现有 toast 反馈。
- 移除旧的 `open-system-hosts` 回调与 `system-hosts-text`、`show-system-hosts` 状态；侧栏入口直接切到查看器系统模式。
- 不改 diff 生成、plan、确认层、备份与 Helper 流程。

## 实装清单

已完成：

- [x] 新增四个图标资源；`preview-height`、`preview-inline-lines` 调整。
- [x] 内联移除嵌套滚动，改为 8/6 行截断，底部统计与「全屏查看」。
- [x] 内联头部统计/只读/导入/全屏在 900px 宽单行不溢出。
- [x] 全屏查看器：唯一滚动层、头部/工具栏/正文/底部、遮罩与 Esc 关闭。
- [x] 自动换行、行号、复制；空内容占位；900×620 与 1080×760 截图。
- [x] 浅色主题检查；查看器前后 hosts 字节一致。
- [x] 侧栏系统 hosts 入口统一，旧弹层与旧状态删除。

待办/已知差距：

- [x] 关闭后焦点归还「全屏」按钮（一次性 focus 钩子，需手工确认）。
- [ ] 真实 macOS 窗口的滚轮/触控板、剪贴板、300+ 行 diff 手工验收。
- [ ] 如需无换行时的横向滚动，再评估 Slint `ScrollView` 的宽度策略。

## 测试方案

原型（已执行）：

- 54 项断言，覆盖内联截断、统计、全屏打开/关闭、滚动边界、换行、行号、tabs、`Esc`、`⌘⇧F`、焦点归还、侧栏系统 hosts 入口、900×620 最小窗口适配、既有 Profile/记录/应用/Tray/设置流程、横向溢出与 SVG 引用。
- 结果：54/54 通过，JavaScript 运行时错误 0。

Slint（已执行）：

- `cargo test -p crabhosts-desktop --features ui-tests`：9 项通过；覆盖内联两个全屏按钮、`Esc` 关闭、查看器渲染、换行/行号属性、系统 hosts 统计、浅色主题、900×620 与 1080×760 快照，以及 hosts 原始字节不变。
- `cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check` 全部通过。
- 截图：`target/ui-shots/preview-viewer-diff.png`、`preview-viewer-wrap.png`、`preview-viewer-system.png`、`preview-viewer-compact.png`、`preview-viewer-light.png`、`profiles-preview-inline.png`。

仍需手工验收：

- 真实 macOS 窗口的滚轮/触控板、复制到系统剪贴板、300+ 行 diff、空 Profile、空 hosts。
- 关闭查看器后的焦点行为（已实现 focus 钩子，需真实窗口确认）。

## 事实边界

- Slint 桌面端已按本文实装；`prototypes/desktop-ui/index.html` 仍是模拟数据的设计对照，不读写真实 `/etc/hosts`。
- 查看器不新增任何写入路径；复制只读写系统剪贴板，不触碰 hosts。
- 真实工程验收以 `apps/desktop` 的 UI 测试与 macOS 原生窗口检查为准。
