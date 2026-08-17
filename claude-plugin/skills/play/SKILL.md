---
name: play
description: >
  Play music with the local mixly CLI player in the background. While playing,
  Claude Code's bottom status line shows the current song, progress and lyric
  (refreshed every 1 second, no model tokens). Use when the user asks to play
  music or you recommended a track via the mixly skill.
allowed-tools:
  - Bash(mixly *)
  - PowerShell(mixly *)
---

# mixly 播放（Claude Code 状态栏）

本插件把 `mixly play` 的进度展示到 Claude Code 底部状态栏。技能本身不做播放决策——选曲请复用 `mixly` skill 的推荐与搜索流程。

## 状态栏

- `mixly play` 每 400ms 写入 `%APPDATA%\mixly\now-playing.json`；
- Claude Code 每秒执行一次 `mixly status --claude` 读取并渲染（宽终端两行、窄终端三行，支持 `NO_COLOR`）；
- 状态栏刷新完全本地执行，不消耗模型 token；播放器退出后 3 秒内状态自动消失。

## 播放规则（与 mixly skill 一致）

1. 只用 CLI，不开 `mixly tui`（交互阻塞，不适合 agent）。
2. `play` / `playlist play` 是长驻进程：**后台运行**，要停就 kill；不要前台干等整首歌。
3. 同一时间只保留一个播放进程（多实例会争抢 mpv 管道）。
4. 登录要人扫码：`mixly login --platform netease|qq|bilibili`，不要假装已登录。
5. 不要把 token 文件内容贴进对话。

## 命令速查

```powershell
mixly play qq 0039MnYb0qxYhV                  # 平台 + id 精确播
mixly play "晴天" --platform qq               # 歌名搜索播
mixly play bilibili BV1xx411c7mD             # B站 视频音频
mixly play "通勤" --playlist --loop           # 歌单播放
mixly playlist play "通勤" --random           # 随机播放歌单
mixly status --json                          # 机器可读状态（调试用）
mixly logout --platform bilibili             # 退出登录并删除本机凭证
```

## 出错时

- 状态栏一直空白 → 确认 `mixly play` 在后台运行，且 `mixly status --claude` 手动有输出。
- 播放链接空 / 播放错误 → 让用户 `login`，或换平台/换曲。
- 退出码非 0 → 把 stderr 关键信息摘要给用户再决定换策略。
