#!/system/bin/sh
# IA Optimizer - post-fs-data.sh : etapas tempranas del arranque (root)
MODDIR=$(dirname "$0")
LOG=/data/adb/ia_optimizer/boot.log

mkdir -p /data/adb/ia_optimizer
: > "$LOG" 2>/dev/null

# Bloqueo de instancia unica: bajo KSU en modo metamodulo post-fs-data se
# ejecuta dos veces; la segunda salida no debe tocar nada (patron aZenith).
LOCK=/dev/.iaoptSingleInstance
if [ -f "$LOCK" ]; then
  echo "[$(date)] post-fs-data ya ejecutado; se omite" >> "$LOG"
  exit 0
fi
touch "$LOCK" 2>/dev/null

# Verificacion de seguridad: si el modulo fallo el arranque 2 veces, se desactiva solo
FLAG=/data/adb/ia_optimizer/failed_boots
FAILS=$(cat "$FLAG" 2>/dev/null || echo 0)
if [ "$FAILS" -ge 2 ]; then
  touch "$MODDIR/disable"
  echo "[$(date)] auto-disable tras $FAILS fallos consecutivos" >> "$LOG"
  exit 0
fi

chmod 0755 "$MODDIR/bin/iaopt" 2>/dev/null
echo "[$(date)] post-fs-data OK" >> "$LOG"
