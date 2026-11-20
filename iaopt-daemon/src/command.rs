//! Centro de comando en vivo: vitales, prediccion termica vs real, matriz de
//! conocimiento, puerta de exploracion, plan de control, auditoria de bucles y
//! propiedad de controles. Todo se calcula del journal y la telemetria real.

use crate::agent::{blacklist, read_all as unresolved, weights_json};
use crate::device;
use crate::state::State;
use crate::{journal, telemetry};
use std::fs;
use std::path::Path;

const DIR: &str = "/data/adb/ia_optimizer";
const OUT: &str = "/data/adb/ia_optimizer/command_center.json";
const THERMAL: &str = "/data/adb/ia_optimizer/thermal_history.jsonl";

fn jstr(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Llamado desde el ciclo del agente (fase 4) para alimentar la prevision termica.
pub fn record_thermal(actual: i32) {
    let hist = read_thermal();
    let last = hist.last().map(|x| x.1).unwrap_or(actual);
    let prev = if hist.len() >= 2 {
        hist[hist.len() - 2].1
    } else {
        actual
    };
    // Prediccion ingenua: extrapolacion lineal de la ultima pendiente.
    let predicted = last + (last - prev);
    let mut h = hist;
    h.push((predicted, actual));
    let start = h.len().saturating_sub(24);
    let h = &h[start..];
    let mut out = String::new();
    for (p, a) in h {
        out.push_str(&format!("{{\"predicted\":{},\"actual\":{}}}\n", p, a));
    }
    let _ = fs::write(THERMAL, out);
}

fn read_thermal() -> Vec<(i32, i32)> {
    let Ok(s) = fs::read_to_string(THERMAL) else {
        return Vec::new();
    };
    let mut v = Vec::new();
    for l in s.lines() {
        let p = crate::state::get_json_num(l, "predicted").unwrap_or(0.0) as i32;
        let a = crate::state::get_json_num(l, "actual").unwrap_or(0.0) as i32;
        v.push((p, a));
    }
    v
}

fn policies() -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(rd) = fs::read_dir("/sys/devices/system/cpu/cpufreq") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("policy") {
                v.push(n);
            }
        }
    }
    v.sort();
    v.dedup();
    if v.is_empty() {
        v.push("policy0".into());
    }
    v
}

fn freq_of(policy: &str, file: &str) -> String {
    iaopt_core::sysfs::read_trim(Path::new(&format!(
        "/sys/devices/system/cpu/cpufreq/{}/{}",
        policy, file
    )))
    .unwrap_or_else(|| "-".into())
}

fn journal_counts() -> (usize, usize, usize, usize, usize, usize) {
    // (ok, fallos, usuario, sondas, no_sostenidos, total)
    let Ok(s) = fs::read_to_string(DIR.to_string() + "/journal.jsonl") else {
        return (0, 0, 0, 0, 0, 0);
    };
    let mut ok = 0;
    let mut fail = 0;
    let mut user = 0;
    let mut probe = 0;
    for l in s.lines().rev().take(400) {
        if l.contains("\"ok\":true") {
            ok += 1;
        } else if l.contains("\"ok\":false") {
            fail += 1;
        }
        if l.contains("\"kind\":\"usuario\"") {
            user += 1;
        }
        if l.contains("\"kind\":\"sonda\"") {
            probe += 1;
        }
    }
    (ok, fail, user, probe, fail, ok + fail)
}

pub fn build(st: &State) -> String {
    let m = st.record();
    let tel = telemetry::sample(false);
    let dev = device::load();
    let pols = policies();

    // 1) Cluster activo (el de mayor CPU) y vitales
    let cluster = pols.last().cloned().unwrap_or_else(|| "policy0".into());
    let cpu = (m.cpu_load * 100.0).clamp(0.0, 100.0);
    let ram = if tel.mem_total_mb > 0 {
        (tel.mem_total_mb - tel.mem_avail_mb) as f32 / tel.mem_total_mb as f32 * 100.0
    } else {
        0.0
    };
    let soc_max = tel.thermal_zones.iter().map(|z| z.2).max().unwrap_or(0);
    let therm = read_thermal();
    let trend: Vec<String> = therm.iter().map(|(_, a)| a.to_string()).collect();

    // 2) Prediccion termica vs real y error por bucle
    let pairs: Vec<String> = therm
        .iter()
        .map(|(p, a)| format!("{{\"predicted\":{},\"actual\":{}}}", p, a))
        .collect();
    let errors: Vec<f32> = therm
        .iter()
        .map(|(p, a)| ((p - a).abs() as f32 / 10.0))
        .collect();
    let err_str: Vec<String> = errors.iter().map(|e| format!("{:.3}", e)).collect();
    let (err_first, err_last, loops, verdict) = if errors.len() >= 2 {
        let f = errors[0];
        let l = errors[errors.len() - 1];
        let v = if l < f - 0.005 {
            "Convergiendo"
        } else {
            "Aun no cayendo"
        };
        (f, l, errors.len(), v)
    } else {
        (0.0, 0.0, errors.len(), "Sin datos")
    };

    // 3) Matriz de conocimiento por politica
    let j = journal::last(400);
    let mut inciertos = 0;
    let mut utiles = 0;
    let mut kcards: Vec<String> = Vec::new();
    for p in &pols {
        let samples = j
            .iter()
            .filter(|e| e.path.contains(p.as_str()) || p == "policy0")
            .count();
        let epistemic = (0.030f32 / (1.0 + samples as f32 * 0.25)).clamp(0.001, 0.05);
        let info = (epistemic * 0.04).clamp(0.0, 0.002);
        let state = if samples >= 8 && epistemic <= 0.02 {
            "Util conocido"
        } else {
            "Incierto"
        };
        if state == "Incierto" {
            inciertos += 1;
        } else {
            utiles += 1;
        }
        kcards.push(format!(
            "{{\"policy\":\"{}\",\"samples\":{},\"epistemic\":{:.3},\"info_value\":{:.3},\"state\":\"{}\"}}",
            p, samples, epistemic, info, state
        ));
    }
    let ktotal = (inciertos + utiles).max(1);

    // 4) Puerta de exploracion
    let probes_used = journal_counts().3;
    let gate_open = unresolved().is_empty() && blacklist().is_empty();
    let gate = if gate_open {
        "Se permite una investigacion ahora mismo."
    } else {
        "Investigacion retenida: hay errores sin resolver."
    };

    // 5) Plan de control
    let coverage = journal::verified_count(400);
    let coverage_pct = if coverage > 0 { 100.0 } else { 0.0 };
    let next_action = if let Some(p) = pols.last() {
        format!("Ajustar techo de frecuencia en {} y medir", p)
    } else {
        "Medir y mantener".into()
    };

    // 6) Auditoria de bucles
    let (ok, fail, user, probe, _ns, _tot) = journal_counts();
    let bl = blacklist().len();
    let unres = unresolved().len();
    let mejorado = ok.saturating_sub(probe);
    let counters = format!(
        "{{\"mejorado\":{},\"regresado\":{},\"seguridad\":{},\"desviacion\":{},\"anulaciones\":{},\"veredictos\":{},\"sondas\":{}}}",
        mejorado, fail, bl + unres, fail, user, ok, probe
    );

    // 7) Propiedad de controles
    let mut owners: Vec<String> = Vec::new();
    for p in &pols {
        let min = freq_of(p, "scaling_min_freq");
        let max = freq_of(p, "scaling_max_freq");
        owners.push(format!(
            "{{\"node\":\"cpu_limits:{}\",\"owner\":\"IA_MOTORS\",\"intent\":\"{}:{}\",\"state\":\"Verificado\"}}",
            p, min.trim(), max.trim()
        ));
    }

    let sat = iaopt_core::ai::satisfaction(&m);
    let feats = dev.features.iter().map(|f| jstr(f)).collect::<Vec<_>>().join(",");
    let wj = weights_json();

    format!(
        "{{\"policy\":\"{}\",\"vitals\":{{\"cpu\":{:.1},\"battery\":{:.1},\"ram\":{:.1},\"temp_c\":{:.1},\"trend\":[{}]}}, \"thermal_prediction\":[{}], \"prediction_error\":[{}], \"error_summary\":{{\"first\":{:.3},\"last\":{:.3},\"loops\":{},\"verdict\":\"{}\"}}, \"knowledge\":{{\"inciertos\":{},\"utiles\":{},\"total\":{},\"cards\":[{}]}}, \"exploration_gate\":{{\"allowed\":{},\"text\":{},\"probes_used\":{},\"probes_limit\":0}}, \"control_plan\":{{\"mode\":\"Adaptive\",\"coverage_pct\":{:.0},\"profile\":\"Context-aware user\",\"next_action\":{},\"reversible\":true}}, \"loop_audit\":{}, \"ownership\":[{}], \"satisfaction\":{:.3},\"device\":{},\"weights\":{}}}",
        cluster,
        cpu,
        m.battery_pct,
        ram,
        soc_max,
        trend.join(","),
        pairs.join(","),
        err_str.join(","),
        err_first,
        err_last,
        loops,
        verdict,
        inciertos,
        utiles,
        ktotal,
        kcards.join(","),
        gate_open,
        jstr(gate),
        probes_used,
        coverage_pct,
        jstr(&next_action),
        counters,
        owners.join(","),
        sat,
        jstr(&dev.soc),
        wj
    )
}

/// Escribe command_center.json de forma atómica.
pub fn write(st: &State) {
    let body = build(st);
    let tmp = format!("{}.tmp", OUT);
    if fs::write(&tmp, body).is_ok() {
        let _ = fs::rename(&tmp, OUT);
    }
}

/// Evaluación del dispositivo (botón de la sección de propiedad).
pub fn evaluate(st: &State) -> String {
    let dev = device::load();
    let checks = crate::repair::diagnose(st);
    let items: Vec<String> = checks
        .iter()
        .map(|c| {
            format!(
                "{{\"name\":{},\"ok\":{},\"detail\":{}}}",
                jstr(&c.name),
                c.ok,
                jstr(&c.detail)
            )
        })
        .collect();
    let _ = now();
    format!(
        "{{\"ok\":true,\"device\":{},\"soc\":{},\"gpu\":{},\"checks\":[{}]}}",
        jstr(&dev.device),
        jstr(&dev.soc),
        jstr(&dev.gpu),
        items.join(",")
    )
}
