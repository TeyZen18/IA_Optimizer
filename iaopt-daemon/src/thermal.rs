//! Control térmico autónomo (hilo del daemon).
//!
//! Idea general tomada de las arquitecturas de reference (thermalcore): medir
//! dos sensores, fusionarlos, elegir un objetivo y actuar con un PID con
//! histéresis para no oscilar. Aquí se implementa con los nodos que el daemon
//! ya conoce y sin dependencias externas.
//!
//!   batería (décimas)  ──► EWMA ──► control ──► objetivo ──► PID ──► intensidad
//!   zona SoC (milésimas) ─► /100 ─►  50/50 ─┘
//!
//! La actuación es siempre limitante: si la intensidad es ~0 no se toca nada,
//! así que nunca compite con el perfil elegido por el usuario.

use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use iaopt_core::sysfs::{self, Probe};

/// Historia y estado publicados para la app.
#[derive(Default, Clone)]
pub struct Info {
    pub cycle: u64,
    pub battery_c: f32,
    pub soc_c: f32,
    pub control_c: f32,
    pub target_c: f32,
    pub intensity: f32,
    pub actuator: String,
    pub interventions: u64,
    pub screen_on: bool,
    pub cpu_load: f32,
    pub note: String,
}

static INFO: Mutex<Option<Info>> = Mutex::new(None);

pub fn info() -> Option<Info> {
    INFO.lock().ok()?.clone()
}

pub fn info_json() -> String {
    let i = match info() {
        Some(i) => i,
        None => {
            return "{\"cycle\":0,\"battery_c\":0,\"soc_c\":0,\"control_c\":0,\"target_c\":0,\
                    \"intensity\":0,\"actuator\":\"sin datos\",\"interventions\":0,\
                    \"screen_on\":true,\"cpu_load\":0,\"note\":\"sin datos\"}"
                .to_string()
        }
    };
    format!(
        "{{\"cycle\":{},\"battery_c\":{:.1},\"soc_c\":{:.1},\"control_c\":{:.1},\"target_c\":{:.1},\"intensity\":{:.2},\"actuator\":{},\"interventions\":{},\"screen_on\":{},\"cpu_load\":{:.3},\"note\":{}}}",
        i.cycle,
        i.battery_c,
        i.soc_c,
        i.control_c,
        i.target_c,
        i.intensity,
        jstr(&i.actuator),
        i.interventions,
        i.screen_on,
        i.cpu_load,
        jstr(&i.note)
    )
}

fn jstr(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ------------------------------- sensores --------------------------------

/// Temperatura de batería en °C (el nodo de Android va en décimas).
fn battery_temp() -> Option<f32> {
    for name in ["battery", "batt"] {
        let p = format!("/sys/class/power_supply/{}/temp", name);
        if let Ok(s) = fs::read_to_string(&p) {
            if let Ok(v) = s.trim().parse::<f32>() {
                if v > 0.0 && v < 1000.0 {
                    return Some(v / 10.0);
                }
            }
        }
    }
    None
}

/// Máximo de las zonas térmicas relacionadas con CPU/SoC/GPU, en °C.
///
/// Las zonas van en milégrados. Se descartan los valores no físicos (0 o el
/// centinela 0xFFFF... que somequen reporta como negativo) para que una zona
/// "muerta" no arrastre el máximo.
fn soc_temp() -> Option<f32> {
    let Ok(rd) = fs::read_dir("/sys/class/thermal") else {
        return None;
    };
    let mut max: Option<f32> = None;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with("thermal_zone") {
            continue;
        }
        let path = e.path();
        let Ok(t) = fs::read_to_string(path.join("type")) else {
            continue;
        };
        let lower = t.trim().to_ascii_lowercase();
        let cpu_like = ["cpu", "soc", "gpu", "tsens", "coretemp", "pkg_temp", "thermal"]
            .iter()
            .any(|k| lower.contains(k));
        if !cpu_like {
            continue;
        }
        let Ok(raw) = fs::read_to_string(path.join("temp")) else {
            continue;
        };
        let Ok(v) = raw.trim().parse::<i64>() else {
            continue;
        };
        if v <= 0 || v > 200_000 {
            continue;
        }
        let c = v as f32 / 1000.0;
        max = Some(max.map_or(c, |m: f32| m.max(c)));
    }
    max
}

fn cpu_load() -> f32 {
    let Ok(s) = fs::read_to_string("/proc/stat") else {
        return 0.0;
    };
    let Some(line) = s.lines().next() else {
        return 0.0;
    };
    let vals: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|x| x.parse().ok())
        .collect();
    if vals.len() < 4 {
        return 0.0;
    }
    let idle = vals[3];
    let total: u64 = vals.iter().sum();
    if total == 0 {
        return 0.0;
    }
    ((total - idle) as f32 / total as f32).clamp(0.0, 1.0)
}

// ------------------------------- decisión --------------------------------

const KP: f32 = 0.008;
const KI: f32 = 0.0005;
const KD: f32 = 0.02;
const DEADBAND: f32 = 3.0;
const EWMA: f32 = 0.3;
const INTEGRAL_LIMIT: f32 = 100.0;

struct Pid {
    integral: f32,
    prev: f32,
}

impl Pid {
    fn new() -> Self {
        Pid { integral: 0.0, prev: 0.0 }
    }
    fn reset(&mut self) {
        self.integral = 0.0;
        self.prev = 0.0;
    }
    /// 0..1: cuánto limitar. Por debajo del objetivo siempre libera.
    fn step(&mut self, control: f32, target: f32, dt: f32) -> f32 {
        if control <= target {
            self.reset();
            return 0.0;
        }
        let error = control - target;
        if error.abs() < DEADBAND {
            self.reset();
            return 0.0;
        }
        self.integral = (self.integral + error * dt).clamp(-INTEGRAL_LIMIT, INTEGRAL_LIMIT);
        let derivative = if dt > 0.0 { (error - self.prev) / dt } else { 0.0 };
        self.prev = error;
        let out = KP * error + KI * self.integral + KD * derivative;
        out.clamp(0.0, 1.0)
    }
}

/// Objetivo de control: modo inactivo, sostenido, ráfaga o enfriamiento.
fn target_for(control: f32, screen_on: bool, load_variance: f32) -> (&'static str, f32) {
    if !screen_on {
        return ("inactivo", 36.0);
    }
    if control >= 48.0 {
        return ("enfriamiento", 36.0);
    }
    if load_variance > 0.15 {
        return ("ráfaga", 44.0);
    }
    ("sostenido", 40.0)
}

// ------------------------------- actuación --------------------------------

struct Last {
    cap_path: String,
    cap_value: String,
    cap_before: String,
    mode_path: String,
    mode_value: String,
    mode_before: String,
}

/// Aplica la intensidad. Solo actúa cuando hay algo que cambiar: si la
/// intensidad es despreciable, se restaura lo que se hubiera limitado.
fn actuate(intensity: f32, knobs: &[sysfs::Knob]) -> (String, String) {
    const FLOOR: f32 = 0.05;
    static LAST: Mutex<Option<Last>> = Mutex::new(None);

    if intensity < FLOOR {
        // Fin de la limitación: dejar los nodos como estaban.
        let prev = LAST.lock().ok().and_then(|mut l| l.take());
        if let Some(p) = prev {
            if !p.cap_path.is_empty() && !p.cap_before.is_empty() {
                sysfs::write_verify(Path::new(&p.cap_path), &p.cap_before);
            }
            if !p.mode_path.is_empty() && !p.mode_before.is_empty() {
                sysfs::write_verify(Path::new(&p.mode_path), &p.mode_before);
            }
            return ("ninguno".into(), "limitación retirada: se restaura el estado previo".into());
        }
        return ("ninguno".into(), "sin limitación: dentro del objetivo".into());
    }

    // 1) Techo de frecuencia (si el kernel lo acepta).
    let mut rechazado = String::new();
    if let Some(k) = knobs
        .iter()
        .find(|k| k.writable && k.path.ends_with("scaling_max_freq"))
    {
        let base_rel = k.path.replace("scaling_max_freq", "cpuinfo_max_freq");
        let avail_rel = k.path.replace("scaling_max_freq", "scaling_available_frequencies");
        let base = knobs
            .iter()
            .find(|m| m.path == base_rel)
            .and_then(|m| m.current.as_ref())
            .and_then(|s| s.parse::<u64>().ok());
        let avail = knobs
            .iter()
            .find(|m| m.path == avail_rel)
            .and_then(|m| m.current.clone())
            .unwrap_or_default();
        if let Some(max) = base {
            let ratio = (1.0 - 0.6 * intensity).clamp(0.35, 1.0);
            let mut target = (max as f32 * ratio).round() as u64;
            if !avail.is_empty() {
                if let Some(n) = sysfs::nearest_freq(&avail, target) {
                    target = n;
                }
            }
            let value = target.to_string();
            let same = LAST
                .lock()
                .ok()
                .and_then(|l| l.as_ref().map(|l| l.cap_path == k.path && l.cap_value == value))
                .unwrap_or(false);
            if !same {
                if sysfs::write_verify(Path::new(&k.path), &value) {
                    if let Ok(mut l) = LAST.lock() {
                        *l = Some(Last {
                            cap_path: k.path.clone(),
                            cap_value: value.clone(),
                            cap_before: k.current.clone().unwrap_or_default(),
                            mode_path: String::new(),
                            mode_value: String::new(),
                            mode_before: String::new(),
                        });
                    }
                    return (
                        "frecuencia".into(),
                        format!("techo limitado a {} ({:.0}%)", value, ratio * 100.0),
                    );
                }
                // El kernel lo rechazó: se intenta el modo de energía antes de
                // dar el chip por perdido.
                rechazado = "el techo de frecuencia".into();
            } else {
                return ("frecuencia".into(), "techo ya aplicado".into());
            }
        }
    }

    // 2) Modo de energía del chip (discreto, pero es el limitador real en
    //    algunos SoC que bloquean scaling_max_freq).
    if let Some(k) = knobs
        .iter()
        .find(|k| k.writable && k.path.ends_with("cpufreq_power_mode"))
    {
        let mode = if intensity > 0.5 { "0" } else { "1" };
        let same = LAST
            .lock()
            .ok()
            .and_then(|l| l.as_ref().map(|l| l.mode_value == mode))
            .unwrap_or(false);
        if !same {
                if sysfs::write_verify(Path::new(&k.path), mode) {
                    if let Ok(mut l) = LAST.lock() {
                        *l = Some(Last {
                            cap_path: String::new(),
                            cap_value: String::new(),
                            cap_before: String::new(),
                            mode_path: k.path.clone(),
                            mode_value: mode.to_string(),
                            mode_before: k.current.clone().unwrap_or_default(),
                        });
                    }
                return (
                    "modo de energía".into(),
                    format!("modo {} (intensidad {:.0}%)", mode, intensity * 100.0),
                );
            }
        }
        return ("modo de energía".into(), "modo ya aplicado".into());
    }

    if !rechazado.is_empty() {
        let msg = format!("el kernel no aceptó {}", rechazado);
        warn_once(&msg);
        return ("ninguno".into(), msg);
    }
    let msg = "el kernel no expone un limitador usable".to_string();
    warn_once(&msg);
    ("ninguno".into(), msg)
}

/// Avisa una sola vez por motivo: si el chip no tiene limitador, el panel no
/// debe llenarse de la misma tarjeta cada tres segundos.
fn warn_once(msg: &str) {
    static LAST: Mutex<Option<String>> = Mutex::new(None);
    let mut guard = match LAST.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if guard.as_deref() == Some(msg) {
        return;
    }
    *guard = Some(msg.to_string());
    drop(guard);
    iaopt_core::tuner::push_warning(
        "/sys/devices/system/cpu/cpufreq",
        "limitador no disponible",
        msg,
    );
}

// ------------------------------- bucle ------------------------------------

pub fn run(period: Duration) {
    std::thread::spawn(move || {
        let mut pid = Pid::new();
        let mut smooth: Option<f32> = None;
        let mut loads: Vec<f32> = Vec::new();
        let mut cycle: u64 = 0;
        let mut interventions: u64 = 0;
        let mut last_intensity = 0.0f32;
        let mut last_actuator = "ninguno".to_string();
        loop {
            std::thread::sleep(period);
            cycle += 1;

            let bat = battery_temp();
            let soc = soc_temp();
            let screen_on = !crate::agent::screen_off();
            let load = cpu_load();
            loads.push(load);
            if loads.len() > 10 {
                loads.remove(0);
            }
            let mean = loads.iter().sum::<f32>() / loads.len().max(1) as f32;
            let variance = if loads.len() > 1 {
                (loads.iter().map(|v| (mean - v) * (mean - v)).sum::<f32>() / loads.len() as f32)
                    .sqrt()
            } else {
                0.0
            };

            // Fusión: la zona SoC se anticipa a la batería; si no hay zona, la
            // batería manda. Unidades igualadas a °C.
            let raw_control = match (bat, soc) {
                (Some(b), Some(s)) => {
                    if s > b {
                        b + 0.5 * (s - b)
                    } else {
                        b
                    }
                }
                (Some(b), None) => b,
                (None, Some(s)) => s,
                (None, None) => {
                    if let Ok(mut i) = INFO.lock() {
                        *i = Some(Info {
                            note: "sin sensores de temperatura".into(),
                            ..Info::default()
                        });
                    }
                    warn_once("no se pudo leer ningún sensor de temperatura");
                    continue;
                }
            };
            smooth = Some(match smooth {
                None => raw_control,
                Some(prev) => prev + EWMA * (raw_control - prev),
            });
            let control = smooth.unwrap_or(raw_control);

            let (mode, target) = target_for(control, screen_on, variance);
            let intensity = pid.step(control, target, period.as_secs_f32().max(0.1));

            let knobs = Probe::new("/").scan();
            let (actuator, note) = actuate(intensity, &knobs);
            if actuator != "ninguno" && actuator != last_actuator || (intensity - last_intensity).abs() > 0.3 {
                interventions += 1;
            }
            last_intensity = intensity;
            last_actuator = actuator.clone();

            // Presupuesto de calor sostenido: reloj real de sesión intensa
            // hasta la primera limitación (se mide, no se estima).
            crate::heatbudget::tick(load, intensity > 0.05, control);

            if let Ok(mut i) = INFO.lock() {
                *i = Some(Info {
                    cycle,
                    battery_c: bat.unwrap_or(0.0),
                    soc_c: soc.unwrap_or(0.0),
                    control_c: control,
                    target_c: target,
                    intensity,
                    actuator,
                    interventions,
                    screen_on,
                    cpu_load: load,
                    note: format!("{} · {}", mode, note),
                });
            }
        }
    });
}
