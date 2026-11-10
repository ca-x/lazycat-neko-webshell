# 来源与授权

本目录的离线识别实现来自 [J3n5en/voicekey](https://github.com/J3n5en/voicekey) 的 `crates/wtlocal`，
经作者同意后引入，并在本项目 README 中注明引用。

相对上游做的适配：

- 依赖：去掉 `rayon` / `memmap2` / `zip` / `tokio` / `async-trait`，
  改为标准库线程（`parallel.rs`）、整文件读入、系统 `unzip`、同步接口
- aarch64：`vdotq_s32` 需要 nightly，改用 stable 的 `vmull_s8 + vpadalq_s16`
- 接口：`AsrChannel`（本项目的识别渠道抽象），支持流式中间结果

模型是腾讯的资源包，**不随仓库分发**：首次使用时从官方 CDN 下载（约 100MB），
校验大小与 MD5 后解包，落在 `$XDG_DATA_HOME/wowtype/wtlocal`。