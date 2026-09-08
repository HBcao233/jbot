use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use grammers_client::client::Client;
use grammers_client::message::Message;
use grammers_session::types::PeerId;
use tokio::time::{Duration, sleep};

const GROUPED_MILLIS: u64 = 500;

// peer_id: GroupedMedias
type PeerGroupedMedias = BTreeMap<i64, GroupedMedias>;
static PEER_GROUPED_MEDIAS: OnceLock<Mutex<PeerGroupedMedias>> = OnceLock::new();

fn peer_grouped_medias() -> &'static Mutex<PeerGroupedMedias> {
    PEER_GROUPED_MEDIAS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

struct GroupedMedias {
    client: Client,
    ended: Arc<AtomicBool>,
    task: Option<tokio::task::JoinHandle<()>>,
    messages: Vec<Arc<Message>>,
    peer_id: i64,
}

impl GroupedMedias {
    pub fn new(client: Client, message: Arc<Message>, peer_id: i64) -> Self {
        let mut messages = Vec::with_capacity(10);
        messages.push(message);
        let mut group = Self {
            client,
            ended: Arc::new(AtomicBool::new(false)),
            task: None,
            messages,
            peer_id,
        };
        group.schedule_task();
        group
    }

    pub fn push(&mut self, message: Arc<Message>) {
        if self.ended.load(Ordering::SeqCst) {
            log::error!("GroupedMedias 在任务执行完后再次被 push。");
            return;
        }

        self.messages.push(message);
        if let Some(task) = &self.task {
            task.abort();
        }
        if self.messages.len() >= 10 {
            self.ended.store(true, Ordering::SeqCst);
        }

        self.schedule_task();
    }

    fn schedule_task(&mut self) {
        let ended = self.ended.clone();
        let client = self.client.clone();
        let messages = self.messages.clone();
        let peer_id = self.peer_id.clone();

        self.task = Some(tokio::spawn(async move {
            if messages.len() >= 10 {
                sleep(Duration::from_millis(GROUPED_MILLIS)).await;

                ended.store(true, Ordering::SeqCst);
            }

            for handler in crate::GROUPED_MESSAGES_HANDLERS {
                handler(client.clone(), messages.clone()).await;
            }

            let mut guard = peer_grouped_medias().lock().unwrap();
            if let Some(current_group) = guard.get(&peer_id) {
                if Arc::ptr_eq(&current_group.ended, &ended) {
                    guard.remove(&peer_id);
                }
            }
        }));
    }
}

pub(super) fn get_or_insert(client: Client, peer_id: PeerId, message: Arc<Message>) {
    let peer_id = peer_id.bot_api_dialog_id().unwrap();
    let mut guard = peer_grouped_medias().lock().unwrap();

    if let Some(group) = guard.get_mut(&peer_id) {
        if !group.ended.load(Ordering::SeqCst) {
            group.push(message);
            return;
        }
    }

    let group = GroupedMedias::new(client, message, peer_id);
    guard.insert(peer_id, group);
}
