//! Controlled A/B: keep the same installed font bytes resident before any UI/font cache exists.
//! No fonts are bundled, exported or modified. Internal Slint API stays inside this lab.
use i_slint_common::sharedfontdb::{fontdb, FONT_DB};
use std::{collections::HashMap, sync::Arc, time::Instant};

pub fn prepare(resident: bool) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    FONT_DB.with(|cell| {
        let mut db=cell.borrow_mut();
        let default=db.family_name(&fontdb::Family::SansSerif).to_owned();
        let mut ids=Vec::new();
        for family in ["Microsoft YaHei UI","Microsoft YaHei","Segoe UI",default.as_str()] {
            for weight in [400,600] {
                if let Some(id)=db.query(&fontdb::Query {families:&[fontdb::Family::Name(family)],weight:fontdb::Weight(weight),..Default::default()}) {
                    if !ids.contains(&id) {ids.push(id);}
                }
            }
        }
        let at=Instant::now();
        let mut accesses=0;
        for _ in 0..24 { for id in &ids {
            db.with_face_data(*id, |bytes,index| { std::hint::black_box((bytes.len(),index,bytes.first())); accesses+=1; });
        }}
        let file_access_ms=at.elapsed().as_secs_f64()*1000.;
        if !resident { return Ok(serde_json::json!({"mode":"system_file","faces":ids.len(),"face_accesses":accesses,"face_access_ms":file_access_ms,"retained_bytes":0})); }
        let at=Instant::now();
        let mut files=HashMap::new(); let mut bytes=0u64;
        for id in &ids {
            if let Some((fontdb::Source::File(path),_))=db.face_source(*id) {
                if files.contains_key(&path) {continue;}
                let file=std::fs::File::open(&path)?; let size=file.metadata()?.len();
                if size>32*1024*1024 || bytes+size>48*1024*1024 {return Err("font probe memory budget exceeded".into());}
                use std::io::Read;
                let mut data=Vec::with_capacity(size as usize);
                file.take(size+1).read_to_end(&mut data)?;
                if data.len() as u64!=size {return Err("font changed during read".into());}
                bytes+=size; files.insert(path,Arc::new(data));
            }
        }
        // Replace all faces of each retained collection, preserving their face metadata.
        let faces:Vec<_>=db.faces().filter_map(|face| {
            if let fontdb::Source::File(path)=&face.source {
                files.get(path).map(|data| {let mut face=face.clone();face.source=fontdb::Source::Binary(data.clone());face})
            } else {None}
        }).collect();
        let mut new_ids=Vec::new();
        for face in faces {db.make_mut().remove_face(face.id);new_ids.push(db.make_mut().push_face_info(face));}
        let preparation_ms=at.elapsed().as_secs_f64()*1000.;
        let at=Instant::now();let mut accesses=0;
        for _ in 0..24 {for id in &new_ids {db.with_face_data(*id,|data,index|{std::hint::black_box((data.len(),index,data.first()));accesses+=1;});}}
        Ok(serde_json::json!({"mode":"resident_copy","faces":new_ids.len(),"files":files.len(),"retained_bytes":bytes,"preparation_ms":preparation_ms,"before_face_access_ms":file_access_ms,"after_face_access_ms":at.elapsed().as_secs_f64()*1000.,"after_face_accesses":accesses}))
    })
}
