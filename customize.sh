#!/system/bin/sh
# IA Optimizer - customize.sh : se ejecuta al flashear el modulo (Magisk/KernelSU/APatch)
MODDIR=${0%/*}
BIN="$MODDIR/bin/iaopt"
PROP="$MODDIR/module.prop"

ui_print " "
ui_print "  ╻ ╻┏━┓   ┏━┓┏━┓┏┳┓┏━┓┏━┓┏━╸┏━┓╻ ╻"
ui_print "  ┣┳┫┣┳┛   ┃ ┃┣━┫┃┃┃┣┳┛┣━┫┃  ┣┳┛┗┳┛"
ui_print "  ╹╹╹╹╹    ┗━┛╹ ╹╹ ╹╹╹╸╹ ╹┗━╸╹╹╸ ╹ "
ui_print "  Adaptive optimizer by TeyZen"
ui_print " "

# Permisos del binario nativo
if [ -f "$BIN" ]; then
  chmod 0755 "$BIN"
  ui_print "- Binario nativo listo: bin/iaopt"
else
  ui_print "! No se encontro bin/iaopt (¿ABI no soportada?)"
fi

# Solo arm64 por ahora
ARCH="$(getprop ro.product.cpu.abi)"
case "$ARCH" in
  arm64-v8a) ui_print "- Arquitectura: $ARCH" ;;
  *) ui_print "! ABI no soportada: $ARCH (se requiere arm64-v8a)" ;;
esac

ui_print "- APK companion incluida en system/app/IAOptimizer (app de sistema)"

# Intento de instalacion inmediata (el gestor tiene root); si no, se instala al reiniciar.
# Patron de aZenith: instalacion con tiempo maximo y reintento por `cmd package`.
APK="$MODDIR/system/app/IAOptimizer/IAOptimizer.apk"
if [ -f "$APK" ]; then
  TMP_APK=/data/local/tmp/IAOptimizer_install.apk
  cp "$APK" "$TMP_APK" 2>/dev/null
  chmod 644 "$TMP_APK" 2>/dev/null
  OUT=$(pm install -r -d --user 0 "$TMP_APK" 2>&1)
  echo "$OUT" | grep -iq "Success" || OUT=$(cmd package install -r -d --user 0 "$TMP_APK" 2>&1)
  rm -f "$TMP_APK"
  if echo "$OUT" | grep -iq "Success"; then
    ui_print "- App IA Optimizer instalada"
  else
    ui_print "- La app IA Optimizer se instalara tras reiniciar"
  fi
else
  ui_print "! No se encontro la APK companion en el modulo"
fi

ui_print "- Abre la app IA Optimizer o pulsa Accion en el modulo"
ui_print " "
