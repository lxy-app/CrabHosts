# CrabHosts AI Harness 接入指南

CrabHosts 的 AI Harness 原生支持由三部分组成：稳定的 `crabhosts` CLI、全局 `--json` 自动化输出，以及随仓库维护的 canonical Agent Skill。它不依赖某个 Harness 的私有接口，也不要求 AI 直接修改系统 hosts。

## 1. Canonical Skill

唯一源文件为：

- [`skills/crabhosts/SKILL.md`](../skills/crabhosts/SKILL.md)：操作流程与强制安全边界。
- [`skills/crabhosts/references/cli.md`](../skills/crabhosts/references/cli.md)：完整 CLI 契约、JSON envelope、退出码和测试环境变量。

修改 CLI 或安全流程时应同步更新这两个文件。复制安装后的 Skill 不会自动跟随仓库更新，升级时应重新导入或复制，并保留 `references/cli.md` 的相对位置。

## 2. 各 Harness 的接入方式

### Claude Code

可将仓库中的 `skills/crabhosts` 目录复制到当前版本支持的用户级或项目级技能目录，也可用符号链接指向该 canonical 目录。常见位置为 `~/.claude/skills/crabhosts` 或项目内 `.claude/skills/crabhosts`；实际自动发现规则以所用 Claude Code 版本文档为准。

### Codex

同样可复制或链接到当前版本支持的通用技能目录。常见位置为 `~/.agents/skills/crabhosts` 或项目内 `.agents/skills/crabhosts`。不要只复制 `SKILL.md` 而遗漏 `references/cli.md`。

### DSH / WorkBuddy

使用产品当前版本提供的**通用 Skill 导入**能力选择 `skills/crabhosts` 目录。若当前环境不导入 Skill，则在项目的 `AGENTS.md` 中明确引用 canonical 文件，例如：

```markdown
执行 CrabHosts 任务前，读取 skills/crabhosts/SKILL.md，
并按 skills/crabhosts/references/cli.md 使用 crabhosts CLI。
```

本文不定义 DSH 或 WorkBuddy 的专用安装路径，也不假设它们会自动发现仓库里的 Skill。应以实际版本的通用导入说明为准。

其他 Harness 也可采用同样原则：能导入 Skill 就导入完整目录；不能导入时，在其通用指令文件中引用 canonical Skill 和 CLI 参考。

## 3. 自动化调用约定

自动执行时统一把全局 `--json` 放在子命令前，例如：

```bash
crabhosts --json status
crabhosts --json profile list
crabhosts --json profile show <name>
crabhosts --json system diff <profile>
```

成功输出 envelope：

```json
{"ok":true,"command":"<command>","data":{}}
```

失败输出 envelope：

```json
{"ok":false,"error":{"code":"<code>","message":"<message>"}}
```

调用方应先检查进程退出码，再检查 `ok`，用 `error.code` 做机器分支；`message` 用于展示，不应通过匹配文案推断错误类型。`data` 的字段由具体命令决定，不要编造未在 CLI 参考中承诺的字段。

已定义的特殊退出码：

| 退出码 | 含义 | Harness 行为 |
| --- | --- | --- |
| 3 | 需要确认 | 展示操作并请求用户决定，不得自动补 `--yes` |
| 4 | 权限不足 | 报告权限问题，不得隐式 `sudo`、提权或改走 shell 写文件 |
| 5 | plan 过期或冲突 | 放弃旧 plan 和旧批准，重新 diff/dry-run 并请求新批准 |

其他非零退出码按失败处理，并保留结构化错误供诊断。

`system diff`、`system apply --dry-run` 和 `system restore --dry-run` 的成功 JSON 均在 `data.plan_id` 返回 SystemPlan 标识。AI/Harness 正式写入必须取**当前 dry-run** 的 `plan_id`，不能用历史预览、其他 operation 或其他 source 的标识代替。

macOS 可先执行 `crabhosts --json helper status` 做只读预检；其成功数据包含 `mode`、`ready` 和 Helper 版本/协议信息。fixture 模式返回 `mode = "direct"`。该命令不安装或更新 Helper。

## 4. 强制系统修改流程

只读查看和 profile 编辑不等于系统应用。`profile create/add/remove/enable/disable` 只修改 profile；`profile delete` 是删除操作，必须先展示目标并获得明确批准后才使用 `--yes`。

### 应用 profile

1. 用 `status`、`profile show <profile>` 确认当前状态和目标内容。
2. 执行 `crabhosts --json system diff <profile>`。
3. 执行 `crabhosts --json system apply <profile> --dry-run`。
4. 从该次 dry-run 的成功 JSON 中读取 `data.plan_id`。
5. 将目标 profile、diff 和 dry-run 结果展示给用户，获得**针对这一次操作和 plan_id** 的明确批准。
6. 仅在批准后执行 `crabhosts --json system apply <profile> --yes --expected-plan <plan-id>`。
7. 读取状态和结果，确认是否成功；失败时保留错误，不重复盲写。

### 恢复备份

1. 用 `backup list` 让用户确认准确的 `<backup-id>`。
2. 执行 `crabhosts --json system restore <backup-id> --dry-run`，展示该恢复将产生的变化。
3. 从该次 dry-run 的成功 JSON 中读取 `data.plan_id`。
4. 获得针对该 backup ID、当前预览和 plan_id 的明确批准。
5. 仅在批准后执行 `crabhosts --json system restore <backup-id> --yes --expected-plan <plan-id>`。

不得把 backup ID 当作 `system diff <profile>` 的参数。SystemPlan 绑定 `operation`、`source`、`current`、`proposed`：分别对应 apply/restore 操作、来源 profile 或 backup、当前系统 hosts 状态和拟写入内容。任何相关变化均使 plan 失效并退出 5；AI/Harness 必须重新 dry-run 取得新 `plan_id`，重新展示变更并获得批准。

人工交互式 CLI 可以在确认后只使用 `--yes`；AI/Harness 正式写入必须将 `--expected-plan <plan-id>` 与 `--yes` 同用，不得省略。

实际系统修改必须由 CrabHosts 核心完成。macOS 正式写入由 CLI 通过共享 Rust 客户端连接已注册并验证签名的 root Helper，不需要 `sudo`；Helper 仅在用户于桌面端显式安装、更新或修复时进行系统授权。AI 不得自行触发生命周期操作。Helper 不可用或协议不匹配时报告错误，不回退到直接写系统文件。连接中断或超时时写入结果可能不确定，应重新读取状态，不得自动重试。

apply 在修改前由原用户进程自动备份并原子替换，只替换 CrabHosts managed block，保留其他系统 hosts；restore 在恢复前同样自动备份当前状态并原子替换，但恢复的是所选备份中的**完整 hosts 快照**，不限于 managed block。AI 不得直接编辑 `/etc/hosts` 或 Windows hosts，不得添加隐式提权、shell 插值或远程触发自动 apply。

## 5. 测试与敏感信息

测试和手工演示必须同时设置 `CRABHOSTS_HOME` 与 `CRABHOSTS_HOSTS_FILE`，指向隔离的临时目录和 fixture；不能使用真实系统 hosts。示例：

```bash
tmpdir="$(mktemp -d)"
export CRABHOSTS_HOME="$tmpdir/home"
export CRABHOSTS_HOSTS_FILE="$tmpdir/hosts"
```

随后应在该临时位置准备测试 fixture，并按测试要求初始化目录。环境变量只隔离 CrabHosts 的读写目标，不代表可以省略系统写入的 diff/dry-run 和批准流程。

不要向外部 AI 服务、issue、日志或同步服务器上传 WebDAV 凭据、恢复口令、私钥、真实 hosts 备份或其他 secrets。profile 中的内网 IP、hostname 和 comment 也可能敏感；需要排障时优先使用脱敏 fixture 和最小错误 envelope。

## 6. 与同步功能的边界

同步设计见 [`docs/sync.md`](sync.md)，其中网络能力是分阶段建议，不应描述为已实现。即使未来接入 WebDAV、relay 或 P2P，AI 也只能同步 profile 文档；active profile 与自动应用保持设备本地，同步完成不得触发系统应用。
