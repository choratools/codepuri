//! Metadata-only receipt probe. This runtime candidate is not an attested APK
//! platform match and does not provide Java compiler class files.
use decomp::java_members::{
    external_metadata::{DexMetadata, ExternalMetadata, MetadataChain, MetadataProvider},
    source_closure::method_key,
};
use dex_format::{DexFile, annotations::Visibility, encoded_value::EncodedValueKind};
use std::{error::Error, fs::File, io::Read, path::Path, process::Command, time::Instant};

type ProbeResult<T> = Result<T, Box<dyn Error>>;

fn descriptor(dex: &DexFile, index: u32) -> ProbeResult<&str> {
    Ok(dex.string(dex.type_id(index)?.descriptor_idx)?)
}

#[test]
#[ignore = "requires authorized Android16 candidate framework receipt, not original APK platform"]
fn samsung_icon_tray_genuine_runtime_candidate_declaration() -> ProbeResult<()> {
    let root = Path::new("/tmp/opencode/samsung-platform-metadata-20261003");
    let path = root.join("framework-android16.jar");
    let receipt = std::fs::read_to_string(root.join("provenance.json"))?;
    assert!(receipt.contains("R3CW70LC55D"));
    assert!(receipt.contains("F731NKSU6GZDU"));
    assert!(receipt.contains("\"original_apk_runtime_compatible\": false"));
    assert!(receipt.contains("\"selected_into_baseline\": false"));
    assert!(receipt.contains("\"javac_classpath\": false"));
    assert_eq!(std::fs::metadata(&path)?.len(), 53_917_961);
    let sha = Command::new("sha256sum").arg(&path).output()?;
    assert!(sha.status.success());
    assert!(
        String::from_utf8(sha.stdout)?
            .starts_with("1360ba6c76b98c2715117ed396fc12d63a162f8c53ba4c8dd896d1a6576cfdcb ")
    );

    let mut archive = zip::ZipArchive::new(File::open(&path)?)?;
    let inventory: Vec<_> = archive
        .file_names()
        .filter(|name| name.ends_with(".dex"))
        .map(str::to_owned)
        .collect();
    assert_eq!(inventory.len(), 7);
    let owner = "Landroid/content/pm/PackageManager;";
    let mut declarations = Vec::new();
    let mut owner_count = 0;
    for ordinal in 1..=7 {
        let name = if ordinal == 1 {
            "classes.dex".to_owned()
        } else {
            format!("classes{ordinal}.dex")
        };
        let mut bytes = Vec::new();
        archive.by_name(&name)?.read_to_end(&mut bytes)?;
        let files = DexFile::from_container(&bytes)?;
        assert_eq!(files.len(), 1);
        for dex in &files {
            println!(
                "DEX {name} bytes={} version={} owners={} references={}",
                bytes.len(),
                dex.header.version,
                dex.classes.len(),
                dex.method_ids.len()
            );
            for class in &dex.classes {
                if descriptor(dex, class.class_idx)? != owner {
                    continue;
                }
                owner_count += 1;
                println!(
                    "OWNER {name} flags={:#x} class_offset={} class_data_offset={} parent={} annotations_offset={}",
                    class.access_flags,
                    class.span.offset,
                    class.class_data_off,
                    descriptor(dex, class.superclass_idx)?,
                    class.annotations_off
                );
                if class.annotations_off != 0 && class.annotations.is_none() {
                    return Err("missing original annotation directory".into());
                }
                let data = class.class_data.as_ref().ok_or("missing class data")?;
                for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                    let key = method_key(dex, method.method_idx).ok_or("invalid method key")?;
                    if key.name != "semGetDrawableForIconTray"
                        || key.parameters != ["Landroid/graphics/drawable/Drawable;", "I"]
                        || key.return_type != "Landroid/graphics/drawable/Drawable;"
                    {
                        continue;
                    }
                    assert_eq!(key.owner, owner);
                    let mut throws = None;
                    let mut method_directories = 0;
                    if let Some(directory) = &class.annotations {
                        for member in &directory.method_annotations {
                            if member.member_idx != method.method_idx {
                                continue;
                            }
                            method_directories += 1;
                            assert_eq!(method_directories, 1);
                            for item in &member.annotations.entries {
                                let annotation_type = descriptor(dex, item.annotation.type_idx)?;
                                println!(
                                    "ANNOTATION type={annotation_type} visibility={:?} offset={} elements={:?}",
                                    item.visibility, item.span.offset, item.annotation.elements
                                );
                                if annotation_type != "Ldalvik/annotation/Throws;" {
                                    continue;
                                }
                                assert!(throws.is_none());
                                assert_eq!(item.visibility, Visibility::System);
                                let [element] = item.annotation.elements.as_slice() else {
                                    return Err("invalid Throws elements".into());
                                };
                                assert_eq!(dex.string(element.name_idx)?, "value");
                                let EncodedValueKind::Array(values) = &element.value.value else {
                                    return Err("invalid Throws value".into());
                                };
                                let mut types = Vec::new();
                                for value in values {
                                    let EncodedValueKind::Type(index) = &value.value else {
                                        return Err("invalid Throws type".into());
                                    };
                                    types.push(descriptor(dex, *index)?.to_owned());
                                }
                                throws = Some(types);
                            }
                        }
                    }
                    println!(
                        "DECLARATION {name} index={} key={key:?} flags={:#x} code_offset={} code={:?} explicit_throws={} original_throws={:?}",
                        method.method_idx,
                        method.access_flags,
                        method.code_off,
                        method.code.as_ref().map(|code| (
                            code.span.offset,
                            code.span.size,
                            code.insns_size,
                            code.tries.len()
                        )),
                        throws.is_some(),
                        throws.as_deref().unwrap_or_default()
                    );
                    declarations.push((
                        key,
                        class.access_flags,
                        method.access_flags,
                        throws.unwrap_or_default(),
                    ));
                }
            }
        }
    }
    assert_eq!(owner_count, 1);
    let [(key, class_flags, method_flags, original_throws)] = declarations.as_slice() else {
        return Err("target must be exactly one actual declaration".into());
    };

    let started = Instant::now();
    let metadata = match DexMetadata::from_path(&path) {
        Ok(metadata) => metadata,
        Err(error) => {
            println!(
                "COMPLETE_ADAPTER_REJECTED elapsed={:?} error={error:?}",
                started.elapsed()
            );
            return Err(error.into());
        }
    };
    println!(
        "COMPLETE_ADAPTER owners={} elapsed={:?}",
        metadata.owners().count(),
        started.elapsed()
    );
    assert_eq!(metadata.class_access(owner), Some(*class_flags));
    let declaration = metadata.method(key).ok_or("missing provider declaration")?;
    assert_eq!(declaration.access_flags, *method_flags);
    assert_eq!(declaration.exceptions.as_ref(), Some(original_throws));
    let parsed = metadata
        .declared_methods(owner)
        .ok_or("missing owner members")?;
    assert_eq!(parsed.iter().filter(|method| method.key == *key).count(), 1);
    assert!(parsed.iter().all(|method| method.code.is_none()));
    let chain = MetadataChain::new(vec![MetadataProvider::Dex(&metadata)]);
    assert_eq!(chain.method(key), Some(declaration));
    assert!(chain.jvm_class(owner).is_none());
    println!(
        "POLICY exact runtime declaration available; original APK compatibility unproven; JVM class/code unavailable; baseline unchanged"
    );
    Ok(())
}
