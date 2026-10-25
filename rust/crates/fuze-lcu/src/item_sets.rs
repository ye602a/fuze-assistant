//! Explicit, scoped item-set transaction; never used by discovery, recommendations or OCR.
use serde_json::{json, Value};
fn managed(uid: &str) -> bool {
    [
        "aramgg-aram-current",
        "aramgg-aram-current-2",
        "aramgg-aram-current-3",
        "aramgg-aram-current-4",
    ]
    .contains(&uid)
}
pub fn merge(
    current: &Value,
    sets: &[Value],
    champion: u32,
    account: u64,
    timestamp: u64,
) -> crate::history::Result<Value> {
    if champion == 0 || account == 0 || sets.is_empty() || sets.len() > 4 {
        return Err("item-sets-invalid");
    }
    let existing = current["itemSets"]
        .as_array()
        .filter(|s| s.len() <= 1000)
        .ok_or("item-sets-current-invalid")?;
    if current
        .get("accountId")
        .is_some_and(|id| id.as_u64() != Some(account))
    {
        return Err("item-sets-account-changed");
    }
    let mut seen = std::collections::HashSet::new();
    for s in sets {
        let uid = s["uid"]
            .as_str()
            .filter(|s| managed(s))
            .ok_or("item-sets-invalid")?;
        if !seen.insert(uid)
            || s["associatedChampions"] != json!([champion])
            || (s["associatedMaps"] != json!([11, 12]) && s["associatedMaps"] != json!([12]))
        {
            return Err("item-sets-invalid");
        }
        let blocks = s["blocks"]
            .as_array()
            .filter(|b| !b.is_empty() && b.len() <= 32)
            .ok_or("item-sets-invalid")?;
        for b in blocks {
            let items = b["items"]
                .as_array()
                .filter(|a| !a.is_empty() && a.len() <= 18)
                .ok_or("item-sets-invalid")?;
            for item in items {
                if item["count"] != 1
                    || item["id"]
                        .as_str()
                        .and_then(|id| id.parse::<u32>().ok())
                        .filter(|id| *id > 0)
                        .is_none()
                {
                    return Err("item-sets-invalid");
                }
            }
        }
    }
    let mut next = sets.to_vec();
    next.extend(
        existing
            .iter()
            .filter(|s| !s["uid"].as_str().is_some_and(managed))
            .cloned(),
    );
    let mut payload = current.clone();
    payload["accountId"] = json!(account);
    payload["timestamp"] = json!(timestamp);
    payload["itemSets"] = json!(next);
    if serde_json::to_vec(&payload)
        .map_err(|_| "item-sets-invalid")?
        .len()
        > 1024 * 1024
    {
        return Err("item-sets-limit");
    }
    Ok(payload)
}
/// A successful PUT is not evidence of persistence. Compare only our contract
/// fields; the client may add its own metadata to the returned sets.
pub fn verify_saved(expected: &Value, observed: &Value) -> crate::history::Result<()> {
    if expected["accountId"] != observed["accountId"] {
        return Err("item-sets-readback-mismatch");
    }
    let actual = observed["itemSets"]
        .as_array()
        .ok_or("item-sets-readback-mismatch")?;
    for set in expected["itemSets"].as_array().ok_or("item-sets-invalid")? {
        let uid = set["uid"].as_str().ok_or("item-sets-readback-mismatch")?;
        let found: Vec<_> = actual.iter().filter(|s| s["uid"] == uid).collect();
        if found.len() != 1 {
            return Err("item-sets-readback-mismatch");
        }
        let fields: &[&str] = if managed(uid) {
            &["title", "associatedChampions", "associatedMaps", "blocks"]
        } else {
            &["title", "blocks"]
        };
        if fields.iter().any(|key| set[*key] != found[0][*key]) {
            return Err("item-sets-readback-mismatch");
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readback_detects_missing_changed_or_wrong_account_sets() {
        let set = json!({"uid":"aramgg-aram-current","title":"route","associatedChampions":[21],"associatedMaps":[11,12],"blocks":[{"type":"core","items":[{"id":"3001","count":1}]}]});
        let expected = json!({"accountId":1,"itemSets":[set]});
        assert!(verify_saved(&expected, &expected).is_ok());
        let mut actual = expected.clone();
        actual["timestamp"] = json!(99);
        assert!(verify_saved(&expected, &actual).is_ok());
        actual["itemSets"][0]["blocks"][0]["items"][0]["id"] = json!("3002");
        assert!(verify_saved(&expected, &actual).is_err());
        assert!(verify_saved(&expected, &json!({"accountId":1,"itemSets":[]})).is_err());
        actual = expected.clone();
        actual["accountId"] = json!(2);
        assert!(verify_saved(&expected, &actual).is_err());
    }
    #[test]
    fn preserve_unrelated_sets_and_metadata_reject_wrong_owner() {
        let current = json!({"accountId":1,"custom":"preserve","itemSets":[{"uid":"my-custom","title":"ARAMGG ARAM similar name"},{"uid":"aramgg-aram-current"}]});
        let new = json!({"uid":"aramgg-aram-current","associatedChampions":[21],"associatedMaps":[11,12],"blocks":[{"items":[{"id":"3001","count":1}]}]});
        let next = merge(&current, &[new.clone()], 21, 1, 123).unwrap();
        assert_eq!(next["itemSets"].as_array().unwrap().len(), 2);
        assert_eq!(next["itemSets"][1], current["itemSets"][0]);
        assert_eq!(next["custom"], "preserve");
        assert!(merge(&current, &[new.clone()], 21, 2, 123).is_err());
        assert!(merge(&current, &[new], 22, 1, 123).is_err());
    }
}
