---
name: crabhosts
description: 管理 CrabHosts hosts profile，并通过 crabhosts CLI 安全查看差异、预览、应用或恢复系统 hosts。用户要求管理 hosts、profile、备份或进行 CrabHosts AI 自动化时使用。
---

# CrabHosts

通过 `crabhosts` CLI 管理 profile 和系统 hosts。执行命令前阅读 [CLI 契约](references/cli.md)；自动化调用始终使用全局 `--json`，不要解析人类可读输出代替 JSON。

## 强制安全边界

- profile 编辑不得隐式修改系统 hosts。
- 实际系统修改只能走 CrabHosts：先展示 diff/dry-run，再获得用户对该次操作的明确批准。AI 正式写入必须将 `--yes` 与 `--expected-plan <plan-id>` 同用，plan_id 取自当前 dry-run 的 `data.plan_id`。
- SystemPlan 绑定 `operation`、`source`、`current`、`proposed`。任何相关变化均退出 5；必须重新 dry-run 取得新 plan_id、重新展示变更并取得批准，不得沿用旧 plan 或旧批准。
- 不直接编辑系统 hosts，不隐式 `sudo` 或提权，不用 shell 重定向绕过 CLI，不远程触发自动 apply。macOS 正式写入由 CLI 自动连接已安装并验证签名的 Helper；不得自行安装、更新或修复 Helper，这些生命周期动作只由用户在桌面端显式发起。
- 可用只读的 `helper status` 预检 Helper。Helper 不可用、协议不匹配或写入结果不确定时停止并报告，不回退到直接写文件，不盲目重试。
- apply 自动备份并原子替换，只替换 CrabHosts managed block，保留其他系统 hosts；restore 恢复所选备份的完整 hosts 快照，恢复前仍自动备份当前状态并原子替换。不得手工改变这两种操作的写入范围。
- 退出码 3 表示需要确认，4 表示权限不足，5 表示 plan 过期或冲突。
- 测试和演示必须同时把 `CRABHOSTS_HOME`、`CRABHOSTS_HOSTS_FILE` 指向隔离的临时路径，禁止使用真实系统 hosts。
- 不上传 WebDAV 凭据、恢复口令、私钥、真实 hosts 备份或其他 secrets；内网 IP、hostname、comment 也应视为敏感信息，排障优先使用脱敏 fixture。

## 操作流程

### 1. 查看与编辑 profile

1. 用 `status`、`profile list`、`profile show <name>` 确认现状。
2. 使用 `profile create/add/remove/enable/disable` 修改指定 profile；这些操作本身不应用系统 hosts。
3. `profile delete <name> --yes` 会删除 profile：先展示准确目标并取得明确批准，不得自行补充 `--yes`。

### 2. 应用 profile

1. 执行 `system diff <profile>`。
2. 执行 `system apply <profile> --dry-run`，从该次成功 JSON 的 `data.plan_id` 取得 plan_id，并向用户展示目标和变更。
3. 询问用户是否批准这一次 apply 和该 plan_id；沉默、笼统授权或上次批准都不算。
4. 仅批准后执行 `system apply <profile> --yes --expected-plan <plan-id>`，再检查结构化结果和状态。

### 3. 恢复备份

1. 用 `backup list` 核对 backup ID。
2. 执行 `system restore <backup-id> --dry-run`，从该次成功 JSON 的 `data.plan_id` 取得 plan_id，并展示变更。
3. 获得针对该 backup ID、当前预览和 plan_id 的明确批准后，才执行 `system restore <backup-id> --yes --expected-plan <plan-id>`。
4. 若返回 plan 过期或冲突，停止并重新执行 dry-run、plan 绑定和批准流程。

### 4. 处理输出与失败

成功响应为 `{ok:true,command,data}`，错误响应为 `{ok:false,error:{code,message}}`。先检查退出码和 `ok`，按 `error.code` 分支；不要依赖 `message` 文案做控制流。`system diff` 及 apply/restore dry-run 的成功数据包含 `data.plan_id`。命令失败时报告错误并停止相关写入，不盲目重试或改用直接文件修改。

完整命令、参数和示例见 [references/cli.md](references/cli.md)。
