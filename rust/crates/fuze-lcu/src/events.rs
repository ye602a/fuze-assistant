//! Bounded event reassembly. Events invalidate context; they never prove authenticated state.
use crate::{Error, Result};
#[derive(Default)]
pub struct Fragments {
    bytes: Vec<u8>,
}
impl Fragments {
    pub fn push(&mut self, bytes: &[u8], complete: bool) -> Result<bool> {
        if self.bytes.len() + bytes.len() > 256 * 1024 {
            crate::wipe(&mut self.bytes);
            self.bytes.clear();
            return Err(Error::ResponseLimit);
        }
        self.bytes.extend_from_slice(bytes);
        if !complete {
            return Ok(false);
        }
        let result = relevant(&self.bytes);
        crate::wipe(&mut self.bytes);
        self.bytes.clear();
        result
    }
}
impl Drop for Fragments {
    fn drop(&mut self) {
        crate::wipe(&mut self.bytes);
    }
}
fn relevant(bytes: &[u8]) -> Result<bool> {
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| Error::InvalidPhase)?;
    let Some(v) = v.as_array() else {
        return Err(Error::InvalidPhase);
    };
    if v.len() != 3 || v[0].as_u64() != Some(8) {
        return Ok(false);
    }
    if !v[1]
        .as_str()
        .is_some_and(|s| s == "OnJsonApiEvent" || s.starts_with("OnJsonApiEvent_"))
    {
        return Ok(false);
    }
    if !matches!(
        v[2]["eventType"].as_str(),
        Some("Create" | "Update" | "Delete")
    ) {
        return Ok(false);
    }
    Ok(matches!(
        v[2]["uri"].as_str(),
        Some(
            "/lol-gameflow/v1/gameflow-phase"
                | "/lol-gameflow/v1/session"
                | "/lol-champ-select/v1/session"
                | "/riotclient/region-locale"
        )
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragments_are_bounded_and_only_relevant_events_refresh() {
        let bytes = br#"[8,"OnJsonApiEvent",{"uri":"/lol-champ-select/v1/session","eventType":"Update","data":{}}]"#;
        let mut f = Fragments::default();
        assert!(!f.push(&bytes[..20], false).unwrap());
        assert!(f.push(&bytes[20..], true).unwrap());
        assert!(!f
            .push(
                br#"[8,"OnJsonApiEvent",{"uri":"/other","eventType":"Update"}]"#,
                true
            )
            .unwrap());
        assert!(f.push(&vec![0; 256 * 1024 + 1], false).is_err());
    }
}
