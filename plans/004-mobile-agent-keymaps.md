# 004 移动 Agent 键盘：功能分类草案

Status: Design approved by independent reviewer Hypatia; implementation complete, final verification in progress

本轮分类设计已完成独立评审；精确 byte 表、版本能力核实及实现由开发阶段补齐。设计批准不等于实现或发布验收通过。

## 推荐分类

保留“常用／导航／Fn／符号／ABC／操作”六个平级入口。仅在常用页内分组，不增加子页：

| 顺序 | 分组／键数 | 排序理由 |
| --- | --- | --- |
| 1 | 发送与排队／2–4 | 输入后立即使用；区分现在发送、稍后处理与取回修改 |
| 2 | 问题与控制／1–3 | 集中取消、问题导航和跳过，避免与发送混淆 |
| 3 | Agent 与模式／2–3 | 属于工作方式切换，与消息操作分开 |
| 4 | 编辑与查看／2–4 | 行首尾、历史、详情；其余留在既有页面或 custom |

Codex 最多14键，其他各不超过12键；缺少可靠动作便少放，不补齐格子。发送组优先露出，其余可纵向滚动。上述频率与触达排序是任务流程假设，未经用户实验；44 CSS px、目标间建议8px来自项目要求及 ui-ux-pro-max 设计启发，不能等同平台单位或实测结论。

## 四家关键动作

以下按参考文章的默认键组织，不能推定目标使用默认配置：

| Agent | 发送与排队 | 问题与控制 | Agent 与模式 | 编辑与查看 |
| --- | --- | --- | --- | --- |
| Codex CLI | Enter发送／运行中注入；Ctrl+J换行；Tab排队；Alt+↑取回或异步编辑 | Esc取消／中断；Alt+↓返回输入方向；Ctrl+]跳过聚焦问题 | Alt+←／→切Agent；Shift+Tab协作模式 | 行首尾、Ctrl+R历史、Ctrl+T转录 |
| Claude Code | Enter发送／运行中排队；Ctrl+J换行；C-x C-s立即发送；↑取回 | Esc取消／中断 | Shift+Tab权限模式；Alt+P模型 | 行首尾、Ctrl+R历史、Ctrl+O详情 |
| Pi | Enter发送／运行中引导；Ctrl+J换行；Alt+Enter追加；Alt+↑取回 | Esc取消／中断 | Shift+Tab思考级别；Ctrl+L模型 | 行首尾、历史或工具详情，择常用项 |
| OpenCode | Enter发送；Ctrl+J换行；C-x Enter仅v2排队 | Esc取消／中断 | C-x A列Agent；C-x M列模型 | 行首尾、↑↓历史 |

## 语义与版本

识别Agent不代表知道弹窗、光标、队列或输入模式。按钮同时显示语义和物理键，在按钮旁附必要条件；不随推测的内部状态换位。Codex Enter标注运行中注入，Tab标注排队；取回／导航只适用于对应异步上下文，不能保证一次取回；跳过不是批准权限。Claude Enter标注运行中排队，↑需在输入首行且可能先移动光标；立即发送指发送排队消息。Pi Enter在当前回复及工具调用结束后引导，Alt+Enter在当前任务完成后追加。

用户后续修订：键盘不显示任何版本提示、版本开关或额外确认。启用动态键盘且身份可靠时，所有对应映射直接可用；仅在设置的帮助文字说明快捷键可能随Agent版本或自定义绑定变化。不探测版本、不改Agent配置。

OpenCode使用文中当前默认映射；Tab／Shift+Tab切Agent存在版本差异，不统一命名。中断、清输入、退出分开；不新增退出、双Esc、批量停止等常用按钮。高级键保留现有导航／Fn／ABC／操作及custom入口。组合键一次点击发送完整序列、不长按重复；byte与实际目标兼容性待主线程核实。不改程序配置、不自动发送斜杠命令。

## 严格 opt-in 与状态边界

- 新选项默认false，旧设置缺失也为false。关闭时键盘没有动态分组、键位、排序或Agent提示。
- 开启时所有基础预设（含operations、editor、Vim／Neovim、Emacs及旧custom）均可临时叠加Agent常用区；其余页面保留，绝不写回保存布局。关闭或身份失效时完整恢复所选基础布局。
- 开启才读取活跃普通pane既有programStatus.app，或Herdr匹配pane／agent_session结构化身份。禁止标题猜测、后台pane代替、多Agent冲突时任选一个。
- 使用纯同步resolver：Herdr缓存必须有效、资源完整且selector匹配活跃视图，通过focused pane及关联结构化agent确定身份；普通pane只用自身programStatus.app。不新增Herdr pane.current、ensure或轮询。旧symbol-agent-controller自主请求与标题猜测路径退出键盘链路；本期符号页保持基础布局。全局program-status和Herdr既有检测不受影响。
- 未知、歧义、断连、退出、closing、回放或失效状态立即恢复基础布局。关闭、切pane／session／连接代次使旧回调与按压失效；晚到回复不得重新激活或发键。

## 触控、多语言与评审

固定快捷栏、系统IME及焦点策略保持；新按钮至少44 CSS px，窄屏文字可换行。滚动不发键；切布局取消按压／重复；兼容click不能重复发送，组合动作不自动重试。输入法组合期间不擅自提交或抢焦点。

设置、分组、动作、条件／设置帮助和aria接入现有en／zh-CN i18n；品牌名与Ctrl+J等物理键不翻译。运行时切语言须重绘当前视图，保留页面、基础布局与焦点意图，不发键。

独立评审重点：分类与键数是否合理；上下文提示是否足够；无版本UI是否符合最新要求；关闭及迟到回复是否完全静态。实施验收另须覆盖原生／Herdr、保存布局不变、无额外请求、触控滚动及中英文窄屏／横屏。精确 byte 矩阵已写入测试；实现以单元测试、浏览器场景和发布前独立审核为准。

参考：[主流编程 Agent 快捷键全集](https://czyt.tech/post/coding-agent-hotkeys/)（已提供的HTML表格）；当前仓库移动键盘与program-status模块。
