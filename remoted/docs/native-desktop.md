# neoko-terminal 原生桌面发行

neoko-terminal 是独立原生终端，桌面端可直接创建本机终端、hc 持久化终端、接入本机 Herdr，也可与 WebShell 或另一台原生桌面配对。远程 WebShell 连接保留普通终端、Herdr 和文件传输；本机桌面暂不提供文件共享。设置中的“默认会话后端”只影响本机新建会话；切换默认值不会改变已有会话。hc 与 Herdr 缺失时显示安装指南和连接/启动入口，不能静默回退到另一后端。
产品更名保留 `community.lazycat.neko.remoted` APP_ID 和系统凭据标识，不迁移或清除已有配对。
可执行文件与 deb 包改名为 `neoko-terminal`；deb 声明替换旧 `neko-remoted-desktop` 包。
macOS 目录为 `neoko-terminal.app`；中文系统显示“小橘终端”，英文显示 `neoko-terminal`。Linux 应用菜单也提供中英文名称。

桌面采用独立版本 **0.5.0**，发行源码 tag 为 `neoko-terminal-desktop-v0.5.0`。
本桌面构件使用共享 crate `0.4.0`，Android 按独立版本更新，既有发布 tag 不变。
桌面版本进入包名、deb/APP 元数据与 `build-info.json`；`crate_version` 记录共享源码的
0.4.0。`cargo_binary_sha256` 标识打包前的 Cargo binary（Mac ad-hoc 签名会改变 APP 内的 binary）。
桌面包通过独立发行流程发布，不使用 Android 的三资产 publisher。

## 支持范围

NSH **0.5.0** 固定 `f4d68f7f9f31eb7b3ecc8a4cbc31096d5c6238a1`，应用使用 Rust 原生入口
`src/main.rs`，分别链接 GTK4 / AppKit / Win32。无需 NSH JS runtime 或安装 NSH CLI。
NSH CLI 的默认桌面打包不包含应用自定义启动锁，且没有所需 deb，因此这里直接构建应用
binary 并按平台打包；没有替换或修改 NSH。

正式 `v0.5.0` tag 解引用到上述 SHA。`TerminalApi` 和 emulator trait 无破坏性
变更；普通终端及 Herdr pane 共用 Ghostty VT buffer，应用按原生 cell 的列、占格和
完整 grapheme 绘制固定网格。NSH Terminal 仅负责原生键盘、IME 和粘贴输入，不决定
共享 VT 的尺寸。禁止重复 feed 或由渲染端回复 PTY 查询。共享 buffer 的 trait 读取
不建立响应式订阅，避免 feed 持有借用期间同步重入行渲染。
Android IME 的验收边界见 [Android 文档](android.md#nsh-050-terminalinput-boundary)。

| 能力 | 桌面实现与限制 |
| --- | --- |
| Relay | 显式填写 HTTPS/WSS 地址，再检查连接；无默认公共地址 |
| 公共中转快捷输入 | “使用公共中转 / Use public relay”只填 `https://neko-relay.czyt.eu.org` 草稿，不连接、不保存、不切换绑定；已知 CF 403 人机挑战仍作为真实连接错误 |
| 配对 | 手输/粘贴设备码，主机显式批准；Relay origin 校验和原绑定不变 |
| 本机身份、绑定、上次 Relay | Windows Credential Manager / macOS Keychain / Linux Secret Service，持久存储；无明文或内存替代 |
| 大凭据 | 分块写入系统凭据库，再发布 generation manifest；兼容 Windows 单条 2560 字节限制；上限 64 KiB |
| 错误处理 | 凭据服务锁定/缺失、损坏 manifest、丢失分块均明确失败；只有完全不存在 manifest 才生成新身份 |
| 多进程 | 数据目录持有整个进程生命周期的文件锁，拒绝同用户同时启动第二实例，避免首次身份互相覆盖 |
| 剪贴板 | NSH 系统文本剪贴板，显式点击“粘贴设备码”；文本框保留系统粘贴。不新增远端剪贴板同步 |
| 扫码 | 桌面不请求摄像头；直接手输/粘贴独立可用；Android 扫码桥保持原样 |
| 上传 | NSH 原生单文件选择，opaque handle，64 KiB 分块读/seek，沿用 4 GiB 限制与哈希验证 |
| 下载 | NSH 原生保存对话框，同目录临时文件，全部写入并校验后 sync/原子替换；取消删除临时文件，不改旧目标 |
| 生命周期 | 关闭窗口结束桌面进程；不是 Android lifecycle 模拟。进程崩溃可能遗留非目标文件的临时下载，不会提交成完整目标 |
| 未启用的 NSH 功能 | 不开启 WebView/JS/shader 等默认可选功能；已有 Rust 终端、Herdr 和文件 UI 继续使用应用自身实现 |

Linux 需要运行且已解锁的 Secret Service，例如 GNOME Keyring。只安装 deb 不保证桌面会话
已启动凭据服务；没有服务时客户端明确报错，不悄悄生成临时身份。Windows 的系统凭据按登录
用户隔离；macOS 用该登录用户的 Keychain。Unix 应用数据目录为 0700，Windows 使用用户本地
应用数据目录并继承用户 ACL；该目录只保存进程锁，私钥不落明文文件。

## 构建和产物

在对应 OS 安装 Rust 1.97.1、Python 3.12；Mac 还需 Xcode Command Line Tools。
Linux 构建依赖（CI 为 Ubuntu 24.04）：

```sh
sudo apt-get install libgtk-4-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  libdbus-1-dev pkg-config dpkg-dev gnome-keyring dbus-x11
rustup target add x86_64-unknown-linux-gnu
python3 remoted/client/scripts/build-desktop.py --version 0.5.0 \
  --target x86_64-unknown-linux-gnu
```

Windows x64 使用 MSVC 和 Windows SDK（GitHub windows-2022 已有）：

```powershell
rustup target add x86_64-pc-windows-msvc
python remoted/client/scripts/build-desktop.py --version 0.5.0 --target x86_64-pc-windows-msvc
```

Mac 两个 CPU 分别打独立包；NSH 使用 macOS 14 API，最低系统版本 **14.0**，不是 12/13：

```sh
export MACOSX_DEPLOYMENT_TARGET=14.0
rustup target add aarch64-apple-darwin x86_64-apple-darwin
python3 remoted/client/scripts/build-desktop.py --version 0.5.0 --target aarch64-apple-darwin
python3 remoted/client/scripts/build-desktop.py --version 0.5.0 --target x86_64-apple-darwin
```

默认输出 `remoted/client/target/desktop-dist/`，每个目标生成包、独立 build-info 和 SHA-256：

- `neoko-terminal-0.5.0-x86_64-pc-windows-msvc.zip`，内含 EXE、安装说明和许可证。
- `neoko-terminal-0.5.0-aarch64-apple-darwin.zip`，内含 `neoko-terminal.app`。
- `neoko-terminal-0.5.0-x86_64-apple-darwin.zip`，独立 Intel APP。
- `neoko-terminal-0.5.0-x86_64-unknown-linux-gnu.deb`，含 desktop entry、图标、许可证。

Windows 运行前需安装 [Microsoft Visual C++ Redistributable x64](https://aka.ms/vs/17/release/vc_redist.x64.exe)；当前 EXE 导入 `VCRUNTIME140.dll` 和 `VCRUNTIME140_1.dll`，ZIP 未附带这两个运行库。Windows 未签名，可能触发 SmartScreen。Mac **仅 ad-hoc 签名，没有 Developer ID、没有公证**；
Gatekeeper 可能要求系统设置中的“仍要打开”。不能把 codesign verify 通过称为官方签名或公证。
Linux deb 用构建机实际 glibc/GTK/GLib/GStreamer 版本声明保守运行下限；本机 Arch 构建仅供本机
提取启动验证，不代表 Ubuntu 可安装。发行用 CI Ubuntu 24.04 包。安装 `sudo apt install ./包.deb`；
退出程序、卸载包不会主动删除系统凭据，重新安装可继续使用原身份。

## CI 与 tag 发布

`.github/workflows/neoko-terminal-desktop.yml` 由 `neoko-terminal-desktop-vX.Y.Z` tag 自动触发，
也可从同一固定 tag 手动重跑，输入版本必须与 tag 一致。四个独立并行作业使用
`fail-fast: false`：Windows x64、Mac arm64、Mac Intel、Linux amd64。
Android 使用独立 `neoko-terminal-android-vX.Y.Z` tag，不触发桌面或 WebShell LPK。

提交源码并推送固定 tag 后自动构建；需要时从同一 tag 手动重跑：

```sh
# 完成 commit/tag/push 后
# workflow 必须已存在于默认分支，才能从该 tag 手动 dispatch。
gh workflow run neoko-terminal-desktop.yml --ref neoko-terminal-desktop-v0.5.0 -f desktop_version=0.5.0
gh run list --workflow neoko-terminal-desktop.yml --limit 5
gh run watch RUN_ID --exit-status
gh run download RUN_ID --dir /tmp/remoted-desktop-ci
```

每个作业先运行客户端 lib tests，再 `cargo build --locked --release --target` 打包。Mac 在
macos-15 runner 上指定 CPU target，两个包都显式构建；不同 runner CPU 不表示产物 CPU。
发布前必须检查四个 job 实际成功，而非看到 workflow 文件就宣称 Windows/Mac 已构建。

桌面端统一发布稳定正式包：使用 Release 构建，Windows x64、Mac arm64、Mac Intel 和
Linux amd64 deb 必须同时交付。最终 GitHub Release 必须为非 draft、非 prerelease；
内部构建产物和暂存草稿不能当作发布完成。

公开 Actions 从私有仓库读取构建产物，不重新编译客户端；核对固定 source tag、成功 run
的 head SHA、四个 build-info 的 `source_sha`/`source_dirty=false`、版本、NSH revision、
target、包内元数据与 SHA-256 后，发布独立 desktop release。校验覆盖 Mac 代码签名页、
Windows PE x64 与 ZIP 内容、deb 元数据和二进制；发布后回读公开附件。
不要让权限不足的 SOURCE_REPO_TOKEN 静默退回 broad OAuth，不移动已有 tag；失败构建
保留原版本记录，代码修正后使用新的补丁 tag，也不覆盖既有 Android 资产。

## 定向验证

```sh
cargo test --manifest-path remoted/client/Cargo.toml --locked --lib
cargo clippy --manifest-path remoted/client/Cargo.toml --locked --all-targets -- -D warnings
actionlint .github/workflows/neoko-terminal-desktop.yml
python3 -m py_compile remoted/client/scripts/build-desktop.py
```

客户端测试包括公共中转草稿不自动发命令、不被异步已保存偏好覆盖、原绑定保留、中英文按钮；
凭据 Unicode 大绑定/Windows blob 限制、失败更新保留旧身份、丢块不当作首次安装；分块读/seek、
句柄方向、取消下载保留目标、commit 才替换目标。

真实 Secret Service 测试默认忽略，必须在独立 D-Bus + 独立解锁 Keyring 中运行，测试使用随机
`desktop-test.*` key，结束删除。示例（临时 home 只作用于测试子进程，不改宿主环境）：

```sh
# 先 cargo test --lib --no-run，再从 target/debug/deps 找到可执行的 neko_remoted_client-* 测试 binary。
# 在以下隔离会话中直接运行该 binary，避免临时 HOME 影响 rustup/Cargo 查找：
test_home=$(mktemp -d)
HOME="$test_home" XDG_DATA_HOME="$test_home/data" dbus-run-session sh -c '
  printf "desktop-test-password" | gnome-keyring-daemon --unlock --components=secrets
  "$1" platform::desktop::credentials::tests::real_secret_service_survives_entry_recreation --ignored --exact
' sh /absolute/path/to/test-binary
```

本地 deb：用 `dpkg-deb --info/--contents/--extract` 检查真实包，从提取目录启动 binary；在 GUI
确认连接页可见、首次未自动连接，验证第二实例拒绝。完整配对和跨 OS 真实键鼠/文件对话框验收
仍需目标机器；Linux 本地构建/Headless 测试不能代替 Windows/macOS CI 或真机验证。


## 界面参考与任务布局

本轮参考 Herdr 官方原生应用 `herdrdev/herdr-app`，本地只读检出位于
`/home/czyt/code/rust/herdr-app`，参考提交
`4c8c622d7965a6f6787f55b1896b70795fc68918`。主要对照
`defaults/layouts/main.json` 和 `defaults/mods/` 中的 spaces、agents、tabs：
侧栏组织设备和工作区、底部定位 Agent，主区使用单行标签并尽量留给终端。
实现继续使用本项目的 NSH 组件和语义主题，没有引入 Herdr 的 GPUI 界面实现。

所有会话统一使用终端交互，没有 Chat 模式切换。桌面直接键盘输入，输入栏按需展开；
移动端默认展开适合触控与中文输入法的输入栏，保留 Esc、Ctrl+C 等快捷键。
Herdr 的纯文本快照会单独规范换行，并保留源列宽；普通 PTY 原始字节不会经过文本规范化。

原生包内嵌 WebShell 同款 Hack Nerd Font Mono、Symbols Nerd Font Mono、Maple Mono NF CN，
无需联网下载或用户安装字体。源码复用 WebShell 的原文件，包内附带授权文本。
主机别名只保存在本客户端，可以离线编辑；修改别名不会重新配对或中断会话。
