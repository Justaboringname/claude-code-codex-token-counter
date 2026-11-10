# Token Counter

[English](README.md) | **简体中文**

https://github.com/user-attachments/assets/081f429b-0f37-4331-abba-3be8cead6b33

最后更新：2026-11-10

一个 macOS 菜单栏弹窗 app，显示 **Claude Code** 和 **OpenAI Codex** 用了多少 token、按标准 API 价格折算值多少钱，以及两家官方的 5 小时 / 每周用量限额。token 和花费的历史来自本地 JSONL；Claude 官方限额那一行会向 Anthropic 发一次请求，Claude Code 的登录凭证过期时，会在钥匙串里把它刷新。ChatGPT 订阅的用量受套餐限额和 credits 约束，并不是按这里显示的美元估算直接计费。

Claude 的模型用暖色的**橙色**色阶，Codex/GPT 的模型用冷色的**蓝色**色阶。

## 背景

我在 Claude Fable 5 发布之后写了 Token Counter，用来看自己到底烧了多少 token。这个东西对个人来说没什么用，对公司会比较有用：想知道每个员工用了多少 token，每人在自己的 Mac 上跑一个就行（目前还没有团队面板）。

Claude Fable 5.2 发布之后，它多了两样东西：

- **云端同步**：Mac 上显示的内容，在手机上也能看。
- **原生 iOS app**：由 Fable 5.2 编写，用了 Liquid Glass、会动的壁纸这些比较 fancy 的东西。

## 功能

- **两家并排，一个计数器**：Claude Code 和 Codex 放在一起，按模型的进度条、双线走势图（橙色是 Claude，蓝色是 Codex）、合并的工具网格
- **按时间范围看花费**：Session / Today / Week / Month / All-time（Session 指两家里最近活跃的那个会话）
- **按模型汇总**：当前和历史的模型 ID 都有明确的 2026 年价格（GPT-5.6 的 Sol/Terra/Luna、GPT-5.4 的 Pro/mini/nano 各档分开计）；缓存命中的输入和未缓存的输入分开计价
- **Claude 官方限额**：来自 Anthropic `/api/oauth/usage` 的真实 5 小时和每周使用率，缓存 60 秒，请求失败时显示上一次成功的结果
- **Chat、Cowork 和 Claude Duet 的估算**：这三个产品不留本地日志，所以 Week、Month 和 All-time 用官方的每周用量占比（`seven_day_breakdown`）对照当周 Claude Code 的花费来估算；app 没读到过的那一周不计入
- **Codex 官方限额**：5 小时和每周的 `used_percent`，直接取自 CLI 自己的限额数据（不是估算）
- **最常用的工具调用**网格（Read、Edit、Bash、Grep……以及 Codex 的 Shell、Apply Patch、Plan），带调用次数
- **14 天走势图**：每天的花费，以及和昨天相比的变化
- **自动刷新**：监听 `~/.claude/projects/` 和 `~/.codex/{sessions,archived_sessions}/`，有新事件就立刻更新；另有 15 秒一次的轮询兜底
- **原生液态玻璃**：用 macOS 的 `NSVisualEffectView`，没有 CSS backdrop-filter 的闪烁
- **深色模式**：和原生窗口外观同步（跟随系统，或手动指定）
- **窗口高度随内容变化**：`ResizeObserver` + `setSize`
- 可以从顶栏**拖动**，从任意边**调整大小**
- **云端同步 + 手机网页 app**（可选）：一台主 Mac 上传端到端加密的快照，其他 Mac 和 iPhone 主屏幕上的网页 app 显示同样的数字，见 [docs/SYNC.md](docs/SYNC.md)
- **原生 Liquid Glass 应用图标**（Icon Composer 的 `.icon`，在 macOS 26+ 上自动适配浅色 / 深色 / 透明 / 着色）

## 构建

```bash
pnpm install
pnpm tauri build        # .app 和 .dmg 生成在 src-tauri/target/release/bundle/
```

开发时带热重载：

```bash
pnpm tauri dev
```

## 云端同步和手机网页 app

默认关闭。从标题栏的云朵按钮打开：选一台负责上传的 Mac，在其他设备上粘贴同一个配对码，再把网页 app 添加到手机主屏幕。服务器（一个你自己的 Supabase 项目）只保存 AES-256-GCM 密文，密钥不会离开你的设备。设置步骤、隐私模型和协议见 [docs/SYNC.md](docs/SYNC.md)。

```bash
npm run build:web                 # 手机网页 app -> dist-web/
node scripts/sync-selftest.mjs    # 协议测试向量 + Rust/TS 互通检查
```

另有一个原生 iPhone app，带主屏幕和锁屏小组件，在 [`ios-app/`](ios-app/README.md)（需要自己用 Xcode 构建，免费 Apple ID 就够）。

## 技术栈

- **外壳**：Tauri 2（Rust + WKWebView）
- **前端**：React 18 + TypeScript + Vite（内联样式，没有 CSS 框架）
- **玻璃**：`window-vibrancy` crate，`NSVisualEffectMaterial::HudWindow` + `NSVisualEffectState::Active`（窗口失去焦点时仍保持激活）
- **文件监听**：`notify` + `notify-debouncer-full`（对 `~/.claude/projects/` 和 `~/.codex/{sessions,archived_sessions}/` 做 500ms 防抖）
- **Claude 限额**：用 `security` 读钥匙串，用 `reqwest`/rustls 请求 Anthropic；遇到 401 时按 Claude Code 自己的方式刷新 OAuth token 并写回（见下文）
- **拖动区域**：Tauri 的 `data-tauri-drag-region` 属性。WKWebView 不认 Electron 的 `-webkit-app-region` CSS

## 数据源里的坑

如果你想做类似的东西，JSONL 事件里有几个坑值得知道：

- **按 `message.id` 去重，而不是按事件的 uuid**：会话恢复或分叉时，Claude Code 会为同一次 API 响应写多行 JSONL。每行的 UUID 都不同，但同一个 `message.id` 会出现在 2–9 处。一个用了很久的 `.claude` 目录里，大约一半的 assistant 事件是重复的。
- **`usage.speed == "fast"` 才是 fast mode 的标志**：原始的 `message.model` 里从来没有 "-fast"，那个后缀是 ccusage 合成的。fast mode 在 Opus 4.8 / 5 / 5.5 上是标准价的 2 倍（已退役的 Opus 4.6 fast mode 是 6 倍）。`usage.service_tier == "priority"` 是 Priority Tier 的容量，不是 fast mode。
- **Opus 4.6/4.7 不按旧的 Opus 4 计价**：每百万 token 是 $5/$25/$6.25/$0.5（输入/输出/缓存写入/缓存读取），不是 2025 年 10 月的 $15/$75。可以用 `claude usage --json`（`npx ccusage` 的别名）交叉核对。
- 归一化模型 ID 时去掉结尾的 `-YYYYMMDD` 日期后缀。
- `isSidechain: true` 标记的是 subagent 调用（一般占事件的约 12%）。目前的代码把它们计入总数，和 ccusage 一致。
- `message.model == "<synthetic>"` 的事件整条跳过。

Claude 的官方限额和 token 统计是两回事。它读钥匙串里的 `Claude Code-credentials` 条目，用和 Claude Usage Monitor 相同的请求头调用 Anthropic 的私有接口 `/api/oauth/usage`。access token 只发给 Anthropic。在 Claude 桌面 app 里运行的 Claude Code 从不刷新这个钥匙串条目，所以遇到 401 时 Token Counter 会自己刷新：发出和 Claude Code 一样的请求，持有 Claude Code 的凭证写入锁，并且只有当条目里仍是它用掉的那个 refresh token 时才把新 token 写回（其他字段原样保留）。如果终端或 headless 的 Claude Code 会话在最近 2 分钟内写过 transcript，它就不动手，因为那个进程会自己刷新。无法刷新时（已登出、被推迟），这一行把上一次成功的结果显示为过期数据。

Codex 的 rollout 文件在 `$CODEX_HOME`（默认 `~/.codex`）下的 `sessions/YYYY/MM/DD/` **以及**平铺的 `archived_sessions/`（归档操作会把线程移到这里），文件名是 `rollout-*.jsonl`，或者在 Codex 把闲置 7 天的文件压缩之后是 `rollout-*.jsonl.zst`。它们有自己的坑：

- **漏的是文件，不是价格**：只读 `sessions/**/*.jsonl` 会漏掉所有归档或压缩过的线程。两个目录、两种形式都要读；压缩或解压到一半、`X.jsonl` 和 `X.jsonl.zst` 同时存在时，读未压缩的那个。
- **`token_usage_record`（Codex ≥ 0.153）才是每次响应的准确数字**：每个 Responses API 响应一行，带用量，以 `response_id` 为键，包括那些任何 `token_count` 都不会报告的远程 compaction。`token_count` 是一个滚动快照（`info.{total,last}_token_usage`），只在没有 record 的区段（旧的 rollout）里才累加：先跳过连续重复的一对，再累加 `last_token_usage`，这样文件中途计数器重置也不受影响。
- **两种用量行都不带模型名和档位**：模型取自该 record 的 `turn_context`（按 `turn_id`），或最近的 `turn_context` / `thread_settings_applied`；请求的服务档位取自 `thread_settings_applied.thread_settings.service_tier`（`priority` 即 Fast，按 2 倍计费；GPT-6 Sol/Luna 默认就是它）。超过 272K token 的提示按输入 2 倍、输出 1.5 倍计费。
- **分叉会复制历史**：旧式分叉会把父线程的 `token_count` 行（以及挂在父线程 `thread_id` 下的 record）带着新的时间戳重写进子线程；分页的 subagent 用 `subagent_history_start_ordinal` 标出继承来的前缀。每个 response id / 被分叉复制的快照只算一次，算在最早出现的地方。
- **`input_tokens` 包含 `cached_input_tokens` 和 `cache_write_input_tokens`**（OpenAI 的惯例）：计价前要先拆开，否则缓存读取会按完整的输入价格计费。
- **`codex-auto-review` 是内部的审批审查器，不是 PR Code Review 那个产品**：没有公开文档说明它对应哪个计费模型，所以显示的美元数明确是按旧价格的估算。
- **`token_count.rate_limits` 已经不是官方限额了**：Codex 只保留一个“最新”槽位，每个限额桶（`codex_other`、按模型的限额、窗口为 null 的纯元数据更新）都会覆盖它，而且只有 Codex 发请求时它才变化。Codex 限额那一行读的是 Codex 自己的 `/status` 读的东西：`GET https://chatgpt.com/backend-api/wham/usage`，用 `$CODEX_HOME/auth.json`（或钥匙串条目 `Codex Auth`）里的 ChatGPT access token。它是只读的：一次性的 refresh token 从不触碰，token 过期时把上一次快照显示为过期数据。实时读取失败时，这一行退回到 rollout 里最新的主桶（`limit_id` 为 `codex`）窗口。
- 没有写进本地 rollout 的用量（Codex 云端任务、临时线程、memories）仍然计入套餐限额，但没法计价。

## 为什么用原生毛玻璃，而不是 CSS 玻璃

早期版本按设计稿用了叠加的 `backdrop-filter` + `mix-blend-mode`。在 macOS 上，窗口获得焦点时会稳定出现约 1 秒的色偏（WKWebView 在成为和失去关键窗口时会重新采样背景、重新合成混合模式）。怎么调 CSS 都修不好：透明度、`will-change`、`contain: paint`、提升到 GPU、降低滤镜复杂度都试过。把玻璃改成通过 `window-vibrancy` 使用 `NSVisualEffectView` 之后，闪烁完全消失，因为玻璃是在 AppKit 层合成的，不在 WKWebView 里。装饰层（色调、高光、光泽）仍然用 CSS 叠在上面。

## 许可证

MIT

---

基于 Anthropic 的 Claude Code Token Counter 原型设计稿构建。目标技术栈（Tauri + React）和液态玻璃的视觉风格遵循设计稿；偏离之处（原生毛玻璃、价格更新）见上文。
