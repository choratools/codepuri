//! APK ZIP extraction for conventional multidex entries.

use std::fs::File;
use std::io::{Read, Seek};
use std::path::Path;

use crate::{DexError, DexFile};

/// Opens an APK path, verifies the manifest entry, and parses every `classes*.dex` entry.
///
/// # Errors
/// Returns an I/O, ZIP, missing-entry, or DEX parsing error.
pub fn parse_apk_path(path: impl AsRef<Path>) -> Result<Vec<DexFile>, DexError> {
    parse_apk(File::open(path)?)
}

/// Parses an APK from a seekable byte source.
///
/// Binary XML and `resources.arsc` are intentionally not decoded.
///
/// # Errors
/// Returns a ZIP, missing-entry, read, or DEX parsing error.
pub fn parse_apk<R: Read + Seek>(reader: R) -> Result<Vec<DexFile>, DexError> {
    let mut archive = zip::ZipArchive::new(reader)?;
    match archive.by_name("AndroidManifest.xml") {
        Ok(_) => {}
        Err(zip::result::ZipError::FileNotFound) => return Err(DexError::MissingManifest),
        Err(error) => return Err(error.into()),
    }
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        let name = archive.by_index(index)?.name().to_owned();
        if let Some(order) = dex_entry_order(&name) {
            entries.push((order, name));
        }
    }
    if entries.is_empty() {
        return Err(DexError::MissingDex);
    }
    entries.sort_unstable_by_key(|entry| entry.0);
    let mut files = Vec::new();
    for (_, name) in entries {
        let mut bytes = Vec::new();
        archive.by_name(&name)?.read_to_end(&mut bytes)?;
        files.extend(DexFile::from_container(&bytes)?);
    }
    Ok(files)
}

fn dex_entry_order(name: &str) -> Option<u32> {
    if name == "classes.dex" {
        return Some(1);
    }
    let suffix = name.strip_prefix("classes")?.strip_suffix(".dex")?;
    if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    suffix.parse::<u32>().ok().filter(|value| *value >= 2)
}
