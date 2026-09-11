/// FNV-1a hash
pub const fn const_hash(s: &str) -> [u8; 4] {
    let mut hash: u32 = 0x811c9dc5;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u32;
        hash = hash.wrapping_mul(0x01000193);
        i += 1;
    }
    hash.to_le_bytes()
}

#[cfg(test)]
pub struct IdRecord {
    pub text: &'static str,
    pub hash: [u8; 4],
    pub file: &'static str,
    pub line: u32,
}

#[cfg(test)]
#[linkme::distributed_slice]
pub static ID_RECORDS: [IdRecord];

#[macro_export]
macro_rules! id {
    ($text:expr) => {{
        const HASH: [u8; 4] = $crate::button::const_hash($text);

        #[cfg(test)]
        #[linkme::distributed_slice($crate::button::ID_RECORDS)]
        static _RECORD: $crate::button::IdRecord = $crate::button::IdRecord {
            text: $text,
            hash: HASH,
            file: file!(),
            line: line!(),
        };

        HASH
    }};
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn check_id_collisions() {
        // id!("yacxa");
        // id!("glbvs");
        let mut map = HashMap::new();

        for record in ID_RECORDS {
            if let Some(existing_record) = map.insert(record.hash, record) {
                if existing_record.text != record.text {
                    panic!(
                        "发现不同id存在相同哈希值:\n\
                        id1: \"{}\" in {}:{}\n\
                        id2: \"{}\" in {}:{}",
                        record.text,
                        record.file,
                        record.line,
                        existing_record.text,
                        existing_record.file,
                        existing_record.line,
                    );
                }
            }
        }
    }
}
