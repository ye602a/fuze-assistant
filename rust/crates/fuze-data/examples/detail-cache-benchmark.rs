//! Explicit public data directory only; no user profile, network, game or artwork access.
use fuze_data::{detail_cache::Cache, prepare, Locale, Root, Scope, Source};
use std::time::Instant;
fn main() {
    let roots = [Root {
        path: std::env::args_os()
            .nth(1)
            .expect("public data directory")
            .into(),
        source: Source::Bundled,
    }];
    let ids: Vec<u32> = prepare(&roots, Locale::ZhCn, Scope::Champions)
        .unwrap()
        .document("champions.json")
        .unwrap()["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["id"].as_u64().map(|n| n as u32))
        .collect();
    let started = Instant::now();
    for id in ids.iter().take(10) {
        prepare(&roots, Locale::ZhCn, Scope::Detail(*id))
            .unwrap()
            .champion_detail(*id)
            .unwrap();
    }
    let old_ten_ms = started.elapsed().as_secs_f64() * 1000.;
    let started = Instant::now();
    let mut cache = Cache::prepare(&roots, "").unwrap();
    let snapshot_ms = started.elapsed().as_secs_f64() * 1000.;
    for id in &ids {
        cache.get(*id).unwrap();
    }
    let all_ready_ms = started.elapsed().as_secs_f64() * 1000.;
    let started = Instant::now();
    for _ in 0..100 {
        for id in &ids {
            std::hint::black_box(cache.get(*id).unwrap());
        }
    }
    println!(
        "{}",
        serde_json::json!({"heroes":ids.len(),"old_ten_ms":old_ten_ms,
        "snapshot_ms":snapshot_ms,"all_ready_ms":all_ready_ms,
        "cached_lookup_mean_us":started.elapsed().as_secs_f64()*1_000_000./(100*ids.len()) as f64})
    );
}
