use crate::{Error, Result};
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Locale {
    ZhCn,
    ZhTw,
    EnUs,
}
pub fn parse(body: &[u8]) -> Result<Locale> {
    if body.len() > 4096 {
        return Err(Error::ResponseLimit);
    }
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| Error::InvalidPhase)?;
    match v.get("locale").and_then(|v| v.as_str()) {
        Some("zh_CN" | "zh-CN") => Ok(Locale::ZhCn),
        Some("zh_TW" | "zh-TW") => Ok(Locale::ZhTw),
        Some("en_US" | "en-US") => Ok(Locale::EnUs),
        _ => Err(Error::InvalidPhase),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn language_is_explicit_not_inferred_from_region() {
        assert_eq!(
            parse(br#"{"locale":"zh_CN","region":"TENCENT"}"#),
            Ok(Locale::ZhCn)
        );
        assert_eq!(
            parse(br#"{"locale":"en_US","region":"CN"}"#),
            Ok(Locale::EnUs)
        );
        assert!(parse(br#"{"region":"CN"}"#).is_err());
        assert!(parse(br#"{"locale":"ko_KR"}"#).is_err());
    }
}
