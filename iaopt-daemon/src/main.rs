//! IA Optimizer - demonio principal (binario `iaopt`).
//!
/// Uso:
///   iaopt serve [--port 8080] [--config /data/adb/modules/ia_optimizer/config.json] [--root /]

/// Salida tolerante a pipes cerrados (evita el pánico EPIPE de stdout).
macro_rules! out {
    ($($a:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), $($a)*);
    }};
}

mod agent;
mod automode;
mod bypass;
mod charge;
mod device;
mod fps;
mod games;
mod command;
mod heatbudget;
mod http;
mod journal;
mod perapp;
mod recorder;
mod repair;
mod renderer;
mod selftest;
mod session;
mod state;
mod telemetry;
mod thermal;
mod tools;
mod validate;
mod warnings;
mod watch;

use http::{Request, Response};
use iaopt_core::profiles::Action;
use iaopt_core::sysfs;
use iaopt_core::tuner;
use iaopt_core::Profile;
use state::State;
use std::sync::atomic::Ordering;
use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("serve");

    match cmd {
        "-V" | "--version" => {
            out!("IA Optimizer v{} (TeyZen)", env!("CARGO_PKG_VERSION"));
        }
        "scan" => {
            // Utilidad de diagnostico: lista interfaces sin tocar nada
            let root = arg_value(&args, "--root").unwrap_or_else(|| "/".into());
            let probe = sysfs::Probe::new(root);
            let knobs = probe.scan();
            out!("{} controles detectados:", knobs.len());
            for k in &knobs {
                out!(
                    "  [{:<14}] {} {}",
                    k.domain.as_str(),
                    if k.writable { "RW" } else { "RO" },
                    k.describe()
                );
            }
        }
        "selftest" => {
            // Prueba de la logica contra un sysfs falso; no toca el sistema real.
            let dir = arg_value(&args, "--dir").unwrap_or_else(|| "/data/local/tmp/iaopt_selftest".into());
            std::process::exit(selftest::run(PathBuf::from(dir).as_path()));
        }
        "state" => {
            let st = make_state(&args);
            out!("{}", state_json(&st));
        }
        "freq-cap" => {
            let st = make_state(&args);
            match args.get(2) {
                Some(v) => match v.parse::<f32>() {
                    Ok(x) => {
                        if ai_active(&st) { out!("{}", MANUAL_BLOCKED); return; }
                        st.set_user_cap(x);
                        out!("freq-cap: {:.0}%", st.user_cap() * 100.0);
                    }
                    Err(_) => {
                        eprintln!("valor invalido: {}", v);
                        std::process::exit(2);
                    }
                },
                None => out!("freq-cap: {:.0}%", st.user_cap() * 100.0),
            }
        }
        "history" => {
            let st = make_state(&args);
            out!("{}", history_json(&st));
        }
        "scan-json" => {
            let st = make_state(&args);
            out!("{}", scan_json(&st, true));
        }
        "ai" => {
            let st = make_state(&args);
            out!("{}", ai_json(&st));
        }
        "ai-run" => {
            let st = make_state(&args);
            out!("{}", ai_cycle(&st, true));
        }
        "ai-toggle" => {
            let Some(v) = args.get(2) else {
                eprintln!("uso: iaopt ai-toggle <0|1>");
                std::process::exit(2);
            };
            let en = !(v == "0" || v.eq_ignore_ascii_case("false"));
            let st = make_state(&args);
            let was = st.ai_enabled.load(Ordering::Relaxed);
            st.ai_enabled.store(en, Ordering::Relaxed);
            // Interruptor maestro: misma transición que /api/ai/toggle.
            if en != was {
                let _ = automode::set_auto(&st, en);
            }
            st.save();
            out!("{{\"ok\":true,\"ai\":{},\"auto\":{}}}", en, st.auto.load(Ordering::Relaxed));
        }
        "refresh" => {
            let Some(v) = args.get(2).and_then(|s| s.parse::<u32>().ok()) else {
                eprintln!("uso: iaopt refresh <0|30-240> (0 = automatico)");
                std::process::exit(2);
            };
            let cmd = if v == 0 {
                "settings delete system peak_refresh_rate; settings delete system min_refresh_rate".to_string()
            } else {
                format!("settings put system peak_refresh_rate {}; settings put system min_refresh_rate {}", v, v)
            };
            let ok = std::process::Command::new("sh").arg("-c").arg(&cmd)
                .output().map(|o| o.status.success()).unwrap_or(false);
            out!("{{\"ok\":{},\"value\":{}}}", ok, v);
        }
        "features" => {
            let st = make_state(&args);
            out!("{}", features_json(&st));
        }
        "repair" => {
            let st = make_state(&args);
            out!("{}", repair_json(&st));
        }
        "bypass" => {
            out!("{}", bypass::status());
        }
        "device" => {
            out!("{}", device_json());
        }
        "agent" => {
            let st = make_state(&args);
            out!("{}", agent::status(&st));
        }
        "clear-journal" => {
            let (ok, detail) = agent::clear_journal();
            out!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail));
        }
        "command-center" => {
            let st = make_state(&args);
            out!("{}", command::build(&st));
        }
        "evaluate" => {
            let st = make_state(&args);
            out!("{}", command::evaluate(&st));
        }
        "audio" => out!("{}", audio_json()),
        "display" => out!("{}", display_json()),
        "resolution" => {
            let Some(v) = args.get(2) else {
                eprintln!("uso: iaopt resolution <default|ANCHOxALTO>");
                std::process::exit(2);
            };
            let cmd = if v == "default" || v.is_empty() {
                "wm size reset".to_string()
            } else {
                format!("wm size {}", v)
            };
            let ok = std::process::Command::new("sh")
                .arg("-c")
                .arg(&cmd)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            out!(
                "{{\"ok\":{},\"detail\":{}}}",
                ok,
                jstr(if ok { "resolucion aplicada" } else { "no se pudo aplicar" })
            );
        }
        "games" => out!("{}", games::json()),
        "apps" => {
            let filter = args.get(2).map(|s| s.as_str()).unwrap_or("all");
            out!("{}", games::apps_json(filter));
        }
        "app-settings" => {
            let (Some(pkg), Some(key), Some(value)) = (args.get(2), args.get(3), args.get(4))
            else {
                eprintln!("uso: iaopt app-settings <paquete> <modo|cpu_max|gpu_max|io|hz|is_game|perf|bypass|preload|fps> <valor>");
                std::process::exit(2);
            };
            let st = make_state(&args);
            if key != "is_game" && ai_active(&st) { out!("{}", MANUAL_BLOCKED); return; }
            let (ok, detail) = games::set_setting(pkg, key, value);
            out!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail));
        }
        "app-apply" => {
            let Some(pkg) = args.get(2) else {
                eprintln!("uso: iaopt app-apply <paquete>");
                std::process::exit(2);
            };
            let st = make_state(&args);
            if ai_active(&st) { out!("{}", MANUAL_BLOCKED); return; }
            let (ok, detail) = games::apply(pkg);
            out!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail));
        }
        "app-info" => {
            let Some(pkg) = args.get(2) else {
                eprintln!("uso: iaopt app-info <paquete>");
                std::process::exit(2);
            };
            out!("{}", games::info_json(pkg));
        }
        "hud" => out!("{}", games::hud()),
        "launch" => {
            let Some(pkg) = args.get(2) else {
                eprintln!("uso: iaopt launch <paquete>");
                std::process::exit(2);
            };
            let (ok, detail) = games::launch(pkg);
            out!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail));
        }
        "quick" => {
            let (Some(a), Some(v)) = (args.get(2), args.get(3)) else {
                eprintln!("uso: iaopt quick <accion> <valor>");
                std::process::exit(2);
            };
            let st = make_state(&args);
            if ai_active(&st) { out!("{}", MANUAL_BLOCKED); return; }
            let (ok, detail) = games::quick(a, v);
            out!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail));
        }
        "agent-retry" => {
            let Some(path) = args.get(2) else {
                eprintln!("uso: iaopt agent-retry <ruta>");
                std::process::exit(2);
            };
            out!("{{\"ok\":{}}}", agent::clear(path));
        }
        "weights" => {
            out!("{}", agent::weights_json());
        }
        "blacklist" => {
            for b in agent::blacklist() {
                out!("{}", b);
            }
        }
        "renderer" => {
            if let Some(v) = args.get(2) {
                let (ok, detail) = renderer::set(v);
                out!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail));
            } else {
                out!("{}", renderer::status_json());
            }
        }
        "bypass-threshold" => {
            let Some(v) = args.get(2) else {
                eprintln!("Uso: iaopt bypass-threshold <30-100>");
                std::process::exit(2);
            };
            let st = make_state(&args);
            if ai_active(&st) { out!("{}", MANUAL_BLOCKED); return; }
            let val: u32 = v.parse().unwrap_or(0);
            let (ok, detail) = bypass::set_threshold(val);
            out!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail));
        }
        "bypass-set" => {
            let Some(v) = args.get(2) else {
                eprintln!("Uso: iaopt bypass-set <0|1> [ruta]");
                std::process::exit(2);
            };
            let st = make_state(&args);
            if ai_active(&st) { out!("{}", MANUAL_BLOCKED); return; }
            let on = v == "1";
            let path = args.get(3).map(|s| s.as_str());
            let (ok, detail) = bypass::set(on, path);
            out!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail));
        }
        "bypass-test" => {
            let Some(p) = args.get(2) else {
                eprintln!("Uso: iaopt bypass-test <ruta>");
                std::process::exit(2);
            };
            let st = make_state(&args);
            if ai_active(&st) { out!("{}", MANUAL_BLOCKED); return; }
            let (ok, detail) = bypass::test(p);
            out!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail));
        }
        "bypass-list" => {
            for n in bypass::scan_nodes() {
                if n.exists {
                    out!("[{}] {} = {} ({})", n.state, n.path, n.current, n.desc);
                }
            }
        }
        "bypass-diagnose" => {
            out!("{}", bypass::diagnose());
        }
        "telemetry" => {
            let st = make_state(&args);
            out!("{}", telemetry_json(&st, true));
        }
        "processes" => {
            out!("{}", processes_json());
        }
        "repair-apply" => {
            let Some(fix) = args.get(2) else {
                eprintln!("Uso: iaopt repair-apply <reparacion>");
                std::process::exit(2);
            };
            let st = make_state(&args);
            let (ok, detail) = repair::apply_fix(&st, fix);
            out!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail));
        }
        "features-set" => {
            let (Some(k), Some(v)) = (args.get(2), args.get(3)) else {
                eprintln!("uso: iaopt features-set <governor|freq|scheduler> <0|1>");
                std::process::exit(2);
            };
            let en = !(v == "0" || v.eq_ignore_ascii_case("false"));
            let st = make_state(&args);
            if ai_active(&st) { out!("{}", MANUAL_BLOCKED); return; }
            {
                let mut f = st.features.lock().unwrap();
                match k.as_str() {
                    "governor" => f.governor = en,
                    "freq" => f.freq = en,
                    "scheduler" => f.scheduler = en,
                    _ => {
                        eprintln!("caracteristica desconocida: {}", k);
                        std::process::exit(2);
                    }
                }
            }
            st.save();
            out!("{}", features_json(&st));
        }
        "apply" => {
            let Some(p) = args.get(2).and_then(|s| Profile::parse(s)) else {
                eprintln!("uso: iaopt apply <performance|balanced|eco>");
                std::process::exit(2);
            };
            let st = make_state(&args);
            if ai_active(&st) { out!("{}", MANUAL_BLOCKED); return; }
            let reps = st.apply_profile(p);
            warnings::drain();
            let applied = reps.iter().filter(|r| r.applied).count();
            let items: Vec<String> = reps
                .iter()
                .map(|r| {
                    format!(
                        "{{\"path\":{},\"requested\":{},\"before\":{},\"after\":{},\"applied\":{},\"note\":{}}}",
                        jstr(&r.path),
                        jstr(&r.requested),
                        jstr(r.before.as_deref().unwrap_or("-")),
                        jstr(r.after.as_deref().unwrap_or("-")),
                        r.applied,
                        jstr(&r.note)
                    )
                })
                .collect();
            out!(
                "{{\"ok\":true,\"profile\":\"{}\",\"applied\":{},\"total\":{},\"reports\":[{}]}}",
                p.as_str(),
                applied,
                reps.len(),
                items.join(",")
            );
        }
        "set-auto" => {
            let Some(v) = args.get(2) else {
                eprintln!("uso: iaopt set-auto <0|1>");
                std::process::exit(2);
            };
            let en = !(v == "0" || v.eq_ignore_ascii_case("false"));
            let st = make_state(&args);
            st.ai_enabled.store(en, Ordering::Relaxed);
            out!("{}", automode::set_auto(&st, en));
        }
        "set-gaming" => {
            let Some(v) = args.get(2) else {
                eprintln!("uso: iaopt set-gaming <0|1>");
                std::process::exit(2);
            };
            let en = !(v == "0" || v.eq_ignore_ascii_case("false"));
            let st = make_state(&args);
            if ai_active(&st) { out!("{}", MANUAL_BLOCKED); return; }
            st.gaming.store(en, Ordering::Relaxed);
            st.save();
            out!("{{\"ok\":true,\"gaming\":{}}}", en);
        }
        "set-knob" => {
            let (Some(path), Some(value)) = (args.get(2), args.get(3)) else {
                eprintln!("uso: iaopt set-knob <path> <value>");
                std::process::exit(2);
            };
            let st = make_state(&args);
            if ai_active(&st) { out!("{}", MANUAL_BLOCKED); return; }
            let value = snap_freq_value(&st, path, value);
            if let Err(e) = validate::check_scanned(&cached_scan(&st), path, &value) {
                out!("{{\"ok\":false,\"note\":{}}}", jstr(&e));
                return;
            }
            let action = Action {
                path: path.clone(),
                value: value.clone(),
                why: "CLI/WebUI".into(),
                pin: false,
            };
            let rep = tuner::apply(&action, &st.sys_root);
            if rep.applied {
                automode::record_override(
                    path,
                    rep.before.as_deref(),
                    rep.after.as_deref().unwrap_or(&value),
                );
                invalidate_scan();
            }
            out!(
                "{{\"ok\":{},\"path\":{},\"before\":{},\"after\":{},\"note\":{}}}",
                rep.applied,
                jstr(&rep.path),
                jstr(rep.before.as_deref().unwrap_or("-")),
                jstr(rep.after.as_deref().unwrap_or("-")),
                jstr(&rep.note)
            );
        }
        "serve" => {
            let port: u16 = arg_value(&args, "--port")
                .and_then(|v| v.parse().ok())
                .unwrap_or(8080);
            let cfg = PathBuf::from(
                arg_value(&args, "--config")
                    .unwrap_or_else(|| "/data/adb/modules/ia_optimizer/config.json".into()),
            );
            let root = PathBuf::from(arg_value(&args, "--root").unwrap_or_else(|| "/".into()));

            if let Some(p) = cfg.parent() {
                let _ = std::fs::create_dir_all(p);
            }

            let st = Arc::new(State::new(cfg.clone(), root));
            st.mark_started();
            agent::init();
            // Sesión de juego colgada de un crash: restaura DND/brillo/FPS
            // pendientes antes de empezar (nunca dejar el sistema tocado).
            session::restore_pending();
            // Corte de carga pendiente de un crash: restaurar siempre.
            charge::restore_pending();
            // Gestión de energía: límite de carga y carga inteligente (5 s).
            charge::run(std::time::Duration::from_secs(5));
            // Grabadora de sesiones: muestreo de 1 s mientras haya sesión.
            recorder::run();

            // Control termico autonomo: fusiona bateria + zona SoC y limita con
            // un PID con histéresis. Nunca sube rendimiento, solo limita.
            thermal::run(std::time::Duration::from_secs(3));

            // Perfil propio por app: si la app en primer plano tiene un perfil
            // elegido, se aplica y se restaura al salir.
            {
                let pa_st = st.clone();
                perapp::run(std::time::Duration::from_secs(2), pa_st);
            }

            // IA Motors: bucle autonomo de 5 fases cada 2 s (solo actua si la IA esta activa)
            {
                let ai_st = st.clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    if ai_st.ai_enabled.load(Ordering::Relaxed) {
                        agent::run_cycle(&ai_st);
                    } else {
                        agent::set_phase("Pausa (IA apagada)");
                        agent::set_activity("IA Motors en pausa");
                    }
                });
            }
            // Vigilancia autónoma: audita, detecta caídas y autocorrige cada 5 s.
            {
                let audit_st = st.clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    watch::audit_once(&audit_st);
                    // Gestión autónoma de la carga directa (solo la preferencia
                    // que el usuario dejó activada; nunca por iniciativa propia).
                    if audit_st.ai_enabled.load(Ordering::Relaxed) {
                        bypass::autotick();
                    }
                    // Modo automatico: aplica la decision del motor aunque la
                    // IA Motors este en pausa.
                    if audit_st.auto.load(Ordering::Relaxed) {
                        let cur = *audit_st.profile.lock().unwrap();
                        let s = audit_st.decide();
                        if s.profile != cur {
                            let applied = audit_st
                                .apply_profile(s.profile)
                                .iter()
                                .filter(|r| r.applied)
                                .count();
                            journal::append_note(
                                "auto",
                                &format!(
                                    "{} -> {} · {} ({} cambio(s))",
                                    cur.as_str(),
                                    s.profile.as_str(),
                                    s.reason,
                                    applied
                                ),
                            );
                        }
                    }
                });
            }
            let (profile, auto) = {
                let p = *st.profile.lock().unwrap();
                (p.as_str().to_string(), st.auto.load(std::sync::atomic::Ordering::Relaxed))
            };
            eprintln!(
                "[IA_Optimizer] perfil={} auto={} config={}",
                profile,
                auto,
                cfg.display()
            );

            let bind = format!("127.0.0.1:{}", port);
            let listener = match TcpListener::bind(&bind) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("[IA_Optimizer] no se pudo abrir {}: {}", bind, e);
                    std::process::exit(1);
                }
            };
            eprintln!("[IA_Optimizer] WebUI en http://{} (solo loopback)", bind);
            // El daemon escribe su propio PID (service.sh no puede saberlo con setsid).
            let _ = fs::write("/data/adb/ia_optimizer/daemon.pid", std::process::id().to_string());

            let handler_st = st.clone();
            http::serve(listener, move |req| route(req, &handler_st));
        }
        other => {
            eprintln!("comando desconocido: {}", other);
            eprintln!("usa: iaopt serve | iaopt scan");
            std::process::exit(2);
        }
    }
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    let i = args.iter().position(|a| a == key)?;
    args.get(i + 1).cloned()
}

fn make_state(args: &[String]) -> State {
    let cfg = PathBuf::from(
        arg_value(args, "--config")
            .unwrap_or_else(|| "/data/adb/modules/ia_optimizer/config.json".into()),
    );
    let root = PathBuf::from(arg_value(args, "--root").unwrap_or_else(|| "/".into()));
    if let Some(p) = cfg.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    State::new(cfg, root)
}

/// Escaneo cacheado: leer ~200 nodos del kernel en cada peticion hacia la API
/// lenta (y la app consulta cada pocos segundos). TTL corto + invalidacion
/// explicita tras cada escritura.
static SCAN_CACHE: std::sync::Mutex<Option<(std::time::Instant, Vec<iaopt_core::sysfs::Knob>)>> =
    std::sync::Mutex::new(None);

fn cached_scan(st: &State) -> Vec<iaopt_core::sysfs::Knob> {
    const TTL: std::time::Duration = std::time::Duration::from_secs(8);
    if let Ok(c) = SCAN_CACHE.lock() {
        if let Some((at, k)) = c.as_ref() {
            if at.elapsed() < TTL {
                return k.clone();
            }
        }
    }
    let k = st.probe().scan();
    if let Ok(mut c) = SCAN_CACHE.lock() {
        *c = Some((std::time::Instant::now(), k.clone()));
    }
    k
}

fn invalidate_scan() {
    if let Ok(mut c) = SCAN_CACHE.lock() {
        *c = None;
    }
}

/// Los cpuinfo_* son limites de fabrica de solo lectura: se usan por dentro
/// para planificar, pero no se muestran como controles (duplicaban las
/// tarjetas de frecuencia maxima/minima).
fn visible_knobs(knobs: &[iaopt_core::sysfs::Knob]) -> Vec<&iaopt_core::sysfs::Knob> {
    knobs
        .iter()
        .filter(|k| {
            let leaf = k.path.rsplit('/').next().unwrap_or("");
            leaf != "cpuinfo_max_freq" && leaf != "cpuinfo_min_freq"
        })
        .collect()
}

/// El kernel solo acepta frecuencias de su tabla: el valor pedido se ajusta
/// a la disponible mas cercana antes de escribir.
fn snap_freq_value(st: &State, path: &str, value: &str) -> String {
    if !(path.ends_with("scaling_max_freq") || path.ends_with("scaling_min_freq")) {
        return value.to_string();
    }
    let Ok(target) = value.trim().parse::<u64>() else {
        return value.to_string();
    };
    let leaf = path.rsplit('/').next().unwrap_or("");
    let avail_rel = path.replace(leaf, "scaling_available_frequencies");
    let avail =
        iaopt_core::sysfs::read_trim(&st.sys_root.join(avail_rel.trim_start_matches('/')))
            .unwrap_or_default();
    match iaopt_core::sysfs::nearest_freq(&avail, target) {
        Some(n) => n.to_string(),
        None => value.to_string(),
    }
}

/// IA Motors activa = modo autónomo: la IA gestiona el sistema y el control
/// manual queda bloqueado. Solo pantalla (renderizador, resolución, refresco),
/// diagnósticos y navegación siguen accesibles.
fn ai_active(st: &State) -> bool {
    st.ai_enabled.load(Ordering::Relaxed)
}

const MANUAL_BLOCKED: &str = "{\"ok\":false,\"blocked\":true,\"detail\":\"IA Motors está activa: apágala en Inicio para entrar en modo manual\"}";

fn route(req: &Request, st: &State) -> Response {
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => Response::html(include_str!("../webui/app.html")),
        ("GET", "/api/state") => Response::json(&state_json(st)),
        ("GET", "/api/history") => Response::json(&history_json(st)),
        ("GET", "/api/ai") => Response::json(&ai_json(st)),
        ("GET", "/api/telemetry") => Response::json(&telemetry_json(st, true)),
        ("GET", "/api/processes") => Response::json(&processes_json()),
        ("GET", "/api/repair") => Response::json(&repair_json(st)),
        ("GET", "/api/bypass") => Response::json(&bypass::status()),
        ("GET", "/api/bypass-diagnose") => Response::json(&bypass::diagnose()),
        ("GET", "/api/device") => Response::json(&device_json()),
        ("GET", "/api/audit") => Response::json(&watch::last_json()),
        ("GET", "/api/thermal") => Response::json(&thermal::info_json()),
        ("GET", "/api/heatbudget") => Response::json(&heatbudget::json()),
        ("GET", "/api/charge") => Response::json(&charge::status()),
        ("GET", "/api/charge-nodes") => Response::json(&charge::nodes_probe()),
        ("POST", "/api/charge") => Response::json(&charge::handle(&req.body)),
        ("GET", "/api/fps") => {
            let pkg = qparam(&req.query, "package");
            Response::json(&fps::json(pkg))
        }
        ("GET", "/api/recorder") => Response::json(&recorder::status()),
        ("POST", "/api/recorder") => Response::json(&recorder::handle(&req.body)),
        ("POST", "/api/prefs") => {
            // Objetivo y comportamiento de la IA: son preferencias del usuario
            // sobre cómo decide el motor, no escrituras manuales al kernel,
            // así que se aceptan incluso con IA Motors activa.
            if let Some(g) = state::get_json_str(&req.body, "goal") { st.set_goal(&g); }
            if let Some(b) = state::get_json_str(&req.body, "behavior") { st.set_behavior(&b); }
            Response::json(&format!(
                "{{\"ok\":true,\"goal\":\"{}\",\"behavior\":\"{}\"}}",
                st.goal(), st.behavior()
            ))
        }
        ("GET", "/api/warnings") => Response::json(&warnings::read_json()),
        ("GET", "/api/agent") => Response::json(&agent::status(st)),
        ("GET", "/api/agent-clear") => {
            let (ok, detail) = agent::clear_journal();
            Response::json(&format!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail)))
        }
        ("GET", "/api/command_center") => Response::json(&command::build(st)),
        ("GET", "/api/evaluate") => Response::json(&command::evaluate(st)),
        ("GET", "/api/audio") => Response::json(&audio_json()),
        ("GET", "/api/display") => Response::json(&display_json()),
        ("POST", "/api/resolution") => {
            let v = state::get_json_str(&req.body, "value").unwrap_or_default();
            let cmd = if v == "default" || v.is_empty() {
                "wm size reset".to_string()
            } else {
                format!("wm size {}", v)
            };
            let ok = std::process::Command::new("sh")
                .arg("-c")
                .arg(&cmd)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            Response::json(&format!(
                "{{\"ok\":{},\"detail\":{}}}",
                ok,
                jstr(if ok { "resolucion aplicada" } else { "no se pudo aplicar" })
            ))
        }
        ("POST", "/api/refresh") => {
            // Pantalla: exento del bloqueo manual (lo pide el usuario incluso
            // con IA Motors activa). 0 = automática (el sistema decide).
            let v = state::get_json_num(&req.body, "value").unwrap_or(0.0) as u32;
            let (ok, detail) = if v == 0 {
                let ok = std::process::Command::new("sh")
                    .arg("-c")
                    .arg("settings delete system peak_refresh_rate; settings delete system min_refresh_rate")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                (ok, "refresco automático: el sistema decide".to_string())
            } else if (30..=240).contains(&v) {
                let cmd = format!(
                    "settings put system peak_refresh_rate {}; settings put system min_refresh_rate {}",
                    v, v
                );
                let ok = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(&cmd)
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                (ok, format!("refresco fijado a {} Hz", v))
            } else {
                (false, "valor de refresco no válido".to_string())
            };
            Response::json(&format!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail)))
        }
        ("GET", "/api/games") => Response::json(&games::json()),
        ("GET", "/api/hud") => Response::json(&games::hud()),
        ("GET", "/api/apps") => {
            let q = req.query.as_str();
            let filter = q
                .split('&')
                .find_map(|p| p.strip_prefix("filter="))
                .unwrap_or("all");
            Response::json(&games::apps_json(filter))
        }
        ("GET", "/api/app-info") => {
            let q = req.query.as_str();
            let pkg = q
                .split('&')
                .find_map(|p| p.strip_prefix("package="))
                .unwrap_or("");
            Response::json(&games::info_json(pkg))
        }
        ("GET", "/api/app-icon") => {
            let q = req.query.as_str();
            let pkg = q
                .split('&')
                .find_map(|p| p.strip_prefix("package="))
                .unwrap_or("");
            match games::icon(pkg) {
                Some(p) => match std::fs::read(&p) {
                    Ok(b) => {
                        let ct = if p.extension().and_then(|e| e.to_str()) == Some("webp") {
                            "image/webp"
                        } else {
                            "image/png"
                        };
                        Response::image(ct, b)
                    }
                    Err(_) => Response::not_found(),
                },
                None => Response::not_found(),
            }
        }
        ("POST", "/api/app-settings") => {
            let pkg = state::get_json_str(&req.body, "package").unwrap_or_default();
            let key = state::get_json_str(&req.body, "key").unwrap_or_default();
            // Marcar juego alimenta la clasificación de la IA: siempre permitido.
            if key != "is_game" && ai_active(st) { return Response::json(MANUAL_BLOCKED); }
            let value = state::get_json_raw(&req.body, "value");
            let (ok, detail) = games::set_setting(&pkg, &key, &value);
            Response::json(&format!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail)))
        }
        ("POST", "/api/app-apply") => {
            if ai_active(st) { return Response::json(MANUAL_BLOCKED); }
            let pkg = state::get_json_str(&req.body, "package").unwrap_or_default();
            let (ok, detail) = games::apply(&pkg);
            Response::json(&format!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail)))
        }
        ("POST", "/api/launch") => {
            let pkg = state::get_json_str(&req.body, "package").unwrap_or_default();
            let (ok, detail) = games::launch(&pkg);
            Response::json(&format!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail)))
        }
        ("POST", "/api/quick") => {
            if ai_active(st) { return Response::json(MANUAL_BLOCKED); }
            let action = state::get_json_str(&req.body, "action").unwrap_or_default();
            let value = state::get_json_str(&req.body, "value").unwrap_or_default();
            let (ok, detail) = games::quick(&action, &value);
            Response::json(&format!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail)))
        }
        ("GET", "/api/renderer") => Response::json(&renderer::status_json()),
        // Sesión de juego in-game: preferencia explícita del usuario que la IA
        // respeta mientras el juego sigue en primer plano (no es un knob manual).
        ("GET", "/api/session") => Response::json(&session::status()),
        ("POST", "/api/session") => Response::json(&session::handle(&req.body)),
        // Herramientas avanzadas de Control (todas con fuentes reales).
        ("GET", "/api/tools/net") => Response::json(&tools::net_speed()),
        ("POST", "/api/tools/fstrim") => Response::json(&tools::fstrim()),
        ("GET", "/api/tools/lan") => Response::json(&tools::lan_json()),
        ("POST", "/api/tools/lan") => Response::json(&tools::lan_scan()),
        ("GET", "/api/tools/usage") => Response::json(&tools::usage_json()),
        ("GET", "/api/tools/appops") => {
            let pkg = qparam(&req.query, "package");
            Response::json(&tools::appops_get(pkg))
        }
        ("POST", "/api/tools/appops") => {
            let pkg = state::get_json_str(&req.body, "package").unwrap_or_default();
            let op = state::get_json_str(&req.body, "op").unwrap_or_default();
            let mode = state::get_json_str(&req.body, "mode").unwrap_or_default();
            Response::json(&tools::appops_set(&pkg, &op, &mode))
        }
        ("POST", "/api/tools/clean") => {
            let action = state::get_json_str(&req.body, "action").unwrap_or_default();
            let pkg = state::get_json_str(&req.body, "package").unwrap_or_default();
            let bytes = state::get_json_num(&req.body, "bytes").unwrap_or(0.0) as u64;
            Response::json(&tools::clean(&action, &pkg, bytes))
        }
        ("GET", "/api/tools/clip") => Response::json(&tools::clip_list()),
        ("POST", "/api/tools/clip") => {
            let action = state::get_json_str(&req.body, "action").unwrap_or_default();
            let limit = state::get_json_num(&req.body, "limit").unwrap_or(60.0) as u64;
            Response::json(&match action.as_str() {
                "start" => tools::clip_start(limit),
                "stop" => tools::clip_stop(),
                _ => "{\"ok\":false,\"detail\":\"accion desconocida\"}".to_string(),
            })
        }
        ("GET", "/clip") => {
            let name = qparam(&req.query, "name");
            match tools::clip_file(name) {
                Some(data) => Response::image("video/mp4", data),
                None => Response::not_found(),
            }
        }
        ("GET", "/api/tools/backup") => Response::json(&tools::backup(st)),
        ("POST", "/api/tools/restore") => Response::json(&tools::restore(st, &req.body)),
        ("GET", "/api/quick-state") => {
            let key = qparam(&req.query, "key");
            let kind = if key.starts_with("wifi") { "wifi" } else { "data" };
            Response::json(&format!("{{\"value\":{}}}", state::quick_state(kind)))
        }
        ("POST", "/api/renderer") => {
            let v = state::get_json_str(&req.body, "value").unwrap_or_default();
            let (ok, detail) = renderer::set(&v);
            Response::json(&format!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail)))
        }
        ("POST", "/api/agent") => {
            let path = state::get_json_str(&req.body, "path").unwrap_or_default();
            let ok = agent::clear(&path);
            Response::json(&format!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&if ok { "reintento habilitado" } else { "sin entrada" })))
        }
        ("POST", "/api/bypass") => {
            if ai_active(st) { return Response::json(MANUAL_BLOCKED); }
            let action = state::get_json_str(&req.body, "action").unwrap_or_default();
            let path = state::get_json_str(&req.body, "path");
            let (ok, detail) = match action.as_str() {
                "on" => bypass::set(true, path.as_deref()),
                "off" => bypass::set(false, path.as_deref()),
                "test" => bypass::test(path.as_deref().unwrap_or("")),
                "threshold" => {
                    let v = state::get_json_num(&req.body, "value").unwrap_or(0.0) as u32;
                    bypass::set_threshold(v)
                }
                _ => (false, "accion desconocida".into()),
            };
            Response::json(&format!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail)))
        }
        ("POST", "/api/bypass-set") => {
            if ai_active(st) { return Response::json(MANUAL_BLOCKED); }
            let enable = state::get_json_bool(&req.body, "enable").unwrap_or(false);
            let path = state::get_json_str(&req.body, "path");
            let (ok, detail) = bypass::set(enable, path.as_deref());
            Response::json(&format!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail)))
        }
        ("POST", "/api/bypass-test") => {
            if ai_active(st) { return Response::json(MANUAL_BLOCKED); }
            let path = state::get_json_str(&req.body, "path").unwrap_or_default();
            let (ok, detail) = bypass::test(&path);
            Response::json(&format!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail)))
        }
        ("POST", "/api/bypass-threshold") => {
            if ai_active(st) { return Response::json(MANUAL_BLOCKED); }
            let v = state::get_json_num(&req.body, "value").unwrap_or(0.0) as u32;
            let (ok, detail) = bypass::set_threshold(v);
            Response::json(&format!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail)))
        }
        ("POST", "/api/repair") => {
            let fix = state::get_json_str(&req.body, "fix").unwrap_or_default();
            let (ok, detail) = repair::apply_fix(st, &fix);
            invalidate_scan();
            Response::json(&format!("{{\"ok\":{},\"detail\":{}}}", ok, jstr(&detail)))
        }
        ("GET", "/api/features") => Response::json(&features_json(st)),

        ("POST", "/api/features") => {
            if ai_active(st) { return Response::json(MANUAL_BLOCKED); }
            let mut changed = false;
            {
                let mut f = st.features.lock().unwrap();
                for key in ["governor", "freq", "scheduler"] {
                    if let Some(v) = state::get_json_bool(&req.body, key) {
                        match key {
                            "governor" => f.governor = v,
                            "freq" => f.freq = v,
                            _ => f.scheduler = v,
                        }
                        changed = true;
                    }
                }
            }
            if changed {
                st.save();
            }
            Response::json(&features_json(st))
        }
        ("POST", "/api/audit/run") => {
            let a = watch::audit_once(st);
            let _ = a;
            Response::json(&watch::last_json())
        }
        ("POST", "/api/ai/run") => Response::json(&ai_cycle(st, true)),

        ("POST", "/api/ai/toggle") => {
            let en = state::get_json_bool(&req.body, "enabled").unwrap_or(true);
            let was = st.ai_enabled.load(Ordering::Relaxed);
            st.ai_enabled.store(en, Ordering::Relaxed);
            // IA Motors es el interruptor maestro del modo autónomo: al
            // encenderla guarda y limpia el estado manual; al apagarla lo
            // devuelve exacto (ver automode.rs).
            let transition = if en != was {
                automode::set_auto(st, en)
            } else {
                String::new()
            };
            st.save();
            invalidate_scan();
            Response::json(&format!(
                "{{\"ok\":true,\"ai\":{},\"auto\":{},\"transition\":{}}}",
                en,
                st.auto.load(Ordering::Relaxed),
                if transition.is_empty() { "null".to_string() } else { transition }
            ))
        }
        ("GET", "/api/scan") => Response::json(&scan_json(st, req.query.contains("force=1"))),

        ("POST", "/api/profile") => {
            if ai_active(st) { return Response::json(MANUAL_BLOCKED); }
            let Some(p) = state::get_json_str(&req.body, "profile").and_then(|v| iaopt_core::Profile::parse(&v)) else {
                return Response::text(400, "perfil invalido");
            };
            // Anotar anulacion del usuario para la autoauditoria
            let _ = journal::append(
                p.as_str(),
                "usuario",
                0.9,
                "usuario",
                &read_foreground_app().unwrap_or_default(),
                p.as_str(),
                "-",
                "manual",
                true,
                "manual",
                "anulacion del usuario",
                0.0,
                0.0,
            );
            let reps = st.apply_profile(p);
            warnings::drain();
            invalidate_scan();
            let mut applied = 0;
            for r in &reps {
                if r.applied {
                    applied += 1;
                }
            }
            Response::json(&format!(
                "{{\"ok\":true,\"profile\":\"{}\",\"applied\":{},\"total\":{}}}",
                p.as_str(),
                applied,
                reps.len()
            ))
        }

        ("POST", "/api/auto") => {
            let en = state::get_json_bool(&req.body, "enabled").unwrap_or(true);
            // El modo automático y IA Motors son el mismo interruptor maestro.
            st.ai_enabled.store(en, Ordering::Relaxed);
            let resp = automode::set_auto(st, en);
            st.save();
            invalidate_scan();
            Response::json(&resp)
        }

        ("POST", "/api/freq-cap") => {
            if ai_active(st) { return Response::json(MANUAL_BLOCKED); }
            let Some(v) = state::get_json_num(&req.body, "value") else {
                return Response::text(400, "valor invalido");
            };
            st.set_user_cap(v as f32);
            // Reaplicar el perfil vigente para que el nuevo techo entre en juego.
            let p = *st.profile.lock().unwrap();
            let _ = st.apply_profile(p);
            warnings::drain();
            invalidate_scan();
            Response::json(&format!("{{\"ok\":true,\"freq_cap\":{:.2}}}", st.user_cap()))
        }

        ("POST", "/api/gaming") => {
            if ai_active(st) { return Response::json(MANUAL_BLOCKED); }
            let en = state::get_json_bool(&req.body, "enabled").unwrap_or(true);
            st.gaming.store(en, std::sync::atomic::Ordering::Relaxed);
            st.save();
            Response::json(&format!("{{\"ok\":true,\"gaming\":{}}}", en))
        }

        ("POST", "/api/knob") => {
            if ai_active(st) { return Response::json(MANUAL_BLOCKED); }
            let (Some(path), Some(value)) = (
                state::get_json_str(&req.body, "path"),
                state::get_json_str(&req.body, "value"),
            ) else {
                return Response::text(400, "se requieren path y value");
            };
            // Frecuencias: ajustar a la tabla real del chip antes de validar.
            let value = snap_freq_value(st, &path, &value);
            // Validacion estricta antes de tocar el kernel.
            if let Err(e) = validate::check_scanned(&cached_scan(st), &path, &value) {
                iaopt_core::tuner::push_warning(
                    &path,
                    "no escribible",
                    &format!("escritura manual rechazada: {}", e),
                );
                let _ = warnings::drain();
                return Response::text(400, &e);
            }
            let action = iaopt_core::profiles::Action {
                path: path.clone(),
                value: value.clone(),
                why: "escritura manual desde WebUI".into(),
                pin: false,
            };
            let rep = iaopt_core::tuner::apply(&action, &st.sys_root);
            if rep.applied {
                // El ajuste manual queda registrado para poder devolverlo al
                // desactivar el modo automático, y en el diario del usuario.
                automode::record_override(
                    &path,
                    rep.before.as_deref(),
                    rep.after.as_deref().unwrap_or(&value),
                );
                let prof = { st.profile.lock().unwrap().as_str().to_string() };
                journal::append(
                    &prof,
                    "ajuste manual",
                    0.9,
                    "usuario",
                    "",
                    "manual",
                    &path,
                    &value,
                    true,
                    &path,
                    "escritura manual",
                    0.0,
                    0.0,
                );
                invalidate_scan();
            } else {
                iaopt_core::tuner::push_warning(&rep.path, "sin efecto", &rep.note);
                let _ = warnings::drain();
            }
            Response::json(&format!(
                "{{\"ok\":{},\"path\":{},\"before\":{},\"after\":{},\"note\":{}}}",
                rep.applied,
                jstr(&rep.path),
                jstr(rep.before.as_deref().unwrap_or("-")),
                jstr(rep.after.as_deref().unwrap_or("-")),
                jstr(&rep.note)
            ))
        }

        _ => Response::text(404, "no encontrado"),
    }
}

/// Parámetro de query sin dependencias (mismo patrón que el resto de rutas).
fn qparam<'a>(query: &'a str, key: &str) -> &'a str {
    let prefix = format!("{}=", key);
    query.split('&').find_map(|p| p.strip_prefix(&prefix)).unwrap_or("")
}

fn state_json(st: &State) -> String {
    // Nota: las lecturas (features, uptime, pid) se hacen en locales antes de
    // formatear; dentro de format! con el lock de profile en mano se observó
    // una cuelgue en algunos dispositivos.
    let m = st.record();
    let s = st.decide();
    let knobs = visible_knobs(&cached_scan(st)).len();
    let p = st.profile.lock().unwrap();
    let feat = *st.features.lock().unwrap();
    let up = st.uptime_secs();
    let pid = fs::read_to_string("/data/adb/ia_optimizer/daemon.pid")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or_else(|| std::process::id());
    let app = read_foreground_app().unwrap_or_default();
    // Las lecturas con lock se hacen en locales antes de formatear (ver nota).
    let cap = st.user_cap();
    format!(
        "{{\"profile\":\"{}\",\"auto\":{},\"ai\":{},\"gaming\":{},\"freq_cap\":{:.2},\"goal\":\"{}\",\"behavior\":\"{}\",\"features\":{{\"governor\":{},\"freq\":{},\"scheduler\":{}}},\"knobs\":{},\"pid\":{},\"uptime\":{},\"app\":{},\"version\":\"{}\",\"metrics\":{{\"cpu_load\":{:.3},\"battery_temp_c\":{:.1},\"battery_pct\":{},\"charging\":{}}},\"suggestion\":{{\"profile\":\"{}\",\"reason\":{},\"confidence\":{:.2}}}}}",
        p.as_str(),
        st.auto.load(std::sync::atomic::Ordering::Relaxed),
        st.ai_enabled.load(std::sync::atomic::Ordering::Relaxed),
        st.gaming.load(std::sync::atomic::Ordering::Relaxed),
        cap,
        st.goal(),
        st.behavior(),
        feat.governor,
        feat.freq,
        feat.scheduler,
        knobs,
        pid,
        up,
        jstr(&app),
        env!("CARGO_PKG_VERSION"),
        m.cpu_load,
        m.battery_temp_c,
        m.battery_pct,
        m.charging,
        s.profile.as_str(),
        jstr(&s.reason),
        s.confidence
    )
}

fn features_json(st: &State) -> String {
    let f = *st.features.lock().unwrap();
    format!(
        "{{\"governor\":{},\"freq\":{},\"scheduler\":{}}}",
        f.governor, f.freq, f.scheduler
    )
}

fn n_u64(v: Option<u64>) -> String {
    v.map(|x| x.to_string()).unwrap_or_else(|| "null".into())
}
fn n_f32(v: Option<f32>) -> String {
    v.map(|x| format!("{:.1}", x)).unwrap_or_else(|| "null".into())
}

/// Cubierta de mando: frecuencias, nucleos, memoria, zram, almacenamiento, red,
/// refresco, voltaje y zonas termicas en vivo.
fn telemetry_json(_st: &State, net: bool) -> String {
    let e = telemetry::sample(net);
    let low_power = std::process::Command::new("sh")
        .arg("-c")
        .arg("settings get global low_power 2>/dev/null")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    // Potencia instantanea de la bateria: W = V * A (con signo: negativo = uso).
    let power_w = match (e.voltage_uv, e.bat_current_ma) {
        (Some(uv), Some(ma)) => Some((uv as f64 / 1_000_000.0) * (ma as f64 / 1000.0)),
        _ => None,
    };
    let tz: Vec<String> = e
        .thermal_zones
        .iter()
        .map(|(n, t, v)| format!("{{\"zone\":\"{}\",\"type\":{},\"temp_c\":{}}}", n, jstr(t), v))
        .collect();
    let cl: Vec<String> = e
        .cpu_clusters
        .iter()
        .map(|c| {
            format!(
                "{{\"policy\":{},\"min_mhz\":{},\"max_mhz\":{},\"cur_mhz\":{}}}",
                jstr(&c.policy),
                c.min_mhz,
                c.max_mhz,
                c.cur_mhz
            )
        })
        .collect();
    format!(
        "{{\"cpu\":{{\"min_mhz\":{},\"max_mhz\":{},\"cur_mhz\":{}}},\"clusters\":[{}],\"cores_online\":{},\"device\":{},\"soc\":{},\"cpu_model\":{},\"mem\":{{\"total_mb\":{},\"avail_mb\":{},\"pressure\":{}}},\"swap\":{{\"total_mb\":{},\"free_mb\":{}}},\"zram\":{{\"total_mb\":{},\"used_mb\":{}}},\"storage\":{{\"total_mb\":{},\"used_mb\":{}}},\"net\":{{\"rx_mbps\":{},\"tx_mbps\":{}}},\"refresh_hz\":{},\"voltage_uv\":{},\"low_power\":{},\"gpu_mhz\":{},\"gpu_util\":{},\"bat\":{{\"temp_c\":{},\"pct\":{},\"charging\":{},\"current_ma\":{},\"power_w\":{}}},\"thermal\":[{}]}}",
        n_u64(e.cpu_min_mhz),
        n_u64(e.cpu_max_mhz),
        n_u64(e.cpu_cur_mhz),
        cl.join(","),
        jstr(&e.cores_online.unwrap_or_default()),
        jstr(&e.device),
        jstr(&e.soc),
        jstr(&e.cpu_model),
        e.mem_total_mb,
        e.mem_avail_mb,
        n_f32(e.mem_pressure),
        e.swap_total_mb,
        e.swap_free_mb,
        e.zram_total_mb,
        e.zram_used_mb,
        e.storage_total_mb,
        e.storage_used_mb,
        n_f32(e.rx_mbps),
        n_f32(e.tx_mbps),
        n_f32(e.refresh_hz),
        n_u64(e.voltage_uv),
        jstr(&low_power),
        n_u64(e.gpu_mhz),
        e.gpu_util.map(|v| v.to_string()).unwrap_or_else(|| "null".into()),
        n_f32(e.bat_temp_c),
        n_u64(e.bat_pct.map(|x| x as u64)),
        e.charging.unwrap_or(false),
        e.bat_current_ma.map(|v| v.to_string()).unwrap_or_else(|| "null".into()),
        power_w.map(|v| format!("{:.2}", v)).unwrap_or_else(|| "null".into()),
        tz.join(",")
    )
}

/// Procesos por consumo de CPU (para el monitor de tareas).
fn processes_json() -> String {
    let p = telemetry::top_processes(40);
    let items: Vec<String> = p
        .iter()
        .map(|x| {
            format!(
                "{{\"pid\":{},\"name\":{},\"cpu\":{:.1},\"rss_mb\":{:.1}}}",
                x.pid,
                jstr(&x.name),
                x.cpu,
                x.rss_mb
            )
        })
        .collect();
    format!("{{\"processes\":[{}]}}", items.join(","))
}

/// Perfil de dispositivo (SoC, GPU, capacidades) para adaptacion universal.
fn device_json() -> String {
    let d = device::load();
    let feats = d.features.iter().map(|f| jstr(f)).collect::<Vec<_>>().join(",");
    let kv = d.android.split_whitespace().take(3).collect::<Vec<_>>().join(" ");
    let hint = match d.db_hint() {
        Some((v, f)) => format!("{{\"vendor\":{},\"features\":{}}}", jstr(v), jstr(f)),
        None => "null".to_string(),
    };
    format!(
        "{{\"device\":{},\"soc\":{},\"vendor\":{},\"gpu\":{},\"android\":{},\"kernel_version\":{},\"features\":[{}],\"db_hint\":{}}}",
        jstr(&d.device),
        jstr(&d.soc),
        jstr(&d.soc_vendor),
        jstr(&d.gpu),
        jstr(&d.kernel),
        jstr(&kv),
        feats,
        hint
    )
}

/// Proceso de IA Motor (API interna, sin usar; el estado vive en agent.rs).

/// Diagnostico de reparacion autonoma (sin escribir).
fn repair_json(st: &State) -> String {
    let f = repair::diagnose(st);
    let items: Vec<String> = f
        .iter()
        .map(|x| {
            format!(
                "{{\"name\":\"{}\",\"ok\":{},\"detail\":{},\"fix\":{}}}",
                x.name,
                x.ok,
                jstr(&x.detail),
                x.fix.as_ref().map(|s| jstr(s)).unwrap_or_else(|| "null".into())
            )
        })
        .collect();
    let bl = fs::read_to_string("/data/adb/ia_optimizer/repair.blacklist").unwrap_or_default();
    let blv: Vec<String> = bl
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| jstr(l.trim()))
        .collect();
    format!(
        "{{\"findings\":[{}],\"blacklist\":[{}]}}",
        items.join(","),
        blv.join(",")
    )
}

/// App en primer plano (para el aprendizaje por app).
///
/// `dumpsys` es un fork relativamente caro y antes se ejecutaba una vez por
/// ciclo del agente, otra por telemetria y otra por cada peticion HTTP. Aquí se
/// cachea el resultado unos segundos: el paquete no cambia tanto y todas esas
/// llamadas ahora comparten una sola lectura.
pub fn read_foreground_app() -> Option<String> {
    static CACHE: std::sync::Mutex<Option<(String, std::time::Instant)>> =
        std::sync::Mutex::new(None);
    const TTL: std::time::Duration = std::time::Duration::from_millis(1200);

    if let Ok(c) = CACHE.lock() {
        if let Some((pkg, at)) = c.as_ref() {
            if at.elapsed() < TTL {
                return Some(pkg.clone());
            }
        }
    }
    let found = read_foreground_app_uncached();
    if let Some(pkg) = found.as_ref() {
        if let Ok(mut c) = CACHE.lock() {
            *c = Some((pkg.clone(), std::time::Instant::now()));
        }
    }
    found
}

fn read_foreground_app_uncached() -> Option<String> {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg("dumpsys activity activities 2>/dev/null; dumpsys window 2>/dev/null")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if line.contains("ResumedActivity")
            || line.contains("topResumedActivity")
            || line.contains("mFocusedApp")
            || line.contains("mCurrentFocus")
        {
            if let Some(tok) = line.split_whitespace().find(|t| t.contains('/')) {
                return Some(tok.split('/').next().unwrap_or(tok).to_string());
            }
        }
    }
    None
}

fn history_json(st: &State) -> String {
    let h = st.history();
    let items: Vec<String> = h
        .iter()
        .map(|s| {
            format!(
                "{{\"t\":{},\"cpu\":{:.3},\"temp\":{:.1},\"charging\":{}}}",
                s.t, s.cpu, s.temp, s.charging
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

fn ai_json(st: &State) -> String {
    let entries = journal::last(8);
    let items: Vec<String> = entries
        .iter()
        .map(|e| {
            format!(
                "{{\"t\":{},\"profile\":{},\"kind\":\"{}\",\"app\":\"{}\",\"mode\":\"{}\",\"reason\":{},\"path\":{},\"value\":{},\"ok\":{},\"note\":{},\"outcome\":{},\"temp_delta\":{:.2}}}",
                e.t,
                jstr(&e.profile),
                e.kind,
                e.app,
                e.mode,
                jstr(&e.reason),
                jstr(&e.path),
                jstr(&e.value),
                e.ok,
                jstr(&e.note),
                jstr(&e.outcome),
                e.temp_delta
            )
        })
        .collect();
    let checks: Vec<String> = st
        .atlas()
        .into_iter()
        .map(|(n, ok, d)| format!("{{\"name\":{},\"ok\":{},\"detail\":{}}}", jstr(&n), ok, jstr(&d)))
        .collect();
    let (lp, lr) = match entries.first() {
        Some(e) => (e.profile.clone(), e.reason.clone()),
        None => ("-".to_string(), "-".to_string()),
    };
    // Objetivo y proximo cambio (sin aplicar)
    let m = st.metrics();
    let sat = iaopt_core::ai::satisfaction(&m);
    let knobs = st.probe().scan();
    let prof = *st.profile.lock().unwrap();
    let feats = *st.features.lock().unwrap();
    let next = iaopt_core::ai::plan_single(prof, &knobs, feats, None);
    let next_json = match next {
        Some(a) => format!("{{\"path\":{},\"value\":{}}}", jstr(&a.path), jstr(&a.value)),
        None => "null".to_string(),
    };
    let wjson: Vec<String> = journal::weights(40)
        .iter()
        .map(|(mode, kind, v, c)| format!("{{\"mode\":\"{}\",\"kind\":\"{}\",\"temp_delta\":{:.2},\"samples\":{}}}", mode, kind, v, c))
        .collect();
    let learning = journal::verified_count(300);
    let apps = journal::per_app(300);
    let ajson: Vec<String> = apps
        .iter()
        .map(|(a, c)| format!("{{\"app\":\"{}\",\"verified\":{}}}", a, c))
        .collect();
    let mut caps: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for k in &knobs {
        *caps.entry(k.domain.as_str().to_string()).or_insert(0) += 1;
    }
    let cjson: Vec<String> = caps
        .iter()
        .map(|(k, v)| format!("{{\"domain\":\"{}\",\"count\":{}}}", k, v))
        .collect();
    format!(
        "{{\"ai\":{},\"auto\":{},\"profile\":\"{}\",\"objective\":{},\"satisfaction\":{:.3},\"target\":{:.2},\"learning\":{},\"apps\":[{}],\"next\":{},\"weights\":[{}],\"capabilities\":[{}],\"last\":{{\"profile\":\"{}\",\"reason\":{}}},\"journal\":[{}],\"atlas\":[{}]}}",
        st.ai_enabled.load(Ordering::Relaxed),
        st.auto.load(Ordering::Relaxed),
        prof.as_str(),
        jstr(&iaopt_core::ai::objective_line(&m)),
        sat,
        iaopt_core::ai::TARGET,
        learning,
        ajson.join(","),
        next_json,
        wjson.join(","),
        cjson.join(","),
        lp,
        jstr(&lr),
        items.join(","),
        checks.join(",")
    )
}

/// Un ciclo de IA Motor: objetivo -> un solo cambio -> verificacion -> medicion.
/// Con `force` actua aunque el perfil no cambie (llamada manual).
fn ai_cycle(st: &State, force: bool) -> String {
    let m = st.record();
    let prev = *st.profile.lock().unwrap();
    let s = iaopt_core::engine::decide(Some(prev), &m);
    let sat0 = iaopt_core::ai::satisfaction(&m);
    if !force && s.profile == prev && sat0 >= iaopt_core::ai::TARGET {
        return format!(
            "{{\"ok\":true,\"changed\":false,\"reason\":{},\"satisfaction\":{:.3}}}",
            jstr(&s.reason),
            sat0
        );
    }
    let knobs = st.probe().scan();
    let avoid = journal::last(1).into_iter().next().map(|e| e.path);
    let feats = *st.features.lock().unwrap();
    let dev = device::load();
    agent::set_activity(&format!("Analizando {} · {}", dev.soc_vendor, dev.soc));
    let action = iaopt_core::ai::plan_single(s.profile, &knobs, feats, avoid.as_deref());
    let Some(a) = action else {
        return format!(
            "{{\"ok\":true,\"changed\":false,\"reason\":{},\"satisfaction\":{:.3}}}",
            jstr(&s.reason),
            sat0
        );
    };
    if !device::path_compatible(&a.path, &dev) {
        agent::set_activity(&format!("Omitido: {} no compatible con {}", a.path, dev.soc_vendor));
        agent::record(
            "incompatible",
            &a.path,
            "no compatible con el dispositivo",
            &dev.soc_vendor,
        );
        return format!(
            "{{\"ok\":true,\"changed\":false,\"reason\":{},\"satisfaction\":{:.3}}}",
            jstr(&s.reason),
            sat0
        );
    }
    agent::set_activity(&format!("Aplicando {} → {}", a.path, a.value));
    let rep = tuner::apply(&a, &st.sys_root);
    if rep.applied {
        agent::set_activity(&format!("Verificado {} → {}", a.path, a.value));
    } else {
        agent::record("no sostenido", &a.path, "el cambio no se mantuvo", &rep.note);
    }
    if rep.applied {
        *st.profile.lock().unwrap() = s.profile;
        st.save();
    }
    // Medir el efecto despues del cambio
    std::thread::sleep(std::time::Duration::from_secs(5));
    let after = st.metrics();
    let out = iaopt_core::engine::outcome(&m, &after);
    let td = after.battery_temp_c - m.battery_temp_c;
    let cd = after.cpu_load - m.cpu_load;
    let app = read_foreground_app().unwrap_or_default();
    journal::append(
        s.profile.as_str(),
        &s.reason,
        s.confidence,
        iaopt_core::ai::kind_of(&a.path),
        &app,
        s.profile.as_str(),
        &rep.path,
        &rep.requested,
        rep.applied,
        &rep.note,
        &out,
        td,
        cd,
    );
    let sat1 = iaopt_core::ai::satisfaction(&after);
    format!(
        "{{\"ok\":true,\"changed\":true,\"profile\":\"{}\",\"path\":{},\"value\":{},\"applied\":{},\"outcome\":{},\"satisfaction_before\":{:.3},\"satisfaction_after\":{:.3},\"target\":{:.2},\"reason\":{}}}",
        s.profile.as_str(),
        jstr(&rep.path),
        jstr(&rep.requested),
        rep.applied,
        jstr(&out),
        sat0,
        sat1,
        iaopt_core::ai::TARGET,
        jstr(&s.reason)
    )
}

/// Pares `clave: valor` de `getprop`.
fn prop_pairs(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|l| {
            let l = l.trim();
            let (k, v) = l.split_once("]: [")?;
            let k = k.trim_start_matches('[').trim();
            let v = v.trim_end_matches(']').trim();
            Some((k.to_string(), v.to_string()))
        })
        .collect()
}

fn getprop_all() -> Vec<(String, String)> {
    std::process::Command::new("getprop")
        .output()
        .map(|o| prop_pairs(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// Estado real del subsistema de audio (propiedades de Android), sin inventar nodos.
pub fn audio_json() -> String {
    let pairs: Vec<(String, String)> = getprop_all()
        .into_iter()
        .filter(|(k, _)| {
            let k = k.to_ascii_lowercase();
            k.contains("audio") || k.contains("aaudio") || k.contains("sound")
        })
        .collect();
    let items: Vec<String> = pairs
        .iter()
        .map(|(k, v)| format!("{{\"key\":{},\"value\":{}}}", jstr(k), jstr(v)))
        .collect();
    format!(
        "{{\"props\":[{}],\"count\":{}}}",
        items.join(","),
        pairs.len()
    )
}

/// Estado real de pantalla: refresco, motor de renderizado, densidad y tamaño.
pub fn display_json() -> String {
    let t = crate::telemetry::sample(false);
    let hz = t.refresh_hz.unwrap_or(0.0);
    let rd = crate::renderer::status_json();
    let density = std::process::Command::new("wm")
        .arg("density")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let size = std::process::Command::new("wm")
        .arg("size")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    format!(
        "{{\"refresh_hz\":{},\"renderer\":{},\"density\":{},\"size\":{}}}",
        hz,
        rd,
        jstr(&density),
        jstr(&size)
    )
}

fn scan_json(st: &State, force: bool) -> String {
    let knobs = if force {
        invalidate_scan();
        st.probe().scan()
    } else {
        cached_scan(st)
    };
    let items: Vec<String> = visible_knobs(&knobs)
        .iter()
        .map(|k| {
            let allowed: Vec<String> = k.allowed.iter().map(|a| jstr(a)).collect();
            format!(
                "{{\"domain\":\"{}\",\"path\":{},\"writable\":{},\"current\":{},\"allowed\":[{}],\"min\":{},\"max\":{}}}",
                k.domain.as_str(),
                jstr(&k.path),
                k.writable,
                jstr(k.current.as_deref().unwrap_or("-")),
                allowed.join(","),
                k.min.map(|v| v.to_string()).unwrap_or_else(|| "null".into()),
                k.max.map(|v| v.to_string()).unwrap_or_else(|| "null".into()),
            )
        })
        .collect();
    format!("[{}]", items.join(","))
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
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}
