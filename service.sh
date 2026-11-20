#!/system/bin/sh
# IA Optimizer - service.sh : arranque tardio (modo servicio, root)
MODDIR=$(dirname "$0")
LOG=/data/adb/ia_optimizer/boot.log

if [ -f "$MODDIR/disable" ]; then
  echo "[$(date)] modulo desactivado por seguridad" >> "$LOG"
  exit 0
fi

# Puerto de la WebUI (loopback unicamente; se puede cambiar en config.json)
PORT=8080
if [ -f "$MODDIR/config.json" ]; then
  P=$(grep -o '"port"[[:space:]]*:[[:space:]]*[0-9]*' "$MODDIR/config.json" | grep -o '[0-9]*$')
  [ -n "$P" ] && PORT="$P"
fi

mkdir -p /data/adb/ia_optimizer

# Esperar a que el sistema termine de arrancar: los nodos del kernel todavia
# se estan montando y leerlos antes da valores falsos (patron aZenith).
until [ "$(getprop sys.boot_completed)" = "1" ]; do
  sleep 1
done

# Restaurar el motor de renderizado elegido (si lo hay)
R=$(getprop persist.sys.iaopt.renderer 2>/dev/null)
[ -n "$R" ] && setprop debug.hwui.renderer "$R" 2>/dev/null

# Si ya hay un daemon vivo (proceso "serve"), no duplicar.
# OJO: `pidof iaopt` tambien coincide con el propio CLI (p. ej. `iaopt repair-apply`),
# asi que se comprueba el cmdline en /proc.
PIDF=/data/adb/ia_optimizer/daemon.pid
if [ -f "$PIDF" ]; then
  P=$(cat "$PIDF" 2>/dev/null)
  if [ -n "$P" ] && grep -q "serve" "/proc/$P/cmdline" 2>/dev/null; then
    echo "[$(date)] iaopt ya estaba activo" >> "$LOG"
    exit 0
  fi
fi
for P in $(pidof iaopt 2>/dev/null); do
  if grep -q "serve" "/proc/$P/cmdline" 2>/dev/null; then
    echo "[$(date)] iaopt ya estaba activo" >> "$LOG"
    exit 0
  fi
done

echo "[$(date)] iniciando iaopt en 127.0.0.1:$PORT" >> "$LOG"
# setsid: nueva sesion para que sobreviva al cierre del shell que lo invoco.
nohup setsid "$MODDIR/bin/iaopt" serve --port "$PORT" >> /data/adb/ia_optimizer/daemon.log 2>&1 &

# Watchdog: si el daemon murio durante el arranque, marca el fallo
sleep 3
if ! pidof iaopt >/dev/null 2>&1; then
  F=$(cat /data/adb/ia_optimizer/failed_boots 2>/dev/null || echo 0)
  echo $((F + 1)) > /data/adb/ia_optimizer/failed_boots
  echo "[$(date)] daemon no vivo; fallos=$((F + 1))" >> "$LOG"
else
  echo 0 > /data/adb/ia_optimizer/failed_boots
fi
