//! Sesión de juego in-game: preferencias explícitas del usuario mientras un
//! juego está en primer plano. El agente las respeta (la protección térmica
//! siempre gana) y se limpian solas al salir del juego.
//!
//! Confort de sesión (no molestar, bloqueo de brillo, límite de FPS por juego
//! vía GameManager): se aplica en el daemon, se guarda el estado PREVIO del
//! sistema y se restaura al acabar la sesión. La restauración pendiente se
//! persiste: si el daemon muere con la sesión activa, al arrancar devuelve el
//! sistema a su estado original (nunca deja DND o el brillo colgados).
//!
//! Lección de la referencia open source analizada: los toggles de confort de
//! su panel eran humo (solo guardaban un boolean); el DND real vivía en el
//! daemon con restauración. Aquí todo toggle tiene efecto verificable.

use crate::state;
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const DIR: &str = "/data/adb/ia_optimizer";
const PATH: &str = "/data/adb/ia_optimizer/session.json";

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

fn raw() -> String {
    fs::read_to_string(PATH).unwrap_or_default()
}

/// Override de perfil vigente (paquete, perfil, extremo).
pub fn read() -> Option<(String, String, bool)> {
    let s = raw();
    let pkg = state::get_json_str(&s, "package")?;
    let profile = state::get_json_str(&s, "profile").unwrap_or_else(|| "balanced".into());
    let extreme = state::get_json_bool(&s, "extreme").unwrap_or(false);
    if pkg.is_empty() { None } else { Some((pkg, profile, extreme)) }
}

fn read_field(key: &str) -> String {
    state::get_json_str(&raw(), key).unwrap_or_default()
}

fn persist(pkg: &str, profile: &str, extreme: bool, dnd: bool, bright: bool, fps: i32, prev_zen: &str, prev_bright: &str) {
    let _ = fs::create_dir_all(DIR);
    let body = format!(
        "{{\"package\":{},\"profile\":{},\"extreme\":{},\"t\":{},\"dnd\":{},\"brightness_lock\":{},\"fps_target\":{},\"prev_zen\":{},\"prev_bright\":{}}}",
        jstr(pkg), jstr(profile), extreme, now(), dnd, bright, fps,
        jstr(prev_zen), jstr(prev_bright)
    );
    let tmp = format!("{}.tmp", PATH);
    if fs::write(&tmp, body).is_ok() {
        let _ = fs::rename(&tmp, PATH);
    }
}

/// Fija el perfil de la sesión; conserva el confort ya aplicado si el juego
/// es el mismo (cambiar de perfil no debe soltar el DND ni el brillo).
fn write_session(pkg: &str, profile: &str, extreme: bool) {
    let s = raw();
    let same = state::get_json_str(&s, "package").as_deref() == Some(pkg);
    let (dnd, bright, fps, pz, pb) = if same {
        (
            state::get_json_bool(&s, "dnd").unwrap_or(false),
            state::get_json_bool(&s, "brightness_lock").unwrap_or(false),
            state::get_json_num(&s, "fps_target").unwrap_or(0.0) as i32,
            state::get_json_str(&s, "prev_zen").unwrap_or_default(),
            state::get_json_str(&s, "prev_bright").unwrap_or_default(),
        )
    } else {
        // Juego distinto: restaura la sesión anterior antes de empezar otra.
        if state::get_json_str(&s, "package").is_some() { end_session(); }
        (false, false, 0, String::new(), String::new())
    };
    persist(pkg, profile, extreme, dnd, bright, fps, &pz, &pb);
}

/// Actualiza campos de confort de la sesión vigente sin tocar el perfil.
fn update_session(dnd: Option<bool>, bright: Option<bool>, fps: Option<i32>, prev_zen: Option<&str>, prev_bright: Option<&str>) {
    let Some((pkg, profile, extreme)) = read() else { return };
    let s = raw();
    let dnd = dnd.unwrap_or_else(|| state::get_json_bool(&s, "dnd").unwrap_or(false));
    let bright = bright.unwrap_or_else(|| state::get_json_bool(&s, "brightness_lock").unwrap_or(false));
    let fps = fps.unwrap_or_else(|| state::get_json_num(&s, "fps_target").unwrap_or(0.0) as i32);
    let pz = prev_zen.map(|v| v.to_string()).unwrap_or_else(|| read_field("prev_zen"));
    let pb = prev_bright.map(|v| v.to_string()).unwrap_or_else(|| read_field("prev_bright"));
    persist(&pkg, &profile, extreme, dnd, bright, fps, &pz, &pb);
}

/// Fin de sesión: restaura TODO lo que la sesión cambió y borra el registro.
fn end_session() {
    let s = raw();
    let pkg = state::get_json_str(&s, "package").unwrap_or_default();
    if pkg.is_empty() {
        let _ = fs::remove_file(PATH);
        return;
    }
    if state::get_json_bool(&s, "dnd").unwrap_or(false) {
        restore_dnd(&state::get_json_str(&s, "prev_zen").unwrap_or_default());
    }
    if state::get_json_bool(&s, "brightness_lock").unwrap_or(false) {
        restore_brightness(&state::get_json_str(&s, "prev_bright").unwrap_or_default());
    }
    let fps = state::get_json_num(&s, "fps_target").unwrap_or(0.0) as i32;
    if fps > 0 {
        game_mode_reset(&pkg);
    }
    let _ = fs::remove_file(PATH);
}

pub fn clear() {
    end_session();
}

/// POST /api/session: fija o limpia el override y aplica confort con restauración.
pub fn handle(body: &str) -> String {
    if state::get_json_bool(body, "clear").unwrap_or(false) {
        end_session();
        return "{\"ok\":true,\"cleared\":true}".into();
    }
    if let Some(p) = state::get_json_str(body, "profile") {
        let p = match p.as_str() {
            "eco" | "balanced" | "performance" => p,
            _ => return "{\"ok\":false,\"detail\":\"perfil invalido\"}".into(),
        };
        let pkg = state::get_json_str(body, "package").unwrap_or_default();
        if pkg.is_empty() {
            return "{\"ok\":false,\"detail\":\"falta package\"}".into();
        }
        let extreme = state::get_json_bool(body, "extreme").unwrap_or(false);
        write_session(&pkg, &p, extreme);
    }
    // Controles de confort con paquete pero sin perfil previo: crea una
    // sesión solo-de-confort (perfil vacío = la IA sigue decidiendo el perfil).
    if read().is_none() {
        if let Some(pkg) = state::get_json_str(body, "package") {
            if !pkg.is_empty()
                && (state::get_json_bool(body, "dnd").is_some()
                    || state::get_json_bool(body, "brightness_lock").is_some()
                    || state::get_json_num(body, "fps_target").is_some())
            {
                persist(&pkg, "", false, false, false, 0, "", "");
            }
        }
    }
    if let Some(v) = state::get_json_bool(body, "dnd") {
        apply_dnd(v);
    }
    if let Some(v) = state::get_json_bool(body, "brightness_lock") {
        apply_brightness(v);
    }
    if let Some(v) = state::get_json_num(body, "fps_target") {
        apply_fps_target(v as i32);
    }
    status()
}

// ------------------------- no molestar -------------------------

fn zen_raw() -> String {
    sh("settings get global zen_mode")
}

/// Aplica DND guardando el zen previo; al desactivar, restaura ese previo.
fn apply_dnd(on: bool) {
    if read().is_none() { return; }
    if on {
        let prev = zen_raw();
        sh("cmd notification set_dnd none 2>/dev/null || settings put global zen_mode 2");
        update_session(Some(true), None, None, Some(&prev), None);
    } else {
        restore_dnd(&read_field("prev_zen"));
        update_session(Some(false), None, None, Some(""), None);
    }
}

fn restore_dnd(prev: &str) {
    // Solo se restaura lo que la sesión cambió: si el usuario ya tenía DND
    // antes de entrar al juego, se devuelve su valor exacto.
    let p = prev.trim();
    if p.is_empty() || p == "0" || p == "null" {
        sh("cmd notification set_dnd all 2>/dev/null || settings put global zen_mode 0");
    } else {
        sh(&format!("settings put global zen_mode {}", p));
    }
}

pub fn dnd_on() -> bool {
    let zen = zen_raw();
    zen.trim() != "0" && !zen.trim().is_empty() && zen.trim() != "null"
}

// ------------------------- bloqueo de brillo -------------------------

fn bright_raw() -> String {
    sh("settings get system screen_brightness_mode")
}

fn apply_brightness(on: bool) {
    if read().is_none() { return; }
    if on {
        let prev = bright_raw();
        sh("settings put system screen_brightness_mode 0");
        update_session(None, Some(true), None, None, Some(&prev));
    } else {
        restore_brightness(&read_field("prev_bright"));
        update_session(None, Some(false), None, None, Some(""));
    }
}

fn restore_brightness(prev: &str) {
    let p = prev.trim();
    let v = if p == "0" { "0" } else { "1" };
    sh(&format!("settings put system screen_brightness_mode {}", v));
}

pub fn brightness_locked() -> bool {
    bright_raw().trim() == "0"
}

// ------------------------- límite de FPS por juego (GameManager) -------------------------

/// Android 13+: cmd game set --mode 2 --fps N <pkg>. Es el cap de FPS oficial
/// por app. Si la ROM no lo soporta, no se finge aplicación. Se restaura al
/// salir del juego (cmd game reset).
fn apply_fps_target(fps: i32) {
    let Some((pkg, _, _)) = read() else { return };
    if fps <= 0 {
        game_mode_reset(&pkg);
        update_session(None, None, Some(0), None, None);
        return;
    }
    let fps = fps.clamp(15, 120);
    let out = sh(&format!("cmd game set --mode 2 --fps {} {}", fps, pkg));
    // GameManager verifica por CONTENIDO, no por silencio: unas ROM contestan
    // en silencio al aplicar y otras imprimen la confirmación
    // ("...fps-override: N"). Se acepta ambas; se rechaza solo un rechazo
    // explícito ("not of game type", error, excepción, comando desconocido).
    let lower = out.to_lowercase();
    let rejected = lower.contains("not of game type")
        || lower.contains("error")
        || lower.contains("exception")
        || lower.contains("unknown")
        || lower.contains("invalid");
    let confirmed = out.trim().is_empty() || out.contains(&format!("fps-override: {}", fps));
    let ok = !rejected && confirmed;
    if ok {
        update_session(None, None, Some(fps), None, None);
    }
}

fn game_mode_reset(pkg: &str) {
    // Devuelve el juego al modo estándar (sin límite de FPS impuesto).
    sh(&format!("cmd game reset {}", pkg));
}

pub fn fps_target() -> i32 {
    state::get_json_num(&raw(), "fps_target").unwrap_or(0.0) as i32
}

// ------------------------- estado y ciclo de vida -------------------------

/// GET /api/session: estado completo para el panel in-game.
pub fn status() -> String {
    let (pkg, profile, extreme) = read().unwrap_or_default();
    format!(
        "{{\"ok\":true,\"package\":{},\"profile\":{},\"extreme\":{},\"active\":{},\"dnd\":{},\"brightness_lock\":{},\"fps_target\":{}}}",
        jstr(&pkg),
        jstr(&profile),
        extreme,
        !pkg.is_empty(),
        dnd_on(),
        brightness_locked(),
        fps_target()
    )
}

/// El agente llama a esto cada ciclo: si el juego de la sesión ya no está en
/// primer plano, la sesión se acaba sola y se restaura el sistema.
pub fn tick(foreground: &str) {
    if let Some((pkg, _, _)) = read() {
        if pkg != foreground {
            end_session();
        }
    }
}

/// Al arrancar el daemon: si quedó una sesión colgada de un crash, restaura
/// el confort pendiente (DND/brillo/FPS) y limpia el registro.
pub fn restore_pending() {
    if fs::metadata(PATH).is_ok() {
        end_session();
    }
}

/// Decisión del agente para la sesión vigente, si aplica.
/// `hot` = protección térmica activa (siempre gana sobre la sesión).
pub fn agent_override(app: &str, hot: bool, battery_pct: u8) -> Option<(crate::Profile, String)> {
    let (pkg, profile, extreme) = read()?;
    if pkg != app || hot {
        return None;
    }
    // Sesión solo-de-confort (sin perfil elegido): la IA sigue decidiendo.
    if profile.is_empty() && !extreme {
        return None;
    }
    // Batería crítica: ni el modo extremo puede ignorarla.
    if battery_pct <= 15 {
        return Some((
            crate::Profile::Eco,
            format!("sesión de juego con batería crítica ({}%)", battery_pct),
        ));
    }
    let p = if extreme {
        crate::Profile::Performance
    } else {
        crate::Profile::parse(&profile).unwrap_or(crate::Profile::Balanced)
    };
    Some((
        p,
        format!(
            "elección del usuario en juego{} ({})",
            if extreme { " · extremo" } else { "" },
            pkg
        ),
    ))
}
