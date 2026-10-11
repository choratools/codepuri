//! Read-only APK method inventory for the SQLite comparison research harness.
use dex_decompiler::decode::{Opcode, SemanticClass, decode_method};
use dex_format::DexFile;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{Read, Write};

fn json(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            ch if ch <= '\u{1f}' => {
                let _ = write!(out, "\\u{:04x}", ch as u32);
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}
fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}
fn ty(dex: &DexFile, index: u32) -> Result<&str, dex_format::DexError> {
    dex.string(dex.type_id(index)?.descriptor_idx)
}
fn proto(dex: &DexFile, index: u32) -> Result<String, dex_format::DexError> {
    let p = dex.proto_id(index)?;
    let mut text = String::from("(");
    if let Some(parameters) = &p.parameters {
        for parameter in &parameters.types {
            text.push_str(ty(dex, u32::from(*parameter))?);
        }
    }
    text.push(')');
    text.push_str(ty(dex, p.return_type_idx)?);
    Ok(text)
}
fn method(dex: &DexFile, index: u32) -> Result<(String, String, String), dex_format::DexError> {
    let m = dex.method_id(index)?;
    Ok((
        ty(dex, u32::from(m.class_idx))?.into(),
        dex.string(m.name_idx)?.into(),
        proto(dex, u32::from(m.proto_idx))?,
    ))
}
fn reference(dex: &DexFile, opcode: u8, index: u32) -> Option<String> {
    match opcode {
        0x1a | 0x1b => {
            let span = dex.strings.get(index as usize)?.span;
            Some(format!(
                "string-mutf8:{}",
                hex(dex
                    .raw_bytes()
                    .get(span.offset as usize..(span.offset + span.size) as usize)?)
            ))
        }
        0x1c | 0x1f | 0x20 | 0x22..=0x25 => Some(format!("type:{}", ty(dex, index).ok()?)),
        0x52..=0x6d => {
            let f = dex.field_id(index).ok()?;
            Some(format!(
                "field:{}->{}:{}",
                ty(dex, u32::from(f.class_idx)).ok()?,
                dex.string(f.name_idx).ok()?,
                ty(dex, u32::from(f.type_idx)).ok()?
            ))
        }
        0x6e..=0x72 | 0x74..=0x78 | 0xfa | 0xfb => {
            let (owner, name, signature) = method(dex, index).ok()?;
            Some(format!("method:{owner}->{name}{signature}"))
        }
        0xff => Some(format!("proto:{}", proto(dex, index).ok()?)),
        // Call sites and method handles require their own symbolic serialization.
        _ => None,
    }
}
fn root_order(name: &str) -> Option<u32> {
    if name == "classes.dex" {
        return Some(1);
    }
    let number = name.strip_prefix("classes")?.strip_suffix(".dex")?;
    if number.is_empty() || !number.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    number.parse::<u32>().ok().filter(|n| *n >= 2)
}

fn disassembly(
    dex: &DexFile,
    code: &dex_format::code::CodeItem,
    instructions: &[dex_decompiler::decode::DecodedInsn],
) -> (String, Vec<String>) {
    let mut text = format!(
        "# DEX code-unit offsets; registers={} incoming_words={} outgoing_words={}\n",
        code.registers_size, code.ins_size, code.outs_size
    );
    let mut calls = Vec::new();
    for (position, instruction) in instructions.iter().enumerate() {
        match instruction.opcode {
            Opcode::Supported(info) => {
                let symbol = instruction
                    .pool_index
                    .and_then(|index| reference(dex, info.opcode, index));
                let target = instruction
                    .branch_offset
                    .map(|relative| i64::from(instruction.offset) + i64::from(relative));
                let _ = writeln!(
                    text,
                    "{:06x}: {} registers={:?} literal={:?} pool={:?} reference={:?} branch_target={:?} payload={:?}",
                    instruction.offset,
                    info.name,
                    instruction.registers,
                    instruction.literal,
                    instruction.pool_index,
                    symbol,
                    target,
                    instruction.payload_offset
                );
                if matches!(info.opcode, 0x1a | 0x1b)
                    && let Some(index) = instruction.pool_index
                    && let Ok(value) = dex.string(index)
                {
                    let _ = writeln!(text, "          string_scalar_view={value:?}");
                }
                if let Some(index) = instruction.secondary_pool_index {
                    let _ = writeln!(
                        text,
                        "          secondary_proto={:?}",
                        proto(dex, index).ok()
                    );
                }
                if info.class == SemanticClass::Invoke {
                    let called = symbol
                        .as_deref()
                        .and_then(|symbol| symbol.strip_prefix("method:"));
                    calls.push(format!(
                        "{{\"offset\":{},\"opcode\":{},\"target\":{}}}",
                        instruction.offset,
                        json(info.name),
                        called.map_or("null".into(), json)
                    ));
                }
            }
            Opcode::Payload { ident } => {
                let end = instructions
                    .get(position + 1)
                    .map_or(code.insns_size, |next| next.offset);
                let raw = code
                    .insns
                    .get(instruction.offset as usize * 2..end as usize * 2)
                    .unwrap_or_default();
                let _ = writeln!(
                    text,
                    "{:06x}: payload ident=0x{ident:04x} raw={}",
                    instruction.offset,
                    hex(raw)
                );
            }
            Opcode::Unsupported { opcode } => {
                let _ = writeln!(
                    text,
                    "{:06x}: unsupported_opcode=0x{opcode:02x}",
                    instruction.offset
                );
            }
        }
    }
    for range in &code.tries {
        let _ = writeln!(
            text,
            "try {:06x}..{:06x} handler=@{:x}",
            range.start_addr,
            u64::from(range.start_addr) + u64::from(range.insn_count),
            range.handler_off
        );
    }
    for handler in &code.handlers {
        for catch in &handler.handlers {
            let _ = writeln!(
                text,
                "handler @{:x} catch {:?} -> {:06x}",
                handler.relative_offset,
                ty(dex, catch.type_idx).ok(),
                catch.address
            );
        }
        if let Some(address) = handler.catch_all_addr {
            let _ = writeln!(
                text,
                "handler @{:x} catch-all -> {:06x}",
                handler.relative_offset, address
            );
        }
    }
    (text, calls)
}

#[cfg(test)]
mod evidence_tests {
    use super::*;

    fn code(words: &[u16]) -> dex_format::code::CodeItem {
        dex_format::code::CodeItem {
            registers_size: 1,
            ins_size: 0,
            outs_size: 0,
            debug_info_off: 0,
            insns_size: words.len() as u32,
            insns: words.iter().flat_map(|w| w.to_le_bytes()).collect(),
            tries: vec![],
            handlers: vec![],
            span: dex_format::RawSpan { offset: 0, size: 0 },
        }
    }

    #[test]
    fn evidence_keeps_code_unit_offsets_and_exact_symbolic_invocation()
    -> Result<(), Box<dyn std::error::Error>> {
        let dex = DexFile::from_bytes(include_bytes!(
            "../../../tests/corpus/dex/clean/classes.dex"
        ))?;
        let body = code(&[0x0012, 0x0071, 0, 0, 0x000e]);
        let decoded = decode_method(&body)?;
        let (text, calls) = disassembly(&dex, &body, &decoded);
        let (owner, name, signature) = method(&dex, 0)?;
        assert!(text.contains("000001: invoke-static"));
        assert!(text.contains("000004: return-void"));
        assert_eq!(
            calls,
            vec![format!(
                "{{\"offset\":1,\"opcode\":\"invoke-static\",\"target\":{}}}",
                json(&format!("{owner}->{name}{signature}"))
            )]
        );
        Ok(())
    }

    #[test]
    fn evidence_keeps_branch_target_without_executing_code()
    -> Result<(), Box<dyn std::error::Error>> {
        let dex = DexFile::from_bytes(include_bytes!(
            "../../../tests/corpus/dex/clean/classes.dex"
        ))?;
        let body = code(&[0x0128, 0x000e]);
        let decoded = decode_method(&body)?;
        let (text, calls) = disassembly(&dex, &body, &decoded);
        assert!(text.contains("branch_target=Some(1)"));
        assert!(calls.is_empty());
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: method_inventory APK [--disassembly]")?;
    let mut include_disassembly = false;
    for option in args {
        match option.as_str() {
            "--disassembly" => include_disassembly = true,
            _ => return Err(format!("unknown option: {option}").into()),
        }
    }
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    let mut names = Vec::new();
    for i in 0..zip.len() {
        let name = zip.by_index(i)?.name().to_owned();
        if name.ends_with(".dex") {
            names.push(name);
        }
    }
    names.sort_by_key(|name| (root_order(name).unwrap_or(u32::MAX), name.clone()));
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    let mut root_pool = 0;
    for entry in names {
        let mut bytes = Vec::new();
        zip.by_name(&entry)?.read_to_end(&mut bytes)?;
        let root = root_order(&entry).is_some();
        let files = match DexFile::from_container(&bytes) {
            Ok(files) => files,
            Err(error) => {
                writeln!(
                    output,
                    "{{\"kind\":\"dex\",\"entry\":{},\"bytes\":{},\"root\":{},\"parse_error\":{}}}",
                    json(&entry),
                    bytes.len(),
                    root,
                    json(&error.to_string())
                )?;
                continue;
            }
        };
        for (logical, dex) in files.iter().enumerate() {
            let pool = if root {
                let p = root_pool;
                root_pool += 1;
                p.to_string()
            } else {
                "null".into()
            };
            writeln!(
                output,
                "{{\"kind\":\"dex\",\"entry\":{},\"bytes\":{},\"root\":{},\"logical\":{},\"pool\":{},\"classes\":{},\"defined_methods\":{}}}",
                json(&entry),
                bytes.len(),
                root,
                logical,
                pool,
                dex.classes.len(),
                dex.classes
                    .iter()
                    .filter_map(|c| c.class_data.as_ref())
                    .map(|d| d.direct_methods.len() + d.virtual_methods.len())
                    .sum::<usize>()
            )?;
            for class in &dex.classes {
                let Some(data) = &class.class_data else {
                    continue;
                };
                for m in data.direct_methods.iter().chain(&data.virtual_methods) {
                    let (owner, name, signature) = method(dex, m.method_idx)?;
                    let identity = format!("{owner}->{name}{signature}");
                    let mut features = BTreeMap::<&str, usize>::new();
                    let mut error = None;
                    let mut canonical = None;
                    let mut count = None;
                    let mut evidence = None;
                    let mut calls = Vec::new();
                    if let Some(code) = &m.code {
                        match decode_method(code) {
                            Err(e) => error = Some(e.to_string()),
                            Ok(insns) => {
                                if include_disassembly {
                                    let (text, symbolic_calls) = disassembly(dex, code, &insns);
                                    evidence = Some(text);
                                    calls = symbolic_calls;
                                }
                                count = Some(
                                    insns
                                        .iter()
                                        .filter(|i| !matches!(i.opcode, Opcode::Payload { .. }))
                                        .count(),
                                );
                                let mut masked = code.insns.clone();
                                let mut refs = Vec::new();
                                let mut complete = true;
                                for insn in &insns {
                                    let Opcode::Supported(info) = insn.opcode else {
                                        continue;
                                    };
                                    let family = match info.class {
                                        SemanticClass::Invoke => "invokes",
                                        SemanticClass::New => "allocations",
                                        SemanticClass::Branch => "branches",
                                        SemanticClass::Switch => "switches",
                                        SemanticClass::Field => "fields",
                                        SemanticClass::Array => "arrays",
                                        _ => "other",
                                    };
                                    *features.entry(family).or_default() += 1;
                                    if let Some(index) = insn.pool_index {
                                        let start = insn.offset as usize * 2 + 2;
                                        let width = if info.opcode == 0x1b { 4 } else { 2 };
                                        if let Some(symbol) = reference(dex, info.opcode, index) {
                                            refs.push(format!("{}:{}", insn.offset, symbol));
                                            if let Some(part) = masked.get_mut(start..start + width)
                                            {
                                                part.fill(0);
                                            } else {
                                                complete = false;
                                            }
                                        } else {
                                            complete = false;
                                        }
                                    }
                                    if let Some(index) = insn.secondary_pool_index {
                                        match proto(dex, index) {
                                            Ok(p) => {
                                                refs.push(format!("{}:secondary:{p}", insn.offset));
                                                let start = insn.offset as usize * 2 + 6;
                                                if let Some(part) = masked.get_mut(start..start + 2)
                                                {
                                                    part.fill(0);
                                                } else {
                                                    complete = false;
                                                }
                                            }
                                            Err(_) => complete = false,
                                        }
                                    }
                                }
                                let handlers: Option<Vec<_>> = code
                                    .handlers
                                    .iter()
                                    .map(|h| {
                                        let pairs: Option<Vec<_>> = h
                                            .handlers
                                            .iter()
                                            .map(|p| {
                                                Some((
                                                    ty(dex, p.type_idx).ok()?.to_owned(),
                                                    p.address,
                                                ))
                                            })
                                            .collect();
                                        Some((h.relative_offset, pairs?, h.catch_all_addr))
                                    })
                                    .collect();
                                if complete && let Some(handlers) = handlers {
                                    canonical = Some(format!(
                                        "symbolic-pools-v1|{}|{}|{}|{}|{:?}|{:?}|{:?}",
                                        code.registers_size,
                                        code.ins_size,
                                        code.outs_size,
                                        hex(&masked),
                                        refs,
                                        code.tries
                                            .iter()
                                            .map(|t| (t.start_addr, t.insn_count, t.handler_off))
                                            .collect::<Vec<_>>(),
                                        handlers
                                    ));
                                }
                            }
                        }
                    }
                    let features = features
                        .iter()
                        .map(|(k, v)| format!("{}:{v}", json(k)))
                        .collect::<Vec<_>>()
                        .join(",");
                    writeln!(
                        output,
                        "{{\"kind\":\"method\",\"entry\":{},\"logical\":{},\"pool\":{},\"method_index\":{},\"identity\":{},\"owner\":{},\"name\":{},\"signature\":{},\"class_flags\":{},\"flags\":{},\"code_offset\":{},\"code_units\":{},\"registers\":{},\"try_count\":{},\"instruction_count\":{},\"features\":{{{}}},\"decode_error\":{},\"raw_insns_hex\":{},\"canonical_code\":{},\"disassembly\":{},\"symbolic_calls\":[{}]}}",
                        json(&entry),
                        logical,
                        pool,
                        m.method_idx,
                        json(&identity),
                        json(&owner),
                        json(&name),
                        json(&signature),
                        class.access_flags,
                        m.access_flags,
                        m.code_off,
                        m.code.as_ref().map_or(0, |c| c.insns_size),
                        m.code.as_ref().map_or(0, |c| c.registers_size),
                        m.code.as_ref().map_or(0, |c| c.tries.len()),
                        count.map_or("null".into(), |n| n.to_string()),
                        features,
                        error.as_deref().map_or("null".into(), json),
                        m.code
                            .as_ref()
                            .map_or("null".into(), |c| json(&hex(&c.insns))),
                        canonical.as_deref().map_or("null".into(), json),
                        evidence.as_deref().map_or("null".into(), json),
                        calls.join(",")
                    )?;
                }
            }
        }
    }
    output.flush()?;
    Ok(())
}
