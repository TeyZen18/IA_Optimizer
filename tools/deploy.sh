#!/system/bin/sh
# Despliegue de iaopt v0.9.1: para el daemon, sustituye el binario con mv
# (atomico, evita ETXTBSY), actualiza module.prop y relanza el daemon.
MOD=/data/adb/modules/ia_optimizer
PIDF=/data/adb/ia_optimizer/daemon.pid

if [ -f "$PIDF" ]; then
  P=$(cat "$PIDF")
  if grep -q serve "/proc/$P/cmdline" 2>/dev/null; then
    kill -9 "$P" 2>/dev/null
    sleep 1
  fi
fi

mv -f /data/local/tmp/iaopt.new "$MOD/bin/iaopt"
chmod 755 "$MOD/bin/iaopt"
cp /data/local/tmp/module.prop.new "$MOD/module.prop"

nohup setsid "$MOD/bin/iaopt" serve --port 8080 >> /data/adb/ia_optimizer/daemon.log 2>&1 &
sleep 2
echo "pidfile=$(cat $PIDF 2>/dev/null)"
grep -c serve "/proc/$(cat $PIDF 2>/dev/null)/cmdline" 2>/dev/null
tail -3 /data/adb/ia_optimizer/daemon.log
