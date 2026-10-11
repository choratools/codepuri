//! Original implementation inventory for conservative library source separation.
use decomp::library_fingerprint;
use serde_json::json;
use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: library_inventory <apk-or-dex-zip>")?;
    let files = decomp::library_evidence::read_dex_archive(std::path::Path::new(&path))?;
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    writeln!(
        output,
        "{}",
        json!({"kind":"input", "format":library_fingerprint::KIND,
        "sha256":decomp::library_evidence::file_sha256(std::path::Path::new(&path))?, "dex_files":files.len()})
    )?;
    for (pool, dex) in files.iter().enumerate() {
        for class in &dex.classes {
            let owner = dex.string(dex.type_id(class.class_idx)?.descriptor_idx)?;
            let fingerprint = library_fingerprint::class_fingerprint(dex, class);
            let anchors = fingerprint
                .as_ref()
                .map(|fingerprint| fingerprint.anchors.clone())
                .unwrap_or_else(|| library_fingerprint::class_anchors(dex, class));
            writeln!(
                output,
                "{}",
                json!({"kind":"class", "pool":pool, "owner":owner,
                "implementation_sha256":fingerprint.map(|fingerprint| fingerprint.sha256), "anchors":anchors})
            )?;
        }
    }
    output.flush()?;
    Ok(())
}
