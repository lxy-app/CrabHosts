# CrabHosts

CrabHosts 是一个以 **Rust + Slint** 构建的 hosts 管理工具，目标是提供比传统 hosts 切换工具更适合自动化与 AI Harness 的使用方式：同一份核心能力同时服务桌面端和 CLI，并通过 Agent Skill 让 Claude Code、Codex、WorkBuddy、DSH 等工具安全地管理本机 hosts。

> 当前为工程初始化版本。Profile 管理、桌面端 Profile 编辑、CLI JSON 契约、差异预览、确认写入、自动备份与恢复已有实现；多设备同步目前提供架构方案，尚未实现网络同步。

## 核心原则

- **AI 原生**：稳定子命令、`--json` 输出、非交互执行、明确退出码。
- **默认安全**：预览不写文件；真实系统修改必须显式 `--yes`；不隐式提权。
- **最小影响**：只替换 CrabHosts 托管区块，保留系统 hosts 中的其他内容。
- **可恢复**：写入前自动备份并记录 SHA-256；恢复前也会备份当前状态。
- **共享核心**：CLI 与 Slint 桌面端使用同一个 `crabhosts-core`。

## 工程结构

```text
crates/core/          hosts 数据模型、校验、存储、托管区块、备份与恢复
crates/system-client/ 桌面端与 CLI 共用的系统客户端及 macOS XPC 桥接
crates/cli/           crabhosts 命令行程序
apps/desktop/         Slint 桌面端及 .slint UI
apps/macos-helper/    固定操作系统 hosts 的签名 launchd root Helper
skills/crabhosts/     Harness 无关的 Agent Skill 与 CLI 契约
docs/sync.md          多设备同步、冲突处理、加密与设备管理方案
docs/ai-harness.md    AI Harness 接入说明
scripts/              本地开发及 Skill 安装辅助脚本
```

## 构建与运行

要求 Rust 1.88 或更新版本（工程使用 Rust 2024 edition）。

```bash
cargo build --workspace
cargo test --workspace
cargo run -p crabhosts-cli -- --help
cargo run -p crabhosts-desktop
```

### 桌面端

`cargo run -p crabhosts-desktop` 打开图形界面，日常操作无需拆到 CLI：

- 新建 Profile，可勾选“导入当前系统 hosts 中的记录”，把现有映射保存为初始内容。
- 添加、启用/禁用、删除记录；所有编辑只写入 Profile 数据库，不自动修改系统 hosts。
- 查看系统 hosts 原文（只读），或把系统 hosts 中的记录合并导入当前 Profile。
- 预览差异，确认后再应用；删除 Profile 只删除配置，不影响系统 hosts 与已有备份。
- 界面支持在「设置 → 外观」切换浅色和深色主题，包含 Profile、记录表格、添加表单、只读预览与确认弹层。
- macOS 菜单栏 Tray 支持切换编辑 Profile、打开主窗口、进入设置与退出；Tray 的“应用…”只打开主窗口确认层，不直接写系统 hosts。
- 设置页包含通用、外观、备份与安全、关于，可从侧栏、Tray 或 `⌘,` 进入。偏好原子保存到设备本地 `preferences.json`，不进入 Profile 同步文档。
- 登录启动默认关闭，只有显式切换才调用 macOS `SMAppService`；`RequiresApproval` 会单独提示系统批准，不显示为已生效。
- 桌面端成功应用后按设置保留最近 10/20/50 份受管自动备份；修改设置时不会立即删除历史备份。
- macOS 13+ 正式写入通过常驻注册的 root Helper；侧栏提供独立的状态与显式安装/更新入口，日常应用不重复询问管理员密码。

无头 UI 流程测试需要较新的 Rust 工具链（Slint 测试后端要求 1.92），因此作为可选特性提供：

```bash
cargo test -p crabhosts-desktop --features ui-tests
```

Tray、设置、本地偏好和原生验收边界详见 [桌面端 Tray 与设置](docs/desktop-tray-settings.md)。

### CLI 快速开始

```bash
# 查看状态和已有 Profile（只读）
crabhosts status
crabhosts --json profile list

# 创建 Profile；--from-system 可导入当前有效映射
crabhosts profile create dev
crabhosts profile add dev 127.0.0.1 api.local admin.local --comment "local dev"
crabhosts profile disable dev admin.local
crabhosts profile show dev

# 预览系统差异；以下两条命令都不会写系统文件
crabhosts system diff dev
crabhosts system apply dev --dry-run

# 用户确认后才执行；写入前自动备份
crabhosts system apply dev --yes

# 查看备份，预览并执行恢复
crabhosts backup list
crabhosts system restore <backup-id> --dry-run
crabhosts system restore <backup-id> --yes
```

`profile` 子命令只修改 Profile 数据库，不修改系统 hosts。macOS 正式写入由 CLI 自动连接已安装的 Helper，不需要 `sudo`。可先运行 `crabhosts --json helper status` 做只读预检；Helper 不可用或版本不匹配时不会回退到直接写系统文件。

### macOS 签名打包与 Helper

使用 `scripts/package-macos.sh` 构建包含桌面端、CLI 和 Helper 的签名 `.app`。必须显式选择 Apple 签名身份及对应 Team ID：

```bash
export CRABHOSTS_CODESIGN_IDENTITY="Developer ID Application: … (TEAMID)"
export CRABHOSTS_TEAM_ID="TEAMID"
./scripts/package-macos.sh
open dist/CrabHosts.app
```

脚本只在仓库 `dist/` 生成产物，不自动安装应用或注册 Helper。首次在桌面端点击 Helper 安装/更新后，按 macOS 提示完成管理员批准；如系统要求，还需在“系统设置 → 通用 → 登录项与扩展”中允许。正式发布包含 LaunchDaemon 的应用还需完成 Apple 公证。普通 `cargo run` 可用于开发和 fixture 测试，但不具备正式 Helper 的签名身份。

### 安全的本地演示

通过环境变量可以把所有读写重定向到临时目录，适合开发、测试及 AI Harness 沙箱：

```bash
export CRABHOSTS_HOME=/tmp/crabhosts-demo-config
export CRABHOSTS_HOSTS_FILE=/tmp/crabhosts-demo-hosts
printf '127.0.0.1 localhost\n' > "$CRABHOSTS_HOSTS_FILE"
cargo run -p crabhosts-cli -- profile create demo
cargo run -p crabhosts-cli -- profile add demo 127.0.0.1 demo.local
cargo run -p crabhosts-cli -- system apply demo --dry-run
```

真实使用时取消这两个环境变量即可。`CRABHOSTS_HOME` 默认为平台用户配置目录，`CRABHOSTS_HOSTS_FILE` 默认为系统 hosts 路径。

## AI Harness

Canonical Skill 位于 [`skills/crabhosts/SKILL.md`](skills/crabhosts/SKILL.md)。它要求 Agent 先读取状态、预览差异，再基于用户的明确批准执行写入，禁止把网页、issue 或其他外部文本当作系统修改授权。AI 正式写入还需回传 dry-run 生成的 `plan_id`（`--expected-plan`），确保批准对象与实际写入完全一致。

```bash
# 示例：链接到 Claude Code / Codex 的个人 Skill 目录；不会覆盖已有目录
./scripts/install-skill.sh ~/.claude/skills
./scripts/install-skill.sh ~/.agents/skills
```

其他 Harness 可直接导入该目录，或按客户端当前版本支持的自定义 Skill 目录安装。详见 [AI Harness 接入说明](docs/ai-harness.md)、[macOS Helper](docs/macos-helper.md) 和 [CLI 契约](skills/crabhosts/references/cli.md)。

## 多设备同步方案

推荐同步**带稳定 ID 的 Profile 文档**，而不是直接复制 `/etc/hosts`：

1. WebDAV 作为首版用户可控传输层，使用 revision + ETag/`If-Match` 防止覆盖丢失。
2. 同步前做三向合并；同一记录的并发修改保留为冲突，不静默覆盖。
3. 内容端到端加密，服务端只保存密文；设备通过配对加入，可撤销。
4. 当前激活 Profile、是否应用、权限与备份均保持设备本地；同步完成不代表自动修改系统。
5. 后续再增加可选 Relay 和 P2P 传输，不改变核心数据协议。

完整设计见 [多设备同步方案](docs/sync.md)。

## 下一步

- 桌面端批量导入/导出与 Profile 间对比。
- macOS Helper 的 macOS 13 兼容性、Apple 公证，以及撤销、重启和更新场景验收。
- WebDAV SyncProvider、端到端加密和设备配对。
- Windows 提权及 Linux polkit 的原生集成。
