//! Descubrimiento y acceso seguro a interfaces del kernel (/sys, /proc).
//!
//! El escaneo solo expone rutas que *existen*: nada de interruptores falsos.
//! Todas las lecturas/escrituras se hacen sobre una raiz inyectable (`Probe::new`)
//! para poder probar la logica en el PC sin un dispositivo Android.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    CpuGovernor,
    CpuFreq,
    Gpu,
    IoScheduler,
    Thermal,
    Memory,
    Network,
    Responsiveness,
    Unknown,
}

impl Domain {
    pub fn as_str(self) -> &'static str {
        match self {
            Domain::CpuGovernor => "cpu_governor",
            Domain::CpuFreq => "cpu_freq",
            Domain::Gpu => "gpu",
            Domain::IoScheduler => "io_scheduler",
            Domain::Thermal => "thermal",
            Domain::Memory => "memory",
            Domain::Network => "network",
            Domain::Responsiveness => "responsiveness",
            Domain::Unknown => "unknown",
        }
    }
}

/// Un control_kernel expuesto por este dispositivo.
#[derive(Debug, Clone)]
pub struct Knob {
    pub domain: Domain,
    /// Ruta absoluta real (en Android empieza con /sys o /proc).
    pub path: String,
    pub writable: bool,
    pub current: Option<String>,
    /// Valores validos cuando la interfaz es un enum (p. ej. gobernadores).
    pub allowed: Vec<String>,
    /// Rango seguro (min, max) para valores numericos.
    pub min: Option<i64>,
    pub max: Option<i64>,
}

impl Knob {
    pub fn describe(&self) -> String {
        let cur = self.current.as_deref().unwrap_or("-");
        if self.allowed.is_empty() {
            format!("{} = {}", self.path, cur)
        } else {
            format!("{} = {}  [{}]", self.path, cur, self.allowed.join(","))
        }
    }
}

pub struct Probe {
    root: PathBuf,
}

impl Probe {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn p(&self, rel: &str) -> PathBuf {
        self.root.join(rel.trim_start_matches('/'))
    }

    /// Ruta real detras de una ruta logica. En Android `cpu0/cpufreq` es un
    /// symlink a `../cpufreq/policy0`: dos rutas distintas, el mismo archivo.
    fn canonical(&self, rel: &str) -> PathBuf {
        let p = self.p(rel);
        fs::canonicalize(&p).unwrap_or(p)
    }

    /// Escanea las interfaces conocidas y devuelve las que existen y son legibles.
    pub fn scan(&self) -> Vec<Knob> {
        let mut out = Vec::new();

        // CPU: cpufreq. Primero las politicas reales (canonicas y presentes en
        // cualquier SoC) y despues las rutas clasicas cpu*/cpufreq, que en
        // algunos kernels son la unica forma de llegar al mismo nodo.
        for pol in self.subdirs("/sys/devices/system/cpu/cpufreq") {
            if !pol.starts_with("policy") {
                continue;
            }
            let base = format!("/sys/devices/system/cpu/cpufreq/{}", pol);
            for (domain, leaf) in [
                (Domain::CpuGovernor, "scaling_available_governors"),
                (Domain::CpuGovernor, "scaling_governor"),
                (Domain::CpuFreq, "scaling_max_freq"),
                (Domain::CpuFreq, "scaling_min_freq"),
                (Domain::CpuFreq, "cpuinfo_max_freq"),
                (Domain::CpuFreq, "cpuinfo_min_freq"),
            ] {
                self.push_knob(&mut out, domain, &format!("{}/{}", base, leaf));
            }
        }
        let mut cores = self.subdirs("/sys/devices/system/cpu");
        cores.retain(|n| {
            n.starts_with("cpu") && n.len() > 3 && n[3..].chars().all(|c| c.is_ascii_digit())
        });
        cores.sort();
        for c in cores {
            let base = format!("/sys/devices/system/cpu/{}/cpufreq", c);
            for (domain, leaf) in [
                (Domain::CpuGovernor, "scaling_governor"),
                (Domain::CpuFreq, "scaling_max_freq"),
                (Domain::CpuFreq, "scaling_min_freq"),
                (Domain::CpuFreq, "cpuinfo_max_freq"),
                (Domain::CpuFreq, "cpuinfo_min_freq"),
            ] {
                self.push_knob(&mut out, domain, &format!("{}/{}", base, leaf));
            }
        }

        // E/S: solo almacenamiento real (sd*, mmcblk*, nvme*, vd*). loop/ram/
        // zram/dm-* son dispositivos virtuales: o no tienen planificador
        // ajustable o solo ofrecen "none", y llenaban la lista de ruido.
        for blk in self.subdirs("/sys/block") {
            let real = blk.starts_with("sd")
                || blk.starts_with("mmcblk")
                || blk.starts_with("nvme")
                || blk.starts_with("vd");
            if !real {
                continue;
            }
            self.push_knob(
                &mut out,
                Domain::IoScheduler,
                &format!("/sys/block/{}/queue/scheduler", blk),
            );
        }
        self.push_knob(&mut out, Domain::IoScheduler, "/sys/block/zram0/comp_algorithm");

        // Termico: zonas reales
        for z in self.subdirs("/sys/class/thermal") {
            if !z.starts_with("thermal_zone") {
                continue;
            }
            for f in ["temp", "type", "policy"] {
                self.push_knob(
                    &mut out,
                    Domain::Thermal,
                    &format!("/sys/class/thermal/{}/{}", z, f),
                );
            }
        }

        // GPU: Adreno (kgsl) y Mali
        for rel in [
            "/sys/class/kgsl/kgsl-3d0/devfreq/governor",
            "/sys/class/kgsl/kgsl-3d0/devfreq/available_frequencies",
            "/sys/class/kgsl/kgsl-3d0/devfreq/max_freq",
            "/sys/class/kgsl/kgsl-3d0/devfreq/min_freq",
            "/sys/class/kgsl/kgsl-3d0/devfreq/cur_freq",
            "/sys/class/kgsl/kgsl-3d0/gpu_available_frequencies",
            "/sys/class/kgsl/kgsl-3d0/max_gpuclk",
            "/sys/class/misc/mali0/device/dvfs_governor",
            "/sys/class/misc/mali0/device/available_frequencies",
            "/sys/class/misc/mali0/device/max_freq",
            "/sys/class/misc/mali0/device/min_freq",
        ] {
            self.push_knob(&mut out, Domain::Gpu, rel);
        }
        // devfreq genérico (GPU, memoria MIF/INT, etc.)
        for d in self.subdirs("/sys/class/devfreq") {
            for f in ["governor", "cur_freq", "min_freq", "max_freq"] {
                self.push_knob(
                    &mut out,
                    Domain::Gpu,
                    &format!("/sys/class/devfreq/{}/{}", d, f),
                );
            }
        }
        // Limitadores de CPU por software (ppm / cpufreq_power_mode)
        for rel in [
            "/proc/ppm/policy/hard_userlimit_max_cpu_freq",
            "/proc/ppm/policy/hard_userlimit_min_cpu_freq",
            "/proc/cpufreq/cpufreq_power_mode",
        ] {
            self.push_knob(&mut out, Domain::CpuFreq, rel);
        }

        // Memoria / maquina virtual
        for rel in [
            "/sys/module/lowmemorykiller/parameters/minfree",
            "/sys/block/zram0/disksize",
            "/proc/sys/vm/swappiness",
            "/proc/sys/vm/vfs_cache_pressure",
            "/proc/sys/vm/dirty_ratio",
            "/proc/sys/vm/dirty_background_ratio",
        ] {
            self.push_knob(&mut out, Domain::Memory, rel);
        }

        // Red
        for rel in [
            "/proc/sys/net/ipv4/tcp_congestion_control",
            "/proc/sys/net/ipv4/tcp_fastopen",
            "/proc/sys/net/core/rmem_max",
            "/proc/sys/net/core/wmem_max",
        ] {
            self.push_knob(&mut out, Domain::Network, rel);
        }

        // Sensibilidad / planificacion
        for rel in [
            "/proc/sys/kernel/sched_latency_ns",
            "/proc/sys/kernel/sched_min_granularity_ns",
            "/proc/sys/kernel/sched_wakeup_granularity_ns",
        ] {
            self.push_knob(&mut out, Domain::Responsiveness, rel);
        }

        // Panel táctil: nodos del vendor (solo si el kernel los expone).
        // Se enumeran los directorios reales en vez de fijar rutas de un
        // fabricante: cada ROM los coloca en un sitio distinto. Tope de 30
        // nodos para no llenar la lista de ruido en kernels muy verbosos.
        let mut touch_count = 0;
        for base in [
            "/sys/class/touchpanel",
            "/sys/devices/virtual/touchpanel",
            "/proc/touchpanel",
            "/sys/touchpanel",
        ] {
            for f in self.files(base) {
                if touch_count >= 30 { break; }
                let before = out.len();
                self.push_knob(&mut out, Domain::Responsiveness, &format!("{}/{}", base, f));
                if out.len() > before { touch_count += 1; }
            }
            for sub in self.subdirs(base) {
                let dir = format!("{}/{}", base, sub);
                for f in self.files(&dir) {
                    if touch_count >= 30 { break; }
                    let before = out.len();
                    self.push_knob(&mut out, Domain::Responsiveness, &format!("{}/{}", dir, f));
                    if out.len() > before { touch_count += 1; }
                }
            }
        }

        // Una sola tarjeta por nodo real: si dos rutas resuelven al mismo
        // archivo (symlink) queda la primera, que es la canonica de la politica
        // (las politicas se anadieron antes que las rutas cpu*/cpufreq).
        // Ademas se deduplica por (nodo, cluster): algunos kernels exponen
        // cpu*/cpufreq como directorios reales (no symlink) y el mismo control
        // aparecia dos veces con rutas distintas.
        let mut cpu_pol: Vec<(String, String)> = Vec::new();
        for pol in self.subdirs("/sys/devices/system/cpu/cpufreq") {
            if !pol.starts_with("policy") {
                continue;
            }
            let base = format!("/sys/devices/system/cpu/cpufreq/{}", pol);
            let cpus = read_trim(&self.p(&format!("{}/related_cpus", base)))
                .or_else(|| read_trim(&self.p(&format!("{}/affected_cpus", base))))
                .unwrap_or_default();
            for c in cpus.split_whitespace() {
                cpu_pol.push((format!("cpu{}", c), pol.clone()));
            }
        }
        let cluster_of = |path: &str| -> String {
            if let Some(i) = path.find("policy") {
                let num: String = path[i + 6..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                if !num.is_empty() {
                    return format!("policy{}", num);
                }
            }
            for (cpu, pol) in &cpu_pol {
                if path.contains(&format!("/{}/", cpu)) {
                    return pol.clone();
                }
            }
            String::new()
        };
        let mut seen: Vec<PathBuf> = Vec::new();
        let mut seen_cluster: Vec<String> = Vec::new();
        out.retain(|k| {
            let real = self.canonical(&k.path);
            if seen.iter().any(|s| *s == real) {
                return false;
            }
            if k.domain == Domain::CpuFreq || k.domain == Domain::CpuGovernor {
                let leaf = k.path.rsplit('/').next().unwrap_or("");
                let cl = cluster_of(&k.path);
                if !cl.is_empty() {
                    let key = format!("{}@{}", leaf, cl);
                    if seen_cluster.contains(&key) {
                        return false;
                    }
                    seen_cluster.push(key);
                }
            }
            seen.push(real);
            true
        });
        // Planificadores sin opciones reales (solo "none"): no hay nada que
        // ajustar; fuera de la lista para no repetir filas vacias.
        out.retain(|k| {
            if k.domain == Domain::IoScheduler && k.path.ends_with("queue/scheduler") {
                k.allowed.len() > 1
            } else {
                true
            }
        });
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    /// Nombres de subdirectorios existentes bajo `rel` (para enumerar bloques,
    /// zonas termicas y politicas cpufreq reales).
    fn subdirs(&self, rel: &str) -> Vec<String> {
        let dir = self.p(rel);
        let Ok(rd) = fs::read_dir(&dir) else {
            return Vec::new();
        };
        rd.filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect()
    }

    /// Nombres de archivos regulares existentes bajo `rel` (nodos táctiles).
    fn files(&self, rel: &str) -> Vec<String> {
        let dir = self.p(rel);
        let Ok(rd) = fs::read_dir(&dir) else {
            return Vec::new();
        };
        rd.filter_map(|e| e.ok())
            .filter(|e| e.metadata().map(|m| m.is_file()).unwrap_or(false))
            .filter_map(|e| e.file_name().into_string().ok())
            .collect()
    }

    fn push_knob(&self, out: &mut Vec<Knob>, domain: Domain, rel: &str) {
        let path = self.p(rel);
        let Ok(meta) = fs::metadata(&path) else { return };
        if !meta.is_file() && !path.exists() {
            return;
        }
        let current = fs::read_to_string(&path).ok().map(|s| s.trim().to_string());
        if current.is_none() {
            return; // no legible -> no lo mostramos (evita controles muertos)
        }
        let writable = is_writable(&path);
        // Modo de potencia del chip: es un enum numerico cuyo contenido es texto
        // ("Default(Normal) mode", "Low Power mode", ...). Se normaliza a la
        // cifra para que la app lo muestre como un selector y el tuner pueda
        // verificar la escritura por relectura.
        if rel.ends_with("cpufreq_power_mode") {
            let mode = current
                .as_deref()
                .and_then(power_mode_value)
                .unwrap_or_else(|| "0".into());
            out.push(Knob {
                domain,
                path: rel.to_string(),
                writable,
                current: Some(mode.clone()),
                allowed: ["0", "1", "2", "3"].iter().map(|s| s.to_string()).collect(),
                min: None,
                max: None,
            });
            return;
        }
        let allowed = if rel.contains("available_governors")
            || rel.contains("scheduler")
            || rel.contains("congestion_control")
            || rel.contains("comp_algorithm")
            || rel.contains("available_frequencies")
        {
            current
                .clone()
                .map(|s| parse_enums(&s))
                .unwrap_or_default()
        } else if rel.ends_with("scaling_governor") {
            // heredar los valores validos del sibling scaling_available_governors
            let sib = self.p(&rel.replace("scaling_governor", "scaling_available_governors"));
            read_trim(&sib)
                .map(|s| parse_enums(&s))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let (range, range_max) = self.safe_range(rel, &current);
        out.push(Knob {
            domain,
            path: rel.to_string(),
            writable,
            current,
            allowed,
            min: range,
            max: range_max,
        });
    }

    /// Rango seguro que el kernel/administrador soporta para este nodo.
    /// (min, max) en las mismas unidades que el valor del nodo.
    fn safe_range(&self, rel: &str, current: &Option<String>) -> (Option<i64>, Option<i64>) {
        let num = |s: &Option<String>| s.as_deref().and_then(|x| x.trim().parse::<i64>().ok());
        // Frecuencias: limites de fabrica del mismo cluster.
        if rel.contains("freq") && rel.contains("scaling_") {
            let base = rel
                .rsplit_once('/')
                .map(|(d, _)| d.to_string())
                .unwrap_or_default();
            let as_hz = |p: &str| -> Option<i64> {
                read_trim(Path::new(p))?
                    .trim()
                    .parse::<i64>()
                    .ok()
            };
            let lo = as_hz(&format!("{}/cpuinfo_min_freq", base));
            let hi = as_hz(&format!("{}/cpuinfo_max_freq", base));
            return (lo, hi);
        }
        if rel.ends_with("sched_latency_ns") {
            return (Some(1_000_000), Some(50_000_000));
        }
        if rel.ends_with("sched_min_granularity_ns") || rel.ends_with("sched_wakeup_granularity_ns") {
            return (Some(100_000), Some(5_000_000));
        }
        if rel.ends_with("rmem_max") || rel.ends_with("wmem_max") {
            return (Some(0), Some(16_777_216));
        }
        if rel.ends_with("swappiness") {
            return (Some(0), Some(200));
        }
        if rel.ends_with("dirty_ratio") || rel.ends_with("dirty_background_ratio") {
            return (Some(0), Some(100));
        }
        if rel.ends_with("tcp_fastopen") {
            return (Some(0), Some(3));
        }
        if rel.ends_with("minfree") {
            let pages = current
                .as_deref()
                .and_then(|s| s.split(',').next())
                .and_then(|x| x.trim().parse::<i64>().ok())
                .unwrap_or(0);
            return (Some(0), Some((pages * 2).max(1)));
        }
        if rel.ends_with("disksize") {
            let total_kb = read_trim(Path::new("/proc/meminfo"))
                .and_then(|s| {
                    s.lines()
                        .find(|l| l.starts_with("MemTotal:"))
                        .and_then(|l| l.split_whitespace().nth(1))
                        .and_then(|v| v.parse::<i64>().ok())
                })
                .unwrap_or(0);
            return (Some(0), Some(total_kb * 2));
        }
        let _ = num;
        (None, None)
    }
}

/// Lee y recorta un valor de sysfs.
pub fn read_trim(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Convierte el estado de un nodo de texto a la cifra que se escribe.
///
/// Los modos de este nodo son frases ("Default(Normal) mode", "Low Power mode",
/// "Just Make mode", "Performance(Sports) mode") y no llevan ninguna cifra, asi
/// que la correspondencia se hace por el nombre. Si el kernel cambia el
/// formato, cae al primer digito encontrado.
pub fn power_mode_value(s: &str) -> Option<String> {
    if let Some(d) = s.chars().find(|c| c.is_ascii_digit()) {
        return Some(d.to_string());
    }
    let l = s.to_ascii_lowercase();
    let m = if l.contains("default") || l.contains("normal") {
        "0"
    } else if l.contains("low") {
        "1"
    } else if l.contains("just") || l.contains("make") {
        "2"
    } else if l.contains("perf") || l.contains("sport") {
        "3"
    } else {
        return None;
    };
    Some(m.to_string())
}

/// Cierto si la lectura `back` corresponde al modo `value` ("0".."3").
pub fn power_mode_holds(back: &str, value: &str) -> bool {
    match power_mode_value(back).as_deref() {
        Some(v) => v == value,
        // Sin conversion posible: vale la comparacion por texto directo.
        None => back.trim() == value.trim(),
    }
}

/// Frecuencias que el kernel acepta en un nodo `*_available_frequencies`.
pub fn parse_freqs(s: &str) -> Vec<u64> {
    s.split_whitespace()
        .filter_map(|x| x.trim().parse::<u64>().ok())
        .collect()
}

/// Valor de la tabla `available` mas cercano al objetivo.
///
/// Los kernels solo aceptan frecuencias de su tabla: pedir un valor intermedio
/// puede acabar en un redondeo silencioso (el nodo queda en otra frecuencia de
/// la que se pidio) y la verificacion por relectura delata un valor que no es
/// el deseado. Mejor elegir el disponible mas proximo.
pub fn nearest_freq(available: &str, target: u64) -> Option<u64> {
    parse_freqs(available)
        .into_iter()
        .min_by_key(|&f| (f as i64 - target as i64).unsigned_abs())
}

/// Frecuencia valida mas cercana al porcentaje pedido del maximo del cluster.
/// `avail` es el contenido de `scaling_available_frequencies` y `max` el de
/// `cpuinfo_max_freq`. Devuelve `None` si no hay tabla disponible.
pub fn freq_at_percent(avail: &str, max: u64, pct: f32) -> Option<u64> {
    if avail.trim().is_empty() {
        return None;
    }
    let target = (max as f64 * pct.clamp(0.0, 1.0) as f64).round() as u64;
    nearest_freq(avail, target)
}

/// Da permisos de escritura a un nodo. Algunos kernels traen `scaling_*` en
/// modo solo lectura; sin esto el write falla con EACCES.
pub fn unlock(path: &Path) {
    set_mode(path, 0o644);
}

/// Bloquea el nodo en modo de solo lectura.
///
/// Patron de aZenith: en el perfil de rendimiento el techo de frecuencia se
/// deja **fijo** para que el daemon de potencia del vendor no lo revierta; en
/// los perfiles de ahorro se vuelve a desbloquear.
fn lock(path: &Path) {
    set_mode(path, 0o444);
}

fn set_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path) {
            let mut p = meta.permissions();
            p.set_mode(mode);
            let _ = fs::set_permissions(path, p);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
}

/// Escribe en sysfs y verifica con relectura que el valor se sostuvo.
/// Algunos kernels confirman la escritura de forma asincrona, asi que la
/// relectura se reintenta unos milisegundos antes de declarar fallo.
/// Devuelve false si el valor no se aplico (el llamador debe hacer rollback).
pub fn write_verify(path: &Path, value: &str) -> bool {
    if !is_writable(path) {
        return false;
    }
    // OJO: no usar File::create (O_CREAT|O_TRUNC) en sysfs; con write(true) basta.
    let mut f = match fs::OpenOptions::new().write(true).open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut buf = String::with_capacity(value.len() + 1);
    buf.push_str(value);
    buf.push('\n'); // sysfs espera una unica escritura terminada en newline
    if f.write_all(buf.as_bytes()).is_err() {
        return false;
    }
    drop(f);

    // Exigir que el valor se SOSTENGA ~1,2 s: algunos dispositivos (p. ej. con
    // daemon de potencia del vendor) aceptan la escritura y la revertiran luego.
    let mut stable_since: Option<std::time::Instant> = None;
    for _ in 0..50 {
        match read_trim(path) {
            Some(back) if holds(&back, value) => {
                let t = *stable_since.get_or_insert_with(std::time::Instant::now);
                if t.elapsed() >= std::time::Duration::from_millis(1200) {
                    return true;
                }
            }
            _ => stable_since = None,
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    false
}

/// Write verificado con bloqueo posterior del nodo (`pin`).
///
/// Se Desbloquea antes de escribir (un nodo fijado en 444 no aceptaria el
/// write) y, si `pin` y la verificacion fue buena, se vuelve a fijar.
pub fn write_verify_pin(path: &Path, value: &str, pin: bool) -> bool {
    unlock(path);
    let ok = write_verify(path, value);
    if ok && pin {
        lock(path);
    }
    ok
}

/// Un nodo tipo enum ("noop deadline [cfq]") se considera aplicado si el valor
/// queda dentro de corchetes; el resto se compara normalizado.
fn holds(back: &str, value: &str) -> bool {
    let b = back.trim();
    let v = value.trim();
    if b.contains(&format!("[{}]", v)) {
        return true;
    }
    if normalize(b) == normalize(v) {
        return true;
    }
    // Hay nodos que se escriben con una cifra pero contestan con una frase
    // ("0" -> "Default(Normal) mode"). Se acepta si la frase describe el modo
    // pedido, o si el texto empieza por la cifra. En un nodo numerico manda la
    // comparacion normal, para no dar por bueno "1" cuando el nodo dice
    // "1000000".
    let respuesta_con_texto = b.chars().any(|c| !c.is_ascii_digit() && !c.is_whitespace());
    if respuesta_con_texto && v.len() == 1 && v.chars().all(|c| c.is_ascii_digit()) {
        return power_mode_holds(b, v);
    }
    false
}

/// Valor activo de un nodo enum: lo que esta entre corchetes, o el texto completo.
pub fn active_token(s: &str) -> String {
    if let Some(i) = s.find('[') {
        if let Some(j) = s[i + 1..].find(']') {
            return s[i + 1..i + 1 + j].to_string();
        }
    }
    s.trim().to_string()
}

/// Writability conservadora: existe, es archivo y su modo tiene bits de escritura.
pub fn is_writable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match fs::metadata(path) {
            Ok(m) => m.permissions().mode() & 0o222 != 0,
            Err(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn parse_enums(s: &str) -> Vec<String> {
    // Formato tipico: "schedutil performance powersave" o "[schedutil] powersave".
    // Sin duplicados: algunos kernels repiten opciones y la UI las mostraba
    // en filas repetidas.
    let mut v: Vec<String> = Vec::new();
    for tok in s.split_whitespace() {
        let t = tok.trim_matches(['[', ']']);
        if !t.is_empty() && !v.iter().any(|x| x == t) {
            v.push(t.to_string());
        }
    }
    v
}

fn normalize(s: &str) -> String {
    s.trim().trim_matches(['[', ']']).to_string()
}
