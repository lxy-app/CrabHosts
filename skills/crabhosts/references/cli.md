# crab CLI 契约

本文是 CrabHosts Skill 的 CLI 参考。命令名为 `crab`，全局 `--json` 用于稳定自动化输出。示例统一将 `--json` 放在子命令前。

## 1. 输出与退出码

成功 envelope：

```json
{"ok":true,"command":"<command>","data":{}}
```

- `ok` 为 `true`。
- `command` 标识实际执行的命令。
- `data` 为该命令的数据载荷；不同命令结构不同，不应臆造固定字段。

错误 envelope：

```json
{"ok":false,"error":{"code":"<code>","message":"<message>"}}
```

调用方按 `error.code` 做机器判断，`message` 仅用于向用户说明。不要通过匹配人类可读文案实现控制流。

| 退出码 | 含义 | 必需处理 |
| --- | --- | --- |
| 0 | 成功 | 校验 JSON envelope 后使用 `data` |
| 3 | 需要确认 | 请求用户明确决定，不自动补 `--yes` |
| 4 | 权限不足 | 报告权限问题，不隐式提权或直接改文件 |
| 5 | plan 过期或冲突 | 丢弃旧 plan 和旧批准，重新 diff/dry-run |

其他非零退出码均视为失败。不要只凭进程启动成功就宣称操作已应用。

## 2. 命令总表

### 全局与状态

```bash
crab --json status
crab --json helper status
```

查看 CrabHosts 当前状态。`helper status` 是只读预检：在正式 macOS 模式下检查 Helper 的连通性、版本与协议；临时 fixture 模式返回 `data.mode = "direct"`。该命令不会安装或更新 Helper，也不会修改系统 hosts。`--json` 是全局选项，适用于本文所有命令。

### Profile

```bash
crab --json profile list
crab --json profile show <name>
crab --json profile create <name> [--from-system]
crab --json profile delete <name> --yes
crab --json profile activate <name>
crab --json profile deactivate <name>
crab --json profile add <profile> <ip> <hostname>... [--disabled] [--comment TEXT]
crab --json profile remove <profile> <hostname>
crab --json profile enable <profile> <hostname>
crab --json profile disable <profile> <hostname>
```

- `list`：列出 profile。
- `show`：查看指定名称的 profile。
- `create`：创建空 profile；带 `--from-system` 时从当前系统 hosts 创建。它读取系统内容，但不把后续 profile 编辑自动应用回系统。
- `delete`：删除指定 profile，并清理其本机启用引用。`--yes` 必须来自用户对准确目标的明确批准，不能由 Agent 默认添加。
- `activate` / `deactivate`：通过跨进程锁事务把稳定 Profile ID 加入或移出本机 `activation.json` 的有序启用集合，避免桌面端与 CLI 并发覆盖；只修改设备本地状态，不写系统 hosts。可重复调用，多个 Profile 可同时启用。缺字段、未知字段、损坏 JSON 或不支持的版本会失败关闭，不按空集合处理。
- `add`：向指定 profile 添加 IP 与一个或多个 hostname。`--disabled` 创建禁用条目，`--comment TEXT` 添加注释。
- `remove`：从指定 profile 移除指定 hostname 对应内容。
- `enable` / `disable`：启用或禁用指定 profile 中的 hostname。

这些命令只改变 profile 数据或本机启用集合，不等于系统应用。`status` 和 activate/deactivate 成功数据包含 `active_profile_ids`；启用状态不进入可同步的 Profile 文档。含真实内网信息的 `show`、`create --from-system` 结果不得随意外发。

Profile 编辑示例：

```bash
crab --json profile create dev
crab --json profile add dev 127.0.0.1 api.test web.test --comment "本地开发"
crab --json profile add dev 127.0.0.1 admin.test --disabled
crab --json profile disable dev api.test
crab --json profile enable dev api.test
crab --json profile remove dev web.test
```

### 系统差异与应用

```bash
crab --json system diff [profile]
crab --json system apply [profile] [--dry-run] [--yes] [--expected-plan <plan-id>]
```

- 省略 `profile` 时，`system diff/apply` 针对 `activation.json` 中的完整启用集合；这是新自动化的推荐形式。
- 保留的 `<profile>` 参数表示“把该 Profile 加入当前候选组合”，不是独占启用。dry-run 不持久化该新增候选；兼容形式的正式应用成功后会在跨进程事务中加入新增启用 ID。`activation_persisted` 明确表示是否持久化成功；若为 `false`，`activation_warning` 说明部分失败，`active_profile_ids` 报告重新加载的真实集合，无法重载时为 `null`。
- `system diff`：比较启用组合与当前系统状态，不修改系统 hosts；成功 JSON 的 `data.plan_id` 标识 SystemPlan，`profile_ids` 是稳定身份，`profile_names` 用于展示。
- `system apply --dry-run`：生成组合应用预览，不修改系统 hosts 或 activation；成功 JSON 的 `data.plan_id` 标识该次 dry-run。
- 空启用集合合法：应用后只移除 CrabHosts 托管区块，保留非托管内容。
- 同一 hostname 对应不同 IP 时返回退出码 2 和 `error.code = "profile_conflict"`；`error.details.conflicts` 包含 hostname、Profile ID/名称及 IP，写入前必须解决。相同 IP 的重复映射只渲染一次。
- `--expected-plan <plan-id>` 必须与 `--yes` 同用，用于要求正式写入匹配指定 plan。
- 人工交互式 CLI 可以在确认后只使用 `--yes`；AI/Harness 正式写入必须从当前 dry-run 取得 `data.plan_id`，并将 `--expected-plan <plan-id>` 与 `--yes` 同用。
- 正式应用时不要再附 `--dry-run`；没有当前预览和明确批准时不得执行写入。

AI/Harness 强制顺序：

```bash
crab --json profile activate dev
crab --json profile activate work
crab --json system diff
crab --json system apply --dry-run
# 从该次 dry-run 的 data.plan_id 取得 <plan-id>，核对完整 profile_ids/profile_names。
# 展示差异，并取得针对该组合、当前预览和 plan_id 的明确批准。
crab --json system apply --yes --expected-plan <plan-id>
# 如需关闭某一项：先 profile deactivate <name>，再重新 diff/dry-run/批准/apply。
```

如果任何 plan 绑定内容发生变化，写入退出 5。收到退出码 5 后必须重新运行 diff 和 dry-run 取得新 plan_id，再次请求批准。

### 备份与恢复

```bash
crab --json backup list
crab --json system restore <backup-id> [--dry-run] [--yes] [--expected-plan <plan-id>]
```

- `backup list`：列出可用备份及其 ID。
- `system restore --dry-run`：预览恢复到指定备份所产生的变化，不修改系统 hosts；成功 JSON 的 `data.plan_id` 标识该次 dry-run 的 SystemPlan。
- `--expected-plan <plan-id>` 必须与 `--yes` 同用。人工交互式 CLI 可以在确认后只使用 `--yes`，AI/Harness 则必须绑定当前 dry-run 的 plan_id。

AI/Harness 恢复顺序：

```bash
crab --json backup list
crab --json system restore <backup-id> --dry-run
# 从该次 dry-run 的 data.plan_id 取得 <plan-id>。
# 展示变更并取得针对该 backup-id、当前预览和 plan_id 的明确批准。
crab --json system restore <backup-id> --yes --expected-plan <plan-id>
```

不要把 `<backup-id>` 传给 `system diff` 的可选 Profile 参数。restore 的变更预览由它自己的 `--dry-run` 提供。若返回退出码 5，重新 dry-run 取得新 plan_id 并重新取得批准。

### SystemPlan 绑定

SystemPlan 绑定以下五部分：

- `operation`：`apply_profiles` 或 `restore_backup`，不能跨操作复用。
- `profile_ids`：apply 的有序稳定 Profile ID 集合；restore 为空。
- `source`：apply 的 Profile 名称摘要，或 restore 的来源 backup。
- `current`：生成预览时的当前系统 hosts 状态。
- `proposed`：该次预览拟写入的内容。

`operation`、`profile_ids`、`source`、`current` 或 `proposed` 的任何相关变化都会使 plan 失效，正式写入退出 5。AI/Harness 不得改用其他 plan_id 或直接去掉 `--expected-plan` 重试；必须重新 dry-run、展示变更并获得明确批准。

## 3. 系统写入保证与禁止事项

实际 apply/restore 由 CrabHosts 核心完成。macOS 正式系统路径通过已注册并验证签名的 root Helper 写入；CLI 自动通过共享客户端连接 Helper，不需要也不应添加 `sudo`。Helper 在安装、更新或修复时由用户显式授权，日常写入不重复询问密码。Helper 不可用、版本不匹配或签名验证失败时命令失败，不回退到直接写系统文件。`helper_unavailable`、`helper_protocol` 和 `helper_write_failed` 均为结构化错误；连接结果不确定时应重新读取状态，不盲目重试。

两者都会在写入前自动备份当前状态、使用原子替换，并在 plan 过期或冲突时拒绝写入。备份仍由原用户进程创建，Helper 不修改用户 Profile 数据库。写入范围如下：

- **apply Profiles**：按有序 `profile_ids` 合成一个外层 managed block，只替换该区块并保留其他系统 hosts 内容；Helper 还会核对区块内 Profile 名称/ID 分节与计划完全一致。
- **restore backup**：恢复到所选备份中的完整 hosts 快照，不限于 managed block；恢复前仍会备份当前状态。

Agent 不得直接写 `/etc/hosts` 或 Windows hosts，不得使用 shell 重定向、隐式 `sudo`、AppleScript 提权或其他方式绕过 CLI。profile 编辑、同步或远程任务都不能隐式触发系统应用。

## 4. 隔离测试环境

测试和手工演示必须同时设置：

- `CRABHOSTS_HOME`：隔离的 CrabHosts 数据目录。
- `CRABHOSTS_HOSTS_FILE`：临时 hosts fixture 路径。

示例：

```bash
tmpdir="$(mktemp -d)"
export CRABHOSTS_HOME="$tmpdir/home"
export CRABHOSTS_HOSTS_FILE="$tmpdir/hosts"
```

随后按测试场景在上述临时位置准备 fixture。不得把变量指向真实系统 hosts 后再做写入测试；使用临时路径也不取消 diff/dry-run、明确批准、plan 绑定、自动备份和各操作自身的写入范围约束。
