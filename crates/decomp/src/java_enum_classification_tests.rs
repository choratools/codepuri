#![allow(clippy::unwrap_used)]
use super::*;
fn corpus() -> Vec<DexFile> {
    dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap()
}
#[test]
fn genuine_constant_specific_owners_retain_all_method_identities_without_borrowing_clinit() {
    let inputs = corpus();
    let mut owners = 0;
    let mut methods = 0;
    for dex in &inputs {
        for class in &dex.classes {
            if class.access_flags & 0x4000 == 0
                || descriptor(dex, class.superclass_idx).ok() == Some("Ljava/lang/Enum;")
            {
                continue;
            }
            let EnumOwnership::ConstantBody {
                parent,
                methods: ids,
                ..
            } = classify(&inputs, dex, class).unwrap()
            else {
                panic!("child must require parent constant-body plan")
            };
            owners += 1;
            methods += ids.len();
            let data = class.class_data.as_ref().unwrap();
            assert!(!data.direct_methods.iter().any(|m| {
                let raw = dex.method_id(m.method_idx).unwrap();
                dex.string(raw.name_idx).unwrap() == "<clinit>"
            }));
            assert!(inputs.iter().any(|file| {
                file.classes
                    .iter()
                    .any(|c| descriptor(file, c.class_idx).ok() == Some(parent.as_str()))
            }));
        }
    }
    assert_eq!((owners, methods), (153, 343));
}
#[test]
fn unrelated_missing_or_duplicate_parent_metadata_never_approves_a_child() {
    let mut inputs = corpus();
    let index = inputs[0]
        .classes
        .iter()
        .position(|c| {
            descriptor(&inputs[0], c.class_idx).ok()
                == Some("Landroidx/datastore/preferences/protobuf/r1$b$a;")
        })
        .unwrap();
    let child = inputs[0].classes[index].clone();
    let parent = child.superclass_idx;
    assert_eq!(
        classify(&[], &inputs[0], &child),
        Err(OwnershipError::ParentUnavailable)
    );
    let parent_index = inputs[0]
        .classes
        .iter()
        .position(|c| c.class_idx == parent)
        .unwrap();
    inputs[0].classes[parent_index].access_flags &= !0x4000;
    assert_eq!(
        classify(&inputs, &inputs[0], &child),
        Err(OwnershipError::ParentNotDirectEnum)
    );
    inputs[0].classes[parent_index].access_flags |= 0x4000;
    let dup = inputs[0].classes[parent_index].clone();
    inputs[0].classes.push(dup);
    assert_eq!(
        classify(&inputs, &inputs[0], &child),
        Err(OwnershipError::ParentUnavailable)
    );
}
