pub mod photo;
pub mod video;

use std::fs;
use std::sync::{Arc, OnceLock};

use redb::{Database, Error};

static MEDIAS_DB: OnceLock<Arc<Database>> = OnceLock::new();

fn medias_db() -> Result<Arc<Database>, Error> {
    Ok(match MEDIAS_DB.get() {
        Some(db) => Arc::clone(&db),
        None => {
            if let Err(e) = fs::create_dir_all("data") {
                log::error!("数据文件夹创建失败: {e:?}");
            }
            let db = Arc::new(Database::create("data/medias.redb")?);
            MEDIAS_DB.set(db).unwrap();
            Arc::clone(MEDIAS_DB.get().unwrap())
        }
    })
}
