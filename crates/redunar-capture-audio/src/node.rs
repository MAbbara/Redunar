use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

const MAX_REGISTRY_BYTES: usize = 4 * 1024 * 1024;
const MAX_NODE_NAME_BYTES: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GameAudioNode {
    pub serial: u32,
    pub process_id: u32,
    pub node_name: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameAudioNodeError {
    RegistryTooLarge,
    MalformedRegistry,
    Ambiguous,
}

impl fmt::Display for GameAudioNodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::RegistryTooLarge => "PipeWire registry exceeds the audio discovery bound",
            Self::MalformedRegistry => "PipeWire registry metadata is malformed",
            Self::Ambiguous => "multiple PipeWire game audio nodes match the running game",
        })
    }
}

impl Error for GameAudioNodeError {}

/// Select exactly one playback stream owned by the game process tree.
///
/// `PipeWire` registry data is treated as untrusted local metadata. A missing
/// match returns `Ok(None)` so games that have not opened audio yet can be
/// retried without degrading video capture.
///
/// # Errors
///
/// Returns [`GameAudioNodeError`] for oversized/malformed metadata or an
/// ambiguous set of game-owned playback streams.
pub fn discover_game_audio_node(
    registry: &[u8],
    process_ids: &BTreeSet<u32>,
) -> Result<Option<GameAudioNode>, GameAudioNodeError> {
    if registry.len() > MAX_REGISTRY_BYTES {
        return Err(GameAudioNodeError::RegistryTooLarge);
    }
    let text = std::str::from_utf8(registry).map_err(|_| GameAudioNodeError::MalformedRegistry)?;
    let objects = json_objects(text)?;
    let mut client_processes = BTreeMap::new();
    for object in &objects {
        if object.contains("\"PipeWire:Interface:Client\"")
            && let (Some(client_id), Some(process_id)) = (
                property_u32(object, "object.id"),
                property_u32(object, "application.process.id"),
            )
        {
            client_processes.insert(client_id, process_id);
        }
    }
    let mut matches = Vec::new();
    for object in objects {
        if !object.contains("\"PipeWire:Interface:Node\"")
            || !property_equals(object, "media.class", "Stream/Output/Audio")
        {
            continue;
        }
        let process_id = property_u32(object, "application.process.id").or_else(|| {
            property_u32(object, "client.id")
                .and_then(|client_id| client_processes.get(&client_id).copied())
        });
        let Some(process_id) = process_id else {
            continue;
        };
        if !process_ids.contains(&process_id) {
            continue;
        }
        let serial =
            property_u32(object, "object.serial").ok_or(GameAudioNodeError::MalformedRegistry)?;
        let node_name =
            property_string(object, "node.name").ok_or(GameAudioNodeError::MalformedRegistry)?;
        if serial == 0 || node_name.is_empty() || node_name.len() > MAX_NODE_NAME_BYTES {
            return Err(GameAudioNodeError::MalformedRegistry);
        }
        matches.push(GameAudioNode {
            serial,
            process_id,
            node_name,
        });
    }
    matches.sort_by_key(|node| node.serial);
    matches.dedup_by_key(|node| node.serial);
    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.pop()),
        _ => Err(GameAudioNodeError::Ambiguous),
    }
}

/// One available physical/input node, with a bounded local display label.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MicrophoneInput {
    pub id: String,
    pub label: String,
}

/// Enumerate input nodes only; playback streams and output monitors are excluded.
///
/// # Errors
/// Oversized or malformed registries are rejected before exposing device choices.
pub fn microphone_inputs_from_registry(
    registry: &[u8],
) -> Result<Vec<MicrophoneInput>, GameAudioNodeError> {
    if registry.len() > MAX_REGISTRY_BYTES {
        return Err(GameAudioNodeError::RegistryTooLarge);
    }
    let text = std::str::from_utf8(registry).map_err(|_| GameAudioNodeError::MalformedRegistry)?;
    // pw-dump supplies an array of objects. The shared object scanner alone
    // also accepts unrelated text; validate this envelope so discovery cannot
    // turn command diagnostics into a successful empty device list.
    let body = text
        .trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or(GameAudioNodeError::MalformedRegistry)?;
    let objects = json_objects(body)?;
    let mut remaining = body.trim();
    for (index, object) in objects.iter().enumerate() {
        if index > 0 {
            remaining = remaining
                .strip_prefix(',')
                .ok_or(GameAudioNodeError::MalformedRegistry)?
                .trim_start();
        }
        remaining = remaining
            .strip_prefix(object)
            .ok_or(GameAudioNodeError::MalformedRegistry)?
            .trim_start();
    }
    if !remaining.is_empty() {
        return Err(GameAudioNodeError::MalformedRegistry);
    }
    let mut inputs = BTreeMap::new();
    for object in objects {
        if !object.contains("\"PipeWire:Interface:Node\"")
            || !property_equals(object, "media.class", "Audio/Source")
        {
            continue;
        }
        let Some(id) = property_string(object, "node.name") else {
            continue;
        };
        if id.is_empty()
            || id.len() > MAX_NODE_NAME_BYTES
            || id.ends_with(".monitor")
            || id.chars().any(char::is_control)
        {
            continue;
        }
        let label = property_string(object, "node.description")
            .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
            .unwrap_or_else(|| id.clone());
        inputs.insert(id.clone(), MicrophoneInput { id, label });
        if inputs.len() > 128 {
            return Err(GameAudioNodeError::RegistryTooLarge);
        }
    }
    Ok(inputs.into_values().collect())
}

/// Select one microphone source from `PipeWire` registry metadata.
///
/// A preferred node name is used when supplied; without one exactly one
/// `Audio/Source` node must exist so Redunar never guesses a physical input.
///
/// # Errors
///
/// Returns [`GameAudioNodeError`] when registry metadata is oversized, malformed,
/// or ambiguous.
pub fn discover_microphone_node(
    registry: &[u8],
    preferred_node_name: Option<&str>,
) -> Result<Option<GameAudioNode>, GameAudioNodeError> {
    if registry.len() > MAX_REGISTRY_BYTES {
        return Err(GameAudioNodeError::RegistryTooLarge);
    }
    let text = std::str::from_utf8(registry).map_err(|_| GameAudioNodeError::MalformedRegistry)?;
    let mut sources = Vec::new();
    for object in json_objects(text)? {
        if !object.contains("\"PipeWire:Interface:Node\"")
            || !property_equals(object, "media.class", "Audio/Source")
        {
            continue;
        }
        let serial =
            property_u32(object, "object.serial").ok_or(GameAudioNodeError::MalformedRegistry)?;
        let node_name =
            property_string(object, "node.name").ok_or(GameAudioNodeError::MalformedRegistry)?;
        if serial != 0 && !node_name.is_empty() && node_name.len() <= MAX_NODE_NAME_BYTES {
            sources.push(GameAudioNode {
                serial,
                process_id: 0,
                node_name,
            });
        }
    }
    sources.sort_by_key(|node| node.serial);
    sources.dedup_by_key(|node| node.serial);
    if let Some(preferred) = preferred_node_name {
        return Ok(sources.into_iter().find(|node| node.node_name == preferred));
    }
    match sources.len() {
        0 => Ok(None),
        1 => Ok(sources.pop()),
        _ => Err(GameAudioNodeError::Ambiguous),
    }
}

/// Select the requested `PipeWire` sink as a fallback monitor target when a
/// game's stream does not expose process ownership metadata. Without a
/// preferred sink, exactly one valid sink must exist; choosing an arbitrary
/// device can silently record an idle HDMI or network output.
///
/// # Errors
///
/// Returns [`GameAudioNodeError`] when the registry is oversized, malformed,
/// or a candidate sink has malformed metadata.
pub fn discover_output_monitor_node(
    registry: &[u8],
    preferred_node_name: Option<&str>,
) -> Result<Option<GameAudioNode>, GameAudioNodeError> {
    if registry.len() > MAX_REGISTRY_BYTES {
        return Err(GameAudioNodeError::RegistryTooLarge);
    }
    let text = std::str::from_utf8(registry).map_err(|_| GameAudioNodeError::MalformedRegistry)?;
    let mut sinks = Vec::new();
    for object in json_objects(text)? {
        if object.contains("\"PipeWire:Interface:Node\"")
            && property_equals(object, "media.class", "Audio/Sink")
        {
            let serial = property_u32(object, "object.serial")
                .ok_or(GameAudioNodeError::MalformedRegistry)?;
            let node_name = property_string(object, "node.name")
                .ok_or(GameAudioNodeError::MalformedRegistry)?;
            if serial != 0 && !node_name.is_empty() && node_name.len() <= MAX_NODE_NAME_BYTES {
                sinks.push(GameAudioNode {
                    serial,
                    process_id: 0,
                    node_name,
                });
            }
        }
    }
    sinks.sort_by_key(|node| node.serial);
    sinks.dedup_by_key(|node| node.serial);
    if let Some(preferred) = preferred_node_name {
        return Ok(sinks.into_iter().find(|node| node.node_name == preferred));
    }
    match sinks.len() {
        0 => Ok(None),
        1 => Ok(sinks.pop()),
        _ => Err(GameAudioNodeError::Ambiguous),
    }
}

fn json_objects(text: &str) -> Result<Vec<&str>, GameAudioNodeError> {
    let bytes = text.as_bytes();
    let mut result = Vec::new();
    let mut depth = 0_usize;
    let mut start = None;
    let mut in_string = false;
    let mut escaped = false;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 {
                    start = Some(index);
                }
                depth = depth
                    .checked_add(1)
                    .ok_or(GameAudioNodeError::MalformedRegistry)?;
            }
            b'}' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or(GameAudioNodeError::MalformedRegistry)?;
                if depth == 0 {
                    let object_start = start.take().ok_or(GameAudioNodeError::MalformedRegistry)?;
                    result.push(&text[object_start..=index]);
                }
            }
            _ => {}
        }
    }
    if depth != 0 || in_string || start.is_some() {
        return Err(GameAudioNodeError::MalformedRegistry);
    }
    Ok(result)
}

fn property_equals(object: &str, key: &str, expected: &str) -> bool {
    property_string(object, key).as_deref() == Some(expected)
}

fn property_u32(object: &str, key: &str) -> Option<u32> {
    let tail = property_tail(object, key)?;
    let tail = tail.trim_start();
    if let Some(quoted) = tail.strip_prefix('"') {
        return quoted.split('"').next()?.parse().ok();
    }
    tail.split(|character: char| !character.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

fn property_string(object: &str, key: &str) -> Option<String> {
    let tail = property_tail(object, key)?.trim_start().strip_prefix('"')?;
    let end = tail.find('"')?;
    let value = &tail[..end];
    if value.contains('\\') || value.chars().any(char::is_control) {
        return None;
    }
    Some(value.to_owned())
}

fn property_tail<'a>(object: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let tail = object.split_once(&needle)?.1;
    Some(tail.split_once(':')?.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry(nodes: &str) -> Vec<u8> {
        format!("[{{\"id\":1,\"type\":\"PipeWire:Interface:Core\"}},{nodes}]").into_bytes()
    }

    fn node(serial: u32, pid: u32, class: &str) -> String {
        format!(
            "{{\"id\":9,\"type\":\"PipeWire:Interface:Node\",\"info\":{{\"props\":{{\"media.class\":\"{class}\",\"application.process.id\":{pid},\"object.serial\":{serial},\"node.name\":\"game-audio-{serial}\"}}}}}}"
        )
    }

    fn client(client_id: u32, pid: u32) -> String {
        [
            "{\"id\":",
            &client_id.to_string(),
            ",\"type\":\"PipeWire:Interface:Client\",\"info\":{\"props\":{\"object.id\":",
            &client_id.to_string(),
            ",\"application.process.id\":",
            &pid.to_string(),
            "}}}",
        ]
        .concat()
    }

    #[test]
    fn enumerates_named_inputs_with_labels_and_excludes_output_nodes() {
        let registry=br#"[{"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Audio/Source","node.name":"usb-mic","node.description":"USB microphone"}}},{"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Audio/Sink","node.name":"speakers"}}},{"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Audio/Source","node.name":"speakers.monitor"}}}]"#;
        assert_eq!(
            microphone_inputs_from_registry(registry).unwrap(),
            vec![MicrophoneInput {
                id: "usb-mic".into(),
                label: "USB microphone".into()
            }]
        );
        for malformed in [b"malformed".as_slice(), b"[bad]", b"[{},]", b"{}"] {
            assert!(microphone_inputs_from_registry(malformed).is_err());
        }
        assert!(microphone_inputs_from_registry(b"[ ]").unwrap().is_empty());
        assert!(microphone_inputs_from_registry(&vec![b' '; MAX_REGISTRY_BYTES + 1]).is_err());
    }
    #[test]
    fn selects_only_one_game_owned_playback_node() {
        let data = registry(&format!(
            "{},{}",
            node(41, 900, "Stream/Output/Audio"),
            node(42, 901, "Audio/Sink")
        ));
        let pids = BTreeSet::from([900]);
        let selected = discover_game_audio_node(&data, &pids)
            .expect("registry")
            .expect("game node");
        assert_eq!(selected.serial, 41);
        assert_eq!(selected.process_id, 900);
    }

    #[test]
    fn unrelated_stream_does_not_match_game_ownership() {
        let data = registry(&node(41, 777, "Stream/Output/Audio"));
        assert_eq!(
            discover_game_audio_node(&data, &BTreeSet::from([900])).expect("registry"),
            None
        );
    }

    #[test]
    fn ambiguous_game_streams_fail_closed() {
        let data = registry(&format!(
            "{},{}",
            node(41, 900, "Stream/Output/Audio"),
            node(42, 900, "Stream/Output/Audio")
        ));
        assert_eq!(
            discover_game_audio_node(&data, &BTreeSet::from([900])),
            Err(GameAudioNodeError::Ambiguous)
        );
    }

    #[test]
    fn resolves_node_ownership_through_pipewire_client_id() {
        let owned_node = node(41, 0, "Stream/Output/Audio")
            .replace("\"application.process.id\":0,", "\"client.id\":77,");
        let data = registry(&format!("{},{}", client(77, 900), owned_node));
        let selected = discover_game_audio_node(&data, &BTreeSet::from([900]))
            .expect("registry")
            .expect("owned node");
        assert_eq!(selected.serial, 41);
        assert_eq!(selected.process_id, 900);
    }

    #[test]
    fn output_fallback_selects_the_named_default_sink() {
        let data = registry(&format!(
            "{},{}",
            node(41, 0, "Audio/Sink"),
            node(42, 0, "Audio/Sink")
        ));
        let selected = discover_output_monitor_node(&data, Some("game-audio-42"))
            .expect("registry")
            .expect("default sink");
        assert_eq!(selected.serial, 42);
    }

    #[test]
    fn microphone_source_selects_named_input() {
        let data = registry(&format!(
            "{},{}",
            node(41, 0, "Audio/Source"),
            node(42, 0, "Audio/Source")
        ));
        let selected = discover_microphone_node(&data, Some("game-audio-42"))
            .expect("registry")
            .expect("microphone");
        assert_eq!(selected.serial, 42);
    }

    #[test]
    fn microphone_source_rejects_ambiguous_default() {
        let data = registry(&format!(
            "{},{}",
            node(41, 0, "Audio/Source"),
            node(42, 0, "Audio/Source")
        ));
        assert_eq!(
            discover_microphone_node(&data, None),
            Err(GameAudioNodeError::Ambiguous)
        );
    }

    #[test]
    fn output_fallback_never_guesses_between_multiple_sinks() {
        let data = registry(&format!(
            "{},{}",
            node(41, 0, "Audio/Sink"),
            node(42, 0, "Audio/Sink")
        ));
        assert_eq!(
            discover_output_monitor_node(&data, None),
            Err(GameAudioNodeError::Ambiguous)
        );
    }
}
