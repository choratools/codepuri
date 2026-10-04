//! Exact receiver-static public class declaration selection.
use super::*;
use crate::java_members::external_metadata::ParsedMethod;

/// Resolve once per original caller, static receiver and pool reference. A
/// resolver is tied to one defining DEX and one frozen metadata chain.
pub(super) fn resolve(
    r: &Resolver<'_>,
    reference: MethodRef,
    caller: &str,
    receiver: &JavaType,
) -> Option<JavaMethod> {
    let JavaType::Class(receiver) = receiver else {
        return None;
    };
    // Ordinary zero-contract calls never need a lookup or negative cache entry.
    let symbolic = method_key(r.dex, reference.index())?;
    if r.input_owners.contains(&symbolic.owner)
        || r.declaration(reference.index())
            .ok()?
            .exceptions
            .as_ref()?
            .is_empty()
    {
        return None;
    }
    let cache_key = (caller.to_owned(), receiver.clone(), reference.index());
    if let Some(result) = r.receiver_methods.lock().ok()?.get(&cache_key) {
        return result.clone();
    }
    let result = uncached(r, reference, caller, receiver);
    r.receiver_methods
        .lock()
        .ok()?
        .insert(cache_key, result.clone());
    result
}

fn uncached(
    r: &Resolver<'_>,
    reference: MethodRef,
    caller: &str,
    receiver: &str,
) -> Option<JavaMethod> {
    let symbolic = method_key(r.dex, reference.index())?;
    let original_receiver = r.original_names.get(receiver)?;
    let original_caller = r.original_names.get(caller)?;
    let receiver_descriptor = format!("L{};", original_receiver.replace('.', "/"));
    if receiver_descriptor == symbolic.owner || symbolic.name.starts_with('<') {
        return None;
    }
    let metadata = r.declaration(reference.index()).ok()?;
    if metadata.access_flags & (1 | 2 | 4 | 8) != 1 || metadata.exceptions.as_ref()?.is_empty() {
        return None;
    }
    let external = r.external?;
    let selected = select(&symbolic, &receiver_descriptor, |owner| {
        if r.input_owners.contains(owner) {
            let (dex, class) = r.field_index?.declared_class(owner)?;
            if class.access_flags & 0x200 != 0
                || (class.annotations_off != 0 && class.annotations.is_none())
                || (class.class_data_off != 0 && class.class_data.is_none())
            {
                return None;
            }
            if let Some(set) = class
                .annotations
                .as_ref()
                .and_then(|d| d.class_annotations.as_ref())
            {
                for item in &set.entries {
                    if descriptor(dex, item.annotation.type_idx)? == "Ldalvik/annotation/Signature;"
                    {
                        return None;
                    }
                }
            }
            let methods = class
                .class_data
                .as_ref()
                .into_iter()
                .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
                .map(|m| method_key(dex, m.method_idx))
                .collect::<Option<Vec<_>>>()?;
            let parent = if class.superclass_idx == u32::MAX {
                None
            } else {
                Some(descriptor(dex, class.superclass_idx)?.to_owned())
            };
            let dotted = owner
                .trim_start_matches('L')
                .trim_end_matches(';')
                .replace('/', ".");
            if class.access_flags & 1 == 0
                && dotted.rsplit_once('.').map(|x| x.0)
                    != original_caller.rsplit_once('.').map(|x| x.0)
            {
                return None;
            }
            Some(Class {
                parent,
                methods,
                declarations: None,
            })
        } else {
            if external.class_access(owner)? & (1 | 0x200) != 1
                || external.class_generic_signature(owner)?.is_some()
            {
                return None;
            }
            let hierarchy = external.class_hierarchy(owner);
            let [declaration] = hierarchy.as_slice() else {
                return None;
            };
            if declaration.is_interface
                || format!("L{};", declaration.name.replace('.', "/")) != owner
            {
                return None;
            }
            let declarations = external.declared_methods(owner)?;
            let methods = declarations.iter().map(|m| m.key.clone()).collect();
            Some(Class {
                parent: declaration
                    .superclass
                    .as_ref()
                    .map(|n| format!("L{};", n.replace('.', "/"))),
                methods,
                declarations: Some(declarations),
            })
        }
    })?;
    let mut method = r.method(reference)?;
    // Keep the exact original member spelling and descriptor arguments. An
    // alias that changes lookup cannot use a classpath override declaration.
    if method.name != symbolic.name {
        return None;
    }
    method.owner = r.aliases.class_name(&selected.key.owner)?.to_owned();
    method.throws = selected
        .metadata
        .exceptions
        .as_ref()?
        .iter()
        .map(|d| match r.ty(d)? {
            JavaType::Class(n) => Some(n),
            _ => None,
        })
        .collect::<Option<_>>()?;
    Some(method)
}

struct Class {
    parent: Option<String>,
    methods: Vec<MethodKey>,
    declarations: Option<Vec<ParsedMethod>>,
}

fn select(
    symbolic: &MethodKey,
    receiver: &str,
    mut class: impl FnMut(&str) -> Option<Class>,
) -> Option<ParsedMethod> {
    let mut owner = receiver.to_owned();
    let mut seen = BTreeSet::new();
    let mut selected = None;
    loop {
        if seen.len() >= 4096 || !seen.insert(owner.clone()) {
            return None;
        }
        let current = class(&owner)?;
        let named = current
            .methods
            .iter()
            .filter(|m| m.name == symbolic.name)
            .collect::<Vec<_>>();
        if selected.is_none() && !named.is_empty() {
            let declarations = current.declarations.as_ref()?;
            // Descriptor argument casts pin overload selection. Reject generic,
            // varargs and covariant same-parameter ambiguity, including bridges.
            if declarations
                .iter()
                .filter(|m| m.key.name == symbolic.name)
                .any(|m| m.signature.is_some() || m.metadata.access_flags & (0x40 | 0x80) != 0)
            {
                return None;
            }
            let exact = declarations
                .iter()
                .filter(|m| m.key.name == symbolic.name && m.key.parameters == symbolic.parameters)
                .collect::<Vec<_>>();
            let [target] = exact.as_slice() else {
                return None;
            };
            if target.key.owner != owner
                || target.key.return_type != symbolic.return_type
                || target.metadata.access_flags & (1 | 2 | 4 | 8 | 0x400) != 1
                || target.metadata.exceptions.is_none()
            {
                return None;
            }
            selected = Some((*target).clone());
        }
        if owner == symbolic.owner {
            let exact = named.iter().filter(|m| ***m == *symbolic).count();
            if exact != 1 {
                return None;
            }
            return selected.filter(|m| m.key.owner != symbolic.owner);
        }
        owner = current.parent?;
    }
}

#[cfg(test)]
#[path = "java_receiver_methods_tests.rs"]
mod tests;

// Additional protected retention never invents an input override publication.
// Every intervening class/interface declaration is known and has no competing
// original or frozen source member name. Existing original access checks run
// before this proof is requested.
pub(super) fn protected_ancestor(r: &Resolver<'_>, reference: MethodRef, receiver: &str) -> bool {
    let Some(symbolic) = method_key(r.dex, reference.index()) else {
        return false;
    };
    if r.input_owners.contains(&symbolic.owner) {
        return false;
    }
    let Some(target) = r.method(reference) else {
        return false;
    };
    if target.name != symbolic.name || target.is_static {
        return false;
    }
    let Some(original) = r.original_names.get(receiver) else {
        return false;
    };
    let receiver = format!("L{};", original.replace('.', "/"));
    let Some(external) = r.external else {
        return false;
    };
    let load = |owner: &str| -> Option<ProtectedClass> {
        if r.input_owners.contains(owner) {
            let (dex, c) = r.field_index?.declared_class(owner)?;
            if c.class_data_off != 0 && c.class_data.is_none()
                || c.interfaces_off != 0 && c.interfaces.is_none()
            {
                return None;
            }
            let methods = c
                .class_data
                .as_ref()
                .into_iter()
                .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods));
            let mut named = false;
            for m in methods {
                let key = method_key(dex, m.method_idx)?;
                let alias = r.aliases.method_name(&key)?;
                named |= key.name == symbolic.name
                    || matches!(alias,MethodAlias::Ordinary(n) if n==&target.name);
            }
            Some(ProtectedClass {
                parent: (c.superclass_idx != u32::MAX)
                    .then(|| descriptor(dex, c.superclass_idx).map(str::to_owned))
                    .flatten(),
                interfaces: c
                    .interfaces
                    .as_ref()
                    .into_iter()
                    .flat_map(|t| t.types.iter())
                    .map(|i| descriptor(dex, u32::from(*i)).map(str::to_owned))
                    .collect::<Option<_>>()?,
                named,
                interface: c.access_flags & 0x200 != 0,
                declaration: None,
            })
        } else {
            let hierarchy = external.class_hierarchy(owner);
            let [c] = hierarchy.as_slice() else {
                return None;
            };
            if format!("L{};", c.name.replace('.', "/")) != owner {
                return None;
            }
            let methods = external.declared_methods(owner)?;
            let exact = methods
                .iter()
                .filter(|m| m.key == symbolic)
                .collect::<Vec<_>>();
            let declaration = if owner == symbolic.owner {
                let [m] = exact.as_slice() else {
                    return None;
                };
                if m.signature.is_some()
                    || m.metadata.access_flags & (1 | 2 | 4 | 8 | 0x40 | 0x80) != 4
                    || m.metadata.exceptions.is_none()
                    || methods
                        .iter()
                        .filter(|other| {
                            other.key.name == symbolic.name
                                && other.key.parameters == symbolic.parameters
                        })
                        .count()
                        != 1
                    || methods.iter().any(|other| {
                        other.key.name == symbolic.name
                            && (other.signature.is_some()
                                || other.metadata.access_flags & (0x40 | 0x80) != 0)
                    })
                {
                    return None;
                }
                Some(m.metadata.access_flags)
            } else {
                None
            };
            Some(ProtectedClass {
                parent: c
                    .superclass
                    .as_ref()
                    .map(|n| format!("L{};", n.replace('.', "/"))),
                interfaces: c
                    .interfaces
                    .iter()
                    .map(|n| format!("L{};", n.replace('.', "/")))
                    .collect(),
                named: methods.iter().any(|m| m.key.name == symbolic.name),
                interface: c.is_interface,
                declaration,
            })
        }
    };
    protected_path(&receiver, &symbolic.owner, load)
}
struct ProtectedClass {
    parent: Option<String>,
    interfaces: Vec<String>,
    named: bool,
    interface: bool,
    declaration: Option<u32>,
}
fn protected_path(
    receiver: &str,
    owner: &str,
    mut load: impl FnMut(&str) -> Option<ProtectedClass>,
) -> bool {
    let mut current = receiver.to_owned();
    let mut seen = BTreeSet::new();
    let mut interfaces = Vec::new();
    loop {
        if seen.len() >= 4096 || !seen.insert(current.clone()) {
            return false;
        }
        let Some(c) = load(&current) else {
            return false;
        };
        if c.interface {
            return false;
        }
        interfaces.extend(c.interfaces);
        if current == owner {
            if c.declaration.is_none() {
                return false;
            }
            break;
        }
        if c.named {
            return false;
        }
        let Some(parent) = c.parent else {
            return false;
        };
        current = parent;
    }
    let mut visited = BTreeSet::new();
    let mut active = BTreeSet::new();
    let mut todo = interfaces
        .into_iter()
        .map(|i| (i, false))
        .collect::<Vec<_>>();
    while let Some((owner, finish)) = todo.pop() {
        if finish {
            active.remove(&owner);
            visited.insert(owner);
            continue;
        }
        if active.contains(&owner) {
            return false;
        }
        if visited.contains(&owner) {
            continue;
        }
        if visited.len() + active.len() + seen.len() > 4096 {
            return false;
        }
        let Some(c) = load(&owner) else {
            return false;
        };
        if !c.interface || c.named {
            return false;
        }
        active.insert(owner.clone());
        todo.push((owner, true));
        todo.extend(c.interfaces.into_iter().map(|i| (i, false)));
    }
    true
}
#[cfg(test)]
#[path = "java_protected_receiver_tests.rs"]
mod protected_tests;
