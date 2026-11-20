//! Motor adaptativo deterministico ("IA"): elige perfil a partir de telemetria.
//!
/// No es una red neuronal ni una caja negra: son reglas declaradas, con confianza
/// y histeresis para no oscilar entre perfiles. Cada decision es explicable.

use crate::Profile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Foreground {
    Game,
    Video,
    Social,
    Other,
}

impl Default for Foreground {
    fn default() -> Self {
        Foreground::Other
    }
}

/// Clasifica el paquete en primer plano para el modo automatico inteligente.
pub fn classify(pkg: &str) -> Foreground {
    let p = pkg.to_ascii_lowercase();
    if p.is_empty() {
        return Foreground::Other;
    }
    const VIDEO: [&str; 24] = [
        "youtube", "netflix", "disney", "primevideo", "hbo", "twitch", "tiktok", "musically",
        "zhiliaoapp", "aweme", "vimeo", "dailymotion", "kodi", "plex", "chromecast",
        "smarttube", "mxplayer", "videoland", "instatv", "ubergizmo", "telemundo", "vix",
        "blim", "stan",
    ];
    const SOCIAL: [&str; 22] = [
        "facebook", "katana", "instagram", "twitter", "snapchat", "telegram", "whatsapp",
        "discord", "reddit", "pinterest", "linkedin", "tinder", "badoo", "meetme", "weibo",
        "wechat", "line", "threads", "com.facebook.orca", "com.vkontakte", "com.ok.ru",
        "com.snap",
    ];
    const GAME: [&str; 30] = [
        "pixonic", "supercell", "activision", "epicgames", "roblox", "mojang", "garena",
        "pubg", "tencent", "mihoyo", "hoyoverse", "nexon", "netmarble", "krafton",
        "blizzard", "gameloft", "zynga", "playrix", "superevilmegacorp", "hypercell",
        "easybrain", "ironsource", "scopely", "playtika", "dreamgames", "cents",
        "com.gg", "com.igg", "com.ea.", "com.wb.",
    ];
    if GAME.iter().any(|k| p.contains(k)) {
        return Foreground::Game;
    }
    if VIDEO.iter().any(|k| p.contains(k)) {
        return Foreground::Video;
    }
    if SOCIAL.iter().any(|k| p.contains(k)) {
        return Foreground::Social;
    }
    Foreground::Other
}

#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    /// Carga media de CPU 0.0..1.0 (media de /proc/stat).
    pub cpu_load: f32,
    /// Temperatura de bateria en grados Celsius.
    pub battery_temp_c: f32,
    /// Nivel de bateria en porcentaje (0-100).
    pub battery_pct: u8,
    /// true si esta cargando.
    pub charging: bool,
    /// true si hay una app de juego en primer plano.
    pub gaming: bool,
    /// Tipo de app en primer plano (juego, video, red social, otro).
    pub foreground: Foreground,
}

#[derive(Debug, Clone)]
pub struct Suggestion {
    pub profile: Profile,
    pub reason: String,
    pub confidence: f32,
}

impl Suggestion {
    fn new(p: Profile, reason: &str, confidence: f32) -> Self {
        Self {
            profile: p,
            reason: reason.to_string(),
            confidence: confidence.clamp(0.0, 1.0),
        }
    }
}

/// Regla unica y legible: ordenada por prioridad, la primera que dispara gana.
pub fn suggest(m: &Metrics) -> Suggestion {
    let t = m.battery_temp_c;
    let l = m.cpu_load.clamp(0.0, 1.0);

    if t >= 47.0 {
        return Suggestion::new(Profile::Eco, "temperatura crítica de batería", 0.95);
    }
    if t >= 42.0 && !m.charging {
        return Suggestion::new(Profile::Eco, "temperatura alta y sin cargar", 0.85);
    }
    // Sostenibilidad: con poca batería el ahorro manda sobre el rendimiento.
    if !m.charging && m.battery_pct <= 15 {
        return Suggestion::new(Profile::Eco, "batería baja: ahorro prioritario", 0.92);
    }
    // 2) Contexto de la app en primer plano (modo automatico inteligente).
    match m.foreground {
        Foreground::Game => {
            if !m.charging && m.battery_pct <= 30 {
                return Suggestion::new(
                    Profile::Balanced,
                    "juego con batería moderada: rendimiento contenido",
                    0.85,
                );
            }
            return Suggestion::new(Profile::Performance, "juego en primer plano", 0.92)
        }
        Foreground::Video => {
            return Suggestion::new(
                Profile::Eco,
                "reproduciendo vídeo: ahorro inteligente",
                0.8,
            )
        }
        Foreground::Social => {
            return Suggestion::new(
                Profile::Balanced,
                "red social: fluidez con consumo moderado",
                0.72,
            )
        }
        Foreground::Other => {}
    }

    if m.gaming && l > 0.5 {
        return Suggestion::new(Profile::Performance, "juego activo con carga sostenida", 0.9);
    }
    if m.charging && l > 0.8 {
        return Suggestion::new(Profile::Performance, "carga pesada mientras se carga", 0.8);
    }
    if l < 0.2 && !m.charging {
        return Suggestion::new(Profile::Eco, "inactividad y con batería", 0.6);
    }
    Suggestion::new(Profile::Balanced, "sin señal dominante", 0.5)
}

/// Decision con histeresis: evita cambios por ruido de telemetria.
pub fn decide(prev: Option<Profile>, m: &Metrics) -> Suggestion {
    let s = suggest(m);
    if s.confidence < 0.7 {
        if let Some(p) = prev {
            return Suggestion::new(
                p,
                &format!("histéresis: se mantiene {} ({})", p.as_str(), s.reason),
                1.0,
            );
        }
    }
    s
}

/// Resumen legible del efecto medido despues de aplicar un cambio.
/// Lo usa el journal de IA Motor para que cada accion tenga un resultado.
pub fn outcome(before: &Metrics, after: &Metrics) -> String {
    let dt = after.battery_temp_c - before.battery_temp_c;
    let dc = (after.cpu_load - before.cpu_load) * 100.0;
    format!("temp {:+.1} °C · cpu {:+.0}%", dt, dc)
}
