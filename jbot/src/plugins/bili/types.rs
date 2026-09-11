use serde::Deserialize;

use super::abv::{av2bv, bv2av};

pub const QN: u8 = 80;

pub(super) const NAV_HOST: &str = "https://api.bilibili.com/x/web-interface/nav";
pub(super) const INFO_HOST: &str = "https://api.bilibili.com/x/web-interface/wbi/view/detail";
pub(super) const PLAYURL_HOST: &str = "https://api.bilibili.com/x/player/wbi/playurl";
pub(super) const GAIA_VGATE_HOST: &str = "https://api.bilibili.com/x/gaia-vgate/v1/register";
pub(super) const GAIA_VALIDATE_HOST: &str = "https://api.bilibili.com/x/gaia-vgate/v1/validate";
pub(super) const FINGER_HOST: &str = "https://api.bilibili.com/x/frontend/finger/spi";

pub(super) const MIXIN_KEY_ENC_TAB: [u8; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19, 29,
    28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4, 22, 25,
    54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
];

#[derive(Deserialize)]
pub(super) struct FingerResult {
    pub(super) data: FingerData,
}

#[derive(Deserialize)]
pub(super) struct FingerData {
    pub(super) b_3: String,
    pub(super) b_4: String,
}

#[derive(Deserialize)]
pub(super) struct NavResult {
    pub(super) data: NavData,
}

#[derive(Deserialize)]
pub(super) struct NavData {
    pub(super) wbi_img: WbiImg,
}

#[derive(Deserialize)]
pub(super) struct WbiImg {
    pub(super) img_url: String,
    pub(super) sub_url: String,
}

#[derive(Clone)]
pub enum BiliId {
    AV(u64),
    BV(String),
}

impl BiliId {
    pub fn to_raw(&self) -> Result<(u64, String), ()> {
        match self {
            Self::AV(aid) => Ok((*aid, av2bv(*aid)?)),
            Self::BV(bvid) => Ok((bv2av(bvid)?, bvid.to_string())),
        }
    }
}

#[derive(Deserialize)]
pub(super) struct BiliResult {
    pub(super) code: i32,
    pub(super) data: BiliData,
    pub(super) message: String,
}

#[derive(Deserialize)]
pub(super) struct BiliData {
    #[serde(rename = "View")]
    pub(super) view: Option<BiliInfo>,
    // pub(super) v_voucher: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BiliInfo {
    // pub aid: i64,
    pub bvid: String,
    // pub cid: i64,
    // 创建日期
    // pub ctime: i64,
    // 发布日期
    // pub pubdate: i64,
    // 简介
    pub desc_v2: Option<Vec<DescItem>>,
    // pub dimension: Dimension,
    // 时长, 单位: 秒
    // pub duration: u32,
    // 作者
    pub owner: Owner,
    // 分P
    pub pages: Vec<Page>,
    // 封面
    pub pic: String,
    // 统计数据
    // pub stat: Stat,
    // 标题
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct DescItem {
    pub biz_id: i32,
    pub raw_text: String,
    pub r#type: u8,
}

#[derive(Debug, Deserialize)]
pub struct Dimension {
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Deserialize)]
pub struct Owner {
    // face: String,
    pub mid: u64,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct Page {
    pub cid: i64,
    // pub ctime: i64,
    pub dimension: Dimension,
    pub duration: u32,
    pub first_frame: String,
    pub page: u16,
}

/*
#[derive(Debug, Deserialize)]
pub struct Stat {
    coin: u32,
    danmaku: u32,
    dislike: u32,
    favorite: u32,
    like: u32,
    reply: u32,
    share: u32,
    view: u32,
}
*/

#[derive(Debug, thiserror::Error)]
pub enum GetBiliError {
    #[error("请求失败")]
    Http(#[from] wreq::Error),

    #[error("状态码错误: {0}")]
    Status(u16),

    #[error("JSON 解析失败: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Bili不存在")]
    NotFound,

    #[error("API 错误: {0}")]
    Api(String),

    #[error("触发哔哩哔哩安全风控策略，访问请求被拒绝。")]
    RiskControl,

    #[error("需要人机验证")]
    Voucher(String),
}

#[derive(Deserialize)]
pub(super) struct PlayurlResult {
    pub(super) data: PlayurlInfo,
}

#[derive(Debug, Deserialize)]
pub struct PlayurlInfo {
    pub v_voucher: Option<String>,
    pub dash: Option<DashInfo>,
    pub durl: Option<Vec<DurlInfo>>,
}

#[derive(Debug, Deserialize)]
pub struct DashInfo {
    // pub duration: u32,
    pub audio: Vec<DashMedia>,
    pub video: Vec<DashMedia>,
}

#[derive(Debug, Deserialize)]
pub struct DashMedia {
    pub id: i32,
    pub base_url: String,
    // pub backup_url: Vec<String>,
    // pub width: u16,
    // pub height: u16,
    pub mime_type: String,
    // pub bandwidth: u32,
    // pub codecid: i32,
    pub codecs: String,
}

#[derive(Debug, Deserialize)]
pub struct DurlInfo {
    pub url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum GetPlayurlError {
    #[error("请求失败: {0}")]
    Http(#[from] wreq::Error),

    #[error("状态码错误: {0}")]
    Status(u16),

    #[error("JSON 解析失败: {0}")]
    Json(#[from] serde_json::Error),

    #[error("需要人机验证")]
    Voucher(String),
}

#[derive(Debug, thiserror::Error)]
pub(super) enum GaiaError {
    #[error("请求失败: {0}")]
    Http(#[from] wreq::Error),

    #[error("状态码错误: {0}")]
    Status(u16),

    #[error("JSON 解析失败: {0}")]
    Json(#[from] serde_json::Error),

    #[error("不支持的人机验证类型")]
    Unsupported,

    #[error("验证失败")]
    ValidateFailed,
}

#[derive(Deserialize)]
pub(super) struct GaiaVgateResult {
    pub(super) data: VgateData,
}

#[derive(Deserialize)]
pub(super) struct VgateData {
    pub(super) r#type: String,
    pub(super) token: String,
    pub(super) geetest: Geetest,
}

#[derive(Deserialize)]
pub(super) struct Geetest {
    pub(super) challenge: String,
    pub(super) gt: String,
}

#[derive(Deserialize)]
pub(super) struct GaiaValidateResult {
    // pub(super) code: i32,
    pub(super) data: GaiaValidateData,
}

#[derive(Deserialize)]
pub(super) struct GaiaValidateData {
    pub(super) is_valid: i32,
    pub(super) grisk_id: String,
}
