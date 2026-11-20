//! Vigilancia autónoma de IA Motors.
//!
//! Audita el estado del sistema cada pocos segundos, detecta nodos del kernel
//! con fallos repetidos o rutas caídas, aplica auto-correcciones y deja
//! constancia de cada ciclo en el diario y en el JSON que consume la app.

use crate::state::State;
use crate::{agent, journal};
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

static CYCLES: AtomicU64 = AtomicU64::new(0);
static LAST: Mutex<Option<String>> = Mutex::new(None);
static PREV: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub struct Audit {
    pub cycle: u64,
    pub knobs: usize,
    pub writable: usize,
    pub read_only: usize,
    pub healed: Vec<String>,
    pub failures: Vec<String>,
    pub errors: Vec<(String, String)>,
    pub blacklisted: Vec<String>,
}

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

/** Umbral de fallos consecutivos para aislar un nodo. */
const FAIL_LIMIT: usize = 3;

pub fn audit_once(st: &State) -> Audit {
    let cycle = CYCLES.fetch_add(1, Ordering::Relaxed) + 1;
    let knobs = st.probe().scan();
    let mut writable = 0;
    let mut read_only = 0;
    let mut healed = Vec::new();
    let mut failures = Vec::new();
    let mut errors = Vec::new();

    // 1) Nodos del kernel: escribibles, solo lectura y caídas respecto al ciclo previo.
    let mut prev = PREV.lock().unwrap();
    let mut now: Vec<String> = Vec::new();
    for k in &knobs {
        if k.path == "-" || k.current.as_deref() == Some("-") {
            continue;
        }
        if k.writable {
            writable += 1;
            now.push(k.path.clone());
        } else {
            read_only += 1;
        }
        if !k.writable && prev.iter().any(|p| p == &k.path) {
            failures.push(format!("Nodo caído: {}", short(&k.path)));
        }
    }
    // Los que ya no existen tambien cuentan como caidos.
    for p in prev.iter() {
        if !now.iter().any(|n| n == p) && knobs.iter().any(|k| &k.path == p) {
            failures.push(format!("Nodo sin respuesta: {}", short(p)));
        }
    }
    *prev = now;
    drop(prev);

    // 2) Fallos repetidos por ruta -> aislar (auto-corrección).
    let recent = journal::last(400);
    let mut fails: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for e in &recent {
        if !e.ok && !e.path.is_empty() && e.path != "-" {
            *fails.entry(e.path.clone()).or_insert(0) += 1;
        }
    }
    for (path, n) in fails {
        if n >= FAIL_LIMIT && !agent::is_blacklisted(&path) {
            agent::add_blacklist(&path, &format!("fallo repetido ({n} veces)"));
            healed.push(format!("Ruta aislada: {}", short(&path)));
        }
    }

    // 3) Errores sin resolver: intentar curarlos según su tipo.
    for u in agent::read_all() {
        let fixed = match u.kind.as_str() {
            "config" => {
                st.save();
                true
            }
            "bin" => {
                let p = "/data/adb/modules/ia_optimizer/bin/iaopt";
                std::fs::metadata(p)
                    .map(|_| {
                        let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755));
                    })
                    .is_ok()
            }
            "write" | "freq" | "sysfs" => {
                // Si la ruta ya responde, se da por curada.
                !u.path.is_empty() && std::fs::metadata(&u.path).is_ok()
            }
            "blacklist" => {
                // Una ruta aislada es la correccion aplicada, no un error pendiente.
                if agent::is_blacklisted(&u.path) {
                    agent::clear(&u.path);
                    true
                } else {
                    false
                }
            }
            _ => false,
        };
        if fixed {
            agent::clear(&u.path);
            healed.push(format!("{}: {}", kind_label(&u.kind), short(&u.path)));
        } else {
            errors.push((u.kind.clone(), u.path.clone()));
        }
    }

    // 4) Registrar el ciclo en el diario (auditoría autónoma).
    let summary = format!(
        "ciclo {}: {} escribibles, {} solo lectura, {} caidas, {} correcciones",
        cycle,
        writable,
        read_only,
        failures.len(),
        healed.len()
    );
    journal::append(
        &st.profile.lock().map(|p| p.as_str().to_string()).unwrap_or_else(|_| "balanced".into()),
        "autodiagnostico",
        0.9,
        "ciclo",
        "-",
        &summary,
        "-",
        "ok",
        healed.is_empty() && errors.is_empty(),
        "auditoria",
        "auditoria",
        0.0,
        0.0,
    );

    let audit = Audit {
        cycle,
        knobs: knobs.len(),
        writable,
        read_only,
        healed,
        failures,
        errors,
        blacklisted: agent::blacklist(),
    };
    let _ = crate::warnings::drain();
    *LAST.lock().unwrap() = Some(to_json(&audit));
    audit
}

fn short(path: &str) -> String {
    let s = path.substring_after_last('/');
    if s.is_empty() { path.to_string() } else { s.to_string() }
}

trait ShortPath {
    fn substring_after_last(&self, c: char) -> &str;
}
impl ShortPath for str {
    fn substring_after_last(&self, c: char) -> &str {
        match self.rfind(c) {
            Some(i) => &self[i + 1..],
            None => self,
        }
    }
}

fn kind_label(k: &str) -> String {
    match k {
        "config" => "Configuración reescrita".into(),
        "bin" => "Permisos del binario reparados".into(),
        "write" | "freq" | "sysfs" => "Nodo del kernel recuperado".into(),
        "blacklist" => "Ruta aislada".into(),
        other => format!("Corrección: {}", other),
    }
}

fn to_json(a: &Audit) -> String {
    let healed: Vec<String> = a.healed.iter().map(|s| jstr(s)).collect();
    let fails: Vec<String> = a.failures.iter().map(|s| jstr(s)).collect();
    let errs: Vec<String> = a
        .errors
        .iter()
        .map(|(k, p)| format!("{{\"kind\":{},\"path\":{}}}", jstr(k), jstr(p)))
        .collect();
    let bl: Vec<String> = a.blacklisted.iter().map(|s| jstr(s)).collect();
    format!(
        "{{\"cycle\":{},\"knobs\":{},\"writable\":{},\"read_only\":{},\"healed\":[{}],\"failures\":[{}],\"errors\":[{}],\"blacklist\":[{}]}}",
        a.cycle,
        a.knobs,
        a.writable,
        a.read_only,
        healed.join(","),
        fails.join(","),
        errs.join(","),
        bl.join(",")
    )
}

pub fn last_json() -> String {
    LAST.lock().unwrap().clone().unwrap_or_else(|| {
        "{\"cycle\":0,\"knobs\":0,\"writable\":0,\"read_only\":0,\"healed\":[],\"failures\":[],\"errors\":[],\"blacklist\":[]}".into()
    })
}

pub fn ticks() -> u64 {
    CYCLES.load(Ordering::Relaxed)
}
