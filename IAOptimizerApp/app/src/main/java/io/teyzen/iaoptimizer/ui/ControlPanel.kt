package io.teyzen.iaoptimizer.ui

import android.content.Intent
import android.media.AudioManager
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.ImageButton
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AppCompatDelegate
import androidx.fragment.app.Fragment
import com.google.android.material.button.MaterialButton
import com.google.android.material.materialswitch.MaterialSwitch
import com.google.android.material.slider.Slider
import com.google.android.material.textfield.MaterialAutoCompleteTextView
import io.teyzen.iaoptimizer.R
import io.teyzen.iaoptimizer.core.AgentParser
import io.teyzen.iaoptimizer.core.BypassParser
import io.teyzen.iaoptimizer.core.GameStore
import io.teyzen.iaoptimizer.core.Knob
import io.teyzen.iaoptimizer.core.Labels
import io.teyzen.iaoptimizer.core.Models
import io.teyzen.iaoptimizer.core.RustBridge
import io.teyzen.iaoptimizer.core.StateCache
import io.teyzen.iaoptimizer.game.IslaController
import io.teyzen.iaoptimizer.hud.HudController
import org.json.JSONObject
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlin.concurrent.thread

/**
 * Control: tarjetas agrupadas por sección y subpanel dedicado por tarjeta.
 * Regla de oro: cada subpanel pinta controles y lecturas REALES; si el kernel
 * no expone los nodos de un dominio, se dice claramente (nada de UI vacía).
 * Los paneles con datos en vivo se refrescan solos mientras están abiertos.
 */
class ControlPanel(private val f: MotorsFragment) {

    private val ctx get() = f.requireContext()
    private val inflater get() = f.layoutInflater
    private val handler = Handler(Looper.getMainLooper())
    private var knobs: List<Knob> = emptyList()
    private var openKey: String? = null
    private var openTitle = ""
    private var paused = false
    // Firma del último escaneo: los paneles de knobs solo se repintan si los
    // nodos cambiaron de verdad (no en cada ciclo, que resetearía el scroll).
    private var lastKnobSig = ""
    // El usuario está arrastrando un slider: no reconstruir nada mientras tanto.
    var interacting = false

    private val groups get() = f.requireView().findViewById<LinearLayout>(R.id.control_groups)
    private val scroll get() = f.requireView().findViewById<ScrollView>(R.id.control_scroll)
    private val sub get() = f.requireView().findViewById<LinearLayout>(R.id.control_sub)
    private val content get() = f.requireView().findViewById<android.widget.FrameLayout>(R.id.control_sub_content)
    private val subTitle get() = f.requireView().findViewById<TextView>(R.id.control_sub_title)

    private data class CardDef(val key: String, val title: String, val subtitle: String, val icon: Int)

    private val sections = listOf(
        "Carriles de rendimiento" to listOf(
            CardDef("cpu", "CPU", "Gobernador y frecuencias por clúster", R.drawable.ic_cpu),
            CardDef("gpu", "GPU", "Frecuencias y gobernador gráfico", R.drawable.ic_hud),
            CardDef("memoria", "Memoria", "ZRAM, presión de memoria y lectura en vivo", R.drawable.ic_ram),
            CardDef("sensibilidad", "Sensibilidad", "Respuesta táctil y planificación", R.drawable.ic_sliders)
        ),
        "Térmica y potencia" to listOf(
            CardDef("termico", "Térmico", "Temperatura de control, objetivo y limitación", R.drawable.ic_thermo),
            CardDef("fuerza", "Fuerza", "Batería, consumo real y carga directa", R.drawable.ic_battery),
            CardDef("mostrar", "Mostrar", "Renderizador, resolución y refresco", R.drawable.ic_phone)
        ),
        "Rutas del sistema" to listOf(
            CardDef("almacenamiento", "Almacenamiento y compilador", "Planificadores de I/O y recorte fstrim", R.drawable.ic_ram),
            CardDef("red", "Red", "Velocidad real, conectividad y ajustes TCP", R.drawable.ic_pulse),
            CardDef("audio", "Audio", "Volumen multimedia y estado del sistema", R.drawable.ic_pulse)
        ),
        "Superficies vivas" to listOf(
            CardDef("hud", "HUD de rendimiento", "Telemetría flotante sobre tus apps", R.drawable.ic_hud),
            CardDef("juegos", "Sesiones de juego", "Perfil propio, HUD e inicio por juego", R.drawable.ic_games),
            CardDef("clips", "Álbum de clips", "Graba la pantalla y reproduce tus clips", R.drawable.ic_doc),
            CardDef("isla", "Isla dinámica", "Píldora compacta animada con FPS y temperatura", R.drawable.ic_pulse)
        ),
        "Herramientas avanzadas" to listOf(
            CardDef("lan", "Escáner de red local", "Equipos conectados a tu red", R.drawable.ic_pulse),
            CardDef("uso", "Uso de la aplicación", "Tiempo en primer plano medido por el motor", R.drawable.ic_doc),
            CardDef("consola", "Consola de registro", "Diario y avisos del kernel en vivo", R.drawable.ic_doc),
            CardDef("backup", "Copia de seguridad máxima", "Exporta e importa tu configuración", R.drawable.ic_shield),
            CardDef("appops", "Permisos y operaciones de aplicaciones", "Permisos especiales por app (app ops)", R.drawable.ic_shield),
            CardDef("limpieza", "Limpieza de aplicaciones", "Recorte de cachés y parada de apps", R.drawable.ic_sliders)
        ),
        "Apariencia" to listOf(
            CardDef("colores", "Esquema de colores", "Tema de la aplicación", R.drawable.ic_sliders)
        )
    )

    // Export/import de la copia de seguridad (SAF, sin permisos extra).
    private val exportLauncher = f.registerForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.CreateDocument("application/json")
    ) { uri ->
        if (uri == null) return@registerForActivityResult
        thread {
            val bundle = RustBridge.http("/api/tools/backup")
            try {
                ctx.contentResolver.openOutputStream(uri)?.use { it.write(bundle.toByteArray()) }
                toast("Copia exportada")
            } catch (e: Exception) {
                toast("No se pudo exportar: ${e.message}")
            }
        }
    }
    private val importLauncher = f.registerForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.OpenDocument()
    ) { uri ->
        if (uri == null) return@registerForActivityResult
        thread {
            val body = try {
                ctx.contentResolver.openInputStream(uri)?.bufferedReader()?.readText() ?: ""
            } catch (e: Exception) { "" }
            if (body.isEmpty()) { toast("Archivo vacío o ilegible"); return@thread }
            val r = RustBridge.post("/api/tools/restore", body)
            val detail = try { JSONObject(r).optString("detail", "") } catch (e: Exception) { "" }
            toast(detail.ifEmpty { "Sin respuesta del daemon" })
            f.refresh()
        }
    }

    /** Construye los grupos de tarjetas (una sola vez). */
    fun attach() {
        val g = groups ?: return
        if (g.childCount > 0) return
        sections.forEach { (section, cards) ->
            g.addView(header(section))
            cards.forEach { c -> g.addView(card(c)) }
        }
        f.requireView().findViewById<ImageButton>(R.id.control_back)?.setOnClickListener { closeSub() }
    }

    fun setKnobs(k: List<Knob>) {
        knobs = k
        val sig = k.joinToString("|") { "${it.path}=${it.current}:${it.writable}" }
        if (sig == lastKnobSig || interacting) return
        lastKnobSig = sig
        if (openKey in listOf("cpu", "gpu", "memoria", "sensibilidad", "almacenamiento", "red")) {
            paint(openKey!!)
        }
    }

    fun pause() {
        paused = true
        handler.removeCallbacks(periodic)
    }

    fun resume() {
        paused = false
        if (openKey != null) schedulePeriodic()
    }

    fun closeSub() {
        handler.removeCallbacks(periodic)
        removeGamesFragment()
        fuerzaViews.clear()
        openKey = null
        sub?.visibility = View.GONE
        scroll?.visibility = View.VISIBLE
    }

    /** Refresco del subpanel abierto (lo llama el ciclo de MotorsFragment).
     *  Los paneles de knobs NO se reconstruyen aquí: hacerlo reseteaba el
     *  scroll y cortaba los sliders mientras el usuario configuraba. Solo
     *  se actualizan cuando el escaneo cambia (setKnobs). Los paneles de
     *  lecturas en vivo sí se refrescan, conservando la posición de scroll. */
    fun refreshOpen() {
        val k = openKey ?: return
        if (paused || interacting) return
        when (k) {
            "cpu", "gpu", "memoria", "sensibilidad", "almacenamiento", "red", "hud" -> {}
            "termico" -> paintTermico()
            "fuerza" -> updateFuerza()
            "mostrar" -> thread { val d = RustBridge.displayJson(); ui { f.paintPantalla(d) } }
            "consola" -> paintConsola()
            "clips" -> paintClips()
            "uso" -> paintUso()
            "juegos" -> (f.childFragmentManager.findFragmentByTag("control_games") as? GameSpaceFragment)?.refreshRecorder()
        }
    }

    private val periodic = object : Runnable {
        override fun run() {
            if (openKey == null || paused) return
            refreshOpen()
            schedulePeriodic()
        }
    }

    private fun schedulePeriodic() {
        handler.removeCallbacks(periodic)
        if (openKey in listOf("termico", "fuerza", "consola", "clips", "hud", "uso")) {
            handler.postDelayed(periodic, 3000)
        }
    }

    // ------------------------- apertura de subpaneles -------------------------

    private fun open(key: String, title: String) {
        removeGamesFragment()
        openKey = key
        openTitle = title
        scroll?.visibility = View.GONE
        sub?.visibility = View.VISIBLE
        subTitle?.text = title
        content?.removeAllViews()
        paint(key)
        schedulePeriodic()
    }

    private fun paint(key: String) {
        when (key) {
            "cpu" -> paintKnobs("CPU", listOf("cpu_governor", "cpu_freq"),
                "Este kernel no expone controles de CPU ajustables.")
            "gpu" -> paintKnobs("GPU", listOf("gpu"),
                "Tu GPU no expone nodos ajustables en este kernel. La frecuencia en vivo se muestra si el hardware la publica.")
            "memoria" -> paintKnobs("Memoria", listOf("memory"),
                "Este kernel no expone controles de memoria ajustables.")
            "sensibilidad" -> paintKnobs("Sensibilidad", listOf("responsiveness"),
                "Tu hardware no expone nodos de sensibilidad táctil ni de planificación ajustable. No se ofrece un control que no existiría.")
            "termico" -> paintTermico()
            "fuerza" -> paintFuerza()
            "mostrar" -> paintMostrar()
            "almacenamiento" -> paintAlmacenamiento()
            "red" -> paintRed()
            "audio" -> paintAudio()
            "hud" -> paintHud()
            "juegos" -> openJuegos()
            "clips" -> paintClips()
            "isla" -> paintIsla()
            "lan" -> paintLan()
            "uso" -> paintUso()
            "consola" -> paintConsola()
            "backup" -> paintBackup()
            "appops" -> paintAppops()
            "limpieza" -> paintLimpieza()
            "colores" -> paintColores()
        }
    }

    // ------------------------- paneles de knobs -------------------------

    private fun paintKnobs(title: String, domains: List<String>, emptyMsg: String) {
        val box = newBox()
        val list = knobs.filter { it.domain in domains }
        if (f.aiActive) {
            box.addView(hint("IA Motors está activa: ella gestiona estos controles. Apágala en la tarjeta maestra para editarlos."))
        }
        if (list.isEmpty()) {
            box.addView(hint(emptyMsg))
        } else {
            list.forEach { k -> box.addView(f.knobRow(k)) }
        }
        if (title == "GPU") {
            val gpuLine = row(box, "Frecuencia GPU en vivo", "midiendo…")
            val utilLine = row(box, "Utilización GPU", "—")
            thread {
                val mhz = RustBridge.gpuMhz()
                val util = try { JSONObject(StateCache.telemetry()).optInt("gpu_util", -1) } catch (e: Exception) { -1 }
                ui {
                    gpuLine.second.text = if (mhz > 0) "$mhz MHz" else "no expuesta en este hardware"
                    utilLine.second.text = if (util >= 0) "$util %" else "no expuesta en este hardware"
                }
            }
        }
        if (title == "Memoria") {
            val tel = Models.parseTelemetry(StateCache.telemetry())
            if (tel != null && tel.memTotalMb > 0) {
                row(box, "RAM en uso", String.format(Locale.US, "%.0f %% de %d MB", tel.ramPct, tel.memTotalMb))
                if (tel.zramTotalMb > 0) row(box, "ZRAM", "${tel.zramUsedMb} / ${tel.zramTotalMb} MB")
            }
        }
    }

    private fun paintTermico() {
        val box = newBox()
        val thermal = StateCache.thermal()
        try {
            val o = JSONObject(thermal)
            row(box, "Temperatura de control", String.format(Locale.US, "%.1f °C", o.optDouble("control_c", 0.0)))
            row(box, "Batería / chip", String.format(Locale.US, "%.1f / %.1f °C", o.optDouble("battery_c", 0.0), o.optDouble("soc_c", 0.0)))
            row(box, "Objetivo", String.format(Locale.US, "%.1f °C", o.optDouble("target_c", 0.0)))
            row(box, "Limitación ahora", String.format(Locale.US, "%.0f %%", o.optDouble("intensity", 0.0) * 100))
            row(box, "Intervenciones", o.optInt("interventions", 0).toString())
            row(box, "Estado", o.optString("note", "—"))
        } catch (e: Exception) {
            box.addView(hint("Sin datos del control térmico (¿daemon activo?)."))
        }
        val zones = knobs.filter { it.domain == "thermal" && it.path.endsWith("/temp") }
        if (zones.isNotEmpty()) {
            box.addView(header("Zonas del kernel"))
            zones.take(12).forEach { k ->
                val name = k.path.substringAfterLast("/thermal_zone").substringBefore("/")
                val type = knobs.firstOrNull { it.path == k.path.replace("/temp", "/type") }?.current ?: "zona $name"
                val v = k.current?.toLongOrNull()
                row(box, type, if (v != null) String.format(Locale.US, "%.1f °C", v / 1000.0) else "—")
            }
        }
    }

    // ------------------------- Fuerza (energía) -------------------------
    // El panel se construye UNA vez al abrirse y se actualiza en el sitio:
    // reconstruirlo en cada refresco hacía que apareciera y desapareciera
    // sin dejar ver ni editar nada.
    private val fuerzaViews = mutableMapOf<String, View>()
    private var syncingFuerza = false

    private fun fv(key: String): View? = fuerzaViews[key]

    private fun paintFuerza() {
        fuerzaViews.clear()
        val box = newBox()

        // Batería en vivo.
        box.addView(header("Batería"))
        val big = TextView(ctx).apply {
            text = "—"
            setTextColor(ctx.resources.getColor(R.color.primary, null))
            textSize = 34f
            typeface = android.graphics.Typeface.MONOSPACE
        }
        box.addView(big)
        fuerzaViews["pct"] = big
        fuerzaViews["state"] = row(box, "Estado", "—").second
        fuerzaViews["stuck"] = row(box, "Entrada de corriente", "—").second
        fuerzaViews["ma"] = row(box, "Consumo ahora", "—").second
        fuerzaViews["temp"] = row(box, "Temperatura", "—").second
        fuerzaViews["volt"] = row(box, "Voltaje", "—").second

        // Carga directa (bypass).
        box.addView(header("Carga directa (bypass térmico)"))
        box.addView(hint("Alimenta el móvil directamente del cargador sin pasar por la batería: menos calor al cargar o jugar. Con IA Motors activa, ella la gestiona por ti si la dejaste activada."))
        fuerzaViews["byp_compat"] = row(box, "Compatibilidad", "—").second
        val bypSwitch = MaterialSwitch(ctx).apply {
            text = "Carga directa"
            setTextColor(ctx.resources.getColor(R.color.text, null))
            setOnCheckedChangeListener { _, on ->
                if (syncingFuerza) return@setOnCheckedChangeListener
                val node = (fv("byp_node") as? TextView)?.text?.toString() ?: ""
                thread {
                    val r = RustBridge.bypassSet(on, node)
                    toast(r.ifEmpty { "Sin conexión" })
                    updateFuerza()
                }
            }
        }
        box.addView(bypSwitch)
        fuerzaViews["byp_switch"] = bypSwitch
        val nodeRow = row(box, "Nodo de carga", "—")
        fuerzaViews["byp_node"] = nodeRow.second
        val sliderVal = TextView(ctx).apply {
            text = "Umbral: —"
            setTextColor(ctx.resources.getColor(R.color.text_dim, null))
            textSize = 12f
        }
        box.addView(sliderVal)
        fuerzaViews["byp_slider_val"] = sliderVal
        val slider = Slider(ctx).apply {
            valueFrom = 50f
            valueTo = 100f
            stepSize = 5f
            value = 80f
            addOnSliderTouchListener(object : Slider.OnSliderTouchListener {
                override fun onStartTrackingTouch(s: Slider) { interacting = true }
                override fun onStopTrackingTouch(s: Slider) {
                    interacting = false
                    sliderVal.text = "Umbral: ${s.value.toInt()} %"
                    thread {
                        RustBridge.bypassThreshold(s.value.toInt())
                        updateFuerza()
                    }
                }
            })
        }
        box.addView(slider)
        fuerzaViews["byp_slider"] = slider
        fuerzaViews["byp_state"] = row(box, "Estado", "—").second
        box.addView(button("Probar nodo (escritura + relectura)") {
            val node = (fv("byp_node") as? TextView)?.text?.toString() ?: ""
            it.isEnabled = false
            thread {
                val r = RustBridge.post("/api/bypass-test", JSONObject().put("path", node).toString())
                val ok = r.contains("\"ok\":true")
                val detail = try { JSONObject(r).optString("detail", "") } catch (e: Exception) { "" }
                ui { it.isEnabled = true; toast(if (ok) "Nodo verificado: $detail" else "El nodo no respondió: $detail") }
            }
        })
        box.addView(button("Diagnóstico de nodos") {
            thread {
                val r = RustBridge.bypassDiagnose()
                ui { showBypassDiagnose(r) }
            }
        })
        fuerzaViews["safety_state"] = row(box, "Temporizador de seguridad", "—").second
        box.addView(hint("Protección del vendor que corta la carga tras muchas horas enchufado. Si la carga se detiene con el cargador puesto, puede haber expirado: reinícialo aquí. Con el límite de carga activo, el módulo lo gestiona solo y lo restaura al apagarlo."))
        box.addView(button("Resetear temporizador de carga") {
            it.isEnabled = false
            thread {
                val off = RustBridge.post("/api/charge", JSONObject().put("safety_timer", false).toString())
                try { Thread.sleep(600) } catch (e: InterruptedException) { }
                val on = RustBridge.post("/api/charge", JSONObject().put("safety_timer", true).toString())
                val ok = on.contains("\"ok\":true")
                ui {
                    it.isEnabled = true
                    toast(if (ok) "Temporizador reiniciado: la carga debería reanudarse" else "El nodo no respondió en este hardware")
                    updateFuerza()
                }
            }
        })

        // Límite de carga.
        box.addView(header("Límite de carga"))
        box.addView(hint("Detiene la carga de la batería al llegar al objetivo (el móvil sigue alimentado por el cargador) y la reanuda si baja 5 puntos. Al desenchufar se restaura siempre."))
        val limSwitch = MaterialSwitch(ctx).apply {
            text = "Limitar carga"
            setTextColor(ctx.resources.getColor(R.color.text, null))
            setOnCheckedChangeListener { _, on ->
                if (syncingFuerza) return@setOnCheckedChangeListener
                thread {
                    val r = RustBridge.post("/api/charge", JSONObject().put("limit_enabled", on).toString())
                    val detail = try { JSONObject(r).optString("detail", "") } catch (e: Exception) { "" }
                    toast(detail.ifEmpty { "Sin conexión" })
                    updateFuerza()
                }
            }
        }
        box.addView(limSwitch)
        fuerzaViews["lim_switch"] = limSwitch
        val pillRow = LinearLayout(ctx).apply { orientation = LinearLayout.HORIZONTAL }
        listOf(60, 70, 80, 90, 100).forEach { t ->
            val pill = TextView(ctx).apply {
                text = "$t %"
                textSize = 12f
                gravity = Gravity.CENTER
                setPadding(dp(8), dp(10), dp(8), dp(10))
                layoutParams = LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginEnd = dp(4) }
                setOnClickListener {
                    thread {
                        RustBridge.post("/api/charge", JSONObject().put("target", t).toString())
                        updateFuerza()
                    }
                }
            }
            fuerzaViews["lim_pill_$t"] = pill
            pillRow.addView(pill)
        }
        box.addView(pillRow)
        fuerzaViews["lim_state"] = row(box, "Estado", "—").second

        // Carga inteligente.
        box.addView(header("Carga inteligente"))
        box.addView(hint("Mantiene el 80 % mientras estás enchufado y completa al 100 % antes de tu hora habitual de desenchufe. La aprende de tus desconexiones reales (necesita al menos 3)."))
        val smartSwitch = MaterialSwitch(ctx).apply {
            text = "Carga inteligente"
            setTextColor(ctx.resources.getColor(R.color.text, null))
            setOnCheckedChangeListener { _, on ->
                if (syncingFuerza) return@setOnCheckedChangeListener
                thread {
                    RustBridge.post("/api/charge", JSONObject().put("smart", on).toString())
                    updateFuerza()
                }
            }
        }
        box.addView(smartSwitch)
        fuerzaViews["smart_switch"] = smartSwitch
        fuerzaViews["smart_state"] = row(box, "Rutina aprendida", "—").second

        updateFuerza()
    }

    /** Actualización EN EL SITIO: solo textos y estados, nunca reconstruir. */
    private fun updateFuerza() {
        if (openKey != "fuerza") return
        thread {
            val byp = try { JSONObject(RustBridge.bypassStatus()) } catch (e: Exception) { JSONObject() }
            val chg = try { JSONObject(RustBridge.http("/api/charge")) } catch (e: Exception) { JSONObject() }
            val tel = try { JSONObject(RustBridge.telemetryJson()) } catch (e: Exception) { JSONObject() }
            ui {
                if (openKey != "fuerza") return@ui
                syncingFuerza = true
                val bat = tel.optJSONObject("bat") ?: JSONObject()
                (fv("pct") as? TextView)?.text = "${bat.optInt("pct", 0)} %"
                (fv("state") as? TextView)?.text = if (bat.optBoolean("charging")) "Cargando" else "En uso"
                val stuck = chg.optBoolean("charger_stuck", false)
                (fv("stuck") as? TextView)?.let {
                    it.text = if (stuck) "Sin entrada: reenchufa o cambia el cable/cargador" else "Normal"
                    it.setTextColor(android.graphics.Color.parseColor(if (stuck) "#F87171" else "#E8F1EE"))
                }
                val ma = bat.optInt("current_ma", 0)
                (fv("ma") as? TextView)?.text =
                    if (ma != 0) String.format(Locale.US, "%d mA · %.1f W", kotlin.math.abs(ma), kotlin.math.abs(bat.optDouble("power_w", 0.0))) else "—"
                (fv("temp") as? TextView)?.text = String.format(Locale.US, "%.1f °C", bat.optDouble("temp_c", 0.0))
                (fv("volt") as? TextView)?.text = String.format(Locale.US, "%.2f V", tel.optDouble("voltage_uv", 0.0) / 1_000_000.0)

                // Bypass.
                val supported = byp.optBoolean("supported", false)
                val limOn = chg.optBoolean("limit_enabled", false)
                (fv("byp_compat") as? TextView)?.text =
                    if (supported) "Compatible con tu dispositivo" else "No disponible en este hardware"
                (fv("byp_node") as? TextView)?.text = byp.optString("node", "—").ifEmpty { "—" }
                val bs = fv("byp_switch") as? MaterialSwitch
                bs?.isChecked = byp.optBoolean("enabled", false)
                bs?.isEnabled = supported && !limOn
                val th = byp.optInt("threshold", 80).coerceIn(50, 100)
                (fv("byp_slider_val") as? TextView)?.text = "Umbral: $th %" + if (limOn) " · en pausa: el límite de carga gestiona el nodo" else ""
                (fv("byp_slider") as? Slider)?.let { sl ->
                    val aligned = (Math.round(th / 5f) * 5f).coerceIn(50f, 100f)
                    if (sl.value != aligned) sl.value = aligned
                    sl.isEnabled = supported && !limOn
                }
                (fv("byp_state") as? TextView)?.text = when {
                    !supported -> "—"
                    byp.optBoolean("active", false) -> "Activa ahora: el móvil va directo al cargador"
                    byp.optBoolean("enabled", false) -> "En espera: se activará con cargador y batería ≥ $th %"
                    else -> "Desactivada"
                }
                (fv("safety_state") as? TextView)?.text = when {
                    chg.isNull("safety_timer") -> "No expuesto en este hardware"
                    chg.optBoolean("safety_timer", true) -> "Activo (protección del vendor)" + if (limOn) " · gestionado por el límite" else ""
                    else -> "Desactivado"
                }

                // Límite de carga.
                val ls = fv("lim_switch") as? MaterialSwitch
                ls?.isChecked = limOn
                ls?.isEnabled = chg.optBoolean("supported", false)
                val target = chg.optInt("target", 80)
                listOf(60, 70, 80, 90, 100).forEach { t ->
                    val p = fv("lim_pill_$t") as? TextView ?: return@forEach
                    val active = t == target
                    p.background = pill(if (active) "#34D399" else "#1C2530", dp(12))
                    p.setTextColor(android.graphics.Color.parseColor(if (active) "#06281C" else "#E8F1EE"))
                    p.typeface = if (active) android.graphics.Typeface.DEFAULT_BOLD else android.graphics.Typeface.DEFAULT
                    p.isEnabled = chg.optBoolean("supported", false)
                }
                (fv("lim_state") as? TextView)?.text = chg.optString("detail", "—")

                // Carga inteligente.
                val ss = fv("smart_switch") as? MaterialSwitch
                ss?.isChecked = chg.optBoolean("smart", false)
                ss?.isEnabled = chg.optBoolean("supported", false)
                val learned = chg.optString("learned_unplug", "")
                (fv("smart_state") as? TextView)?.text =
                    if (learned.isNotEmpty() && learned != "null") {
                        "Hora habitual de desenchufe: $learned · completará al 100 % desde 60 min antes"
                    } else {
                        "Aprendiendo tu rutina · ${chg.optInt("unplugs", 0)} desconexiones registradas (mín. 3)"
                    }
                syncingFuerza = false
            }
        }
    }

    private fun showBypassDiagnose(json: String) {
        val sb = StringBuilder()
        try {
            val o = JSONObject(json)
            sb.append("Nodo recomendado: ").append(o.optString("recommended", "—")).append("\n\n")
            val arr = o.optJSONArray("results")
            if (arr != null) for (i in 0 until arr.length()) {
                val r = arr.optJSONObject(i) ?: continue
                sb.append(if (r.optBoolean("ok")) "✓ " else "✗ ")
                sb.append(r.optString("desc")).append("\n")
                sb.append("   ").append(r.optString("detail")).append("\n")
                if (r.optBoolean("risky")) sb.append("   (nodo de riesgo: solo prueba, no activación)\n")
                sb.append('\n')
            }
        } catch (e: Exception) {
            sb.append("Sin datos de diagnóstico.")
        }
        androidx.appcompat.app.AlertDialog.Builder(ctx)
            .setTitle("Diagnóstico de nodos de carga")
            .setMessage(sb.toString())
            .setPositiveButton("Cerrar", null)
            .show()
    }

    private fun paintMostrar() {
        val box = newBox()
        val pane = inflater.inflate(R.layout.pane_pantalla, box, false)
        box.addView(pane)
        pane.findViewById<View>(R.id.btnDisplayApply)?.setOnClickListener {
            thread {
                val r1 = RustBridge.setRenderer(f.lastRenderer)
                val r2 = RustBridge.setResolution(f.lastResolution)
                val d = RustBridge.displayJson()
                ui {
                    toast(listOf(r1, r2).firstOrNull { it.isNotBlank() } ?: "sin conexión")
                    f.paintPantalla(d)
                }
            }
        }
        thread { val d = RustBridge.displayJson(); ui { f.paintPantalla(d) } }
    }

    private fun paintAlmacenamiento() {
        val box = newBox()
        val io = knobs.filter { it.domain == "io_scheduler" }
        if (io.isEmpty()) box.addView(hint("Este kernel no expone planificadores de I/O ajustables."))
        io.forEach { k -> box.addView(f.knobRow(k)) }
        box.addView(header("Mantenimiento"))
        val tel = Models.parseTelemetry(StateCache.telemetry())
        if (tel != null && tel.storageTotalMb > 0) {
            row(box, "Almacenamiento", "${tel.storageUsedMb} / ${tel.storageTotalMb} MB en uso")
        }
        box.addView(hint("fstrim recorta los bloques liberados del almacenamiento: mejora la escritura sostenida. Ejecútalo con la pantalla encendida y sin cargas pesadas."))
        box.addView(button("Ejecutar fstrim") {
            it.isEnabled = false
            thread {
                val r = RustBridge.post("/api/tools/fstrim", "{}")
                val ok = r.contains("\"ok\":true")
                ui { it.isEnabled = true; toast(if (ok) "Recorte ejecutado" else "No se pudo ejecutar en esta ROM") }
            }
        })
    }

    private fun paintRed() {
        val box = newBox()
        val speed = row(box, "Velocidad ahora", "midiendo…")
        thread {
            val r = RustBridge.http("/api/tools/net")
            val txt = try {
                val o = JSONObject(r)
                String.format(Locale.US, "↓ %.2f Mbps · ↑ %.2f Mbps", o.optDouble("rx_mbps", 0.0), o.optDouble("tx_mbps", 0.0))
            } catch (e: Exception) { "sin datos" }
            ui { speed.second.text = txt }
        }
        box.addView(header("Conectividad"))
        if (f.aiActive) box.addView(hint("IA Motors activa: los interruptores de conectividad están bloqueados por el daemon."))
        var syncingQuick = false
        val wifi = switchRow("Wi-Fi", false) { on -> if (!syncingQuick) quick("wifi", on) }
        val data = switchRow("Datos móviles", false) { on -> if (!syncingQuick) quick("data", on) }
        box.addView(wifi)
        box.addView(data)
        thread {
            val w = quickState("wifi_on")
            val d = quickState("mobile_data")
            ui { syncingQuick = true; wifi.isChecked = w; data.isChecked = d; syncingQuick = false }
        }
        val net = knobs.filter { it.domain == "network" }
        if (net.isNotEmpty()) {
            box.addView(header("Ajustes TCP del kernel"))
            net.forEach { k -> box.addView(f.knobRow(k)) }
        }
    }

    private fun quickState(key: String): Boolean = try {
        // Estado leído por el daemon en /api/state no incluye wifi; se usa el
        // valor del sistema vía root (best-effort, el switch es una orden).
        JSONObject(RustBridge.http("/api/quick-state?key=$key")).optBoolean("value", false)
    } catch (e: Exception) { false }

    private fun quick(kind: String, on: Boolean) {
        thread {
            val r = RustBridge.quick("${kind}_${if (on) "on" else "off"}", "")
            val blocked = r.contains("\"blocked\":true")
            val detail = try { JSONObject(r).optString("detail", "") } catch (e: Exception) { "" }
            ui { toast(if (blocked) "IA Motors gestiona esto: apágala para cambiarlo" else detail.ifEmpty { "sin conexión" }) }
        }
    }

    private fun paintAudio() {
        val box = newBox()
        val am = ctx.getSystemService(android.content.Context.AUDIO_SERVICE) as AudioManager
        val max = am.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
        val cur = am.getStreamVolume(AudioManager.STREAM_MUSIC)
        box.addView(header("Volumen multimedia"))
        val volText = TextView(ctx).apply {
            text = "$cur / $max"
            setTextColor(ctx.resources.getColor(R.color.primary, null))
            textSize = 14f
        }
        box.addView(volText)
        val slider = Slider(ctx).apply {
            valueFrom = 0f
            valueTo = max.toFloat()
            stepSize = 1f
            this.value = cur.toFloat().coerceIn(0f, max.toFloat())
            addOnChangeListener { _, v, fromUser ->
                if (fromUser) {
                    am.setStreamVolume(AudioManager.STREAM_MUSIC, v.toInt(), 0)
                    volText.text = "${v.toInt()} / $max"
                }
            }
        }
        box.addView(slider)
        box.addView(hint("El volumen multimedia se ajusta directamente en el sistema, sin root."))
        box.addView(header("Estado de audio del sistema"))
        thread {
            val r = RustBridge.audioJson()
            val props = try { JSONObject(r).optJSONArray("props") } catch (e: Exception) { null }
            ui {
                if (props == null || props.length() == 0) {
                    box.addView(hint("Esta ROM no expone propiedades de audio consultables."))
                } else {
                    for (i in 0 until minOf(props.length(), 12)) {
                        val p = props.optJSONObject(i) ?: continue
                        row(box, p.optString("key").substringAfterLast('.'), p.optString("value", "—"))
                    }
                    box.addView(hint("Lectura informativa: esta ROM no expone nodos de audio ajustables de forma segura."))
                }
            }
        }
    }

    // ------------------------- superficies vivas -------------------------

    private fun paintHud() {
        val box = newBox()
        val ok = HudController.canOverlay(ctx)
        box.addView(switchRow("HUD flotante", false) { on ->
            if (on && !HudController.canOverlay(ctx)) {
                HudController.requestOverlay(ctx)
                toast("Concede el permiso de superposición y vuelve a activarlo")
            } else if (on) HudController.start(ctx)
            else HudController.stop(ctx)
        })
        box.addView(hint(if (ok) "Permiso de superposición: concedido" else "Permiso de superposición: pendiente"))
        box.addView(button("Permiso de superposición") { HudController.requestOverlay(ctx) })
        box.addView(hint("Telemetría compacta arrastrable: CPU, GPU, temperatura, FPS y perfil. Se coloca sobre cualquier app; dentro de un juego mide los FPS de ese juego."))
    }

    private fun openJuegos() {
        removeGamesFragment()
        content?.removeAllViews()
        f.childFragmentManager.beginTransaction()
            .replace(R.id.control_sub_content, GameSpaceFragment(), "control_games")
            .commitNowAllowingStateLoss()
    }

    private fun removeGamesFragment() {
        val frag = f.childFragmentManager.findFragmentByTag("control_games")
        if (frag != null) {
            f.childFragmentManager.beginTransaction().remove(frag).commitNowAllowingStateLoss()
        }
    }

    private fun paintClips() {
        val box = newBox()
        thread {
            val r = RustBridge.http("/api/tools/clip")
            val o = try { JSONObject(r) } catch (e: Exception) { JSONObject() }
            val recording = o.optBoolean("recording", false)
            val files = o.optJSONArray("files")
            ui {
                box.removeAllViews()
                box.addView(header("Grabación de pantalla"))
                box.addView(hint("Graba la pantalla con el grabador del sistema (con root). Los clips quedan en /sdcard/IAOptimizer/clips y se reproducen desde aquí."))
                if (recording) {
                    box.addView(button("Detener grabación") {
                        thread { RustBridge.post("/api/tools/clip", "{\"action\":\"stop\"}"); ui { paintClips() } }
                    })
                } else {
                    box.addView(button("Grabar 60 s") {
                        thread {
                            val rr = RustBridge.post("/api/tools/clip", "{\"action\":\"start\",\"limit\":60}")
                            val ok = rr.contains("\"ok\":true")
                            ui { toast(if (ok) "Grabando…" else "No se pudo grabar en esta ROM"); paintClips() }
                        }
                    })
                }
                box.addView(header("Álbum"))
                if (files == null || files.length() == 0) {
                    box.addView(hint("Todavía no hay clips grabados."))
                } else {
                    for (i in 0 until files.length()) {
                        val c = files.optJSONObject(i) ?: continue
                        val name = c.optString("name")
                        val t = SimpleDateFormat("dd/MM HH:mm", Locale.getDefault()).format(Date(c.optLong("t") * 1000))
                        val r0 = row(box, "$t · ${String.format(Locale.US, "%.1f MB", c.optDouble("size_mb", 0.0))}", name)
                        r0.first.setOnClickListener {
                            try {
                                val intent = Intent(Intent.ACTION_VIEW).apply {
                                    setDataAndType(Uri.parse("http://127.0.0.1:8080/clip?name=$name"), "video/mp4")
                                    addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                                }
                                ctx.startActivity(intent)
                            } catch (e: Exception) {
                                toast("No hay reproductor disponible")
                            }
                        }
                    }
                }
            }
        }
    }

    private fun paintIsla() {
        val box = newBox()
        box.addView(switchRow("Isla dinámica", false) { on ->
            if (on && !HudController.canOverlay(ctx)) {
                HudController.requestOverlay(ctx)
                toast("Concede el permiso de superposición y vuelve a activarla")
            } else if (on) IslaController.start(ctx)
            else IslaController.stop(ctx)
        })
        box.addView(hint("Píldora compacta y animada con FPS y temperatura en vivo, arrastrable a cualquier borde. Pensada para molestar lo mínimo mientras juegas."))
    }

    // ------------------------- herramientas avanzadas -------------------------

    private fun paintLan() {
        val box = newBox()
        box.addView(hint("Equipos vistos por el kernel en tu red (tabla ARP). El sondeo activo hace ping a las 254 direcciones de tu subred y tarda unos segundos."))
        val list = LinearLayout(ctx).apply { orientation = LinearLayout.VERTICAL }
        box.addView(list)
        fun load(json: String) {
            list.removeAllViews()
            val arr = try {
                JSONObject(json).let { it.optJSONArray("neighbors") ?: it.optJSONArray("hosts") }
            } catch (e: Exception) { null }
            if (arr == null || arr.length() == 0) {
                list.addView(hint("Sin equipos visibles todavía."))
            } else {
                for (i in 0 until arr.length()) {
                    val h = arr.optJSONObject(i) ?: continue
                    row(list, h.optString("ip"), h.optString("mac", "—").ifEmpty { "—" })
                }
            }
        }
        thread { val r = RustBridge.http("/api/tools/lan"); ui { load(r) } }
        box.addView(button("Escanear red (ping)") {
            it.isEnabled = false
            toast("Sondeando tu subred…")
            thread {
                val r = RustBridge.post("/api/tools/lan", "{}")
                ui { it.isEnabled = true; load(r) }
            }
        })
    }

    private fun paintUso() {
        val box = newBox()
        box.addView(hint("Tiempo en primer plano medido por el motor, por día. Se acumula solo mientras el daemon está activo; no usa permisos de uso del sistema."))
        thread {
            val r = RustBridge.http("/api/tools/usage")
            val arr = try { JSONObject(r).optJSONArray("apps") } catch (e: Exception) { null }
            ui {
                if (arr == null || arr.length() == 0) {
                    box.addView(hint("Sin medición todavía: el motor registra el uso desde que arranca."))
                } else {
                    for (i in 0 until arr.length()) {
                        val a = arr.optJSONObject(i) ?: continue
                        val s = a.optLong("seconds")
                        val t = if (s >= 3600) String.format(Locale.US, "%d h %02d min", s / 3600, (s % 3600) / 60)
                        else String.format(Locale.US, "%d min %02d s", s / 60, s % 60)
                        row(box, a.optString("package"), t)
                    }
                }
            }
        }
    }

    private fun paintConsola() {
        val box = newBox()
        val a = AgentParser.parse(StateCache.agent())
        box.addView(header("Diario del motor (en vivo)"))
        if (a == null || a.journal.isEmpty()) {
            box.addView(hint("Sin entradas todavía."))
        } else {
            val fmt = SimpleDateFormat("HH:mm:ss", Locale.getDefault())
            a.journal.take(60).forEach { e ->
                val title = "${fmt.format(Date(e.t * 1000))} · ${e.kind} · ${if (e.ok) "ok" else "no sostenido"}"
                val body = buildString {
                    append(e.reason)
                    if (e.path.isNotEmpty() && e.path != "-") append(" · ").append(Labels.choice("", e.path, e.path.substringAfterLast('/')))
                    if (e.value.isNotEmpty() && e.value != "-") append(" = ").append(Labels.choice("", e.path, e.value))
                }
                row(box, title, body)
            }
        }
        box.addView(header("Avisos del kernel"))
        val warns = try { JSONObject(StateCache.warnings()).optJSONArray("warnings") } catch (e: Exception) { null }
        if (warns == null || warns.length() == 0) {
            box.addView(hint("Sin avisos: todos los controles usados son accesibles."))
        } else {
            for (i in 0 until warns.length()) {
                val w = warns.optJSONObject(i) ?: continue
                row(box, Labels.choice("", w.optString("path"), w.optString("path").substringAfterLast('/')), w.optString("kind"))
            }
        }
    }

    private fun paintBackup() {
        val box = newBox()
        box.addView(hint("Exporta tu configuración completa (perfil, preferencias de la IA, juegos y pesos) a un archivo JSON. Al importar, se valida que la copia sea de este mismo dispositivo y SoC: nunca se restaura la configuración de otro móvil."))
        box.addView(button("Exportar copia") {
            exportLauncher.launch("ia-optimizer-backup.json")
        })
        box.addView(button("Importar copia") {
            importLauncher.launch(arrayOf("application/json", "text/plain"))
        })
        box.addView(hint("La restauración de los pesos y el perfil se aplica al reiniciar el daemon (o al cambiar de perfil)."))
    }

    private var appopsPkg = ""

    private fun paintAppops() {
        val box = newBox()
        box.addView(hint("Permisos especiales por aplicación (app ops del sistema, con root). Elige una app y ajusta cada operación; el cambio se verifica con relectura."))
        val picker = MaterialAutoCompleteTextView(ctx).apply {
            hint = "Elige una aplicación"
            setSimpleItems(emptyArray())
        }
        box.addView(picker)
        val opsBox = LinearLayout(ctx).apply { orientation = LinearLayout.VERTICAL }
        box.addView(opsBox)
        thread {
            val r = RustBridge.appsJson("user")
            val arr = try { JSONObject(r).optJSONArray("apps") } catch (e: Exception) { null }
            val names = mutableListOf<String>()
            val pkgs = mutableListOf<String>()
            if (arr != null) for (i in 0 until arr.length()) {
                val o = arr.optJSONObject(i) ?: continue
                names.add(o.optString("name", o.optString("package")))
                pkgs.add(o.optString("package"))
            }
            ui {
                picker.setSimpleItems(names.toTypedArray())
                picker.setOnItemClickListener { _, _, pos, _ ->
                    if (pos in pkgs.indices) {
                        appopsPkg = pkgs[pos]
                        loadOps(appopsPkg, opsBox)
                    }
                }
            }
        }
    }

    private fun loadOps(pkg: String, box: LinearLayout) {
        box.removeAllViews()
        thread {
            val r = RustBridge.http("/api/tools/appops?package=$pkg")
            val arr = try { JSONObject(r).optJSONArray("ops") } catch (e: Exception) { null }
            ui {
                if (arr == null) { box.addView(hint("Sin respuesta del daemon.")); return@ui }
                for (i in 0 until arr.length()) {
                    val o = arr.optJSONObject(i) ?: continue
                    val op = o.optString("op")
                    val mode = o.optString("mode", "default")
                    val r0 = row(box, opLabel(op), mode)
                    val modes = listOf("allow", "deny", "ignore", "default")
                    r0.first.setOnClickListener {
                        val popup = android.widget.PopupMenu(ctx, r0.first)
                        modes.forEach { m -> popup.menu.add(opLabel(m)) }
                        popup.setOnMenuItemClickListener { item ->
                            val m = modes.getOrNull(item.itemId) ?: "default"
                            thread {
                                val rr = RustBridge.post("/api/tools/appops",
                                    JSONObject().put("package", pkg).put("op", op).put("mode", m).toString())
                                val verified = rr.contains("\"verified\":true")
                                ui {
                                    toast(if (verified) "Permiso aplicado y verificado" else "La ROM no aceptó el cambio")
                                    loadOps(pkg, box)
                                }
                            }
                            true
                        }
                        popup.show()
                    }
                }
            }
        }
    }

    private fun opLabel(op: String): String = when (op) {
        "SYSTEM_ALERT_WINDOW" -> "Mostrar sobre otras apps"
        "WRITE_SETTINGS" -> "Modificar ajustes del sistema"
        "POST_NOTIFICATION" -> "Notificaciones"
        "RUN_IN_BACKGROUND" -> "Ejecutar en segundo plano"
        "RUN_ANY_IN_BACKGROUND" -> "Cualquier actividad en segundo plano"
        "CAMERA" -> "Cámara"
        "RECORD_AUDIO" -> "Micrófono"
        "ACCESS_FINE_LOCATION" -> "Ubicación precisa"
        "READ_CONTACTS" -> "Contactos"
        "READ_SMS" -> "SMS"
        "WAKE_LOCK" -> "Mantener despierto"
        "allow" -> "Permitir"
        "deny" -> "Denegar"
        "ignore" -> "Ignorar (silencioso)"
        "default" -> "Predeterminado"
        else -> op
    }

    private fun paintLimpieza() {
        val box = newBox()
        box.addView(header("Cachés del sistema"))
        box.addView(hint("Pide al sistema recortar las cachés de todas las apps hasta liberar espacio. No borra datos de usuario."))
        box.addView(button("Recortar cachés (1 GB)") {
            it.isEnabled = false
            thread {
                val r = RustBridge.post("/api/tools/clean", "{\"action\":\"trim\",\"bytes\":1073741824}")
                val ok = r.contains("\"ok\":true")
                ui { it.isEnabled = true; toast(if (ok) "Cachés recortadas" else "No se pudo en esta ROM") }
            }
        })
        box.addView(header("Detener apps"))
        box.addView(hint("Fuerza la parada de una app (como el ajuste del sistema). Úsalo con apps que no necesites en segundo plano; no se detienen apps del sistema críticas."))
        thread {
            val r = RustBridge.appsJson("user")
            val arr = try { JSONObject(r).optJSONArray("apps") } catch (e: Exception) { null }
            ui {
                if (arr == null || arr.length() == 0) {
                    box.addView(hint("Sin lista de apps disponible."))
                } else {
                    for (i in 0 until minOf(arr.length(), 40)) {
                        val o = arr.optJSONObject(i) ?: continue
                        val pkg = o.optString("package")
                        val name = o.optString("name", pkg)
                        val r0 = row(box, name, pkg)
                        val stop = button("Detener") {
                            androidx.appcompat.app.AlertDialog.Builder(ctx)
                                .setTitle("Detener $name")
                                .setMessage("La app se cerrará y dejará de ejecutarse en segundo plano hasta que la abras de nuevo.")
                                .setPositiveButton("Detener") { _, _ ->
                                    thread {
                                        val rr = RustBridge.post("/api/tools/clean",
                                            JSONObject().put("action", "stop").put("package", pkg).toString())
                                        ui { toast(if (rr.contains("\"ok\":true")) "App detenida" else "No se pudo detener") }
                                    }
                                }
                                .setNegativeButton("Cancelar", null)
                                .show()
                        }
                        r0.first.addView(stop)
                    }
                }
            }
        }
    }

    private fun paintColores() {
        val box = newBox()
        box.addView(hint("Tema de la aplicación. El panel in-game y el HUD usan su propia paleta oscura gaming (verde-cian) para ser legibles sobre cualquier juego."))
        val current = GameStore.themeMode(ctx)
        listOf("sistema" to "Seguir al sistema", "claro" to "Claro", "oscuro" to "Oscuro").forEach { (key, label) ->
            val r0 = row(box, label, if (key == current) "activo" else "")
            r0.first.setOnClickListener {
                GameStore.setThemeMode(ctx, key)
                AppCompatDelegate.setDefaultNightMode(
                    when (key) {
                        "claro" -> AppCompatDelegate.MODE_NIGHT_NO
                        "oscuro" -> AppCompatDelegate.MODE_NIGHT_YES
                        else -> AppCompatDelegate.MODE_NIGHT_FOLLOW_SYSTEM
                    }
                )
                f.requireActivity().recreate()
            }
        }
        box.addView(hint("El esquema de color global de la pantalla (matriz SurfaceFlinger) varía por ROM y no se ofrece sin verificar los códigos de tu kernel: se añadirá cuando se pueda hacer con seguridad."))
    }

    // ------------------------- helpers de UI -------------------------

    private fun newBox(): LinearLayout {
        val c = content ?: return LinearLayout(ctx)
        // Conservar la posición de scroll entre refrescos: el usuario no
        // debe volver arriba cada vez que llega un dato nuevo.
        val prevScroll = (c.getChildAt(0) as? ScrollView)?.scrollY ?: 0
        c.removeAllViews()
        val scroll = ScrollView(ctx).apply {
            layoutParams = android.widget.FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT
            )
        }
        val box = LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(12), dp(4), dp(12), dp(20))
        }
        scroll.addView(box)
        c.addView(scroll)
        if (prevScroll > 0) scroll.post { scroll.scrollTo(0, prevScroll) }
        return box
    }

    private fun dp(v: Int): Int = (v * ctx.resources.displayMetrics.density).toInt()

    private fun pill(color: String, radius: Int): android.graphics.drawable.GradientDrawable =
        android.graphics.drawable.GradientDrawable().apply {
            setColor(android.graphics.Color.parseColor(color))
            cornerRadius = radius.toFloat()
        }

    private fun header(text: String): TextView = TextView(ctx).apply {
        this.text = text.uppercase()
        setTextColor(ctx.resources.getColor(R.color.primary, null))
        textSize = 12f
        letterSpacing = 0.08f
        setPadding(dp(4), dp(16), dp(4), dp(6))
    }

    private fun hint(text: String): TextView = TextView(ctx).apply {
        this.text = text
        setTextColor(ctx.resources.getColor(R.color.text_dim, null))
        textSize = 12f
        setPadding(dp(4), dp(4), dp(4), dp(8))
    }

    private fun row(parent: LinearLayout, title: String, value: String): Pair<LinearLayout, TextView> {
        val v = inflater.inflate(R.layout.item_row, parent, false)
        val t = v.findViewById<TextView>(R.id.rowTitle)
        val val0 = v.findViewById<TextView>(R.id.rowValue)
        t.text = title
        val0.text = value
        parent.addView(v)
        return (v as LinearLayout) to val0
    }

    private fun card(c: CardDef): View {
        val v = inflater.inflate(R.layout.item_row, groups, false) as LinearLayout
        v.orientation = LinearLayout.HORIZONTAL
        v.gravity = Gravity.CENTER_VERTICAL
        val icon = android.widget.ImageView(ctx).apply {
            setImageResource(c.icon)
            setColorFilter(ctx.resources.getColor(R.color.primary, null))
            layoutParams = LinearLayout.LayoutParams(dp(28), dp(28))
        }
        val col = LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL
            layoutParams = LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginStart = dp(12) }
        }
        val t = TextView(ctx).apply {
            text = c.title
            setTextColor(ctx.resources.getColor(R.color.text, null))
            textSize = 14f
            typeface = android.graphics.Typeface.DEFAULT_BOLD
        }
        val s = TextView(ctx).apply {
            text = c.subtitle
            setTextColor(ctx.resources.getColor(R.color.text_dim, null))
            textSize = 11f
        }
        col.addView(t)
        col.addView(s)
        val chev = android.widget.ImageView(ctx).apply {
            setImageResource(R.drawable.ic_chevron)
            setColorFilter(ctx.resources.getColor(R.color.text_dim, null))
            layoutParams = LinearLayout.LayoutParams(dp(24), dp(24))
        }
        v.removeAllViews()
        v.addView(icon)
        v.addView(col)
        v.addView(chev)
        v.setOnClickListener { open(c.key, c.title) }
        return v
    }

    private fun button(text: String, onClick: (MaterialButton) -> Unit): MaterialButton =
        MaterialButton(ctx).apply {
            this.text = text
            isAllCaps = false
            layoutParams = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply {
                topMargin = dp(6)
            }
            setOnClickListener { onClick(this) }
        }

    private fun switchRow(title: String, initial: Boolean, onChange: (Boolean) -> Unit): MaterialSwitch =
        MaterialSwitch(ctx).apply {
            text = title
            setTextColor(ctx.resources.getColor(R.color.text, null))
            isChecked = initial
            setPadding(0, dp(6), 0, dp(6))
            setOnCheckedChangeListener { _, on -> onChange(on) }
        }

    private fun toast(msg: String) {
        f.activity?.runOnUiThread { Toast.makeText(ctx, msg, Toast.LENGTH_SHORT).show() }
    }

    private fun ui(block: () -> Unit) {
        f.activity?.runOnUiThread { if (f.view != null) block() }
    }
}
