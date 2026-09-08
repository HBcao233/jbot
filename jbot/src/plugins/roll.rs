use std::sync::Arc;

use grammers_client::Client;
use grammers_client::message::{InputMessage, Message};
use grammers_session::types::PeerKind;
use rand::random_range;

const HELP: &str = "用法: /roll [最小点数 最大点数]
最小点数最小值: -32768
最大点数最大值: 32767";

#[crate::on_new_message]
async fn handler(_client: Client, message: Arc<Message>) {
    if message.outgoing() {
        return;
    }

    if message.peer_id().kind() != PeerKind::User {
        return;
    }

    let text = message.text();
    if text.starts_with("/roll") {
        let text = text
            .strip_prefix("/roll")
            .unwrap()
            .trim()
            .replace('~', "")
            .replace('〜', "");
        let arr: Vec<_> = text.split(' ').filter(|x| !x.is_empty()).collect();
        let mut min = 1;
        let mut max = 6;
        if !arr.is_empty() {
            if arr.len() < 2 {
                if let Err(e) = message.reply(InputMessage::new().text(HELP)).await {
                    log::error!("消息发送失败: {e}")
                }
                return;
            }

            match arr[0].parse() {
                Ok(m) => {
                    min = m;
                }
                Err(_) => {
                    if let Err(e) = message.reply(InputMessage::new().text(HELP)).await {
                        log::error!("消息发送失败: {e}")
                    }
                    return;
                }
            }
            match arr[1].parse() {
                Ok(m) => {
                    max = m;
                }
                Err(_) => {
                    if let Err(e) = message.reply(InputMessage::new().text(HELP)).await {
                        log::error!("消息发送失败: {e}")
                    }
                    return;
                }
            }
        }

        if min > max {
            std::mem::swap(&mut min, &mut max);
        }

        let res: i16 = random_range(min..=max);
        let text = format!("🎲 骰到了 {res} ({min} ~ {max})");
        if let Err(e) = message.reply(text).await {
            log::error!("消息发送失败: {e}")
        }
    }
}
