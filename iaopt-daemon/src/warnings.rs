//! Advertencias del kernel: nodos que no se pudieron leer o escribir.
//!
//! El motor sigue funcionando; solo deja constancia de lo que el kernel
//! restringe para mostrarlo en IA Motors con una explicacion amigable.

use iaopt_core::tuner::Warning;
use std::fs;

const DIR: &str = "/data/adb/ia_optimizer";
const FILE: &str = "/data/adb/ia_optimizer/warnings.jsonl";

fn jstr(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Guarda los avisos recientes (sin duplicar por ruta) y devuelve el total.
pub fn record_all(ws: &[Warning]) -> usize {
    if ws.is_empty() {
        return count();
    }
    let _ = fs::create_dir_all(DIR);
    let existing = fs::read_to_string(FILE).unwrap_or_default();
    let mut paths: Vec<String> = existing
        .lines()
        .filter_map(|l| crate::state::get_json_str(l, "path"))
        .collect();
    let mut out = existing;
    for w in ws {
        if paths.iter().any(|p| p == &w.path) {
            continue;
        }
        paths.push(w.path.clone());
        out.push_str(&format!(
            "{{\"t\":{},\"kind\":{},\"path\":{},\"detail\":{}}}\n",
            now(),
            jstr(&w.kind),
            jstr(&w.path),
            jstr(&w.detail)
        ));
    }
    let _ = fs::write(FILE, out);
    paths.len()
}

pub fn count() -> usize {
    fs::read_to_string(FILE)
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

pub fn read_json() -> String {
    let s = fs::read_to_string(FILE).unwrap_or_default();
    let items: Vec<String> = s
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let t = crate::state::get_json_num(l, "t").unwrap_or(0.0) as u64;
            let kind = crate::state::get_json_str(l, "kind").unwrap_or_default();
            let path = crate::state::get_json_str(l, "path").unwrap_or_default();
            let detail = crate::state::get_json_str(l, "detail").unwrap_or_default();
            format!(
                "{{\"t\":{},\"kind\":{},\"path\":{},\"detail\":{}}}",
                t,
                jstr(&kind),
                jstr(&path),
                jstr(&detail)
            )
        })
        .collect();
    format!(
        "{{\"warnings\":[{}],\"total\":{}}}",
        items.join(","),
        items.len()
    )
}

/// Vacía el bufer de avisos del tuner y los persiste.
pub fn drain() -> usize {
    let ws = iaopt_core::tuner::drain_warnings();
    record_all(&ws)
}
