use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use grammers_client::client::Client;
use grammers_client::message::Message;
use grammers_session::types::PeerId;
use tokio::time::{Duration, sleep};

type PeerGroupedMedias = BTreeMap<i64, BTreeMap<i64, GroupedMedias>>;
static PEER_GROUPED_MEDIAS: OnceLock<Mutex<PeerGroupedMedias>> = OnceLock::new();

fn peer_grouped_medias() -> &'static Mutex<PeerGroupedMedias> {
    PEER_GROUPED_MEDIAS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

struct GroupedMedias {
    client: Client,
    ended: Arc<AtomicBool>,
    task: Option<tokio::task::JoinHandle<()>>,
    messages: Vec<Arc<Message>>,
}

impl GroupedMedias {
    pub fn new(client: Client, message: Arc<Message>) -> Self {
        let mut messages = Vec::with_capacity(10);
        messages.push(message);
        Self {
            client,
            ended: Arc::new(AtomicBool::new(false)),
            task: None,
            messages,
        }
    }

    pub fn push(&mut self, message: Arc<Message>) {
        if self.ended.load(Ordering::SeqCst) {
            log::error!("GroupedMedias 在任务执行完后再次被 push。");
        }
        self.messages.push(message);
        if let Some(task) = &self.task {
            task.abort();
        }

        let ended = self.ended.clone();
        let client = self.client.clone();
        let messages = self.messages.clone();

        self.task = Some(tokio::spawn(async move {
            sleep(Duration::from_secs(3)).await;

            ended.store(true, Ordering::SeqCst);
            for handler in crate::GROUPED_MESSAGES_HANDLERS {
                handler(client.clone(), messages.clone()).await;
            }
        }));
    }
}

pub(super) fn get_or_insert(
    client: Client,
    peer_id: PeerId,
    grouped_id: i64,
    message: Arc<Message>,
) {
    let mut guard = peer_grouped_medias().lock().unwrap();
    let map = guard
        .entry(peer_id.bot_api_dialog_id().unwrap())
        .or_insert_with(BTreeMap::new);
    match map.get_mut(&grouped_id) {
        Some(group) => {
            group.push(message);
        }
        None => {
            map.insert(grouped_id, GroupedMedias::new(client, message));
        }
    }
}
