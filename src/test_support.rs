pub fn tempdir() -> tempfile::TempDir {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-state");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("test-")
        .tempdir_in(root)
        .unwrap()
}
