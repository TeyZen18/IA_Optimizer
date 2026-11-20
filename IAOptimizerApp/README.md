# IA Optimizer · APK companion (v0.9.0)

Interfaz nativa Android (Kotlin) que acompaña al módulo Magisk/KernelSU **IA Optimizer**
y sustituye a la WebUI del gestor root. Se comunica con el backend Rust (`iaopt`) que
corre en `/data/adb/modules/ia_optimizer/bin/iaopt`.

## Stack

- **Kotlin + Views** (sin Compose: menor consumo en segundo plano).
- **Material 3** oscuro con acentos cian.
- **PackageManager** nativo: listado instantáneo con nombre real e icono.
- **Intent nativos** para lanzar apps (`startActivity`).
- **SharedPreferences** para marcar juegos y perfiles por app (sin dependencias).
- **Foreground Service + TYPE_APPLICATION_OVERLAY** para el HUD flotante.

## Estructura

```
app/src/main/java/io/teyzen/iaoptimizer/
  MainActivity.kt              # barra inferior + fragmentos
  core/AppInfo.kt              # modelo de app
  core/AppRepository.kt        # PackageManager (lista, iconos, lanzado)
  core/GameStore.kt            # marcadores de juego y perfiles
  core/RustBridge.kt           # HTTP 127.0.0.1:8080 + respaldo root (iaopt)
  core/Models.kt               # parsers de state/telemetry
  core/Parsers.kt              # parsers de scan/agent/display/repair/bypass
  hud/HudService.kt            # servicio en primer plano + vista overlay
  ui/HomeFragment.kt           # banner, dispositivo, rendimiento, perfiles, auto, IA
  ui/AppsFragment.kt           # lista + búsqueda + filtros
  ui/AppAdapter.kt             # RecyclerView de apps
  ui/AppDetailActivity.kt      # ajustes por app
  ui/GameSpaceFragment.kt      # carrusel de juegos
  ui/GameAdapter.kt
  ui/MotorsFragment.kt         # IA Motors: Resumen/Tweaks/Diario/Aprendizaje/Energía/Sistema
  ui/SettingsFragment.kt       # HUD + Pantalla/Registro/Acerca de
```

## Pestañas

| Pestaña | Contenido |
|---|---|
| Inicio | Banner Omega, información del dispositivo, rendimiento, temperatura, perfiles, modo automático e IA |
| Aplicaciones | Listado nativo (PackageManager) con iconos, búsqueda, filtros y ficha por app |
| Game Space | Carrusel solo con apps marcadas como juego y botón de inicio |
| IA Motors | Resumen, Tweaks (controles del kernel), Diario, Aprendizaje, Errores, Energía y Sistema |
| Ajustes | HUD, permiso de superposición, Pantalla, Registro y Acerca de |

## APK companion dentro del módulo

El zip del módulo incluye la APK en `system/app/IAOptimizer/IAOptimizer.apk`, de modo que al
flashear el módulo la app queda instalada como app de sistema. `customize.sh` además intenta
`pm install -r -d` para dejarla disponible de inmediato; si no, se instala al reiniciar.

```
unzip -l IA_Optimizer-v0.8.7.12.zip
#  module.prop, action.sh, customize.sh, ..., bin/iaopt, system/app/IAOptimizer/IAOptimizer.apk
```

## Abrir desde el gestor (Aster/KernelSU/Magisk)

El módulo incluye `action.sh`, que el gestor ejecuta al pulsar **Acción**. Imprime el
deep link `iaoptimizer://open` (registrado en el manifiesto) y además arranca la
actividad directamente. Ya no hay `webroot/`, por lo que el gestor no muestra el
botón de WebUI.

```bash
adb shell am start -a android.intent.action.VIEW -d "iaoptimizer://open"
```

## Permisos

| Permiso | Uso |
|---|---|
| `SYSTEM_ALERT_WINDOW` | Dibujar el HUD sobre los juegos |
| `FOREGROUND_SERVICE` / `FOREGROUND_SERVICE_SPECIAL_USE` | Mantener el HUD vivo |
| `POST_NOTIFICATIONS` | Notificación del servicio |
| `QUERY_ALL_PACKAGES` | Listar todas las apps |
| `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` | Evitar restricciones |

## Puente con Rust

1. **HTTP**: `GET/POST http://127.0.0.1:8080/api/...` (el demonio ya expone la API).
2. **Respaldo root**: `su -c '/data/adb/modules/ia_optimizer/bin/iaopt <cmd>'`.

No se piden credenciales; el root lo gestiona KernelSU/Magisk.

## Compilar

Requiere JDK 17, Gradle 8.10+ y Android SDK (platform 35+, build-tools 34+).

```bash
gradle assembleDebug
# apk: app/build/outputs/apk/debug/app-debug.apk
```

Con la cadena portátil (JDK + Gradle en una carpeta):

```bash
export JAVA_HOME=/ruta/jdk-17
export ANDROID_HOME=/ruta/android-sdk
gradle assembleDebug
```

## Instalar

```bash
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

Después, conceder "Mostrar sobre otras apps" en Ajustes ▸ Apps ▸ IA Optimizer.

## Pestañas activas

Las solapas de **IA Motors** y **Ajustes** usan `TabLayout` con un estado activo evidente:
píldora de fondo cian sólido, texto oscuro en negrita y barra indicadora inferior.

## Motor autónomo (Rust)

`iaopt-daemon/src/watch.rs` audita el sistema cada 5 s:

- Recuento de controles escribibles y de solo lectura.
- Detección de **nodos caídos** (rutas que dejan de responder entre ciclos).
- Aislamiento automático de rutas con **fallos repetidos** (lista negra dinámica).
- **Auto-corrección** de errores sin resolver (configuración, permisos del binario, nodos).
- Registro de cada ciclo en el diario (`kind: "ciclo"`) y API `GET /api/audit`,
  `POST /api/audit/run`, que alimenta la solapa **Errores** de la app.

## Notas

- **Diseño:** tarjetas con fondo semitransparente y borde cian tenue (`bg_card`), radio de
  20 dp, espaciado según `dimens.xml` y jerarquía tipográfica Material 3.
- **Lenguaje claro:** `core/Labels.kt` traduce cada control del kernel a un título y una
  explicación en español. No se muestran rutas, variables ni volcados en pantalla.
- **Controles:** frecuencias con slider (escala GHz/MHz), valores enumerados con selector
  de una sola opción y pantallas con desplegables (renderizador, resolución).
- **Modos:** Rendimiento, Balanceado y Eco cambian gobernador, techo de frecuencia y
  ahorro de batería del sistema; la app confirma con un aviso lo que se aplicó.

- El HUD usa `TYPE_APPLICATION_OVERLAY`; en Android 15 funciona con
  `foregroundServiceType="specialUse"`.
- Los iconos se cachean en `LruCache` (160) para no recargar drawables.
- El backend Rust sigue siendo el encargado de escribir en el kernel; la APK solo
  envía órdenes y muestra telemetría.
