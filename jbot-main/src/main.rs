extern crate jbot_macro;
mod button;
pub mod curl;
pub mod database;
mod grouped;
mod plugins;
mod utils;

use std::env;
use std::sync::Arc;
use std::time::Duration;

use grammers_client::Client;
use grammers_client::message::Message;
use grammers_client::sender::{SenderPool, UpdatesConfiguration};
use grammers_client::update::Update;
use grammers_session::storages::SqliteSession;
pub use jbot_macro::{on_grouped_messages, on_new_message, on_setup, on_update};
use log::LevelFilter;
use simple_logger::SimpleLogger;
use tokio::runtime;
use tokio::task::JoinSet;
use tokio::time::interval;

const SYNC_INTERVAL: Duration = Duration::from_secs(60);
const SESSION_FILE: &str = "jbot.session";

type HandlerResult = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;
type Handler = fn(client: Client, update: Update) -> HandlerResult;
type NewMessageHandler = fn(client: Client, message: Arc<Message>) -> HandlerResult;
type GroupedMessagesHandler = fn(client: Client, message: Vec<Arc<Message>>) -> HandlerResult;

#[linkme::distributed_slice]
pub static SETUPS: [fn() -> anyhow::Result<()>];

#[linkme::distributed_slice]
pub static HANDLERS: [Handler];

#[linkme::distributed_slice]
pub static NEW_MESSAGE_HANDLERS: [NewMessageHandler];

#[linkme::distributed_slice]
pub static GROUPED_MESSAGES_HANDLERS: [GroupedMessagesHandler];

async fn handle_update(client: Client, update: Update) {
    for handler in HANDLERS {
        handler(client.clone(), update.clone()).await;
    }

    match update {
        Update::NewMessage(message) => {
            let peer_id = message.peer_id();
            if !message.outgoing() {
                if let Some(sender_id) = message.sender_id() {
                    let sender_info = crate::utils::get_peer_info(&sender_id, message.sender());
                    let text = crate::utils::safe_truncate(message.text(), 30);

                    let peer_info = if sender_id != peer_id {
                        let peer_info = crate::utils::get_peer_info(&peer_id, message.peer());
                        &format!(" in {peer_info}")
                    } else {
                        ""
                    };
                    log::info!("{sender_info}{}: {text}", peer_info);
                }
            }

            let message = Arc::new(message.into_inner());
            for handler in NEW_MESSAGE_HANDLERS {
                handler(client.clone(), Arc::clone(&message)).await;
            }

            if let Some(media) = message.media() {
                if crate::utils::can_grouped(&media) {
                    grouped::get_or_insert(client.clone(), peer_id, message);
                }
            }
        }
        _ => {}
    }
}

async fn async_main() {
    SimpleLogger::new()
        .with_level(LevelFilter::Info)
        .init()
        .unwrap();

    dotenvy::dotenv().unwrap();

    for setup in SETUPS {
        if let Err(e) = setup() {
            log::error!("初始化失败: {e}");
            return;
        }
    }

    let api_id = env::var("TG_ID")
        .unwrap_or_default()
        .parse()
        .expect("TG_ID invalid");
    let api_hash = env::var("TG_HASH").unwrap();
    let token = env::var("TOKEN").expect("token missing");

    let session = Arc::new(SqliteSession::open(SESSION_FILE).await.unwrap());

    let SenderPool {
        runner,
        updates,
        handle,
    } = SenderPool::new(Arc::clone(&session), api_id);
    let client = Client::new(handle.clone());
    let pool_task = tokio::spawn(runner.run());

    if !client.is_authorized().await.unwrap() {
        log::info!("Signing in...");
        client
            .bot_sign_in(&token, &api_hash)
            .await
            .expect("Sign in failed.");
        log::info!("Signed in!");
    }

    log::info!("Waiting for messages...");

    let mut handler_tasks = JoinSet::new();
    let mut updates = client
        .stream_updates(updates, UpdatesConfiguration { catch_up: true })
        .await
        .unwrap();
    let mut timer = interval(SYNC_INTERVAL);
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            result = updates.next() => {
                match result {
                    Ok(update) => {
                        let handle = client.clone();
                        handler_tasks.spawn(handle_update(handle, update));
                    }
                    Err(e) => {
                        log::error!("获取更新失败: {e}");
                    }
                }
            }
            Some(res) = handler_tasks.join_next(), if !handler_tasks.is_empty() => {
                if let Err(e) = res {
                    log::error!("handler task panicked: {e}");
                }
            }
            _ = timer.tick() => {
                if handler_tasks.is_empty() {
                    log::info!("Saving session periodically...");
                    if let Err(e) = updates
                        .sync_update_state()
                        .await {
                        log::error!("Sync update state failed: {e}");
                    }
                }
            }
        }
    }

    log::info!("Saving session file...");
    if let Err(e) = updates.sync_update_state().await {
        log::error!("Sync update state failed: {e}")
    }

    // Pool's `run()` won't finish until all handles are dropped or quit is called.
    // Here there are at least three handles alive: `handle`, `client` and `updates`
    // which contains a `client`. Any ongoing `handle_update` handlers have one client too.
    // In this case, it's easier to call `handle.quit()` to close them all.
    //
    // You don't need to explicitly close the connection, but this is a way to do it gracefully.
    // This also gives a chance to the handlers to finish their work by handling the `Dropped`
    // error from any pending method calls (RPC invocations).
    //
    // You can try this graceful shutdown by sending a message saying "slow" and then pressing Ctrl+C.
    log::info!("Gracefully closing connection to notify all pending handlers...");
    handle.quit();
    let _ = pool_task.await;

    // Give a chance to all on-going handlers to finish.
    // log::info!("Waiting for any slow handlers to finish...");
    // while let Some(_) = handler_tasks.join_next().await {}
}

fn main() {
    runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async_main());
}
