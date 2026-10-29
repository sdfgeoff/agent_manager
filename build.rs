use std::{env, fs, path::Path};
fn collect(root: &Path, path: &Path, out: &mut String) {
    let mut entries = fs::read_dir(path)
        .expect("Read frontend build")
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            collect(root, &p, out);
        } else {
            let name = p
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let mime = match p.extension().and_then(|s| s.to_str()).unwrap_or("") {
                "html" => "text/html; charset=utf-8",
                "js" => "text/javascript; charset=utf-8",
                "css" => "text/css; charset=utf-8",
                "svg" => "image/svg+xml",
                "png" => "image/png",
                "webmanifest" => "application/manifest+json",
                _ => "application/octet-stream",
            };
            out.push_str(&format!(
                "{name:?}=>Some(({mime:?},include_bytes!({:?}))),\n",
                p.canonicalize().unwrap()
            ));
        }
    }
}
fn main() {
    let root = Path::new("web/dist");
    println!("cargo:rerun-if-changed=web/dist");
    assert!(
        root.join("index.html").exists(),
        "Frontend build missing. Run `npm ci --prefix web && npm run build --prefix web` before Cargo."
    );
    let mut source =
        "pub fn asset(path:&str)->Option<(&'static str,&'static [u8])>{match path {\n".to_owned();
    collect(root, root, &mut source);
    source.push_str("_=>None}}\n");
    fs::write(
        Path::new(&env::var("OUT_DIR").unwrap()).join("frontend.rs"),
        source,
    )
    .unwrap();
}
