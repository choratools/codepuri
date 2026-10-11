//! Evidence-qualified, configurable library source selection.
//!
//! The evidence file names trusted reference artifacts and their DEX builds.
//! The CLI rechecks input/reference hashes, exact original class implementations
//! and anchor uniqueness; a raw detector score alone never removes source.
use crate::library_fingerprint::{self, ClassFingerprint};
use dex_format::DexFile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

/// Default candidate profile threshold; implementation proof is mandatory at any threshold.
pub const DEFAULT_MIN_SCORE: f64 = 0.95;

/// A locally trusted artifact and its reproducible DEX reference build.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    /// Stable artifact coordinate or provenance identifier.
    pub id: String,
    /// Original JAR/AAR file; relative paths are relative to the evidence file.
    pub artifact_file: PathBuf,
    /// SHA-256 of the original artifact file.
    pub artifact_sha256: String,
    /// ZIP/APK containing the reference artifact's root classes*.dex.
    pub dex_file: PathBuf,
    /// SHA-256 of the reference DEX archive.
    pub dex_sha256: String,
    /// Detector candidate similarity, not a probability of authorship.
    pub score: f64,
    /// Source of the detector/profile score.
    pub score_basis: String,
}

/// Versioned candidate evidence. Exact classes are verified locally, not asserted here.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    /// Must equal `codepuri-library-evidence-v1`.
    pub format: String,
    /// SHA-256 of the original input APK.
    pub apk_sha256: String,
    /// Candidate artifact references to verify.
    pub references: Vec<Reference>,
}

/// A class retained or separated, with an explicit selection basis.
#[derive(Clone, Debug, Serialize)]
pub struct ClassDecision {
    /// Original class descriptor; package prefixes are not substituted.
    pub owner: String,
    /// Reference artifact identifier.
    pub artifact: String,
    /// Candidate profile similarity.
    pub score: f64,
    /// Whether the class passed all mandatory checks.
    pub selected: bool,
    /// Exact implementation fingerprint, when available.
    pub implementation_sha256: Option<String>,
    /// Distinctive supported method-body anchors.
    pub unique_anchors: usize,
    /// Human-readable decision, without claiming version/license certainty.
    pub reason: String,
}

/// Reproducible selection receipt, separate from Java publication coverage.
#[derive(Debug, Serialize)]
pub struct Selection {
    /// Receipt schema version.
    pub format: &'static str,
    /// Exact input APK hash.
    pub apk_sha256: String,
    /// Applied CLI threshold.
    pub min_score: f64,
    /// Original class owners that passed all checks.
    pub owners: BTreeSet<String>,
    /// Original artifact/reference hashes and score provenance.
    pub references: Vec<Reference>,
    /// Decisions for same-owner candidates, including rejected candidates.
    pub decisions: Vec<ClassDecision>,
}

/// Validate a finite score in the inclusive range 0..=1.
pub fn validate_score(score: f64) -> Result<(), String> {
    if score.is_finite() && (0.0..=1.0).contains(&score) {
        Ok(())
    } else {
        Err("library score must be finite and between 0 and 1".into())
    }
}

/// SHA-256 of a file without loading it into another full-size buffer.
pub fn file_sha256(path: &Path) -> Result<String, std::io::Error> {
    let mut stream = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn checked_hash(path: &Path, expected: &str) -> Result<(), String> {
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid SHA-256 for {}", path.display()));
    }
    let actual = file_sha256(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!("SHA-256 mismatch: {}", path.display()))
    }
}

fn relative(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    }
}

/// Parse root DEX entries in an APK or reference ZIP without requiring an APK
/// manifest. Reference artifacts are DEX containers, not runnable applications.
pub fn read_dex_archive(path: &Path) -> Result<Vec<DexFile>, String> {
    let mut archive = zip::ZipArchive::new(File::open(path).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        let name = archive
            .by_index(index)
            .map_err(|error| error.to_string())?
            .name()
            .to_owned();
        let order = if name == "classes.dex" {
            Some(1)
        } else {
            name.strip_prefix("classes")
                .and_then(|text| text.strip_suffix(".dex"))
                .filter(|text| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
                .and_then(|text| text.parse::<u32>().ok())
                .filter(|number| *number >= 2)
        };
        if let Some(order) = order {
            entries.push((order, index));
        }
    }
    entries.sort_unstable();
    if entries.is_empty() {
        return Err("reference archive has no root classes*.dex".into());
    }
    let mut files = Vec::new();
    for (_, index) in entries {
        let mut bytes = Vec::new();
        archive
            .by_index(index)
            .map_err(|error| error.to_string())?
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        files.extend(DexFile::from_container(&bytes).map_err(|error| error.to_string())?);
    }
    Ok(files)
}

/// Load candidate references and verify exact original class implementations.
/// References that are merely below threshold remain recorded but cannot select source.
pub fn select(
    path: &Path,
    apk: &Path,
    files: &[DexFile],
    min_score: f64,
) -> Result<Selection, String> {
    validate_score(min_score)?;
    let evidence: Evidence =
        serde_json::from_reader(File::open(path).map_err(|error| error.to_string())?)
            .map_err(|error| format!("library evidence: {error}"))?;
    if evidence.format != "codepuri-library-evidence-v1" {
        return Err("unsupported library evidence format; raw LibScout output is not implementation evidence".into());
    }
    checked_hash(apk, &evidence.apk_sha256)?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let mut input = BTreeMap::<String, Option<ClassFingerprint>>::new();
    let mut anchor_classes = HashMap::<String, usize>::new();
    for dex in files {
        for class in &dex.classes {
            let owner = dex
                .type_id(class.class_idx)
                .and_then(|id| dex.string(id.descriptor_idx))
                .map_err(|error| error.to_string())?;
            if input.contains_key(owner) {
                return Err(format!("ambiguous original class definition: {owner}"));
            }
            let fingerprint = library_fingerprint::class_fingerprint(dex, class);
            let anchors = fingerprint
                .as_ref()
                .map(|fingerprint| fingerprint.anchors.clone())
                .unwrap_or_else(|| library_fingerprint::class_anchors(dex, class));
            for anchor in anchors {
                *anchor_classes.entry(anchor).or_default() += 1;
            }
            input.insert(owner.to_owned(), fingerprint);
        }
    }
    let mut selection = Selection {
        format: "codepuri-library-selection-v1",
        apk_sha256: evidence.apk_sha256.to_lowercase(),
        min_score,
        owners: BTreeSet::new(),
        references: Vec::new(),
        decisions: Vec::new(),
    };
    let mut ids = BTreeSet::new();
    for mut reference in evidence.references {
        validate_score(reference.score)?;
        if reference.id.is_empty()
            || reference.score_basis.is_empty()
            || !ids.insert(reference.id.clone())
        {
            return Err(
                "library references require unique identifiers and a nonempty score basis".into(),
            );
        }
        let artifact = relative(base, &reference.artifact_file);
        let dex_path = relative(base, &reference.dex_file);
        checked_hash(&artifact, &reference.artifact_sha256)?;
        checked_hash(&dex_path, &reference.dex_sha256)?;
        if reference
            .artifact_sha256
            .eq_ignore_ascii_case(&selection.apk_sha256)
        {
            return Err("input APK cannot serve as its own library artifact".into());
        }
        let references = read_dex_archive(&dex_path)
            .map_err(|error| format!("reference {}: {error}", reference.id))?;
        let mut seen = BTreeSet::new();
        for dex in references {
            for class in &dex.classes {
                let owner = dex
                    .type_id(class.class_idx)
                    .and_then(|id| dex.string(id.descriptor_idx))
                    .map_err(|error| error.to_string())?
                    .to_owned();
                if !seen.insert(owner.clone()) {
                    return Err(format!("ambiguous reference class: {owner}"));
                }
                let Some(original) = input.get(&owner) else {
                    continue;
                };
                let fingerprint = library_fingerprint::class_fingerprint(&dex, class);
                let (matched, anchors) = match (original, fingerprint.as_ref()) {
                    (Some(original), Some(expected)) if original.sha256 == expected.sha256 => (
                        true,
                        original
                            .anchors
                            .iter()
                            .filter(|anchor| anchor_classes.get(*anchor) == Some(&1))
                            .count(),
                    ),
                    _ => (false, 0),
                };
                let selected = reference.score >= min_score && matched && anchors > 0;
                let reason = if reference.score < min_score {
                    "profile score below threshold"
                } else if !matched {
                    "exact declarations/implementation not established"
                } else if anchors == 0 {
                    "no nontrivial globally distinctive implementation anchor"
                } else {
                    "exact original class implementation and distinctive anchor verified"
                };
                if selected {
                    selection.owners.insert(owner.clone());
                }
                selection.decisions.push(ClassDecision {
                    owner,
                    artifact: reference.id.clone(),
                    score: reference.score,
                    selected,
                    implementation_sha256: fingerprint.map(|fingerprint| fingerprint.sha256),
                    unique_anchors: anchors,
                    reason: reason.into(),
                });
            }
        }
        reference.artifact_file = artifact;
        reference.dex_file = dex_path;
        selection.references.push(reference);
    }
    Ok(selection)
}

/// Write a selection receipt without overwriting an existing artifact.
pub fn write_receipt(path: &Path, selection: &Selection) -> Result<(), String> {
    let mut stream = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    serde_json::to_writer_pretty(&mut stream, selection).map_err(|error| error.to_string())?;
    stream.write_all(b"\n").map_err(|error| error.to_string())
}
