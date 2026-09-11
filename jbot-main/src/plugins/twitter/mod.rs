mod data_source;
mod tweet;

use std::env;
use std::sync::{Arc, OnceLock};

use anyhow::Context;
use data_source::{get_tweet, parse_msg};
use grammers_client::Client;
use grammers_client::media::{InputMedia, Media};
use grammers_client::message::{InputMessage, Message};
use grammers_session::types::{PeerKind, PeerRef};
use grammers_tl_types as tl;
use regex::regex;

use crate::curl::stream_download;
use crate::database as db;

const HELP: &'static str = r#"推特解析，支持批量解析多条链接。
用法: /tid <url/tid> [url2 url3...]"#;

static CSRF_TOKEN: OnceLock<String> = OnceLock::new();
static AUTH_TOKEN: OnceLock<String> = OnceLock::new();

#[crate::on_setup]
fn setup() -> anyhow::Result<()> {
    let csrf_token = env::var("twitter_csrf_token")
        .context("environment variable \"twitter_csrf_token\" not found")?;
    let auth_token = env::var("twitter_auth_token")
        .context("environment variable \"twitter_auth_token\" not found")?;
    let _ = CSRF_TOKEN.set(csrf_token);
    let _ = AUTH_TOKEN.set(auth_token);
    Ok(())
}

#[crate::on_new_message]
async fn handler(client: Client, message: Arc<Message>) {
    if message.outgoing() {
        return;
    }

    let peer_id = message.peer_id();
    if peer_id.kind() != PeerKind::User {
        return;
    }

    let peer_ref = message
        .peer_ref()
        .await
        .ok()
        .and_then(|x| x)
        .unwrap_or_else(|| peer_id.to_ambient_ref());

    let msg_id = message.id();
    let text = message.text();
    let mut matched = false;
    for (_, [tid]) in
        regex!(r"(?:https?://)?[a-z]*?(?:twitter|x)\.com/[a-zA-Z0-9_]+/status/(\d{13,20})")
            .captures_iter(text)
            .map(|c| c.extract())
    {
        matched = true;
        if let Err(e) = send_twitter(client.clone(), peer_ref, msg_id, tid.to_string()).await {
            log::error!("发送twitter失败: {e:?}");
        }
    }

    if !matched && text.starts_with("/tid") {
        if let Err(e) = client
            .send_message(
                peer_ref,
                InputMessage::new().text(HELP).reply_to(Some(msg_id)),
            )
            .await
        {
            log::error!("发送帮助信息失败: {e:?}");
        }
    }
}

async fn send_twitter(
    client: Client,
    peer_ref: PeerRef,
    msg_id: i32,
    tid: String,
) -> anyhow::Result<()> {
    log::info!("tid: {tid}");

    let mid = client
        .send_message(
            peer_ref,
            InputMessage::new()
                .text(format!("[{tid}] 请等待..."))
                .reply_to(Some(msg_id)),
        )
        .await?;

    let wreq_client = crate::curl::get_client().build()?;
    let tweet = match get_tweet(&wreq_client, &tid).await {
        Ok(tweet) => tweet,
        Err(e) => {
            mid.edit(format!("[{tid}] {e}")).await?;
            return Ok(());
        }
    };

    let msg = parse_msg(&tweet);
    let mut medias = Vec::with_capacity(4);
    if let Some(entities) = &tweet.entities().media {
        let headers = Vec::new();
        let count = entities.len();
        for (index, media) in entities.into_iter().enumerate() {
            let media_type = media.r#type.as_str();
            let key = format!("{tid}_{}", index + 1);

            let cache = match media_type {
                "photo" => db::photo::get(key.clone()).await?.map(|photo| {
                    InputMedia::new().media(tl::types::InputMediaPhoto {
                        spoiler: false,
                        id: photo,
                        ttl_seconds: None,
                        live_photo: false,
                        video: None,
                    })
                }),
                "video" => db::video::get(key.clone()).await?.map(|document| {
                    InputMedia::new().media(tl::types::InputMediaDocument {
                        spoiler: false,
                        id: document,
                        video_cover: None,
                        video_timestamp: None,
                        ttl_seconds: None,
                        query: None,
                    })
                }),
                _ => {
                    log::warn!("不支持的媒体类型: {}", media_type);
                    continue;
                }
            };

            let mut input_media = if let Some(m) = cache {
                log::info!("使用已发送过的媒体: {key}");
                m
            } else {
                mid.edit(format!("[{tid}] 媒体下载中 {} / {}...", index + 1, count))
                    .await?;

                let (ext, url) = match media_type {
                    "photo" => {
                        let url = &media.media_url_https;
                        let url = if url.contains('?') {
                            format!("{}&name=orig", url)
                        } else {
                            format!("{}?name=orig", url)
                        };
                        ("jpg", url)
                    }
                    "video" => {
                        let video = media
                            .video_info
                            .as_ref()
                            .unwrap()
                            .variants
                            .iter()
                            .max_by_key(|v| {
                                if v.content_type == "video/mp4" {
                                    v.bitrate.unwrap_or(0)
                                } else {
                                    0
                                }
                            });
                        ("mp4", video.unwrap().url.clone())
                    }
                    _ => {
                        log::warn!("不支持的媒体类型: {}", media_type);
                        continue;
                    }
                };
                let name = format!("{key}.{ext}");

                let path = match stream_download(&wreq_client, url, &name, &headers).await {
                    Ok(path) => path,
                    Err(e) => {
                        let tip = format!("[{tid}] 媒体 {} 下载失败", index + 1);
                        log::error!("{tip}: {e}");
                        mid.edit(tip).await?;
                        return Ok(());
                    }
                };

                mid.edit(format!("[{tid}] 媒体上传中 {} / {}...", index + 1, count))
                    .await?;
                let Ok(uploaded) = client.upload_file(path).await else {
                    mid.edit(format!("[{tid}] 媒体 {} 上传失败", index + 1))
                        .await?;
                    return Ok(());
                };

                match media_type {
                    "photo" => InputMedia::new().mime_type("image/jpeg").photo(uploaded),
                    "video" => {
                        let thumb_url = &media.media_url_https;
                        let thumb_url = if thumb_url.contains('?') {
                            format!("{}&name=orig", thumb_url)
                        } else {
                            format!("{}?name=orig", thumb_url)
                        };
                        let thumb_name = format!("{key}_thumb.jpg");
                        let thumb = match stream_download(&wreq_client, thumb_url, &thumb_name, &headers).await
                        {
                            Ok(path) => match client.upload_file(path).await {
                                Ok(uploaded) => Some(uploaded.raw),
                                Err(e) => {
                                    log::warn!("上传 {thumb_name} 失败: {e}");
                                    None
                                }
                            },
                            Err(e) => {
                                log::warn!("下载 {thumb_name} 失败: {e}");
                                None
                            }
                        };

                        let m = tl::types::InputMediaUploadedDocument {
                            nosound_video: true,
                            force_file: false,
                            spoiler: false,
                            file: uploaded.raw,
                            thumb,
                            mime_type: "video/mp4".to_string(),
                            attributes: vec![
                                tl::types::DocumentAttributeFilename { file_name: name }.into(),
                                tl::types::DocumentAttributeVideo {
                                    round_message: false,
                                    supports_streaming: true,
                                    nosound: false,
                                    duration: media.video_info.as_ref().unwrap().duration_millis
                                        / 1000.0,
                                    w: media.original_info.width,
                                    h: media.original_info.height,
                                    preload_prefix_size: None,
                                    video_start_ts: None,
                                    video_codec: None,
                                }
                                .into(),
                            ],
                            stickers: None,
                            ttl_seconds: None,
                            video_cover: None,
                            video_timestamp: None,
                        };
                        InputMedia::new().media(m)
                    }
                    _ => {
                        log::warn!("不支持的媒体类型: {}", media_type);
                        continue;
                    }
                }
            };

            if index == 0 {
                input_media = input_media.html(&msg).reply_to(Some(msg_id));
            }
            medias.push(input_media)
        }
    }

    if medias.is_empty() {
        client
            .send_message(
                peer_ref,
                InputMessage::new().html(msg).reply_to(Some(msg_id)),
            )
            .await?;
    } else {
        let messages = match client.send_album(peer_ref, medias).await {
            Ok(m) => m,
            Err(e) => {
                let text = format!("[{tid}] 媒体发送失败");
                log::error!("{text}: {e:?}");
                mid.edit(text).await?;
                return Ok(());
            }
        };

        for (index, message) in messages.into_iter().enumerate() {
            let key = format!("{tid}_{}", index + 1);
            if let Some(m) = message {
                if let Some(media) = m.media() {
                    match media {
                        Media::Photo(photo) => match photo.to_raw_input_photo() {
                            tl::enums::InputPhoto::Photo(x) => {
                                db::photo::insert(key.clone(), x).await?;
                                log::info!("添加缓存图片: {key}");
                            }
                            _ => {}
                        },
                        Media::Document(document) => {
                            if crate::utils::is_video(&document) {
                                match document.to_raw_input_document() {
                                    tl::enums::InputDocument::Document(x) => {
                                        db::video::insert(key.clone(), x).await?;
                                        log::info!("添加缓存视频: {key}");
                                    }
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    mid.delete().await?;

    Ok(())
}
