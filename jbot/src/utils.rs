use grammers_client::media::{Document, Media};
use grammers_tl_types as tl;

pub fn is_video(document: &Document) -> bool {
    match document.raw.document.as_ref() {
        Some(tl::enums::Document::Document(d)) => {
            for attr in &d.attributes {
                match attr {
                    tl::enums::DocumentAttribute::Video(_) => return true,
                    _ => {}
                }
            }
            false
        }
        _ => false,
    }
}

pub fn can_grouped(media: &Media) -> bool {
    match media {
        Media::Photo(_) => true,
        Media::Document(document) => is_video(document),
        _ => false,
    }
}
