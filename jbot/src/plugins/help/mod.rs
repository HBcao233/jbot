use std::path::Path;
use std::sync::Arc;
use std::thread::sleep;

use grammers_client::Client;
use grammers_client::message::{InputMessage, Message};
use grammers_session::types::PeerKind;
use sysinfo::{Disks, System};

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
    } else if text.starts_with("/ping") {
        if let Err(e) = message.reply("小派魔存活中").await {
            log::error!("消息发送失败: {e}")
        }
    } else if text.starts_with("/status") {
        let text = get_server_status().await;
        if let Err(e) = message.reply(InputMessage::new().html(text)).await {
            log::error!("消息发送失败: {e}")
        }
    }
}

pub async fn get_server_status() -> String {
    tokio::task::spawn_blocking(server_status).await.unwrap()
}

fn server_status() -> String {
    let mut sys = System::new_all();

    // 获取CPU使用率
    sys.refresh_cpu_usage();
    sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_cpu_usage();
    let cpu_percent = sys.global_cpu_usage();

    // 获取内存使用情况
    sys.refresh_memory();
    let mem_used = sys.used_memory() as f64 / (1024.0 * 1024.0);
    let mem_total = sys.total_memory() as f64 / (1024.0 * 1024.0);
    let mem_percent = mem_used / mem_total * 100.0;

    // 获取磁盘使用情况（根目录）
    let disks = Disks::new_with_refreshed_list();
    let (disk_used, disk_total, disk_percent) = disks
        .iter()
        .find(|d| d.mount_point() == Path::new("/"))
        .map(|d| {
            let total = d.total_space() as f64 / (1024.0 * 1024.0);
            let used = (d.total_space() - d.available_space()) as f64 / (1024.0 * 1024.0);
            (used, total, used / total * 100.0)
        })
        .unwrap_or((0.0, 0.0, 0.0));

    // 获取系统运行时长
    let uptime_seconds = System::uptime();
    let hours = uptime_seconds / 3600;
    let minutes = (uptime_seconds % 3600) / 60;
    let seconds = uptime_seconds % 60;
    let uptime = format!("{:02}小时{:02}分钟{:02}秒", hours, minutes, seconds);

    format!(
        "<b>服务器状态</b>:\n\
         ● <b>CPU使用率</b>: {:.1}%\n\
         ● <b>内存使用</b>: {:.2}MB/{:.2}MB ({:.1}%)\n\
         ● <b>磁盘使用</b>: {:.2}MB/{:.2}MB ({:.1}%)\n\
         ● <b>运行时长</b>: {}\n",
        cpu_percent, mem_used, mem_total, mem_percent, disk_used, disk_total, disk_percent, uptime,
    )
}
