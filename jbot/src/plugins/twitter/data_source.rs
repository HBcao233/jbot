use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use jiff::Timestamp;
use regex::regex;
use serde_json::{Value, json};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use wreq::StatusCode;
use wreq::header::{AUTHORIZATION, COOKIE, HeaderMap, HeaderName, HeaderValue};

use super::tweet::{GetTweetError, Tweet, TweetRaw};

const GET_TWEET_DETAIL_URL: &'static str =
    "https://x.com/i/api/graphql/u5Tij6ERlSH2LZvCUqallw/TweetDetail";

const FEATURES: &str = r#"{"rweb_video_screen_enabled":false,"payments_enabled":false,"rweb_xchat_enabled":false,"profile_label_improvements_pcf_label_in_post_enabled":true,"rweb_tipjar_consumption_enabled":true,"verified_phone_label_enabled":false,"creator_subscriptions_tweet_preview_api_enabled":true,"responsive_web_graphql_timeline_navigation_enabled":true,"responsive_web_graphql_skip_user_profile_image_extensions_enabled":false,"premium_content_api_read_enabled":false,"communities_web_enable_tweet_community_results_fetch":true,"c9s_tweet_anatomy_moderator_badge_enabled":true,"responsive_web_grok_analyze_button_fetch_trends_enabled":false,"responsive_web_grok_analyze_post_followups_enabled":true,"responsive_web_jetfuel_frame":true,"responsive_web_grok_share_attachment_enabled":true,"articles_preview_enabled":true,"responsive_web_edit_tweet_api_enabled":true,"graphql_is_translatable_rweb_tweet_is_translatable_enabled":true,"view_counts_everywhere_api_enabled":true,"longform_notetweets_consumption_enabled":true,"responsive_web_twitter_article_tweet_consumption_enabled":true,"tweet_awards_web_tipping_enabled":false,"responsive_web_grok_show_grok_translated_post":false,"responsive_web_grok_analysis_button_from_backend":false,"creator_subscriptions_quote_tweet_preview_enabled":false,"freedom_of_speech_not_reach_fetch_enabled":true,"standardized_nudges_misinfo":true,"tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled":true,"longform_notetweets_rich_text_read_enabled":true,"longform_notetweets_inline_media_enabled":true,"responsive_web_grok_image_annotation_enabled":true,"responsive_web_grok_imagine_annotation_enabled":true,"responsive_web_grok_community_note_auto_translation_is_enabled":false,"responsive_web_enhance_cards_enabled":false}"#;
const FIELD_TOGGLES: &str = r#"{"withArticleRichContentState":true,"withArticlePlainText":false,"withGrokAnalyze":false,"withDisallowedReplyControls":false}"#;

fn csrf_token() -> String {
    super::CSRF_TOKEN.get().unwrap().to_string()
}

fn auth_token() -> String {
    super::AUTH_TOKEN.get().unwrap().to_string()
}

fn twitter_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA"));
    headers.insert(
        COOKIE,
        HeaderValue::from_str(&format!(
            "auth_token={}; ct0={}",
            auth_token(),
            csrf_token()
        ))
        .unwrap(),
    );
    headers.insert(
        HeaderName::from_static("x-csrf-token"),
        HeaderValue::from_str(&csrf_token()).unwrap(),
    );
    headers.insert(
        HeaderName::from_static("x-twitter-client-language"),
        HeaderValue::from_static("zh-cn"),
    );
    headers.insert(
        HeaderName::from_static("x-twitter-active-user"),
        HeaderValue::from_static("yes"),
    );
    headers
}

pub async fn get_tweet(client: &wreq::Client, tid: &str) -> Result<Tweet, GetTweetError> {
    let cache_dir = Path::new("cache/tweets");
    if let Err(e) = fs::create_dir_all(cache_dir).await {
        log::error!("缓存文件夹创建失败: {e:?}");
    }

    let cache_file = cache_dir.join(&format!("{tid}.json"));
    let mut res: Value = if let Ok(text) = fs::read_to_string(&cache_file).await {
        log::info!("使用缓存: {}", cache_file.display());
        serde_json::from_str(&text)?
    } else {
        let variables = json!({
            "focalTweetId": tid,
            "with_rux_injections": false,
            "rankingMode": "Relevance",
            "includePromotedContent": true,
            "withCommunity": true,
            "withQuickPromoteEligibilityTweetFields": true,
            "withBirdwatchNotes": true,
            "withVoice": true,
        });

        let response = client
            .get(GET_TWEET_DETAIL_URL)
            .headers(twitter_headers())
            .query(&[
                ("variables", variables.to_string()),
                ("features", FEATURES.to_string()),
                ("fieldToggles", FIELD_TOGGLES.to_string()),
            ])
            .send()
            .await?;
        let status = response.status();
        if status != StatusCode::OK {
            return Err(GetTweetError::Status(status.as_u16()));
        }

        let res = response.json().await?;
        let pretty = serde_json::to_string_pretty(&res)?;
        if let Err(e) = fs::write(&cache_file, &pretty).await {
            log::error!("缓存json文件失败: {e:?}");
        } else {
            log::info!("写入缓存: {}", cache_file.display());
        }
        res
    };

    // api errors.
    if let Some(errors) = res["errors"].as_array() {
        if let Some(first) = errors.first() {
            if first["code"].as_i64() == Some(144) {
                return Err(GetTweetError::NotFound);
            }
            let msg = first["message"].as_str().unwrap_or("unknown").to_string();
            return Err(GetTweetError::Api(msg));
        }
    }

    // get entries.
    let instructions =
        res["data"]["threaded_conversation_with_injections_v2"]["instructions"].take();
    let instructions_arr = match instructions {
        Value::Array(v) => v,
        _ => {
            return Err(GetTweetError::ParseFailed(
                "instructions not found".to_string(),
            ));
        }
    };
    let entries = instructions_arr
        .into_iter()
        .find_map(|mut inst| match inst["entries"].take() {
            Value::Array(v) => Some(v),
            _ => None,
        })
        .ok_or(GetTweetError::ParseFailed("entries not found".to_string()))?
        .into_iter()
        .map(|mut v| v.take());

    // find tweet entry.
    let target_id_upper = format!("Tweet-{tid}");
    let target_id_lower = format!("tweet-{tid}");

    let mut tweet_entry = entries
        .into_iter()
        .find(|e| {
            let eid = e["entryId"].as_str().unwrap_or("");
            eid == target_id_upper || eid == target_id_lower
        })
        .ok_or(GetTweetError::ParseFailed("entry not found".to_string()))?;

    // finish parse.
    let mut tweet_results = tweet_entry["content"]["itemContent"]["tweet_results"].take();
    if tweet_results.is_null() {
        return Err(GetTweetError::ParseFailed(
            "tweet_results not found".to_string(),
        ));
    }
    let mut tweet_result = tweet_results["result"].take();
    if tweet_result.is_null() {
        return Err(GetTweetError::NotFound);
    }

    let tweet = if tweet_result["tweet"].is_null() {
        tweet_result
    } else {
        tweet_result["tweet"].take()
    };

    let raw: TweetRaw = serde_json::from_value(tweet)?;
    if let Some(tombstone) = raw.tombstone {
        return Err(GetTweetError::Api(tombstone.text.text));
    }

    Ok(raw.into())
}

pub fn parse_msg(tweet: &Tweet) -> String {
    let author = tweet.author();
    let name = &replace_unsupported_characters(author.name());
    let username = author.username();
    let tid = &tweet.id();

    let mut full_text = replace_unsupported_characters(&tweet.display_text());
    if let Some(entities) = &tweet.entities().urls {
        for entity in entities {
            full_text = full_text.replace(&entity.url, &entity.expanded_url);
        }
    }

    // @mention
    full_text = regex!(r"([^@]*[^/@]+)@([0-9a-zA-Z_]*)")
        .replace_all(&full_text, r#"${1}<a href="https://x.com/$2">@$2</a>"#)
        .to_string();

    if regex!(r"暗号|暗语").is_match(&full_text)
        && regex!(r"t\.me|通道|直通|领取|联系|渠道|进群|飞机").is_match(&full_text)
    {
        full_text = format!("\u{26a0}推文内容疑似推广诈骗，请注意甄别\n\n{full_text}");
    }

    let created_at = Timestamp::strptime("%a %b %d %H:%M:%S %z %Y", tweet.created_at());
    match created_at {
        Ok(c) => {
            full_text.push('\n');
            full_text.push_str(&c.strftime("%Y-%m-%d %H:%M:%S").to_string());
        }
        Err(e) => {
            log::error!("created_at {} 解析失败: {e}", tweet.created_at());
        }
    }

    let msg = format!("<a href=\"https://x.com/{username}/status/{tid}\">{name} - X/Twitter</a>");
    if full_text.is_empty() {
        msg
    } else {
        format!("{msg}:\n<blockquote expandable>{full_text}</blockquote>")
    }
}

pub async fn download_media(
    client: &wreq::Client,
    url: String,
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

    let response = client.get(url).send().await?;

    let status = response.status();
    if !status.is_success() {
        return Err(anyhow::anyhow!(format!(
            "下载失败，HTTP 状态码：{}",
            status,
        )));
    }

    let mut stream = response.bytes_stream();
    let mut file = fs::File::create(&path).await?;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;

        file.write_all(&chunk).await?;
    }

    file.flush().await?;

    Ok(path)
}

fn replace_unsupported_characters(s: &str) -> String {
    s.replace('\u{17b5}', "\\u17b5")
}
