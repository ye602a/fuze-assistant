//! Reuses the existing bounded icon worker/cache. Never downloads or decodes on the OCR/UI thread.
use crate::{
    champion_assets::{Reply, Request, Service},
    Result,
};
use std::{collections::HashMap, sync::Arc};
pub struct Art {
    service: Service,
    urls: HashMap<u32, String>,
    requested: Vec<u32>,
    reply: Reply,
    version: String,
    offline: bool,
    pub download: bool,
}
impl Art {
    pub fn new(data: &fuze_data::Prepared, offline: bool) -> Result<Self> {
        let urls = data
            .document("augments.json")
            .and_then(|d| d["data"].as_array())
            .into_iter()
            .flatten()
            .filter_map(|r| {
                let id = u32::try_from(r["id"].as_u64()?).ok()?;
                let url = r["iconUrl"].as_str()?;
                (id > 0 && url.len() <= 200).then(|| (id, url.to_owned()))
            })
            .collect::<HashMap<_, _>>();
        if urls.is_empty() {
            return Err("overlay-icons-unavailable");
        }
        Ok(Self {
            service: Service::new(vec![], offline)?,
            urls,
            requested: vec![],
            reply: Default::default(),
            version: data.identity(),
            offline,
            download: false,
        })
    }
    pub fn request(&mut self, slots: &[fuze_contract::Slot; 3]) {
        let ids: Vec<_> = slots
            .iter()
            .filter_map(|s| match s {
                fuze_contract::Slot::Known { id, .. } if self.urls.contains_key(id) => Some(*id),
                _ => None,
            })
            .collect();
        if ids == self.requested {
            return;
        }
        self.requested = ids;
        self.reply = Default::default();
        self.service.request(if self.requested.is_empty() {
            None
        } else {
            Some(Request {
                presentation: Some(Arc::new(fuze_data::champion_detail::Detail {
                    title: String::new(),
                    lines: vec![],
                })),
                version: self.version.clone(),
                id: 0,
                download: self.download && !self.offline,
                icons: self
                    .requested
                    .iter()
                    .filter_map(|id| self.urls.get(id).cloned())
                    .collect(),
            })
        });
    }
    pub fn poll(&mut self, overlay: &fuze_native_overlay::native::HiddenOverlay) {
        let Some(reply) = self.service.changed(&self.reply) else {
            return;
        };
        let pictures = self
            .requested
            .iter()
            .filter_map(|id| {
                let pic = reply.icons.get(self.urls.get(id)?)?;
                Some((*id, Arc::new(pic.bgra.clone())))
            })
            .collect();
        self.reply = reply;
        // Cosmetic assets must not fail or delay the recognition state machine on an unavailable UI.
        let _ = overlay.icons(pictures);
    }
}
