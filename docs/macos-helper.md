# macOS 系统写入 Helper

CrabHosts 在 macOS 13+ 使用长期注册的 launchd root Helper 修改系统 hosts。Helper 在安装、更新或修复时由用户显式授权；日常桌面端和 AI Harness 写入不重复输入管理员密码。

## 架构与调用边界

```text
Slint 桌面端 ─┐
              ├─→ crabhosts-system-client（Rust 库）─→ XPC ─→ root Helper
crabhosts CLI ─┘                                             └─→ crabhosts-core
```

桌面端不启动 CLI 子进程。AI Harness 调用 CLI，CLI 与桌面端直接依赖同一个 Rust 客户端库。Helper 不连接网络，不接受 shell 命令、目标路径、Profile 数据库路径或授权凭据。

签名标识：

- 桌面端：`dev.crabhosts.desktop`
- CLI：`dev.crabhosts.cli`
- Helper / Mach 服务：`dev.crabhosts.hosts-helper`

XPC 双向设置 peer code-signing requirement。Helper 还对每条请求调用 `SecCodeCreateWithXPCMessage`，使用消息关联的 audit token 验证发送者；不使用 PID 或私有 API 判断身份。允许的客户端必须属于打包时指定的 Apple Team ID，并使用上述精确标识。

## 写入时序

1. 用户态生成 `SystemPlan` 和 diff。
2. 桌面端要求显式确认；AI Harness 正式写入必须同时传 `--yes` 与当前 dry-run 的 `--expected-plan`。
3. 用户态再次核对当前内容并创建备份。备份失败则不联系 Helper。
4. Helper 读取固定的 `/etc/hosts`，忽略调用方的 `CRABHOSTS_HOME` 和 `CRABHOSTS_HOSTS_FILE`。
5. Helper 重算计划哈希；Profile 应用还会验证只有 managed block 发生变化，restore 则明确恢复完整快照。
6. 生产写入要求目标已存在且为普通文件，以 `O_NOFOLLOW` 打开核验，在同目录写临时文件，保留属主和权限，原子替换并复读验证。
7. 用户态把先前创建的 backup ID 附加到 `ApplyReport`。

XPC 请求有版本、字段和大小限制。Helper 在串行队列处理写入。连接中断或超时时结果可能不确定，调用方必须重新读取状态，不能盲目重试。

## 打包

```bash
export CRABHOSTS_CODESIGN_IDENTITY="Developer ID Application: … (TEAMID)"
export CRABHOSTS_TEAM_ID="TEAMID"
./scripts/package-macos.sh
```

产物为 `dist/CrabHosts.app`，其中包含：

```text
Contents/MacOS/CrabHosts
Contents/SharedSupport/crabhosts
Contents/Library/LaunchServices/dev.crabhosts.hosts-helper
Contents/Library/LaunchDaemons/dev.crabhosts.hosts-helper.plist
```

脚本逐个签名嵌套可执行文件和外层 app，并执行 `codesign --verify` 与 `plutil -lint`。不使用 `codesign --deep`，不接受无法通过 Helper 身份校验的 ad-hoc 签名。正式发布需要 Developer ID Application、hardened runtime、secure timestamp 和 Apple 公证；Apple Development 签名仅用于本机开发验收。

## 安装、批准与更新

- 打开应用不会自动注册 Helper。
- 用户在侧栏显式选择安装、更新或修复后才调用 `SMAppService`。
- `register()` 成功不代表 launchd 立即可用；若状态为 `requiresApproval`，需要在“系统设置 → 通用 → 登录项与扩展”中允许。
- Helper 更新要求重新注册。不要在 `requiresApproval` 状态循环注销/注册。
- 生命周期操作只存在于签名桌面应用包中；独立 CLI 只负责状态检查和日常写入。

CLI 只读预检：

```bash
crabhosts --json helper status
```

该命令不会安装 Helper 或修改 hosts。Helper 不可用、版本不匹配或签名验证失败时，正式写入失败且不直接回退写 `/etc/hosts`。

## AI Harness

正式命令保持不变：

```bash
crabhosts --json system apply <profile> --yes --expected-plan <plan-id>
crabhosts --json system restore <backup-id> --yes --expected-plan <plan-id>
```

AI 不直接连接 XPC，不调用安装接口，不添加 `sudo`，也不把 profile 编辑或同步转换为系统应用。

安装 Helper 后，本机活动用户上下文中能执行签名 CLI 的进程可以请求系统写入而不再输入密码。`--yes` 与 `plan_id` 是工作流保护，不是本机恶意进程之间的人类同意证明；这是该权限模式的明确边界。

## 测试边界

同时设置 `CRABHOSTS_HOME` 与 `CRABHOSTS_HOSTS_FILE` 时，用户态客户端只使用临时 fixture 的直接后端，绝不把该计划发送给生产 Helper。root Helper 自身始终忽略这些变量。

核心、CLI 和无头 UI 自动化只使用临时路径。签名、SMAppService 批准、System Settings 撤销、更新、重启持久性、桌面/CLI/AI Harness 端到端以及 Apple 公证需在对应 macOS 环境中单独验收，不能由单元测试替代。

## 2026-11-08 本机验收记录

已在 macOS 26.6.2 arm64 上使用 Apple Development 签名完成以下真实验证：

- `SMAppService` 注册和 Helper 启动；桌面端显示 Helper v0.1.0、协议 1 已就绪。
- 签名 CLI 的 `helper status` 成功，精确标识和 Team ID requirement 校验通过。
- ad-hoc 签名客户端被 Helper 拒绝，未获得系统写入能力。
- AI Harness 使用的 CLI 路径完成真实 apply，备份 ID 为 `20261108T132957Z-a8493b3e`。
- CLI 完成真实 restore，写入前备份为 `20261108T143846Z-f665f2de`。
- 桌面端完成真实 apply，写入前备份为 `20261108T152216Z-dd2ce08c`；随后 dry-run 返回 `changed: false`，当前哈希与目标哈希一致。
- 首次 apply 曾出现“写入成功但响应为冲突”的结果确认异常；共享客户端已增加精确内容核对，并以模拟丢失响应测试确保不会重复写入。

2026-11-10，包含当前桌面端实装的 arm64 包已使用 Developer ID Application、hardened runtime 和 secure timestamp 完成签名，Apple 公证返回 `Accepted`，并已附加公证票据。Gatekeeper 返回 `accepted` / `Notarized Developer ID`；最终 ZIP 解压后再次通过签名、票据和 Gatekeeper 复验。该结果只覆盖当时的 arm64 分发快照；后续代码变更必须重新打包和公证，也不替代以下平台与生命周期验收。

尚未覆盖：macOS 13 实机/VM、System Settings 撤销、重启持久性、Helper 更新和 x86_64/universal 包。2026-11-08 的开发签名包不作为正式发布验收。

参考：[SMAppService](https://developer.apple.com/documentation/servicemanagement/smappservice)、[SecCodeCreateWithXPCMessage](https://developer.apple.com/documentation/security/seccodecreatewithxpcmessage(_:_:_:))、Apple [TN3127](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements)。
