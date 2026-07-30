---
name: mixly
description: >
  Operate the local mixly CLI music player via shell commands: search Netease/QQ
  tracks, import local audio, manage playlists, play songs or playlists. Use when
  the user asks to play music, manage mixly playlists, import local songs, search
  songs, "用 mixly", "放首歌", "建歌单", "导入本地", or runs /mixly. Do not use for
  implementing mixly itself unless the user is coding the player.
argument-hint: "[search|play|playlist|status]"
---

# mixly 操作手册

用 shell 命令控制本机 `mixly`（网易云 + QQ + 本地文件混合歌单播放器，依赖 PATH 上的 mpv）。

## 规则

1. 只用 CLI，不开 `mixly tui`（交互阻塞，不适合 agent）。
2. 找不到 `mixly` 时试 `~\.cargo\bin\mixly.exe`；仍没有则请用户安装。
3. `play` / `playlist play` 是长驻进程：**后台运行**，要停就 kill；不要前台干等整首歌。
4. 登录要人扫码：启动 `mixly login --platform netease|qq` 后让用户扫终端二维码，不要假装已登录。未登录时 QQ 常拿不到播放链。
5. 搜索输出行 `[平台] 歌名-专辑-歌手  id=X`：播放/加歌单用 `id=` 的值；平台参数固定 `netease` | `qq` | `local`，不要用中文。
6. 不要并行开多个 play（抢 mpv 管道）；不要把 token 文件内容贴进对话。

## 命令速查

```powershell
mixly search "关键词" [--platform all|netease|qq|local] [--limit 10]
mixly play qq 0039MnYb0qxYhV                        # 平台 + id 精确播
mixly play "晴天" [--platform qq] [--loop]           # 歌名搜索播（优先完全匹配；同名歌单优先播歌单）
mixly play "通勤" [--playlist] [--random] [--loop]   # 歌单：--playlist 强制按歌单，--random 打乱，--loop 循环
mixly playlist create|list|show|rename|delete ...
mixly playlist add "通勤" qq <id> --name "晴天" --artist "周杰伦"
mixly playlist remove "通勤" 0                       # 下标见 show，从 0 起
mixly playlist move "通勤" 2 0                       # from → to
mixly playlist play "通勤" [--random] [--loop]
mixly local import <文件或目录> [--playlist "混合"]
mixly local list | search "关键词" | remove <路径>    # remove 不删磁盘文件
mixly login --platform netease|qq
mixly config prefer qq|netease                       # 双平台优先侧持久化（默认 qq）
mixly config show
mixly --prefer netease <子命令>                       # 仅本次覆盖优先侧
mixly --proxy socks5://127.0.0.1:7890 <子命令>        # 需代理时
```

配置与 token 在 `%APPDATA%\mixly\`。

## 常见流程

- **放一首 X**：`search "X"` → 选一条记下 platform + id → 后台 `play <platform> <id>` → 告诉用户在播哪首。
- **建歌单加歌**：`playlist create` → 逐首 `search` + `add`（带 `--name`/`--artist` 便于展示）→ `show` 确认。
- **整理歌单**：`show` 看下标 → `remove` / `move` / `add` → 再 `show` 复述结果。

## 出错时

- 播放链接空 / 播放错误 → 让用户 `login`，或换平台/换曲。
- 搜索无结果 / 网络错 → 试 `--proxy`、缩小关键词。
- mpv 相关失败 → 提示安装 mpv 并加入 PATH。
- 退出码非 0 → 把 stderr 关键信息摘要给用户再决定换策略。
