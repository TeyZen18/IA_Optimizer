#!/system/bin/sh
# IA Optimizer - action.sh
# El administrador (KernelSU/Magisk/APatch) ejecuta este script y abre la URL
# que imprime. Ahora la interfaz completa vive en la APK companion, asi que
# abrimos la app en lugar de la WebUI del modulo.
#
# Deep link registrado en el manifiesto de la APK (iaoptimizer://open).
echo "iaoptimizer://open"

# Respaldo: si el gestor no abre la URL, intentamos arrancar la actividad.
am start -n io.teyzen.iaoptimizer/.MainActivity >/dev/null 2>&1
exit 0
