//! Bounded Signature syntax/erasure evidence for exact super dispatch.
use super::MethodKey;

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
    depth: usize,
    bounds: Option<std::collections::BTreeMap<String, String>>,
    variables: std::collections::BTreeSet<String>,
}
impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }
    fn take(&mut self, byte: u8) -> Option<()> {
        (self.peek()? == byte).then(|| self.at += 1)
    }
    fn identifier(&mut self) -> Option<String> {
        let start = self.at;
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$'))
        {
            self.at += 1;
        }
        if self.at == start || self.bytes[start].is_ascii_digit() {
            return None;
        }
        String::from_utf8(self.bytes[start..self.at].to_vec()).ok()
    }
    fn ty(&mut self, allow_void: bool) -> Option<(String, bool, bool)> {
        if self.depth >= 32 {
            return None;
        }
        self.depth += 1;
        let result = self.ty_inner(allow_void);
        self.depth -= 1;
        result
    }
    fn ty_inner(&mut self, allow_void: bool) -> Option<(String, bool, bool)> {
        match self.peek()? {
            b'[' => {
                self.at += 1;
                let (t, v, p) = self.ty(false)?;
                Some((format!("[{t}"), v, p))
            }
            b'T' => {
                self.at += 1;
                let name = self.identifier()?;
                self.take(b';')?;
                self.variables.insert(name.clone());
                let erased = match &self.bounds {
                    Some(bounds) => bounds.get(&name)?.clone(),
                    None => String::new(),
                };
                Some((erased, true, false))
            }
            b'L' => {
                self.at += 1;
                let mut name = String::new();
                let mut parameterized = false;
                loop {
                    name.push_str(&self.identifier()?);
                    if self.peek() == Some(b'<') {
                        parameterized = true;
                        self.at += 1;
                        let mut count = 0;
                        while self.peek() != Some(b'>') {
                            count += 1;
                            if count > 128 {
                                return None;
                            }
                            if self.peek() == Some(b'*') {
                                self.at += 1;
                                continue;
                            }
                            if matches!(self.peek(), Some(b'+' | b'-')) {
                                self.at += 1;
                            }
                            if !matches!(self.peek(), Some(b'L' | b'T' | b'[')) {
                                return None;
                            }
                            self.ty(false)?;
                        }
                        if count == 0 {
                            return None;
                        }
                        self.at += 1;
                    }
                    match self.peek()? {
                        b'/' => {
                            if parameterized {
                                return None;
                            }
                            self.at += 1;
                            name.push('/');
                        }
                        b'.' => {
                            self.at += 1;
                            name.push('$');
                        }
                        b';' => {
                            self.at += 1;
                            break;
                        }
                        _ => return None,
                    }
                }
                Some((format!("L{name};"), false, parameterized))
            }
            b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z' => {
                let c = self.peek()?;
                self.at += 1;
                Some(((c as char).to_string(), false, false))
            }
            b'V' if allow_void => {
                self.at += 1;
                Some(("V".into(), false, false))
            }
            _ => None,
        }
    }
}
fn parser(s: &str) -> Option<Parser<'_>> {
    (s.len() <= 16384 && s.is_ascii()).then_some(Parser {
        bytes: s.as_bytes(),
        at: 0,
        depth: 0,
        bounds: None,
        variables: std::collections::BTreeSet::new(),
    })
}
/// Exact descriptor erasure; true means a top-level class variable needs raw context.
#[cfg(test)]
pub(super) fn method(signature: &str, key: &MethodKey) -> Option<bool> {
    method_inner(signature, key, None)
}
pub(super) fn checked_method(
    signature: &str,
    key: &MethodKey,
    class: Option<&str>,
) -> Option<bool> {
    let bounds = match class {
        Some(s) => self::class(s)?.bounds,
        None => std::collections::BTreeMap::new(),
    };
    method_inner(signature, key, Some(bounds))
}
fn method_inner(
    signature: &str,
    key: &MethodKey,
    bounds: Option<std::collections::BTreeMap<String, String>>,
) -> Option<bool> {
    let mut p = parser(signature)?;
    p.bounds = bounds;
    p.take(b'(')?;
    let mut variable = false;
    for expected in &key.parameters {
        let (actual, v, _) = p.ty(false)?;
        if v {
            if !(actual == *expected || reference_descriptor(&actual, expected)) {
                return None;
            }
        } else if actual != *expected {
            return None;
        }
        variable |= v;
    }
    p.take(b')')?;
    let (actual, v, _) = p.ty(true)?;
    if v {
        if !(actual == key.return_type || reference_descriptor(&actual, &key.return_type)) {
            return None;
        }
    } else if actual != key.return_type {
        return None;
    }
    variable |= v;
    (p.at == p.bytes.len()).then_some(variable)
}
fn reference_descriptor(prefix: &str, s: &str) -> bool {
    let Some(tail) = s.strip_prefix(prefix) else {
        return false;
    };
    tail.starts_with('L') && tail.ends_with(';')
}
/// Genuine class declaration and direct superclass source spelling.
pub(super) struct ClassSignature {
    pub generic: bool,
    pub parent: String,
    pub parent_parameterized: bool,
    pub bounds: std::collections::BTreeMap<String, String>,
}
pub(super) fn class(signature: &str) -> Option<ClassSignature> {
    let mut p = parser(signature)?;
    let generic = p.peek() == Some(b'<');
    let mut bounds = std::collections::BTreeMap::new();
    if generic {
        p.at += 1;
        let mut names = std::collections::BTreeSet::new();
        while p.peek() != Some(b'>') {
            let name = p.identifier()?;
            if names.len() >= 128 || !names.insert(name.clone()) {
                return None;
            }
            p.take(b':')?;
            let mut bound = None;
            if p.peek() != Some(b':') {
                if !matches!(p.peek(), Some(b'L' | b'[')) {
                    return None;
                }
                let (t, v, _) = p.ty(false)?;
                if v {
                    return None;
                }
                bound = Some(t);
            }
            while p.peek() == Some(b':') {
                p.at += 1;
                if !matches!(p.peek(), Some(b'L')) {
                    return None;
                }
                let (t, v, _) = p.ty(false)?;
                if v {
                    return None;
                }
                if bound.is_none() {
                    bound = Some(t);
                }
            }
            bounds.insert(name, bound?);
        }
        if names.is_empty() {
            return None;
        }
        p.at += 1;
    }
    if p.variables.iter().any(|v| !bounds.contains_key(v)) {
        return None;
    }
    p.bounds = Some(bounds.clone());
    if p.peek() != Some(b'L') {
        return None;
    }
    let (parent, v, args) = p.ty(false)?;
    if v {
        return None;
    }
    while p.at < p.bytes.len() {
        if p.peek() != Some(b'L') {
            return None;
        }
        p.ty(false)?;
    }
    Some(ClassSignature {
        generic,
        parent,
        parent_parameterized: args,
        bounds,
    })
}
