//! Reglas de ajuste por perfil. Cada regla se traduce en acciones concretas
//! sobre rutas reales detectadas; si la ruta no existe, simplemente no se ejecuta.

use crate::sysfs::Knob;
use crate::{Profile};

#[derive(Debug, Clone)]
pub struct Action {
    pub path: String,
    pub value: String,
    /// Por que se propone este cambio (auditable desde la WebUI).
    pub why: String,
    /// Fijar el nodo en solo lectura tras aplicar (patron aZenith: el techo de
    /// frecuencia del perfil de rendimiento no lo revierte el daemon del vendor).
    pub pin: bool,
}

/// Gobernador preferido por perfil, en orden de preferencia.
fn governor_prefs(p: Profile) -> &'static [&'static str] {
    match p {
        Profile::Performance => &["performance", "schedutil", "walt", "interactive"],
        Profile::Balanced => &["schedutil", "walt", "interactive", "ondemand"],
        Profile::Eco => &["powersave", "schedutil", "conservative"],
    }
}

/// Una accion solo tiene sentido si su caracteristica esta activada.
pub fn action_enabled(f: crate::Features, a: &Action) -> bool {
    if a.path.ends_with("scaling_governor") {
        f.governor
    } else if a.path.ends_with("scaling_max_freq") || a.path.ends_with("scaling_min_freq") {
        f.freq
    } else if a.path.contains("queue/scheduler") {
        f.scheduler
    } else {
        true
    }
}

/// Planificador de E/S preferido por perfil, en orden de preferencia.
fn sched_prefs(p: Profile) -> &'static [&'static str] {
    match p {
        Profile::Performance => &["cfq", "bfq", "mq-deadline", "kyber", "deadline"],
        Profile::Balanced => &["mq-deadline", "kyber", "deadline", "cfq", "bfq"],
        Profile::Eco => &["noop", "deadline", "mq-deadline"],
    }
}

/// Construye la lista de acciones para un perfil dado los controles existentes.
///
/// Principio: solo se tocan governor y techo de frecuencia, que son reversibles
/// y verificables. Todo lo demas queda fuera del camino automatico.
pub fn plan(profile: Profile, knobs: &[Knob]) -> Vec<Action> {
    plan_with_cap(profile, knobs, None)
}

/// Igual que [`plan`] pero con un limitador adicional elegido por el usuario
/// (`user_cap`, 0.0..1.0 del maximo del cluster). El techo efectivo es el mas
/// bajo de los dos: el perfil manda en su terreno, el usuario en el suyo.
pub fn plan_with_cap(profile: Profile, knobs: &[Knob], user_cap: Option<f32>) -> Vec<Action> {
    let mut actions = Vec::new();
    let cap = match user_cap {
        Some(u) => profile.freq_cap().min(u.clamp(0.0, 1.0)),
        None => profile.freq_cap(),
    };

    // 1) Gobernador
    'outer: for gov in governor_prefs(profile) {
        for k in knobs {
            if k.domain == crate::sysfs::Domain::CpuGovernor
                && k.writable
                && k.allowed.iter().any(|a| a == gov)
            {
                actions.push(Action {
                    path: k.path.clone(),
                    value: gov.to_string(),
                    why: format!("perfil {:?}: gobernador {}", profile.as_str(), gov),
                    pin: false,
                });
                break 'outer;
            }
        }
    }

    // 2) Techo de frecuencia por politica/cluster
    if cap < 1.0 {
        // tomamos el maximo base de cada politica y lo recortamos
        for k in knobs {
            if !k.writable {
                continue;
            }
            if !(k.path.ends_with("scaling_max_freq")) {
                continue;
            }
            // buscar el maximo base, el minimo actual y la tabla de frecuencias
            let base_rel = k.path.replace("scaling_max_freq", "cpuinfo_max_freq");
            let min_rel = k.path.replace("scaling_max_freq", "scaling_min_freq");
            let avail_rel = k.path.replace("scaling_max_freq", "scaling_available_frequencies");
            if let Some(maxk) = knobs.iter().find(|m| m.path == base_rel) {
                let floor = knobs
                    .iter()
                    .find(|m| m.path == min_rel)
                    .and_then(|m| m.current.as_ref().and_then(|s| s.parse::<u64>().ok()))
                    .unwrap_or(0);
                let avail = knobs
                    .iter()
                    .find(|m| m.path == avail_rel)
                    .and_then(|m| m.current.clone())
                    .unwrap_or_default();
                if let (Some(base), Some(cur)) = (
                    maxk.current.as_ref().and_then(|s| s.parse::<u64>().ok()),
                    k.current.as_ref().and_then(|s| s.parse::<u64>().ok()),
                ) {
                    let ceiling = cur.min(base);
                    let mut target = ((ceiling as f32 * cap).round() as u64).max(floor);
                    if target > ceiling {
                        target = ceiling;
                    }
                    // El kernel solo acepta valores de su tabla: pedir uno
                    // intermedio dejaria el nodo en una frecuencia distinta.
                    if !avail.is_empty() {
                        if let Some(nearest) = crate::sysfs::nearest_freq(&avail, target) {
                            target = nearest;
                        }
                    }
                    if target < ceiling {
                        actions.push(Action {
                            path: k.path.clone(),
                            value: target.to_string(),
                            why: format!(
                                "perfil {}: techo {:.0}% del maximo ({})",
                                profile.as_str(),
                                cap * 100.0,
                                k.path
                            ),
                            pin: false,
                        });
                    }
                }
            }
        }
    }

    // 2.5) Modo de potencia del chip (algunos SoC no dejan escribir
    // `scaling_max_freq`; este nodo si responde y es el limitador real).
    if let Some(k) = knobs
        .iter()
        .find(|k| k.writable && k.path.ends_with("cpufreq_power_mode"))
    {
        let mode = match profile {
            Profile::Eco => "1",        // Low Power
            Profile::Performance => "3", // Performance (Sports)
            Profile::Balanced => "0",     // Default (Normal)
        };
        if k.current.as_deref() != Some(mode) {
            actions.push(Action {
                path: k.path.clone(),
                value: mode.to_string(),
                why: format!("perfil {}: modo de potencia del chip {}", profile.as_str(), mode),
                pin: false,
            });
        }
    }

    // 3) Planificador de E/S (si el dispositivo lo expone)
    'sched: for s in sched_prefs(profile) {
        for k in knobs {
            if k.domain == crate::sysfs::Domain::IoScheduler
                && k.writable
                && k.allowed.iter().any(|a| a == s)
            {
                actions.push(Action {
                    path: k.path.clone(),
                    value: s.to_string(),
                    why: format!("perfil {}: planificador de E/S {}", profile.as_str(), s),
                    pin: false,
                });
                break 'sched;
            }
        }
    }

    // 4) Rendimiento: desbloquear el techo de frecuencia y elevar limites termicos.
    if profile == Profile::Performance {
        for k in knobs {
            if !k.writable || !k.path.ends_with("scaling_max_freq") {
                continue;
            }
            let base_path = k.path.replace("scaling_max_freq", "cpuinfo_max_freq");
            if let Some(base) = knobs.iter().find(|m| m.path == base_path) {
                if let (Some(cur), Some(mx)) = (
                    k.current.as_ref().and_then(|s| s.parse::<u64>().ok()),
                    base.current.as_ref().and_then(|s| s.parse::<u64>().ok()),
                ) {
                    if mx > cur {
                        actions.push(Action {
                            path: k.path.clone(),
                            value: mx.to_string(),
                            why: "perfil performance: techo de frecuencia desbloqueado".into(),
                            // Fijado a 444: el daemon de potencia del vendor no
                            // puede revertirlo (patron aZenith).
                            pin: true,
                        });
                    }
                }
            }
        }
        // Limites termicos de las zonas de CPU (tope de seguridad 95 C).
        for k in knobs {
            if !k.writable || k.domain != crate::sysfs::Domain::Thermal {
                continue;
            }
            let lower = k.path.to_ascii_lowercase();
            if !lower.contains("trip_point") || !lower.ends_with("_temp") || !lower.contains("cpu") {
                continue;
            }
            if let Some(cur) = k.current.as_ref().and_then(|s| s.parse::<u64>().ok()) {
                if cur == 0 || cur > 200_000 {
                    continue;
                }
                let target = cur.saturating_add(5_000).min(95_000);
                if target > cur {
                    actions.push(Action {
                        path: k.path.clone(),
                        value: target.to_string(),
                        why: "perfil performance: limite termico +5C (tope 95C)".into(),
                        pin: false,
                    });
                }
            }
        }
    }

    // Si ya no hay nada que hacer, lo decimos claro.
    if actions.is_empty() {
        actions.push(Action {
            path: "-".into(),
            value: "-".into(),
            why: format!(
                "perfil {}: el kernel no expone controles aplicables; sin cambios",
                profile.as_str()
            ),
            pin: false,
        });
    }
    actions
}
