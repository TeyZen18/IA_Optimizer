# Ronda 2b: histeresis termica bajo carga, UI bloqueada (scroll), monkey.
import json, subprocess, time, os, re, urllib.request, xml.etree.ElementTree as ET

ADB = os.path.join(os.environ["LOCALAPPDATA"], "Android", "Sdk", "platform-tools", "adb.exe")
BASE = "http://127.0.0.1:18080"
EV = open(os.path.join(os.environ["TMPDIR"], "evidence-r2b.txt"), "w", encoding="utf-8")

def log(*a):
    s = " ".join(str(x) for x in a)
    print(s); EV.write(s + "\n"); EV.flush()

def adb(*args, timeout=60):
    return subprocess.run([ADB] + list(args), capture_output=True, text=True, timeout=timeout).stdout

def req(method, path, body=None, timeout=60):
    r = urllib.request.Request(BASE + path, method=method,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(r, timeout=timeout) as resp:
        return json.loads(resp.read().decode())

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
                           "cx": (int(b[0][0])+int(b[1][0]))//2, "cy": (int(b[0][1])+int(b[1][1]))//2})
    except Exception as e:
        log("dump error:", e)
    return ns

def find(ns, text=None, id=None):
    for n in ns:
        if text is not None and n["text"] == text: return n
        if id is not None and n["id"] == id: return n
    return None
def tap(n): adb("shell", "input", "tap", str(n["cx"]), str(n["cy"]))

log("=== 1. AUTONOMIA CON HISTERESIS (carga 25 s) ===")
st0 = req("GET", "/api/state")
log("antes:", st0["profile"])
# carga auto-terminada: el propio shell mata los yes a los 25 s (sin hangs)
adb("shell", "su", "-c",
    "nohup sh -c 'yes >/dev/null & yes >/dev/null & yes >/dev/null & yes >/dev/null & sleep 25; pkill -x yes' >/dev/null 2>&1 &")
profiles = []
for i in range(7):
    time.sleep(5)
    st = req("GET", "/api/state")
    ag = req("GET", "/api/agent")
    th = req("GET", "/api/thermal")
    profiles.append(st["profile"])
    log(f"t+{(i+1)*5}s: profile={st['profile']} soc={th.get('soc_c')} fase={ag.get('phase')} act={str(ag.get('current_activity'))[:64]}")
log("secuencia de perfiles:", profiles)
cambios = sum(1 for a, b in zip(profiles, profiles[1:]) if a != b)
log("cambios de perfil durante la carga:", cambios, "(esperado <=2 con histeresis)")
time.sleep(20)
st = req("GET", "/api/state")
log("reposo tras enfriar:", st["profile"], "|", st["suggestion"]["reason"])

log("=== 2. UI INICIO: slider y hint bloqueados (scroll) ===")
adb("shell", "input", "keyevent", "KEYCODE_WAKEUP")
adb("shell", "am", "start", "-n", "io.teyzen.iaoptimizer/.MainActivity")
time.sleep(3)
ns = dump()
ni = find(ns, id="io.teyzen.iaoptimizer:id/nav_inicio")
tap(ni); time.sleep(1.5)
for i in range(3):
    ns = dump()
    fc = find(ns, id="io.teyzen.iaoptimizer:id/freqCapSlider")
    if fc: break
    adb("shell", "input", "swipe", "540", "1800", "540", "800", "250"); time.sleep(0.7)
log("freqCapSlider enabled:", fc["enabled"] if fc else None, "(esperado False)")
ts = [n["text"] for n in ns if n["text"]]
log("hint bloqueo visible:", any("gestionando el sistema" in t for t in ts))
pb = find(ns, id="io.teyzen.iaoptimizer:id/profile_buttons")
log("profile_buttons enabled:", pb["enabled"] if pb else None, "(esperado False)")

log("=== 3. UI CONTROLES: sliders disabled (scroll) ===")
nm = find(ns, id="io.teyzen.iaoptimizer:id/nav_motors")
tap(nm); time.sleep(2)
ns = dump()
ct = find(ns, text="Controles")
if ct: tap(ct); time.sleep(2)
found = []
for i in range(5):
    ns = dump()
    sl = [n for n in ns if n["cls"].endswith("Slider")]
    bt = [n for n in ns if n["cls"].endswith("MaterialButton") and n["text"] in ("Sin reordenar","Plazo fijo","CFQ","Adaptativo","Rendimiento máximo","Ahorro de energía","Normal","Modo juego")]
    found += sl + bt
    if sl or bt: break
    adb("shell", "input", "swipe", "540", "1800", "540", "700", "250"); time.sleep(0.7)
log("controles interactivos visibles:", len(found), "| todos disabled:", all(not n["enabled"] for n in found) if found else "n/a")
for n in found[:6]: log("  ", n["cls"].split(".")[-1], n["text"][:30], "enabled=", n["enabled"])

log("=== 4. MONKEY: 800 eventos aleatorios ===")
adb("logcat", "-b", "crash", "-c")
out = adb("shell", "monkey", "-p", "io.teyzen.iaoptimizer", "--throttle", "150", "-v", "800", timeout=420)
tail = out.strip().splitlines()[-6:]
for l in tail: log("monkey:", l)
crash = adb("logcat", "-b", "crash", "-d")
hits = [l for l in crash.splitlines() if "iaoptimizer" in l and "Process:" in l]
log("crashes tras monkey:", len(hits))
for l in hits[:4]: log("  ", l)
anr = adb("shell", "dumpsys", "activity", "processes")
log("ANR entries:", anr.count("ANR in io.teyzen.iaoptimizer"))

log("=== 5. ESTADO FINAL ===")
st = req("GET", "/api/state")
log("ai:", st["ai"], "auto:", st["auto"], "profile:", st["profile"], "cap:", st["freq_cap"], "version:", st["version"])
log("yes residuales:", adb("shell", "su", "-c", "pidof yes || echo ninguno").strip())
EV.close()
