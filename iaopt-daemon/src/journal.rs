//! Journal de IA Motor: cada accion con su resultado medido.
//!
/// Se guarda como JSONL en /data/adb/ia_optimizer/journal.jsonl.
/// Un archivo (no memoria) para que las llamadas CLI de la WebUI de KernelSU
/// y el demonio vean el mismo historial.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

const RUNTIME_DIR: &str = "/data/adb/ia_optimizer";

fn journal_path() -> PathBuf {
    PathBuf::from(RUNTIME_DIR).join("journal.jsonl")
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub t: u64,
    pub profile: String,
    pub reason: String,
    pub confidence: f32,
    pub kind: String,
    pub app: String,
    pub mode: String,
    pub path: String,
    pub value: String,
    pub ok: bool,
    pub note: String,
    pub outcome: String,
    pub temp_delta: f32,
    pub cpu_delta: f32,
}

/// Entrada simple de diario (para el modo automatico).
pub fn append_note(kind: &str, note: &str) {
    append(
        "-", note, 1.0, kind, "", "auto", "", "", true, note, "", 0.0, 0.0,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn append(
    profile: &str,
    reason: &str,
    confidence: f32,
    kind: &str,
    app: &str,
    mode: &str,
    path: &str,
    value: &str,
    ok: bool,
    note: &str,
    outcome: &str,
    temp_delta: f32,
    cpu_delta: f32,
) {
    let _ = fs::create_dir_all(RUNTIME_DIR);
    let t = unix_now();
    let line = format!(
        "{{\"t\":{},\"profile\":{},\"reason\":{},\"conf\":{:.2},\"kind\":{},\"app\":{},\"mode\":{},\"path\":{},\"value\":{},\"ok\":{},\"note\":{},\"outcome\":{},\"temp_d\":{:.2},\"cpu_d\":{:.2}}}\n",
        t, jstr(profile), jstr(reason), confidence, jstr(kind), jstr(app), jstr(mode),
        jstr(path), jstr(value), ok, jstr(note), jstr(outcome), temp_delta, cpu_delta
    );
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(journal_path())
    {
        let _ = f.write_all(line.as_bytes());
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Ultimas `n` entradas (la mas reciente primero).
pub fn last(n: usize) -> Vec<Entry> {
    let Ok(s) = fs::read_to_string(journal_path()) else {
        return Vec::new();
    };
    let mut v: Vec<Entry> = s.lines().filter_map(parse).collect();
    v.reverse();
    v.truncate(n);
    v
}

/// Numero de acciones fallidas en las ultimas `n` entradas.
pub fn failures(n: usize) -> usize {
    let Ok(s) = fs::read_to_string(journal_path()) else {
        return 0;
    };
    s.lines().rev().take(n).filter(|l| {
        l.contains("\"ok\":false")
    }).count()
}

/// Memoria de aprendizaje por (modo, tipo de control): delta medio de temperatura
/// solo con acciones verificadas. Negativo = enfrio; positivo = calienta.
pub fn weights(n: usize) -> Vec<(String, String, f32, usize)> {
    let Ok(s) = fs::read_to_string(journal_path()) else {
        return Vec::new();
    };
    let mut acc: HashMap<String, (f32, usize)> = HashMap::new();
    for l in s.lines().rev().take(n) {
        if !l.contains("\"ok\":true") {
            continue;
        }
        let kind = match crate::state::get_json_str(l, "kind") {
            Some(k) => k,
            None => continue,
        };
        let mode = crate::state::get_json_str(l, "mode").unwrap_or_else(|| "-".into());
        let d = crate::state::get_json_num(l, "temp_d").unwrap_or(0.0) as f32;
        let e = acc.entry(format!("{}/{}", mode, kind)).or_insert((0.0, 0));
        e.0 += d;
        e.1 += 1;
    }
    let mut v: Vec<(String, String, f32, usize)> = acc
        .into_iter()
        .map(|(k, (sum, c))| {
            let (mode, kind) = k.split_once('/').unwrap_or((k.as_str(), ""));
            (mode.to_string(), kind.to_string(), if c > 0 { sum / c as f32 } else { 0.0 }, c)
        })
        .collect();
    v.sort_by(|a, b| (a.0.clone(), a.1.clone()).cmp(&(b.0.clone(), b.1.clone())));
    v
}

/// Acciones verificadas por app (para la tarjeta de aprendizaje).
pub fn per_app(n: usize) -> Vec<(String, usize)> {
    let Ok(s) = fs::read_to_string(journal_path()) else {
        return Vec::new();
    };
    let mut acc: HashMap<String, usize> = HashMap::new();
    for l in s.lines().rev().take(n) {
        if !l.contains("\"ok\":true") {
            continue;
        }
        if let Some(app) = crate::state::get_json_str(l, "app") {
            if !app.is_empty() {
                *acc.entry(app).or_insert(0) += 1;
            }
        }
    }
    let mut v: Vec<(String, usize)> = acc.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v
}

/// Total de acciones verificadas (progreso de aprendizaje).
pub fn verified_count(n: usize) -> usize {
    let Ok(s) = fs::read_to_string(journal_path()) else {
        return 0;
    };
    s.lines().rev().take(n).filter(|l| l.contains("\"ok\":true")).count()
}

fn parse(l: &str) -> Option<Entry> {
    Some(Entry {
        t: crate::state::get_json_num(l, "t")? as u64,
        profile: crate::state::get_json_str(l, "profile").unwrap_or_default(),
        reason: crate::state::get_json_str(l, "reason").unwrap_or_default(),
        confidence: crate::state::get_json_num(l, "conf").unwrap_or(0.0) as f32,
        kind: crate::state::get_json_str(l, "kind").unwrap_or_else(|| "otro".into()),
        app: crate::state::get_json_str(l, "app").unwrap_or_default(),
        mode: crate::state::get_json_str(l, "mode").unwrap_or_default(),
        path: crate::state::get_json_str(l, "path").unwrap_or_default(),
        value: crate::state::get_json_str(l, "value").unwrap_or_default(),
        ok: crate::state::get_json_bool(l, "ok").unwrap_or(false),
        note: crate::state::get_json_str(l, "note").unwrap_or_default(),
        outcome: crate::state::get_json_str(l, "outcome").unwrap_or_default(),
        temp_delta: crate::state::get_json_num(l, "temp_d").unwrap_or(0.0) as f32,
        cpu_delta: crate::state::get_json_num(l, "cpu_d").unwrap_or(0.0) as f32,
    })
}

fn jstr(v: &str) -> String {
    let mut o = String::with_capacity(v.len() + 2);
    o.push('"');
    for c in v.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            _ => o.push(c),
        }
    }
    o.push('"');
    o
}
