extern crate jbot_macro;
mod button;
mod curl;
pub mod database;
mod grouped;
mod plugins;
mod utils;

use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};

use curl::get_client;
use grammers_client::Client;
use grammers_client::message::Message;
use grammers_client::peer::Peer;
use grammers_client::sender::{SenderPool, UpdatesConfiguration};
use grammers_client::update::Update;
use grammers_session::storages::SqliteSession;
use grammers_session::types::PeerId;
pub use jbot_macro::{on_grouped_messages, on_new_message, on_setup, on_update};
use log::LevelFilter;
use simple_logger::SimpleLogger;
use tokio::runtime;
use tokio::task::JoinSet;

const SYNC_REST_SECONDS: u64 = 60;
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
                    let sender_info = get_peer_info(&sender_id, message.sender());
                    let text = safe_truncate(message.text(), 30);

                    let peer_info = if sender_id != peer_id {
                        let peer_info = get_peer_info(&peer_id, message.peer());
                        &format!(" in {peer_info}")
                    } else {
                        ""
                    };
                    log::info!("{sender_info}{}: {text}", peer_info);
                }
            }

            let msg = Arc::new(message.into_inner());
            for handler in NEW_MESSAGE_HANDLERS {
                handler(client.clone(), Arc::clone(&msg)).await;
            }

            if let Some(grouped_id) = msg.grouped_id() {
                grouped::get_or_insert(client.clone(), peer_id, grouped_id, msg);
            }
        }
        _ => {}
    }
}

fn safe_truncate(text: &str, num: usize) -> String {
    if text.chars().count() <= num {
        text.to_string()
    } else {
        let text: String = text.chars().take(num).collect();
        format!("{}...", text)
    }
}

fn peer_full_name(peer: &Peer) -> Option<String> {
    match peer {
        Peer::User(user) => Some(user.full_name()),
        Peer::Group(group) => group.title().map(str::to_string),
        Peer::Channel(channel) => Some(channel.title().to_string()),
        Peer::Community(community) => Some(community.title().to_string()),
    }
}

fn get_peer_info(peer_id: &PeerId, peer: Option<&Peer>) -> String {
    let name = peer
        .and_then(peer_full_name)
        .unwrap_or("Unknown".to_string());
    let username = peer.and_then(|p| p.username());
    let username = match username {
        Some(x) => &format!(" <@{x}>"),
        None => "",
    };
    format!("{name}({peer_id}{username})")
}

async fn async_main() -> anyhow::Result<()> {
    SimpleLogger::new()
        .with_level(LevelFilter::Info)
        .init()
        .unwrap();

    dotenvy::dotenv()?;

    for setup in SETUPS {
        setup()?;
    }

    let api_id = env::var("TG_ID")?.parse().expect("TG_ID invalid");
    let token = env::var("TOKEN").expect("token missing");

    let session = Arc::new(SqliteSession::open(SESSION_FILE).await?);

    let SenderPool {
        runner,
        updates,
        handle,
    } = SenderPool::new(Arc::clone(&session), api_id);
    let client = Client::new(handle.clone());
    let pool_task = tokio::spawn(runner.run());

    if !client.is_authorized().await? {
        log::info!("Signing in...");
        client.bot_sign_in(&token, &env::var("TG_HASH")?).await?;
        log::info!("Signed in!");
    }

    log::info!("Waiting for messages...");

    // This example spawns a task to handle each update.
    // To guarantee that all handlers run to completion, they're stored in this set.
    // You can use `task::spawn` if you don't care about dropping unfinished handlers midway.
    let mut handler_tasks = JoinSet::new();
    let mut updates = client
        .stream_updates(updates, UpdatesConfiguration { catch_up: true })
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    let mut now = Instant::now();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            Some(res) = handler_tasks.join_next(), if !handler_tasks.is_empty() => {
                if let Err(e) = res {
                    panic!("handler task panicked: {e}");
                } else if handler_tasks.is_empty() && now.elapsed() >= Duration::from_secs(SYNC_REST_SECONDS) {
                    now = Instant::now();
                    log::info!("Saving session when idle...");
                    updates
                        .sync_update_state()
                        .await
                        .map_err(|e| anyhow::anyhow!(e))?;
                }
            }
            update = updates.next() => {
                let update = update?;
                let handle = client.clone();
                handler_tasks.spawn(handle_update(handle, update));
            }
        }
    }

    log::info!("Saving session file...");
    updates
        .sync_update_state()
        .await
        .map_err(|e| anyhow::anyhow!(e))?; // you usually want this before closing the session

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
    log::info!("Waiting for any slow handlers to finish...");
    while let Some(_) = handler_tasks.join_next().await {}

    Ok(())
}

fn main() -> anyhow::Result<()> {
    runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async_main())
}
