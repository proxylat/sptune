//! All objects related to context

use chrono::serde::ts_milliseconds;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Deserializer, Serialize};

use std::collections::HashMap;

use crate::{
    custom_serde::option_duration_ms, CurrentlyPlayingType, Device, DisallowKey, PlayableItem,
    RepeatState, Type,
};

/// Context object
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Context {
    /// The URI may be of any type, so it's not parsed into a [`crate::Id`]
    pub uri: String,
    pub href: String,
    pub external_urls: HashMap<String, String>,
    #[serde(rename = "type")]
    pub _type: Type,
}

/// Currently playing object
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CurrentlyPlayingContext {
    pub context: Option<Context>,
    #[serde(with = "ts_milliseconds")]
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    #[serde(with = "option_duration_ms", rename = "progress_ms")]
    pub progress: Option<Duration>,
    pub is_playing: bool,
    pub item: Option<PlayableItem>,
    pub currently_playing_type: CurrentlyPlayingType,
    pub actions: Actions,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CurrentPlaybackContext {
    pub device: Device,
    pub repeat_state: RepeatState,
    pub shuffle_state: bool,
    /// Smart Shuffle flag returned by Get Playback State. Undocumented in
    /// the official reference but present in real responses; defaults to
    /// false so older payloads without it still parse.
    #[serde(default)]
    pub smart_shuffle: bool,
    pub context: Option<Context>,
    #[serde(with = "ts_milliseconds")]
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    #[serde(with = "option_duration_ms", rename = "progress_ms")]
    pub progress: Option<Duration>,
    pub is_playing: bool,
    pub item: Option<PlayableItem>,
    pub currently_playing_type: CurrentlyPlayingType,
    pub actions: Actions,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CurrentUserQueue {
    pub currently_playing: Option<PlayableItem>,
    pub queue: Vec<PlayableItem>,
}

/// Actions object
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Actions {
    pub disallows: Vec<DisallowKey>,
}

impl Serialize for Actions {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        struct OriginalActions {
            pub disallows: HashMap<DisallowKey, bool>,
        }
        let original_actions = OriginalActions {
            disallows: self.disallows.iter().map(|&key| (key, true)).collect(),
        };

        original_actions.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Actions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct OriginalActions {
            pub disallows: HashMap<DisallowKey, bool>,
        }
        let orignal_actions = OriginalActions::deserialize(deserializer)?;
        Ok(Self {
            disallows: orignal_actions
                .disallows
                .into_iter()
                .filter(|(_, value)| *value)
                .map(|(key, _)| key)
                .collect(),
        })
    }
}

#[test]
fn test_smart_shuffle_field() {
    // Real Get Playback State responses carry an undocumented smart_shuffle
    // flag; payloads without it (older captures) must still parse as false.
    let with_flag = r#"{
        "device": {
            "id": "mock-device", "is_active": true, "is_private_session": false,
            "is_restricted": false, "name": "Mock", "type": "Computer",
            "volume_percent": 50
        },
        "repeat_state": "off", "shuffle_state": true, "smart_shuffle": true,
        "context": null, "timestamp": 0, "is_playing": true, "item": null,
        "currently_playing_type": "track", "actions": { "disallows": {} }
    }"#;
    let ctx: CurrentPlaybackContext = serde_json::from_str(with_flag).unwrap();
    assert!(ctx.smart_shuffle);
    let without_flag = with_flag.replace(r#""smart_shuffle": true,"#, "");
    let ctx: CurrentPlaybackContext = serde_json::from_str(&without_flag).unwrap();
    assert!(!ctx.smart_shuffle);
}
