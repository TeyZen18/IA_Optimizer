#!/system/bin/sh
# IA Optimizer - uninstall.sh : limpieza al remover el modulo
MODDIR=${0%/*}
pkill -f iaopt 2>/dev/null
rm -rf /data/adb/ia_optimizer
rm -f "$MODDIR/config.json"
exit 0
