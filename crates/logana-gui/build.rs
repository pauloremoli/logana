fn main() {
    println!("cargo:rerun-if-changed=../../assets/logana.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/logana.ico");
        let resource =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("logana.rc");
        std::fs::write(
            &resource,
            format!(
                "1 ICON \"{}\"\n",
                icon.display().to_string().replace('\\', "/")
            ),
        )
        .unwrap();
        embed_resource::compile_for(resource, ["logana-gui"], embed_resource::NONE)
            .manifest_required()
            .unwrap();
    }
}
