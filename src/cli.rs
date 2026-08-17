use clap::{Parser, Subcommand, ValueEnum};

use crate::skill::SkillAgent;

#[derive(Debug, Parser)]
#[command(
    name = "mixly",
    version,
    about = "轻量 CLI/TUI 音乐播放器（网易云 + QQ + B站 + 本地文件）"
)]
pub struct Cli {
    /// 代理地址（如 socks5://host:port 或 http://...）。优先于配置文件与环境变量。
    #[arg(long, global = true)]
    pub proxy: Option<String>,

    /// 在线平台搜索时优先的平台（覆盖配置文件）。qq | netease | bilibili
    #[arg(long, global = true, value_enum)]
    pub prefer: Option<PreferArg>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum PlatformArg {
    Netease,
    Qq,
    /// Bilibili 视频音频（需扫码登录）
    Bilibili,
    /// 本地音乐库
    Local,
    All,
}

/// 在线平台优先侧（与配置 general.preferred_platform 对应）
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PreferArg {
    Qq,
    Netease,
    Bilibili,
}

impl PreferArg {
    pub fn to_platform(self) -> crate::models::Platform {
        match self {
            Self::Qq => crate::models::Platform::Qq,
            Self::Netease => crate::models::Platform::Netease,
            Self::Bilibili => crate::models::Platform::Bilibili,
        }
    }
}

impl PlatformArg {
    /// 在线平台列表；`all` 时按 `prefer` 排序。不含 local（本地库单独查）。
    pub fn as_online_platforms(
        &self,
        prefer: crate::models::Platform,
    ) -> Vec<crate::models::Platform> {
        match self {
            Self::Netease => vec![crate::models::Platform::Netease],
            Self::Qq => vec![crate::models::Platform::Qq],
            Self::Bilibili => vec![crate::models::Platform::Bilibili],
            Self::Local => vec![],
            Self::All => {
                // 首选平台在前，其余按固定顺序补齐
                let fixed = [
                    crate::models::Platform::Qq,
                    crate::models::Platform::Netease,
                    crate::models::Platform::Bilibili,
                ];
                let mut order = Vec::with_capacity(3);
                order.push(prefer);
                for p in fixed {
                    if p != prefer && !order.contains(&p) {
                        order.push(p);
                    }
                }
                order
            }
        }
    }

    pub fn includes_local(&self) -> bool {
        matches!(self, Self::Local | Self::All)
    }
}

#[cfg(test)]
mod prefer_order_tests {
    use super::*;
    use crate::models::Platform;

    #[test]
    fn all_prefers_qq_first_by_default_order() {
        let v = PlatformArg::All.as_online_platforms(Platform::Qq);
        assert_eq!(v, vec![Platform::Qq, Platform::Netease, Platform::Bilibili]);
    }

    #[test]
    fn all_can_prefer_netease_first() {
        let v = PlatformArg::All.as_online_platforms(Platform::Netease);
        assert_eq!(v, vec![Platform::Netease, Platform::Qq, Platform::Bilibili]);
    }

    #[test]
    fn all_can_prefer_bilibili_first() {
        let v = PlatformArg::All.as_online_platforms(Platform::Bilibili);
        assert_eq!(v, vec![Platform::Bilibili, Platform::Qq, Platform::Netease]);
    }

    #[test]
    fn explicit_bilibili_only_that_platform() {
        let v = PlatformArg::Bilibili.as_online_platforms(Platform::Qq);
        assert_eq!(v, vec![Platform::Bilibili]);
    }
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// 搜索歌曲（网易云 / QQ / B站 / 本地库，默认 all）
    Search {
        /// 搜索关键词
        keyword: String,
        /// 平台：netease | qq | bilibili | local | all
        #[arg(long, value_enum, default_value_t = PlatformArg::All)]
        platform: PlatformArg,
        /// 每个平台返回条数
        #[arg(long, default_value_t = 10)]
        limit: u64,
    },
    /// 播放：平台+ID、歌名（搜索首条）、或本地歌单名
    ///
    /// 示例：
    ///   mixly play netease 347230
    ///   mixly play bilibili BV1xx411c7mD
    ///   mixly play bilibili https://www.bilibili.com/video/BV1xx/?p=2
    ///   mixly play "晴天"
    ///   mixly play "晴天" --platform qq --loop
    ///   mixly play "我的歌单"
    ///   mixly play "我的歌单" --random --loop
    ///   mixly play "我的歌单" --playlist
    Play {
        /// 一个参数：歌名或歌单名；两个参数：平台 + 歌曲 ID（如 netease 347230 / bilibili BV1xx:p2）
        #[arg(required = true, num_args = 1..=2)]
        targets: Vec<String>,
        /// 循环：单曲=单曲循环；歌单=列表循环（可与 --random 合用）
        #[arg(long = "loop")]
        r#loop: bool,
        /// 随机播放（主要对歌单：打乱顺序后播放；可与 --loop 合用）
        #[arg(long = "random")]
        random: bool,
        /// 按歌名搜索时的平台（默认 all）；平台+ID 模式可省略
        #[arg(long, value_enum, default_value_t = PlatformArg::All)]
        platform: PlatformArg,
        /// 强制当作本地歌单名（不搜索歌曲）
        #[arg(long)]
        playlist: bool,
    },
    /// 扫码登录指定平台
    Login {
        /// 登录平台：netease | qq | bilibili
        #[arg(long, value_enum)]
        platform: LoginPlatform,
    },
    /// 退出登录并删除本机凭证（仅影响指定平台）
    Logout {
        /// 退出平台：netease | qq | bilibili
        #[arg(long, value_enum)]
        platform: LoginPlatform,
    },
    /// 本地混合歌单管理
    Playlist {
        #[command(subcommand)]
        action: PlaylistCmd,
    },
    /// 本地音乐库（导入文件/目录，可加入混合歌单）
    Local {
        #[command(subcommand)]
        action: LocalCmd,
    },
    /// 启动交互式终端界面（TUI）
    Tui,
    /// 查看 / 修改本地配置（如双平台优先侧）
    Config {
        #[command(subcommand)]
        action: ConfigCmd,
    },
    /// 安装或移除 AI Agent Skill
    Skill {
        #[command(subcommand)]
        action: SkillCmd,
    },
}

#[derive(Debug, Subcommand)]
pub enum SkillCmd {
    /// 安装内嵌的 mixly Skill
    #[command(group(
        clap::ArgGroup::new("scope")
            .required(true)
            .multiple(false)
            .args(["global", "project"])
    ))]
    Install {
        #[arg(value_enum)]
        agent: SkillAgent,
        #[arg(long)]
        global: bool,
        #[arg(long)]
        project: bool,
        /// 覆盖内容不同的现有 Skill
        #[arg(long)]
        force: bool,
    },
    /// 卸载 mixly Skill
    #[command(group(
        clap::ArgGroup::new("scope")
            .required(true)
            .multiple(false)
            .args(["global", "project"])
    ))]
    Uninstall {
        #[arg(value_enum)]
        agent: SkillAgent,
        #[arg(long)]
        global: bool,
        #[arg(long)]
        project: bool,
        /// 删除已被修改的 Skill
        #[arg(long)]
        force: bool,
    },
    /// 显示 Skill 的安装路径
    #[command(group(
        clap::ArgGroup::new("scope")
            .required(true)
            .multiple(false)
            .args(["global", "project"])
    ))]
    Path {
        #[arg(value_enum)]
        agent: SkillAgent,
        #[arg(long)]
        global: bool,
        #[arg(long)]
        project: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum LocalCmd {
    /// 导入音频文件或目录到本地库（可顺带加入歌单）
    Import {
        /// 文件或文件夹路径
        path: String,
        /// 导入后加入该本地歌单
        #[arg(long)]
        playlist: Option<String>,
    },
    /// 列出本地库曲目
    List,
    /// 在本地库中按关键词搜索
    Search {
        keyword: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// 从本地库移除（不删除磁盘文件）
    Remove {
        /// 路径或 id
        id_or_path: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCmd {
    /// 打印当前配置要点
    Show,
    /// 设置搜索优先平台：qq | netease | bilibili
    Prefer {
        /// qq、netease 或 bilibili
        platform: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum LoginPlatform {
    Netease,
    Qq,
    Bilibili,
}

#[derive(Debug, Subcommand)]
pub enum PlaylistCmd {
    /// 新建本地歌单
    Create {
        /// 歌单名称
        name: String,
    },
    /// 列出本地歌单
    List,
    /// 查看歌单内歌曲
    Show {
        /// 歌单 ID 或名称
        id_or_name: String,
    },
    /// 删除歌单
    Delete {
        /// 歌单 ID 或名称
        id_or_name: String,
    },
    /// 重命名歌单
    Rename {
        /// 歌单 ID 或名称
        id_or_name: String,
        /// 新名称
        new_name: String,
    },
    /// 向歌单添加歌曲
    Add {
        /// 歌单 ID 或名称
        id_or_name: String,
        /// 平台：netease | qq | bilibili | local（local 时 song_id 为文件路径）
        platform: String,
        /// 歌曲 ID（B站 为 BV 号/分P；local 平台填绝对/相对路径）
        song_id: String,
        /// 歌曲名（可选）
        #[arg(long)]
        name: Option<String>,
        /// 歌手（可选）
        #[arg(long)]
        artist: Option<String>,
    },
    /// 按序号删除歌单中的歌曲（从 0 开始）
    Remove {
        /// 歌单 ID 或名称
        id_or_name: String,
        /// 歌曲下标（从 0 开始）
        index: usize,
    },
    /// 调整歌单内歌曲顺序（把 from 下标移到 to 下标）
    Move {
        /// 歌单 ID 或名称
        id_or_name: String,
        /// 原下标（从 0 开始）
        from: usize,
        /// 目标下标（从 0 开始）
        to: usize,
    },
    /// 按 mixly 内部队列播放整张歌单
    Play {
        /// 歌单 ID 或名称
        id_or_name: String,
        /// 列表循环
        #[arg(long = "loop")]
        r#loop: bool,
        /// 随机播放（打乱顺序）
        #[arg(long = "random")]
        random: bool,
    },
}
