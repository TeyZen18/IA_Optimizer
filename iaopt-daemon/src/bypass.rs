//! Carga bypass: detecta nodos del fabricante, permite apagar/encender y
//! probar cada nodo con verificacion y restauracion.
//!
//! Lista de rutas y comportamiento inspirados en AZenith (Apache-2.0, Zexshia).
//! La implementacion es propia; ver NOTICE.md.

use iaopt_core::sysfs::{is_writable, read_trim, write_verify};
use std::fs;
use std::io::Write;

const CFG: &str = "/data/adb/ia_optimizer/bypass.json";

pub struct Node {
    pub path: &'static str,
    pub desc: &'static str,
    pub on: &'static str,
    pub off: &'static str,
    pub risky: bool,
}

pub const NODES: &[Node] = &[
    Node { path: "/proc/mtk_battery_cmd/en_power_path", desc: "MTK: ruta de alimentacion (power path)", on: "1", off: "0", risky: false },
    Node { path: "/proc/mtk_battery_cmd/current_cmd", desc: "MTK: comando de carga (0 0 = bypass)", on: "0 0", off: "1 1", risky: true },
    Node { path: "/sys/class/power_supply/battery/charging_enabled", desc: "Bateria: habilitar carga", on: "0", off: "1", risky: false },
    Node { path: "/sys/class/power_supply/battery/charge_enabled", desc: "Bateria: habilitar carga", on: "0", off: "1", risky: false },
    Node { path: "/sys/class/power_supply/ac/charging_enabled", desc: "Cargador AC: habilitar carga", on: "0", off: "1", risky: false },
    Node { path: "/sys/class/power_supply/dc/charging_enabled", desc: "Cargador DC: habilitar carga", on: "0", off: "1", risky: false },
    Node { path: "/sys/class/power_supply/battery/charge_disable", desc: "Bateria: desactivar carga", on: "1", off: "0", risky: false },
    Node { path: "/sys/class/power_supply/battery/charging_disable", desc: "Bateria: desactivar carga", on: "1", off: "0", risky: false },
    Node { path: "/sys/class/power_supply/battery/input_suspend", desc: "Bateria: suspender entrada", on: "1", off: "0", risky: false },
    Node { path: "/sys/class/power_supply/battery/battery_input_suspend", desc: "Bateria: suspender entrada", on: "1", off: "0", risky: false },
    Node { path: "/sys/class/power_supply/battery/stop_charge", desc: "Bateria: detener carga", on: "1", off: "0", risky: false },
    Node { path: "/sys/class/power_supply/battery/restricted_charging", desc: "Bateria: carga restringida", on: "1", off: "0", risky: false },
    Node { path: "/sys/class/power_supply/wireless/restricted_charging", desc: "Inalambrica: carga restringida", on: "1", off: "0", risky: false },
    Node { path: "/sys/class/power_supply/battery/op_disable_charge", desc: "Bateria: desactivar carga (OP)", on: "1", off: "0", risky: false },
    Node { path: "/sys/class/power_supply/battery/connect_disable", desc: "Bateria: desactivar conexion de carga", on: "1", off: "0", risky: false },
    Node { path: "/sys/class/power_supply/battery/batt_slate_mode", desc: "Bateria: modo slate", on: "1", off: "0", risky: false },
    Node { path: "/sys/class/power_supply/battery/mmi_charging_enable", desc: "Bateria: habilitar carga (MMI)", on: "0", off: "1", risky: false },
    Node { path: "/sys/class/power_supply/battery/device/Charging_Enable", desc: "Bateria: habilitar carga (device)", on: "0", off: "1", risky: false },
    Node { path: "/sys/class/oplus_chg/battery/mmi_charging_enable", desc: "Oplus: habilitar carga", on: "0", off: "1", risky: false },
    Node { path: "/sys/class/qcom-battery/charging_enabled", desc: "QCOM: habilitar carga", on: "0", off: "1", risky: false },
    Node { path: "/sys/class/qcom-battery/input_suspend", desc: "QCOM: suspender entrada", on: "1", off: "0", risky: false },
    Node { path: "/sys/devices/platform/mt-battery/disable_charger", desc: "MTK: desactivar cargador", on: "1", off: "0", risky: false },
    Node { path: "/sys/devices/platform/charger/bypass_charger", desc: "Plataforma: bypass de cargador", on: "1", off: "0", risky: false },
    Node { path: "/sys/kernel/nubia_charge/charger_bypass", desc: "Nubia: bypass de cargador", on: "1", off: "0", risky: false },
    Node { path: "/sys/kernel/debug/google_charger/chg_suspend", desc: "Google: suspender carga", on: "1", off: "0", risky: false },
    Node { path: "/sys/class/hw_power/charger/charge_data/enable_charger", desc: "HW power: habilitar cargador", on: "0", off: "1", risky: false },
    Node { path: "/sys/class/asuslib/charging_suspend_en", desc: "Asus: suspender carga", on: "1", off: "0", risky: false },
    Node { path: "/sys/class/power_supply/bq2589x_charger/enable_charging", desc: "BQ2589x: habilitar carga", on: "0", off: "1", risky: false },
    Node { path: "/sys/module/qpnp_adaptive_charge/parameters/blocking", desc: "QNOP: bloqueo de carga adaptativa", on: "1", off: "0", risky: false },
    Node { path: "/sys/module/pm8921_charger/parameters/disabled", desc: "PM8921: cargador desactivado", on: "1", off: "0", risky: false },
    Node { path: "/sys/module/pmic8058_charger/parameters/disabled", desc: "PMIC8058: cargador desactivado", on: "1", off: "0", risky: false },
    Node { path: "/sys/module/smb137b/parameters/disabled", desc: "SMB137B: cargador desactivado", on: "1", off: "0", risky: false },
    Node { path: "/proc/driver/charger_limit_enable", desc: "Driver: limite de carga", on: "1", off: "0", risky: false },
];

#[derive(Default, Clone)]
pub struct NodeInfo {
    pub path: String,
    pub desc: String,
    pub exists: bool,
    pub writable: bool,
    pub current: String,
    pub state: String,
    pub risky: bool,
}

pub fn cfg_get(key: &str) -> Option<String> {
    let s = fs::read_to_string(CFG).ok()?;
    crate::state::get_json_str(&s, key)
}

fn cfg_num(key: &str, default: u32) -> u32 {
    let Some(s) = fs::read_to_string(CFG).ok() else {
        return default;
    };
    crate::state::get_json_num(&s, key)
        .map(|v| v as u32)
        .unwrap_or(default)
}

fn cfg_set(node: &str, enabled: bool, threshold: u32) {
    let body = format!(
        "{{\"node\":{},\"enabled\":{},\"threshold\":{}}}\n",
        json_str(node),
        enabled,
        threshold
    );
    let _ = fs::write(CFG, body);
}

fn json_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// ¿Hay cargador conectado? (usb/ac online o bateria cargando/full)
pub fn charger_connected() -> bool {
    for p in [
        "/sys/class/power_supply/usb/online",
        "/sys/class/power_supply/ac/online",
        "/sys/class/power_supply/dc/online",
    ] {
        if let Some(v) = read_trim(std::path::Path::new(p)) {
            if v.trim() == "1" {
                return true;
            }
        }
    }
    if let Some(s) = read_trim(std::path::Path::new("/sys/class/power_supply/battery/status")) {
        let s = s.to_ascii_lowercase();
        if s.contains("charging") || s.contains("full") {
            return true;
        }
    }
    false
}

fn node_by_path(path: &str) -> Option<&'static Node> {
    NODES.iter().find(|n| n.path == path)
}

fn state_of(n: &Node, current: &str) -> String {
    let c = current.trim();
    if c == n.on {
        "on".into()
    } else if c == n.off {
        "off".into()
    } else {
        "unknown".into()
    }
}

fn state_of_path(path: &str) -> String {
    if let Some(n) = node_by_path(path) {
        let p = std::path::Path::new(n.path);
        if p.exists() {
            return state_of(n, &read_trim(p).unwrap_or_default());
        }
    }
    "no".into()
}

/// ¿El nodo configurado esta actualmente en su valor "on" (bypass activo)?
pub fn active() -> bool {
    let node = cfg_get("node").unwrap_or_default();
    !node.is_empty() && state_of_path(&node) == "on"
}

pub fn scan_nodes() -> Vec<NodeInfo> {
    NODES
        .iter()
        .map(|n| {
            let p = std::path::Path::new(n.path);
            let exists = p.exists();
            NodeInfo {
                path: n.path.to_string(),
                desc: n.desc.to_string(),
                exists,
                writable: exists && is_writable(p),
                current: if exists { read_trim(p).unwrap_or_default() } else { String::new() },
                state: if exists {
                    state_of(n, &read_trim(p).unwrap_or_default())
                } else {
                    "no".into()
                },
                risky: n.risky,
            }
        })
        .collect()
}

pub fn status() -> String {
    let nodes = scan_nodes();
    let present: Vec<&NodeInfo> = nodes.iter().filter(|n| n.exists).collect();
    let node = cfg_get("node").unwrap_or_default();
    let enabled = cfg_get("enabled")
        .map(|s| s == "true")
        .unwrap_or(false);
    let threshold = cfg_num("threshold", 80);
    let charger = charger_connected();
    let items: Vec<String> = nodes
        .iter()
        .map(|n| {
            format!(
                "{{\"path\":{},\"desc\":{},\"exists\":{},\"writable\":{},\"current\":{},\"state\":\"{}\",\"risky\":{}}}",
                json_str(&n.path),
                json_str(&n.desc),
                n.exists,
                n.writable,
                json_str(&n.current),
                n.state,
                n.risky
            )
        })
        .collect();
    let bat_pct = crate::telemetry::sample(false).bat_pct.unwrap_or(0);
    format!(
        "{{\"supported\":{},\"charger\":{},\"enabled\":{},\"active\":{},\"node\":{},\"threshold\":{},\"battery_pct\":{},\"nodes\":[{}]}}",
        !present.is_empty(),
        charger,
        enabled,
        active(),
        json_str(&node),
        threshold,
        bat_pct,
        items.join(",")
    )
}

/// Umbral de bateria elegido por el usuario (30–100%; recomendado 50–80%).
pub fn set_threshold(v: u32) -> (bool, String) {
    if !(30..=100).contains(&v) {
        return (false, "el umbral debe estar entre 30% y 100%".into());
    }
    let node = cfg_get("node").unwrap_or_default();
    let enabled = cfg_get("enabled").map(|s| s == "true").unwrap_or(false);
    cfg_set(&node, enabled, v);
    crate::agent::set_activity(&format!("Umbral de bypass: {}%", v));
    (true, format!("umbral de batería: {}%", v))
}

/// Aplica bypass en el nodo indicado (o en el guardado).
pub fn set(enabled: bool, path: Option<&str>) -> (bool, String) {
    let node_path = path
        .map(|s| s.to_string())
        .or_else(|| cfg_get("node"))
        .unwrap_or_default();
    crate::agent::set_activity(&format!(
        "Carga bypass: {} en {}",
        if enabled { "activar" } else { "apagar" },
        node_path
    ));
    let Some(n) = node_by_path(&node_path) else {
        crate::agent::record("bypass", &node_path, "nodo no reconocido", "");
        return (false, "nodo no reconocido".into());
    };
    if n.risky {
        return (false, "nodo de riesgo: solo se puede probar, no activar".into());
    }
    if enabled && !charger_connected() {
        let m = "conecta el cargador para activar bypass".to_string();
        crate::agent::record("bypass", &node_path, "sin cargador", &m);
        return (false, m);
    }
    let threshold = cfg_num("threshold", 80);
    let pct = crate::telemetry::sample(false).bat_pct.unwrap_or(0);
    if enabled && threshold > 0 && pct < threshold {
        let m = format!("batería {}% por debajo del umbral {}%", pct, threshold);
        crate::agent::record("bypass", &node_path, "batería bajo el umbral", &m);
        return (false, m);
    }
    let value = if enabled { n.on } else { n.off };
    let p = std::path::Path::new(n.path);
    if !is_writable(p) {
        let m = "el nodo no es escribible".to_string();
        crate::agent::record("bypass", &node_path, "no escribible", &m);
        return (false, m);
    }
    if !write_verify(p, value) {
        let m = "el nodo no sostuvo el cambio".to_string();
        crate::agent::record("bypass", &node_path, "no se sostuvo", &m);
        return (false, m);
    }
    cfg_set(&node_path, enabled, threshold);
    crate::agent::set_activity(&format!(
        "Bypass {} y verificado",
        if enabled { "activado" } else { "apagado" }
    ));
    (
        true,
        format!(
            "bypass {} en {} (bateria {}%)",
            if enabled { "activado" } else { "desactivado" },
            node_path,
            pct
        ),
    )
}

/// Gestión autónoma de la carga directa (bypass). Solo respeta la preferencia
/// que el usuario dejó activada en modo manual (opt-in): con IA Motors activa
/// el daemon la aplica solo — cargador conectado y batería sobre el umbral →
/// activa; sin cargador o batería baja → desactiva, conservando la preferencia
/// para volver a activarla cuando se cumplan las condiciones. Nunca la activa
/// por iniciativa propia si el usuario no la habilitó.
pub fn autotick() {
    // Si el límite de carga gestiona el nodo, él manda: no pelearse.
    if crate::charge::manages_node() {
        return;
    }
    let enabled = cfg_get("enabled").map(|s| s == "true").unwrap_or(false);
    let node = cfg_get("node").unwrap_or_default();
    if node.is_empty() {
        return;
    }
    let charger = charger_connected();
    let pct = crate::telemetry::sample(false).bat_pct.unwrap_or(0);
    let threshold = cfg_num("threshold", 80);
    let is_active = active();
    if enabled && charger && pct >= threshold && !is_active {
        let (ok, _) = set(true, None);
        if ok {
            crate::journal::append_note(
                "bypass",
                "carga directa activada automáticamente (cargador y batería sobre el umbral)",
            );
        }
    } else if is_active && (!charger || pct < threshold.saturating_sub(5)) {
        let (ok, _) = set(false, None);
        // set(false) guarda enabled=false: se restaura la preferencia del
        // usuario para que la IA pueda reactivarla cuando toque.
        cfg_set(&node, enabled, threshold);
        if ok {
            crate::journal::append_note(
                "bypass",
                "carga directa desactivada automáticamente (sin cargador o batería bajo el umbral)",
            );
        }
    }
}

fn write_once(p: &std::path::Path, value: &str) -> bool {
    if !is_writable(p) {
        return false;
    }
    let mut f = match fs::OpenOptions::new().write(true).open(p) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut buf = String::with_capacity(value.len() + 1);
    buf.push_str(value);
    buf.push('\n');
    f.write_all(buf.as_bytes()).is_ok()
}

/// Prueba un nodo: anota el valor, escribe el valor opuesto, relee y restaura.
/// Usa una comprobacion rapida (400 ms) para no demorar el diagnostico; la
/// aplicacion real (set) verifica con la escritura estricta de write_verify.
pub fn test(path: &str) -> (bool, String) {
    let Some(n) = node_by_path(path) else {
        return (false, "nodo no reconocido".into());
    };
    let p = std::path::Path::new(n.path);
    if !p.exists() {
        return (false, "el nodo no existe en este dispositivo".into());
    }
    let before = read_trim(p).unwrap_or_default();
    let target = if before.trim() == n.on { n.off } else { n.on };
    if !is_writable(p) {
        return (false, format!("no escribible (valor actual: {})", before.trim()));
    }
    // Corriente de batería antes/durante/después: evidencia física de que
    // el nodo cambia el camino de carga (no solo un valor que se relee).
    let cur_ma = || -> Option<i32> {
        read_trim(std::path::Path::new("/sys/class/power_supply/battery/current_now"))
            .and_then(|s| s.parse::<i64>().ok())
            .map(|v| if v.abs() >= 10_000 { (v / 1000) as i32 } else { v as i32 })
    };
    let i_before = cur_ma();
    if !write_once(p, target) {
        return (
            false,
            format!("no aceptó la escritura (antes: {})", before.trim()),
        );
    }
    std::thread::sleep(std::time::Duration::from_millis(400));
    let after = read_trim(p).unwrap_or_default();
    let i_during = cur_ma();
    let ok = after.trim() == target;
    let restored_ok = write_once(p, &before);
    std::thread::sleep(std::time::Duration::from_millis(120));
    let restored = read_trim(p).unwrap_or_default();
    let i_after = cur_ma();
    let fmt_ma = |v: Option<i32>| v.map(|x| format!("{} mA", x)).unwrap_or_else(|| "—".into());
    let charger = charger_connected();
    let current_note = if charger {
        format!(
            " · corriente: antes {}, durante {}, tras restaurar {}",
            fmt_ma(i_before), fmt_ma(i_during), fmt_ma(i_after)
        )
    } else {
        " · sin cargador: la corriente no es concluyente".to_string()
    };
    (
        ok && restored_ok && restored.trim() == before.trim(),
        format!(
            "antes={} escrito={} leído={} restaurado={}{}",
            before.trim(),
            target,
            after.trim(),
            restored.trim(),
            current_note
        ),
    )
}

/// Prueba todos los nodos presentes y dice cuales sostienen el cambio.
/// Devuelve el nodo recomendado: primero el configurado si funciona y no es
/// riesgoso; si no, el primero seguro que funciona; si no, el primero que
/// funciona; si ninguno, cadena vacia.
pub fn diagnose() -> String {
    let cfg_node = cfg_get("node").unwrap_or_default();
    let mut results: Vec<String> = Vec::new();
    let mut first_ok_safe = String::new();
    let mut first_ok_any = String::new();
    let mut cfg_ok = false;
    let mut tested = 0u32;
    for n in scan_nodes().iter().filter(|n| n.exists) {
        tested += 1;
        let (ok, detail) = test(&n.path);
        if ok {
            if first_ok_any.is_empty() {
                first_ok_any = n.path.clone();
            }
            if !n.risky && first_ok_safe.is_empty() {
                first_ok_safe = n.path.clone();
            }
            if n.path == cfg_node && !n.risky {
                cfg_ok = true;
            }
        }
        results.push(format!(
            "{{\"path\":{},\"desc\":{},\"writable\":{},\"risky\":{},\"current\":{},\"state\":\"{}\",\"ok\":{},\"detail\":{}}}",
            json_str(&n.path),
            json_str(&n.desc),
            n.writable,
            n.risky,
            json_str(&n.current),
            n.state,
            ok,
            json_str(&detail)
        ));
    }
    let recommended = if cfg_ok {
        cfg_node
    } else if !first_ok_safe.is_empty() {
        first_ok_safe
    } else {
        first_ok_any
    };
    format!(
        "{{\"recommended\":{},\"tested\":{},\"results\":[{}]}}",
        json_str(&recommended),
        tested,
        results.join(",")
    )
}
