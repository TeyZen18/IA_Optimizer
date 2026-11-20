//! Estado del demonio: configuracion persistente, telemetria y perfil activo.

use iaopt_core::engine::{self, Metrics, Suggestion};
use iaopt_core::{profiles, sysfs, tuner, Profile};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const RUNTIME_DIR: &str = "/data/adb/ia_optimizer";

fn hist_path() -> PathBuf {
    PathBuf::from(RUNTIME_DIR).join("history.jsonl")
}
fn start_path() -> PathBuf {
    PathBuf::from(RUNTIME_DIR).join("daemon.start")
}
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Una muestra de telemetria para las graficas de la WebUI.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub t: u64,
    pub cpu: f32,
    pub temp: f32,
    pub charging: bool,
}

pub struct State {
    pub cfg_path: PathBuf,
    pub sys_root: PathBuf,
    pub profile: Mutex<Profile>,
    pub auto: AtomicBool,
    pub gaming: AtomicBool,
    pub ai_enabled: AtomicBool,
    pub features: Mutex<iaopt_core::Features>,
    /// Límite de frecuencia elegido por el usuario (0.0..1.0 del maximo).
    pub freq_cap: Mutex<f32>,
    /// Para qué optimiza la IA: "actuacion" | "equilibrado" | "bateria".
    pub goal: Mutex<String>,
    /// Cómo interviene ante una brecha medida: "calma" | "equilibrado" | "decisivo".
    pub behavior: Mutex<String>,
    /// App a la que se le aplicó su perfil propio (mientras está en primer plano).
    pub perapp: Mutex<Option<String>>,
    /// Perfil que estaba activo antes de ceder al perfil propio de la app.
    pub perapp_prev: Mutex<Option<Profile>>,
    pub last: Mutex<Option<Suggestion>>,
    pub start: Instant,
}

impl State {
    pub fn new(cfg_path: PathBuf, sys_root: PathBuf) -> Self {
        let cfg = load_config(&cfg_path);
        State {
            cfg_path,
            sys_root,
            profile: Mutex::new(cfg.profile),
            auto: AtomicBool::new(cfg.auto),
            gaming: AtomicBool::new(cfg.gaming),
            ai_enabled: AtomicBool::new(cfg.ai),
            features: Mutex::new(cfg.features),
            freq_cap: Mutex::new(cfg.freq_cap),
            goal: Mutex::new(cfg.goal),
            behavior: Mutex::new(cfg.behavior),
            perapp: Mutex::new(None),
            perapp_prev: Mutex::new(None),
            last: Mutex::new(None),
            start: Instant::now(),
        }
    }

    /// Techo de frecuencia del usuario (1.0 = sin limite).
    pub fn user_cap(&self) -> f32 {
        self.freq_cap.lock().map(|v| *v).unwrap_or(1.0)
    }

    /// Fija el techo de frecuencia del usuario y lo persiste.
    pub fn set_user_cap(&self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        if let Ok(mut c) = self.freq_cap.lock() {
            *c = v;
        }
        self.save();
    }

    pub fn probe(&self) -> sysfs::Probe {
        sysfs::Probe::new(self.sys_root.clone())
    }

    /// Objetivo de optimización elegido por el usuario (persistido).
    pub fn goal(&self) -> String {
        self.goal.lock().map(|v| v.clone()).unwrap_or_else(|_| "equilibrado".into())
    }

    pub fn set_goal(&self, v: &str) {
        let v = match v { "actuacion" | "bateria" => v, _ => "equilibrado" };
        if let Ok(mut g) = self.goal.lock() { *g = v.to_string(); }
        self.save();
    }

    /// Comportamiento de intervención elegido por el usuario (persistido).
    pub fn behavior(&self) -> String {
        self.behavior.lock().map(|v| v.clone()).unwrap_or_else(|_| "equilibrado".into())
    }

    pub fn set_behavior(&self, v: &str) {
        let v = match v { "calma" | "decisivo" => v, _ => "equilibrado" };
        if let Ok(mut b) = self.behavior.lock() { *b = v.to_string(); }
        self.save();
    }

    /// Telemetria real del dispositivo (mejor esfuerzo: nunca falla el servidor).
    pub fn metrics(&self) -> Metrics {
        let cpu_load = read_cpu_load().unwrap_or(0.0);
        let battery_temp_c = read_battery_temp().unwrap_or(0.0);
        let charging = read_charging();
        let fg_pkg = crate::read_foreground_app().unwrap_or_default();
        Metrics {
            cpu_load,
            battery_temp_c,
            battery_pct: read_battery_pct().unwrap_or(0),
            charging,
            gaming: self.gaming.load(Ordering::Relaxed),
            foreground: iaopt_core::engine::classify(&fg_pkg),
        }
    }

    /// Telemetria + registro en el historial compartido (para las graficas).
    /// El historial vive en un archivo: asi las llamadas CLI de la WebUI de
    /// KernelSU y el demonio ven las mismas muestras.
    pub fn record(&self) -> Metrics {
        let m = self.metrics();
        let _ = fs::create_dir_all(RUNTIME_DIR);
        let line = format!(
            "{{\"t\":{},\"cpu\":{:.3},\"temp\":{:.1},\"charging\":{}}}\n",
            unix_now(), m.cpu_load, m.battery_temp_c, m.charging
        );
        if let Ok(mut f) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(hist_path())
        {
            let _ = f.write_all(line.as_bytes());
        }
        self.prune_history();
        m
    }

    pub fn history(&self) -> Vec<Sample> {
        let Ok(s) = fs::read_to_string(hist_path()) else {
            return Vec::new();
        };
        s.lines().filter_map(parse_sample).collect()
    }

    /// Marca el arranque del demonio (lo hace `serve`, no cada CLI).
    pub fn mark_started(&self) {
        let _ = fs::create_dir_all(RUNTIME_DIR);
        let _ = fs::write(start_path(), unix_now().to_string());
    }

    pub fn uptime_secs(&self) -> u64 {
        if let Ok(s) = fs::read_to_string(start_path()) {
            if let Ok(t) = s.trim().parse::<u64>() {
                return unix_now().saturating_sub(t);
            }
        }
        self.start.elapsed().as_secs()
    }

    fn prune_history(&self) {
        let Ok(s) = fs::read_to_string(hist_path()) else { return };
        let lines: Vec<&str> = s.lines().collect();
        if lines.len() > 400 {
            let keep = &lines[lines.len() - 300..];
            let _ = fs::write(hist_path(), keep.join("\n") + "\n");
        }
    }

    /// Motor adaptativo con histeresis sobre el perfil vigente.
    pub fn decide(&self) -> Suggestion {
        let m = self.record();
        let prev = *self.profile.lock().unwrap();
        let s = engine::decide(Some(prev), &m);
        *self.last.lock().unwrap() = Some(s.clone());
        s
    }

    pub fn apply_profile(&self, p: Profile) -> Vec<tuner::Report> {
        let knobs = self.probe().scan();
        let cap = self.user_cap();
        let actions: Vec<_> = profiles::plan_with_cap(p, &knobs, Some(cap))
            .into_iter()
            .filter(|a| a.path != "-")
            .filter(|a| profiles::action_enabled(*self.features.lock().unwrap(), a))
            .collect();
        let root = self.sys_root.clone();
        let reps: Vec<_> = actions.iter().map(|a| tuner::apply(a, &root)).collect();
        // Ajustes de sistema segun el perfil (ahorro real de bateria).
        let extra = match p {
            Profile::Eco => "settings put global low_power 1; settings put global adaptive_battery_management_enabled 1",
            _ => "settings put global low_power 0",
        };
        let _ = std::process::Command::new("sh").arg("-c").arg(extra).output();
        *self.profile.lock().unwrap() = p;
        self.save();
        reps
    }

    pub fn save(&self) {
        let p = self.profile.lock().unwrap().as_str();
        let a = self.auto.load(Ordering::Relaxed);
        let ai = self.ai_enabled.load(Ordering::Relaxed);
        let f = *self.features.lock().unwrap();
        let json = format!(
            "{{\n  \"profile\": \"{}\",\n  \"auto\": {},\n  \"ai\": {},\n  \"gaming\": {},\n  \"freq_cap\": {:.2},\n  \"goal\": \"{}\",\n  \"behavior\": \"{}\",\n  \"features\": {{\"governor\": {},\"freq\": {},\"scheduler\": {}}}\n}}\n",
            p,
            a,
            ai,
            self.gaming.load(Ordering::Relaxed),
            self.user_cap(),
            self.goal(),
            self.behavior(),
            f.governor,
            f.freq,
            f.scheduler
        );
        let _ = fs::write(&self.cfg_path, json);
    }

    /// Diagnostico estilo "Atlas": comprobaciones sin escribir en el kernel.
    pub fn atlas(&self) -> Vec<(String, bool, String)> {
        let mut v = Vec::new();
        let cfg_ok = fs::read_to_string(&self.cfg_path)
            .ok()
            .and_then(|s| get_json_str(&s, "profile"))
            .is_some();
        v.push((
            "Configuración válida".into(),
            cfg_ok,
            if cfg_ok { "profile y modo leídos" } else { "no se pudo leer config.json" }.into(),
        ));
        let mod_ok = Path::new("/data/adb/modules/ia_optimizer/module.prop").exists();
        v.push((
            "Módulo montado".into(),
            mod_ok,
            if mod_ok { "module.prop presente" } else { "no se encontró el módulo" }.into(),
        ));
        let n = self.probe().scan().len();
        v.push((
            "Controles detectados".into(),
            n > 0,
            format!("{} interfaces activas", n),
        ));
        let fails = crate::journal::failures(20);
        v.push((
            "Escrituras recientes".into(),
            fails == 0,
            if fails == 0 {
                "sin fallos en las últimas 20 acciones".into()
            } else if fails == 1 {
                "1 acción no se sostuvo".into()
            } else {
                format!("{} acciones no se sostuvieron", fails)
            },
        ));
        v
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub profile: Profile,
    pub auto: bool,
    pub ai: bool,
    pub gaming: bool,
    pub features: iaopt_core::Features,
    /// Techo de frecuencia elegido por el usuario (0.0..1.0 del maximo).
    /// 1.0 = sin limite adicional; lo Combina con el limite del perfil.
    pub freq_cap: f32,
    /// Objetivo de optimización: actuacion | equilibrado | bateria.
    pub goal: String,
    /// Comportamiento de intervención: calma | equilibrado | decisivo.
    pub behavior: String,
}

fn load_config(path: &Path) -> Config {
    let def = || Config {
        profile: Profile::Balanced,
        auto: true,
        ai: true,
        gaming: false,
        features: iaopt_core::Features::default(),
        freq_cap: 1.0,
        goal: "equilibrado".into(),
        behavior: "equilibrado".into(),
    };
    let Ok(s) = fs::read_to_string(path) else {
        return def();
    };
    let features = iaopt_core::Features {
        governor: get_json_bool(&s, "governor").unwrap_or(true),
        freq: get_json_bool(&s, "freq").unwrap_or(true),
        scheduler: get_json_bool(&s, "scheduler").unwrap_or(true),
    };
    // las features pueden ir anidadas en "features": {...} o planas
    let features = if s.contains("\"features\"") {
        iaopt_core::Features {
            governor: get_json_bool(&s, "governor").unwrap_or(features.governor),
            freq: get_json_bool(&s, "freq").unwrap_or(features.freq),
            scheduler: get_json_bool(&s, "scheduler").unwrap_or(features.scheduler),
        }
    } else {
        features
    };
    Config {
        profile: get_json_str(&s, "profile")
            .and_then(|v| Profile::parse(&v))
            .unwrap_or(Profile::Balanced),
        auto: get_json_bool(&s, "auto").unwrap_or(true),
        ai: get_json_bool(&s, "ai").unwrap_or(true),
        gaming: get_json_bool(&s, "gaming").unwrap_or(false),
        features,
        freq_cap: get_json_num(&s, "freq_cap")
            .map(|v| (v as f32).clamp(0.0, 1.0))
            .unwrap_or(1.0),
        goal: get_json_str(&s, "goal")
            .filter(|v| v == "actuacion" || v == "bateria")
            .unwrap_or_else(|| "equilibrado".into()),
        behavior: get_json_str(&s, "behavior")
            .filter(|v| v == "calma" || v == "decisivo")
            .unwrap_or_else(|| "equilibrado".into()),
    }
}

/// Lector de JSON plano: basta para configuracion y API interna.
/// Valor JSON en bruto (admite comillas o booleano/number).
pub fn get_json_raw(s: &str, key: &str) -> String {
    let pat = format!("\"{}\"", key);
    let Some(i) = s.find(&pat) else {
        return String::new();
    };
    let rest = &s[i + pat.len()..];
    let Some(c) = rest.find(':') else {
        return String::new();
    };
    let rest = rest[c + 1..].trim_start();
    let end = rest
        .find([',', '}'])
        .unwrap_or(rest.len())
        .min(rest.len());
    rest[..end].trim().trim_matches('"').to_string()
}

pub fn get_json_str(s: &str, key: &str) -> Option<String> {
    let pat = format!("\"{}\"", key);
    let i = s.find(&pat)? + pat.len();
    let rest = &s[i..];
    let c = rest.find(':')? + 1;
    let rest = rest[c..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let rest = &rest[1..];
    let mut out = String::new();
    let mut esc = false;
    for ch in rest.chars() {
        if esc {
            out.push(ch);
            esc = false;
        } else if ch == '\\' {
            esc = true;
        } else if ch == '"' {
            break;
        } else {
            out.push(ch);
        }
    }
    Some(out)
}

pub fn get_json_bool(s: &str, key: &str) -> Option<bool> {
    let pat = format!("\"{}\"", key);
    let i = s.find(&pat)? + pat.len();
    let rest = &s[i..];
    let c = rest.find(':')? + 1;
    let v = rest[c..].trim_start();
    if v.starts_with("true") {
        Some(true)
    } else if v.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

pub fn get_json_num(s: &str, key: &str) -> Option<f64> {
    let pat = format!("\"{}\"", key);
    let i = s.find(&pat)? + pat.len();
    let rest = &s[i..];
    let c = rest.find(':')? + 1;
    let v = rest[c..].trim_start();
    let end = v
        .find(|ch: char| !(ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == 'e' || ch == 'E' || ch == '+'))
        .unwrap_or(v.len());
    v[..end].parse().ok()
}

fn parse_sample(line: &str) -> Option<Sample> {
    let t = get_json_num(line, "t")? as u64;
    let cpu = get_json_num(line, "cpu")? as f32;
    let temp = get_json_num(line, "temp")? as f32;
    let charging = get_json_bool(line, "charging").unwrap_or(false);
    Some(Sample { t, cpu, temp, charging })
}

// --- telemetria ---

fn read_cpu_load() -> Option<f32> {
    let s = fs::read_to_string("/proc/stat").ok()?;
    let line = s.lines().next()?;
    let vals: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|x| x.parse().ok())
        .collect();
    if vals.len() < 4 {
        return None;
    }
    let idle = vals[3];
    let total: u64 = vals.iter().sum();
    if total == 0 {
        return None;
    }
    Some((total - idle) as f32 / total as f32)
}

fn read_battery_temp() -> Option<f32> {
    // Android expone temp en decimas de grado
    for name in ["battery", "batt"] {
        let p = format!("/sys/class/power_supply/{}/temp", name);
        if let Ok(s) = fs::read_to_string(&p) {
            if let Ok(v) = s.trim().parse::<f32>() {
                return Some(v / 10.0);
            }
        }
    }
    None
}

fn read_battery_pct() -> Option<u8> {
    for name in ["battery", "batt"] {
        let p = format!("/sys/class/power_supply/{}/capacity", name);
        if let Ok(s) = fs::read_to_string(&p) {
            if let Ok(v) = s.trim().parse::<u8>() {
                return Some(v);
            }
        }
    }
    None
}

fn read_charging() -> bool {
    for name in ["battery", "batt"] {
        let p = format!("/sys/class/power_supply/{}/status", name);
        if let Ok(s) = fs::read_to_string(&p) {
            let v = s.trim().to_ascii_lowercase();
            // "Full" implica enchufe al 100%; se trata como cargando
            return v == "charging" || v == "full";
        }
    }
    false
}


// ---------- Utilidades para Game Space ----------

/// Porcentaje de CPU desde el arranque (aproximado, sin bloqueos).
pub fn cpu_pct() -> Option<u32> {
    let s = std::fs::read_to_string("/proc/stat").ok()?;
    let line = s.lines().next()?;
    let nums: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|x| x.parse().ok())
        .collect();
    if nums.len() < 4 {
        return None;
    }
    let idle = nums[3];
    let total: u64 = nums.iter().sum();
    if total == 0 {
        return None;
    }
    Some(((total - idle) as f32 / total as f32 * 100.0).clamp(0.0, 100.0) as u32)
}

/// Velocidad de red: requiere muestreo; el HUD muestra 0 si no hay datos.
pub fn net_mbps() -> Option<(f32, f32)> {
    let _ = std::fs::read_to_string("/proc/net/dev");
    None
}

/// Estado de conectividad vía settings globales.
pub fn quick_state(kind: &str) -> bool {
    let key = match kind {
        "wifi" => "wifi_on",
        "data" => "mobile_data",
        _ => return false,
    };
    std::process::Command::new("settings")
        .args(["get", "global", key])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
        .unwrap_or(false)
}
