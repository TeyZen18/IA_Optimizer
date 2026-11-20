//! IA Motors: agente autonomo de 5 fases (diagnostico, decision, aplicacion con
//! verificacion, journal/aprendizaje y auto-sanacion) con estado atómico para la
//! WebUI. Sin panicos: todo error se registra y el ciclo continua.

use crate::device::{self, Profile};
use crate::state::State;
use crate::{journal, telemetry, tuner};
use iaopt_core::profiles::{self, Action};
use std::fs;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const DIR: &str = "/data/adb/ia_optimizer";
const STATUS: &str = "/data/adb/ia_optimizer/status.json";
const WEIGHTS: &str = "/data/adb/ia_optimizer/weights.json";
const BLACKLIST: &str = "/data/adb/ia_optimizer/blacklist.conf";
const ACTIVITY: &str = "/data/adb/ia_optimizer/activity.txt";
const PLAN: &str = "/data/adb/ia_optimizer/plan.txt";
const UNRESOLVED: &str = "/data/adb/ia_optimizer/unresolved.jsonl";

struct AgentSlot {
    activity: String,
    phase: String,
    plan: Vec<String>,
    cycle: u64,
}

fn slot() -> &'static Mutex<AgentSlot> {
    static S: OnceLock<Mutex<AgentSlot>> = OnceLock::new();
    S.get_or_init(|| {
        let act = fs::read_to_string(ACTIVITY)
            .unwrap_or_default()
            .trim()
            .to_string();
        Mutex::new(AgentSlot {
            activity: if act.is_empty() { "En espera".into() } else { act },
            phase: "En espera".into(),
            plan: Vec::new(),
            cycle: 0,
        })
    })
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn jstr(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn set_activity(s: &str) {
    let _ = fs::create_dir_all(DIR);
    let _ = fs::write(ACTIVITY, s);
    if let Ok(mut a) = slot().lock() {
        a.activity = s.to_string();
    }
}

pub fn set_phase(s: &str) {
    if let Ok(mut a) = slot().lock() {
        a.phase = s.to_string();
    }
}

pub fn set_plan(p: Vec<String>) {
    let _ = fs::write(PLAN, p.join("\n"));
    if let Ok(mut a) = slot().lock() {
        a.plan = p;
    }
}

pub fn get_activity() -> String {
    if let Ok(s) = fs::read_to_string(ACTIVITY) {
        let s = s.trim();
        if !s.is_empty() {
            return s.to_string();
        }
    }
    slot().lock().map(|a| a.activity.clone()).unwrap_or_default()
}

fn get_plan() -> Vec<String> {
    fs::read_to_string(PLAN)
        .unwrap_or_default()
        .lines()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn cycle_inc() -> u64 {
    let mut n = 0;
    if let Ok(mut a) = slot().lock() {
        a.cycle += 1;
        n = a.cycle;
    }
    n
}

// ---------- Pesos de aprendizaje (weights.json) ----------

#[derive(Clone)]
pub struct Weight {
    pub mode: String,
    pub kind: String,
    pub score: i32,
    pub temp_delta: f32,
}

fn load_weights() -> Vec<Weight> {
    let Ok(s) = fs::read_to_string(WEIGHTS) else {
        return Vec::new();
    };
    let mut v = Vec::new();
    for l in s.lines() {
        if l.trim().is_empty() {
            continue;
        }
        v.push(Weight {
            mode: crate::state::get_json_str(l, "mode").unwrap_or_default(),
            kind: crate::state::get_json_str(l, "kind").unwrap_or_default(),
            score: crate::state::get_json_num(l, "score").unwrap_or(0.0) as i32,
            temp_delta: crate::state::get_json_num(l, "temp").unwrap_or(0.0) as f32,
        });
    }
    v
}

fn save_weights(w: &[Weight]) {
    let mut out = String::new();
    for x in w {
        out.push_str(&format!(
            "{{\"mode\":{},\"kind\":{},\"score\":{},\"temp\":{:.2}}}\n",
            jstr(&x.mode),
            jstr(&x.kind),
            x.score,
            x.temp_delta
        ));
    }
    let _ = fs::write(WEIGHTS, out);
}

/// Refuerza configuraciones estables (+1) y penaliza las inestables (-1).
pub fn bump_weight(mode: &str, kind: &str, ok: bool, temp: f32) {
    let mut w = load_weights();
    if let Some(e) = w.iter_mut().find(|x| x.mode == mode && x.kind == kind) {
        e.score = (e.score + if ok { 1 } else { -1 }).clamp(-10, 50);
        e.temp_delta = (e.temp_delta * 0.7) + (temp * 0.3);
    } else {
        w.push(Weight {
            mode: mode.to_string(),
            kind: kind.to_string(),
            score: if ok { 1 } else { -1 },
            temp_delta: temp,
        });
    }
    save_weights(&w);
}

pub fn weights_json() -> String {
    let w = load_weights();
    let items: Vec<String> = w
        .iter()
        .map(|x| {
            format!(
                "{{\"mode\":{},\"kind\":{},\"score\":{},\"temp_delta\":{:.2}}}",
                jstr(&x.mode),
                jstr(&x.kind),
                x.score,
                x.temp_delta
            )
        })
        .collect();
    format!("{{\"weights\":[{}]}}", items.join(","))
}

// ---------- Lista negra dinámica (blacklist.conf) ----------

pub fn blacklist() -> Vec<String> {
    fs::read_to_string(BLACKLIST)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

pub fn is_blacklisted(path: &str) -> bool {
    blacklist().iter().any(|b| b == path)
}

pub fn add_blacklist(path: &str, reason: &str) {
    if is_blacklisted(path) {
        return;
    }
    let mut s = fs::read_to_string(BLACKLIST).unwrap_or_default();
    s.push_str(&format!("# {}\n{}\n", reason, path));
    let _ = fs::write(BLACKLIST, s);
    record("blacklist", path, "fallo repetido", reason);
}

// ---------- Errores sin resolver ----------

pub struct Unresolved {
    pub t: u64,
    pub kind: String,
    pub path: String,
    pub reason: String,
    pub detail: String,
}

pub fn record(kind: &str, path: &str, reason: &str, detail: &str) {
    let _ = fs::create_dir_all(DIR);
    let mut items = read_all();
    items.retain(|u: &Unresolved| u.path != path);
    items.push(Unresolved {
        t: now(),
        kind: kind.to_string(),
        path: path.to_string(),
        reason: reason.to_string(),
        detail: detail.to_string(),
    });
    let mut out = String::new();
    for u in items.iter().rev().take(30) {
        out.push_str(&format!(
            "{{\"t\":{},\"kind\":{},\"path\":{},\"reason\":{},\"detail\":{}}}\n",
            u.t,
            jstr(&u.kind),
            jstr(&u.path),
            jstr(&u.reason),
            jstr(&u.detail)
        ));
    }
    let _ = fs::write(UNRESOLVED, out);
}

pub fn read_all() -> Vec<Unresolved> {
    let Ok(s) = fs::read_to_string(UNRESOLVED) else {
        return Vec::new();
    };
    let mut v = Vec::new();
    for l in s.lines() {
        if l.trim().is_empty() {
            continue;
        }
        v.push(Unresolved {
            t: crate::state::get_json_num(l, "t").unwrap_or(0.0) as u64,
            kind: crate::state::get_json_str(l, "kind").unwrap_or_default(),
            path: crate::state::get_json_str(l, "path").unwrap_or_default(),
            reason: crate::state::get_json_str(l, "reason").unwrap_or_default(),
            detail: crate::state::get_json_str(l, "detail").unwrap_or_default(),
        });
    }
    v
}

pub fn clear(path: &str) -> bool {
    let items = read_all();
    let mut out = String::new();
    let mut found = false;
    for u in items {
        if u.path == path {
            found = true;
            continue;
        }
        out.push_str(&format!(
            "{{\"t\":{},\"kind\":{},\"path\":{},\"reason\":{},\"detail\":{}}}\n",
            u.t,
            jstr(&u.kind),
            jstr(&u.path),
            jstr(&u.reason),
            jstr(&u.detail)
        ));
    }
    if found {
        let _ = fs::write(UNRESOLVED, out);
    }
    found
}

// ---------- Contexto ----------

pub(crate) fn screen_off() -> bool {
    let Ok(out) = std::process::Command::new("sh")
        .arg("-c")
        .arg("dumpsys power 2>/dev/null | grep -m1 mWakefulness")
        .output()
    else {
        return false;
    };
    let s = String::from_utf8_lossy(&out.stdout).to_ascii_lowercase();
    s.contains("asleep") || s.contains("dozing")
}

pub fn foreground_app() -> String {
    crate::read_foreground_app().unwrap_or_default()
}

fn is_game(pkg: &str) -> bool {
    const GAMES: &[&str] = &[
        "com.mihoyo", "com.tencent.ig", "com.pubg", "com.activision.callofduty",
        "com.riotgames", "com.supercell", "com.king", "com.epicgames", "com.nintendo",
        "com.playkong", "com.gameloft", "com.skydance", "com.netease", "com.moonton",
        "com.dts.freefire", "com.roblox", "com.minecraft", "com.niantic",
    ];
    let p = pkg.to_ascii_lowercase();
    p.contains("game") || GAMES.iter().any(|g| p.starts_with(g))
}

fn journal_counts() -> (usize, usize) {
    let Ok(s) = fs::read_to_string(DIR.to_string() + "/journal.jsonl") else {
        return (0, 0);
    };
    let mut ok = 0;
    let mut fail = 0;
    for l in s.lines().rev().take(300) {
        if l.contains("\"ok\":true") {
            ok += 1;
        } else if l.contains("\"ok\":false") {
            fail += 1;
        }
    }
    (ok, fail)
}

// ---------- Decisión ----------

struct Decision {
    profile: iaopt_core::Profile,
    reason: String,
    plan: Vec<String>,
    force_off: bool,
}

fn decide(dev: &Profile, m: &iaopt_core::engine::Metrics, app: &str, gaming: bool, prev: iaopt_core::Profile, goal: &str, behavior: &str) -> Decision {
    let bat = m.battery_temp_c;
    let soc_max = telemetry::sample(false)
        .thermal_zones
        .iter()
        .map(|z| z.2)
        .max()
        .unwrap_or(0);
    // Histéresis térmica: se entra en eco por calor a 65 °C, pero no se sale
    // hasta bajar del margen de salida. Sin este margen, el perfil oscila
    // eco/balanceado cada pocos segundos bajo carga sostenida. El comportamiento
    // elegido ajusta el margen: Calma espera más (sale a 58), Decisivo menos
    // (sale a 62). La entrada de protección (65 °C) nunca se relaja.
    let soc_exit = match behavior { "calma" => 58, "decisivo" => 62, _ => 60 };
    let bat_exit = match behavior { "calma" => 37.0, "decisivo" => 40.0, _ => 39.0 };
    let soc_hot = soc_max > 65 || (prev == iaopt_core::Profile::Eco && soc_max > soc_exit);
    let bat_hot = bat > 42.0 || (prev == iaopt_core::Profile::Eco && bat > bat_exit);
    let off = screen_off();
    let mut plan = Vec::new();
    let mut profile = iaopt_core::Profile::Balanced;
    let mut reason = String::from("equilibrio");
    let mut force_off = false;

    // Sesión in-game: elección explícita del usuario mientras su juego está en
    // primer plano. La protección térmica, la pantalla apagada y la batería
    // crítica siempre ganan sobre la sesión.
    if let Some((p, why)) = crate::session::agent_override(app, soc_hot || bat_hot || off, m.battery_pct) {
        plan.push(format!("Sesión de juego: perfil {}", p.as_str()));
        plan.push(format!("Dispositivo: {} · {}", dev.soc_vendor, dev.soc));
        plan.push("Verificar cada cambio con read-back".into());
        return Decision { profile: p, reason: why, plan, force_off: false };
    }

    if off {
        profile = iaopt_core::Profile::Eco;
        reason = "pantalla apagada".into();
        plan.push("Perfil ECO por pantalla apagada".into());
        force_off = true;
    } else if soc_hot {
        profile = iaopt_core::Profile::Eco;
        reason = format!("SoC a {} °C (protección térmica con histéresis)", soc_max);
        plan.push("Limitar frecuencia y governor (SoC caliente)".into());
    } else if bat_hot {
        profile = iaopt_core::Profile::Eco;
        reason = format!("batería a {:.1} °C (>42)", bat);
        plan.push("Reducir techo de frecuencia".into());
    } else if !m.charging && m.battery_pct <= 15 {
        profile = iaopt_core::Profile::Eco;
        reason = format!("batería baja ({}%): ahorro prioritario", m.battery_pct);
        plan.push("Perfil ECO para preservar la batería".into());
    } else {
        match iaopt_core::engine::classify(app) {
            iaopt_core::engine::Foreground::Game => {
                // El objetivo decide el techo del juego: Batería contiene el
                // rendimiento salvo carga enchufada; Actuación lo libera salvo
                // batería crítica; Equilibrado usa la regla moderada.
                let go_perf = match goal {
                    "bateria" => m.charging && m.battery_pct > 60,
                    "actuacion" => m.charging || m.battery_pct > 15,
                    _ => m.charging || m.battery_pct > 30,
                };
                if go_perf {
                    profile = iaopt_core::Profile::Performance;
                    reason = format!(
                        "juego en primer plano ({})",
                        if app.is_empty() { "forzado" } else { app }
                    );
                    plan.push("Perfil rendimiento para juego".into());
                } else {
                    profile = iaopt_core::Profile::Balanced;
                    reason = format!("juego con batería moderada ({}%)", m.battery_pct);
                    plan.push("Rendimiento contenido para cuidar la batería".into());
                }
            }
            iaopt_core::engine::Foreground::Video => {
                if goal == "actuacion" {
                    profile = iaopt_core::Profile::Balanced;
                    reason = format!("vídeo con objetivo actuación ({})", app);
                    plan.push("Margen de rendimiento para reproducción fluida".into());
                } else {
                    profile = iaopt_core::Profile::Eco;
                    reason = format!("reproduciendo vídeo ({})", app);
                    plan.push("Perfil eco: ahorro inteligente en reproducción".into());
                }
            }
            iaopt_core::engine::Foreground::Social => {
                if goal == "bateria" {
                    profile = iaopt_core::Profile::Eco;
                    reason = format!("red social con objetivo batería ({})", app);
                    plan.push("Perfil eco: el desplazamiento no necesita más".into());
                } else {
                    profile = iaopt_core::Profile::Balanced;
                    reason = format!("red social en primer plano ({})", app);
                    plan.push("Perfil balanceado: fluidez con consumo moderado".into());
                }
            }
            iaopt_core::engine::Foreground::Other => {
                // Sin app dominante, manda la carga real: así el agente y el
                // bucle automático deciden lo mismo y no oscilan. El objetivo
                // y el comportamiento mueven los umbrales, no la seguridad.
                let mut perf_th: f32 = match behavior { "calma" => 0.9, "decisivo" => 0.6, _ => 0.8 };
                let mut eco_th: f32 = match behavior { "calma" => 0.15, "decisivo" => 0.3, _ => 0.2 };
                match goal {
                    "actuacion" => { perf_th = perf_th.min(0.5); eco_th = 0.1; }
                    "bateria" => { perf_th = 0.95; eco_th = eco_th.max(0.35); }
                    _ => {}
                }
                let perf_ok = (m.cpu_load > perf_th && m.charging)
                    || (goal == "actuacion" && m.cpu_load > 0.85);
                if perf_ok {
                    profile = iaopt_core::Profile::Performance;
                    reason = format!(
                        "carga pesada sostenida ({:.0}%){}",
                        m.cpu_load * 100.0,
                        if m.charging { " mientras se carga" } else { "" }
                    );
                    plan.push("Perfil rendimiento para la carga sostenida".into());
                } else if m.cpu_load < eco_th && !m.charging {
                    profile = iaopt_core::Profile::Eco;
                    reason = "inactividad y con batería".into();
                    plan.push("Perfil eco en reposo".into());
                } else {
                    plan.push("Mantener equilibrio y buscar mejora minima".into());
                }
            }
        }
    }
    plan.push(format!("Dispositivo: {} · {}", dev.soc_vendor, dev.soc));
    plan.push("Verificar cada cambio con read-back".into());
    Decision { profile, reason, plan, force_off }
}

// ---------- Aplicación con verificación y auto-sanación ----------

fn fail_count(path: &str) -> usize {
    let Ok(s) = fs::read_to_string(DIR.to_string() + "/journal.jsonl") else {
        return 0;
    };
    s.lines()
        .rev()
        .take(200)
        .filter(|l| l.contains(path) && l.contains("\"ok\":false"))
        .count()
}

fn apply_action(st: &State, a: &Action, dev: &Profile) -> bool {
    if !device::path_compatible(&a.path, dev) {
        record("incompatible", &a.path, "no compatible con el dispositivo", &dev.soc_vendor);
        return false;
    }
    if is_blacklisted(&a.path) {
        record("blacklist", &a.path, "en lista negra", "fallos repetidos previos");
        return false;
    }
    let rep = tuner::apply(a, &st.sys_root);
    if !rep.applied && fail_count(&a.path) >= 3 {
        add_blacklist(&a.path, "fallo repetido");
        set_activity(&format!("Sanado: {} en lista negra", a.path));
    }
    rep.applied
}

// ---------- Ciclo principal de 5 fases ----------

pub fn run_cycle(st: &State) {
    let n = cycle_inc();
    let m = st.record();
    let dev = device::load();
    let app = foreground_app();
    // La sesión de juego se acaba sola cuando el juego deja el primer plano.
    crate::session::tick(&app);
    // Uso de apps: segundos reales en primer plano (medición propia, diaria).
    crate::tools::usage_tick(&app);
    let gaming = st.gaming.load(std::sync::atomic::Ordering::Relaxed);

    // 1. Diagnóstico
    set_phase("1/5 Diagnóstico");
    set_activity(&format!(
        "Diagnostico: {} · batería {:.1} °C",
        dev.soc, m.battery_temp_c
    ));
    let before = st.metrics();

    // 2. Toma de decisiones
    set_phase("2/5 Decisión");
    let prev_profile = *st.profile.lock().unwrap();
    let d = decide(&dev, &m, &app, gaming, prev_profile, &st.goal(), &st.behavior());
    set_plan(d.plan.clone());
    set_activity(&format!("Decide: {}", d.reason));

    // 3. Aplicación con verificación (read-back dentro de tuner::apply)
    set_phase("3/5 Aplicación y verificación");
    let knobs = st.probe().scan();
    let feats = *st.features.lock().unwrap();
    let prev = *st.profile.lock().unwrap();
    let mut applied = 0;
    // Mientras una app con perfil propio esta en primer plano, el agente no
    // vuelve a aplicar su decision: si no, los dos hilos oscilarian.
    let perapp_active = st
        .perapp
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .as_deref()
        == Some(app.as_str());
    if perapp_active {
        set_activity(&format!("Perfil propio de {}", app));
    } else if !d.force_off && d.profile != prev {
        let actions: Vec<Action> = profiles::plan_with_cap(d.profile, &knobs, Some(st.user_cap()))
            .into_iter()
            .filter(|a| a.path != "-")
            .filter(|a| profiles::action_enabled(feats, a))
            .filter(|a| !is_blacklisted(&a.path))
            .collect();
        for a in &actions {
            if apply_action(st, a, &dev) {
                applied += 1;
            }
        }
        if applied > 0 {
            *st.profile.lock().unwrap() = d.profile;
            st.save();
        }
        set_activity(&format!("Aplicado {} cambio(s) para {}", applied, d.profile.as_str()));
    } else {
        set_activity(&format!("Sin cambios: {}", d.reason));
    }

    // 4. Journal y aprendizaje
    set_phase("4/5 Journal y aprendizaje");
    std::thread::sleep(std::time::Duration::from_millis(900));
    let after = st.metrics();
    let outcome = iaopt_core::engine::outcome(&before, &after);
    let td = after.battery_temp_c - before.battery_temp_c;
    if applied > 0 {
        let mode = d.profile.as_str().to_string();
        journal::append(
            &mode,
            &d.reason,
            0.8,
            "perfil",
            &app,
            &mode,
            &format!("perfil {}", d.profile.as_str()),
            "perfil",
            true,
            "ciclo agente",
            &outcome,
            td,
            after.cpu_load - before.cpu_load,
        );
        bump_weight(&mode, "perfil", true, td);
    } else if n % 20 == 0 {
        // Sonda de conocimiento: medicion real sin cambios (alimenta Diario y auditoria)
        journal::append(
            d.profile.as_str(),
            "sonda",
            0.7,
            "sonda",
            &app,
            d.profile.as_str(),
            "-",
            "medido",
            true,
            "sin cambios",
            &outcome,
            td,
            after.cpu_load - before.cpu_load,
        );
    }
    // Nota: si no hubo cambios no se castiga al peso; solo se penaliza un intento fallido.

    // 5. Auto-sanación
    set_phase("5/5 Auto-sanación");
    let unresolved = read_all().len();
    set_activity(&format!(
        "Ciclo {} completo: {} cambio(s), {} error(es) sin resolver",
        n, applied, unresolved
    ));
    // Centro de comando: prediccion termica y estado atómico
    let tel = telemetry::sample(false);
    let soc_max = tel.thermal_zones.iter().map(|z| z.2).max().unwrap_or(0);
    crate::command::record_thermal(soc_max);
    crate::command::write(st);
    write_status(st);
}


/// Volcado atómico de status.json para la WebUI.
pub fn write_status(st: &State) {
    let body = status_body(st);
    let tmp = format!("{}.tmp", STATUS);
    if fs::write(&tmp, body).is_ok() {
        let _ = fs::rename(&tmp, STATUS);
    }
}

fn status_body(st: &State) -> String {
    let m = st.record();
    let dev = device::load();
    let tel = telemetry::sample(false);
    let sat = iaopt_core::ai::satisfaction(&m);
    let goal = st.goal();
    let behavior = st.behavior();
    let target = iaopt_core::ai::target_for(&behavior);
    let heat = crate::heatbudget::json();
    let (applied, failed) = journal_counts();
    let learning = journal::verified_count(300);

    // Diario: entradas recientes con tipo decision/sonda/alerta
    let entries = journal::last(60);
    let total = entries.len();
    let measured = entries.iter().filter(|e| e.ok).count();
    let overrides = entries.iter().filter(|e| e.kind == "usuario").count();
    let window_s = match (entries.first(), entries.last()) {
        (Some(newest), Some(oldest)) => newest.t.saturating_sub(oldest.t),
        _ => 0,
    };
    let per_app = journal::per_app(60);
    let per_app_json: Vec<String> = per_app
        .iter()
        .map(|(a, c)| format!("{{\"app\":{},\"verified\":{}}}", jstr(a), c))
        .collect();
    let honesty = if total > 0 {
        measured as f32 / total as f32 * 100.0
    } else {
        100.0
    };
    let interventions = entries.iter().filter(|e| e.temp_delta < -0.05).count();
    let jitems: Vec<String> = entries
        .iter()
        .map(|e| {
            format!(
                "{{\"t\":{},\"kind\":{},\"profile\":{},\"reason\":{},\"conf\":{:.2},\"note\":{},\"path\":{},\"value\":{},\"ok\":{},\"outcome\":{},\"app\":{},\"mode\":{},\"temp_d\":{:.2},\"cpu_d\":{:.2}}}",
                e.t,
                jstr(&e.kind),
                jstr(&e.profile),
                jstr(&e.reason),
                e.confidence,
                jstr(&e.note),
                jstr(&e.path),
                jstr(&e.value),
                e.ok,
                jstr(&e.outcome),
                jstr(&e.app),
                jstr(&e.mode),
                e.temp_delta,
                e.cpu_delta
            )
        })
        .collect();

    let unresolved = read_all();
    let uitems: Vec<String> = unresolved
        .iter()
        .take(10)
        .map(|u| {
            format!(
                "{{\"t\":{},\"kind\":{},\"path\":{},\"reason\":{},\"detail\":{}}}",
                u.t,
                jstr(&u.kind),
                jstr(&u.path),
                jstr(&u.reason),
                jstr(&u.detail)
            )
        })
        .collect();

    let w = journal::weights(300);
    let weights: Vec<String> = w
        .iter()
        .map(|(m, k, d, c)| {
            format!(
                "{{\"mode\":{},\"kind\":{},\"temp_delta\":{:.2},\"samples\":{}}}",
                jstr(m),
                jstr(k),
                d,
                c
            )
        })
        .collect();

    let feats = dev.features.iter().map(|f| jstr(f)).collect::<Vec<_>>().join(",");
    let plan = get_plan();
    let plan_json = plan.iter().map(|p| jstr(p)).collect::<Vec<_>>().join(",");
    let phase = slot().lock().map(|a| a.phase.clone()).unwrap_or_default();
    let cycle = slot().lock().map(|a| a.cycle).unwrap_or(0);

    // Contexto de app en primer plano
    let app = foreground_app();
    let screen = if screen_off() { "apagada" } else { "encendida" };
    let prof = st.profile.lock().map(|p| p.as_str().to_string()).unwrap_or_default();

    // Objetivos y seguridad termica
    let soc_max = tel.thermal_zones.iter().map(|z| z.2).max().unwrap_or(0);
    let governor = iaopt_core::sysfs::read_trim(std::path::Path::new(
        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
    ))
    .unwrap_or_default();
    let actuacion = (m.cpu_load * 100.0).clamp(0.0, 100.0);
    let espacio = (((soc_max.max(30) - 30) as f32) / 50.0 * 100.0).clamp(0.0, 100.0);
    let bateria = m.battery_pct as f32;
    let ram_pct = if tel.mem_total_mb > 0 {
        (tel.mem_total_mb - tel.mem_avail_mb) as f32 / tel.mem_total_mb as f32 * 100.0
    } else {
        0.0
    };

    format!(
        "{{\"current_activity\":{},\"phase\":{},\"cycle\":{},\"satisfaction\":{:.3},\"target\":{:.2},\"agent_plan\":[{}],\"learning_stats\":{{\"verified\":{},\"applied\":{},\"failed\":{},\"weights\":[{}],\"honesty_pct\":{:.1},\"probes_measured\":{},\"user_overrides\":{},\"measured_decisions\":{},\"sample\":{},\"window_s\":{},\"per_app\":[{}]}},\"device_profile\":{{\"device\":{},\"soc\":{},\"vendor\":{},\"gpu\":{},\"android\":{},\"features\":[{}]}},\"app_context\":{{\"package\":{},\"screen\":\"{}\",\"profile\":{},\"preset\":\"IA Motors\",\"ram_pct\":{:.1},\"battery_pct\":{},\"charging\":{}}},\"prefs\":{{\"goal\":{},\"behavior\":{}}},\"heat_budget\":{},\"objectives\":{{\"actuacion\":{:.1},\"bateria\":{:.1},\"espacio_termico\":{:.1}}},\"thermal\":{{\"governor\":{},\"interventions\":{},\"soc_max_c\":{},\"battery_c\":{:.1}}},\"applied_count\":{},\"failed_count\":{},\"unresolved_errors\":[{}],\"journal\":[{}]}}",
        jstr(&get_activity()),
        jstr(&phase),
        cycle,
        sat,
        target,
        plan_json,
        learning,
        applied,
        failed,
        weights.join(","),
        honesty,
        measured,
        overrides,
        measured,
        total,
        window_s,
        per_app_json.join(","),
        jstr(&dev.device),
        jstr(&dev.soc),
        jstr(&dev.soc_vendor),
        jstr(&dev.gpu),
        jstr(&dev.kernel),
        feats,
        jstr(&app),
        screen,
        jstr(&prof),
        ram_pct,
        m.battery_pct,
        m.charging,
        jstr(&goal),
        jstr(&behavior),
        heat,
        actuacion,
        bateria,
        espacio,
        jstr(&governor),
        interventions,
        soc_max,
        m.battery_temp_c,
        applied,
        failed,
        uitems.join(","),
        jitems.join(",")
    )
}

pub fn status(st: &State) -> String {
    status_body(st)
}

/// Limpia el diario (lo pide la pestaña Controles).
pub fn clear_journal() -> (bool, String) {
    let _ = fs::write(DIR.to_string() + "/journal.jsonl", "");
    (true, "diario limpio".into())
}

pub fn init() {
    let _ = fs::create_dir_all(DIR);
    let _ = fs::create_dir_all("/data/adb/.config/AiOptimizer");
}
