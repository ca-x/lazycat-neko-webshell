# Remoted 独立设计审查

2026-11-12；审查对象：`2026-11-12-remoted-design.md` 当前稿。只做设计审查，没有修改实现，没有运行功能测试或生成 Android 产物。

**结论：架构方向可行；当前稿尚不能作为完整实现规范。下列阻塞项落实到设计和验收项后可进入实现，不需要等待发布阶段才发现平台/API缺口。** 用户确认采用内置“远程控制”设置分类和默认关闭总开关；首版必须同时包含普通终端、Herdr、文件，不可缩减为只有现有 PTY。

## 阻塞

### B1 平台授权主体与后台委托机制没有闭合

证据：`src/lightos_admin.rs:228` 从请求取得平台用户；`src/client_terminal.rs:601` 的物理客户端授权依赖请求身份及可见实例；`src/herdr.rs:887`、`src/terminal.rs:2923` 则经 LightOS selector/login 授权；`src/lightos.rs:494` 通过 `lightosctl ps` 枚举。上述入口不是同一种授权来源，不能把主机进程可见性当成某个配对者的权限。

必须定义：管理 API 的受信平台身份来源、配置/设备记录按何用户或单用户部署范围隔离、批准时如何验证资源属于该授权者、后台每次操作如何确认授权仍有效。设备只能引用主机签发的 opaque grant ID，不能传 selector/loginUser 来重建授权。不得伪造平台身份头、保存用户 ticket 或退回全局 root 资源枚举。无法无票据重新授权的目标类型必须明确拒绝并显示原因；不能承诺物理客户端/MicroVM 的现有流程会自动适用于本中转。为 LightOS/SSH/物理客户端/MicroVM 列支持矩阵，明确首版普通终端后端范围，同时保证三种业务能力真实可用。

### B2 Noise 握手与批准状态缺少可互通定义

XXpsk3 + IK 可以作为候选，尚未发现选型本身不可用；但模式名不足以构成协议。固定完整 suite（例如 `Noise_XXpsk3_25519_ChaChaPoly_BLAKE2s` / 对应 IK）、双方 initiator/responder、PSK 32字节的生成/派生规则、握手帧编码、prologue（应用/版本/模式/路由）、握手与 transport 长度边界及 nonce 生命周期。若配对秘密只有128bit，必须明确到32byte PSK的带域分离派生，或直接使用32byte随机秘密，不能把16byte直接塞给库。

必须画状态机：邀请→握手完成→待批准→原子持久化绑定→端到端批准确认→可用。IK 第一条消息可能携带应用数据，完成身份检查和授权前一律不执行；不得把解密成功当成批准。邀请单次消耗应在认证成功后原子完成，不能让无效请求烧掉邀请；批准绑定已认证公钥与当前邀请generation，拒绝/超时销毁待配对会话。客户端应校验/显示主机指纹并钉住，主机显示客户端指纹；不要只按设备名称批准。跨模式/旧邀请重放与主机重启必须有互通测试。

### B3 只读连接直接复用 TerminalControlHub 会获得控制权

证据：`src/terminal_control.rs:57` 首连接自动成为 controller；`disconnect`、`release_control` 会从剩余 connections 中自动选 controller，当前连接集合没有“只读资格”字段。仅拒绝只读设备的 take_control 不足够，也可能使浏览器正常连接失去控制。

必须让观察连接不进入可选 controller 集合，或扩展仲裁资格并让所有自动选举遵守资格；写设备也按显式申请规则接入。权限降级必须释放控制并通知其他浏览器/远程连接。针对“只读首先连接”“控制者断线后仅余只读”“降级后自动交接”做测试。

### B4 撤销的承诺超出了当前本地 generation 能保证的范围

把检查和远端写入按设备串行化，仍不能撤回已经发送给 target 的 Herdr命令/PTY字节/rename。禁止用本地检查后的generation声称已经取消远端副作用。需要写明线性化边界：进入目标提交的操作视为在途，撤销阻止后续入队、取消尚未提交工作、停止事件/下载流并释放控制；撤销确认须说明在途操作状态，不能报告保证未执行。跨多个该设备连接必须共用授权generation；关闭总开关也走同一栅栏。不要为等候任意远端调用无限持锁，导致撤销按钮永久挂起。上传最终commit与权限栅栏必须组合设计，测试撤销/降级与排队输入、慢上传、Herdr调用并发。

### B5 NSH 终端适配尚未解决任意字节、自动回复与重连画面

已核对 pinned `nsh` `0e4a20b`：`crates/nsh-base/src/term/mod.rs:90` 的 `TerminalRef::write` 接收 String；`:142` 把 `TerminalEmulatorEvent::Reply` 与用户输入送到同一个发送入口。网站 Terminal 文档也明确 onData 包含查询回复。不能把 onData 全部转发给主机，这会违反服务器唯一回复权威/replay不答探测，操作设备同样受影响。

必须选择可执行适配：使用已存在的 TerminalEmulatorFactory 包装 emulator、过滤 Reply（始终保留服务器权威），不能靠字符串猜测用户输入是不是查询回复；验证 Rust真实 trait/API。传输保留原始bytes；到String视图使用跨chunk的增量UTF-8解码，明确无效字节处理；禁止逐chunk from_utf8_lossy 导致合法拆分码点损坏。二进制文件绝不经过字符串终端链路。重连需区分同一存活renderer续传与新renderer初始化，明确快照/有界重放及output_gap，不能只有cursor却没有重建画面的内容。OSC7501/status仍由现有服务器权威处理。

### B6 Android 需要自有 embedding 工程，常规 nsh package 不能满足扫码/Keystore

已核对 `website/docs/packaging.md:320`：NSH没有camera高层API，标准packaged Android runtime只声明 INTERNET且不能添加Java代码；`examples/embed-android` 提供自有Activity + NshView路径。现有 AppActivity是final，不能设计成继承它增加扫码。没有验证到NSH提供设备身份Keystore高层接口。

修订为应用自有Gradle宿主、manifest、camera runtime permission、CameraX/扫码库或其他经验证相机方案，以及窄JNI桥接；NSH用于原生UI。Noise的X25519私钥若需供snow读取，可用Android Keystore中的不可导出AES包装密钥加密后保存到应用私有目录，**不能承诺X25519私钥自身永不离开Keystore**，除非已验证snow自定义crypto resolver和对应平台密钥能力。禁用身份数据云备份/设备迁移，处理重装、密钥失效与注销，不能用APK签名keystore代替运行时设备密钥库。先完成最小Android包验证JNI、终端IME、相机结果、密钥持久化，再扩展完整UI；未验证部分不得称已支持。

### B7 文件根目录隔离及提交需要目标端能力，不能直接套现有接口

证据：`src/plugin/resources/files.rs:27` 重新resolve目标并授权，`TargetFiles::execute` 接受path；`src/plugins/file_transfer/uploads.rs:15` 的staging为内存Map，`:95` 将chunk累计到Vec，隔离键主要是session。它们不等价于受设备/根目录约束的有界流式文件系统。

必须定义目标端root-handle或等效无竞态策略，原子防symlink替换/重命名逃逸，不能canonicalize后再按原字符串open。若目标只有shell/python helper，明确实现、支持范围和验证，而非只要求“canonical root”。上传标识绑定设备+授权generation+target+root+目的路径，限制单文件/总staging/并发与超时，使用同文件系统原子提交。定义下载offset/总长度/校验、目录列表上限、取消及应用重启的staging清理，避免使用内存全量buffer假称流式传输。安全修复若必须改变target agent，按AGENTS独立版本规则处理。

## 重要

### I1 Herdr 方法与嵌套资源必须列完整授权矩阵

`src/herdr_socket_contract.json` 是兼容allowlist，不是远程权限矩阵。实现前列出每个首版允许的方法、读/写类别、参数内资源引用（workspace/tab/pane等）、目标绑定方式和事件过滤范围。未知方法默认拒绝。list/events/快照不能泄露未授权工作区；禁止允许的写方法通过嵌套参数影响未授权pane。明确Herdr终端输出、输入和resize对应的真实bridge接口，不能只做SockAPI列表页宣称Herdr交互完成。live_handoff/server管理保持禁用。

### I2 CF生命周期与限额需落实到数字和恢复策略

当前“常量约束”没有生产默认值。建议先定义每主机设备数、每设备连接数、握手并发、pending批准、每流inflight块数、主机总缓冲bytes、每秒消息/bytes、请求/握手/空闲超时；计算最坏内存。64KiB必须明确是明文、Noise密文还是外层WS消息，Noise tag和envelope有开销，不能把64KiB明文直接视为可接受的单条Noise消息。

一个host WebSocket只能有一个Writer调度，分peer/stream有界队列并公平调度，避免慢peer拖垮所有设备；quota必须在Noise握手前也生效。Cloudflare 休眠不是耐久业务会话保证：若首版不实现attachment恢复，可明确不启用hibernation；若启用要存route/role/generation且测试重建。主机重复连接、旧close、部署与DO重建都必须让端到端会话重握手、旧任务终止。文档区分免费/付费计划、WS消息计费、心跳防休眠的duration成本和实际限制；不要作免费/无限连接承诺。

### I3 Relay登记新候选可采纳，但需明确抗抢占边界

**可以使用本机随机32byte routing secret，route=SHA256(secret)，host仅在WSS握手Authorization中提交secret，Worker解码后常数时间检查hash。** 这能在TLS及secret保密前提下防止只知道route的第三方抢占相同路由，不需客户端账户，也不强迫用户录额外凭据；Noise仍承担设备身份和E2EE。route建议使用规范编码的完整256bit hash、严格长度限制，secret必须独立于Noise私钥/配对PSK，主机持久化且不出现在邀请码、URL、客户端、日志、状态API。

这不是防恶意Relay方案：Relay能看到routing secret并能阻断/冒充路由状态，但不能因此绕过客户端钉住的Noise主机身份。原稿禁止Relay看到配对秘密、设备私钥和平台票据，这与本候选不冲突；应补充明确路由凭据是Relay可见的传输鉴权凭据，不能把所有secret混为一谈。 Worker不能提供“先注册route所有权”的无证明入口；每次host连接都验证hash，凭正确secret替换host并生成新generation。缺少/错误secret不能让现有主机下线。添加可选部署级registration key限制新路由创建，且已有路由重连与key轮换规则明确。开放自建服务若无registration key，仍需限制新DO创建/匿名连接/已知route的握手洪水，并说明费用滥用风险。手输设备码含route和PSK，暴露route即允许向主机发起握手，不应把route当匿名客户端的防滥用凭据。

### I4 发布范围需增加真实Android构建与更新连续性

原稿正确要求公开独立remoted-client且不覆盖已有client/server。再补：首版明确Android ABI/最低API/版本号/包名；长期稳定release签名和后续升级路径、固定NSH revision、Gradle/NDK/JDK、camera依赖许可证、共享协议来源；公开源码和Worker项目不能引用私有仓库绝对路径。APK通过可安装、签名/16KiB alignment验证，真机未验与编译通过分开报告。Linux客户端若也承诺发布，应列明确artifact与验证；未产出不能称桌面全平台已发布。缺少CF账户不妨碍提供可部署项目，但不能宣称完成线上CF部署。

## 建议

- 设置模块使用 `src/frontend/src/remoted/` 等应用级目录；状态/设备/邀请与管理API分离，不再以plugin开关做第二层开关。原生端与小橘都先填relay，再显示配对；高级部署key默认折叠。
- 邀请包含版本、route、PSK、校验，不自动替换当前Relay；建议包含不泄密的relay origin摘要用于发现双方填错地址。主机指纹、错误恢复、过期和审批取消均有就近文本提示。
- 设备“断开”与“撤销”文案区分；断开允许设备重连，不能显示成永久禁止。RTT显示测量时刻/过期，后台Android不能假显示在线；只读文本与控制权状态分别呈现。
- Noise pairing状态持久化、撤销、协议版本和三类业务流先做共享crate互通测试，UI再接状态机；不要把未核实API写进漂亮但无法运行的界面。

## 进入实现条件与审查证据

把B1—B7写成明确决策及验收项（其中Android用最小验证工程落实API路径），然后可进入分阶段实现；所有阻塞不必一次写完产品代码，但不能保持“以后再决定”即开始完整实现。I1—I4应在对应阶段前落实，发布前再次独立审核真实代码与产物。本次没有代码变更，不代表实现已通过安全审核。

直接阅读了设计稿、AGENTS、上述Rust授权/terminal/files入口、Herdr contract、本地Alma Relay `cloudflare/worker.mjs`（存在hibernation/close处理和不同协议流式路径），以及pinned NSH terminal源码与Android打包/embedding文档。未验证snow实际互通、Android编译/运行或CF运行时，本文不声称它们已可用。

Nowledge定向检索：CLI `nmem --json m search 'TMP 协议 remoted 授权' -n 3`，服务返回模式`fast_bm25_vector`；返回内容主要为LightOS历史，不能据此证明TMP协议兼容。TMP约束以主任务已提供的`4305eaf4-8407-4983-854c-de456b2f3d53`及当前规范为准，没有把历史记忆当新协议的认证方案。本次没有写入推测性记忆。


## 实现复审记录（2026-11-12）

- 协议 / Relay / 传输独立审查发现的准入表到期回收、每客户端额度隔离、超时后迟发问题已修正，并通过协议、真实 workerd 和网络回归验证。
- 主机复核覆盖平台部署归属、管理鉴权、资源 grant、只读与控制权、文件根身份、撤销与在途提交。数据库在 SQLite 打开前收紧 DB/WAL/SHM 为 0600。最后两项 P2 已增加跨挂载上传开始前拒绝、传输取消优先及已提交变更等待结果的处理；工作区 709 项测试（8 项忽略）、Python 15 项、strict Clippy 均通过，等待独立定向复核。
- 前端后台 GET 不再保留看似可用的动作；输入草稿与焦点保留。授权失效时清理设备、审批、邀请、身份指纹、操作状态及密钥草稿。独立复核通过，748 项前端测试和浏览器受控 fixture 通过。
- Native 只读 / 未取得控制权的终端没有可编辑 Input，观察器与可输入终端共享实际 ANSI 状态；普通文件错误保留工作区；支持移除本机绑定。35 项原生测试通过。
- Android 0.1.1 的签名 APK 已在 API 35 模拟器通过九组平台检查；云端发布构建正在执行。0.1.0 构建因 runner 缺少 SDK 初始化失败，未公开发布，标签未移动。
- 上述测试不能替代真实懒猫部署端到端、Android 真机 IME / 物理扫码、云文件提供方及 4 GiB 压力验证。CF 只完成本地真实 workerd 和可部署源码验证，没有代用户部署线上服务。
