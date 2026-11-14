use std::collections::HashMap;

use futures_util::StreamExt;
use zbus::{Connection, MatchRule, MessageStream, OwnedVariant, Variant, message::Type};

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Track {
    pub player: String,
    pub title: String,
    pub artist: String,
    pub art: String,
    pub playing: bool,
    pub can_next: bool,
    pub can_previous: bool,
}

impl Track {
    pub fn app(&self) -> &str {
        let name = self.player.strip_prefix(PREFIX).unwrap_or(&self.player);
        name.split('.').next().unwrap_or(name)
    }

    pub fn art_path(&self) -> Option<std::path::PathBuf> {
        let path = self.art.strip_prefix("file://")?;
        Some(std::path::PathBuf::from(percent_decode(path)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Toggle,
    Next,
    Previous,
}

pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let decoded = (bytes[index] == b'%')
            .then(|| text.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match decoded {
            Some(byte) => {
                out.push(byte);
                index += 3;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn string(value: &Variant<'_>) -> String {
    match value {
        Variant::Str(text) => text.as_str().to_string(),
        Variant::Variant(inner) => string(inner),
        _ => String::new(),
    }
}

fn strings(value: &Variant<'_>) -> Vec<String> {
    match value {
        Variant::Array(array) => array
            .iter()
            .map(string)
            .filter(|text| !text.is_empty())
            .collect(),
        Variant::Variant(inner) => strings(inner),
        Variant::Str(_) => vec![string(value)],
        _ => Vec::new(),
    }
}

fn boolean(value: &Variant<'_>) -> bool {
    match value {
        Variant::Bool(value) => *value,
        Variant::Variant(inner) => boolean(inner),
        _ => false,
    }
}

fn metadata(value: &Variant<'_>, track: &mut Track) {
    let dict = match value {
        Variant::Dict(dict) => dict,
        Variant::Variant(inner) => return metadata(inner, track),
        _ => return,
    };
    for (key, value) in dict.iter() {
        match string(key).as_str() {
            "xesam:title" => track.title = string(value),
            "xesam:artist" => track.artist = strings(value).join(", "),
            "mpris:artUrl" => track.art = string(value),
            _ => {}
        }
    }
}

pub fn parse(player: &str, props: &HashMap<String, OwnedVariant>) -> Track {
    let mut track = Track {
        player: player.to_string(),
        ..Track::default()
    };
    let get = |name: &str| props.get(name).map(|value| &**value);
    track.playing = get("PlaybackStatus").map(string).as_deref() == Some("Playing");
    track.can_next = get("CanGoNext").is_some_and(boolean);
    track.can_previous = get("CanGoPrevious").is_some_and(boolean);
    if let Some(value) = get("Metadata") {
        metadata(value, &mut track);
    }
    track
}

pub fn pick(tracks: &[Track], current: &str) -> Option<Track> {
    let named = |track: &&Track| !track.title.is_empty();
    tracks
        .iter()
        .filter(named)
        .find(|track| track.playing && track.player == current)
        .or_else(|| tracks.iter().filter(named).find(|track| track.playing))
        .or_else(|| {
            tracks
                .iter()
                .filter(named)
                .find(|track| track.player == current)
        })
        .or_else(|| tracks.iter().find(named))
        .cloned()
}

async fn read(connection: &Connection, player: &str) -> zbus::Result<Track> {
    let reply = connection
        .call_method(Some(player), PATH, Some(PROPERTIES), "GetAll", &(PLAYER,))
        .await?;
    let props = reply
        .body()
        .deserialize::<HashMap<String, OwnedVariant>>()?;
    Ok(parse(player, &props))
}

fn relevant(message: &zbus::Message) -> bool {
    let header = message.header();
    let member = header
        .member()
        .map(|name| name.to_string())
        .unwrap_or_default();
    if member == "NameOwnerChanged" {
        return message
            .body()
            .deserialize::<(String, String, String)>()
            .is_ok_and(|(name, _, _)| name.starts_with(PREFIX));
    }
    message
        .body()
        .deserialize::<(String, HashMap<String, OwnedVariant>, Vec<String>)>()
        .is_ok_and(|(interface, changed, _)| {
            interface == PLAYER
                && changed.keys().any(|key| {
                    matches!(
                        key.as_str(),
                        "PlaybackStatus" | "Metadata" | "CanGoNext" | "CanGoPrevious"
                    )
                })
        })
}

pub async fn watch(callback: impl Fn(Option<Track>)) -> zbus::Result<()> {
    let connection = Connection::session().await?;
    let dbus = zbus::fdo::DBusProxy::new(&connection).await?;
    let mut streams = Vec::new();
    for rule in [
        MatchRule::builder()
            .msg_type(Type::Signal)
            .interface(PROPERTIES)
            .member("PropertiesChanged")
            .path(PATH)
            .build()?,
        MatchRule::builder()
            .msg_type(Type::Signal)
            .sender("org.freedesktop.DBus")
            .member("NameOwnerChanged")
            .build()?,
    ] {
        streams.push(MessageStream::for_match_rule(rule, &connection, None).await?);
    }
    let mut stream = futures_util::stream::select_all(streams);
    let mut last: Option<Option<Track>> = None;
    let mut current = String::new();
    loop {
        let mut tracks = Vec::new();
        for name in dbus.list_names().await? {
            let name = name.to_string();
            if !name.starts_with(PREFIX) || name.contains("playerctld") {
                continue;
            }
            if let Ok(track) = read(&connection, &name).await {
                tracks.push(track);
            }
        }
        let track = pick(&tracks, &current);
        if let Some(track) = &track {
            current = track.player.clone();
        }
        if last.as_ref() != Some(&track) {
            last = Some(track.clone());
            callback(track);
        }
        loop {
            let message = stream
                .next()
                .await
                .ok_or_else(|| zbus::Error::Failure("media stream ended".into()))?;
            if message.is_ok_and(|message| relevant(&message)) {
                break;
            }
        }
    }
}

pub async fn send(player: String, command: Command) -> zbus::Result<()> {
    let connection = Connection::session().await?;
    let method = match command {
        Command::Toggle => "PlayPause",
        Command::Next => "Next",
        Command::Previous => "Previous",
    };
    connection
        .call_method(Some(player.as_str()), PATH, Some(PLAYER), method, &())
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(player: &str, title: &str, playing: bool) -> Track {
        Track {
            player: format!("{PREFIX}{player}"),
            title: title.into(),
            playing,
            ..Track::default()
        }
    }

    #[test]
    fn playing_player_wins() {
        let tracks = [
            track("mpv", "a", false),
            track("firefox.instance1", "b", true),
        ];
        let picked = pick(&tracks, &tracks[0].player);
        assert_eq!(picked.as_ref().map(Track::app), Some("firefox"));
    }

    #[test]
    fn paused_current_player_stays() {
        let tracks = [track("mpv", "a", false), track("spotify", "b", false)];
        let picked = pick(&tracks, &tracks[1].player);
        assert_eq!(picked.as_ref().map(Track::app), Some("spotify"));
        assert_eq!(pick(&[track("mpv", "", true)], ""), None);
        assert_eq!(pick(&[], ""), None);
    }

    #[test]
    fn properties_parse() {
        let mut metadata: HashMap<String, Variant<'_>> = HashMap::new();
        metadata.insert("xesam:title".into(), Variant::from("song"));
        metadata.insert(
            "xesam:artist".into(),
            Variant::from(vec!["one".to_string(), "two".to_string()]),
        );
        metadata.insert(
            "mpris:artUrl".into(),
            Variant::from("file:///tmp/a%20b.png"),
        );
        let mut props: HashMap<String, OwnedVariant> = HashMap::new();
        let owned = |value: Variant<'_>| OwnedVariant::try_from(value).ok();
        if let Some(value) = owned(Variant::from(metadata)) {
            props.insert("Metadata".into(), value);
        }
        if let Some(value) = owned(Variant::from("Playing")) {
            props.insert("PlaybackStatus".into(), value);
        }
        if let Some(value) = owned(Variant::from(true)) {
            props.insert("CanGoNext".into(), value);
        }
        let track = parse("org.mpris.MediaPlayer2.mpv", &props);
        assert_eq!(track.title, "song");
        assert_eq!(track.artist, "one, two");
        assert!(track.playing && track.can_next && !track.can_previous);
        assert_eq!(
            track.art_path(),
            Some(std::path::PathBuf::from("/tmp/a b.png"))
        );
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("a%20b%2Fc"), "a b/c");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }
}
