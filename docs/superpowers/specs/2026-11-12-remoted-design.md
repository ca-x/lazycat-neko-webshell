# Remoted：中转、设备认证与 NSH 客户端

状态：待独立内部审核；审核通过并处理阻塞项后才实现。2026-11-12。

## 需求与交付

小橘增加独立“远程控制”设置分类及默认关闭的总开关（用户最新确认允许不做插件，采用此应用级方案）。内部模块名 `remoted`。小橘与原生客户端都先手动填写 Relay HTTPS/WSS 地址，连接成功后才显示配对操作；不预设公共服务，不从扫码内容悄悄切换中转。设置页显示连接中/在线/重连/离线及错误、已配对设备、权限、当前连接和延迟；支持多个设备、断开、撤销和权限修改。设备认证代替客户端对懒猫网络的依赖，不代替目标资源授权。

首版包括普通终端、Herdr 和文件传输。远程桌面是后续独立 capability，首版不暴露无实现的入口。移动端先 Android，NSH Rust 原生 UI，不使用 WebView。客户端提供扫码与手动设备码，两者走同一验证逻辑。发布小橘新版、原生客户端及公开仓库中的 Worker 项目和部署说明；不得用源码或占位 APK 冒充客户端产物。

## 调研来源与取舍

- Alma Relay 本地 `/home/czyt/code/ref/alma-relay`，调研 revision `30d56513aefc4d96b624d878af758aaa5468d9f8`。参考其主机主动 WSS 出站、客户端连接 Relay、Durable Object 按主机路由、断线清理和有界缓冲。其转发协议不是设备授权或 E2EE 协议，不能直接把中转 token 当小橘授权。
- Cindy 本地 `/home/czyt/code/ref/cindy`：参考设备身份、公钥绑定、申请/批准/撤销和连接权限分离，不引入完整账号/通知/媒体/租户网络系统。
- Nowledge TMP 记忆 `4305eaf4-8407-4983-854c-de456b2f3d53`；本地 `/home/czyt/code/rust/tm/docs/protocol-v1.md` 才是当前规范，旧 CBOR 草案已废弃。TMP 是无业务授权的可靠多路流内核，不提供设备身份、加密、拨号和持久化。本次连接层如采用专用 envelope，不得声称兼容 `tmp.pb.v1`；后续引入 TMP 必须明确协商新的承载版本，不能借用其子协议名。
- 本设计优先成熟 Noise 握手库（Rust `snow`），禁止自己拼 ECDH/AES 握手；同一协议 crate 给服务端、客户端及互通测试使用。

## 信任边界

1. Relay 只路由密文。不能看到终端内容、文件数据、设备私钥、配对秘密或 LightOS 票据。TLS 验证必须保留，生产只接受 wss/https；开发 ws 仅明确允许的 loopback。
2. Relay 管理员可以阻断/观察连接元数据，不能假冒已绑定设备，也不能批准配对。注册主机需要独立的 Relay 注册凭据，不出现在客户端邀请、URL、日志或状态 API。
3. 主机保存长期 Noise 静态身份，客户端独立生成身份。首次配对凭短期高熵秘密建立 Noise XXpsk3，手机展示等待批准；小橘显示名称、公钥指纹、申请时间，由用户显式选择权限与资源后批准。配对秘密使用安全随机源，单次、有效期 5 分钟，常数时间验证，最多一个活动邀请、有限待审批数。拒绝/超时/重启不自动批准。
4. 手输码编码主机路由 ID 和至少 128 位随机配对秘密，带版本和校验；可分组、可粘贴，不采用可离线猜测的六位密码。扫码只包含同样内容，不携带长期私钥、平台注册凭据或目标 ticket。扫描相机能力和权限须由实际 NSH Android API 验证；不得虚构接口。
5. 后续连接使用 Noise IK，客户端钉住配对时确认的主机静态公钥，主机只接受批准过的客户端静态公钥。新会话生成独立密钥/nonce，不恢复旧加密状态。握手 10 秒超时，重放/损坏/模式混淆即关闭，不回退明文。
6. 设备授权持久化到现有数据库的 remoted 私有命名空间。远程控制状态与设备列表不返回密钥。客户端身份使用平台安全存储；若当前 NSH 缺失，须实现窄范围 Android Keystore 适配，不把私钥写入可导出普通配置或日志。
7. 远程控制管理操作只能来自已有鉴权/CSRF 边界；中转不能调用管理 API。批准时记录平台授权者和完整资源身份（session ID、selector、login user、后端类别）；远端请求不能提交/覆盖这些字段。每次操作重新验证绑定身份和当前目标授权，禁止发现新 selector、转发用户票据或偷偷创建目标会话。

## 数据面与只读约束

一个主机 WSS 连接承载多个 peer，每 peer 独立 Noise 会话和有界任务；Relay 无法通过替换 peer ID 切换已认证设备身份。外层仅有版本、peer 连接标识、打开/关闭与密文；不实现通用 URL/HTTP 代理，不暴露本机端口。

端到端显式版本协商、请求 ID、capabilities 和结构化错误。终端输出、Herdr 事件、文件流可并行，输入/取消优先，文件采用分块+确认流控，不让大文件阻塞输入。最大消息 64 KiB、数据块不超过 32 KiB；最大设备/peer/请求/队列、握手和请求超时均为常量约束，生产默认值写进协议文档。

- **普通终端**：用户明确授权已有会话。只读仅观察输出和状态，不启动进程、发送输入、自动回复终端探测、调整尺寸或取得控制权。操作设备通过现有控制仲裁显式获得控制；每次输入/resize 都重新验证权限、目标身份和控制权。断网仅 detach，不 close PTY。重连保留 sequence 并报告 output_gap；replay 不重新跑程序探测或 OSC 回复。
- **Herdr**：在现有 allowlist 中再建立独立远程只读/写方法集合。只读仅快照、列表、读取和事件；不能直接把方法名前缀当权限。写方法按具体方法和目标授权。未知方法、全局 server.stop/live_handoff、任意 shell/agent 管理及绕过确认的 live handoff 不开放。保留较新客户端/较旧 server 的单向兼容边界，不把原生客户端版本当 Herdr 协议版本。PTY 与 SockAPI 适配器分开。
- **文件**：用户明确授权目标与根目录。只读允许浏览、stat、下载；操作允许上传，删除/覆盖需要显式用户意图。拒绝绝对路径逃逸、`..`、NUL、symlink 越界；目标端以 canonical root 和实际打开对象授权，不靠字符串前缀。沿用现有 target 文件边界，不能退化成主机进程文件系统访问。上传分块、有界 staging、最终校验及原子提交；中断/取消清理临时文件，未确认提交不报成功；不跨设备共享 staging。
- **后续远程桌面**：通过显式 capability/profile 另行协商 capture/input/clipboard，默认关闭，不让终端操作权限自动获得桌面权限。

权限变更/撤销须与输入/写入提交按设备串行化；写入前再次验证 generation，不能有“检查后等待再写”的撤销竞态。撤销永久删授权并断开所有该设备连接；断开仅关闭当前连接并轮换当前连接 generation，允许后续手动重连。关闭总开关关闭所有连接、废弃邀请和审批，但保留已绑定设备记录；删除设备才永久撤销绑定，重启不能恢复在途命令。无业务操作的自动重试（尤其输入、上传提交）以免重复执行。

## Cloudflare 部署

Worker + 单主机 Durable Object，使用官方 WebSocket APIs；可使用 Hibernation 但持久化/恢复 socket 标识必须验证，不能在休眠后遗失身份状态。部署配置需有 migrations、日期、注册 secret 配置、生产 TLS 入口、预算与限额说明。主机注册凭据与 CF API token 分离，Worker 只用前者。无服务器磁盘依赖、无 TCP listener、无环境代理或通用转发。主机断开向所有 peer 发送真实离线并关闭；客户端断开通知主机销毁 Noise/任务而非结束会话。重复主机连接按 generation 淘汰旧连接，旧 close 不得杀新连接；无无界离线消息队列。

**延迟**：小橘↔Relay 是主机自己的 nonce ping/pong RTT；设备↔小橘是端到端加密 nonce ping/pong RTT。最近测量超过 30 秒或断线即 `null`，界面显示“不可用”；不把 relay RTT 标成设备 RTT，不把上次在线时间当活跃。设备可能多连接，要明确显示各连接/聚合规则。

## UI / UX

远程控制设置遵循现有主题变量。默认只显示 Relay 地址表单和连接按钮；连接后紧凑显示状态/主机 RTT，配对码仅点击“添加设备”后展示。设备列表每行名称、指纹摘要、只读/操作、在线状态与 RTT，次要操作折叠；展开可选会话/文件根目录，权限变更和撤销结果就近反馈，危险撤销有明确对象。错误可恢复，不能用空列表掩盖加载失败。Relay 地址修改结束旧连接并要求重新连接，不能静默迁移已钉住的设备信任。

NSH 按 Design/Coding Guides 先建应用设计标准、语义 light/dark tokens 和可复用 styled controls，再建页面。手机 48pt 触摸目标、系统字体、安全区、IME 避让、NavStack/NavPage；桌面使用更紧凑的指针密度。任务层级为连接→配对→已授权会话（终端/Herdr/文件）。只读要有可读文本和禁写操作，不仅灰色按钮。终端占工作区，低频动作进菜单，状态不反复挤压输出布局。扫码失败可直接切手输，相机拒绝不阻止手输。

应用用户指定 ui-ux-pro-max / emil-design-eng / apple-design / animate / frontend-design。高频终端输入与输出不加动效；配对 sheet 仅在需要解释层级时使用短 transform/opacity 过渡并尊重 reduced motion。NSH 页面不生搬 CSS/DOM API。

## 验证与发布闸门

- 协议：两端真实密文互通、错误 static key/PSK/过期/重放/篡改/模式错配拒绝，多设备 peer 隔离，slow-reader/backpressure 有界。
- 授权：未批准/只读/撤销/禁用、selector/login变化、权限降级并发写、控制权竞争；验证所有 Herdr 方法分组；文件 traversal/symlink/取消/校验/原子提交。
- Relay：本地 Worker runtime 测试真实 WS 断开/重连、替换主机、休眠恢复（如启用）、超限、无主机、无注册凭据；部署检查和文档实测。
- UI：远程控制设置 light/dark 和手机宽度，配对/错误/延迟过期；NSH headless 行为、Linux 实际渲染和 Android 安装/相机/IME/前后台/断网恢复能力分别报告，不以 headless 冒充真机。
- 发布前跑仓库 fmt/clippy/Rust/frontend/release 既有检查。仅目标 agent 代码/行为/运行依赖变化才按 AGENTS 版本规则升级 agent。
- Android 真正生成可安装 APK，并验证应用标识、版本、ABI、签名和 16KiB alignment；签名类型明确说明。公开仓库独立组件名称 `remoted-client`，不得覆盖已有 client/server 产物。发布源代码/协议、Worker配置、部署文档、版本与 SHA256。小橘按现有 private build→public publish workflow，核实产物及双商店真实结果；不能把 workflow 总绿当官方提交成功。

## 实施前需要审核的边界

Noise 模式/配对码/Keystore 的互通性和可审计性；普通终端、Herdr、文件跨目标授权的可复用入口；当前 NSH Android terminal/camera/secure storage 的真实 API；CF 连接鉴权、配额和休眠语义；完整首版三能力是否有遗漏；公开仓库组件和 Android 构建发布能力。阻塞问题必须解决或显式记录真实环境阻碍，不能降级为没有数据面的空壳界面。

## 独立审核后的实现决策（B1—B7）

以下覆盖前文仍有歧义的描述。审核报告见同目录 `2026-11-12-remoted-review.md`。

### B1：授权与支持矩阵

首版管理者限定为当前部署所有者；不能由首个请求任意占领配置。平台模式必须有受平台入口投射的用户ID，与运行时部署所有者身份对应，不信任body/query自报身份。未能获得可信部署身份时管理端失败关闭；generic模式仅显式本机开发身份，不自动对公网开启。当前 owner 及其 app-owned session 资源构成持久设备委托，设备不能访问批准后新出现的会话。

批准时，当前真实请求通过既有 fixed-origin delegated discovery 核对目标属于该所有者，保存 source、完整selector、登录用户、session ID、backend 和随机 grant ID。首版只开放部署所有者自有 LightOS 实例和其显式配置启用的 SSH profile；不得将共享给该用户的第三方实例转成永久后台委托。后台对 grant 精确匹配当前 session/profile 身份，重新执行相应 selector/login/profile授权；身份变化/缺失/权限检测错误一律拒绝，不能用缓存成功代替错误。物理客户端和 MicroVM 的创建/附着依赖短期用户 ticket，本版明确标注暂不可委托并拒绝，不合成header、不保存ticket。三种业务能力均针对支持的目标提供。

### B2：固定握手与状态机

套件为 `Noise_XXpsk3_25519_ChaChaPoly_BLAKE2s`（首次配对）及 `Noise_IK_25519_ChaChaPoly_BLAKE2s`（已绑定重连）。客户端始终initiator，主机responder。PSK使用独立安全随机32byte原值，不截断不自造KDF。prologue为固定UTF-8应用标识、协议版本、mode及规范route的无歧义编码。握手payload必须为空，拒绝IK首条携带业务内容。端到端transport记录不超过61440byte（含JSON），Noise加密后不超过61456byte；外层envelope单条不超过96KiB，文件数据块最大32KiB。

配对顺序：当前邀请→XXpsk3认证完成→原子消耗邀请并登记已认证static key为pending→收到设备名称→主机用户核对公钥指纹并批准→先持久化设备+资源grant→发送加密approved（带主机static公钥/指纹与权限）→客户端将绑定安全持久化后发ack→host ready。pending只允许名称、等待、取消与保活，不能列资源。重连IK完成后核对已批准公钥、enabled、generation、grant与权限，发送ready才接受业务；客户端验证钉住公钥，不根据Relay声明身份。邀请/待批准在重启时全部失效，握手失败不消耗邀请。状态过渡和错误路径必须真实互通测试。

### B3：仲裁与观察

只读设备不调用 `TerminalControlHub.connect`，不参与connections和自动选举。可操作设备也先以观察者附着，只有用户显式“取得控制”才加入控制连接集合并调用take_control；断开/权限降级释放该connection。这样不改变浏览器原有自动仲裁。每次输入和resize仍检查当前controller与设备权限；观察模式不发resize。用测试证明只读首连、仅余只读和控制者断线的行为。

### B4：撤销线性化

每设备共享generation和提交gate，所有连接的写请求先进入有界队列。提交gate内再检查enabled/设备/权限/grant/目标身份后，交给具体目标transport；越过交付边界即“在途”，不能撤回保证。撤销先阻止新入队并增加generation，丢弃未提交请求、停止stream/下载、释放控制和终止上传；在途命令以真实确认/失败/结果未知报告，不自动重试。管理API有上限等待并返回在途数，不为等待远端无限持锁。文件commit只有越过同一gate才能发送；远端已收到rename时撤销不能声称未写入。

### B5：终端适配

以NSH已有 `TerminalEmulatorFactory` 包装VtEmulator，拦截并丢弃其Reply事件（用户Data保留），保持服务端唯一回复权威。共享增量UTF-8 decoder保留跨块未完成码点，真正无效字节采用确定替代策略，断流再flush；绝不逐块from_utf8_lossy。客户端在新附着epoch先reset再加载有界历史，不把仅sequence后的增量当成全屏恢复。已有权威端生成的原始输出允许正常渲染，但replay及本地emulator不回复探测。高频渲染无装饰动画。

### B6：Android平台适配

采用自有Android embedding宿主（app-owned Activity + NshView），不继承final AppActivity、不修改NSH本体。Rust/NSH负责UI和协议，JNI窄接口只负责扫码、Keystore、系统文件选择和Activity生命周期。先编译并运行最小平台验证，再接正式页面。扫码采用应用内相机解码，不依赖用户额外安装扫码App；相机拒绝和取消返回可恢复状态，保留手输。Keystore AES-GCM密钥不可导出，用于包裹Noise私钥后放app-private storage，禁止明文备份；重装/密钥失效要求重新配对。APK首版arm64-v8a/API28+，稳定包名和签名证书；不得把Android Debug签名描述成生产release签名。尚未验证平台能力时不得称已实现扫码或安全存储。

### B7：文件适配

不复用现有全量Vec上传作为流式实现。使用目标端固定Python3 helper（检测不到Python3时明确不可用），通过受控命令以目标登录用户启动；root打开成目录fd，逐段O_NOFOLLOW+dir_fd解析，路径中拒绝..、空/NUL及绝对路径；以fd为授权对象，禁止canonicalize后按原始字符串重新open。上传每设备/generation/grant隔离，32KiB分块、每块ACK、SHA256最终校验；staging在目标同文件系统私有0700目录/0600文件，exclusive创建，不复用其他设备临时文件。最终在已授权parent fd下rename或无覆盖提交，目标端确认后才报告成功。helper进程退出清理未提交staging，主机重启不恢复旧上传。

最大单文件4GiB、每设备2个传输、主机8个传输；不在内存累计文件。下载含offset/总长/最终hash，目录列表最大1000项并显式标记截断。取消/断线/撤销终止helper并清理，未知commit结果单独显示，不能重试rename掩盖歧义。

### I2/I3：Relay注册、生命周期与限额

主机本地生成并保存独立32byte routing secret，route是SHA256(secret)的规范base64url完整值。每次WSS注册仅Authorization头带secret，Worker解码验证hash后才能替换host。客户端只见route，错误secret不能使原host下线。无需新账号或用户额外录注册secret；可选部署级registration key只限制新主机登记（高级项），不进入设备码。Relay可见routing secret；其不能代替Noise设备身份。

首版不启用Hibernation：使用标准DO WebSocket生命周期，关闭/部署重建终止全部Noise会话并重握手，文档说明持续socket的duration费用。每route最多16个peer，主机最多32设备，8个待批准，16个同时握手；每peer发送队列8条，每host发送队列128条，超限拒绝/关闭不无界缓存；握手10秒，pending120秒，ping10秒，30秒无响应离线，退避1—30秒带抖动。消息/连接速率限制在握手前执行，Worker路由创建速率部署级限制，固定host单writer按peer公平排队。旧host close只能清理自己generation；替换host关闭全部旧peer，无业务消息离线缓存。

### I1：Herdr首版方法矩阵

只读（绑定整个已批准Herdr session，不提供暗含细粒度pane隔离承诺）：session.snapshot、workspace.list/get、tab.list/get、pane.list/get/read/process_info、agent.list/get/read、受限events.subscribe。事件流只允许当前授权selector/login/session的实例，不串其他目标。写：pane.send_text/send_keys/send_input/resize、agent.prompt；引用pane/agent必须先在同一target快照验证归属。首版不开放跨目标、任意方法、server.*、handoff、integration、plugin、shell或资源创建/删除。只读默认拒绝所有不在读集合的方法；可操作也只能用写集合，不是完整Herdr管理权限。

B1补充可信归属字段：使用现有 lzc-sdk `ApiGateway.package_manager().query_application(QueryApplicationRequest { deploy_ids, ... })` 的权威 `AppInfo.deploy_id` / `AppInfo.owner`。本应用精确的运行时 `LAZYCAT_APP_DEPLOY_ID` 查得owner，管理请求的受信 `X-HC-User-ID` 必须与之相同；不得把deployment ID当用户UID，也不得使用原有广泛env fallback猜测。LightOS selector的完整owner deployment也通过此API查owner，与批准者一致后才能签发grant；每次后台操作重查owner和目标登录身份，SDK错误/缺记录立即拒绝。初始discovery仅证明当前可见，不能单独证明拥有。SSH profile属于当前独立部署的应用数据，并检查启用及指纹/身份稳定。具体平台若不给应用查询所需记录，显示无法验证归属并拒绝，而不是绕过。

B7补充无覆盖与回收：无覆盖以目标同文件系统的 `link(staged, destination, *_dir_fd=...)` 原子创建（目标已存在即失败），成功后删除staged；不采用stat+rename。显式允许覆盖才使用基于已打开parent fd的原子rename。staging目录具固定版本命名和owner/权限验证，仅回收本实现格式、当前目标登录用户拥有、超过24小时且不在当前活跃集合的regular file，不跟随symlink，不递归删除非本实现内容；启动及有限周期执行。崩溃/SIGKILL留下的文件由此回收，不声称finally覆盖崩溃。

### 首版方法与界面验收收敛

共享远程 allowlist 移除 `events.subscribe` 与 `agent.prompt`：前者不通过一次性 RPC 假装提供订阅，Herdr 输出通过终端适配器持续传输；后者不在无法可靠验证 agent 对应 pane 控制权时开放。其余 pane 写入还要验证当前设备在对应 grant/pane 的附加终端上持有控制权，不能从直接 Herdr RPC 绕过仲裁。

管理状态公开主机公钥指纹供客户端核对，不公开密钥或 routing secret。管理界面展示撤销/中断后结果未确认的已提交操作，移除设备不清空该反馈。Android 采用真实 Activity 与系统控件，明暗主题包含原生标题栏；最后验证成功的 Relay 单独保存在系统保护存储，重启预填但不自动配对或更改 Relay。
