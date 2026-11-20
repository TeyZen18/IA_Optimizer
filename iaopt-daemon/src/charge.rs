//! Gestión de energía: límite de carga (hasta X %) y carga inteligente.
//!
//! - Límite de carga: al llegar al objetivo con el cargador puesto, se corta
//!   la carga de la batería (el móvil sigue alimentado por el cargador) y se
//!   reanuda si baja 5 puntos. Usa el mismo catálogo de nodos verificados del
//!   bypass; nunca un nodo de riesgo.
//! - Carga inteligente: mantiene el 80 % mientras estás enchufado y completa
//!   al 100 % antes de tu hora habitual de desenchufe, que APRENDE de tus
//!   desconexiones reales (mediana de las últimas 14). Sin rutina aprendida,
//!   mantiene el 80 % y lo dice.
//!
//! Seguridad: nunca se corta la carga sin cargador; al desenchufar, al
//! desactivar o al arrancar tras un crash se restaura siempre el nodo.
//! Cada transición se verifica con relectura y se registra en el diario.

use crate::state;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const CFG: &str = "/data/adb/ia_optimizer/charge.json";
const STATE: &str = "/data/adb/ia_optimizer/charge_state.json";
const MARGIN: u32 = 5;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Minuto del día local (0..1439) para la rutina de desenchufe.
fn minute_of_day() -> u32 {
    let secs = now() % 86400;
    // El dispositivo está en UTC-3 (Argentina); se usa la hora local del
    // sistema vía `date` para no depender de la zona del proceso.
    let out = std::process::Command::new("date")
        .arg("+%H:%M")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if let Some((h, m)) = out.split_once(':') {
        if let (Ok(h), Ok(m)) = (h.parse::<u32>(), m.parse::<u32>()) {
            return (h * 60 + m).min(1439);
        }
    }
    (secs / 60) as u32
}

fn jstr(s: &str) -> String {
    let clean: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    format!("\"{}\"", clean.replace('\\', "\\\\").replace('"', "\\\""))
}

struct Cfg {
    limit_enabled: bool,
    target: u32,
    smart: bool,
    node: String,
}

fn cfg() -> Cfg {
    let s = fs::read_to_string(CFG).unwrap_or_default();
    Cfg {
        limit_enabled: state::get_json_bool(&s, "limit_enabled").unwrap_or(false),
        target: state::get_json_num(&s, "target").map(|v| v as u32).unwrap_or(80).clamp(40, 100),
        smart: state::get_json_bool(&s, "smart").unwrap_or(false),
        node: state::get_json_str(&s, "node").unwrap_or_default(),
    }
}

fn save_cfg(c: &Cfg) {
    let body = format!(
        "{{\"limit_enabled\":{},\"target\":{},\"smart\":{},\"node\":{}}}",
        c.limit_enabled, c.target, c.smart, jstr(&c.node)
    );
    let tmp = format!("{}.tmp", CFG);
    if fs::write(&tmp, body).is_ok() {
        let _ = fs::rename(&tmp, CFG);
    }
}

#[derive(Default)]
struct St {
    holding: bool,
    last_charger: bool,
    unplugs: Vec<u32>,
    effect_ok: bool,
    /// Ventana deslizante de muestras de corriente (true = cargando) para
    /// el detector de cargador bloqueado. Solo se avisa, no se actúa.
    stuck_window: Vec<bool>,
    stuck_notified: bool,
}

fn slot() -> &'static Mutex<St> {
    static S: OnceLock<Mutex<St>> = OnceLock::new();
    S.get_or_init(|| {
        let mut st = St::default();
        if let Ok(s) = fs::read_to_string(STATE) {
            st.holding = state::get_json_bool(&s, "holding").unwrap_or(false);
            st.last_charger = state::get_json_bool(&s, "last_charger").unwrap_or(false);
            if let Some(body) = s.split("\"unplugs\":[").nth(1) {
                let body = body.split(']').next().unwrap_or("");
                for tok in body.split(',') {
                    if let Ok(v) = tok.trim().parse::<u32>() {
                        st.unplugs.push(v);
                    }
                }
            }
        }
        Mutex::new(st)
    })
}

fn save_state(st: &St) {
    let u: Vec<String> = st.unplugs.iter().map(|v| v.to_string()).collect();
    let body = format!(
        "{{\"holding\":{},\"last_charger\":{},\"unplugs\":[{}]}}",
        st.holding, st.last_charger, u.join(",")
    );
    let tmp = format!("{}.tmp", STATE);
    if fs::write(&tmp, body).is_ok() {
        let _ = fs::rename(&tmp, STATE);
    }
}

/// Nodo elegido (o el primero compatible no riesgoso del catálogo).
fn pick_node(c: &Cfg) -> Option<&'static crate::bypass::Node> {
    if !c.node.is_empty() {
        if let Some(n) = crate::bypass::NODES.iter().find(|n| n.path == c.node) {
            if !n.risky && Path::new(n.path).exists() {
                return Some(n);
            }
        }
    }
    crate::bypass::NODES.iter().find(|n| {
        !n.risky && Path::new(n.path).exists() && fs::metadata(n.path).map(|m| !m.permissions().readonly()).unwrap_or(false)
    })
}

/// Escritura con relectura (el nodo debe sostener el valor).
fn write_node(n: &crate::bypass::Node, on: bool) -> bool {
    let v = if on { n.on } else { n.off };
    let p = Path::new(n.path);
    if fs::write(p, format!("{}\n", v)).is_err() {
        return false;
    }
    std::thread::sleep(Duration::from_millis(350));
    let after = fs::read_to_string(p).unwrap_or_default().trim().to_string();
    after == v
}

/// true cuando el límite de carga gestiona el nodo: el autotick del bypass
/// manual no debe pelearse con él.
pub fn manages_node() -> bool {
    let c = cfg();
    c.limit_enabled && pick_node(&c).is_some()
}

const SAFETY_TIMER: &str = "/proc/mtk_battery_cmd/en_safety_timer";

/// Estado del temporizador de seguridad de carga MTK (None si no existe).
pub fn safety_timer() -> Option<bool> {
    fs::read_to_string(SAFETY_TIMER)
        .ok()
        .map(|s| s.trim() == "1")
}

/// Activa/desactiva el temporizador con relectura. Este temporizador corta
/// la carga tras muchas horas acumuladas con el cargador puesto (protección
/// del vendor): al expirar, el móvil deja de cargar aunque diga "Cargando"
/// (VBUS presente, IBUS=0). Un corte largo del límite de carga puede
/// agotarlo; por eso el módulo lo gestiona cuando el límite está activo.
pub fn safety_timer_set(on: bool) -> bool {
    let v = if on { "1" } else { "0" };
    if fs::write(SAFETY_TIMER, format!("{}\n", v)).is_err() {
        return false;
    }
    std::thread::sleep(Duration::from_millis(250));
    fs::read_to_string(SAFETY_TIMER)
        .map(|s| s.trim() == v)
        .unwrap_or(false)
}

const CURRENT_CMD: &str = "/proc/mtk_battery_cmd/current_cmd";

/// MTK: current_cmd acepta "<cmd> <valor>"; "1 0" fuerza corriente de carga
/// 0 y "0 0" libera el comando. VERIFICADO en mt6360: en_power_path a solas
/// NO detiene la carga (el contador sube igual) — solo alimenta el sistema
/// por el cargador; el corte real de batería aquí es current_cmd.
fn mtk_cmd() -> bool {
    std::path::Path::new(CURRENT_CMD).exists()
}

fn write_cmd(v: &str) -> bool {
    if fs::write(CURRENT_CMD, format!("{}\n", v)).is_err() {
        return false;
    }
    std::thread::sleep(Duration::from_millis(300));
    fs::read_to_string(CURRENT_CMD)
        .map(|s| s.trim() == v)
        .unwrap_or(false)
}

/// Corte físico de la carga: current_cmd en MTK; nodo power-path en el
/// resto de hardware (donde sí desconecta la batería).
fn actuator_hold() -> bool {
    if mtk_cmd() {
        write_cmd("1 0")
    } else {
        pick_node(&cfg()).map(|n| write_node(n, true)).unwrap_or(false)
    }
}

fn actuator_release() -> bool {
    let ok = if mtk_cmd() {
        write_cmd("0 0")
    } else {
        pick_node(&cfg()).map(|n| write_node(n, false)).unwrap_or(false)
    };
    // En MTK el comando puede dejar el power-path activado: si el bypass
    // manual no está activo, devuélvelo a 0 (no dejar nodos huérfanos).
    let bypass_on = crate::bypass::cfg_get("enabled").map(|s| s == "true").unwrap_or(false);
    if !bypass_on {
        let p = std::path::Path::new("/proc/mtk_battery_cmd/en_power_path");
        if fs::read_to_string(p).map(|s| s.trim() == "1").unwrap_or(false) {
            let _ = fs::write(p, "0\n");
        }
    }
    ok
}

fn actuator_name() -> &'static str {
    if mtk_cmd() { "current_cmd" } else { "power_path" }
}

/// Corriente de batería en mA (positiva = cargando).
fn charging_ma() -> i32 {
    fs::read_to_string("/sys/class/power_supply/battery/current_now")
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .map(|v| if v.abs() >= 10_000 { (v / 1000) as i32 } else { v as i32 })
        .unwrap_or(0)
}

/// Verificación de EFECTO físico (no solo relectura del nodo): tras el
/// corte la corriente de batería debe caer. >250 mA = sigue cargando.
fn effect_ok() -> bool {
    charging_ma() <= 250
}

/// Hora habitual de desenchufe aprendida (mediana, mín. 3 muestras).
fn learned_unplug(st: &St) -> Option<u32> {
    if st.unplugs.len() < 3 {
        return None;
    }
    let mut v = st.unplugs.clone();
    v.sort_unstable();
    Some(v[v.len() / 2])
}

/// Objetivo efectivo: el elegido, o 80/100 según la rutina en carga inteligente.
fn effective_target(c: &Cfg, st: &St) -> u32 {
    if !c.smart {
        return c.target;
    }
    match learned_unplug(st) {
        Some(unplug) => {
            let m = minute_of_day();
            // Ventana de completar: desde 60 min antes hasta 20 min después
            // de la hora habitual (con vuelta a medianoche).
            let start = (unplug + 1440 - 60) % 1440;
            let end = (unplug + 20) % 1440;
            let in_window = if start <= end {
                m >= start && m <= end
            } else {
                m >= start || m <= end
            };
            if in_window { 100 } else { 80 }
        }
        None => 80,
    }
}

fn fmt_minute(m: u32) -> String {
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// Un tick del lazo (cada 5 s).
pub fn tick() {
    let c = cfg();
    let Ok(mut st) = slot().lock() else { return };
    let charger = crate::bypass::charger_connected();
    let pct = crate::telemetry::sample(false).bat_pct.unwrap_or(0) as u32;
    let mut changed = false;

    // Transición de cargador: registrar desenchufe real y restaurar siempre.
    if st.last_charger && !charger {
        st.unplugs.push(minute_of_day());
        if st.unplugs.len() > 14 {
            st.unplugs.remove(0);
        }
        changed = true;
        if st.holding {
            let _ = actuator_release();
            st.holding = false;
            st.effect_ok = true;
            crate::journal::append_note("carga", "carga restaurada al desenchufar (seguridad)");
        }
    }
    st.last_charger = charger;

    // Detector de cargador bloqueado: ventana deslizante de 12 muestras
    // (60 s); avisa si ≥8 no tienen corriente de carga. Tolera ráfagas
    // breves de un cargador inestable. (A 100 % la corriente cae de forma
    // legítima al terminar la carga: no avisar ahí.)
    if charger && pct < 95 && !st.holding {
        let sample_ok = charging_ma() >= 100;
        st.stuck_window.push(sample_ok);
        if st.stuck_window.len() > 12 {
            st.stuck_window.remove(0);
        }
        let bad = st.stuck_window.iter().filter(|v| !**v).count();
        if st.stuck_window.len() >= 12 && bad >= 8 && !st.stuck_notified {
            st.stuck_notified = true;
            crate::journal::append_note(
                "carga",
                "cargador conectado sin corriente de entrada la mayor parte de 60 s: negociación bloqueada o cargador/cable defectuoso (reenchufar o cambiar)",
            );
        }
        if bad < 8 {
            st.stuck_notified = false;
        }
    } else {
        st.stuck_window.clear();
        st.stuck_notified = false;
    }

    if !c.limit_enabled {
        if st.holding {
            let _ = actuator_release();
            st.holding = false;
            st.effect_ok = true;
            changed = true;
            crate::journal::append_note("carga", "límite de carga desactivado: carga restaurada");
        }
        if changed { save_state(&st); }
        return;
    }

    // Con el límite activo y cargador puesto, el temporizador de seguridad
    // del vendor se desactiva (un corte largo lo agotaría y dejaría el móvil
    // sin cargar al soltar el límite). Sin cargador, se restaura.
    if charger {
        if safety_timer() == Some(true) {
            if safety_timer_set(false) {
                crate::journal::append_note("carga", "temporizador de seguridad desactivado (límite de carga activo)");
            }
        }
    } else if safety_timer() == Some(false) {
        let _ = safety_timer_set(true);
    }

    if !mtk_cmd() && pick_node(&c).is_none() {
        if changed { save_state(&st); }
        return;
    }

    if charger {
        let target = effective_target(&c, &st);
        if pct >= target && !st.holding {
            if actuator_hold() {
                st.holding = true;
                changed = true;
                // Verificación de efecto físico: la corriente debe caer.
                std::thread::sleep(Duration::from_millis(1500));
                st.effect_ok = effect_ok();
                crate::journal::append_note(
                    "carga",
                    &format!(
                        "carga detenida al {}% (objetivo {}%, actuador {}, efecto {})",
                        pct, target, actuator_name(),
                        if st.effect_ok { "verificado" } else { "NO sostenido" }
                    ),
                );
                if !st.effect_ok {
                    crate::warnings::record_all(&[iaopt_core::tuner::Warning {
                        path: if mtk_cmd() { CURRENT_CMD.to_string() } else { pick_node(&c).map(|n| n.path.to_string()).unwrap_or_default() },
                        kind: "sin efecto".to_string(),
                        detail: "el corte se escribió pero la batería sigue cargando".to_string(),
                    }]);
                }
            } else {
                crate::warnings::record_all(&[iaopt_core::tuner::Warning {
                    path: if mtk_cmd() { CURRENT_CMD.to_string() } else { pick_node(&c).map(|n| n.path.to_string()).unwrap_or_default() },
                    kind: "sin efecto".to_string(),
                    detail: "el nodo de carga no aceptó el corte".to_string(),
                }]);
            }
        } else if st.holding {
            // Re-verificación continua del efecto mientras se mantiene.
            st.effect_ok = effect_ok();
            if pct <= target.saturating_sub(MARGIN) {
                if actuator_release() {
                    st.holding = false;
                    st.effect_ok = true;
                    changed = true;
                    crate::journal::append_note(
                        "carga",
                        &format!("carga reanudada al {}% (objetivo {}%)", pct, target),
                    );
                }
            }
        }
    } else if st.holding {
        // Sin cargador nunca se mantiene el corte.
        let _ = actuator_release();
        st.holding = false;
        st.effect_ok = true;
        changed = true;
    }

    if changed { save_state(&st); }
}

/// Arranque tras crash: si quedó un corte pendiente, se restaura.
pub fn restore_pending() {
    let Ok(mut st) = slot().lock() else { return };
    if st.holding {
        let _ = actuator_release();
        st.holding = false;
        st.effect_ok = true;
        save_state(&st);
        crate::journal::append_note("carga", "corte de carga pendiente restaurado al arrancar");
    }
    // Nodo power-path huérfano de un corte antiguo (bypass apagado): a 0.
    let bypass_on = crate::bypass::cfg_get("enabled").map(|s| s == "true").unwrap_or(false);
    if !bypass_on {
        let p = std::path::Path::new("/proc/mtk_battery_cmd/en_power_path");
        if fs::read_to_string(p).map(|s| s.trim() == "1").unwrap_or(false) {
            let _ = fs::write(p, "0\n");
        }
    }
}

pub fn run(period: Duration) {
    std::thread::spawn(move || loop {
        std::thread::sleep(period);
        tick();
    });
}

/// POST /api/charge: preferencias del usuario (opt-in, verificadas).
pub fn handle(body: &str) -> String {
    // Escritura de diagnóstico con lista blanca y restauración automática.
    if let Some(pw) = body.split("\"probe_write\"").nth(1) {
        let path = state::get_json_str(pw, "path").unwrap_or_default();
        let value = state::get_json_str(pw, "value").unwrap_or_default();
        let path2 = state::get_json_str(pw, "path2");
        let value2 = state::get_json_str(pw, "value2");
        return probe_write_pair(&path, &value, path2.as_deref(), value2.as_deref());
    }
    // Interruptor manual del temporizador de seguridad (diagnóstico: carga
    // detenida con cargador puesto por temporizador del vendor expirado).
    if let Some(v) = state::get_json_bool(body, "safety_timer") {
        let ok = safety_timer_set(v);
        crate::journal::append_note(
            "carga",
            &format!("temporizador de seguridad {} manualmente ({})", if v { "activado" } else { "desactivado" }, if ok { "verificado" } else { "sin efecto" }),
        );
        return format!("{{\"ok\":{},\"safety_timer\":{}}}", ok, safety_timer().map(|b| b.to_string()).unwrap_or_else(|| "null".into()));
    }
    let mut c = cfg();
    if let Some(v) = state::get_json_bool(body, "limit_enabled") {
        c.limit_enabled = v;
        if !v && safety_timer() == Some(false) {
            // Al apagar el límite, devuelve la protección del vendor.
            let _ = safety_timer_set(true);
        }
    }
    if let Some(v) = state::get_json_num(body, "target") {
        c.target = (v as u32).clamp(40, 100);
    }
    if let Some(v) = state::get_json_bool(body, "smart") {
        c.smart = v;
    }
    if let Some(v) = state::get_json_str(body, "node") {
        if crate::bypass::NODES.iter().any(|n| n.path == v && !n.risky) {
            c.node = v;
        } else {
            return "{\"ok\":false,\"detail\":\"nodo no compatible o de riesgo\"}".into();
        }
    }
    // Al activar sin nodo elegido, se toma el primero compatible (en MTK
    // el actuador es current_cmd y no necesita nodo power-path).
    if c.limit_enabled && c.node.is_empty() && !mtk_cmd() {
        if let Some(n) = pick_node(&c) {
            c.node = n.path.to_string();
        } else {
            return "{\"ok\":false,\"detail\":\"tu hardware no expone un nodo de corte de carga compatible\"}".into();
        }
    }
    save_cfg(&c);
    // Aplicar el cambio inmediatamente (no esperar al siguiente tick).
    tick();
    status()
}

/// GET /api/charge-nodes: candidatos a actuador de corte con su estado
/// (diagnóstico de hardware: qué expone realmente este kernel).
pub fn nodes_probe() -> String {
    let mut items: Vec<String> = Vec::new();
    let mut probe = |path: &str| {
        let p = std::path::Path::new(path);
        let exists = p.exists();
        let cur = fs::read_to_string(p).unwrap_or_default().trim().to_string();
        let writable = exists
            && fs::metadata(p).map(|m| !m.permissions().readonly()).unwrap_or(false);
        items.push(format!(
            "{{\"path\":{},\"exists\":{},\"writable\":{},\"current\":{}}}",
            jstr(path), exists, writable, jstr(&cur)
        ));
    };
    probe("/proc/mtk_battery_cmd/current_cmd");
    probe("/proc/mtk_battery_cmd/en_power_path");
    probe("/sys/class/power_supply/battery/constant_charge_current_max");
    probe("/sys/class/power_supply/battery/input_current_limit");
    probe("/sys/class/power_supply/battery/charge_control_limit");
    probe("/sys/class/power_supply/battery/charging_enabled");
    probe("/sys/class/power_supply/ac/input_current_limit");
    if let Ok(rd) = fs::read_dir("/sys/class/charger_class") {
        for e in rd.flatten() {
            let base = e.path();
            if let Ok(files) = fs::read_dir(&base) {
                for f in files.flatten() {
                    let p = f.path();
                    if p.is_file() {
                        probe(&p.to_string_lossy());
                    }
                }
            }
        }
    }
    for dir in ["/sys/devices/platform/charger", "/sys/devices/platform/mt-battery", "/sys/devices/platform/mtk-charger"] {
        if let Ok(rd) = fs::read_dir(dir) {
            for f in rd.flatten() {
                let p = f.path();
                if p.is_file() {
                    probe(&p.to_string_lossy());
                }
            }
        }
    }
    format!("{{\"nodes\":[{}]}}", items.join(","))
}

/// POST /api/charge {"probe_write":{"path":...,"value":...}}: escritura de
/// diagnóstico con lista blanca, relectura, medición de corriente antes/
/// durante y restauración automática del valor original.
pub fn probe_write(path: &str, value: &str) -> String {
    probe_write_pair(path, value, None, None)
}

/// Probe de uno o dos nodos a la vez (p. ej. enable_sc + sc_ibat_limit),
/// con restauración de ambos valores originales.
pub fn probe_write_pair(path: &str, value: &str, path2: Option<&str>, value2: Option<&str>) -> String {
    let allowed = |p: &str, v: &str| match p {
        "/sys/devices/platform/charger/chg1_current" => v == "0" || v == "4294967295" || v == "-1",
        "/sys/devices/platform/charger/enable_sc" => v == "0" || v == "1",
        "/sys/devices/platform/charger/sc_tuisoc" => v.parse::<u32>().map(|x| (40..=100).contains(&x)).unwrap_or(false),
        "/sys/devices/platform/charger/sc_ibat_limit" => v.parse::<u32>().map(|x| x <= 5000).unwrap_or(false),
        "/proc/mtk_battery_cmd/current_cmd" => v == "0 0" || v == "1 0",
        "/proc/mtk_battery_cmd/en_power_path" => v == "0" || v == "1",
        "/sys/devices/platform/charger/pe40" => v == "0" || v == "1",
        "/sys/devices/platform/charger/pe20" => v == "0" || v == "1",
        _ => false,
    };
    if !allowed(path, value) {
        return "{\"ok\":false,\"detail\":\"ruta/valor fuera de la lista blanca de diagnóstico\"}".into();
    }
    if let (Some(p2), Some(v2)) = (path2, value2) {
        if !allowed(p2, v2) {
            return "{\"ok\":false,\"detail\":\"ruta/valor 2 fuera de la lista blanca\"}".into();
        }
    }
    let p = std::path::Path::new(path);
    let original = fs::read_to_string(p).unwrap_or_default().trim().to_string();
    let original2 = path2.map(|p2| fs::read_to_string(p2).unwrap_or_default().trim().to_string());
    let before = charging_ma();
    if fs::write(p, format!("{}\n", value)).is_err() {
        return "{\"ok\":false,\"detail\":\"escritura rechazada\"}".into();
    }
    if let (Some(p2), Some(v2)) = (path2, value2) {
        let _ = fs::write(p2, format!("{}\n", v2));
    }
    std::thread::sleep(Duration::from_millis(3000));
    let readback = fs::read_to_string(p).unwrap_or_default().trim().to_string();
    let during = charging_ma();
    // Restaurar los valores originales (dejar el sistema como estaba).
    let mut restored = !original.is_empty() && fs::write(p, format!("{}\n", original)).is_ok();
    if let (Some(p2), Some(o2)) = (path2, original2) {
        if !o2.is_empty() {
            let _ = fs::write(p2, format!("{}\n", o2));
        }
    }
    std::thread::sleep(Duration::from_millis(500));
    let after = charging_ma();
    let conclusive = before > 250;
    format!(
        "{{\"ok\":true,\"path\":{},\"written\":{},\"readback\":{},\"ma_before\":{},\"ma_during\":{},\"ma_after\":{},\"restored\":{},\"conclusive\":{},\"effect\":{}}}",
        jstr(path), jstr(value), jstr(&readback), before, during, after, restored,
        conclusive, conclusive && during <= 250
    )
}

/// GET /api/charge: estado completo para el panel de energía.
pub fn status() -> String {
    let c = cfg();
    let st = slot().lock().map(|s| St {
        holding: s.holding,
        last_charger: s.last_charger,
        unplugs: s.unplugs.clone(),
        effect_ok: s.effect_ok,
        stuck_window: s.stuck_window.clone(),
        stuck_notified: s.stuck_notified,
    }).unwrap_or_default();
    let charger = crate::bypass::charger_connected();
    let pct = crate::telemetry::sample(false).bat_pct.unwrap_or(0);
    let node = pick_node(&c);
    let supported = mtk_cmd() || node.is_some();
    let target = effective_target(&c, &st);
    let learned = learned_unplug(&st);
    let mut detail = if !supported {
        "Tu hardware no expone un nodo de corte de carga compatible.".to_string()
    } else if !c.limit_enabled {
        "Límite de carga desactivado.".to_string()
    } else if !charger {
        "Sin cargador: la carga nunca se corta.".to_string()
    } else if st.holding {
        format!("Manteniendo {}%: la batería no carga, el móvil va al cargador.", target)
    } else if c.smart && learned.is_none() {
        format!("Cargando hacia {}% · aprendiendo tu rutina de desenchufe.", target)
    } else {
        format!("Cargando hacia {}%.", target)
    };
    if st.holding && !st.effect_ok {
        detail.push_str(" AVISO: el corte no se sostuvo físicamente (la batería sigue cargando).");
    }
    let stuck = st.stuck_window.len() >= 12
        && st.stuck_window.iter().filter(|v| !**v).count() >= 8;
    if stuck {
        detail.push_str(" ⚠ Cargador conectado sin corriente de entrada: reenchúfalo o prueba otro cable/cargador.");
    }
    format!(
        "{{\"ok\":true,\"supported\":{},\"node\":{},\"actuator\":{},\"effect_ok\":{},\"charger_stuck\":{},\"limit_enabled\":{},\"target\":{},\"smart\":{},\"holding\":{},\"charger\":{},\"battery_pct\":{},\"effective_target\":{},\"learned_unplug\":{},\"unplugs\":{},\"safety_timer\":{},\"detail\":{}}}",
        supported,
        jstr(node.map(|n| n.path).unwrap_or("")),
        jstr(actuator_name()),
        st.effect_ok,
        stuck,
        c.limit_enabled,
        c.target,
        c.smart,
        st.holding,
        charger,
        pct,
        target,
        learned.map(|m| jstr(&fmt_minute(m))).unwrap_or_else(|| "null".into()),
        st.unplugs.len(),
        safety_timer().map(|b| b.to_string()).unwrap_or_else(|| "null".into()),
        jstr(&detail)
    )
}
