//! Deterministic Java names shared by declarations and resolved references.
//!
//! Collect the complete unit (including referenced external symbols) before
//! calling [`AliasInventory::build`]. Lookups never allocate names: emission
//! order, pool indices and static/instance callsites cannot change an alias.
use std::collections::{BTreeMap, BTreeSet};

/// Unicode mode preserves a conservative subset of Java letters. Characters
/// outside that subset are encoded, rather than guessed to be Java identifiers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AliasMode {
    /// Preserve supported Unicode letters in generated identifiers.
    #[default]
    Unicode,
    /// Encode non-ASCII characters in generated identifiers.
    Ascii,
}

/// Identity includes the return descriptor: DEX permits return-only overloads,
/// whereas Java does not. Staticness deliberately is not part of identity.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct MethodKey {
    /// Original declaring type descriptor.
    pub owner: String,
    /// Original DEX method name, including special JVM names.
    pub name: String,
    /// Parameter descriptors in declaration order, excluding the receiver.
    pub parameters: Vec<String>,
    /// Return descriptor used to distinguish return-only overloads.
    pub return_type: String,
}

/// Original field identity, independent of its DEX pool index.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct FieldKey {
    /// Original declaring type descriptor.
    pub owner: String,
    /// Original DEX field name.
    pub name: String,
    /// Original field type descriptor.
    pub descriptor: String,
}

/// Special JVM method names are never sanitized into ordinary methods.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MethodAlias {
    /// Allocated Java name for an ordinary method.
    Ordinary(String),
    /// Instance initializer, emitted as a constructor.
    Constructor,
    /// Static initializer, emitted as a class initialization block.
    ClassInitializer,
}

/// Original identities, independent of DEX pool numbering.
#[derive(Clone, Debug, Default)]
pub struct AliasInventory {
    classes: BTreeSet<String>,
    methods: BTreeSet<MethodKey>,
    fields: BTreeSet<FieldKey>,
}

impl AliasInventory {
    /// Add a reference descriptor, or a primitive/array descriptor. Referenced
    /// array element classes are included automatically. Invalid descriptors
    /// return an error and do not insert a partial class name.
    pub fn add_type(&mut self, descriptor: &str) -> Result<(), String> {
        let base = descriptor.trim_start_matches('[');
        if descriptor.len() - base.len() > 255 {
            return Err("array rank exceeds 255".into());
        }
        if matches!(base, "Z" | "B" | "S" | "C" | "I" | "J" | "F" | "D")
            || (base == "V" && base == descriptor)
        {
            return Ok(());
        }
        if !base.starts_with('L') || !base.ends_with(';') || base.len() < 3 {
            return Err(format!("invalid type descriptor: {descriptor:?}"));
        }
        let body = &base[1..base.len() - 1];
        if body.contains(';') || body.contains('[') || body.split('/').any(str::is_empty) {
            return Err(format!("invalid class descriptor: {descriptor:?}"));
        }
        self.classes.insert(base.to_owned());
        Ok(())
    }

    /// Validate and add a method and its referenced types transactionally.
    pub fn add_method(&mut self, key: MethodKey) -> Result<(), String> {
        // Validate transactionally, so malformed metadata leaves no partial inventory.
        let mut types = Self::default();
        types.add_owner(&key.owner)?;
        for parameter in &key.parameters {
            if parameter == "V" {
                return Err("void method parameter".into());
            }
            types.add_type(parameter)?;
        }
        types.add_type(&key.return_type)?;
        if matches!(key.name.as_str(), "<init>" | "<clinit>") && key.return_type != "V" {
            return Err("special method must return void".into());
        }
        if key.name == "<clinit>" && !key.parameters.is_empty() {
            return Err("class initializer must have no parameters".into());
        }
        self.classes.extend(types.classes);
        self.methods.insert(key);
        Ok(())
    }

    /// Validate and add a non-void field and its referenced types transactionally.
    pub fn add_field(&mut self, key: FieldKey) -> Result<(), String> {
        let mut types = Self::default();
        types.add_owner(&key.owner)?;
        if key.descriptor == "V" {
            return Err("void field".into());
        }
        types.add_type(&key.descriptor)?;
        self.classes.extend(types.classes);
        self.fields.insert(key);
        Ok(())
    }

    fn add_owner(&mut self, owner: &str) -> Result<(), String> {
        if !owner.starts_with('L') && !owner.starts_with('[') {
            return Err(format!("member owner is not a class: {owner:?}"));
        }
        self.add_type(owner)
    }

    /// Union a fully validated partial inventory. Set insertion is
    /// order-independent, so the merge never changes the collected unit.
    pub(crate) fn append(&mut self, other: AliasInventory) {
        self.classes.extend(other.classes);
        self.methods.extend(other.methods);
        self.fields.extend(other.fields);
    }

    /// Allocate deterministic, collision-free aliases for the collected unit.
    pub fn build(&self, mode: AliasMode) -> JavaAliases {
        self.build_with_shadow_lookup(mode, false)
    }

    fn build_with_shadow_lookup(&self, mode: AliasMode, _legacy_scan: bool) -> JavaAliases {
        let mut result = JavaAliases::default();
        // Packages and classes share a case-insensitive sibling namespace.
        // This also prevents a directory from shadowing a same-named class.
        let mut children: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for descriptor in &self.classes {
            let mut parent = String::new();
            for component in descriptor[1..descriptor.len() - 1].split('/') {
                children
                    .entry(parent.clone())
                    .or_default()
                    .insert(component.into());
                if !parent.is_empty() {
                    parent.push('/');
                }
                parent.push_str(component);
            }
        }
        let mut components = BTreeMap::new();
        for (parent, names) in children {
            for (original, alias) in allocate(names, mode, true) {
                components.insert((parent.clone(), original), alias);
            }
        }
        // A top-level class's simple name shadows a root package even in a
        // fully qualified reference (m0.q0 implements q0.o). Reserve root
        // package names for leaf classes, across the frozen inventory.
        let root_packages: BTreeSet<_> = self
            .classes
            .iter()
            .filter_map(|d| {
                d[1..d.len() - 1]
                    .split_once('/')
                    .map(|(p, _)| components[&(String::new(), p.to_string())].clone())
            })
            .collect();
        // Allocation gives each sibling a unique alias. Index its original
        // component so shadow checks can also exclude the class itself.
        let mut sibling_aliases: BTreeMap<&str, BTreeMap<&str, &str>> = BTreeMap::new();
        for ((parent, original), alias) in &components {
            sibling_aliases
                .entry(parent.as_str())
                .or_default()
                .insert(alias.as_str(), original.as_str());
        }
        let sibling_collision = |key: &(String, String), alias: &str| {
            #[cfg(test)]
            if _legacy_scan {
                return components
                    .iter()
                    .any(|(k, v)| k != key && k.0 == key.0 && v == alias);
            }
            sibling_aliases
                .get(key.0.as_str())
                .and_then(|siblings| siblings.get(alias))
                .is_some_and(|original| *original != key.1)
        };
        let mut class_leaves = BTreeMap::new();
        for descriptor in &self.classes {
            let body = &descriptor[1..descriptor.len() - 1];
            let (parent, leaf) = body.rsplit_once('/').unwrap_or(("", body));
            let key = (parent.to_string(), leaf.to_string());
            if root_packages.contains(&components[&key]) {
                let base = format!("{}$type", components[&key]);
                let mut alias = base.clone();
                let mut suffix = 0;
                while root_packages.contains(&alias) || sibling_collision(&key, &alias) {
                    suffix += 1;
                    alias = format!("{base}${suffix}");
                }
                class_leaves.insert(descriptor.clone(), alias);
            }
        }
        for descriptor in &self.classes {
            let mut parent = String::new();
            let mut aliases = Vec::new();
            let parts: Vec<_> = descriptor[1..descriptor.len() - 1].split('/').collect();
            for (index, component) in parts.iter().enumerate() {
                let alias = if index + 1 == parts.len() {
                    class_leaves.get(descriptor)
                } else {
                    None
                };
                aliases.push(
                    alias
                        .unwrap_or(&components[&(parent.clone(), (*component).into())])
                        .clone(),
                );
                if !parent.is_empty() {
                    parent.push('/');
                }
                parent.push_str(component);
                if index + 1 < parts.len() {
                    result.packages.insert(parent.clone(), aliases.join("."));
                }
            }
            result
                .paths
                .insert(descriptor.clone(), format!("{}.java", aliases.join("/")));
            result.classes.insert(descriptor.clone(), aliases.join("."));
        }

        let mut methods_by_owner: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
        let mut fields_by_owner: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
        let mut method_keys: BTreeMap<&str, Vec<&MethodKey>> = BTreeMap::new();
        let mut field_keys: BTreeMap<&str, Vec<&FieldKey>> = BTreeMap::new();
        for key in &self.methods {
            method_keys.entry(&key.owner).or_default().push(key);
            if !matches!(key.name.as_str(), "<init>" | "<clinit>") {
                methods_by_owner
                    .entry(&key.owner)
                    .or_default()
                    .insert(key.name.clone());
            }
        }
        for key in &self.fields {
            field_keys.entry(&key.owner).or_default().push(key);
            fields_by_owner
                .entry(&key.owner)
                .or_default()
                .insert(key.name.clone());
        }
        for (owner, names) in methods_by_owner {
            let families = allocate(names, mode, false);
            // Reserve every family name before assigning return-only overloads.
            let mut used: BTreeSet<_> = families.values().cloned().collect();
            let mut suffixes = BTreeMap::new();
            let mut signatures = BTreeSet::new();
            for &key in &method_keys[owner] {
                if let Some(base) = families.get(&key.name) {
                    let signature = (base.clone(), key.parameters.clone());
                    let alias = if signatures.insert(signature) {
                        base.clone()
                    } else {
                        unique(base, &mut used, &mut suffixes, false)
                    };
                    result
                        .methods
                        .insert(key.clone(), MethodAlias::Ordinary(alias));
                }
            }
        }
        for key in &self.methods {
            match key.name.as_str() {
                "<init>" => {
                    result.methods.insert(key.clone(), MethodAlias::Constructor);
                }
                "<clinit>" => {
                    result
                        .methods
                        .insert(key.clone(), MethodAlias::ClassInitializer);
                }
                _ => {}
            }
        }
        for (owner, names) in fields_by_owner {
            let families = allocate(names, mode, false);
            let mut used: BTreeSet<_> = families.values().cloned().collect();
            let mut suffixes = BTreeMap::new();
            let mut seen = BTreeSet::new();
            for &key in &field_keys[owner] {
                let base = &families[&key.name];
                let alias = if seen.insert(key.name.clone()) {
                    base.clone()
                } else {
                    unique(base, &mut used, &mut suffixes, false)
                };
                result.fields.insert(key.clone(), alias);
            }
        }
        result
    }
}

/// Frozen whole-unit lookup table. Unknown identities return `None`; callers
/// should report missing inventory rather than silently creating a new alias.
#[derive(Clone, Debug, Default)]
pub struct JavaAliases {
    classes: BTreeMap<String, String>,
    packages: BTreeMap<String, String>,
    paths: BTreeMap<String, String>,
    methods: BTreeMap<MethodKey, MethodAlias>,
    fields: BTreeMap<FieldKey, String>,
    final_bridge_targets: BTreeMap<MethodKey, bridge_families::FinalBridgeAliasProof>,
}

impl JavaAliases {
    /// Use attested classpath member names only for external references. Input
    /// declarations remain flattened units with their original deterministic paths.
    pub fn apply_external_member_names(
        &mut self,
        names: &BTreeMap<String, String>,
        input_owners: &BTreeSet<String>,
    ) {
        for (descriptor, name) in names {
            if !input_owners.contains(descriptor)
                && !descriptor
                    .match_indices('$')
                    .any(|(index, _)| input_owners.contains(&format!("{};", &descriptor[..index])))
                && let Some(alias) = self.classes.get_mut(descriptor)
            {
                *alias = name.clone();
            }
        }
    }
    /// Look up the qualified Java name for an inventoried class descriptor.
    pub fn class_name(&self, descriptor: &str) -> Option<&str> {
        self.classes.get(descriptor).map(String::as_str)
    }

    /// Original slash-separated package, without `L` or a trailing slash.
    pub fn package_name(&self, original: &str) -> Option<&str> {
        if original.is_empty() {
            Some("")
        } else {
            self.packages.get(original).map(String::as_str)
        }
    }

    /// Look up the slash-separated relative `.java` path for a class descriptor.
    pub fn source_path(&self, descriptor: &str) -> Option<&str> {
        self.paths.get(descriptor).map(String::as_str)
    }

    /// Look up an ordinary alias or special-method classification by identity.
    pub fn method_name(&self, key: &MethodKey) -> Option<&MethodAlias> {
        self.methods.get(key)
    }

    /// Look up the allocated Java field name by original identity.
    pub fn field_name(&self, key: &FieldKey) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }

    /// Rewrites a validated descriptor, preserving array dimensions/primitives.
    pub fn type_name(&self, descriptor: &str) -> Option<String> {
        let base = descriptor.trim_start_matches('[');
        let dimensions = descriptor.len() - base.len();
        let name = match base {
            "V" if dimensions == 0 => "void",
            "Z" => "boolean",
            "B" => "byte",
            "S" => "short",
            "C" => "char",
            "I" => "int",
            "J" => "long",
            "F" => "float",
            "D" => "double",
            _ => self.class_name(base)?,
        };
        Some(format!("{name}{}", "[]".repeat(dimensions)))
    }
}

fn allocate(names: BTreeSet<String>, mode: AliasMode, path: bool) -> BTreeMap<String, String> {
    let mut ordered: Vec<_> = names
        .into_iter()
        .map(|name| {
            let base = sanitize(&name, mode, path);
            (base != name, name, base)
        })
        .collect();
    // Keep existing valid names ahead of synthesized names that may resemble them.
    ordered.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let mut used = BTreeSet::new();
    let mut suffixes = BTreeMap::new();
    ordered
        .into_iter()
        .map(|(_, name, base)| {
            let alias = unique(&base, &mut used, &mut suffixes, path);
            (name, alias)
        })
        .collect()
}

fn unique(
    base: &str,
    used: &mut BTreeSet<String>,
    suffixes: &mut BTreeMap<String, u64>,
    path: bool,
) -> String {
    let key = |s: &str| if path { s.to_lowercase() } else { s.to_owned() };
    if used.insert(key(base)) {
        return base.into();
    }
    // Occupancy only grows: previously rejected suffixes cannot become free.
    // Fold the stem too, so case variants share their collision progress.
    let next = suffixes.entry(key(base)).or_insert(2);
    for suffix in *next..=u64::MAX {
        let candidate = format!("{base}__{suffix}");
        *next = suffix.saturating_add(1);
        if used.insert(key(&candidate)) {
            return candidate;
        }
    }
    unreachable!("finite inventory exhausted u64 aliases")
}

fn letter(c: char, mode: AliasMode) -> bool {
    c.is_ascii_alphabetic()
        || c == '_'
        || c == '$'
        || (mode == AliasMode::Unicode
            && c.is_alphabetic()
            && matches!(c as u32, 0x00c0..=0x02ff | 0x0370..=0x052f |
                0x1e00..=0x1fff | 0x3040..=0x30ff | 0x3400..=0x4dbf |
                0x4e00..=0x9fff | 0xac00..=0xd7a3))
}

fn sanitize(original: &str, mode: AliasMode, path: bool) -> String {
    let mut name = String::new();
    for (index, c) in original.chars().enumerate() {
        if letter(c, mode) || (index > 0 && c.is_ascii_digit()) {
            name.push(c);
        } else {
            name.push_str(&format!("_u{:x}_", c as u32));
        }
    }
    if name.is_empty() {
        name.push_str("_empty");
    }
    if keyword(&name) || (path && windows_device(&name)) {
        name.push('_');
    }
    name
}

fn windows_device(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) || (upper.len() == 4
        && (upper.starts_with("COM") || upper.starts_with("LPT"))
        && matches!(upper.as_bytes()[3], b'1'..=b'9'))
}

fn keyword(name: &str) -> bool {
    matches!(
        name,
        "abstract"
            | "assert"
            | "boolean"
            | "break"
            | "byte"
            | "case"
            | "catch"
            | "char"
            | "class"
            | "const"
            | "continue"
            | "default"
            | "do"
            | "double"
            | "else"
            | "enum"
            | "extends"
            | "final"
            | "finally"
            | "float"
            | "for"
            | "goto"
            | "if"
            | "implements"
            | "import"
            | "instanceof"
            | "int"
            | "interface"
            | "long"
            | "native"
            | "new"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "return"
            | "short"
            | "static"
            | "strictfp"
            | "super"
            | "switch"
            | "synchronized"
            | "this"
            | "throw"
            | "throws"
            | "transient"
            | "try"
            | "void"
            | "volatile"
            | "while"
            | "true"
            | "false"
            | "null"
            | "_"
            | "var"
            | "yield"
            | "record"
            | "sealed"
            | "permits"
            | "module"
            | "open"
            | "opens"
            | "exports"
            | "requires"
            | "transitive"
            | "to"
            | "uses"
            | "provides"
            | "with"
            | "when"
    )
}

#[cfg(test)]
mod tests {
    // Fixture construction and successful-result assertions intentionally panic on failure.
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn assert_same_aliases(actual: &JavaAliases, expected: &JavaAliases) {
        assert_eq!(actual.classes, expected.classes);
        assert_eq!(actual.packages, expected.packages);
        assert_eq!(actual.paths, expected.paths);
        assert_eq!(actual.methods, expected.methods);
        assert_eq!(actual.fields, expected.fields);
        assert_eq!(
            format!("{:?}", actual.final_bridge_targets),
            format!("{:?}", expected.final_bridge_targets)
        );
    }

    #[test]
    fn indexed_shadow_lookup_preserves_sibling_suffixes_and_parent_scope() {
        let mut inventory = AliasInventory::default();
        for descriptor in [
            "Lm0/q0;",
            "Lq0/o;",
            "Lm0/q0$type;",
            "Lm0/q0$type$1;",
            "Lm1/q0;",
            "Lm2/q0$type$2;",
            "Lq0$type$2/x;",
            "Lq0$type$3/x;",
            "Lm0/é;",
            "Lé/z;",
            "Lm0/é$type;",
        ] {
            inventory.add_type(descriptor).unwrap();
        }
        for mode in [AliasMode::Ascii, AliasMode::Unicode] {
            assert_same_aliases(
                &inventory.build(mode),
                &inventory.build_with_shadow_lookup(mode, true),
            );
        }
        assert_eq!(
            inventory.build(AliasMode::Unicode).class_name("Lm0/q0;"),
            Some("m0.q0$type$4")
        );
    }

    #[test]
    #[ignore = "local Samsung Pass APK; full alias map parity and build attribution"]
    fn samsung_alias_build_attribution_and_full_map_parity() {
        let files = dex_format::apk::parse_apk_path(
            "/storage_0/decomphify/tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
        )
        .unwrap();
        let mut inventory = AliasInventory::default();
        for dex in &files {
            for ty in &dex.type_ids {
                inventory
                    .add_type(dex.string(ty.descriptor_idx).unwrap())
                    .unwrap();
            }
            let descriptor = |index: u32| {
                dex.string(dex.type_ids[index as usize].descriptor_idx)
                    .unwrap()
                    .to_owned()
            };
            for method in &dex.method_ids {
                let proto = &dex.proto_ids[method.proto_idx as usize];
                inventory
                    .add_method(MethodKey {
                        owner: descriptor(u32::from(method.class_idx)),
                        name: dex.string(method.name_idx).unwrap().into(),
                        parameters: proto
                            .parameters
                            .iter()
                            .flat_map(|p| &p.types)
                            .map(|t| descriptor(u32::from(*t)))
                            .collect(),
                        return_type: descriptor(proto.return_type_idx),
                    })
                    .unwrap();
            }
            for field in &dex.field_ids {
                inventory
                    .add_field(FieldKey {
                        owner: descriptor(u32::from(field.class_idx)),
                        name: dex.string(field.name_idx).unwrap().into(),
                        descriptor: descriptor(u32::from(field.type_idx)),
                    })
                    .unwrap();
            }
        }
        for run in 0..3 {
            let started = std::time::Instant::now();
            let expected = inventory.build_with_shadow_lookup(AliasMode::Ascii, true);
            let baseline = started.elapsed();
            let started = std::time::Instant::now();
            let actual = inventory.build(AliasMode::Ascii);
            let indexed = started.elapsed();
            assert_same_aliases(&actual, &expected);
            eprintln!(
                "alias_build run={run} baseline={baseline:?} indexed={indexed:?} classes={} methods={} fields={} parity=all_maps",
                actual.classes.len(),
                actual.methods.len(),
                actual.fields.len()
            );
        }
    }

    #[test]
    fn class_simple_names_cannot_shadow_root_packages_java17() {
        let mut inventory = AliasInventory::default();
        for d in ["Lm0/q0;", "Lq0/o;", "Lm0/q0$type;"] {
            inventory.add_type(d).unwrap();
        }
        let aliases = inventory.build(AliasMode::Unicode);
        assert_eq!(aliases.class_name("Lq0/o;"), Some("q0.o"));
        let owner = aliases.class_name("Lm0/q0;").unwrap();
        assert_ne!(owner, "m0.q0");
        assert_ne!(owner, aliases.class_name("Lm0/q0$type;").unwrap());
        let root =
            std::env::temp_dir().join(format!("decomp-package-shadow-{}", std::process::id()));
        std::fs::create_dir_all(root.join("m0")).unwrap();
        std::fs::create_dir_all(root.join("q0")).unwrap();
        let source = root.join(format!("{}.java", owner.replace('.', "/")));
        std::fs::write(
            &source,
            format!(
                "package m0; public abstract class {} implements q0.o {{}}",
                owner.rsplit('.').next().unwrap()
            ),
        )
        .unwrap();
        let dependency = root.join("q0/o.java");
        std::fs::write(&dependency, "package q0; public interface o {}").unwrap();
        let result = std::process::Command::new("javac")
            .arg("--release")
            .arg("17")
            .arg("-d")
            .arg(&root)
            .arg(&source)
            .arg(&dependency)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn legacy_unique(base: &str, used: &mut BTreeSet<String>, path: bool) -> String {
        for suffix in 1u64.. {
            let candidate = if suffix == 1 {
                base.into()
            } else {
                format!("{base}__{suffix}")
            };
            let key = if path {
                candidate.to_lowercase()
            } else {
                candidate.clone()
            };
            if used.insert(key) {
                return candidate;
            }
        }
        unreachable!()
    }

    #[test]
    fn suffix_progress_is_shared_by_casefolded_stems() {
        let mut used: BTreeSet<String> = ["a", "a__2", "a__4"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let mut suffixes = BTreeMap::new();
        assert_eq!(unique("A", &mut used, &mut suffixes, true), "A__3");
        for suffix in 5..4096 {
            let stem = if suffix % 2 == 0 { "A" } else { "a" };
            assert_eq!(
                unique(stem, &mut used, &mut suffixes, true),
                format!("{stem}__{suffix}")
            );
            assert_eq!(suffixes["a"], suffix + 1);
        }
        assert_eq!(suffixes.len(), 1);
    }

    fn legacy_allocate(
        names: BTreeSet<String>,
        mode: AliasMode,
        path: bool,
    ) -> BTreeMap<String, String> {
        let mut names: Vec<_> = names.into_iter().collect();
        names.sort_by_key(|name| (sanitize(name, mode, path) != *name, name.clone()));
        let mut used = BTreeSet::new();
        names
            .into_iter()
            .map(|name| {
                let alias = legacy_unique(&sanitize(&name, mode, path), &mut used, path);
                (name, alias)
            })
            .collect()
    }

    #[test]
    fn randomized_inventory_matches_legacy_aliases() {
        let names = [
            "A",
            "a",
            "A__2",
            "a__3",
            "CON",
            "con",
            "CON_",
            "CON___2",
            "LPT1",
            "lpt1",
            "LPT1_",
            "α",
            "Α",
            "İ",
            "i",
            "漢",
            "💩",
            "_u1f4a9_",
            "class",
            "class_",
            "class___2",
            "a-b",
            "a_u2d_b",
            "",
            "9x",
        ];
        let mut state = 42u64;
        for _ in 0..32 {
            let mut order: Vec<_> = (0..names.len()).collect();
            for i in (1..order.len()).rev() {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                order.swap(i, (state as usize) % (i + 1));
            }
            let mut inventory = AliasInventory::default();
            for &i in &order {
                // Empty member names are valid inventory entries, but not class components.
                if !names[i].is_empty() {
                    inventory.add_type(&format!("Lp/{};", names[i])).unwrap();
                }
                for ret in ["I", "J", "F", "D", "V"] {
                    for parameters in [&[][..], &["I"][..]] {
                        inventory
                            .add_method(method(names[i], parameters, ret))
                            .unwrap();
                    }
                    if ret != "V" {
                        inventory
                            .add_field(FieldKey {
                                owner: "Lp/C;".into(),
                                name: names[i].into(),
                                descriptor: ret.into(),
                            })
                            .unwrap();
                    }
                }
            }
            for mode in [AliasMode::Ascii, AliasMode::Unicode] {
                let aliases = inventory.build(mode);
                let classes = inventory
                    .classes
                    .iter()
                    .map(|d| d[3..d.len() - 1].to_owned())
                    .collect();
                for (name, alias) in legacy_allocate(classes, mode, true) {
                    assert_eq!(
                        aliases.class_name(&format!("Lp/{name};")),
                        Some(format!("p.{alias}").as_str())
                    );
                    assert_eq!(
                        aliases.source_path(&format!("Lp/{name};")),
                        Some(format!("p/{alias}.java").as_str())
                    );
                }
                let families =
                    legacy_allocate(names.iter().map(|s| (*s).into()).collect(), mode, false);
                let mut used: BTreeSet<_> = families.values().cloned().collect();
                let mut signatures = BTreeSet::new();
                for key in &inventory.methods {
                    let base = &families[&key.name];
                    let expected = if signatures.insert((base.clone(), key.parameters.clone())) {
                        base.clone()
                    } else {
                        legacy_unique(base, &mut used, false)
                    };
                    assert_eq!(
                        aliases.method_name(key),
                        Some(&MethodAlias::Ordinary(expected))
                    );
                }
                let mut used: BTreeSet<_> = families.values().cloned().collect();
                let mut seen = BTreeSet::new();
                for key in &inventory.fields {
                    let base = &families[&key.name];
                    let expected = if seen.insert(key.name.clone()) {
                        base.clone()
                    } else {
                        legacy_unique(base, &mut used, false)
                    };
                    assert_eq!(aliases.field_name(key), Some(expected.as_str()));
                }
            }
        }
    }

    fn method(name: &str, parameters: &[&str], ret: &str) -> MethodKey {
        MethodKey {
            owner: "Lp/C;".into(),
            name: name.into(),
            parameters: parameters.iter().map(|s| (*s).into()).collect(),
            return_type: ret.into(),
        }
    }

    #[test]
    fn overloads_and_return_only_collisions_are_stable() {
        let keys = [
            method("run", &["I"], "V"),
            method("run", &["J"], "V"),
            method("run", &["I"], "I"),
            method("run__2", &[], "V"),
            method("a-b", &[], "V"),
            method("a_u2d_b", &[], "V"),
        ];
        let mut a = AliasInventory::default();
        let mut b = AliasInventory::default();
        for key in &keys {
            a.add_method(key.clone()).unwrap();
        }
        for key in keys.iter().rev() {
            b.add_method(key.clone()).unwrap();
        }
        let a = a.build(AliasMode::Unicode);
        let b = b.build(AliasMode::Unicode);
        for key in &keys {
            assert_eq!(a.method_name(key), b.method_name(key));
        }
        assert_ne!(a.method_name(&keys[0]), a.method_name(&keys[2]));
        assert_eq!(a.method_name(&keys[1]), a.method_name(&keys[2]));
        assert_ne!(a.method_name(&keys[0]), a.method_name(&keys[3]));
        assert_ne!(a.method_name(&keys[4]), a.method_name(&keys[5]));
    }

    #[test]
    fn bad_names_unicode_keywords_and_special_methods() {
        let mut inventory = AliasInventory::default();
        for name in ["class", "", "9x", "a.b", "a\\b", "α漢", "💩"] {
            inventory.add_method(method(name, &[], "V")).unwrap();
        }
        let init = method("<init>", &["I"], "V");
        let clinit = method("<clinit>", &[], "V");
        inventory.add_method(init.clone()).unwrap();
        inventory.add_method(clinit.clone()).unwrap();
        let unicode = inventory.build(AliasMode::Unicode);
        let ascii = inventory.build(AliasMode::Ascii);
        assert_eq!(unicode.method_name(&init), Some(&MethodAlias::Constructor));
        assert_eq!(
            unicode.method_name(&clinit),
            Some(&MethodAlias::ClassInitializer)
        );
        assert_eq!(
            unicode.method_name(&method("α漢", &[], "V")),
            Some(&MethodAlias::Ordinary("α漢".into()))
        );
        for (key, alias) in &ascii.methods {
            if let MethodAlias::Ordinary(name) = alias {
                assert!(name.is_ascii());
                assert!(!keyword(name));
                assert!(
                    name.chars()
                        .enumerate()
                        .all(|(i, c)| letter(c, AliasMode::Ascii) || (i > 0 && c.is_ascii_digit())),
                    "{key:?}: {name}"
                );
            }
        }
    }

    #[test]
    fn sanitized_paths_are_safe_and_case_insensitively_unique() {
        let descriptors = [
            "Lclass/CON;",
            "Lclass/con;",
            "Lclass/CON_;",
            "Lp/a.b;",
            "Lp/a_u2e_b;",
            "Lp/../C;",
            "Lp/A;",
            "Lp/a;",
            "Lp/A/Child;",
            "LPRN/NUL;",
        ];
        let mut inventory = AliasInventory::default();
        for d in descriptors {
            inventory.add_type(d).unwrap();
        }
        let aliases = inventory.build(AliasMode::Ascii);
        let mut paths = BTreeSet::new();
        for d in descriptors {
            let path = aliases.source_path(d).unwrap();
            assert!(paths.insert(path.to_lowercase()), "{path}");
            assert!(!path.contains('\\'));
            for part in path.strip_suffix(".java").unwrap().split('/') {
                assert!(!windows_device(part));
                assert!(!keyword(part));
                assert!(!part.contains('.'));
            }
            assert_eq!(
                aliases.class_name(d).unwrap().replace('.', "/") + ".java",
                path
            );
        }
        assert_eq!(aliases.package_name("class"), Some("class_"));
        assert_eq!(
            aliases.type_name("[[Lp/A;"),
            Some(format!("{}[][]", aliases.class_name("Lp/A;").unwrap()))
        );
    }

    #[test]
    fn field_identity_and_reference_lookup() {
        let mut inventory = AliasInventory::default();
        let keys: Vec<_> = ["I", "J"]
            .into_iter()
            .map(|descriptor| FieldKey {
                owner: "Lp/C;".into(),
                name: "bad-name".into(),
                descriptor: descriptor.into(),
            })
            .collect();
        for key in &keys {
            inventory.add_field(key.clone()).unwrap();
        }
        let aliases = inventory.build(AliasMode::Ascii);
        assert_ne!(aliases.field_name(&keys[0]), aliases.field_name(&keys[1]));
        // Freshly reconstructed reference keys hit exactly the declaration map.
        assert_eq!(
            aliases.field_name(&keys[0]),
            aliases.field_name(&keys[0].clone())
        );
        assert!(aliases.class_name("Lp/Missing;").is_none());
        assert!(aliases.method_name(&method("missing", &[], "V")).is_none());
    }

    #[test]
    fn invalid_inventory_is_rejected_transactionally() {
        let mut inventory = AliasInventory::default();
        for d in ["", "[V", "L;", "Lp//C;", "Lp/C;;"] {
            assert!(inventory.add_type(d).is_err());
        }
        assert!(inventory.add_method(method("bad", &["V"], "V")).is_err());
        assert!(inventory.add_method(method("<init>", &[], "I")).is_err());
        assert!(
            inventory
                .add_method(method("<clinit>", &["I"], "V"))
                .is_err()
        );
        assert!(inventory.classes.is_empty());
    }
}

#[path = "java_aliases_bridge.rs"]
mod bridge_families;
pub use bridge_families::BridgeAliasReport;
pub use bridge_families::FinalBridgeAliasProof;
impl JavaAliases {
    /// Restore the shared Java spelling of uniquely proven override obligations.
    /// All original DEX identities and bodies remain present.
    pub fn align_override_obligations(
        &mut self,
        files: &[dex_format::DexFile],
        external: &dyn crate::java_external_metadata::ExternalMetadata,
        hierarchy: &crate::type_compatibility::Hierarchy,
    ) -> BridgeAliasReport {
        bridge_families::align_obligations(self, files, external, hierarchy)
    }
    /// Align proved input-only covariant bridge families without changing any
    /// original descriptor, declaration flags, body, or exception contract.
    pub fn align_final_bridge_targets(
        &mut self,
        files: &[dex_format::DexFile],
        external: &dyn crate::java_external_metadata::ExternalMetadata,
        hierarchy: &crate::type_compatibility::Hierarchy,
    ) -> BridgeAliasReport {
        bridge_families::align(self, files, external, hierarchy)
    }
    /// Return read-only final-target source evidence only while every proven
    /// alias binding remains unchanged; this is not body/throws publication.
    pub fn final_bridge_alias(&self, reference: &MethodKey) -> Option<FinalBridgeAliasProof> {
        let proof = self.final_bridge_targets.get(reference)?;
        proof.still_matches(self).then(|| proof.clone())
    }
}
