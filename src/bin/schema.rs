fn main() {
    let schema = schemars::schema_for!(logana::config::Config);
    let json = serde_json::to_string_pretty(&schema).expect("failed to serialize schema");
    println!("{json}");
}
