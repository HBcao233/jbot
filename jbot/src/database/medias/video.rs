use grammers_tl_types as tl;
use redb::{Error, ReadableDatabase, TableDefinition, TableError};
use tokio::task::spawn_blocking;

use super::medias_db;

const VIDEOS: TableDefinition<&str, (i64, i64, Vec<u8>)> = TableDefinition::new("videos");

pub async fn get(key: String) -> Result<Option<tl::enums::InputDocument>, Error> {
    spawn_blocking(move || {
        let db = medias_db()?;
        let read_txn = db.begin_read()?;
        let table = match read_txn.open_table(VIDEOS) {
            Ok(t) => t,
            Err(TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        Ok(table.get_owned(&*key)?.map(|v| {
            let (id, access_hash, file_reference) = v.value();
            tl::types::InputDocument {
                id,
                access_hash,
                file_reference,
            }
            .into()
        }))
    })
    .await
    .unwrap()
}

pub async fn insert(key: String, video: tl::types::InputDocument) -> Result<(), Error> {
    spawn_blocking(move || {
        let tl::types::InputDocument {
            id,
            access_hash,
            file_reference,
        } = video;
        let db = medias_db()?;
        let write_txn = db.begin_write()?;
        {
            let mut table = write_txn.open_table(VIDEOS)?;
            table.insert(&*key, (id, access_hash, file_reference))?;
        }
        write_txn.commit()?;

        Ok(())
    })
    .await
    .unwrap()
}
