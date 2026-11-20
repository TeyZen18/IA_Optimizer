//! Telemetria ampliada para la Cubierta de mando: frecuencias, nucleos, memoria,
//! zram, almacenamiento, red, refresco, voltaje, zonas termicas y procesos.

use std::fs;
use std::time::{Duration, Instant};

#[derive(Default, Clone)]
pub struct Proc {
    pub pid: u32,
    pub name: String,
    pub pkg: String,
    pub cpu: f32,
    pub rss_mb: f32,
}

#[derive(Default, Clone)]
pub struct Cluster {
    pub policy: String,
    pub min_mhz: u64,
    pub max_mhz: u64,
    pub cur_mhz: u64,
}

#[derive(Default, Clone)]
pub struct Extra {
    pub cpu_min_mhz: Option<u64>,
    pub cpu_max_mhz: Option<u64>,
    pub cpu_cur_mhz: Option<u64>,
    pub cpu_clusters: Vec<Cluster>,
    pub cores_online: Option<String>,
    pub mem_total_mb: u64,
    pub mem_avail_mb: u64,
    pub mem_pressure: Option<f32>,
    pub swap_total_mb: u64,
    pub swap_free_mb: u64,
    pub zram_total_mb: u64,
    pub zram_used_mb: u64,
    pub storage_total_mb: u64,
    pub storage_used_mb: u64,
    pub rx_mbps: Option<f32>,
    pub tx_mbps: Option<f32>,
    pub refresh_hz: Option<f32>,
    pub voltage_uv: Option<u64>,
    /// Corriente de bateria en mA (positiva = cargando, negativa = en uso).
    pub bat_current_ma: Option<i32>,
    pub bat_temp_c: Option<f32>,
    pub bat_pct: Option<u32>,
    pub charging: Option<bool>,
    /// Frecuencia GPU en MHz (nodo del vendor; None si no se expone).
    pub gpu_mhz: Option<u64>,
    /// Utilización GPU 0-100 si el driver la expone.
    pub gpu_util: Option<u32>,
    pub thermal_zones: Vec<(String, String, i32)>,
    pub device: String,
    pub soc: String,
    pub cpu_model: String,
    pub processes: Vec<Proc>,
}

fn read_trim(p: &str) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

/// Frecuencia GPU en MHz. Adreno/devfreq/Mali sysfs exponen un número
/// directo; MTK (/proc/mali) expone un ÍNDICE de OPP ("GPU Frequency
/// Index: N") cuya tabla idx→kHz está en /proc/gpufreq/gpufreq_opp_dump.
/// Legible desde el contexto del daemon, no desde el shell adb. Las
/// unidades varían por driver (Hz/kHz/MHz): se normalizan por magnitud.
fn gpu_freq_mhz() -> Option<u64> {
    fn norm(v: u64) -> Option<u64> {
        let mhz = if v >= 100_000_000 {
            v / 1_000_000
        } else if v >= 100_000 {
            v / 1_000
        } else {
            v
        };
        if mhz > 0 && mhz < 5000 { Some(mhz) } else { None }
    }
    let candidates = [
        "/sys/class/kgsl/kgsl-3d0/gpuclk",
        "/sys/class/kgsl/kgsl-3d0/devfreq/cur_freq",
        "/sys/class/devfreq/gpu/cur_freq",
        "/sys/class/misc/mali0/device/cur_freq",
        "/sys/class/misc/mali0/device/dvfs_cur_freq",
    ];
    for p in candidates {
        if let Some(v) = read_trim(p)
            .and_then(|s| s.split_whitespace().next().map(|t| t.to_string()))
            .and_then(|s| s.parse::<u64>().ok())
        {
            if let Some(mhz) = norm(v) {
                return Some(mhz);
            }
        }
    }
    // MTK: índice de OPP + tabla del driver gpufreq.
    let idx = read_trim("/proc/mali/frequency")
        .and_then(|s| s.rsplit(':').next().map(|t| t.trim().to_string()))
        .and_then(|s| s.parse::<usize>().ok());
    if let Some(idx) = idx {
        if let Some(table) = read_trim("/proc/gpufreq/gpufreq_opp_dump") {
            let tag = format!("[{:02}]", idx);
            for line in table.lines() {
                if let Some(rest) = line.trim().strip_prefix(&tag) {
                    if let Some(f) = rest.split("freq = ").nth(1) {
                        let khz: String = f.chars().take_while(|c| c.is_ascii_digit()).collect();
                        if let Ok(v) = khz.parse::<u64>() {
                            if let Some(mhz) = norm(v) {
                                return Some(mhz);
                            }
                        }
                    }
                }
            }
        }
    }
    if let Ok(rd) = fs::read_dir("/sys/class/devfreq") {
        for ent in rd.flatten() {
            let name = ent.file_name().to_string_lossy().to_string();
            if name.contains("gpu") || name.contains("mali") {
                if let Some(v) = read_trim(&format!("/sys/class/devfreq/{}/cur_freq", name))
                    .and_then(|s| s.parse::<u64>().ok())
                {
                    if let Some(mhz) = norm(v) {
                        return Some(mhz);
                    }
                }
            }
        }
    }
    None
}

/// Utilización GPU 0-100. MTK la publica como
/// "gpu/cljs0/cljs1=72/0/0, frequency index=..."; otros drivers, un número.
fn gpu_util_pct() -> Option<u32> {
    let s = read_trim("/proc/mali/utilization")?;
    let v = if let Some(rest) = s.split("gpu/cljs0/cljs1=").nth(1) {
        rest.split('/').next()?.trim().parse::<u32>().ok()?
    } else {
        s.split_whitespace().next()?.parse::<u32>().ok()?
    };
    if v <= 100 { Some(v) } else { None }
}

fn kb_field(l: &str) -> Option<u64> {
    let mut it = l.split_whitespace();
    let _key = it.next()?;
    it.next()?.parse::<u64>().ok()
}

fn mhz(p: &str) -> Option<u64> {
    read_trim(p)?.parse::<u64>().ok().map(|k| k / 1000)
}

pub fn sample(sample_net: bool) -> Extra {
    let mut e = Extra::default();

    // CPU: frecuencias y nucleos. Se leen TODOS los nucleos activos y se
    // reporta la frecuencia mas alta en ese instante (no un valor fijo).
    let mut cur_max: u64 = 0;
    let mut cur_min: u64 = u64::MAX;
    let mut seen_core = false;
    if let Ok(rd) = fs::read_dir("/sys/devices/system/cpu") {
        let mut cores: Vec<String> = rd
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| {
                n.starts_with("cpu") && n.len() > 3 && n[3..].chars().all(|c| c.is_ascii_digit())
            })
            .collect();
        cores.sort();
        for c in cores {
            let base = format!("/sys/devices/system/cpu/{}/cpufreq", c);
            if fs::metadata(&base).is_err() {
                continue;
            }
            if let Some(v) = mhz(&format!("{}/scaling_cur_freq", base)) {
                seen_core = true;
                cur_max = cur_max.max(v);
                cur_min = cur_min.min(v);
            }
            if e.cpu_min_mhz.is_none() {
                e.cpu_min_mhz = mhz(&format!("{}/cpuinfo_min_freq", base));
            }
            if e.cpu_max_mhz.is_none() {
                e.cpu_max_mhz = mhz(&format!("{}/cpuinfo_max_freq", base));
            }
        }
    }
    // Por si el listado de nucleos no expone cpufreq, probar las politicas.
    for i in 0..16 {
        let base = format!("/sys/devices/system/cpu/cpufreq/policy{}", i);
        if fs::metadata(&base).is_err() {
            continue;
        }
        let mn = mhz(&format!("{}/cpuinfo_min_freq", base));
        let mx = mhz(&format!("{}/cpuinfo_max_freq", base));
        let cur = mhz(&format!("{}/scaling_cur_freq", base));
        if let Some(v) = cur {
            seen_core = true;
            cur_max = cur_max.max(v);
            cur_min = cur_min.min(v);
        }
        e.cpu_clusters.push(Cluster {
            policy: format!("policy{}", i),
            min_mhz: mn.unwrap_or(0),
            max_mhz: mx.unwrap_or(0),
            cur_mhz: cur.unwrap_or(0),
        });
        if e.cpu_min_mhz.is_none() {
            e.cpu_min_mhz = mn;
        }
        if e.cpu_max_mhz.is_none() {
            e.cpu_max_mhz = mx;
        }
    }
    if seen_core {
        e.cpu_cur_mhz = Some(cur_max);
        e.cpu_min_mhz = Some(e.cpu_min_mhz.unwrap_or(cur_min));
    }
    // Núcleos activos como "N/M" (activos / posibles): el rango crudo "0-7"
    // no dice cuántos hay. Se expanden los rangos de cpu/online y cpu/possible.
    e.cores_online = cores_ratio();

    // Memoria
    if let Some(mi) = read_trim("/proc/meminfo") {
        for l in mi.lines() {
            let key = l.split(':').next().unwrap_or("").trim();
            let v = match kb_field(l) {
                Some(v) => v,
                None => continue,
            };
            match key {
                "MemTotal" => e.mem_total_mb = v / 1024,
                "MemAvailable" => e.mem_avail_mb = v / 1024,
                "SwapTotal" => e.swap_total_mb = v / 1024,
                "SwapFree" => e.swap_free_mb = v / 1024,
                _ => {}
            }
        }
    }
    // Presion de memoria (PSI) si el kernel lo expone
    if let Some(psi) = read_trim("/proc/pressure/memory") {
        if let Some(seg) = psi.lines().next() {
            for tok in seg.split_whitespace() {
                if let Some(v) = tok.strip_prefix("avg10=") {
                    e.mem_pressure = v.parse().ok();
                }
            }
        }
    }

    // ZRAM
    e.zram_total_mb = read_trim("/sys/block/zram0/disksize")
        .and_then(|s| s.parse::<u64>().ok())
        .map(|b| b / 1024 / 1024)
        .unwrap_or(0);
    if let Some(mm) = read_trim("/sys/block/zram0/mm_stat") {
        if let Some(first) = mm.split_whitespace().next() {
            e.zram_used_mb = first.parse::<u64>().map(|b| b / 1024 / 1024).unwrap_or(0);
        }
    }

    // Almacenamiento (via df; sin dependencias)
    if let Ok(out) = std::process::Command::new("df").arg("-k").arg("/data").output() {
        let s = String::from_utf8_lossy(&out.stdout);
        if let Some(l) = s.lines().nth(1) {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() >= 3 {
                e.storage_total_mb = f[1].parse().unwrap_or(0) / 1024;
                e.storage_used_mb = f[2].parse().unwrap_or(0) / 1024;
            }
        }
    }

    // Red: velocidad de descarga/subida (delta de 1 s si se pide)
    if sample_net {
        let a = net_bytes();
        let t0 = Instant::now();
        std::thread::sleep(Duration::from_secs(1));
        let b = net_bytes();
        let secs = t0.elapsed().as_secs_f32().max(0.2);
        if let (Some(a), Some(b)) = (a, b) {
            let d = (b.0.saturating_sub(a.0)) as f32 * 8.0 / secs / 1_000_000.0;
            let u = (b.1.saturating_sub(a.1)) as f32 * 8.0 / secs / 1_000_000.0;
            e.rx_mbps = Some(d);
            e.tx_mbps = Some(u);
        }
    }

    // Tasa de refresco y voltaje (dumpsys / power_supply)
    if let Ok(out) = std::process::Command::new("sh")
        .arg("-c")
        .arg("dumpsys display 2>/dev/null | grep -o -m1 'renderFrameRate [0-9.]*'")
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout);
        if let Some(last) = s.split_whitespace().last() {
            e.refresh_hz = last.parse().ok();
        }
    }
    if e.refresh_hz.is_none() {
        if let Ok(out) = std::process::Command::new("settings")
            .args(["get", "system", "peak_refresh_rate"])
            .output()
        {
            let s = String::from_utf8_lossy(&out.stdout);
            e.refresh_hz = s.trim().parse().ok();
        }
    }
    e.voltage_uv = read_trim("/sys/class/power_supply/battery/voltage_now")
        .and_then(|s| s.parse().ok());
    // current_now viene en uA; algunos PMIC lo exponen ya en mA.
    e.bat_current_ma = read_trim("/sys/class/power_supply/battery/current_now")
        .and_then(|s| s.parse::<i64>().ok())
        .map(|v| if v.abs() >= 10_000 { (v / 1000) as i32 } else { v as i32 });
    e.bat_temp_c = read_trim("/sys/class/power_supply/battery/temp")
        .and_then(|s| s.parse::<i32>().ok())
        .map(|t| if t.abs() >= 1000 { t as f32 / 1000.0 } else { t as f32 / 10.0 });
    e.bat_pct = read_trim("/sys/class/power_supply/battery/capacity")
        .and_then(|s| s.parse().ok());
    e.charging = read_trim("/sys/class/power_supply/battery/status").map(|s| {
        s.eq_ignore_ascii_case("charging") || s.eq_ignore_ascii_case("full")
    });
    e.gpu_mhz = gpu_freq_mhz();
    e.gpu_util = gpu_util_pct();

    // Zonas termicas reales
    if let Ok(rd) = fs::read_dir("/sys/class/thermal") {
        for ent in rd.flatten() {
            let name = ent.file_name().to_string_lossy().to_string();
            if !name.starts_with("thermal_zone") {
                continue;
            }
            let base = format!("/sys/class/thermal/{}", name);
            let ztype = read_trim(&format!("{}/type", base)).unwrap_or_default();
            let temp = read_trim(&format!("{}/temp", base))
                .and_then(|s| s.parse::<i32>().ok())
                .map(|t| if t.abs() >= 1000 { t / 1000 } else { t / 10 })
                .unwrap_or(0);
            e.thermal_zones.push((name, ztype, temp));
        }
    }
    e.thermal_zones.sort_by(|a, b| a.0.cmp(&b.0));

    // Identidad del dispositivo / SoC / procesador
    if let Ok(out) = std::process::Command::new("sh")
        .arg("-c")
        .arg("getprop ro.product.model; getprop ro.soc.model; getprop ro.board.platform")
        .output()
    {
        let lines: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        e.device = lines.first().cloned().unwrap_or_default();
        e.soc = lines.get(1).cloned().unwrap_or_default();
        if e.soc.is_empty() {
            e.soc = lines.get(2).cloned().unwrap_or_default();
        }
    }
    if let Some(ci) = read_trim("/proc/cpuinfo") {
        for l in ci.lines() {
            let low = l.to_ascii_lowercase();
            if !(low.starts_with("hardware") || low.starts_with("model name")) {
                continue;
            }
            if let Some(v) = l.split(':').nth(1) {
                let v = v.trim();
                if !v.is_empty() && !v.chars().all(|c| c.is_ascii_digit()) {
                    e.cpu_model = v.to_string();
                    break;
                }
            }
        }
    }

    e.processes = Vec::new();
    e
}

fn net_bytes() -> Option<(u64, u64)> {
    let s = read_trim("/proc/net/dev")?;
    let mut rx = 0u64;
    let mut tx = 0u64;
    for l in s.lines() {
        if !l.contains(':') {
            continue;
        }
        let (iface, rest) = l.split_once(':')?;
        let iface = iface.trim();
        if iface == "lo" || iface.starts_with("dummy") || iface.starts_with("tun") {
            continue;
        }
        let f: Vec<&str> = rest.split_whitespace().collect();
        if f.len() >= 10 {
            rx += f[0].parse::<u64>().unwrap_or(0);
            tx += f[8].parse::<u64>().unwrap_or(0);
        }
    }
    Some((rx, tx))
}

/// Top N procesos por CPU (delta de /proc/stat entre dos lecturas de 1 s).
pub fn top_processes(n: usize) -> Vec<Proc> {
    let snap = |total: &mut u64| -> Vec<(u32, u64, u64, String, u64)> {
        let mut v = Vec::new();
        if let Ok(rd) = fs::read_dir("/proc") {
            for ent in rd.flatten() {
                let pid = match ent.file_name().to_string_lossy().parse::<u32>() {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                let stat = match read_trim(&format!("/proc/{}/stat", pid)) {
                    Some(s) => s,
                    None => continue,
                };
                let (l, r) = match stat.rsplit_once(')') {
                    Some(x) => x,
                    None => continue,
                };
                let f: Vec<&str> = r.split_whitespace().collect();
                if f.len() < 12 {
                    continue;
                }
                let utime: u64 = f[11].parse().unwrap_or(0);
                let stime: u64 = f[12].parse().unwrap_or(0);
                let name = l
                    .rsplit_once('(')
                    .map(|(_, c)| c.trim().to_string())
                    .unwrap_or_default();
                let _ = &name;
                let rss_pages: u64 = read_trim(&format!("/proc/{}/statm", pid))
                    .and_then(|s| s.split_whitespace().nth(1).and_then(|x| x.parse().ok()))
                    .unwrap_or(0);
                let _ = total;
                v.push((pid, utime, stime, name, rss_pages));
            }
        }
        v
    };

    let mut total = 0u64;
    let a = snap(&mut total);
    let t0 = Instant::now();
    std::thread::sleep(Duration::from_millis(700));
    let b = snap(&mut total);
    let _ = t0.elapsed();
    let page_kb = 4u64;
    let mut out: Vec<Proc> = b
        .into_iter()
        .filter_map(|(pid, ut, st, name, rss)| {
            let a = a.iter().find(|x| x.0 == pid)?;
            let d = (ut + st).saturating_sub(a.1 + a.2) as f32;
            Some(Proc {
                pid,
                name,
                pkg: String::new(),
                cpu: (d / 100.0 * 14.3).min(999.0), // aproximacion por ticks
                rss_mb: (rss * page_kb) as f32 / 1024.0,
            })
        })
        .collect();
    out.sort_by(|x, y| y.cpu.partial_cmp(&x.cpu).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(n);
    out
}


/// "N/M": núcleos activos sobre posibles, expandiendo rangos ("0-7" -> 8).
fn cores_ratio() -> Option<String> {
    fn count(spec: &str) -> u32 {
        spec.split(',')
            .filter_map(|p| {
                let p = p.trim();
                if let Some((a, b)) = p.split_once('-') {
                    let a: u32 = a.trim().parse().ok()?;
                    let b: u32 = b.trim().parse().ok()?;
                    Some(b.saturating_sub(a) + 1)
                } else {
                    p.parse::<u32>().ok().map(|_| 1)
                }
            })
            .sum()
    }
    let online = read_trim("/sys/devices/system/cpu/online")?;
    let possible = read_trim("/sys/devices/system/cpu/possible")
        .unwrap_or_else(|| online.clone());
    Some(format!("{}/{}", count(&online), count(&possible)))
}
