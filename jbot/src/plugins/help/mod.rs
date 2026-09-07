use std::sync::Arc;

use grammers_client::Client;
use grammers_client::message::{InputMessage, Message};
use grammers_session::types::PeerKind;

const HELP: &str = "<b>Hi! 这里是小派魔6号姬!</b>
小派魔正在用 Rust 重构中，有任何问题欢迎前往 <a href=\"https://t.me/HBcaoHome\">🍀派魔喵の家🍥</a> 反馈喵！

指令列表:
◆ 常规
● /ping
\u{3000}查看小派魔是否存活
● /status
\u{3000}查看小派魔运行状态
● /roll
\u{3000}发动吧命运之骰
● /help
\u{3000}显示此帮助

◆ 爬虫
◆ 发送链接自动解析可爬取内容
● 支持 Twitter 等站点

对小派魔有任何建议或意见欢迎前往 <a href=\"https://t.me/HBcaoHome\">🍀派魔喵の家🍥</a> 私聊或评论喵！";

#[crate::on_new_message]
async fn handler(_client: Client, message: Arc<Message>) {
    if message.outgoing() {
        return;
    }
    if message.peer_id().kind() != PeerKind::User {
        return;
    }

    let text = message.text();
    if text.starts_with("/start") || text.starts_with("/help") {
        if let Err(e) = message.reply(InputMessage::new().html(HELP)).await {
            log::error!("消息发送失败: {e}")
        }
    }
}
