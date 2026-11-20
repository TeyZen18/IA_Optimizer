//! Herramientas avanzadas de Control: escáner LAN, uso de apps, permisos
//! (app ops), limpieza, clips de pantalla, copia de seguridad, fstrim y
//! velocidad de red. Todo std-only, con verificación y sin inventar datos:
//! si una fuente no existe en la ROM, se dice.

use crate::state::{self, State};
use std::fs;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const CLIP_DIR: &str = "/sdcard/IAOptimizer/clips";
const USAGE: &str = "/data/adb/ia_optimizer/usage.json";
const CLIP_PID: &str = "/data/adb/ia_optimizer/clip.pid";

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn jstr(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn sh(cmd: &str) -> String {
    Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .map(|o| {
            let mut s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let e = String::from_utf8_lossy(&o.stderr).trim().to_string();
            if !e.is_empty() {
                if !s.is_empty() { s.push(' '); }
                s.push_str(&e);
            }
            s
        })
        .unwrap_or_default()
}

/// Token seguro para interpolar en shell (paquetes, ops, modos).
fn safe_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

// ------------------------- fstrim -------------------------

pub fn fstrim() -> String {
    let out = sh("vdc fstrim dotrim 2>&1 || sm fstrim 2>&1");
    let ok = !out.contains("Error") && !out.contains("Exception");
    format!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(if ok { "Recorte de almacenamiento ejecutado" } else { &out }))
}

// ------------------------- velocidad de red -------------------------

fn net_bytes() -> (u64, u64) {
    let s = fs::read_to_string("/proc/net/dev").unwrap_or_default();
    let (mut rx, mut tx) = (0u64, 0u64);
    for line in s.lines().skip(2) {
        let Some((iface, rest)) = line.split_once(':') else { continue };
        if iface.trim() == "lo" { continue; }
        let v: Vec<u64> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        if v.len() >= 9 {
            rx += v[0];
            tx += v[8];
        }
    }
    (rx, tx)
}

/// Muestreo real de 600 ms (bloquea solo el hilo de la petición).
pub fn net_speed() -> String {
    let (r1, t1) = net_bytes();
    std::thread::sleep(Duration::from_millis(600));
    let (r2, t2) = net_bytes();
    let rx = (r2.saturating_sub(r1)) as f64 / 0.6 / 125_000.0;
    let tx = (t2.saturating_sub(t1)) as f64 / 0.6 / 125_000.0;
    format!("{{\"rx_mbps\":{:.2},\"tx_mbps\":{:.2}}}", rx, tx)
}

// ------------------------- escáner LAN -------------------------

fn arp_neighbors() -> Vec<(String, String, String)> {
    let s = fs::read_to_string("/proc/net/arp").unwrap_or_default();
    let mut out = Vec::new();
    for line in s.lines().skip(1) {
        let v: Vec<&str> = line.split_whitespace().collect();
        if v.len() >= 6 && v[3] != "00:00:00:00:00:00" {
            out.push((v[0].to_string(), v[3].to_string(), v[5].to_string()));
        }
    }
    out
}

/// Vecinos ARP reales (instantáneo, sin sondeo).
pub fn lan_json() -> String {
    let items: Vec<String> = arp_neighbors()
        .iter()
        .map(|(ip, mac, dev)| format!("{{\"ip\":{},\"mac\":{},\"iface\":{}}}", jstr(ip), jstr(mac), jstr(dev)))
        .collect();
    format!("{{\"neighbors\":[{}],\"count\":{}}}", items.join(","), items.len())
}

/// Sondeo ping del /24 local (POST /api/lan/scan). 32 hilos, ping de 1 s.
/// Devuelve los equipos que respondieron + su MAC si quedó en ARP.
pub fn lan_scan() -> String {
    // IP local por la ruta por defecto (sin ifs de red).
    let route = sh("ip route get 8.8.8.8 2>/dev/null");
    let src = route.split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .find(|w| w[0] == "src")
        .map(|w| w[1].to_string())
        .unwrap_or_default();
    let parts: Vec<&str> = src.split('.').collect();
    if parts.len() != 4 {
        return "{\"ok\":false,\"detail\":\"no se pudo determinar la subred local\"}".into();
    }
    let base = format!("{}.{}.{}", parts[0], parts[1], parts[2]);
    let hosts: Vec<String> = (1..255).map(|i| format!("{}.{}", base, i)).collect();
    let alive = Mutex::new(Vec::<String>::new());
    // 32 hilos en un único scope: cada uno sondea 8 direcciones con ping de
    // 1 s. El scope espera a todos antes de continuar (~2 s en total).
    std::thread::scope(|s| {
        for group in hosts.chunks(8) {
            let g: Vec<String> = group.to_vec();
            let a = &alive;
            s.spawn(move || {
                for ip in g {
                    let r = Command::new("ping")
                        .args(["-c", "1", "-W", "1", &ip])
                        .output();
                    if let Ok(o) = r {
                        if o.status.success() {
                            if let Ok(mut v) = a.lock() { v.push(ip); }
                        }
                    }
                }
            });
        }
    });
    let mut found = alive.lock().map(|v| v.clone()).unwrap_or_default();
    found.sort();
    let neigh = arp_neighbors();
    let items: Vec<String> = found
        .iter()
        .map(|ip| {
            let mac = neigh.iter().find(|(i, _, _)| i == ip).map(|(_, m, _)| m.clone()).unwrap_or_default();
            format!("{{\"ip\":{},\"mac\":{}}}", jstr(ip), jstr(&mac))
        })
        .collect();
    format!("{{\"ok\":true,\"subnet\":{},\"hosts\":[{}],\"count\":{}}}", jstr(&format!("{}.0/24", base)), items.join(","), items.len())
}

// ------------------------- uso de apps (medición propia) -------------------------

struct UsageSlot {
    day: u64,
    last: u64,
    totals: Vec<(String, u64)>,
}

fn usage_slot() -> &'static Mutex<UsageSlot> {
    static S: OnceLock<Mutex<UsageSlot>> = OnceLock::new();
    S.get_or_init(|| {
        let mut slot = UsageSlot { day: now() / 86400, last: now(), totals: Vec::new() };
        if let Ok(s) = fs::read_to_string(USAGE) {
            if let Some(d) = state::get_json_num(&s, "day") {
                if d as u64 == slot.day {
                    if let Some(body) = s.split("\"totals\":{").nth(1) {
                        let body = body.split('}').next().unwrap_or("");
                        for pair in body.split(',') {
                            if let Some((k, v)) = pair.split_once(':') {
                                let pkg = k.trim().trim_matches('"').to_string();
                                if let Ok(secs) = v.trim().parse::<u64>() {
                                    slot.totals.push((pkg, secs));
                                }
                            }
                        }
                    }
                }
            }
        }
        Mutex::new(slot)
    })
}

/// Acumula segundos en primer plano por paquete (lo llama el ciclo del agente).
/// Persiste como mucho una vez por minuto para no castigar el flash.
pub fn usage_tick(pkg: &str) {
    if pkg.is_empty() { return; }
    let Ok(mut u) = usage_slot().lock() else { return };
    let n = now();
    let day = n / 86400;
    if day != u.day {
        u.day = day;
        u.totals.clear();
    }
    let dt = n.saturating_sub(u.last).min(10);
    u.last = n;
    if dt == 0 { return; }
    if let Some(e) = u.totals.iter_mut().find(|(p, _)| p == pkg) {
        e.1 += dt;
    } else {
        u.totals.push((pkg.to_string(), dt));
    }
    if n % 60 < 3 {
        persist_usage(&u);
    }
}

fn persist_usage(u: &UsageSlot) {
    let items: Vec<String> = u.totals.iter().map(|(p, s)| format!("{}:{}", jstr(p), s)).collect();
    let body = format!("{{\"day\":{},\"totals\":{{{}}}}}", u.day, items.join(","));
    let tmp = format!("{}.tmp", USAGE);
    if fs::write(&tmp, body).is_ok() {
        let _ = fs::rename(&tmp, USAGE);
    }
}

pub fn usage_json() -> String {
    let Ok(u) = usage_slot().lock() else { return "{\"apps\":[]}".into() };
    let mut v = u.totals.clone();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    let items: Vec<String> = v.iter().take(30)
        .map(|(p, s)| format!("{{\"package\":{},\"seconds\":{}}}", jstr(p), s))
        .collect();
    format!("{{\"day\":{},\"apps\":[{}]}}", u.day, items.join(","))
}

// ------------------------- app ops (permisos por app) -------------------------

const OPS: &[&str] = &[
    "SYSTEM_ALERT_WINDOW", "WRITE_SETTINGS", "POST_NOTIFICATION",
    "RUN_IN_BACKGROUND", "RUN_ANY_IN_BACKGROUND", "CAMERA", "RECORD_AUDIO",
    "ACCESS_FINE_LOCATION", "READ_CONTACTS", "READ_SMS", "WAKE_LOCK",
];

pub fn appops_get(pkg: &str) -> String {
    if !safe_token(pkg) {
        return "{\"ok\":false,\"detail\":\"paquete invalido\"}".into();
    }
    let out = sh(&format!("cmd appops get {}", pkg));
    let mut items = Vec::new();
    for op in OPS {
        // "OP_NAME: mode" en la salida de cmd appops get.
        let mode = out.lines()
            .find(|l| l.trim_start().starts_with(&format!("{}:", op)))
            .and_then(|l| l.split_once(':'))
            .map(|(_, m)| m.trim().to_string())
            .unwrap_or_else(|| "default".into());
        items.push(format!("{{\"op\":{},\"mode\":{}}}", jstr(op), jstr(&mode)));
    }
    format!("{{\"ok\":true,\"package\":{},\"ops\":[{}]}}", jstr(pkg), items.join(","))
}

pub fn appops_set(pkg: &str, op: &str, mode: &str) -> String {
    if !safe_token(pkg) || !OPS.contains(&op) {
        return "{\"ok\":false,\"detail\":\"paquete u operacion invalidos\"}".into();
    }
    let mode = match mode { "allow" | "deny" | "ignore" | "default" => mode, _ => return "{\"ok\":false,\"detail\":\"modo invalido\"}".into() };
    let out = sh(&format!("cmd appops set {} {} {}", pkg, op, mode));
    let ok = !out.contains("Error") && !out.contains("Exception") && !out.contains("Unknown");
    // Read-back: el modo debe quedar como se pidió.
    let after = sh(&format!("cmd appops get {} {}", pkg, op));
    let verified = after.contains(mode) || (mode == "default" && !after.contains("allow"));
    format!("{{\"ok\":{},\"verified\":{},\"detail\":{}}}", ok && verified, verified, jstr(&after))
}

// ------------------------- limpieza -------------------------

pub fn clean(action: &str, pkg: &str, bytes: u64) -> String {
    match action {
        "trim" => {
            let b = if bytes == 0 { 1_073_741_824 } else { bytes.min(8_589_934_592) };
            let out = sh(&format!("cmd package trim-caches {}", b));
            let ok = !out.contains("Error") && !out.contains("Exception");
            format!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(if ok { "Cachés de apps recortadas" } else { &out }))
        }
        "stop" => {
            if !safe_token(pkg) {
                return "{\"ok\":false,\"detail\":\"paquete invalido\"}".into();
            }
            let out = sh(&format!("am force-stop {}", pkg));
            let ok = !out.contains("Error") && !out.contains("Exception");
            format!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(if ok { "App detenida" } else { &out }))
        }
        _ => "{\"ok\":false,\"detail\":\"accion desconocida\"}".into(),
    }
}

// ------------------------- clips de pantalla -------------------------

pub fn clip_start(limit: u64) -> String {
    if fs::read_to_string(CLIP_PID).ok().and_then(|s| s.trim().parse::<u32>().ok())
        .map(|pid| fs::metadata(format!("/proc/{}", pid)).is_ok()).unwrap_or(false)
    {
        return "{\"ok\":false,\"detail\":\"ya hay una grabacion en curso\"}".into();
    }
    let _ = fs::create_dir_all(CLIP_DIR);
    let limit = limit.clamp(5, 180);
    let name = format!("clip_{}.mp4", now());
    let path = format!("{}/{}", CLIP_DIR, name);
    // screenrecord se lanza detached: el daemon no debe bloquearse ni morir
    // con la petición. El pid queda registrado para poder pararlo.
    let cmd = format!(
        "nohup screenrecord --time-limit {} --bit-rate 8000000 {} >/dev/null 2>&1 & echo $! > {}",
        limit, path, CLIP_PID
    );
    sh(&cmd);
    format!("{{\"ok\":true,\"file\":{},\"limit_s\":{}}}", jstr(&name), limit)
}

pub fn clip_stop() -> String {
    if let Ok(s) = fs::read_to_string(CLIP_PID) {
        if let Ok(pid) = s.trim().parse::<u32>() {
            // SIGINT: screenrecord cierra el mp4 correctamente al recibirlo.
            sh(&format!("kill -2 {} 2>/dev/null", pid));
        }
    }
    let _ = fs::remove_file(CLIP_PID);
    "{\"ok\":true,\"detail\":\"grabacion detenida\"}".into()
}

pub fn clip_list() -> String {
    let recording = fs::read_to_string(CLIP_PID).ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .map(|pid| fs::metadata(format!("/proc/{}", pid)).is_ok())
        .unwrap_or(false);
    let mut files: Vec<(String, u64, u64)> = Vec::new();
    if let Ok(rd) = fs::read_dir(CLIP_DIR) {
        for e in rd.filter_map(|e| e.ok()) {
            let name = e.file_name().into_string().unwrap_or_default();
            if !name.ends_with(".mp4") { continue; }
            if let Ok(m) = e.metadata() {
                let t = m.modified().ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs()).unwrap_or(0);
                files.push((name, m.len(), t));
            }
        }
    }
    files.sort_by(|a, b| b.2.cmp(&a.2));
    let items: Vec<String> = files.iter().take(50)
        .map(|(n, s, t)| format!("{{\"name\":{},\"size_mb\":{:.1},\"t\":{}}}", jstr(n), *s as f64 / 1_048_576.0, t))
        .collect();
    format!("{{\"recording\":{},\"files\":[{}]}}", recording, items.join(","))
}

/// Sirve un clip por HTTP (el reproductor del sistema hace streaming).
pub fn clip_file(name: &str) -> Option<Vec<u8>> {
    if !safe_token(name) || !name.ends_with(".mp4") { return None; }
    fs::read(format!("{}/{}", CLIP_DIR, name)).ok()
}

// ------------------------- copia de seguridad -------------------------

pub fn backup(st: &State) -> String {
    let dev = crate::device::load();
    let cfg = fs::read_to_string(&st.cfg_path).unwrap_or_else(|_| "{}".into());
    let games = fs::read_to_string("/data/adb/ia_optimizer/app_settings.json").unwrap_or_else(|_| "{}".into());
    let weights = fs::read_to_string("/data/adb/ia_optimizer/weights.json").unwrap_or_else(|_| "{}".into());
    format!(
        "{{\"version\":1,\"device\":{},\"soc\":{},\"t\":{},\"config\":{},\"games\":{},\"weights\":{}}}",
        jstr(&dev.device), jstr(&dev.soc), now(), cfg, games, weights
    )
}

pub fn restore(st: &State, body: &str) -> String {
    let dev = crate::device::load();
    let bdev = state::get_json_str(body, "device").unwrap_or_default();
    let bsoc = state::get_json_str(body, "soc").unwrap_or_default();
    if bdev != dev.device || bsoc != dev.soc {
        return format!(
            "{{\"ok\":false,\"detail\":\"la copia es de otro dispositivo ({} / {}); este es {} / {}\"}}",
            jstr(&bdev), jstr(&bsoc), jstr(&dev.device), jstr(&dev.soc)
        );
    }
    let Some(cfg) = state::get_json_raw(body, "config").into_opt() else {
        return "{\"ok\":false,\"detail\":\"copia sin configuracion\"}".into();
    };
    // Validación mínima antes de escribir: perfil y preferencias reconocidos.
    if state::get_json_str(&cfg, "profile").is_none() {
        return "{\"ok\":false,\"detail\":\"configuracion invalida\"}".into();
    }
    let tmp = format!("{}.tmp", st.cfg_path.display());
    if fs::write(&tmp, &cfg).is_err() {
        return "{\"ok\":false,\"detail\":\"no se pudo escribir\"}".into();
    }
    let _ = fs::rename(&tmp, &st.cfg_path);
    if let Some(games) = state::get_json_raw(body, "games").into_opt() {
        let _ = fs::write("/data/adb/ia_optimizer/app_settings.json", games);
    }
    "{\"ok\":true,\"detail\":\"copia restaurada; reinicia el daemon para aplicar todo\"}".into()
}

/// Pequeño adaptador para tratar el JSON crudo opcional.
trait IntoOpt { fn into_opt(self) -> Option<String>; }
impl IntoOpt for String {
    fn into_opt(self) -> Option<String> {
        if self.is_empty() || self == "null" { None } else { Some(self) }
    }
}
