# CrabHosts

CrabHosts 是一个以 **Rust + Slint** 构建的 hosts 管理工具，目标是提供比传统 hosts 切换工具更适合自动化与 AI Harness 的使用方式：同一份核心能力同时服务桌面端和 CLI，并通过 Agent Skill 让 Claude Code、Codex、WorkBuddy、DSH 等工具安全地管理本机 hosts。

> 当前为工程初始化版本。Profile 管理、CLI JSON 契约、差异预览、确认写入、自动备份与恢复已有实现；多设备同步目前提供架构方案，尚未实现网络同步。

## 核心原则

- **AI 原生**：稳定子命令、`--json` 输出、非交互执行、明确退出码。
- **默认安全**：预览不写文件；真实系统修改必须显式 `--yes`；不隐式提权。
- **最小影响**：只替换 CrabHosts 托管区块，保留系统 hosts 中的其他内容。
- **可恢复**：写入前自动备份并记录 SHA-256；恢复前也会备份当前状态。
- **共享核心**：CLI 与 Slint 桌面端使用同一个 `crabhosts-core`。

## 工程结构

```text
crates/core/          hosts 数据模型、校验、存储、托管区块、备份与恢复
crates/cli/           crabhosts 命令行程序
apps/desktop/         Slint 桌面端及 .slint UI
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

`profile` 子命令只修改 Profile 数据库，不修改系统 hosts。系统文件通常需要管理员/root 权限；CrabHosts 不会自行调用 `sudo` 或弹隐式提权命令。

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

其他 Harness 可直接导入该目录，或按客户端当前版本支持的自定义 Skill 目录安装。详见 [AI Harness 接入说明](docs/ai-harness.md) 和 [CLI 契约](skills/crabhosts/references/cli.md)。

## 多设备同步方案

推荐同步**带稳定 ID 的 Profile 文档**，而不是直接复制 `/etc/hosts`：

1. WebDAV 作为首版用户可控传输层，使用 revision + ETag/`If-Match` 防止覆盖丢失。
2. 同步前做三向合并；同一记录的并发修改保留为冲突，不静默覆盖。
3. 内容端到端加密，服务端只保存密文；设备通过配对加入，可撤销。
4. 当前激活 Profile、是否应用、权限与备份均保持设备本地；同步完成不代表自动修改系统。
5. 后续再增加可选 Relay 和 P2P 传输，不改变核心数据协议。

完整设计见 [多设备同步方案](docs/sync.md)。

## 下一步

- 桌面端 Profile 编辑器与批量导入/导出。
- 独立的最小权限系统写入 Helper 与审计日志。
- WebDAV SyncProvider、端到端加密和设备配对。
- Windows 提权、macOS 授权及 Linux polkit 的原生集成。
