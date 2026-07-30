---
name: mixly
description: >
  Operate the local mixly CLI music player via shell commands: search Netease/QQ
  tracks, import local audio files, create/list/show/rename/delete playlists,
  add/remove/reorder songs, play a track or playlist. Use when the user asks to
  play music, manage mixly playlists, import local songs, search songs, "用 mixly",
  "放首歌", "建歌单", "导入本地", or runs /mixly. Prefer non-interactive CLI over
  TUI. Do not use for implementing mixly itself unless the user is coding the player.
argument-hint: "[search|play|playlist|status]"
---

# mixly — Agent 操作手册

通过**执行 shell 命令**控制本机已安装的 `mixly`（网易云 + QQ 混合歌单 CLI）。  
你是操作员，不是改播放器源码（除非用户明确在写 mixly 代码）。

## 硬性规则

1. **优先用 CLI，不要开 `mixly tui`**（TUI 交互阻塞，不适合 agent）。
2. **先确认二进制**：`mixly --version`（失败则试 `"$env:USERPROFILE\.cargo\bin\mixly.exe" --version` 或 `F:\own\mixly\target\release\mixly.exe`）。
3. **播放是长驻进程**：`mixly play` / `mixly playlist play` 会阻塞到播完或 Ctrl+C。  
   - 在 Grok/可后台环境：用 **background=true** 启动，需要停时再 kill。  
   - 不要在无超时保护的前台里干等整首歌。
4. **登录需要人扫码**：`mixly login --platform netease|qq` 会打印终端二维码并轮询。  
   - 需要登录时：启动命令、告诉用户去扫终端里的码；**不要**假装已登录。  
   - 未登录时 QQ 常拿不到播放链；网易云部分歌也可匿名。
5. **解析搜索结果里的 `id=`**：展示行是 `[平台] 歌名-专辑-歌手`，**播放用的是后面的 id**。
6. 平台 token 字面量固定为：`netease` | `qq`（不要用中文当参数）。
7. 用户可见输出多为中文；命令名与参数仍是英文。

## 环境与代理

```powershell
mixly --version
# 需要代理时（全局参数）：
mixly --proxy socks5://127.0.0.1:7890 <子命令...>
```

配置与 token 目录（Windows）：`%APPDATA%\mixly\`  
（`netease_token.json` / `qq_token.json` / `playlists\` / `config.toml`）

依赖：系统 PATH 上要有 **mpv**。

### 双平台优先（默认 QQ）

`search` / `play "歌名"` 在 `--platform all` 时：**优先平台排前**（默认 QQ）。  
选曲：先完全匹配歌名（列表中靠前即优先平台），否则取列表第 0 条（优先平台第一条）。

```powershell
mixly config prefer qq          # 持久：优先 QQ
mixly config prefer netease     # 持久：优先网易云
mixly config show
mixly --prefer netease search "晴天"   # 仅本次覆盖
mixly --prefer qq play "晴天"
```

---

## 命令速查

### 本地音乐库

```powershell
mixly local import "D:\Music\song.mp3"
mixly local import "D:\Music\Album" --playlist "混合"
mixly local list
mixly local search "关键词"
mixly local remove "D:\Music\song.mp3"
mixly play local "D:\Music\song.mp3"
mixly playlist add "混合" local "D:\Music\song.mp3"
```

### 搜索

```powershell
mixly search "关键词"                    # 含本地库 + 在线
mixly search "关键词" --platform local
mixly search "关键词" --platform netease
mixly search "关键词" --platform qq --limit 15
```

输出示例：

```text
[QQ] 晴天-叶惠美-周杰伦  id=0039MnYb0qxYhV
[网易云] 海阔天空-乐与怒-Beyond  id=347230
```

**播放/加歌单时用 `id=` 后的值**，平台用 `qq` 或 `netease`。

### 播放

```powershell
# 精确：平台 + ID
mixly play qq 0039MnYb0qxYhV
mixly play netease 347230 --loop

# 歌名（搜索后自动选曲；优先完全匹配歌名）
mixly play "晴天"
mixly play "海阔天空" --platform netease
mixly play "七里香" --platform qq --loop

# 本地歌单名（若存在同名歌单则优先播歌单）
mixly play "通勤"
mixly play "通勤" --playlist          # 强制歌单
mixly play "通勤" --loop              # 歌单列表循环
mixly play "通勤" --random            # 随机播一轮
mixly play "通勤" --random --loop     # 随机 + 列表循环
mixly playlist play "通勤" --random   # 等价
```

Agent 流程：
- 用户给了明确 ID → `play <platform> <id>`  
- 用户只给歌名 → `play "歌名"`（可加 `--platform`）  
- 用户说播某歌单 → `play "歌单名"` 或 `playlist play "名"`  
- 播放命令建议**后台**运行；`--loop` 时不会在一曲后退出，需 kill 才停。
- 队列播放时：当前曲进度 ≥70% 或剩余 ≤约 18s 会预取下一首到临时文件；无需 agent 干预。

### 登录（需用户扫码）

```powershell
mixly login --platform netease
mixly login --platform qq
```

### 歌单 CRUD

```powershell
mixly playlist create "通勤"
mixly playlist list
mixly playlist show "通勤"
mixly playlist rename "通勤" "夜车"
mixly playlist delete "夜车"
```

### 整理歌单（加 / 删 / 调序）

```powershell
# 添加（name/artist 建议带上，方便展示）
mixly playlist add "通勤" qq 0039MnYb0qxYhV --name "晴天" --artist "周杰伦"
mixly playlist add "通勤" netease 347230 --name "海阔天空" --artist "Beyond"

# 按下标删除（show 里左侧数字，从 0 起）
mixly playlist remove "通勤" 0

# 调序：把 from 移到 to
mixly playlist move "通勤" 2 0

# 按歌单连播（长驻，建议后台）
mixly playlist play "通勤"
```

---

## 推荐工作流（给 agent）

### A. 「放一首 XX」

1. `mixly search "XX" --limit 10`
2. 根据歌名/歌手选一条，记下 `platform` + `id`
3. **后台**执行：`mixly play <platform> <id>`
4. 回复用户：正在播哪一首；若失败则提示登录或换平台/换曲

### B. 「建一个叫 X 的歌单并加入几首歌」

1. `mixly playlist create "X"`（若已存在则 `list` 后直接用）
2. 对每首歌：`search` → `playlist add "X" <platform> <id> --name ... --artist ...`
3. `mixly playlist show "X"` 确认
4. 需要顺序：`move` 调整
5. 用户要听：`playlist play "X"`（后台）

### C. 「整理歌单」

1. `playlist show "名"` 看下标
2. `remove` / `move` / `add` 按需
3. 再 `show` 复述最终列表

### D. 「现在有什么歌单」

```powershell
mixly playlist list
```

---

## 错误处理

| 现象 | 做法 |
|------|------|
| 找不到 `mixly` | 用完整路径 `~\.cargo\bin\mixly.exe`，或 `cargo install --path <mixly仓库>` |
| 搜索无结果 / 网络错 | 加重试；试 `--proxy`；缩小关键词 |
| 播放链接空 / 播放错误 | 请用户 `login`；换平台或另一首歌 |
| mpv 相关失败 | 提示安装 mpv 并保证在 PATH |
| 歌单不存在 | `list` 核对名称/id；名称可模糊匹配已有逻辑 |

退出码非 0 时：把 stderr/stdout 关键信息原样或摘要给用户，再决定是否换策略。

---

## 不要做的事

- 不要为「放首歌」去改 mixly 源码或重装（除非用户要求升级）。
- 不要用 TUI 完成自动化。
- 不要把 token 文件内容贴进对话。
- 不要假设英文帮助文案；用户输出以中文为准。
- 不要并行开多个 `play`（会抢 mpv/命名管道）。

---

## 自检（可选）

```powershell
mixly --help
mixly playlist list
mixly search "test" --limit 3
```

能跑通 list/search 即可操作歌单；play 再依赖登录与网络。
