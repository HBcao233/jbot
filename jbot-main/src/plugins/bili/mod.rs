mod abv;
mod data_source;
mod types;

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use data_source::{get_bili, get_gaia, get_playurl, parse_msg, validate_gaia};
use grammers_client::Client;
use grammers_client::media::Media;
use grammers_client::message::{Button, InputMessage, Message, ReplyMarkup};
use grammers_session::types::{PeerId, PeerKind, PeerRef};
use grammers_tl_types as tl;
use regex::regex;
use tokio::fs;
use tokio::process::Command;
use tokio::sync::{Mutex, oneshot};
use types::BiliId;

use crate::curl::{stream_download, stream_download_with_callback};
use crate::database as db;
use crate::progress::{Progress, ProgressScheduler};
use crate::utils::upload_file_with_callback;

const HELP: &str = "Bilibili 解析。用法: /bili <url>";

static SESSDATA: OnceLock<Option<String>> = OnceLock::new();

type Gaia = Arc<Mutex<HashMap<PeerId, oneshot::Sender<(String, String)>>>>;
static GAIA: OnceLock<Gaia> = OnceLock::new();

#[crate::on_setup]
fn setup() -> anyhow::Result<()> {
    let sessdata = env::var("bili_SESSDATA").ok();
    if sessdata.is_none() {
        log::warn!("未提供 bili_SESSDATA, 可能会解析失败或画质受限");
    }
    let _ = SESSDATA.set(sessdata);
    Ok(())
}

fn gaia() -> Gaia {
    Arc::clone(GAIA.get_or_init(|| Arc::new(Mutex::new(HashMap::new()))))
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

    let re =
        regex!(r"(?:(?:https?://)?bilibili\.com/video/)?(av\d{2,16}|(?:BV|bv)[0-9a-zA-Z]{8,12})");
    let b23_re = regex!(r"(?:https?://)?b23\.tv\\?/([0-9a-zA-Z]{7,7})");

    let msg_id = message.id();
    let mut text = message.text().to_string();
    // log::info!("text: {text}");
    if text.starts_with("validate=") {
        if let Some(tx) = gaia().lock().await.remove(&peer_id) {
            let arr: Vec<_> = text.split('&').collect();
            let [mut validate, mut seccode] = arr[..2] else {
                let _ = message.reply("结果格式不正确").await;
                return;
            };
            validate = validate.split('=').last().unwrap();
            seccode = seccode.split('=').last().unwrap();
            log::info!("validate: {}, seccode: {}", validate, seccode);
            let _ = tx.send((validate.to_string(), seccode.to_string()));
        } else {
            let _ = message.reply("验证已过期").await;
        }
        return;
    }

    let starts_with_bili = text.starts_with("/bili");

    if let Some(caps) = b23_re.captures(&text) {
        let (_, [b23_id]) = caps.extract();
        log::info!("b23.tv: {b23_id}");
        let url = format!("https://b23.tv/{}", b23_id);
        let wreq_client = crate::curl::get_client().build().unwrap();
        let response = match wreq_client.get(url).send().await {
            Ok(r) => r,
            Err(e) => {
                log::error!("b23.tv请求失败: {e}");
                if let Err(e) = message.reply("短链解析失败").await {
                    log::error!("消息发送失败: {e}");
                }
                return;
            }
        };
        if let Some(location) = response
            .headers()
            .get("location")
            .and_then(|l| l.to_str().ok())
        {
            text = location.to_string();
        } else {
            if let Err(e) = message.reply("短链解析失败").await {
                log::error!("消息发送失败: {e}");
            }
            return;
        }
    }

    let mut matched = false;
    if let Some(caps) = re.captures(&text) {
        let (_, [id]) = caps.extract();
        log::info!("input: {id}");
        let bili_id = if id.starts_with('a') {
            let id = id.strip_prefix("av").unwrap();
            BiliId::AV(id.parse().unwrap())
        } else {
            BiliId::BV(id.to_string())
        };

        matched = true;
        let Ok((aid, bvid)) = bili_id.to_raw() else {
            let _ = message.reply("avid/bvid 解析错误").await;
            return;
        };
        if let Err(e) = send_bili(client.clone(), peer_ref, msg_id, aid, bvid).await {
            log::error!("发送bili失败: {e:?}");
        }
    }

    if !matched && starts_with_bili {
        if let Err(e) = message.reply(HELP).await {
            log::error!("消息发送失败: {e}")
        }
    }
}

async fn send_bili(
    client: Client,
    peer_ref: PeerRef,
    msg_id: i32,
    aid: u64,
    bvid: String,
) -> anyhow::Result<()> {
    log::info!("aid: {aid}, bvid: {bvid}");

    let mid = client
        .send_message(
            peer_ref,
            InputMessage::new()
                .text(format!("[{bvid}] 请等待..."))
                .reply_to(Some(msg_id)),
        )
        .await?;
    let mid = Arc::new(mid);

    let wreq_client = crate::curl::get_client().build()?;
    let referer = format!("https://www.bilibili.com/video/{}/", bvid);
    let headers = vec![("referer", referer)];

    let info = match get_bili(&wreq_client, aid, &bvid, None).await {
        Ok(info) => info,
        Err(e) => match e {
            types::GetBiliError::Voucher(v_voucher) => {
                match get_gaia(&wreq_client, v_voucher).await {
                    Ok((token, challenge, gt)) => {
                        let _ = mid.delete().await;
                        let url = format!(
                            "https://hbcao233.github.io/geetest-validator/?challenge={challenge}&gt={gt}"
                        );
                        let reply_markup =
                            ReplyMarkup::from_buttons_row(&[Button::url("人机验证", url)]);
                        client.send_message(peer_ref, InputMessage::new().text("请打开下面链接进行 Bilibili 的人机验证，将验证结果发送给小派魔").reply_to(Some(msg_id)).reply_markup(reply_markup)).await?;
                        let (tx, rx) = oneshot::channel();
                        {
                            let g = gaia();
                            let mut guard = g.lock().await;
                            guard.insert(peer_ref.id, tx);
                        }

                        let (validate, seccode) = rx.await?;
                        let grisk_id =
                            match validate_gaia(&wreq_client, token, challenge, validate, seccode)
                                .await
                            {
                                Ok(g) => g,
                                Err(e) => {
                                    client
                                        .send_message(
                                            peer_ref,
                                            InputMessage::new()
                                                .text(e.to_string())
                                                .reply_to(Some(msg_id)),
                                        )
                                        .await?;
                                    return Ok(());
                                }
                            };

                        match get_bili(&wreq_client, aid, &bvid, Some(grisk_id)).await {
                            Ok(x) => x,
                            Err(e) => {
                                client
                                    .send_message(
                                        peer_ref,
                                        InputMessage::new()
                                            .text(e.to_string())
                                            .reply_to(Some(msg_id)),
                                    )
                                    .await?;
                                return Ok(());
                            }
                        }
                    }
                    Err(e) => {
                        mid.edit(e.to_string()).await?;
                        return Ok(());
                    }
                }
            }
            _ => {
                mid.edit(e.to_string()).await?;
                return Ok(());
            }
        },
    };

    let Ok((msg, page, pic)) = parse_msg(info, 1) else {
        let _ = mid.edit("视频分P 不存在").await;
        return Ok(());
    };
    let cid = page.cid;
    let duration: f64 = page.duration.into();
    let w: i32 = page.dimension.width.into();
    let h: i32 = page.dimension.height.into();
    let first_frame = page.first_frame;

    let key = format!("{bvid}_{cid}");

    let input_media: Option<tl::enums::InputMedia> = if let Some(document) =
        db::video::get(key.clone()).await?
    {
        log::info!("使用已发送过的媒体: {key}");
        Some(
            tl::types::InputMediaDocument {
                spoiler: false,
                id: document,
                video_cover: None,
                video_timestamp: None,
                ttl_seconds: None,
                query: None,
            }
            .into(),
        )
    } else {
        let playurl = match get_playurl(&wreq_client, aid, &bvid, cid, None).await {
            Ok(p) => p,
            Err(e) => match e {
                types::GetPlayurlError::Voucher(v_voucher) => {
                    match get_gaia(&wreq_client, v_voucher).await {
                        Ok((token, challenge, gt)) => {
                            let _ = mid.delete().await;
                            let url = format!(
                                "https://hbcao233.github.io/geetest-validator/?challenge={challenge}&gt={gt}"
                            );
                            let reply_markup =
                                ReplyMarkup::from_buttons_row(&[Button::url("人机验证", url)]);
                            client.send_message(peer_ref, InputMessage::new().text("请打开下面链接进行 Bilibili 的人机验证，将验证结果发送给小派魔").reply_to(Some(msg_id)).reply_markup(reply_markup)).await?;
                            let (tx, rx) = oneshot::channel();
                            {
                                let g = gaia();
                                let mut guard = g.lock().await;
                                guard.insert(peer_ref.id, tx);
                            }

                            let (validate, seccode) = rx.await?;
                            let grisk_id = match validate_gaia(
                                &wreq_client,
                                token,
                                challenge,
                                validate,
                                seccode,
                            )
                            .await
                            {
                                Ok(g) => g,
                                Err(e) => {
                                    client
                                        .send_message(
                                            peer_ref,
                                            InputMessage::new()
                                                .text(e.to_string())
                                                .reply_to(Some(msg_id)),
                                        )
                                        .await?;
                                    return Ok(());
                                }
                            };
                            log::info!("grisk_id: {grisk_id}");

                            match get_playurl(&wreq_client, aid, &bvid, cid, Some(grisk_id)).await {
                                Ok(p) => p,
                                Err(e) => {
                                    client
                                        .send_message(
                                            peer_ref,
                                            InputMessage::new()
                                                .text(e.to_string())
                                                .reply_to(Some(msg_id)),
                                        )
                                        .await?;
                                    return Ok(());
                                }
                            }
                        }
                        Err(e) => {
                            if let Err(e) = mid.edit(e.to_string()).await {
                                log::error!("消息发送失败: {e}");
                            }
                            return Ok(());
                        }
                    }
                }
                _ => {
                    if let Err(e) = mid.edit(e.to_string()).await {
                        log::error!("消息发送失败: {e}");
                    }
                    return Ok(());
                }
            },
        };

        let thumb_name = format!("{key}_thumb.jpg");
        let thumb = match stream_download(&wreq_client, first_frame, &thumb_name, &headers).await {
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

        let bar = ProgressScheduler::new(Progress::new(Arc::clone(&mid)));

        if let Some(dash) = playurl.dash {
            let audio = dash.audio.into_iter().max_by_key(|x| x.id).unwrap();
            let video = dash
                .video
                .into_iter()
                .filter(|x| &x.mime_type == "video/mp4" && x.codecs.starts_with("avc1"))
                .max_by_key(|x| x.id)
                .unwrap();
            log::info!("使用 audio id: {}", audio.id);
            log::info!("使用 video id: {}", video.id);

            let audio_url = audio.base_url;
            let video_url = video.base_url;

            let audio_name = format!("{key}_audio.mp4");
            let video_name = format!("{key}_video.mp4");
            let name = format!("{key}.mp4");

            let prefix = format!("[{bvid}] 下载音频中...");
            bar.prefix(&prefix);
            mid.edit(prefix).await?;
            let audio_path = match stream_download_with_callback(
                &wreq_client,
                audio_url,
                &audio_name,
                &headers,
                |downloaded, total| {
                    bar.sync_update(downloaded, total);
                },
            )
            .await
            {
                Ok(path) => path,
                Err(e) => {
                    let tip = format!("[{bvid}] 音频下载失败");
                    log::error!("{tip}: {e}");
                    mid.edit(tip).await?;
                    return Ok(());
                }
            };

            let prefix = format!("[{bvid}] 下载视频中...");
            bar.prefix(&prefix);
            mid.edit(prefix).await?;
            let video_path = match stream_download_with_callback(
                &wreq_client,
                video_url,
                &video_name,
                &headers,
                |downloaded, total| {
                    bar.sync_update(downloaded, total);
                },
            )
            .await
            {
                Ok(path) => path,
                Err(e) => {
                    let tip = format!("[{bvid}] 视频下载失败");
                    log::error!("{tip}: {e}");
                    mid.edit(tip).await?;
                    return Ok(());
                }
            };

            mid.edit(format!("[{bvid}] 处理中...")).await?;
            let path = match merge_media(audio_path, video_path, &name).await {
                Ok(p) => p,
                Err(e) => {
                    let tip = format!("[{bvid}] 视频处理失败");
                    log::error!("{tip}: {e}");
                    mid.edit(tip).await?;
                    return Ok(());
                }
            };

            let prefix = format!("[{bvid}] 上传中...");
            bar.prefix(&prefix);
            mid.edit(prefix).await?;
            let Ok(uploaded) = upload_file_with_callback(&client, path, |uploaded, total| {
                bar.sync_update(uploaded, total);
            })
            .await
            else {
                mid.edit(format!("[{bvid}] 上传失败")).await?;
                return Ok(());
            };

            Some(
                tl::types::InputMediaUploadedDocument {
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
                            duration,
                            w,
                            h,
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
                }
                .into(),
            )
        } else if let Some(durl) = playurl.durl {
            let url = durl.into_iter().next().unwrap().url.clone();

            mid.edit(format!("[{bvid}] 下载中...")).await?;
            let name = format!("{key}.mp4");
            let path = match stream_download(&wreq_client, url, &name, &headers).await {
                Ok(path) => path,
                Err(e) => {
                    let tip = format!("[{bvid}] 下载失败");
                    log::error!("{tip}: {e}");
                    mid.edit(tip).await?;
                    return Ok(());
                }
            };

            mid.edit(format!("[{bvid}] 上传中...")).await?;
            let Ok(uploaded) = client.upload_file(path).await else {
                mid.edit(format!("[{bvid}] 上传失败")).await?;
                return Ok(());
            };

            Some(
                tl::types::InputMediaUploadedDocument {
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
                            duration,
                            w,
                            h,
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
                }
                .into(),
            )
        } else {
            None
        }
    };

    let mut input_message = InputMessage::new().html(msg).reply_to(Some(msg_id));
    if let Some(m) = input_media {
        input_message = input_message.media(m);
    }
    match client.send_message(peer_ref, input_message).await {
        Ok(message) => {
            if let Some(media) = message.media() {
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
        Err(e) => {
            log::error!("消息发送失败: {e}");
        }
    };
    let _ = mid.delete().await;

    // 封面
    let name = format!("{key}_pic.jpg");
    let path = match stream_download(&wreq_client, pic, &name, &headers).await {
        Ok(path) => path,
        Err(e) => {
            log::error!("[{bvid}] 封面下载失败: {e}");
            return Ok(());
        }
    };

    let Ok(uploaded) = client.upload_file(path).await else {
        log::error!("[{bvid}] 上传封面失败");
        return Ok(());
    };

    client
        .send_message(
            peer_ref,
            InputMessage::new().photo(uploaded).reply_to(Some(msg_id)),
        )
        .await?;

    Ok(())
}

pub async fn merge_media(
    audio_path: PathBuf,
    video_path: PathBuf,
    name: &str,
) -> anyhow::Result<PathBuf> {
    let cache_dir = Path::new("cache");
    if let Err(e) = fs::create_dir_all(cache_dir).await {
        log::error!("缓存文件夹创建失败: {e:?}");
    }

    let path = cache_dir.join(name);
    if path.is_file() {
        return Ok(path);
    }

    let status = Command::new("ffmpeg")
        .args([
            "-i",
            audio_path.to_string_lossy().as_ref(),
            "-i",
            video_path.to_string_lossy().as_ref(),
            "-c:a",
            "copy",
            "-c:v",
            "copy",
            "-y",
            path.to_string_lossy().as_ref(),
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit()) // 想看 ffmpeg 日志就 inherit，不想看就 null
        .status()
        .await?;

    if !status.success() {
        log::error!("ffmpeg exited with status: {:?}", status.code());
        return Err(anyhow::anyhow!("convert failed"));
    }

    Ok(path)
}
