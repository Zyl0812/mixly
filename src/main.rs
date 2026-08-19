use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use mixly::cli::{
    Cli, Commands, ConfigCmd, LocalCmd, LoginPlatform, PlatformArg, PlaylistCmd, PreferArg,
    SkillCmd, StatusLineCmd,
};
use mixly::config::{
    default_mpv_socket_path, inject_proxy_env, load_config, mask_proxy_url,
    preferred_platform_from_config, quality_from_config, resolve_proxy, save_config, AppPaths,
    ProxyDecision,
};
use mixly::local::{song_for_play_path, LocalLibrary};
use mixly::lyrics::lyrics_state_from_lrc;
use mixly::player::MpvPlayer;
use mixly::playlist::{PlayQueue, PlaylistStore};
use mixly::prefetch::{
    log_prefetch_fail, log_prefetch_start, prefetchable, should_prefetch, PrefetchJob,
    PrefetchedTrack,
};
use mixly::relay::AudioRelay;
use mixly::skill::{install_skill, skill_file_path, uninstall_skill, SkillAgent, SkillScope};
use mixly::status::{read_now_playing, render_claude_status};
use mixly::tui::run_tui;
use mixly::{ApiClient, LyricsState, Platform, PlayMode, Song};
use tokio::sync::Mutex;
use tracing::warn;
use tracing_subscriber::fmt::writer::BoxMakeWriter;

fn main() {
    let cli = Cli::parse();

    // `status` 是最轻量的命令：在网络/播放器/tracing 初始化之前处理，
    // 保证 Claude Code 每秒调用足够快。
    if let Commands::Status { claude, .. } = &cli.command {
        let out = cmd_status(*claude);
        print!("{out}");
        return;
    }

    if let Commands::Skill { action } = &cli.command {
        if let Err(e) = cmd_skill(action) {
            eprintln!("错误: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    if let Commands::Statusline { action } = &cli.command {
        if let Err(e) = cmd_statusline(action) {
            eprintln!("错误: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    let paths = match AppPaths::discover() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("配置路径错误: {e}");
            std::process::exit(1);
        }
    };
    let _ = paths.ensure();
    let cfg = load_config(&paths.config_file).unwrap_or_default();

    let decision = resolve_proxy(cli.proxy.as_deref(), &cfg);
    // Must run before Tokio threads exist.
    inject_proxy_env(&decision);

    // TUI 用备用屏，任何写 stderr 的日志（哪怕只是 warn）都会糊在界面上：改写日志文件。
    // ponytail: 每次启动截断，不做轮转；真嫌大再加。
    let log_file = matches!(cli.command, Commands::Tui)
        .then(|| std::fs::File::create(paths.config_dir.join("mixly.log")).ok())
        .flatten();
    let to_file = log_file.is_some();
    let writer = match log_file {
        Some(f) => BoxMakeWriter::new(std::sync::Mutex::new(f)),
        None => BoxMakeWriter::new(std::io::stderr),
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .with_ansi(!to_file)
        .with_writer(writer)
        .init();

    match &decision {
        ProxyDecision::Proxy(url) => {
            tracing::info!(
                proxy = %mixly::mask_proxy_url(url),
                "代理已启用（命令行 > 配置 > 环境变量）"
            );
        }
        ProxyDecision::Direct => {
            tracing::info!("代理：直连");
        }
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    if let Err(e) = rt.block_on(async_main(cli, paths, cfg, decision)) {
        eprintln!("错误: {e:#}");
        std::process::exit(1);
    }
}

async fn async_main(
    cli: Cli,
    paths: AppPaths,
    cfg: mixly::Config,
    proxy: ProxyDecision,
) -> Result<()> {
    let quality = quality_from_config(&cfg);
    let prefer = resolve_prefer(cli.prefer, &cfg);
    let api = ApiClient::new(paths.clone(), quality);
    let store = PlaylistStore::new(&paths.playlists_dir)?;

    match cli.command {
        Commands::Search {
            keyword,
            platform,
            limit,
        } => cmd_search(&api, &keyword, platform, limit, prefer).await,
        Commands::Play {
            targets,
            r#loop,
            random,
            platform,
            playlist,
        } => {
            cmd_play_resolve(
                api, &store, &paths, &cfg, targets, r#loop, random, platform, playlist, prefer,
            )
            .await
        }
        Commands::Login { platform } => {
            let p = login_platform_to_platform(platform);
            api.login_qr(p).await
        }
        Commands::Logout { platform } => {
            let p = login_platform_to_platform(platform);
            api.logout(p)?;
            println!("已退出登录（{}），本机凭证已删除。", p.label_zh());
            Ok(())
        }
        Commands::Playlist { action } => {
            cmd_playlist(Arc::new(api), &store, &paths, &cfg, action).await
        }
        Commands::Local { action } => cmd_local(&paths, &store, action),
        Commands::Tui => cmd_tui(api, store, &cfg, proxy, prefer).await,
        Commands::Config { action } => cmd_config(&paths, cfg, action),
        Commands::Status { .. } => unreachable!("status handled before runtime setup"),
        Commands::Skill { .. } => unreachable!("skill command handled before runtime setup"),
        Commands::Statusline { .. } => {
            unreachable!("statusline command handled before runtime setup")
        }
    }
}

fn login_platform_to_platform(p: LoginPlatform) -> Platform {
    match p {
        LoginPlatform::Netease => Platform::Netease,
        LoginPlatform::Qq => Platform::Qq,
        LoginPlatform::Bilibili => Platform::Bilibili,
    }
}

/// `status` 命令：不初始化网络/播放器/tracing，保证足够轻量。
fn cmd_status(claude: bool) -> String {
    let Ok(paths) = AppPaths::discover() else {
        return String::new();
    };
    let Some(np) = read_now_playing(&paths.now_playing) else {
        return String::new(); // 无播放或过期：退出码 0，不输出音乐行
    };
    if claude {
        let columns = std::env::var("COLUMNS")
            .ok()
            .and_then(|c| c.parse::<usize>().ok())
            .unwrap_or(80);
        render_claude_status(&np, columns)
    } else {
        match serde_json::to_string(&np) {
            Ok(json) => format!("{json}\n"),
            Err(_) => String::new(),
        }
    }
}

fn cmd_skill(action: &SkillCmd) -> Result<()> {
    match action {
        SkillCmd::Install {
            agent,
            global,
            project,
            force,
        } => {
            let scope = skill_scope(*global, *project);
            if *agent == SkillAgent::Claude && *global {
                mixly::skill::preflight_claude_plugin_install(*force)?;
            }
            let (path, changed) = install_skill(*agent, scope, *force)?;
            if changed {
                println!("Installed mixly skill to:");
            } else {
                println!("Mixly skill is already installed at:");
            }
            println!("{}", path.display());
            // Claude 全局安装顺带装插件 + status line
            if *agent == SkillAgent::Claude && *global {
                let (plugin_path, plugin_changed) = mixly::skill::install_claude_plugin(*force)?;
                if plugin_changed {
                    println!("Installed Claude plugin + status line:");
                    println!("  {}", plugin_path.display());
                    println!("  ~/.claude/settings.json 已配置 mixly status --claude");
                } else {
                    println!("Claude plugin + status line already configured.");
                }
            }
            println!("Restart the agent if the skill is not detected automatically.");
        }
        SkillCmd::Uninstall {
            agent,
            global,
            project,
            force,
        } => {
            let scope = skill_scope(*global, *project);
            if *agent == SkillAgent::Claude && *global {
                mixly::skill::preflight_claude_plugin_uninstall(*force)?;
            }
            let (path, changed) = uninstall_skill(*agent, scope, *force)?;
            if changed {
                println!("Removed mixly skill from:");
            } else {
                println!("Mixly skill is not installed at:");
            }
            println!("{}", path.display());
            if *agent == SkillAgent::Claude && *global {
                let (plugin_path, plugin_changed) = mixly::skill::uninstall_claude_plugin(*force)?;
                if plugin_changed {
                    println!("Removed Claude plugin + status line:");
                    println!("  {}", plugin_path.display());
                } else {
                    println!("Claude plugin not installed (nothing to remove).");
                }
            }
        }
        SkillCmd::Path {
            agent,
            global,
            project,
        } => println!(
            "{}",
            skill_file_path(*agent, skill_scope(*global, *project))?.display()
        ),
    }
    Ok(())
}

fn cmd_statusline(action: &StatusLineCmd) -> Result<()> {
    match action {
        StatusLineCmd::Install { force } => {
            let (path, changed) = mixly::skill::install_claude_status_line(*force)?;
            if changed {
                println!("Installed Mixly status line in:");
            } else {
                println!("Mixly status line is already configured in:");
            }
            println!("{}", path.display());
        }
        StatusLineCmd::Uninstall { force } => {
            let (path, changed) = mixly::skill::uninstall_claude_status_line(*force)?;
            if changed {
                println!("Removed Mixly status line from:");
            } else {
                println!("Mixly status line was not configured in:");
            }
            println!("{}", path.display());
        }
    }
    Ok(())
}

fn skill_scope(global: bool, project: bool) -> SkillScope {
    debug_assert!(global ^ project);
    if global {
        SkillScope::Global
    } else {
        SkillScope::Project
    }
}

fn cmd_local(paths: &AppPaths, store: &PlaylistStore, action: LocalCmd) -> Result<()> {
    let mut lib = LocalLibrary::open(paths)?;
    match action {
        LocalCmd::Import { path, playlist } => {
            let added = lib.import_path(std::path::Path::new(&path))?;
            if added.is_empty() {
                println!("没有新导入的音频（可能已在库中，或不是支持的格式）");
            } else {
                println!("已导入 {} 首：", added.len());
                for s in &added {
                    println!("  {}  path={}", s.display_line(), s.id);
                }
            }
            if let Some(pl_name) = playlist {
                for s in &added {
                    store.add_song(&pl_name, s.clone())?;
                }
                if !added.is_empty() {
                    println!("并已加入歌单「{pl_name}」");
                }
            }
        }
        LocalCmd::List => {
            let songs = lib.list();
            if songs.is_empty() {
                println!("（本地库为空，使用 mixly local import <路径> 导入）");
            }
            for (i, s) in songs.iter().enumerate() {
                println!("{i}: {}  path={}", s.display_line(), s.id);
            }
        }
        LocalCmd::Search { keyword, limit } => {
            let hits = lib.search(&keyword, limit);
            if hits.is_empty() {
                println!("本地库无匹配「{keyword}」");
            }
            for s in hits {
                println!("{}  path={}", s.display_line(), s.id);
            }
        }
        LocalCmd::Remove { id_or_path } => {
            let s = lib.remove(&id_or_path)?;
            println!("已从本地库移除: {}（磁盘文件未删除）", s.display_line());
        }
    }
    Ok(())
}

/// CLI `--prefer` > config `general.preferred_platform` > qq
fn resolve_prefer(cli: Option<PreferArg>, cfg: &mixly::Config) -> Platform {
    cli.map(PreferArg::to_platform)
        .unwrap_or_else(|| preferred_platform_from_config(cfg))
}

fn cmd_config(paths: &AppPaths, mut cfg: mixly::Config, action: ConfigCmd) -> Result<()> {
    match action {
        ConfigCmd::Show => {
            println!("配置文件: {}", paths.config_file.display());
            println!("音质: {}", cfg.general.quality);
            println!(
                "搜索优先: {} （qq | netease | bilibili）",
                preferred_platform_from_config(&cfg).label_zh()
            );
            println!(
                "代理: enabled={} url={}",
                cfg.proxy.enabled,
                mask_proxy_url(&cfg.proxy.url)
            );
            println!("mpv: {}", cfg.player.mpv_path);
        }
        ConfigCmd::Prefer { platform } => {
            let p = parse_platform(&platform)?;
            cfg.general.preferred_platform = match p {
                Platform::Qq => "qq".into(),
                Platform::Netease => "netease".into(),
                Platform::Bilibili => "bilibili".into(),
                Platform::Local => bail!("优先平台只能是 qq、netease 或 bilibili"),
            };
            save_config(&paths.config_file, &cfg)?;
            println!(
                "已设置搜索优先为「{}」，写入 {}",
                p.label_zh(),
                paths.config_file.display()
            );
            println!("临时覆盖可用: mixly --prefer qq|netease|bilibili search|play ...");
        }
    }
    Ok(())
}

fn parse_platform(s: &str) -> Result<Platform> {
    Platform::parse(s)
        .ok_or_else(|| anyhow!("未知平台「{s}」，请使用 netease | qq | bilibili | local"))
}

async fn cmd_search(
    api: &ApiClient,
    keyword: &str,
    platform: PlatformArg,
    limit: u64,
    prefer: Platform,
) -> Result<()> {
    if matches!(platform, PlatformArg::All) {
        println!("搜索范围: 本地 + 在线（在线优先 {}）", prefer.label_zh());
    }
    let mut any = false;

    if platform.includes_local() {
        match api.search(Platform::Local, keyword, limit).await {
            Ok(songs) => {
                if songs.is_empty() && matches!(platform, PlatformArg::Local) {
                    println!("[local] （无结果）");
                }
                for s in songs {
                    println!("{}  id={}", s.display_line(), s.id);
                    any = true;
                }
            }
            Err(e) => eprintln!("[local] 搜索失败: {e:#}"),
        }
    }

    for p in platform.as_online_platforms(prefer) {
        if p == Platform::Bilibili && !api.is_logged_in(Platform::Bilibili) {
            if matches!(platform, PlatformArg::All) {
                // all 模式：未登录时跳过 Bilibili，其余平台不受影响
                continue;
            }
            eprintln!("[bilibili] Bilibili 未登录，请执行 mixly login --platform bilibili");
            continue;
        }
        match api.search(p, keyword, limit).await {
            Ok(songs) => {
                if songs.is_empty() {
                    println!("[{p}] （无结果）");
                }
                for s in songs {
                    println!("{}  id={}", s.display_line(), s.id);
                    any = true;
                }
            }
            Err(e) => eprintln!("[{p}] 搜索失败: {e:#}"),
        }
    }
    if !any {
        bail!("没有搜索结果");
    }
    Ok(())
}

async fn build_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .context("build reqwest client")
}

async fn spawn_player(cfg: &mixly::Config) -> Result<MpvPlayer> {
    let socket = cfg
        .player
        .socket
        .clone()
        .unwrap_or_else(default_mpv_socket_path);
    MpvPlayer::spawn(&cfg.player.mpv_path, &socket).await
}

/// 解析 play 参数：
/// - `play netease|qq|local <id或路径>` → 精确播
/// - `play "歌名"` → 搜索后播
/// - `play "歌单名"` / 已有本地文件路径
#[allow(clippy::too_many_arguments)]
async fn cmd_play_resolve(
    api: ApiClient,
    store: &PlaylistStore,
    paths: &AppPaths,
    cfg: &mixly::Config,
    targets: Vec<String>,
    loop_flag: bool,
    random: bool,
    search_platform: PlatformArg,
    force_playlist: bool,
    prefer: Platform,
) -> Result<()> {
    let api = Arc::new(api);

    // 两参数且第一段是平台 → 平台+ID/路径
    if targets.len() == 2 {
        if let Some(platform) = Platform::parse(&targets[0]) {
            let id = targets[1].as_str();
            let song = resolve_song_for_platform_id(&api, paths, platform, id).await?;
            if random {
                eprintln!("提示: 单曲播放忽略 --random");
            }
            let mode = if loop_flag {
                PlayMode::LoopOne
            } else {
                PlayMode::Sequential
            };
            return play_songs(api, paths, cfg, vec![song], mode, None).await;
        }
        let keyword = targets.join(" ");
        return play_by_search(
            api,
            paths,
            cfg,
            &keyword,
            search_platform,
            prefer,
            loop_flag,
            random,
        )
        .await;
    }

    let name = targets
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("请提供歌曲 ID、歌名、路径或歌单名"))?;

    if force_playlist {
        return play_local_playlist(api, paths, store, cfg, &name, loop_flag, random).await;
    }

    // 磁盘上的音频文件
    let p = std::path::Path::new(&name);
    if p.is_file() {
        let song = song_for_play_path(p)?;
        println!("播放本地文件: {}", song.display_line());
        let mode = if loop_flag {
            PlayMode::LoopOne
        } else {
            PlayMode::Sequential
        };
        return play_songs(api, paths, cfg, vec![song], mode, None).await;
    }

    // 优先本地歌单
    if store.find(&name).is_ok() {
        println!("按本地歌单播放：「{name}」");
        return play_local_playlist(api, paths, store, cfg, &name, loop_flag, random).await;
    }

    play_by_search(
        api,
        paths,
        cfg,
        &name,
        search_platform,
        prefer,
        loop_flag,
        random,
    )
    .await
}

async fn resolve_song_for_platform_id(
    api: &ApiClient,
    paths: &AppPaths,
    platform: Platform,
    id: &str,
) -> Result<Song> {
    if platform == Platform::Local {
        if let Ok(lib) = LocalLibrary::open(paths) {
            if let Some(s) = lib.get(id) {
                return Ok(s.clone());
            }
        }
        return song_for_play_path(std::path::Path::new(id));
    }
    match api.song_detail(platform, id).await {
        Ok(s) => Ok(s),
        Err(e) => {
            warn!(%platform, id, error = %e, "song detail failed; using bare id");
            Ok(Song {
                platform,
                id: id.to_string(),
                name: id.to_string(),
                artists: vec![],
                album: None,
                duration_ms: None,
                cover_url: None,
            })
        }
    }
}

/// 歌单播放模式：random 打乱曲目顺序；loop 决定播完是否循环列表。
fn playlist_play_mode(loop_flag: bool, random: bool) -> (PlayMode, bool) {
    // returns (mode, do_shuffle_vec)
    match (loop_flag, random) {
        (_, true) => {
            // 先打乱再顺序/列表循环，保证整张歌单每轮随机顺序且 Sequential 能正常结束
            let mode = if loop_flag {
                PlayMode::LoopAll
            } else {
                PlayMode::Sequential
            };
            (mode, true)
        }
        (true, false) => (PlayMode::LoopAll, false),
        (false, false) => (PlayMode::Sequential, false),
    }
}

async fn play_local_playlist(
    api: Arc<ApiClient>,
    paths: &AppPaths,
    store: &PlaylistStore,
    cfg: &mixly::Config,
    id_or_name: &str,
    loop_flag: bool,
    random: bool,
) -> Result<()> {
    let pl = store.find(id_or_name)?;
    if pl.songs.is_empty() {
        bail!("歌单「{}」为空", pl.name);
    }
    let mut songs = pl.songs;
    let (mode, do_shuffle) = playlist_play_mode(loop_flag, random);
    if do_shuffle {
        mixly::playlist::shuffle(&mut songs);
        println!(
            "歌单「{}」随机播放（{} 首{}）",
            pl.name,
            songs.len(),
            if loop_flag { "，列表循环" } else { "" }
        );
    } else if loop_flag {
        println!("歌单「{}」列表循环（{} 首）", pl.name, songs.len());
    } else {
        println!("播放歌单「{}」（{} 首）", pl.name, songs.len());
    }
    play_songs(api, paths, cfg, songs, mode, Some(store)).await
}

#[allow(clippy::too_many_arguments)]
async fn play_by_search(
    api: Arc<ApiClient>,
    paths: &AppPaths,
    cfg: &mixly::Config,
    keyword: &str,
    platform: PlatformArg,
    prefer: Platform,
    loop_flag: bool,
    random: bool,
) -> Result<()> {
    println!(
        "按歌名搜索并播放：「{keyword}」…（在线优先 {}）",
        prefer.label_zh()
    );
    let mut hits = Vec::new();
    // 本地库优先于在线，便于 play 本地曲名
    if platform.includes_local() {
        match api.search(Platform::Local, keyword, 5).await {
            Ok(mut songs) => hits.append(&mut songs),
            Err(e) => eprintln!("[local] 搜索失败: {e:#}"),
        }
    }
    for p in platform.as_online_platforms(prefer) {
        if p == Platform::Bilibili && !api.is_logged_in(Platform::Bilibili) {
            if !matches!(platform, PlatformArg::All) {
                eprintln!("[bilibili] Bilibili 未登录，请执行 mixly login --platform bilibili");
            }
            continue;
        }
        match api.search(p, keyword, 5).await {
            Ok(mut songs) => hits.append(&mut songs),
            Err(e) => eprintln!("[{p}] 搜索失败: {e:#}"),
        }
    }
    if hits.is_empty() {
        bail!("没有搜到与「{keyword}」相关的歌曲（也不是本地歌单名）");
    }

    // 优先歌名完全匹配（忽略大小写）；匹配项中已按 prefer 平台顺序排在前
    // 否则取列表第 0 条（即优先平台的搜索第一条）
    let keyword_l = keyword.to_lowercase();
    let idx = hits
        .iter()
        .position(|s| s.name.to_lowercase() == keyword_l)
        .unwrap_or(0);
    let song = hits.swap_remove(idx);
    println!("选中: {}  id={}", song.display_line(), song.id);
    if random {
        eprintln!("提示: 单曲播放忽略 --random");
    }
    let mode = if loop_flag {
        PlayMode::LoopOne
    } else {
        PlayMode::Sequential
    };
    play_songs(api, paths, cfg, vec![song], mode, None).await
}

async fn play_songs(
    api: Arc<ApiClient>,
    paths: &AppPaths,
    cfg: &mixly::Config,
    songs: Vec<Song>,
    mode: PlayMode,
    store: Option<&PlaylistStore>,
) -> Result<()> {
    if songs.is_empty() {
        bail!("没有可播放的歌曲");
    }
    let client = build_http_client().await?;
    let relay = AudioRelay::start(client.clone()).await?;
    let mut player = spawn_player(cfg).await?;
    let mut queue = PlayQueue::from_songs(songs);
    queue.set_mode(mode);

    println!("经本地中继播放: {}", relay.current_url());
    match mode {
        PlayMode::LoopOne => println!("模式: 单曲循环；按 Ctrl+C 停止。"),
        PlayMode::LoopAll => println!("模式: 列表循环；按 Ctrl+C 停止。"),
        PlayMode::Shuffle => println!("模式: 随机；按 Ctrl+C 停止。"),
        PlayMode::Sequential => println!("保持此进程运行；按 Ctrl+C 停止。"),
    }

    // 进行中的预取任务；切歌时若命中则直接用本地缓存。
    let mut prefetch_job: Option<PrefetchJob> = None;
    let mut ready_prefetch: Option<PrefetchedTrack> = None;

    while let Some(mut song) = queue.current().cloned() {
        // 歌单/旧条目可能缺专辑：播放前按平台补全，并回写本地歌单。
        song = api.enrich_song_metadata(&song).await;
        queue.update_current(song.clone());
        if let Some(store) = store {
            if let Err(e) = store.backfill_album(&song) {
                warn!(error = %e, "persist enriched album failed");
            }
        }
        println!("→ {}", song.display_line());

        // 若预取完成且就是本曲，用缓存；否则实时拉流
        let mut prefetched = None;
        if let Some(p) = ready_prefetch.take() {
            if p.matches(&song) {
                println!("  （使用预取缓存）");
                prefetched = Some(p);
            } else {
                p.cleanup();
            }
        }
        // 进行中的任务若匹配本曲则等待完成
        if prefetched.is_none() {
            if let Some(job) = prefetch_job.take() {
                if job.matches_song(&song) {
                    match job.join().await {
                        Ok(p) => {
                            println!("  （使用预取缓存）");
                            prefetched = Some(p);
                        }
                        Err(e) => log_prefetch_fail(&e),
                    }
                } else {
                    job.abort();
                }
            }
        }

        let mut lyrics_state =
            match load_into_player(api.clone(), &relay, &mut player, &song, prefetched).await {
                Ok(state) => state,
                Err(e) => {
                    eprintln!("播放错误: {e:#}");
                    if mode == PlayMode::LoopOne {
                        break;
                    }
                    if !queue.advance() {
                        break;
                    }
                    continue;
                }
            };

        let mut saw_play = false;
        let started = std::time::Instant::now();
        let mut prefetch_started = false;
        loop {
            tokio::time::sleep(Duration::from_millis(400)).await;
            if let Err(e) = player.ensure_alive().await {
                warn!(error = %e, "mpv 重启失败");
                break;
            }
            match player.snapshot().await {
                Ok(snap) => {
                    if snap.duration > 0.0 || snap.time_pos > 0.5 {
                        saw_play = true;
                    }
                    if saw_play && snap.eof {
                        break;
                    }
                    if !saw_play && started.elapsed() > Duration::from_secs(20) {
                        eprintln!("等待开播超时，跳过本曲");
                        break;
                    }

                    // 写入播放状态，供 `mixly status --claude` 读取
                    lyrics_state.update_index(snap.time_pos + if snap.paused { 0.0 } else { 0.8 });
                    let np = mixly::status::NowPlaying {
                        version: mixly::status::NOW_PLAYING_VERSION,
                        pid: std::process::id(),
                        updated_at_ms: mixly::status::now_ms(),
                        platform: song.platform.as_str().to_string(),
                        song_id: song.id.clone(),
                        title: song.name.clone(),
                        artists: song.artists.clone(),
                        position_secs: snap.time_pos,
                        duration_secs: snap.duration,
                        paused: snap.paused,
                        lyric: lyrics_state.current_line().map(|s| s.to_string()),
                    };
                    if let Err(e) = mixly::status::write_now_playing(&paths.now_playing, &np) {
                        warn!(error = %e, "写入播放状态失败");
                    }

                    // 进度 ≥70% 或剩余 ≤18s → 预取下一首（Bilibili 不预取）
                    if !prefetch_started
                        && prefetch_job.is_none()
                        && ready_prefetch.is_none()
                        && should_prefetch(snap.time_pos, snap.duration)
                    {
                        if let Some(next) = queue.peek_next().cloned() {
                            if prefetchable(next.platform) {
                                log_prefetch_start(&next);
                                prefetch_job =
                                    Some(PrefetchJob::spawn(api.clone(), client.clone(), next));
                                prefetch_started = true;
                            }
                        }
                    }

                    // 预取已完成则收进 ready，避免 join 阻塞在切歌瞬间
                    if let Some(res) = mixly::prefetch::try_take_finished(&mut prefetch_job).await {
                        match res {
                            Ok(p) => ready_prefetch = Some(p),
                            Err(e) => log_prefetch_fail(&e),
                        }
                    }
                }
                Err(_) => {
                    if started.elapsed() > Duration::from_secs(20) {
                        break;
                    }
                }
            }
        }

        if !queue.advance() {
            println!("播放队列已结束。");
            break;
        }
        if mode == PlayMode::LoopOne {
            println!("↻ 单曲循环，重新播放…");
        }
    }

    if let Some(job) = prefetch_job.take() {
        job.abort();
    }
    drop(ready_prefetch.take());

    // 正常退出：仅当状态文件的 pid 仍是本进程时清理
    mixly::status::clear_now_playing(&paths.now_playing, std::process::id());
    let _ = player.shutdown().await;
    Ok(())
}

async fn load_into_player(
    api: Arc<ApiClient>,
    relay: &AudioRelay,
    player: &mut MpvPlayer,
    song: &Song,
    prefetched: Option<PrefetchedTrack>,
) -> Result<LyricsState> {
    let track = if let Some(p) = prefetched {
        p.into_track()
    } else {
        let t = api.resolve_track(song).await?;
        debug_assert!(!song.platform.is_online() || t.refresh.is_some());
        t
    };
    relay.set_track(track).await;
    player.loadfile(&relay.current_url()).await?;
    player.set_pause(false).await?;

    // Bilibili 无歌词返回空内容；其余平台失败也按无歌词处理，不阻断播放
    match api.lyrics(song.platform, &song.id).await {
        Ok(lrc) if !lrc.is_empty() => {
            let state = lyrics_state_from_lrc(&lrc);
            if let Some((_, line)) = state.lines.first() {
                println!("  ♪ {line}");
            }
            Ok(state)
        }
        _ => Ok(LyricsState::default()),
    }
}

async fn cmd_playlist(
    api: Arc<ApiClient>,
    store: &PlaylistStore,
    paths: &AppPaths,
    cfg: &mixly::Config,
    action: PlaylistCmd,
) -> Result<()> {
    match action {
        PlaylistCmd::Create { name } => {
            let pl = store.create(&name)?;
            println!("已创建歌单「{}」id={}", pl.name, pl.id);
        }
        PlaylistCmd::List => {
            let list = store.list()?;
            if list.is_empty() {
                println!("（暂无歌单）");
            }
            for pl in list {
                println!("{}  {}  （{} 首）", pl.id, pl.name, pl.songs.len());
            }
        }
        PlaylistCmd::Show { id_or_name } => {
            let pl = store.find(&id_or_name)?;
            println!("{} — {}", pl.name, pl.id);
            for (i, s) in pl.songs.iter().enumerate() {
                println!("  {i}: {}", s.display_line());
            }
        }
        PlaylistCmd::Delete { id_or_name } => {
            store.delete(&id_or_name)?;
            println!("已删除歌单 {id_or_name}");
        }
        PlaylistCmd::Rename {
            id_or_name,
            new_name,
        } => {
            let pl = store.rename(&id_or_name, &new_name)?;
            println!("已重命名为「{}」", pl.name);
        }
        PlaylistCmd::Add {
            id_or_name,
            platform,
            song_id,
            name,
            artist,
        } => {
            let platform = parse_platform(&platform)?;
            let song = if platform == Platform::Local {
                let mut s = song_for_play_path(std::path::Path::new(&song_id))?;
                // 同时写入本地库，便于 search
                if let Ok(mut lib) = LocalLibrary::open(paths) {
                    let _ = lib.import_path(std::path::Path::new(&song_id));
                }
                if let Some(n) = name {
                    s.name = n;
                }
                if let Some(a) = artist {
                    s.artists = vec![a];
                }
                s
            } else {
                // 按歌曲所属平台拉详情补全专辑等；CLI --name/--artist 可覆盖
                let mut song = match api.song_detail(platform, &song_id).await {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(%platform, id = %song_id, error = %e, "song detail failed; bare add");
                        Song {
                            platform,
                            id: song_id.clone(),
                            name: song_id.clone(),
                            artists: vec![],
                            album: None,
                            duration_ms: None,
                            cover_url: None,
                        }
                    }
                };
                if let Some(n) = name {
                    song.name = n;
                }
                if let Some(a) = artist {
                    song.artists = vec![a];
                }
                song
            };
            let pl = store.add_song(&id_or_name, song)?;
            println!("已加入「{}」（共 {} 首）", pl.name, pl.songs.len());
        }
        PlaylistCmd::Remove { id_or_name, index } => {
            let pl = store.remove_at(&id_or_name, index)?;
            println!(
                "已从「{}」移除下标 {index}（剩余 {} 首）",
                pl.name,
                pl.songs.len()
            );
        }
        PlaylistCmd::Move {
            id_or_name,
            from,
            to,
        } => {
            let pl = store.reorder(&id_or_name, from, to)?;
            println!(
                "已调整「{}」顺序：{} → {}（共 {} 首）",
                pl.name,
                from,
                to,
                pl.songs.len()
            );
            for (i, s) in pl.songs.iter().enumerate() {
                println!("  {i}: {}", s.display_line());
            }
        }
        PlaylistCmd::Play {
            id_or_name,
            r#loop,
            random,
        } => {
            play_local_playlist(api, paths, store, cfg, &id_or_name, r#loop, random).await?;
        }
    }
    Ok(())
}

async fn cmd_tui(
    api: ApiClient,
    store: PlaylistStore,
    cfg: &mixly::Config,
    proxy: ProxyDecision,
    prefer: Platform,
) -> Result<()> {
    // 未登录时不直接报错：交互选择登录网易云 / QQ / 两者。
    ensure_login_for_tui(&api).await?;

    let client = build_http_client().await?;
    let relay = AudioRelay::start(client).await?;
    let player = spawn_player(cfg).await?;
    let player = Arc::new(Mutex::new(player));

    let result = run_tui(mixly::tui::app::TuiDeps {
        api: Arc::new(api),
        store,
        relay,
        player: player.clone(),
        proxy,
        prefer,
        quality: mixly::quality_from_config(cfg),
    })
    .await;

    if let Ok(mut p) = player.try_lock() {
        let _ = p.shutdown().await;
    }
    result
}

/// If no platform token exists, prompt for login (single or both) before TUI.
async fn ensure_login_for_tui(api: &ApiClient) -> Result<()> {
    let ne = api.is_logged_in(Platform::Netease);
    let qq = api.is_logged_in(Platform::Qq);

    if ne || qq {
        let mut parts = Vec::new();
        if ne {
            parts.push("网易云");
        }
        if qq {
            parts.push("QQ 音乐");
        }
        println!("已登录: {}", parts.join("、"));
        return Ok(());
    }

    println!();
    println!("当前尚未登录任何平台。");
    println!("请选择登录方式后进入 TUI：");
    println!("  1) 仅登录网易云");
    println!("  2) 仅登录 QQ 音乐");
    println!("  3) 登录两个平台（先网易云，再 QQ）");
    println!("  0) 跳过登录，直接进入（部分歌曲可能无法播放）");
    print!("请输入选项 [0-3]，默认 3: ");
    let _ = std::io::Write::flush(&mut std::io::stdout());

    let choice = read_line_trimmed()?;
    let choice = if choice.is_empty() {
        "3".to_string()
    } else {
        choice
    };

    match choice.as_str() {
        "1" => {
            api.login_qr(Platform::Netease).await?;
        }
        "2" => {
            api.login_qr(Platform::Qq).await?;
        }
        "3" => {
            api.login_qr(Platform::Netease).await?;
            println!();
            api.login_qr(Platform::Qq).await?;
        }
        "0" => {
            println!("已跳过登录，进入 TUI。");
        }
        other => {
            bail!("无效选项「{other}」，请重新运行 mixly tui 并输入 0–3");
        }
    }
    Ok(())
}

fn read_line_trimmed() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin()
        .read_line(&mut buf)
        .context("读取键盘输入失败")?;
    Ok(buf.trim().to_string())
}
