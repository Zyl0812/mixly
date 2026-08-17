pub mod api;
pub mod cli;
pub mod config;
pub mod local;
pub mod lyrics;
pub mod models;
pub mod player;
pub mod playlist;
pub mod prefetch;
pub mod qr_term;
pub mod relay;
pub mod skill;
pub mod status;
pub mod tui;

pub use local::LocalLibrary;

pub use api::ApiClient;
pub use config::{
    default_mpv_socket_path, inject_proxy_env, load_config, mask_proxy_url,
    preferred_platform_from_config, quality_from_config, resolve_proxy, save_config, AppPaths,
    Config, ProxyDecision,
};
pub use models::{LyricsState, Platform, PlayMode, Playlist, Quality, Song};
pub use playlist::{PlayQueue, PlaylistStore};
pub use relay::{range_response_meta, AudioRelay, TrackSource};
