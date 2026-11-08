# CrabHosts 同步设计建议

> 状态：本文是推荐设计与分阶段路线，不代表 WebDAV 同步、端到端加密、设备配对、审计、relay 或 P2P 已经实现。在这些能力真正落地并完成验证前，不应向用户承诺自动同步或跨设备恢复。

## 1. 核心决策

首版应同步 **CrabHosts profile 文档**，不要同步 `/etc/hosts`、Windows 系统 hosts 或系统 hosts 备份。

原因：

- 系统 hosts 含有设备本地内容，直接覆盖会破坏其他软件和人工维护的条目。
- CrabHosts apply 的安全边界是“自动备份、原子替换、只替换 CrabHosts managed block 并保留其他系统 hosts”；restore 会先备份当前状态，再恢复所选备份中的完整 hosts 快照。同步系统文件会绕过这些保护。
- profile 有稳定结构，适合做版本控制、三向合并和冲突提示；原始 hosts 文本不适合作为跨设备数据库。
- 内网域名、IP 和注释可能敏感，应在结构化文档边界内统一加密和审计。

**同步成功不等于应用成功。** 下载、上传或合并 profile 都不得隐式执行 `system apply` 或 `system restore`。

## 2. 同步与设备本地状态

### 应同步

- profile 的稳定 `profile_id`、名称和结构化的 hosts 条目。
- 每条记录的稳定 `entry_id`、IP、hostnames、disabled 状态和 comment。
- `schema_version`、单调递增的 `revision`、更新设备和更新时间。
- 条目及 profile 的删除 tombstone，防止离线设备把已删除内容重新同步回来。

### 仅保存在设备本地

- 当前 active profile。
- 是否自动应用，以及任何定时、启动后或远程触发的应用设置；自动应用默认关闭。
- 最近生成的 apply/restore plan、系统 hosts 快照、备份 ID 与备份文件。
- WebDAV 地址及凭据、设备私钥、密钥链引用、同步游标和冲突处理界面状态。
- UI 偏好和平台权限状态。

同步文档不得包含 active profile 或自动应用标记。设备收到新 revision 后只能更新本地 profile 库；若用户明确选择应用，仍须重新执行 diff/dry-run、展示变更并获得该次明确批准。

## 3. 文档模型与本地提交

建议以“一个 profile 一个文档”为一致性和合并单元，另外维护可重建的 manifest 用于发现 profile。manifest 不应成为唯一事实来源，避免 manifest 与 profile 两次写入产生伪原子性。

每个文档至少包含：

- `schema_version`：用于兼容迁移；遇到未知高版本应停止覆盖并提示升级。
- `profile_id`：创建时生成且永不复用；名称不是身份标识。
- `revision`：每次基于同一远端版本成功提交时加一。
- `entries`：以 `entry_id` 为身份，不用数组下标或“IP + hostname”猜测记录身份。
- `tombstones`：记录删除对象、删除设备和时间，保留到所有已知设备确认后再按保留策略清理。
- `updated_by`、`updated_at`：用于审计和展示，不用时间戳强行判定冲突胜负。

本地编辑应先校验 IP、hostname、名称、comment、schema 和 managed block 约束，再原子写入本地文档及待同步队列。崩溃恢复后不得出现半个 JSON 文档或丢失待同步记录。

## 4. 首版传输：用户自托管 WebDAV

WebDAV 适合作为首版用户自托管传输：部署门槛低，并可利用 HTTP 条件请求实现乐观并发控制。建议远端逻辑布局为：

```text
/crabhosts/v1/manifest
/crabhosts/v1/profiles/<profile_id>
/crabhosts/v1/key-envelopes/<key_id>
```

具体扩展名可随明文或密文格式确定；不要把本机路径、用户名或真实系统 hosts 路径放进远端对象名。

### ETag / If-Match CAS

对单个 profile 文档执行 compare-and-swap：

1. 记录上次下载或上传成功时的远端强 ETag、完整快照和 revision，作为 base。
2. 新建对象使用 `If-None-Match: *`，防止两个设备同时创建同一对象时互相覆盖。
3. 更新对象使用 `If-Match: <base-etag>`，请求体包含 `revision = base.revision + 1`。
4. 成功后保存服务器返回的新 ETag 和 revision；若响应未给新 ETag，则立即重新 GET 再记录。
5. `412 Precondition Failed`（服务器也可能以冲突状态响应）表示 base 已过期。保留本地修改，重新下载并合并，禁止去掉条件头重试覆盖。
6. 下载可使用 `If-None-Match` 减少流量；写入本地缓存仍需原子替换。

revision 用于文档历史和合并，ETag 用于传输层 CAS，二者不能相互替代。若服务器不提供可用于 `If-Match` 的可靠强 ETag，应标记为“不支持安全并发同步”，不能静默退化为最后写入者覆盖。WebDAV `LOCK` 可作为兼容性辅助，但不能替代 CAS。

同步周期建议为：读取本地待同步队列 → 拉取远端元数据 → 对无本地修改的文档快进 → 对本地修改执行 CAS → 对双方都修改的文档三向合并 → 保存冲突和审计结果。所有重试都必须携带原 base 的条件头，网络超时后先重新读取远端，不能把“超时”当作“未提交”。

## 5. 三向合并与冲突

设共同祖先为 **B**，本地版本为 **L**，远端版本为 **R**：

- 只有 L 改：接受 L。
- 只有 R 改：接受 R。
- L、R 改成相同值：接受该值并去重。
- L、R 修改不同 entry，或同一 entry 的不同可独立字段：自动合并，然后重新校验整个 profile。
- 同一字段被改为不同值、同一 `entry_id` 的 add/add 内容不同、delete/update 相交：产生冲突，不按更新时间、设备名或字典序擅自选边。
- delete/delete：合并为同一 tombstone。
- profile 删除与另一设备更新相交：保留 profile tombstone 和更新快照，要求用户决定恢复还是删除。

冲突处理要求：

1. 保存 B、L、R 三份加密前的本地受保护快照及各自 revision/ETag，保留双方修改。
2. 将冲突文档标记为“需要人工解决”，不覆盖本地可用副本，不自动应用系统 hosts。
3. 提供“保留本地”“保留远端”“逐条合并”三种操作；选择后形成新的本地版本。
4. 解决结果必须基于最新远端 ETag 再执行 CAS，revision 基于最新远端递增；若又发生冲突则重新合并。
5. 合并后再次验证 IP/hostname 合法性、schema 和业务不变量；同一 hostname 可对应多个 IP。校验失败不能上传。

tombstone 的清理需要设备确认或足够保守的保留期。仅靠“远端已删除”会让长期离线设备在重连后复活旧数据。

## 6. 端到端加密方向

推荐采用 **XChaCha20-Poly1305 + Argon2id** 的加密方向，并在真实实现前完成协议评审和测试向量验证：

- 初始化同步空间时生成随机 256 位同步根密钥；每个文档使用随机数据密钥和全新的 192 位 XChaCha20-Poly1305 nonce，禁止 nonce 重用。
- AAD 绑定格式版本、`profile_id` 和 `key_id`，防止密文被移动到其他 profile 或密钥上下文后误解密。
- 恢复口令通过 Argon2id 派生密钥加密密钥，用于包裹同步根密钥。每个同步空间使用独立随机 salt，参数按目标设备性能标定并随 envelope 保存，以便未来调整；不得使用固定 salt、固定口令或全局硬编码参数。
- WebDAV 中只放密文、加密后的 manifest 和必要的密钥 envelope。ETag/CAS 作用于密文，revision 放在认证密文内部。
- TLS 仍必须使用：端到端加密不替代传输身份认证，也不能隐藏文件大小、访问时间等全部元数据。
- WebDAV 凭据、同步根密钥和恢复口令保存在系统密钥链或等效安全存储中，不写入 profile、日志、Git 或 AI 上下文。

E2EE 不能防止恶意或失效服务器删除、回滚密文。客户端应检测 revision/ETag 回退并保留本地备份；不要把服务器“返回成功”视为内容可信或可恢复的充分条件。

## 7. 设备配对、撤销、审计与恢复

### 配对

- 每台设备使用独立设备 ID 和长期设备密钥；设备记录只保存必要的公钥、名称、创建/撤销时间。
- 新设备由已授权设备发起一次性、短时有效配对，并在第二台设备上核对设备名和短验证码/指纹。
- 通过经过认证的临时配对通道交付同步密钥或密钥 envelope；不得把根密钥直接放进二维码明文、URL、日志或聊天消息。
- 配对完成后立即进行加密文档往返验证，再标记设备可用。

### 撤销

撤销应同时执行：移除设备授权、轮换同步根密钥、为剩余设备重新包裹密钥，并按协议重加密后续文档。若 WebDAV 支持每设备凭据，也应同时撤销对应凭据。

撤销无法追回设备已经下载或解密的旧数据，也不能替代服务器访问控制；界面和文档必须明确这一限制。

### 审计

在本机记录配对、撤销、恢复、上传、下载、合并、冲突和密钥轮换事件，字段包括时间、设备 ID、操作、profile ID、revision 和结果。可链接前序事件哈希以发现本地删改，但不要把普通 WebDAV 服务日志宣传为可信审计。

审计不得记录 IP、hostname、comment、口令、私钥、WebDAV 凭据或完整密文；系统 apply/restore 审计与同步审计分开保存。

### 恢复

- 新设备可通过已授权设备配对，或使用恢复口令解开 Argon2id 保护的恢复 envelope。
- 支持轮换恢复 envelope，并验证恢复后能解密小规模真实同步样本，而不只验证口令“格式正确”。
- 保留本地加密备份和冲突快照，以应对远端误删、回滚和 WebDAV 服务不可用。
- 若所有已授权设备和恢复秘密都丢失，按设计无法恢复明文，不能提供绕过加密的后门。

## 8. 系统 hosts 与移动端边界

桌面端在同步后如需应用，必须调用 `crabhosts` 的安全流程，而不是直接写系统文件：

1. `system diff <profile>` 查看差异。
2. `system apply <profile> --dry-run` 生成预览，从该次成功 JSON 的 `data.plan_id` 取得 plan_id。
3. 向用户展示目标 profile、实际变更和 plan_id，获得该次明确批准。
4. AI/Harness 使用 `system apply <profile> --yes --expected-plan <plan-id>`；自动备份、原子替换和 managed block 保护由核心层完成，其他系统 hosts 保持不变。

restore 同样先以 `system restore <backup-id> --dry-run` 预览并取得当前 `data.plan_id`，再针对该 backup ID 和 plan_id 获得批准；AI/Harness 使用 `system restore <backup-id> --yes --expected-plan <plan-id>`。restore 在恢复前仍备份当前状态并原子替换，但恢复的是所选备份中的完整 hosts 快照，不限于 managed block。

SystemPlan 绑定 `operation`、`source`、`current`、`proposed`；任何相关变化均使写入退出 5。此时 apply 必须重新 diff/dry-run，restore 必须重新 dry-run，并取得新 plan_id 和新的用户批准。人工交互式 CLI 可以在确认后只使用 `--yes`，AI/Harness 不得省略 `--expected-plan`。不得因同步、远程命令或自动化任务而绕过这些步骤。

iOS、iPadOS 和未 root 的 Android 普通应用通常不能修改系统 hosts。移动端即使实现 profile 查看、编辑和同步，也不能承诺桌面端同等 apply/restore 能力；只能在平台明确授权的接口范围内提供能力，并清楚标注“仅同步 profile”。不得要求用户上传真实系统 hosts 来模拟移动端应用。

## 9. 分阶段路线

1. **模型前置**：实现并测试稳定 ID、revision、tombstone、schema 校验、本地原子写入，以及 active/apply 状态隔离。此阶段不宣称具备网络同步。
2. **WebDAV 首版**：完成条件 GET、ETag/If-Match CAS、离线队列、三向合并、冲突 UI 和真实 WebDAV 服务兼容性测试。敏感内网数据在 E2EE 落地前不应按“端到端安全”宣传或使用。
3. **E2EE 与设备管理**：加入 XChaCha20-Poly1305、Argon2id 恢复 envelope、配对、撤销、密钥轮换、审计和恢复演练，通过独立安全评审。
4. **可选 relay / P2P**：relay 只转发密文，不获得明文或系统应用权限；P2P 复用文档 ID、revision 和合并规则，但必须另行实现等价 CAS、身份认证和反重放，不能假设存在 WebDAV ETag。

每阶段的验收至少覆盖并发 412、离线重连、重复提交、崩溃恢复、删除复活、未知 schema、冲突保留、撤销后访问、恢复失败和“同步绝不自动写系统 hosts”。未完成这些验证，不应把对应阶段标记为完成。
