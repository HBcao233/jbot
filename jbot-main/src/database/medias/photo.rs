use grammers_tl_types as tl;
use redb::{Error, ReadableDatabase, TableDefinition, TableError};
use tokio::task::spawn_blocking;

use super::medias_db;

const PHOTOS: TableDefinition<&str, (i64, i64, Vec<u8>)> = TableDefinition::new("photos");

pub async fn get(key: String) -> Result<Option<tl::enums::InputPhoto>, Error> {
    spawn_blocking(move || {
        let db = medias_db()?;
        let read_txn = db.begin_read()?;
        let table = match read_txn.open_table(PHOTOS) {
            Ok(t) => t,
            Err(TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        Ok(table.get_owned(&*key)?.map(|v| {
            let (id, access_hash, file_reference) = v.value();
            tl::types::InputPhoto {
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

pub async fn insert(key: String, photo: tl::types::InputPhoto) -> Result<(), Error> {
    spawn_blocking(move || {
        let tl::types::InputPhoto {
            id,
            access_hash,
            file_reference,
        } = photo;
        let db = medias_db()?;
        let write_txn = db.begin_write()?;
        {
            let mut table = write_txn.open_table(PHOTOS)?;
            table.insert(&*key, (id, access_hash, file_reference))?;
        }
        write_txn.commit()?;

        Ok(())
    })
    .await
    .unwrap()
}
