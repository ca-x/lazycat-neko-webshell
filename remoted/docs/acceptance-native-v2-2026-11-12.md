# neoko-terminal 桌面 0.4.1 / Android 0.3.0 验收记录

本轮源码版本为 WebShell 0.14.2、共享客户端及 Android 0.3.0/build 8、桌面发行包
0.4.1。原生应用固定 NSH 0.5.0 提交
`f4d68f7f9f31eb7b3ecc8a4cbc31096d5c6238a1`，Ghostty Rust binding
`cd4d497e8c698b27054a677ce19d1bd1a1a4d8bf`、Ghostty 内核
`108cacb17ed8990840cd14abfce43d8ed8503ca1` 和 Zig 0.17.1。
本记录只说明实际已执行的验证；安装包和商店交付以相应 Actions/Release 状态为准。
此前版本的记录保留在 `acceptance-2026-11-12.md`，不能将它作为本轮发行证据。

## 公共 Relay v2

实际部署地址为 `https://neko-relay.czyt.eu.org`。Worker version ID
`66c49549-1d1b-4b2d-bdbd-b4a81fe8c60a`，源码公开提交
`b576ca98b523e9eebd7bc1204a86d4cb826313ce`，构建时间
`2026-11-12T07:13:50Z`。根 GET 和 `/healthz` 已回读，确认包版本 0.2.0、
协议 `neko.remoted.v2` / version 2，根响应为不缓存 JSON。

Relay 通过 26 项 Node 单元测试和九项真实 workerd/Durable Object 集成测试。
覆盖精确逐帧 ACK、消费后返还额度、多 peer 隔离、关闭后迟到或重复 ACK 幂等、
非法/未来 token 拒绝和共享资源清理。ACK 只确认本地缓冲已释放，不代表业务操作成功。
本轮不兼容 v1，主机和原生客户端须一起更新。

## 实际网络和终端链路

下列测试均使用上述公共 Relay，非本机 Relay 替身。

- Provider 注册并测量 RTT：
  `deployed_relay_host_registers_and_measures_rtt`，一项通过。
- Provider 实际 Python 文件辅助程序：`public_relay_real_python_files`，一项通过；
  验证两块原子上传、列目录、下载、只读下载与上传拒绝、目录逃逸拒绝。
  下载的 37001 字节与上传 SHA-256
  `ea149eb67503edf6bfb7b1a5984b354d24e3bafa7daf9931dfdd11e5642a3a29` 一致。
- 原生 host/client：
  `public_v2_host_pair_shared_pty_readonly_revoke_reconnect_lifecycle`，实际运行约 33 秒，
  一项通过；最终 VT resize 修正后再次实际运行约 43 秒通过，验证生产 Host supervisor、Noise 配对和明确批准、真实共享 PTY、
  本机和远端控制权、断开重连保留同一 PTY、只读拒绝、关闭资源清 grants、撤销后拒绝重连。
  子进程显式传 `--include-ignored`，并校验确实执行了一项测试，避免 ignored 测试假绿。

Provider 文件验收在隔离 fixture 中将平台目标派发换成本机子进程；这不代表实际
LightOS 浏览器委托票据的完整验收。原生 host/client 验收使用真实本机 PTY和生产协议路径。

## 本机 Herdr 和 PTY 生命周期

使用实际安装的 Herdr 0.9.5 / SockAPI protocol 22、隔离 endpoint 和当前用户身份，
验证显式启动、读取屏幕、输入、两个不同 pane 的同时 attach、重复 pane 拒绝，以及
Noise 只读权限。未安装或未运行时，界面提供相应提示和显式启动入口。

本机 PTY 测试覆盖输出/回放、单控制者、detach 后保留进程、慢订阅隔离、真实退出码、
显式 close 以及晚到 attach/output/status 清理。标准 VT 查询由 live PTY reader 的唯一
Ghostty authority 回复，回放和客户端 renderer 不回复；真实 shell 的 DA、尺寸查询和
回放不重复回复已有集成测试。忽略 SIGHUP 的 shell 也由拥有未 reap child 的 waiter
通过原始 child handle 结束，不用裸 PID 作为延迟终止标识。

VT 修正时 host 56 项测试通过、两项显式手动验收忽略；加入三后端后最终为 72 项通过、三项手动验收忽略。live output、输入和 resize 由同一
有界 IO authority 排队；空闲程序启用 mode 2048 后也能立即收到 resize 报告。
create/resize 共用轴长与屏幕容量校验，超出容量的尺寸在改变 PTY 前被拒绝，
不会在下一次输出时停止读流。

## 三后端与默认设置

原生客户端最终 122 项测试通过、两项图形预览忽略，严格 Clippy 和 rustfmt 通过。
共享 hc codec 的 18 项常规测试及一项真实隔离 hc 测试通过；provider 的 27 项 hc
回归通过。host 另外显式执行真实 `/usr/sbin/hc` 0.1.20 验收一项通过，验证创建、
附着、输入、OSC 7501 状态、111×37 resize、host 重启后恢复同一终端、真实退出码 7
与结束后重挂，以及显式终止运行中的 PTY。fixture 覆盖模糊创建不重试、core 身份变化
拒绝、只读权限、慢消费者、状态表上限和撤销后的 stream 归属校验。

hc holder 丢失、未知退出状态与 worker EOF 使用 `TerminalUnavailable`，不伪造进程
退出；同一 session 的实际 Native PTY 仍可读写，旧 stream 被拒绝，重开不再 run。
客户端保留该流的最后画面，禁用输入和控制权，仅本地关闭其标签；晚到状态/控制事件
不能恢复已中断的流。

默认后端为 Native/hc/Herdr，仅用于本机新建，旧偏好按 Native 读取。测试验证错误偏好
不变成默认、写入未确认时仍使用旧值、远程不显示本机创建、Herdr 分类也提供 hc 引导
以及检测/连接期间禁止重复点击。真实 GTK 应用中已选择并保存 hc，原 Native 会话仍
可见；退出再启动后，恢复 hc 默认并显示相应连接引导。Android 隐藏本机默认设置。

## 已执行的原生界面检查

Linux GTK 原生程序在任务独有 Xvfb、隔离 DBus/系统密钥环和 XDG 目录中运行，
没有使用或改写用户的终端、桌面配置、系统剪贴板或凭据。

实际完成本机创建 shell、取得控制权、键盘输入并读取命令结果、切换到对话提交输入，
以及中文设置保存后退出/重新启动恢复。用户提交、主机确认和原始终端输出分开显示。
独立原生预览检查了浅色、深色、Herdr 和 390×844 窄窗口。
参考 Herdr 源码后的最终布局另做同样视口检查，不能将旧截图作为新布局验收。

本轮追加真实 GTK 深色中文、三项后端下拉及 390×844 浅色英文设置检查。UI 内部审核
发现的引导作用域、重复点击与空态文案已修正，并有针对性回归。该窄窗口属于 Linux
GTK，不能当作 Android 实机证据。新增状态使用现有语义主题和按压反馈，不额外添加动画。

## 发行和平台边界

私有 Actions 构建，公开 Actions 校验来源、版本、不可移动 tag、GitHub artifact digest、
包内容、Ghostty 输入和许可后发布；公开仓库不重建原生客户端。
Android 正式包必须保持生产证书、应用 ID、版本和 16 KiB 对齐。
Windows/macOS 包以各自原生 runner 的 CI 和包检查为证据，没有声称已在这些平台真机使用。

483 项终端主题来自既有 WebShell 主题目录，应用外观、主题、字号和中文/英文独立保存。
NSH 当前 Terminal trait 无法表达的鼠标协议、Kitty 键盘和图片等能力没有额外宣称支持。
本机 PTY 只在应用运行期间保留；退出应用不是后台 daemon hosting。配对记录没有闲置 TTL，
断连/撤销释放连接、订阅和控制资源，不会自动销毁仍被本机或其他设备使用的终端。

## 粘贴事件

用户于 2026-11-12 确认粘贴卡死是操作系统问题，重启已恢复；停止应用侧排查并删除临时诊断
测试，不将此事件写入软件修复清单。详细证据和限制见 `paste-diagnosis-2026-11-12.md`。

WebShell 0.14.2 私有构建 `43185111106` 已通过，公开发布运行 `43189700537` 已通过，
公开 Release 于 2026-11-12 发布为非 draft 的稳定版本，附 LPK 与 checksums.txt。
Android 与桌面包仍以之后各平台的实际构建和公开发布回读为准，不能据此记录声称已交付。
