use grammers_client::media::{Document, Media};
use grammers_client::peer::Peer;
use grammers_session::types::PeerId;
use grammers_tl_types as tl;

pub fn safe_truncate(text: &str, num: usize) -> String {
    if text.chars().count() <= num {
        text.to_string()
    } else {
        let text: String = text.chars().take(num).collect();
        format!("{}...", text)
    }
}

pub fn peer_full_name(peer: &Peer) -> Option<String> {
    match peer {
        Peer::User(user) => Some(safe_truncate(&user.full_name(), 20)),
        Peer::Group(group) => group.title().map(str::to_string),
        Peer::Channel(channel) => Some(channel.title().to_string()),
        Peer::Community(community) => Some(community.title().to_string()),
    }
}

pub fn get_peer_info(peer_id: &PeerId, peer: Option<&Peer>) -> String {
    let name = peer
        .and_then(peer_full_name)
        .unwrap_or("Unknown".to_string());
    let name = safe_truncate(&name, 20);
    let username = peer.and_then(|p| p.username());
    let username = match username {
        Some(x) => &format!(" <@{x}>"),
        None => "",
    };
    format!("{name}({peer_id}{username})")
}

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
