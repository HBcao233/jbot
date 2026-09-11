use std::sync::{Arc, OnceLock};

use grammers_client::media::Media;
use grammers_client::message::Message;
use grammers_session::types::PeerId;
use grammers_tl_types as tl;
use redb::{Database, Error, ReadableDatabase, ReadableTable, TableDefinition, TableError};
use tokio::task::spawn_blocking;

static MERGE_DB: OnceLock<Arc<Database>> = OnceLock::new();

/// key: (peer_id, message_id)
/// value: (MediaType, id, access_hash, file_reference)
const MEDIAS: TableDefinition<(i64, i32), (u8, i64, i64, Vec<u8>)> = TableDefinition::new("medias");

/// key: peer_id, index
/// value: Vec<message_id>
const SESSIONS: TableDefinition<(i64, u8), Vec<i32>> = TableDefinition::new("sessions");

fn merge_db() -> Result<Arc<Database>, Error> {
    Ok(match MERGE_DB.get() {
        Some(db) => Arc::clone(&db),
        None => {
            if let Err(e) = std::fs::create_dir_all("data") {
                log::error!("数据文件夹创建失败: {e:?}");
            }
            let db = Arc::new(Database::create("data/merge.redb")?);
            MERGE_DB.set(Arc::clone(&db)).unwrap();
            db
        }
    })
}

#[repr(u8)]
#[derive(Debug, Copy, Clone)]
enum MediaType {
    Photo = 0,
    Video,
}

impl TryFrom<u8> for MediaType {
    type Error = ();

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            x if x == MediaType::Photo as u8 => Ok(MediaType::Photo),
            x if x == MediaType::Video as u8 => Ok(MediaType::Video),
            _ => Err(()),
        }
    }
}

pub async fn get_medias(
    peer_id: PeerId,
    message_ids: Vec<i32>,
) -> Result<Vec<Option<tl::enums::InputMedia>>, Error> {
    let peer_id = peer_id.bot_api_dialog_id().unwrap();
    spawn_blocking(move || {
        let db = merge_db()?;
        let read_txn = db.begin_read()?;
        let table = match read_txn.open_table(MEDIAS) {
            Ok(t) => t,
            Err(TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };

        let mut result = Vec::with_capacity(message_ids.len());
        for message_id in message_ids {
            result.push(table.get_owned((peer_id, message_id))?.map(|v| {
                let (media_type, id, access_hash, file_reference) = v.value();
                let media_type: MediaType = media_type.try_into().unwrap();
                match media_type {
                    MediaType::Photo => tl::types::InputMediaPhoto {
                        id: tl::types::InputPhoto {
                            id,
                            access_hash,
                            file_reference,
                        }
                        .into(),
                        spoiler: false,
                        live_photo: false,
                        ttl_seconds: None,
                        video: None,
                    }
                    .into(),
                    MediaType::Video => tl::types::InputMediaDocument {
                        id: tl::types::InputDocument {
                            id,
                            access_hash,
                            file_reference,
                        }
                        .into(),
                        spoiler: false,
                        video_cover: None,
                        video_timestamp: None,
                        ttl_seconds: None,
                        query: None,
                    }
                    .into(),
                }
            }))
        }
        Ok(result)
    })
    .await
    .unwrap()
}

pub async fn insert_medias(peer_id: PeerId, messages: &[Arc<Message>]) -> Result<(), Error> {
    let peer_id = peer_id.bot_api_dialog_id().unwrap();
    let mut medias = Vec::with_capacity(messages.len());
    for message in messages {
        let msg_id = message.id();
        if let Some(media) = message.media() {
            match media {
                Media::Photo(photo) => {
                    if let tl::enums::InputPhoto::Photo(tl::types::InputPhoto {
                        id,
                        access_hash,
                        file_reference,
                    }) = photo.to_raw_input_photo()
                    {
                        medias.push((
                            msg_id,
                            (MediaType::Photo as u8, id, access_hash, file_reference),
                        ));
                    }
                }
                Media::Document(document) if crate::utils::is_video(&document) => {
                    if let tl::enums::InputDocument::Document(tl::types::InputDocument {
                        id,
                        access_hash,
                        file_reference,
                    }) = document.to_raw_input_document()
                    {
                        medias.push((
                            msg_id,
                            (MediaType::Video as u8, id, access_hash, file_reference),
                        ));
                    }
                }
                _ => {}
            }
        }
    }

    spawn_blocking(move || {
        let db = merge_db()?;
        let write_txn = db.begin_write()?;
        {
            let mut table = write_txn.open_table(MEDIAS)?;
            for (msg_id, media) in medias {
                table.insert((peer_id, msg_id), media)?;
            }
        }
        write_txn.commit()?;

        Ok(())
    })
    .await
    .unwrap()
}

pub async fn get_session(peer_id: PeerId) -> Result<Vec<i32>, Error> {
    let peer_id = peer_id.bot_api_dialog_id().unwrap();
    spawn_blocking(move || {
        let db = merge_db()?;
        let read_txn = db.begin_read()?;
        let table = match read_txn.open_table(SESSIONS) {
            Ok(t) => t,
            Err(TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };

        let mut message_ids = Vec::new();
        for entry in table.range((peer_id, 0)..(peer_id, u8::MAX))? {
            let (_, ids_guard) = entry?;
            message_ids.extend(ids_guard.value());
        }
        Ok(message_ids)
    })
    .await
    .unwrap()
}

pub async fn insert_session(peer_id: PeerId, message_ids: Vec<i32>) -> Result<(), Error> {
    let peer_id = peer_id.bot_api_dialog_id().unwrap();
    spawn_blocking(move || {
        let db = merge_db()?;
        let write_txn = db.begin_write()?;
        {
            let mut table = write_txn.open_table(SESSIONS)?;

            let mut new_idx = 0;
            if let Some(last_entry) = table.range((peer_id, 0)..=(peer_id, u8::MAX))?.last() {
                let (key_guard, _) = last_entry?;
                let (_, last_idx) = key_guard.value();
                new_idx = last_idx + 1;
            }

            table.insert((peer_id, new_idx), message_ids)?;
        }
        write_txn.commit()?;

        Ok(())
    })
    .await
    .unwrap()
}

pub async fn finish_session(peer_id: PeerId) -> Result<(), Error> {
    let peer_id = peer_id.bot_api_dialog_id().unwrap();
    spawn_blocking(move || {
        let db = merge_db()?;
        let write_txn = db.begin_write()?;
        {
            let mut table = write_txn.open_table(SESSIONS)?;

            let mut keys_to_delete = Vec::new();
            for entry in table.range((peer_id, u8::MIN)..=(peer_id, u8::MAX))? {
                let (key_guard, _) = entry?;
                let key = key_guard.value();
                keys_to_delete.push(key);
            }

            for key in keys_to_delete {
                table.remove(key)?;
            }
        }
        write_txn.commit()?;

        Ok(())
    })
    .await
    .unwrap()
}
