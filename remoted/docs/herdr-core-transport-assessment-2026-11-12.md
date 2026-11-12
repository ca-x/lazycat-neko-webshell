# Herdr 官方接入方式与 neoko-terminal 的差异评估

日期：2026-11-12。参考的是本地 `/home/czyt/code/rust/herdr-app`，
提交 `4c8c622d7965a6f6787f55b1896b70795fc68918`。这是一份源码评估，
不表示本轮发行已将旧 Herdr SockAPI 接入替换为 hc。

## 结论

保留 `原生 PTY`、`hc（herdr-core）` 和 `Herdr` 三类后端，统一上层接口与能力表达，
不要将 Herdr 偷换成 hc，也不要静默更改既有持久化选项。WebShell 和原生端可共用
hc 的连接与会话适配层。官方 herdr-app 已使用
`hc connect` 的分通道二进制协议，而我们当前的 Herdr 类型仍通过旧 SockAPI
读取 pane 屏幕后转发；现有普通终端的可选 hc 持久化后端已经使用 hc framed transport，
原生远程客户端通过 WebShell 主机也已经能使用该后端；本轮原生本机 host 已接入独立 hc 后端，包的实际交付状态另见验收记录。Relay 的 Noise 密文帧不是 Herdr 原生协议，不能把两者混为一谈。

收益来自统一会话所有权、事件订阅、消费后返还额度和明确的退出/断线语义，
不是来自将 JSON 单纯改成二进制。仍然需要终端渲染、输入、尺寸、显示事实和
权限管理，但应避免再实现一套由 core 已经拥有的终端生命周期与状态数据库。

## 官方实际实现

- `herdr-app/src/hc.rs`：通过 `hc connect` 建立单一 framed 连接；channel 0
  传 JSON-RPC，独立 channel 传终端原始字节。帧头为九字节，DATA、CREDIT、CLOSE、
  RESIZE、FOCUS、FACTS 和经能力协商的 AUTOMATIC_INPUT 分开表达。
- `Core::attach` 调用 `terminal.attach`，返回 channel 与 terminal 记录；输入和
  resize 经该 channel 发送。`events.subscribe` 返回初始 snapshot 与 seq，后续
  DATA 内的事件按行组装；错过事件时使用新的权威 snapshot 替换状态。
- `herdr-app/src/terminal.rs` 在终端处理输出后返还 CREDIT，区分连接 Lost 和
  进程 Closed；core 连接丢失后重挂同一终端。自动 VT 回复与用户输入有独立能力协商。
- `herdr-app/src/window.rs` 在 core generation 变化时重建订阅；工作区和布局文档
  经 core 文档接口同步。Agent 状态属于终端记录/事件和标签，界面负责呈现。

官方应用仍使用 Ghostty 终端渲染，并没有取消终端组件或只传聊天消息。

## 当前实现的确切差异

| 路径 | 当前机制 | 与官方的差异 |
| --- | --- | --- |
| WebShell 浏览器的 Herdr | WebSocket/SockAPI bridge、session/pane 接口及既有屏幕同步 | 浏览器不必经公共 Relay，也不是 hc channel 的透明客户端 |
| 经 WebShell 主机的远程 Herdr | `src/remoted/terminals.rs::attach_herdr` 通过 adapter 读屏、去重并发送 replacement output | 不是持续的 hc 原始终端流 |
| 原生桌面主机的本机/远程 Herdr | `remoted/host/src/herdr/mod.rs::screen` 每 250 ms 调 `pane.read`，有变化才 reset/转发屏幕 | 短时输出和滚动历史依赖读屏语义，存在额外轮询与状态转换 |
| 普通终端的可选 hc 后端 | `crates/hc-client` + provider/native 适配器使用 framed `hc connect` / `terminal.attach` | 共用有界 codec；provider/native 仍保持各自 namespace 与授权记录 |
| Relay | 端到端加密的 remoted v2 消息，精确 ACK 与有界队列 | 不理解 Herdr/core，不创建 PTY；其 ACK 也不是 core 的字节 CREDIT |

## 三后端的统一边界

| 后端 | 会话所有者 | 应复用的接入 | 对用户保持的含义 |
| --- | --- | --- | --- |
| 原生 PTY | WebShell agent 或原生 host 的 PTY registry | 现有 attach/input/resize/control 与 bounded replay | 不依赖 hc/Herdr 安装，可直接开 shell |
| hc / herdr-core | hc core | 共享 framed transport、terminal 身份与事件适配 | 可选的持久化终端；沿用现有后端设置 |
| Herdr | 对应用户的 Herdr 服务 | 独立 SockAPI allowlist、workspace/tab/pane 与 Agent 事件 | 保留 Herdr 工作区、布局、跳转与版本兼容 |

共同接口只描述 resources、attach/detach、input、resize、control、output、exit 和
program status。设备侧栏维持一个主机入口；普通终端下由创建时的后端选项区分
原生 PTY 与 hc，Herdr 继续按自己的工作区和 tab 层级显示。后端选项只影响新建会话，
已经存在的会话记录其原后端，重连不能按新的默认值改挂到另一个后端。
`supports_resize`、`persistent`、`workspace`、`files` 等能力必须来自
具体后端；界面不得由“都能输出终端文本”推断它们完全等价。只读与控制权在 host
检查；Herdr pane 的尺寸继续归布局所有，不因复用接口而开放外部任意 resize。

配对是“设备 ↔ 主机”关系，终端/工作区是该设备被明确授权的资源；不需要每个 tab
再做一遍设备配对。内部 attachment/channel 仍按真实会话隔离，不能为了减少绑定
记录而让多个设备共用一个未经区分的控制者身份。协议不承诺把 ANSI 输出变成聊天角色。

## 建议的落地方向

1. 将当前 `src/hc/protocol.rs` 的有界连接与能力协商抽成可供 provider/native host
   共用的 Rust crate；继续把 UI 的工作区/Agent 语义留在应用层。不要直接复制官方的
   无界接收队列，也不要把 GPUI 界面实现引入 NSH。
2. 使用完整的后端、目标 selector、登录用户、namespace、`core.id` 与 terminal address
   共同绑定会话身份，走 `terminal.attach`、`events.subscribe` 和 snapshot/seq 恢复。
   provider 当前使用按目标/用户隔离的私有 `HC_SESSION`/`HC_SOCKET`，不能假定它就是
   官方应用的默认 core。原生端仅连接自己选定的受控 namespace，不接管其他 core。
   `terminal.list` 与全局事件必须先按资源授权过滤。创建前保存客户端生成的 UUID intent；
   断线只 detach，创建结果不确定时查询该身份，不能重新 run。
   channel CLOSE 按 reason 分类；kept terminal 可以重挂。区分 detach、transport lost、
   channel closed 与已确认的进程退出。现有 `TerminalExit { code: i32 }` 不能表达缺失退出码、
   signal 或可恢复关闭；未经确认的 CLOSE/Lost 不伪造成成功退出。结束进程使用重新验证身份后
   的 `terminal.kill`，不能按缓存 PID 直接发送信号。
3. 本机端直接接该适配层；远程端通过现有 Noise/Relay 传输同样的授权操作与二进制
   数据。Relay 继续只中转；host 必须检查方法 allowlist、终端授权、只读/操作权限、
   channel 归属、帧大小和 generation，不能提供不受约束的 core socket 隧道。
4. 区分 core 字节 CREDIT、应用队列消费与 Relay 帧 ACK：当前 hc adapter 在输出进入
   有界应用队列后返还 CREDIT，Relay 在密文从有界 inbox 取出后 ACK，两者均不表示 GUI
   已完成渲染，更不表示操作执行成功。若需要按渲染完成计费，必须增加独立确认和字节账本。
   满队列不能丢字节后宣称流完整；慢 attachment 单独结束或隔离，控制、心跳和撤销仍可前进。
   未知结果的输入或创建请求不自动重试。自动 VT 回复必须同时满足 hello feature 与
   per-attach 的 AUTOMATIC_INPUT 协商；共享旧 codec 只接受 kind 0–5，不支持 kind 6 时
   必须明确拒绝，不能盲目回退 DATA。普通输入、自动回复、FOCUS 与 FACTS 都检查 channel、
   只读和控制权。hc 自己负责探针，不把输出送进 host PTY Replies；回放和多个 viewer
   不得产生额外回复。
5. 先统一终端数据面和身份，再对齐工作区文档、Agent 标签/事件和跳转；避免对官方
   layout JSON 做无条件双向覆盖。UI 仍区分普通终端和 Herdr 工作流。

## 兼容与验收范围

旧 Herdr SockAPI 的 pane/workspace id 不能直接当成 hc terminal address。三类后端始终保留；
探测决定是否可用，用户设置决定新建默认，原会话继续使用原后端。选择的后端不可用时明确报错，
不能静默回退或改写设置。连接 generation、hc hello API generation 和设备授权 generation
各有独立作用；hc 按 API generation/minimum/methods 协商，Herdr 按 minimum 与 SockAPI
allowlist 协商，target-local agent 按自己的 protocol/version 生命周期处理。
探测失败不构成自动升级、重启或接管的理由。遵守仓库的 Herdr 方向兼容与升级确认规则。
本轮不移动已有配对身份、应用 ID、签名、已发布 tag 或原有终端会话。

实际迁移前后应通过同一 core 上的本机/浏览器/原生远程互操作、断线重挂同一终端、
慢消费者流控、只读拒绝、设备撤销、真实退出码、短时大量输出和主题/尺寸/Agent
事件同步来验收。官方交互和协议都可借鉴；不能仅用一次能连上或 UI 截图声称迁移完成。

## 本轮已落地的边界

共享 codec 保留 hello/API allowlist、帧上限、调用/握手期限、可取消读取及有界写入。
原生 host 的 hc 记录在创建前持久化 UUID intent，绑定 namespace 与 core.id；重启只解析
原记录，不重发模糊创建。原生 PTY、hc 和 Herdr 继续独立列出和授权。
本机设置默认 Native，旧偏好缺少字段时按 Native 读取；损坏或未知值报错。写入成功确认后
才应用新默认，加载失败禁用创建，已有会话不切换后端。Android 不提供本机后端设置。

流中断使用 `TerminalUnavailable` 只结束该 attachment，保留客户端最后画面、禁止输入
和控制权操作，允许关闭标签后重开同一资源。未知退出状态不伪造数字退出码；明确退出
仍用真实 `TerminalExit`。它与资源撤销后的退出事件一样，须属于当前认证 session。
独立 fixture 和真实隔离 hc 验证记录见 `acceptance-native-v2-2026-11-12.md`。
