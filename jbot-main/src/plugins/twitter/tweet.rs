use serde::Deserialize;

#[derive(Debug)]
pub struct Tweet {
    id: String,
    // views: i32,
    created_at: String,
    full_text: String,
    display_text_range: (usize, usize),
    entities: Entities,
    // pub lang: String,
    // pub possibly_sensitive: bool,
    // pub bookmark_count: i32,
    // pub favorite_count: i32,
    // pub quote_count: i32,
    // pub reply_count: i32,
    // pub retweet_count: i32,
    author: User,
}

impl Tweet {
    pub fn id(&self) -> &str {
        &self.id
    }

    /*pub fn views(&self) -> i32 {
        self.views
    }*/

    pub fn created_at(&self) -> &str {
        &self.created_at
    }

    pub fn full_text(&self) -> &str {
        &self.full_text
    }

    pub fn display_text(&self) -> String {
        let full_text = self.full_text();
        let (start, end) = self.display_text_range;
        full_text.chars().skip(start).take(end - start).collect()
    }

    pub fn entities(&self) -> &Entities {
        &self.entities
    }

    pub fn author(&self) -> &User {
        &self.author
    }
}

#[derive(Debug)]
pub struct User {
    // avatar_url: String,
    name: String,
    screen_name: String,
    // description: String,
    // entities: UserLegacyEntities,
    // followers_count: i32,
    // normal_followers_count: i32,
    // fast_followers_count: i32,
    // friends_count: i32,
    // statuses_count: i32,
    // media_count: i32,
    // favourites_count: i32,
    // listed_count: i32,
}

impl User {
    /*pub fn avatar_url(&self) -> &str {
        &self.avatar_url
    }*/

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn username(&self) -> &str {
        &self.screen_name
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GetTweetError {
    #[error("HTTP 请求失败: {0}")]
    Http(#[from] wreq::Error),

    #[error("状态码错误: {0}")]
    Status(u16),

    #[error("JSON 解析失败: {0}")]
    Json(#[from] serde_json::Error),

    #[error("推文不存在")]
    NotFound,

    #[error("API 错误: {0}")]
    Api(String),

    #[error("JSON 解析失败, 可能是老马又修改了接口: {0}")]
    ParseFailed(String),
}

#[derive(Deserialize)]
pub(super) struct TweetRaw {
    // article:
    core: Core,
    legacy: Legacy,
    // views: Views,
    pub(super) tombstone: Option<Tombstone>,
}

/*
#[derive(Deserialize)]
struct Views {
    count: String,
}
*/

#[derive(Deserialize)]
struct Legacy {
    id_str: String,
    // user_id_str: String,
    created_at: String,
    full_text: String,
    display_text_range: (usize, usize),
    entities: Entities,
    // extended_entities: ExtendedEntities,
    // lang: String,
    // possibly_sensitive: bool,
    // bookmark_count: i32,
    // favorite_count: i32,
    // quote_count: i32,
    // reply_count: i32,
    // retweet_count: i32,
}

#[derive(Debug, Deserialize)]
pub struct Entities {
    pub urls: Option<Vec<UrlEntity>>,
    pub media: Option<Vec<MediaEntity>>,
    // pub user_mentions: Option<Vec<UserMentionEntity>>,
}

#[derive(Debug, Deserialize)]
pub struct UrlEntity {
    // 文本范围
    // pub indices: (usize, usize),
    // t.co 短链
    pub url: String,
    // 实际链接
    pub expanded_url: String,
    // 显示链接
    // pub display_url: String,
}

#[derive(Debug, Deserialize)]
pub struct MediaEntity {
    // 文本范围
    // pub indices: (usize, usize),
    // t.co 短链
    // pub url: String,
    // 实际链接
    // pub expanded_url: String,
    // 显示链接
    // pub display_url: String,
    /// 媒体类型: photo, video
    pub r#type: String,
    // pub media_key: String,
    pub media_url_https: String,
    pub original_info: OriginalInfo,
    pub video_info: Option<VideoInfo>,
}

#[derive(Debug, Deserialize)]
pub struct OriginalInfo {
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Deserialize)]
pub struct VideoInfo {
    pub duration_millis: f64,
    pub variants: Vec<VideoVariant>,
}

#[derive(Debug, Deserialize)]
pub struct VideoVariant {
    pub content_type: String,
    pub url: String,
    pub bitrate: Option<i32>,
}

/*
#[derive(Debug, Deserialize)]
pub struct UserMentionEntity {
    // 文本范围
    pub indices: (usize, usize),
    pub name: String,
    pub screen_name: String,
}
*/

#[derive(Deserialize)]
struct Core {
    user_results: UsersResults,
}

#[derive(Deserialize)]
struct UsersResults {
    result: UserRaw,
}

#[derive(Deserialize)]
struct UserRaw {
    // avatar: Avatar,
    core: UserCore,
    // legacy: UserLegacy,
}

/*
#[derive(Deserialize)]
struct Avatar {
    image_url: String,
}
*/

#[derive(Deserialize)]
struct UserCore {
    // created_at: String,
    name: String,
    screen_name: String,
}

/*
#[derive(Deserialize)]
struct UserLegacy {
    description: String,
    entities: UserLegacyEntities,
    // followers_count: i32,
    // normal_followers_count: i32,
    // fast_followers_count: i32,
    // friends_count: i32,
    // statuses_count: i32,
    // media_count: i32,
    // favourites_count: i32,
    // listed_count: i32,
}
#[derive(Debug, Deserialize)]
pub struct UserLegacyEntities {
    // description: Entities,
}
*/

#[derive(Deserialize)]
pub(super) struct Tombstone {
    pub(super) text: TombstoneInner,
}

#[derive(Deserialize)]
pub(super) struct TombstoneInner {
    pub(super) text: String,
}

impl From<TweetRaw> for Tweet {
    fn from(x: TweetRaw) -> Self {
        let Legacy {
            id_str,
            created_at,
            full_text,
            display_text_range,
            entities,
        } = x.legacy;
        let user = x.core.user_results.result;
        Self {
            id: id_str,
            created_at,
            full_text,
            display_text_range,
            entities,
            author: user.into(),
        }
    }
}

impl From<UserRaw> for User {
    fn from(x: UserRaw) -> Self {
        let UserCore { name, screen_name } = x.core;
        Self { name, screen_name }
    }
}
