# Ronda 2: bloqueo manual con IA activa, interruptor maestro, energia, autonomia.
import json, subprocess, time, os, re, urllib.request, xml.etree.ElementTree as ET

ADB = os.path.join(os.environ["LOCALAPPDATA"], "Android", "Sdk", "platform-tools", "adb.exe")
BASE = "http://127.0.0.1:18080"
EV = open(os.path.join(os.environ["TMPDIR"], "evidence-r2.txt"), "w", encoding="utf-8")

def log(*a):
    s = " ".join(str(x) for x in a)
    print(s); EV.write(s + "\n"); EV.flush()

def adb(*args, timeout=40):
    return subprocess.run([ADB] + list(args), capture_output=True, text=True, timeout=timeout).stdout

def req(method, path, body=None, timeout=60):
    r = urllib.request.Request(BASE + path, method=method,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(r, timeout=timeout) as resp:
        return json.loads(resp.read().decode())

def sh(cmd):
    return adb("shell", "su", "-c", cmd).strip()

def dump():
    adb("shell", "uiautomator", "dump", "/sdcard/ui.xml")
    ns = []
    try:
        root = ET.fromstring(adb("shell", "cat", "/sdcard/ui.xml"))
        for n in root.iter("node"):
            b = re.findall(r"\[(\d+),(\d+)\]", n.get("bounds", ""))
            if len(b) == 2:
                ns.append({"text": n.get("text", ""), "id": n.get("resource-id", ""),
                           "cls": n.get("class", ""), "enabled": n.get("enabled") == "true",
                           "cx": (int(b[0][0])+int(b[1][0]))//2, "cy": (int(b[0][1])+int(b[1][1]))//2,
                           "y1": int(b[0][1])})
    except Exception as e:
        log("dump error:", e)
    return ns

def texts(ns): return [n["text"] for n in ns if n["text"]]
def find(ns, text=None, id=None, cls=None):
    for n in ns:
        if text is not None and n["text"] == text: return n
        if id is not None and n["id"] == id: return n
        if cls is not None and n["cls"] == cls: return n
    return None
def tap(n): adb("shell", "input", "tap", str(n["cx"]), str(n["cy"]))

SDA = "/sys/block/sda/queue/scheduler"
P0 = "/sys/devices/system/cpu/cpufreq/policy0"

log("=== 1. ESTADO INICIAL ===")
st = req("GET", "/api/state")
log("ai:", st["ai"], "auto:", st["auto"], "profile:", st["profile"], "version:", st["version"])

log("=== 2. ESCRITURAS MANUALES BLOQUEADAS CON IA ACTIVA ===")
blocked_tests = [
    ("/api/knob", {"path": SDA, "value": "cfq"}),
    ("/api/profile", {"profile": "eco"}),
    ("/api/freq-cap", {"value": 0.6}),
    ("/api/gaming", {"enabled": True}),
    ("/api/bypass-set", {"enable": True}),
    ("/api/app-settings", {"package": "com.iaopt.test", "key": "mode", "value": "manual"}),
    ("/api/app-apply", {"package": "com.iaopt.test"}),
    ("/api/quick", {"action": "wifi", "value": "1"}),
]
for path, body in blocked_tests:
    r = req("POST", path, body)
    log(path, "->", "BLOQUEADO" if r.get("blocked") else "NO BLOQUEADO!", r.get("detail", "")[:60])

log("=== 3. EXENTOS: pantalla y marcar juego ===")
r = req("POST", "/api/app-settings", {"package": "com.iaopt.test", "key": "is_game", "value": "true"})
log("is_game ->", r)
r = req("POST", "/api/refresh", {"value": 60})
log("refresh 60 ->", r)
time.sleep(1)
r = req("POST", "/api/refresh", {"value": 0})
log("refresh auto ->", r)
rd = req("GET", "/api/renderer")
cur = rd.get("current", "")
r = req("POST", "/api/renderer", {"value": cur})
log("renderer (mismo valor) ->", r.get("ok"), r.get("detail", ""))

log("=== 4. UI: INICIO BLOQUEADO ===")
adb("shell", "input", "keyevent", "KEYCODE_WAKEUP")
adb("shell", "am", "start", "-n", "io.teyzen.iaoptimizer/.MainActivity")
time.sleep(3)
ns = dump()
ts = texts(ns)
log("switch IA Motors:", bool(find(ns, text="IA Motors")))
log("switch Automático fuera:", find(ns, text="Automático") is None)
log("hint bloqueo:", any("gestionando el sistema" in t for t in ts))
pb = find(ns, id="io.teyzen.iaoptimizer:id/profile_buttons")
fc = find(ns, id="io.teyzen.iaoptimizer:id/freqCapSlider")
log("profile_buttons enabled:", pb["enabled"] if pb else None, "(esperado False)")
log("freqCapSlider enabled:", fc["enabled"] if fc else None, "(esperado False)")

log("=== 5. UI: CONTROLES BLOQUEADOS ===")
nm = find(ns, id="io.teyzen.iaoptimizer:id/nav_motors")
tap(nm); time.sleep(2)
ns = dump()
ct = find(ns, text="Controles")
if ct: tap(ct); time.sleep(2)
ns = dump()
ts = texts(ns)
log("nota Modo autónomo:", any("Modo autónomo" in t for t in ts))
sliders = [n for n in ns if n["cls"].endswith("Slider")]
log("sliders visibles:", len(sliders), "| todos disabled:", all(not s["enabled"] for s in sliders) if sliders else "n/a")
btns = [n for n in ns if n["cls"].endswith("MaterialButton") and n["text"] in ("Sin reordenar", "Plazo fijo", "CFQ", "Adaptativo", "Rendimiento máximo", "Ahorro de energía")]
log("botones enum visibles:", len(btns), "| todos disabled:", all(not b["enabled"] for b in btns) if btns else "n/a")

log("=== 6. UI: ENERGIA AMIGABLE ===")
en = find(ns, text="Energía")
if en: tap(en); time.sleep(2)
ns = dump()
ts = texts(ns)
log("Carga directa:", any("Carga directa" in t for t in ts))
log("Nodo elegido fuera:", not any("Nodo elegido" in t for t in ts))
log("descripción amigable:", any("menos calor" in t for t in ts))
log("textos energía:", [t for t in ts if t not in ("Resumen","Diario","Aprendizaje","Controles","Energía","Sistema","Pantalla","IA Motors")][:16])

log("=== 7. UI: PANTALLA SIGUE CONTROLABLE ===")
tab = find(ns, text="Energía")
y = tab["cy"] if tab else 260
for i in range(3):
    adb("shell", "input", "swipe", "950", str(y), "150", str(y), "200"); time.sleep(0.6)
    ns = dump()
    if find(ns, text="Pantalla"): break
pt = find(ns, text="Pantalla")
if pt:
    tap(pt); time.sleep(2)
    ns = dump()
    ts = texts(ns)
    log("Frecuencia de actualización:", any("Frecuencia de actualización" in t for t in ts))
    rv = find(ns, id="io.teyzen.iaoptimizer:id/refreshValue")
    rdv = find(ns, id="io.teyzen.iaoptimizer:id/rendererValue")
    log("refreshValue enabled:", rv["enabled"] if rv else None, "(esperado True)")
    log("rendererValue enabled:", rdv["enabled"] if rdv else None, "(esperado True)")

log("=== 8. IA OFF -> CONTROL MANUAL DEVUELTO ===")
r = req("POST", "/api/ai/toggle", {"enabled": False})
log("toggle off:", r)
st = req("GET", "/api/state")
log("ai:", st["ai"], "auto:", st["auto"])
r = req("POST", "/api/profile", {"profile": "eco"})
log("profile eco manual ->", r.get("ok"), "applied:", r.get("applied"))
r = req("POST", "/api/knob", {"path": SDA, "value": "cfq"})
log("knob sda cfq manual ->", r.get("ok"), r.get("note"))
r = req("POST", "/api/freq-cap", {"value": 0.6})
log("freq-cap 0.6 manual ->", r)
time.sleep(3)
adb("shell", "am", "start", "-n", "io.teyzen.iaoptimizer/.MainActivity")
tap(find(dump(), id="io.teyzen.iaoptimizer:id/nav_inicio")); time.sleep(3)
ns = dump()
pb = find(ns, id="io.teyzen.iaoptimizer:id/profile_buttons")
fc = find(ns, id="io.teyzen.iaoptimizer:id/freqCapSlider")
log("UI manual: profile_buttons enabled:", pb["enabled"] if pb else None, "| freqCapSlider enabled:", fc["enabled"] if fc else None)

log("=== 9. IA ON -> RESET AUTONOMO ===")
r = req("POST", "/api/ai/toggle", {"enabled": True})
log("toggle on:", json.dumps(r)[:200])
st = req("GET", "/api/state")
log("profile:", st["profile"], "(esperado balanced) cap:", st["freq_cap"], "(esperado 1.0)")
log("sda:", sh(f"cat {SDA}"), "(esperado [deadline])")
log("snapshot:", sh("cat /data/adb/ia_optimizer/manual_snapshot.json")[:160])

log("=== 10. AUTONOMIA BAJO CARGA REAL ===")
log("antes:", req("GET", "/api/state")["profile"], "| carga:", req("GET", "/api/telemetry")["cpu"])
load = subprocess.Popen([ADB, "shell", "su", "-c",
    "timeout 30 sh -c 'yes >/dev/null & yes >/dev/null & yes >/dev/null & yes >/dev/null & wait'"],
    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
seen = set()
for i in range(8):
    time.sleep(5)
    st = req("GET", "/api/state")
    ag = req("GET", "/api/agent")
    tel = req("GET", "/api/telemetry")
    seen.add(st["profile"])
    log(f"t+{(i+1)*5}s: profile={st['profile']} cpu={tel['cpu']['cur_mhz']}MHz fase={ag.get('phase')} act={str(ag.get('current_activity'))[:70]}")
load.wait()
log("perfiles vistos bajo carga:", seen)
time.sleep(15)
st = req("GET", "/api/state")
log("tras la carga (reposo):", st["profile"], "| sugerencia:", st["suggestion"])

log("=== 11. ESTADO FINAL ===")
st = req("GET", "/api/state")
log("ai:", st["ai"], "auto:", st["auto"], "profile:", st["profile"], "cap:", st["freq_cap"])
log("sda:", sh(f"cat {SDA}"), "| max0:", sh(f"cat {P0}/scaling_max_freq"), "| min0:", sh(f"cat {P0}/scaling_min_freq"))
crash = adb("logcat", "-b", "crash", "-d")
log("crashes:", len([l for l in crash.splitlines() if "iaoptimizer" in l]))
EV.close()
