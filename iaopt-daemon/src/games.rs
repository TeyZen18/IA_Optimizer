//! Game Space y Apps: listado real de aplicaciones, ajustes por app y HUD.
//!
//! Regla: el Game Space solo muestra aplicaciones marcadas como juego
//! (activadas por el usuario o detectadas por heurística). La pestaña Apps
//! muestra la lista completa (usuario y sistema) con buscador.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

const SETTINGS: &str = "/data/adb/ia_optimizer/app_settings.json";

pub struct App {
    pub package: String,
    pub name: String,
    pub size_mb: u64,
    pub installed: String,
    pub is_game: bool,
    pub system: bool,
    pub profile: String,
    pub mode: String,
    pub cpu_max: String,
    pub gpu_max: String,
    pub io: String,
    pub hz: u32,
    pub perf: String,
    pub bypass: String,
    pub preload: String,
    pub fps: String,
}

#[derive(Clone, Default)]
pub struct Settings {
    pub mode: String,   // "auto" | "manual"
    /// Perfil propio de la app: "auto" | eco | balanced | performance.
    pub profile: String,
    pub cpu_max: String,
    pub gpu_max: String,
    pub io: String,
    pub hz: u32,
    /// None = sin decisión del usuario (se usa la auto-detección); Some = forzar.
    pub is_game: Option<bool>,
    /// "Modo de rendimiento ligero": "" (predeterminado) | light | balanced | performance.
    pub perf: String,
    /// Carga por bypass: "" (predeterminado) | off | on.
    pub bypass: String,
    /// Precarga del juego: "" (predeterminado) | on.
    pub preload: String,
    /// Tasa de fotogramas objetivo: "" | 60 | 90 | 120.
    pub fps: String,
}

fn jstr(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn shell(cmd: &str) -> Option<String> {
    let out = Command::new("sh").arg("-c").arg(cmd).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn shq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Icono real de una app: busca en el APK base y en los splits el PNG/WebP mas
/// parecido al icono de launcher (ic_launcher > mipmap/icon > mipmap) y lo
/// cachea en disco. Devuelve la ruta cacheada o None si no se pudo.
pub fn icon(pkg: &str) -> Option<std::path::PathBuf> {
    if pkg.is_empty()
        || pkg.contains(' ')
        || pkg.contains(';')
        || pkg.contains('$')
        || pkg.contains('\'')
        || pkg.contains('"')
        || pkg.contains('`')
    {
        return None;
    }
    let dir = std::path::PathBuf::from("/data/adb/ia_optimizer/icons");
    let _ = fs::create_dir_all(&dir);
    let safe = pkg.replace('/', "_");
    for ext in ["png", "webp"] {
        let out = dir.join(format!("{}.{}", safe, ext));
        if out.exists() {
            return Some(out);
        }
    }
    let paths = shell(&format!("pm path {} 2>/dev/null", pkg))?;
    let apks: Vec<String> = paths
        .lines()
        .filter_map(|l| l.strip_prefix("package:"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && Path::new(s).exists())
        .collect();
    if apks.is_empty() {
        return None;
    }
    let mut best: Option<(i32, i32, String, String)> = None;
    for apk in &apks {
        let listing = match shell(&format!("unzip -l {} 2>/dev/null", shq(apk))) {
            Some(l) => l,
            None => continue,
        };
        for line in listing.lines() {
            let e = match line.split_whitespace().last() {
                Some(e) => e,
                None => continue,
            };
            let lower = e.to_ascii_lowercase();
            if !e.starts_with("res/")
                || !(lower.ends_with(".png") || lower.ends_with(".webp"))
            {
                continue;
            }
            let name = if lower.contains("ic_launcher") {
                100
            } else if lower.contains("icon") && lower.contains("mipmap") {
                60
            } else if lower.contains("mipmap") {
                40
            } else if lower.contains("icon") {
                20
            } else {
                continue;
            };
            let dens = if lower.contains("xxxhdpi") {
                5
            } else if lower.contains("xxhdpi") {
                4
            } else if lower.contains("xhdpi") {
                3
            } else if lower.contains("hdpi") {
                2
            } else if lower.contains("mdpi") {
                1
            } else {
                0
            };
            let better = match &best {
                None => true,
                Some((bn, bd, _, _)) => (name, dens) > (*bn, *bd),
            };
            if better {
                best = Some((name, dens, e.to_string(), apk.clone()));
            }
        }
    }
    let (_, _, entry, apk) = best?;
    let ext = if entry.to_ascii_lowercase().ends_with(".webp") {
        "webp"
    } else {
        "png"
    };
    let out = dir.join(format!("{}.{}", safe, ext));
    let dest = out.to_string_lossy().to_string();
    let cmd = format!("unzip -p {} {} > {}", shq(&apk), shq(&entry), shq(&dest));
    let _ = Command::new("sh").arg("-c").arg(&cmd).output();
    if out.exists() {
        let ok = fs::read(&out)
            .map(|b| {
                b.len() > 8
                    && (b[0..4] == [0x89, b'P', b'N', b'G']
                        || b[0..4] == [b'R', b'I', b'F', b'F'])
            })
            .unwrap_or(false);
        if ok {
            return Some(out);
        }
        let _ = fs::remove_file(&out);
    }
    None
}

/// Paquetes de juegos conocidos / editores (heurística propia).
fn is_game_pkg(pkg: &str, name: &str) -> bool {
    const HINTS: &[&str] = &[
        "mihoyo", "hoyoverse", "tencent", "pubg", "activision", "riotgames", "supercell",
        "king", "epicgames", "nintendo", "moonton", "netease", "gameloft", "roblox",
        "minecraft", "niantic", "krafton", "garena", "freefire", "blizzard", "playkong",
    ];
    let p = pkg.to_ascii_lowercase();
    let n = name.to_ascii_lowercase();
    n.contains("game") || n.contains("war ") || n.contains("battle") || n.contains("racing")
        || HINTS.iter().any(|h| p.contains(h))
}

fn pretty(pkg: &str) -> String {
    let last = pkg.rsplit('.').next().unwrap_or(pkg);
    let cleaned = last.replace('_', " ");
    let mut out = String::new();
    for (i, w) in cleaned.split_whitespace().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let mut ch = w.chars();
        if let Some(f) = ch.next() {
            out.extend(f.to_uppercase());
            out.push_str(ch.as_str());
        }
    }
    if out.is_empty() {
        out.push_str(pkg);
    }
    out
}

fn read_settings() -> BTreeMap<String, Settings> {
    let mut m = BTreeMap::new();
    let Ok(s) = fs::read_to_string(SETTINGS) else {
        return m;
    };
    for l in s.lines() {
        let Some(pkg) = crate::state::get_json_str(l, "package") else {
            continue;
        };
        m.insert(
            pkg,
            Settings {
                mode: crate::state::get_json_str(l, "mode").unwrap_or_else(|| "auto".into()),
                profile: crate::state::get_json_str(l, "profile")
                    .unwrap_or_else(|| "auto".into()),
                cpu_max: crate::state::get_json_str(l, "cpu_max").unwrap_or_default(),
                gpu_max: crate::state::get_json_str(l, "gpu_max").unwrap_or_default(),
                io: crate::state::get_json_str(l, "io").unwrap_or_default(),
                hz: crate::state::get_json_str(l, "hz")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
                is_game: if l.contains("\"game_set\":true") {
                    Some(l.contains("\"is_game\":true"))
                } else {
                    None
                },
                perf: crate::state::get_json_str(l, "perf").unwrap_or_default(),
                bypass: crate::state::get_json_str(l, "bypass").unwrap_or_default(),
                preload: crate::state::get_json_str(l, "preload").unwrap_or_default(),
                fps: crate::state::get_json_str(l, "fps").unwrap_or_default(),
            },
        );
    }
    m
}

fn save_setting(pkg: &str, key: &str, value: &str) -> (bool, String) {
    if pkg.is_empty() || pkg.contains(' ') || pkg.contains(';') || pkg.contains('$') {
        return (false, "paquete no valido".into());
    }
    let mut all = read_settings();
    let s = all.entry(pkg.to_string()).or_default();
    match key {
        "mode" => {
            if value != "auto" && value != "manual" {
                return (false, "modo no valido (auto|manual)".into());
            }
            s.mode = value.to_string();
        }
        "profile" => {
            const OK: &[&str] = &["auto", "eco", "balanced", "performance"];
            if !OK.contains(&value) {
                return (false, "perfil no valido (auto|eco|balanced|performance)".into());
            }
            s.profile = value.to_string();
        }
        "cpu_max" => {
            if !value.is_empty() {
                match value.parse::<i64>() {
                    Ok(n) if n > 0 && n <= 3_500_000 => {}
                    _ => return (false, "frecuencia de CPU fuera de rango".into()),
                }
            }
            s.cpu_max = value.to_string();
        }
        "gpu_max" => {
            if !value.is_empty() {
                match value.parse::<i64>() {
                    Ok(n) if n > 0 && n <= 3_000_000_000 => {}
                    _ => return (false, "frecuencia de GPU fuera de rango".into()),
                }
            }
            s.gpu_max = value.to_string();
        }
        "io" => {
            const OK: &[&str] = &["", "none", "noop", "cfq", "bfq", "mq-deadline", "kyber", "deadline"];
            if !OK.contains(&value) {
                return (false, "planificador de E/S no valido".into());
            }
            s.io = value.to_string();
        }
        "hz" => {
            let v: u32 = value.parse().unwrap_or(0);
            if ![60, 90, 120].contains(&v) {
                return (false, "tasa de refresco no permitida (60/90/120)".into());
            }
            s.hz = v;
        }
        "is_game" => {
            if value == "auto" {
                s.is_game = None;
            } else {
                s.is_game = Some(value == "true");
            }
        }
        "perf" => {
            const OK: &[&str] = &["", "light", "balanced", "performance"];
            if !OK.contains(&value) {
                return (false, "modo de rendimiento no valido".into());
            }
            s.perf = value.to_string();
        }
        "bypass" => {
            const OK: &[&str] = &["", "off", "on"];
            if !OK.contains(&value) {
                return (false, "modo de bypass no valido".into());
            }
            s.bypass = value.to_string();
        }
        "preload" => {
            const OK: &[&str] = &["", "on"];
            if !OK.contains(&value) {
                return (false, "precarga no valida".into());
            }
            s.preload = value.to_string();
        }
        "fps" => {
            const OK: &[&str] = &["", "60", "90", "120"];
            if !OK.contains(&value) {
                return (false, "tasa de fotogramas no valida (60/90/120)".into());
            }
            s.fps = value.to_string();
        }
        _ => return (false, "ajuste desconocido".into()),
    }
    // Reescribir el archivo completo (pequeno: solo apps marcadas).
    let mut out = String::new();
    for (p, v) in &all {
        out.push_str(&format!(
            "{{\"package\":{},\"mode\":{},\"profile\":{},\"cpu_max\":{},\"gpu_max\":{},\"io\":{},\"hz\":{},\"is_game\":{},\"game_set\":{},\"perf\":{},\"bypass\":{},\"preload\":{},\"fps\":{}}}\n",
            jstr(p),
            jstr(&v.mode),
            jstr(&v.profile),
            jstr(&v.cpu_max),
            jstr(&v.gpu_max),
            jstr(&v.io),
            v.hz,
            v.is_game.unwrap_or(false),
            v.is_game.is_some(),
            jstr(&v.perf),
            jstr(&v.bypass),
            jstr(&v.preload),
            jstr(&v.fps)
        ));
    }
    let _ = fs::write(SETTINGS, out);
    (true, format!("{} = {} para {}", key, value, pkg))
}

/// Paquetes con su APK en una sola llamada (`pm list packages -f`).
fn packages_with_apk(system: bool) -> Vec<(String, String)> {
    let flag = if system { "-s" } else { "-3" };
    shell(&format!("pm list packages -f {} 2>/dev/null", flag))
        .map(|s| {
            s.lines()
                .filter_map(|l| {
                    let rest = l.trim().strip_prefix("package:")?;
                    let (apk, pkg) = rest.rsplit_once('=')?;
                    Some((pkg.trim().to_string(), apk.trim().to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn build(package: &str, apk: &str, system: bool, st: &Settings) -> App {
    let name = pretty(package);
    let auto = is_game_pkg(package, &name);
    let mut size_mb = 0u64;
    let mut installed = String::new();
    if !apk.is_empty() && Path::new(apk).exists() {
        if let Ok(m) = fs::metadata(apk) {
            size_mb = m.len() / 1024 / 1024;
            if let Ok(t) = m.modified() {
                if let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) {
                    installed = format!("{}", 1970 + d.as_secs() / 86400 / 365);
                }
            }
        }
    }
    let mode = if st.mode.is_empty() { "auto".to_string() } else { st.mode.clone() };
    let profile = if st.profile.is_empty() {
        "auto".to_string()
    } else {
        st.profile.clone()
    };
    App {
        package: package.to_string(),
        name,
        size_mb,
        installed,
        is_game: st.is_game.unwrap_or(auto),
        system,
        profile: profile.clone(),
        mode,
        cpu_max: st.cpu_max.clone(),
        gpu_max: st.gpu_max.clone(),
        io: st.io.clone(),
        hz: st.hz,
        perf: st.perf.clone(),
        bypass: st.bypass.clone(),
        preload: st.preload.clone(),
        fps: st.fps.clone(),
    }
}

/// Listado de aplicaciones. `filter`: all | user | system | games.
pub fn apps(filter: &str) -> Vec<App> {
    let settings = read_settings();
    let mut out = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let user = packages_with_apk(false);
    let sys = packages_with_apk(true);
    for (pkg, apk, system) in user
        .iter()
        .map(|(p, a)| (p, a, false))
        .chain(sys.iter().map(|(p, a)| (p, a, true)))
    {
        if seen.contains(pkg) {
            continue;
        }
        seen.push(pkg.clone());
        let st = settings.get(pkg).cloned().unwrap_or_default();
        let a = build(pkg, apk, system, &st);
        let keep = match filter {
            "user" => !a.system,
            "system" => a.system,
            "games" => a.is_game,
            _ => true,
        };
        if keep {
            out.push(a);
        }
    }
    out.sort_by(|a, b| b.is_game.cmp(&a.is_game).then(a.name.cmp(&b.name)));
    out
}

fn app_json(a: &App) -> String {
    format!(
        "{{\"package\":{},\"name\":{},\"size_mb\":{},\"installed\":{},\"is_game\":{},\"system\":{},\"mode\":{},\"cpu_max\":{},\"gpu_max\":{},\"io\":{},\"hz\":{},\"perf\":{},\"bypass\":{},\"preload\":{},\"fps\":{}}}",
        jstr(&a.package),
        jstr(&a.name),
        a.size_mb,
        jstr(&a.installed),
        a.is_game,
        a.system,
        jstr(&a.mode),
        jstr(&a.cpu_max),
        jstr(&a.gpu_max),
        jstr(&a.io),
        a.hz,
        jstr(&a.perf),
        jstr(&a.bypass),
        jstr(&a.preload),
        jstr(&a.fps)
    )
}

/// `/api/apps` (lista completa o filtrada).
pub fn apps_json(filter: &str) -> String {
    let list = apps(filter);
    let items: Vec<String> = list.iter().map(app_json).collect();
    format!(
        "{{\"filter\":{},\"apps\":[{}],\"count\":{}}}",
        jstr(filter),
        items.join(","),
        list.len()
    )
}

/// `/api/games`: solo aplicaciones marcadas como juego.
pub fn json() -> String {
    let list = apps("games");
    let items: Vec<String> = list.iter().map(app_json).collect();
    format!("{{\"games\":[{}],\"count\":{}}}", items.join(","), list.len())
}

pub fn set_setting(pkg: &str, key: &str, value: &str) -> (bool, String) {
    save_setting(pkg, key, value)
}

/// Información de paquete (version) para la pantalla por app.
pub fn info_json(pkg: &str) -> String {
    if pkg.is_empty() || pkg.contains(' ') || pkg.contains(';') || pkg.contains('$') {
        return "{\"error\":\"paquete no valido\"}".into();
    }
    let text = Command::new("sh")
        .arg("-c")
        .arg(format!("dumpsys package {} 2>/dev/null", pkg))
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let pick = |key: &str| {
        text.lines()
            .find(|l| l.contains(key))
            .and_then(|l| l.split(key).nth(1))
            .map(|s| s.trim().split_whitespace().next().unwrap_or("").to_string())
            .unwrap_or_default()
    };
    let version = pick("versionName=");
    let code = pick("versionCode=");
    format!(
        "{{\"package\":{},\"version\":{},\"version_code\":{}}}",
        jstr(pkg),
        jstr(&version),
        jstr(&code)
    )
}

/// Perfil propio de una app (la clave `profile` de su ficha).
/// Devuelve `None` cuando no hay ficha o está en "auto".
pub fn profile_of(pkg: &str) -> Option<String> {
    let s = read_settings().get(pkg)?.profile.clone();
    if s.is_empty() || s == "auto" {
        None
    } else {
        Some(s)
    }
}

/// Aplica los limites almacenados de una app (CPU, GPU, E/S y refresco).
/// Best-effort: informa cuantos ajustes se sostuvieron.
pub fn apply(pkg: &str) -> (bool, String) {
    let all = read_settings();
    let Some(s) = all.get(pkg) else {
        return (false, "sin ajustes para la app".into());
    };
    let knobs = iaopt_core::sysfs::Probe::new("/").scan();
    let mut ok = 0usize;
    let mut bad = 0usize;

    // Perfil propio de la app: si el usuario eligio uno (no "auto"), se aplica
    // el plan completo del perfil (gobernador, techo, E/S y modo de potencia).
    if s.profile != "auto" {
        if let Some(p) = iaopt_core::Profile::parse(&s.profile) {
            for a in iaopt_core::profiles::plan(p, &knobs) {
                if a.path == "-" {
                    continue;
                }
                let path = Path::new(&a.path);
                if iaopt_core::sysfs::write_verify_pin(path, &a.value, a.pin) {
                    ok += 1;
                } else {
                    bad += 1;
                }
            }
        }
    }
    if !s.cpu_max.is_empty() {
        if let Some(k) = knobs
            .iter()
            .find(|k| k.writable && k.path.ends_with("scaling_max_freq"))
        {
            if iaopt_core::sysfs::write_verify(Path::new(&k.path), &s.cpu_max) {
                ok += 1;
            } else {
                bad += 1;
            }
        }
    }
    if !s.gpu_max.is_empty() {
        if let Some(k) = knobs.iter().find(|k| {
            k.writable && k.domain == iaopt_core::sysfs::Domain::Gpu && k.path.ends_with("max_freq")
        }) {
            if iaopt_core::sysfs::write_verify(Path::new(&k.path), &s.gpu_max) {
                ok += 1;
            } else {
                bad += 1;
            }
        }
    }
    if !s.io.is_empty() {
        if let Some(k) = knobs
            .iter()
            .find(|k| k.writable && k.path.contains("queue/scheduler"))
        {
            if iaopt_core::sysfs::write_verify(Path::new(&k.path), &s.io) {
                ok += 1;
            } else {
                bad += 1;
            }
        }
    }
    if s.hz != 0 {
        if run_checked(
            &format!("settings put system peak_refresh_rate {}", s.hz),
            "refresco aplicado",
        )
        .0
        {
            ok += 1;
        } else {
            bad += 1;
        }
    }
    (
        ok > 0,
        format!("{} ajustes aplicados, {} no sostenidos", ok, bad),
    )
}

pub fn launch(pkg: &str) -> (bool, String) {
    if pkg.is_empty() || pkg.contains(' ') || pkg.contains(';') || pkg.contains('$') {
        return (false, "paquete no valido".into());
    }
    let cmd = format!("monkey -p {} -c android.intent.category.LAUNCHER 1", pkg);
    match Command::new("sh").arg("-c").arg(&cmd).output() {
        Ok(_) => (true, format!("iniciando {}", pkg)),
        Err(e) => (false, format!("no se pudo iniciar: {}", e)),
    }
}

/// HUD en vivo: frecuencias, carga, batería, temperatura y red.
pub fn hud() -> String {
    let t = crate::telemetry::sample(false);
    let cpu_ghz = t.cpu_cur_mhz.map(|m| m as f32 / 1000.0).unwrap_or(0.0);
    format!(
        "{{\"cpu_ghz\":{:.2},\"cpu_pct\":{},\"gpu_mhz\":{},\"battery_pct\":{},\"temp_c\":{},\"ram_gb\":{:.1},\"wifi\":{},\"data\":{}}}",
        cpu_ghz,
        crate::state::cpu_pct().unwrap_or(0),
        0,
        t.bat_pct.unwrap_or(0),
        t.thermal_zones.iter().map(|z| z.2).max().unwrap_or(0),
        (t.mem_total_mb.saturating_sub(t.mem_avail_mb)) as f32 / 1024.0,
        crate::state::quick_state("wifi"),
        crate::state::quick_state("data")
    )
}

/// Acciones rapidas con validacion estricta de la accion.
pub fn quick(action: &str, value: &str) -> (bool, String) {
    match action {
        "wifi_on" => run_checked("svc wifi enable", "wifi activado"),
        "wifi_off" => run_checked("svc wifi disable", "wifi desactivado"),
        "data_on" => run_checked("svc data enable", "datos activados"),
        "data_off" => run_checked("svc data disable", "datos desactivados"),
        "dnd_on" => run_checked("settings put global zen_mode 2", "no molestar activado"),
        "dnd_off" => run_checked("settings put global zen_mode 0", "no molestar desactivado"),
        "hz" => {
            let hz: u32 = value.parse().unwrap_or(0);
            if ![60, 90, 120].contains(&hz) {
                return (false, "frecuencia de refresco no permitida (60/90/120)".into());
            }
            let _ = run_checked(
                &format!("settings put system peak_refresh_rate {}", hz),
                "refresco aplicado",
            );
            run_checked(
                &format!("settings put system min_refresh_rate {}", hz),
                &format!("refresco {} Hz", hz),
            )
        }
        _ => (false, "accion desconocida".into()),
    }
}

fn run_checked(cmd: &str, ok: &str) -> (bool, String) {
    match Command::new("sh").arg("-c").arg(cmd).output() {
        Ok(_) => (true, ok.into()),
        Err(e) => (false, format!("no se pudo aplicar: {}", e)),
    }
}
