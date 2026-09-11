use std::path::Path;
use std::sync::OnceLock;

use jiff::Timestamp;
use serde_json::Value;
use tokio::fs;
use wreq::Client;
use wreq::StatusCode;

use super::types::{
    BiliInfo, BiliResult, DescItem, FINGER_HOST, FingerResult, GAIA_VALIDATE_HOST, GAIA_VGATE_HOST,
    GaiaError, GaiaValidateResult, GaiaVgateResult, Geetest, GetBiliError, GetPlayurlError,
    INFO_HOST, MIXIN_KEY_ENC_TAB, NAV_HOST, NavResult, PLAYURL_HOST, Page, PlayurlInfo,
    PlayurlResult, QN, VgateData, WbiImg,
};

static BUVID: OnceLock<(String, String)> = OnceLock::new();

async fn get_buvid(client: &Client) -> Option<(String, String)> {
    match BUVID.get() {
        Some((b3, b4)) => Some((b3.clone(), b4.clone())),
        None => {
            let response = client.get(FINGER_HOST).send().await.ok()?;
            let res: FingerResult = response.json().await.ok()?;
            let b3 = res.data.b_3;
            let b4 = res.data.b_4;
            BUVID.set((b3.clone(), b4.clone())).unwrap();
            Some((b3, b4))
        }
    }
}

async fn get_mixin_key(client: &Client) -> wreq::Result<String> {
    let response = client.get(NAV_HOST).send().await?;
    let res: NavResult = response.json().await?;
    let WbiImg { img_url, sub_url } = res.data.wbi_img;
    let img_key = img_url.rsplit('/').next().unwrap();
    let img_key = img_key.split('.').next().unwrap();
    let sub_key = sub_url.rsplit('/').next().unwrap();
    let sub_key = sub_key.split('.').next().unwrap();

    let orig: Vec<char> = img_key.chars().chain(sub_key.chars()).collect();
    let mixin_key: String = MIXIN_KEY_ENC_TAB.iter().fold(String::new(), |mut acc, i| {
        acc.push(*orig.get(*i as usize).unwrap());
        acc
    });
    Ok(mixin_key.chars().take(32).collect())
}

pub fn wbi(query: &mut Vec<(&'_ str, String)>, mixin_key: &str) {
    let now = Timestamp::now();
    let now = now.as_second().to_string();
    query.push(("wts", now));

    let mut q = query.clone();
    q.sort_by_key(|x| x.0);
    let iter = q.into_iter().map(|(k, mut v)| {
        v.retain(|c| !matches!(c, '!' | '\'' | '(' | ')' | '*'));
        (k, v)
    });
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(iter)
        .finish();
    let w_rid = format!("{:x}", md5::compute(encoded + mixin_key));

    query.push(("w_rid", w_rid));
}

pub async fn get_bili(
    client: &Client,
    aid: u64,
    bvid: &str,
    grisk_id: Option<String>,
) -> Result<BiliInfo, GetBiliError> {
    let cache_dir = Path::new("cache/bilis");
    if let Err(e) = fs::create_dir_all(cache_dir).await {
        log::error!("缓存文件夹创建失败: {e:?}");
    }

    let cache_file = cache_dir.join(&format!("{bvid}.json"));
    let res: BiliResult = if let Ok(text) = fs::read_to_string(&cache_file).await {
        log::info!("使用缓存: {}", cache_file.display());
        serde_json::from_str(&text)?
    } else {
        let mixin_key = get_mixin_key(&client).await?;
        let mut query = vec![
            ("aid", aid.to_string()),
            ("need_view", String::from("1")),
            ("isGaiaAvoided", String::from("false")),
            ("web_location", String::from("1315873")),
        ];
        if let Some(ref g) = grisk_id {
            query.push(("gaia_vtoken", g.to_string()));
        }
        wbi(&mut query, &mixin_key);

        let buvid = get_buvid(client).await;
        let cookie: String = {
            let mut c = Vec::new();
            if let Some((b3, b4)) = buvid {
                c.push(("buvid3", b3));
                c.push(("buvid4", b4));
            }
            if let Some(s) = super::SESSDATA.get().unwrap() {
                c.push(("SESSDATA", s.to_string()));
            }
            if let Some(g) = grisk_id {
                c.push(("x-bili-gaia-vtoken", g.to_string()));
            }
            let c: Vec<String> = c.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
            c.join("; ")
        };

        let referer = format!("https://www.bilibili.com/video/{}/", bvid);
        let response = client
            .get(INFO_HOST)
            .query(&query)
            .header("cookie", cookie)
            .header("referer", referer)
            .send()
            .await?;
        let status = response.status();
        if status != StatusCode::OK {
            return Err(GetBiliError::Status(status.as_u16()));
        }

        let header_voucher = response.headers().get("x-bili-gaia-vvoucher").cloned();
        let res: Value = response.json().await?;
        match res["code"].as_i64().unwrap() {
            -404 | 62002 | 62004 | 0 | -352 => {
                if let Some(v_voucher) = header_voucher {
                    return Err(GetBiliError::Voucher(
                        v_voucher.to_str().unwrap().to_string(),
                    ));
                } else if let Some(v_voucher) = res["data"]["v_voucher"].as_str() {
                    return Err(GetBiliError::Voucher(v_voucher.to_string()));
                } else {
                    let pretty = serde_json::to_string_pretty(&res)?;
                    if let Err(e) = fs::write(&cache_file, &pretty).await {
                        log::error!("缓存json文件失败: {e:?}");
                    } else {
                        log::info!("写入缓存: {}", cache_file.display());
                    }
                }
            }
            _ => {}
        }

        serde_json::from_value(res)?
    };

    match res.code {
        -404 | 62002 | 62004 => {
            log::info!("{}", res.message);
            return Err(GetBiliError::NotFound);
        }
        0 | -352 => {}
        412 => {
            return Err(GetBiliError::RiskControl);
        }
        _ => {
            let msg = format!("未知状态码: {} {}", res.code, res.message);
            log::error!("{msg}");
            return Err(GetBiliError::Api(msg));
        }
    }

    match res.data.view {
        Some(view) => Ok(view),
        None => {
            return Err(GetBiliError::RiskControl);
        }
    }
}

pub fn parse_msg(info: BiliInfo, p: u16) -> Result<(String, Page, String), ()> {
    let bvid = info.bvid;
    let p_url = if p > 1 {
        format!("?p={p}")
    } else {
        String::new()
    };
    let p_tip = if p > 1 {
        format!(" P{p}")
    } else {
        String::new()
    };

    let mut pa = None;
    for page in info.pages {
        if page.page == p {
            pa = Some(page);
            break;
        }
    }
    let pa = pa.ok_or(())?;

    let title = info
        .title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let uid = info.owner.mid;
    let nickname = info.owner.name;
    let mut desc = parse_desc(info.desc_v2.unwrap_or_default());
    if &desc == "-" {
        desc.clear();
    }
    if !desc.is_empty() {
        desc = format!(":\n<blockquote expandable>{desc}</blockquote>");
    }

    let msg = format!(
        "<a href=\"https://www.bilibili.com/video/{bvid}{p_url}\">{title}{p_tip}</a> | \
         <a href=\"https://space.bilibili.com/{uid}\">{nickname}</a> #Bilibili{desc}"
    );
    Ok((msg, pa, info.pic))
}

fn parse_desc(desc: Vec<DescItem>) -> String {
    if desc.is_empty() {
        return String::new();
    }
    desc.into_iter()
        .map(|d| {
            if d.r#type == 2 {
                format!(
                    "<a href=\"https://space.bilibili.com/{}\">{}</a>",
                    d.biz_id, d.raw_text
                )
            } else {
                crate::utils::safe_truncate(&d.raw_text, 900)
            }
        })
        .collect::<Vec<String>>()
        .join("")
}

pub async fn get_playurl(
    client: &Client,
    aid: u64,
    bvid: &str,
    cid: i64,
    grisk_id: Option<String>,
) -> Result<PlayurlInfo, GetPlayurlError> {
    let cache_dir = Path::new("cache/bilis");
    if let Err(e) = fs::create_dir_all(cache_dir).await {
        log::error!("缓存文件夹创建失败: {e:?}");
    }

    let cache_file = cache_dir.join(&format!("{bvid}_playurl.json"));

    let mixin_key = get_mixin_key(&client).await?;

    let mut query = vec![
        ("avid", aid.to_string()),
        ("bvid", bvid.to_string()),
        ("cid", cid.to_string()),
        ("qn", QN.to_string()),
        ("fnver", "0".to_string()),
        ("fnval", "4048".to_string()),
        ("fourk", "1".to_string()),
        ("gaia_source", "".to_string()),
        ("from_client", "BROWSER".to_string()),
        ("is_main_page", "true".to_string()),
        ("need_fragment", "false".to_string()),
        ("isGaiaAvoided", "false".to_string()),
        ("client_attr", "0".to_string()),
        ("version_name", "4.9.96-rc.5539.0".to_string()),
        ("app_id", "100".to_string()),
        ("voice_balance", "1".to_string()),
        ("try_look", "1".to_string()),
        ("web_location", "1315873".to_string()),
    ];
    let buvid = get_buvid(client).await;
    if let Some((ref b3, _)) = buvid {
        let now = jiff::Timestamp::now();
        let session = format!("{}{}", b3, now.as_millisecond());
        let session = format!("{:x}", md5::compute(session));
        query.push(("session", session));
    }
    if let Some(ref g) = grisk_id {
        query.push(("gaia_vtoken", g.to_string()));
    }
    wbi(&mut query, &mixin_key);

    let cookie: String = {
        let mut c = Vec::new();
        if let Some((b3, b4)) = buvid {
            c.push(("buvid3", b3));
            c.push(("buvid4", b4));
        }
        if let Some(s) = super::SESSDATA.get().unwrap() {
            c.push(("SESSDATA", s.to_string()));
        }
        if let Some(g) = grisk_id {
            c.push(("x-bili-gaia-vtoken", g.to_string()));
        }
        let c: Vec<String> = c.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
        c.join("; ")
    };

    let referer = format!("https://www.bilibili.com/video/{}/", bvid);
    let request = client
        .get(PLAYURL_HOST)
        .query(&query)
        .header("cookie", cookie)
        .header("referer", referer);

    let response = request.send().await?;
    let status = response.status();
    if status != StatusCode::OK {
        return Err(GetPlayurlError::Status(status.as_u16()));
    }

    let res: Value = response.json().await?;

    let pretty = serde_json::to_string_pretty(&res)?;
    if let Err(e) = fs::write(&cache_file, &pretty).await {
        log::error!("缓存json文件失败: {e:?}");
    } else {
        log::info!("写入缓存: {}", cache_file.display());
    }
    let res: PlayurlResult = serde_json::from_value(res)?;

    if let Some(v_voucher) = res.data.v_voucher {
        return Err(GetPlayurlError::Voucher(v_voucher));
    }
    Ok(res.data)
}

pub async fn get_gaia(
    wreq_client: &wreq::Client,
    v_voucher: String,
) -> Result<(String, String, String), GaiaError> {
    let mut form_data = std::collections::HashMap::new();
    form_data.insert("v_voucher", v_voucher);

    let response = wreq_client
        .post(GAIA_VGATE_HOST)
        .form(&form_data)
        .send()
        .await?;
    let status = response.status();
    if status != StatusCode::OK {
        return Err(GaiaError::Status(status.into()));
    }

    let res: GaiaVgateResult = response.json().await?;
    let VgateData {
        r#type,
        token,
        geetest,
    } = res.data;
    match r#type.as_str() {
        "geetest" => {
            let Geetest { challenge, gt } = geetest;
            Ok((token, challenge, gt))
        }
        _ => Err(GaiaError::Unsupported),
    }
}

pub async fn validate_gaia(
    wreq_client: &wreq::Client,
    token: String,
    challenge: String,
    validate: String,
    seccode: String,
) -> Result<String, GaiaError> {
    let mut form_data = std::collections::HashMap::new();
    form_data.insert("token", token);
    form_data.insert("challenge", challenge);
    form_data.insert("validate", validate);
    form_data.insert("seccode", seccode);

    let response = wreq_client
        .post(GAIA_VALIDATE_HOST)
        .form(&form_data)
        .send()
        .await?;
    let status = response.status();
    if status != StatusCode::OK {
        return Err(GaiaError::Status(status.into()));
    }

    let res: GaiaValidateResult = response.json().await?;
    if res.data.is_valid != 1 {
        return Err(GaiaError::ValidateFailed);
    }
    Ok(res.data.grisk_id)
}
