//! Explicit read-only data inspection. Outputs aggregate data metadata, never paths or row text.
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("arguments".into());
    }
    let id = args[1].parse::<u32>()?;
    let data = fuze_data::prepare(
        &[fuze_data::Root {
            path: args[0].clone().into(),
            source: fuze_data::Source::Bundled,
        }],
        fuze_data::Locale::ZhCn,
        fuze_data::Scope::Champion(id),
    )?;
    let stats = data.recommendations(id)?;
    println!(
        "{}",
        serde_json::json!({"data_version":data.version(),"locale":data.locale().code(),
        "catalog_entries":data.catalog().len(),"statistic_entries":stats.len(),
        "with_win_rate":stats.values().filter(|r|r.win_rate.is_some()).count(),
        "with_pick_rate":stats.values().filter(|r|r.pick_rate.is_some()).count(),
        "with_recommendation":stats.values().filter(|r|r.score.is_some()).count(),"scope":"overall"})
    );
    Ok(())
}
fn main() {
    if run().is_err() {
        eprintln!("data-inspection-failed");
        std::process::exit(1);
    }
}
