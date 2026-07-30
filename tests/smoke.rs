//! Live API smoke tests — run with `cargo test -- --ignored`.

use mixly::{ApiClient, AppPaths, Platform, Quality};

#[tokio::test]
#[ignore]
async fn smoke_netease_search_url_lyrics() {
    let paths = AppPaths::discover().expect("paths");
    let api = ApiClient::new(paths, Quality::Exhigh);
    let songs = api
        .search(Platform::Netease, "海阔天空", 5)
        .await
        .expect("netease search");
    assert!(!songs.is_empty(), "expected netease search hits");

    // Try several hits — copyright / region may blank some URLs without login.
    let mut got_url = false;
    let mut last_err = String::new();
    for song in &songs {
        match api.play_url(Platform::Netease, &song.id).await {
            Ok(url) if url.starts_with("http") => {
                got_url = true;
                println!("netease url ok id={} prefix={}", song.id, &url[..url.len().min(48)]);
                break;
            }
            Ok(_) => last_err = format!("empty url for {}", song.id),
            Err(e) => last_err = e.to_string(),
        }
    }
    if !got_url {
        eprintln!(
            "netease play url unavailable without login / region ({last_err}); search still OK"
        );
    }

    let lrc = api
        .lyrics(Platform::Netease, &songs[0].id)
        .await
        .expect("netease lyrics");
    assert!(!lrc.is_empty(), "expected lyrics text");
}

#[tokio::test]
#[ignore]
async fn smoke_qq_search_url_lyrics() {
    let paths = AppPaths::discover().expect("paths");
    let api = ApiClient::new(paths, Quality::Exhigh);
    let songs = api
        .search(Platform::Qq, "晴天", 3)
        .await
        .expect("qq search");
    assert!(!songs.is_empty(), "expected qq search hits");
    let song = &songs[0];
    match api.play_url(Platform::Qq, &song.id).await {
        Ok(u) => {
            assert!(u.starts_with("http"), "url={u}");
            println!("qq url ok prefix={}", &u[..u.len().min(48)]);
        }
        Err(e) => {
            eprintln!("qq play url needs login (expected without token): {e:#}");
        }
    }
    match api.lyrics(Platform::Qq, &song.id).await {
        Ok(t) => println!("qq lyrics bytes={}", t.len()),
        Err(e) => eprintln!("qq lyrics: {e:#}"),
    }
}
