//! Perfil universal de dispositivo: identifica SoC, GPU y capacidades reales, y
//! decide qué funciones son compatibles. Solo se activa lo que el dispositivo
//! expone de verdad.

use std::fs;

const CFG: &str = "/data/adb/ia_optimizer/device.json";
const CFG_ALT: &str = "/data/adb/.config/AiOptimizer/device_profile.json";

/// Base de datos minima de SoCs conocidos (modelo -> vendor, pistas de capacidad).
/// Es una pista: solo se aplica lo que el dispositivo realmente expone.
fn known_soc(model: &str) -> Option<(&'static str, &'static str)> {
    const DB: &[(&str, &str, &str)] = &[
        ("mt6785", "Mediatek", "ppm,mtk_cmd,devfreq,mali"),
        ("mt6873", "Mediatek", "ppm,mtk_cmd,devfreq,mali"),
        ("mt6877", "Mediatek", "ppm,mtk_cmd,devfreq,mali"),
        ("mt6885", "Mediatek", "ppm,mtk_cmd,devfreq,mali"),
        ("mt6893", "Mediatek", "ppm,mtk_cmd,devfreq,mali"),
        ("mt6983", "Mediatek", "ppm,mtk_cmd,devfreq,mali"),
        ("sm8250", "Qualcomm", "kgsl,qcom-battery,devfreq"),
        ("sm8350", "Qualcomm", "kgsl,qcom-battery,devfreq"),
        ("sm8450", "Qualcomm", "kgsl,qcom-battery,devfreq"),
        ("sm8550", "Qualcomm", "kgsl,qcom-battery,devfreq"),
        ("sm8650", "Qualcomm", "kgsl,qcom-battery,devfreq"),
        ("exynos2100", "Samsung", "ppm,mali,thermal_zones"),
        ("exynos2200", "Samsung", "ppm,mali,thermal_zones"),
        ("exynos2400", "Samsung", "ppm,mali,thermal_zones"),
        ("ums517", "Unisoc", "ppm,mali,thermal_zones"),
        ("t618", "Unisoc", "ppm,mali,thermal_zones"),
        ("gs101", "Google", "devfreq,thermal_zones"),
        ("gs201", "Google", "devfreq,thermal_zones"),
    ];
    let m = model.to_ascii_lowercase();
    DB.iter()
        .find(|(k, _, _)| m.contains(k))
        .map(|(_, v, f)| (*v, *f))
}

#[derive(Clone)]
pub struct Profile {
    pub device: String,
    pub soc: String,
    pub soc_vendor: String,
    pub gpu: String,
    pub kernel: String,
    pub android: String,
    pub features: Vec<String>,
}

fn read(p: &str) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

fn getprop(prop: &str) -> Option<String> {
    let out = std::process::Command::new("getprop").arg(prop).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn exists(p: &str) -> bool {
    std::path::Path::new(p).exists()
}

/// Directorios que indican un tipo de GPU.
fn detect_gpu() -> String {
    if exists("/sys/class/kgsl") {
        return "Adreno".into();
    }
    // Mali solo si expone nodos reales (algunos SoC dejan el directorio vacio).
    let mali_real = exists("/sys/class/misc/mali0/device")
        || exists("/sys/class/misc/mali0/devfreq")
        || exists("/sys/class/mali");
    if mali_real {
        // Nombre exacto si el driver lo expone (p. ej. "Mali-G76 4 cores r0p0").
        if let Some(gi) = read("/sys/class/misc/mali0/device/gpuinfo") {
            let first: Vec<&str> = gi.split_whitespace().take(2).collect();
            if !first.is_empty() {
                return first.join(" ");
            }
        }
        return "Mali".into();
    }
    if let Some(egl) = getprop("ro.hardware.egl") {
        let e = egl.to_ascii_lowercase();
        if e.contains("powervr") || e.contains("meow") {
            return "PowerVR".into();
        }
        if e.contains("adreno") {
            return "Adreno".into();
        }
        if e.contains("mali") {
            return "Mali".into();
        }
        if !e.is_empty() {
            return egl;
        }
    }
    if exists("/sys/class/drm") {
        return "DRM/Generic".into();
    }
    "Desconocido".into()
}

fn detect_soc(cpuinfo: &str, props: &str) -> (String, String) {
    let mut hay = format!("{} {}", cpuinfo, props).to_ascii_lowercase();
    // todas las lineas utiles de /proc/cpuinfo
    if let Some(ci) = read("/proc/cpuinfo") {
        hay.push(' ');
        hay.push_str(&ci.to_ascii_lowercase());
    }
    let vendor = if hay.contains("mtk") || hay.contains("mediatek") || hay.contains("mt67")
        || hay.contains("mt68") || hay.contains("mt8")
    {
        "Mediatek"
    } else if hay.contains("qualcomm") || hay.contains("msm") || hay.contains("sm8")
        || hay.contains("sm7") || hay.contains("sm6") || hay.contains("lahaina")
        || hay.contains("crown") || hay.contains("yupik") || hay.contains("cape")
    {
        "Qualcomm"
    } else if hay.contains("exynos") || hay.contains("s5p") || hay.contains("samsung")
    {
        "Samsung"
    } else if hay.contains("unisoc") || hay.contains("ums51") || hay.contains("t618")
        || hay.contains("t606") || hay.contains("sc98")
    {
        "Unisoc"
    } else if hay.contains("tensor") || hay.contains("gs101") || hay.contains("gs201")
    {
        "Google"
    } else {
        "Desconocido"
    };
    let soc = getprop("ro.soc.model")
        .or_else(|| getprop("ro.board.platform"))
        .or_else(|| {
            cpuinfo
                .lines()
                .find(|l| {
                    let l = l.to_ascii_lowercase();
                    l.starts_with("hardware") || l.starts_with("model name")
                })
                .and_then(|l| l.split(':').nth(1).map(|s| s.trim().to_string()))
        })
        .unwrap_or_else(|| "Desconocido".into());
    (vendor.to_string(), soc)
}

pub fn detect() -> Profile {
    let cpuinfo = read("/proc/cpuinfo").unwrap_or_default();
    let props = format!(
        "{} {}",
        getprop("ro.board.platform").unwrap_or_default(),
        getprop("ro.hardware").unwrap_or_default()
    );
    let (soc_vendor, soc) = detect_soc(&cpuinfo, &props);
    let gpu = detect_gpu();

    let mut features = Vec::new();
    let mut push = |k: &str, ok: bool| {
        if ok {
            features.push(k.to_string());
        }
    };
    push("cpu_governor", exists("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"));
    push("cpu_freq", exists("/sys/devices/system/cpu/cpu0/cpufreq/scaling_max_freq"));
    push("devfreq", exists("/sys/class/devfreq"));
    push("kgsl", exists("/sys/class/kgsl"));
    push("mali", exists("/sys/class/misc/mali0/device") || exists("/sys/class/mali"));
    push("thermal_zones", exists("/sys/class/thermal/thermal_zone0/temp"));
    push("psi", exists("/proc/pressure/memory"));
    push("zram", exists("/sys/block/zram0"));
    push("io_scheduler", exists("/sys/block"));
    push("network", exists("/proc/sys/net/ipv4/tcp_congestion_control"));
    push("responsiveness", exists("/proc/sys/kernel/sched_latency_ns"));
    push("ppm", exists("/proc/ppm/policy/hard_userlimit_max_cpu_freq"));
    push("mtk_cmd", exists("/proc/mtk_battery_cmd/en_power_path"));
    push("bypass", crate::bypass::scan_nodes().iter().any(|n| n.exists));
    push("renderer", getprop("debug.hwui.renderer").is_some());

    Profile {
        device: getprop("ro.product.model").unwrap_or_else(|| "Desconocido".into()),
        soc,
        soc_vendor,
        gpu,
        kernel: getprop("ro.build.version.release")
            .map(|v| format!("Android {}", v))
            .unwrap_or_default(),
        android: read("/proc/version").unwrap_or_default(),
        features,
    }
}

/// Propiedad del sistema (getprop), best-effort.
fn prop(k: &str) -> String {
    std::process::Command::new("getprop")
        .arg(k)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

pub fn load() -> Profile {
    if let Ok(s) = fs::read_to_string(CFG) {
        if let Some(v) = crate::state::get_json_str(&s, "device") {
            if !v.is_empty() {
                // Si el perfil cacheado no corresponde a este equipo, se regenera.
                let cur = prop("ro.product.device");
                if !cur.is_empty() && cur != v {
                    let np = detect();
                    np.save();
                    return np;
                }
                let p = Profile {
                    device: v,
                    soc: crate::state::get_json_str(&s, "soc").unwrap_or_default(),
                    soc_vendor: crate::state::get_json_str(&s, "soc_vendor").unwrap_or_default(),
                    gpu: crate::state::get_json_str(&s, "gpu").unwrap_or_default(),
                    kernel: crate::state::get_json_str(&s, "kernel").unwrap_or_default(),
                    android: crate::state::get_json_str(&s, "android").unwrap_or_default(),
                    features: crate::state::get_json_str(&s, "features")
                        .map(|f| f.split(',').map(|x| x.to_string()).collect())
                        .unwrap_or_default(),
                };
                // Publicar tambien en la ruta que consume la WebUI.
                let _ = fs::create_dir_all("/data/adb/.config/AiOptimizer");
                let feats = p.features.join(",");
                let body = format!(
                    "{{\"device\":{},\"soc\":{},\"soc_vendor\":{},\"gpu\":{},\"kernel\":{},\"android\":{},\"features\":{}}}\n",
                    jstr(&p.device),
                    jstr(&p.soc),
                    jstr(&p.soc_vendor),
                    jstr(&p.gpu),
                    jstr(&p.kernel),
                    jstr(&p.android),
                    jstr(&feats)
                );
                let _ = fs::write(CFG_ALT, body);
                return p;
            }
        }
    }
    let p = detect();
    p.save();
    p
}

/// Perfil ya cacheado: lo vuelve a publicar en la ruta expuesta.
pub fn publish(p: &Profile) {
    p.save();
}

impl Profile {
    pub fn save(&self) {
        let feats = self.features.join(",");
        let body = format!(
            "{{\"device\":{},\"soc\":{},\"soc_vendor\":{},\"gpu\":{},\"kernel\":{},\"android\":{},\"features\":{}}}\n",
            jstr(&self.device),
            jstr(&self.soc),
            jstr(&self.soc_vendor),
            jstr(&self.gpu),
            jstr(&self.kernel),
            jstr(&self.android),
            jstr(&feats)
        );
        let _ = fs::write(CFG, &body);
        // Copia en la ruta pedida por la WebUI (perfil expuesto).
        let _ = fs::create_dir_all("/data/adb/.config/AiOptimizer");
        let _ = fs::write(CFG_ALT, body);
    }

    /// Pista de la base de datos de SoCs conocidos (si coincide).
    pub fn db_hint(&self) -> Option<(&'static str, &'static str)> {
        known_soc(&self.soc)
    }

    /// ¿El dispositivo es compatible con esta funcion?
    pub fn supports(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }
}

fn jstr(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Compatibilidad de un camino sysfs con el perfil (para filtrar acciones de IA).
pub fn path_compatible(path: &str, p: &Profile) -> bool {
    let path = path.to_ascii_lowercase();
    if path.contains("kgsl") {
        return p.supports("kgsl");
    }
    if path.contains("mali") {
        return p.supports("mali");
    }
    if path.contains("/proc/ppm") {
        return p.supports("ppm");
    }
    if path.contains("mtk_battery_cmd") {
        return p.supports("mtk_cmd");
    }
    if path.contains("/sys/class/devfreq") {
        return p.supports("devfreq");
    }
    if path.contains("thermal_zone") {
        return p.supports("thermal_zones");
    }
    if path.contains("cpufreq") || path.contains("scaling_") {
        return p.supports("cpu_freq") || p.supports("cpu_governor");
    }
    true
}
