package io.teyzen.iaoptimizer.ui

import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.GridLayout
import android.widget.LinearLayout
import android.widget.TextView
import androidx.fragment.app.Fragment
import com.google.android.material.slider.Slider
import com.google.android.material.snackbar.Snackbar
import io.teyzen.iaoptimizer.MainActivity
import io.teyzen.iaoptimizer.R
import io.teyzen.iaoptimizer.VisibleTab
import io.teyzen.iaoptimizer.core.Labels
import io.teyzen.iaoptimizer.core.Models
import io.teyzen.iaoptimizer.core.RustBridge
import io.teyzen.iaoptimizer.databinding.FragmentHomeBinding
import io.teyzen.iaoptimizer.hud.HudController
import org.json.JSONArray
import java.util.Locale
import kotlin.concurrent.thread

/**
 * Inicio: cuadrícula modular de tarjetas flotantes.
 *
 * Cada tarjeta tiene una sola función (estado, procesador, lecturas, memoria,
 * modo, automatización, acciones) y los valores en vivo van en monoespaciado
 * para que no salten al actualizarse. Ninguna sección deja huecos: las rejillas
 * son de dos columnas con espaciado fijo.
 */
class HomeFragment : Fragment(), VisibleTab {

    private var _binding: FragmentHomeBinding? = null
    private val binding get() = _binding!!
    private val handler = Handler(Looper.getMainLooper())
    private var ai = false
    private var syncing = false
    private var cpuHistory = FloatArray(0)

    private val refresh = object : Runnable {
        override fun run() {
            thread {
                val state = Models.parseState(RustBridge.stateJson())
                val telemetryRaw = RustBridge.telemetryJson()
                val telemetry = Models.parseTelemetry(telemetryRaw)
                val root = RustBridge.hasRoot()
                val daemon = RustBridge.http("/api/state").isNotEmpty()
                val device = RustBridge.deviceJson()
                val agent = RustBridge.agentJson()
                val history = historyCpu(RustBridge.historyJson())
                handler.post {
                    if (_binding == null) return@post
                    cpuHistory = history
                    applyState(state, telemetry, root, daemon)
                    paintCpu(telemetry, agent)
                    paintStats(agent, telemetry, state)
                    paintMemory(telemetry)
                    paintDevice(device)
                    // Antes se repetía la consulta HTTP dentro del hilo de UI:
                    // NetworkOnMainThreadException dejaba el hardware vacío.
                    paintHardware(telemetryRaw)
                    paintMotor(agent)
                }
            }
            handler.postDelayed(this, 2000)
        }
    }

    override fun onCreateView(
        inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?
    ): View {
        _binding = FragmentHomeBinding.inflate(inflater, container, false)
        return binding.root
    }

    override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
        binding.profileButtons.addOnButtonCheckedListener { _, checkedId, isChecked ->
            if (syncing) return@addOnButtonCheckedListener
            if (!isChecked) return@addOnButtonCheckedListener
            if (ai) {
                restoreCheckedProfile()
                snack("IA Motors está activa. Apágala para elegir un modo manualmente.")
                return@addOnButtonCheckedListener
            }
            val profile = when (checkedId) {
                R.id.btn_eco -> "eco"
                R.id.btn_performance -> "performance"
                else -> "balanced"
            }
            showModeHint(profile)
            thread {
                val res = RustBridge.setProfile(profile)
                handler.post { if (_binding != null) snack(summarize(profile, res)) }
            }
        }
        binding.switchAi.setOnCheckedChangeListener { _, on ->
            ai = on
            applyLockState()
            showModeHint(currentProfile())
            if (on) snack("IA Motors activada: gestionará el sistema por ti. Tus ajustes manuales quedan guardados.")
            else snack("IA Motors apagada: se restaura tu control manual tal como lo dejaste.")
            thread { RustBridge.setAi(on) }
        }
        binding.btnHud.setOnClickListener {
            if (!HudController.canOverlay(requireContext())) {
                HudController.requestOverlay(requireContext())
            } else {
                HudController.toggle(requireContext())
            }
        }
        binding.btnApps.setOnClickListener { (activity as? MainActivity)?.selectTab(R.id.nav_apps) }

        // El límite se aplica al soltar el control, no en cada paso.
        binding.freqCapSlider.addOnSliderTouchListener(object : Slider.OnSliderTouchListener {
            override fun onStartTrackingTouch(slider: Slider) {}
            override fun onStopTrackingTouch(slider: Slider) {
                if (ai) {
                    snack("IA Motors gestiona el límite de frecuencia. Apágala para ajustarlo.")
                    return
                }
                val pct = slider.value
                thread {
                    val res = RustBridge.setFreqCap(pct / 100f)
                    handler.post {
                        if (_binding == null) return@post
                        binding.freqCapValue.text = freqCapText(pct)
                        snack(
                            if (res.contains("\"ok\":true")) {
                                "Límite de frecuencia: ${pct.toInt()} %"
                            } else {
                                "El kernel no aceptó el límite en este dispositivo"
                            }
                        )
                    }
                }
            }
        })

        // Pintado INMEDIATO con lo último conocido: la primera pantalla no
        // espera al daemon; el bucle de refresco actualiza en segundo plano.
        val cache = io.teyzen.iaoptimizer.core.StateCache
        if (cache.has("/api/state")) {
            val state = Models.parseState(cache.state())
            val telemetryRaw = cache.telemetry()
            val telemetry = Models.parseTelemetry(telemetryRaw)
            val agent = cache.agent()
            cpuHistory = historyCpu(cache.history())
            applyState(state, telemetry, RustBridge.hasRoot(), true)
            paintCpu(telemetry, agent)
            paintStats(agent, telemetry, state)
            paintMemory(telemetry)
            paintDevice(cache.device())
            paintHardware(telemetryRaw)
            paintMotor(agent)
        }
    }

    override fun onResume() {
        super.onResume()
        onTabVisible()
    }

    override fun onPause() {
        super.onPause()
        onTabHidden()
    }

    override fun onTabVisible() {
        handler.removeCallbacks(refresh)
        handler.post(refresh)
    }

    override fun onTabHidden() {
        handler.removeCallbacks(refresh)
    }

    override fun onDestroyView() {
        super.onDestroyView()
        _binding = null
    }

    // ------------------------------- piezas -------------------------------

    private fun currentProfile(): String = when (binding.profileButtons.checkedButtonId) {
        R.id.btn_eco -> "eco"
        R.id.btn_performance -> "performance"
        else -> "balanced"
    }

    private fun applyLockState() {
        binding.textLockHint.visibility = if (ai) View.VISIBLE else View.GONE
        binding.profileButtons.isEnabled = !ai
        binding.freqCapSlider.isEnabled = !ai
    }

    private fun restoreCheckedProfile() {
        val id = when (binding.profileButtons.checkedButtonId) {
            R.id.btn_eco -> R.id.btn_eco
            R.id.btn_performance -> R.id.btn_performance
            else -> R.id.btn_balanced
        }
        syncing = true
        binding.profileButtons.check(id)
        syncing = false
    }

    private fun showModeHint(profile: String) {
        binding.textModeHint.text = Labels.mode(profile).hint
    }

    /** Resumen en lenguaje claro de lo que hizo el modo. */
    private fun summarize(profile: String, raw: String): String {
        val accion = when (profile) {
            "eco" -> "Ahorro de batería activado, CPU limitada y gobernador de bajo consumo."
            "performance" -> "Frecuencia máxima desbloqueada y gobernador de rendimiento."
            else -> "Gobernador adaptativo y límites restablecidos."
        }
        var applied = 0
        var total = 0
        try {
            val o = org.json.JSONObject(raw)
            applied = o.optInt("applied", 0)
            total = o.optInt("total", 0)
        } catch (e: Exception) {
        }
        return if (raw.isBlank()) "$accion (sin conexión)"
        else "$accion Cambios aplicados: $applied de $total."
    }

    private fun applyState(
        state: Models.State?,
        telemetry: Models.Telemetry?,
        root: Boolean,
        daemon: Boolean
    ) {
        if (_binding == null) return
        telemetry?.let { binding.tempValue.text = String.format("%.1f", it.tempC) }
        state?.let { s ->
            applyFreqCap(s.freqCap)
            binding.textReason.text = Labels.reason(s.reason)
            binding.textStatus.text = if (s.ai) "IA Motors activo" else "Modo manual"
            ai = s.ai
            binding.switchAi.setOnCheckedChangeListener(null)
            binding.switchAi.isChecked = s.ai
            binding.switchAi.setOnCheckedChangeListener { _, on ->
                ai = on
                applyLockState()
                showModeHint(currentProfile())
                if (on) snack("IA Motors activada: gestionará el sistema por ti. Tus ajustes manuales quedan guardados.")
                else snack("IA Motors apagada: se restaura tu control manual tal como lo dejaste.")
                thread { RustBridge.setAi(on) }
            }
            applyLockState()
            showModeHint(s.profile)
            val id = when (s.profile) {
                "eco" -> R.id.btn_eco
                "performance" -> R.id.btn_performance
                else -> R.id.btn_balanced
            }
            if (binding.profileButtons.checkedButtonId != id) {
                syncing = true
                binding.profileButtons.check(id)
                syncing = false
            }
        }
        binding.textBridge.text = getString(R.string.estado_puente) + ": " +
            (if (root) "root" else "sin root") + " · demonio " +
            (if (daemon) "activo" else "apagado")
    }

    /** Refleja el límite guardado en el control deslizante, alineado a su
     *  rejilla (valueFrom + n*stepSize): un valor fuera de rejilla crashea
     *  Material Slider al dibujar (p. ej. 60 % con paso 25). */
    private fun applyFreqCap(fraction: Float) {
        val slider = binding.freqCapSlider
        val raw = if (fraction >= 1.0f) slider.valueTo
        else (fraction * 100).coerceIn(slider.valueFrom, slider.valueTo)
        val step = slider.stepSize
        val pct = if (step > 0f) {
            val maxN = Math.floor(((slider.valueTo - slider.valueFrom) / step).toDouble()).toLong()
            val n = Math.round(((raw - slider.valueFrom) / step).toDouble()).coerceIn(0, maxN)
            slider.valueFrom + n * step
        } else {
            raw
        }
        slider.value = pct
        binding.freqCapValue.text = freqCapText(pct)
    }

    private fun freqCapText(pct: Float): String = when {
        pct >= 100f -> "Sin límite: la CPU sube hasta donde pueda."
        else -> String.format("Techo de frecuencia: %d %% de la máxima del chip", pct.toInt())
    }

    /** CPU: carga real, frecuencia del núcleo más activo y su historial. */
    private fun paintCpu(t: Models.Telemetry?, agentJson: String) {
        if (t != null) {
            binding.cpuLoadValue.text = String.format("%d %%", loadFrom(agentJson))
            binding.cpuGovernorValue.text = Labels.choice("", "", t.governor)
            binding.cpuFreqValue.text =
                if (t.cpuMhz > 0) String.format("%d MHz", t.cpuMhz) else "—"
            binding.cpuFreqMax.text = if (t.cpuMaxMhz > 0) {
                "máx ${String.format("%.2f", t.cpuMaxMhz / 1000f)} GHz · núcleos activos ${t.coresOnline}"
            } else {
                "—"
            }
        }
        binding.cpuSpark.setData(cpuHistory)
    }

    /** Carga de CPU media del daemon (el agente la calcula de verdad). */
    private fun loadFrom(agentJson: String): Int = try {
        val o = org.json.JSONObject(agentJson)
        (o.optJSONObject("objectives")?.optDouble("actuacion", 0.0) ?: 0.0).toInt()
    } catch (e: Exception) {
        0
    }

    private fun paintStats(agentJson: String, t: Models.Telemetry?, s: Models.State?) {
        val grid = binding.statsBox
        grid.removeAllViews()
        val m = resources.getDimensionPixelSize(R.dimen.space_xs)
        val tiles = mutableListOf<Pair<String, String>>()
        if (t != null) {
            tiles.add("Voltaje" to if (t.voltageV > 0f) String.format("%.2f V", t.voltageV) else "—")
            tiles.add("Núcleos en línea" to t.coresOnline)
            tiles.add(
                "RAM en uso" to if (t.memTotalMb > 0) {
                    String.format("%.1f GB", (t.memTotalMb - t.memAvailMb) / 1024f)
                } else "—"
            )
            tiles.add(
                "Consumo" to when {
                    t.charging && t.currentMa != 0 ->
                        "cargando · ${t.batteryPct} % · ${kotlin.math.abs(t.currentMa)} mA"
                    t.charging -> "cargando · ${t.batteryPct} %"
                    t.currentMa != 0 -> String.format(
                        Locale.US,
                        "%d %% · %d mA · %.1f W",
                        t.batteryPct, kotlin.math.abs(t.currentMa), kotlin.math.abs(t.powerW)
                    )
                    else -> "${t.batteryPct} % · en uso"
                }
            )
            tiles.add("Frecuencia máxima" to if (t.cpuMaxMhz > 0) {
                String.format("%.2f GHz", t.cpuMaxMhz / 1000f)
            } else "—")
            tiles.add("Pantalla" to if (t.refreshHz > 0f) {
                String.format("%.0f Hz", t.refreshHz)
            } else "—")
        }
        tiles.forEach { (label, value) ->
            val tile = layoutInflater.inflate(R.layout.item_stat, grid, false)
            tile.findViewById<TextView>(R.id.statLabel).text = label
            tile.findViewById<TextView>(R.id.statValue).text = value
            val lp = GridLayout.LayoutParams().apply {
                width = 0
                height = GridLayout.LayoutParams.WRAP_CONTENT
                columnSpec = GridLayout.spec(GridLayout.UNDEFINED, 1, 1f)
                rowSpec = GridLayout.spec(GridLayout.UNDEFINED)
                setMargins(m, m, m, m)
            }
            tile.layoutParams = lp
            grid.addView(tile)
        }
    }

    /** Barras de memoria: RAM, memoria comprimida y almacenamiento. */
    private fun paintMemory(t: Models.Telemetry?) {
        val box = binding.memoryBox
        box.removeAllViews()
        if (t == null) {
            metric(box, "RAM", 0, "sin datos", R.color.text_dim)
            return
        }
        val ram = barra(t.memTotalMb, t.memTotalMb - t.memAvailMb, "GB")
        metric(box, "RAM", ram.first, ram.second, R.color.primary)
        val zram = barra(t.zramTotalMb, t.zramUsedMb, "MB")
        metric(box, "Memoria comprimida", zram.first, zram.second, R.color.secondary)
        val st = barra(t.storageTotalMb, t.storageUsedMb, "GB")
        metric(box, "Almacenamiento", st.first, st.second, R.color.warning)
    }

    private fun barra(total: Long, used: Long, unit: String): Pair<Int, String> {
        if (total <= 0) return 0 to "sin datos"
        val pct = (used * 100 / total).coerceIn(0, 100).toInt()
        val div = if (unit == "GB") 1024f else 1f
        return pct to String.format("%.1f / %.1f %s", used / div, total / div, unit)
    }

    private fun metric(parent: LinearLayout, label: String, pct: Int, value: String, colorRes: Int) {
        val v = layoutInflater.inflate(R.layout.item_metric, parent, false)
        v.findViewById<TextView>(R.id.metricLabel).text = label
        v.findViewById<TextView>(R.id.metricValue).text = String.format("%d %% · %s", pct, value)
        val bar = v.findViewById<android.widget.ProgressBar>(R.id.metricBar)
        bar.progress = pct
        // Tinte solo en la capa de progreso (el fondo conserva su color).
        val d = bar.progressDrawable?.mutate()
        if (d is android.graphics.drawable.LayerDrawable) {
            d.findDrawableByLayerId(android.R.id.progress)?.setColorFilter(
                resources.getColor(colorRes, null),
                android.graphics.PorterDuff.Mode.SRC_IN
            )
        } else {
            d?.setColorFilter(
                resources.getColor(colorRes, null),
                android.graphics.PorterDuff.Mode.SRC_IN
            )
        }
        parent.addView(v)
    }

    private fun paintDevice(json: String) {
        val box = binding.deviceBox
        box.removeAllViews()
        try {
            val o = org.json.JSONObject(json)
            row(box, "Modelo", o.optString("device", "-"))
            row(box, "Chipset", o.optString("soc", "-"))
            row(box, "Gráficos", o.optString("gpu", "-"))
            row(box, "Sistema", if (!o.isNull("android")) o.optString("android") else "Android")
        } catch (e: Exception) {
            row(box, "Modelo", json.ifEmpty { "sin datos" })
        }
    }

    /** Hardware real y dinámico: clústeres de CPU y ZRAM. */
    private fun paintHardware(json: String) {
        val box = binding.hardwareBox
        box.removeAllViews()
        try {
            val o = org.json.JSONObject(json)
            val clusters = o.optJSONArray("clusters")
            if (clusters != null && clusters.length() > 0) {
                for (i in 0 until clusters.length()) {
                    val c = clusters.optJSONObject(i) ?: continue
                    row(
                        box,
                        c.optString("policy").replace("policy", "Clúster ") +
                            "  ·  ${c.optLong("min_mhz", 0)}–${c.optLong("max_mhz", 0)} MHz",
                        if (c.optLong("cur_mhz", 0) > 0) "ahora ${c.optLong("cur_mhz")} MHz" else "en reposo"
                    )
                }
            }
            val zram = o.optJSONObject("zram")
            if (zram != null) {
                row(
                    box,
                    "Memoria comprimida (ZRAM)",
                    "${zram.optLong("used_mb", 0)} de ${zram.optLong("total_mb", 0)} MB"
                )
            }
        } catch (e: Exception) {
            row(box, "Hardware", "sin datos")
        }
    }

    private fun paintMotor(agentJson: String) {
        try {
            val o = org.json.JSONObject(agentJson)
            binding.motorPhase.text = o.optString("phase", "—")
            binding.motorActivity.text = o.optString("current_activity", "—")
        } catch (e: Exception) {
            binding.motorPhase.text = "—"
            binding.motorActivity.text = "—"
        }
    }

    /** Historial de carga para el gráfico (normalizado 0..1). */
    private fun historyCpu(json: String): FloatArray = try {
        val arr = JSONArray(json)
        val out = FloatArray(arr.length())
        for (i in 0 until arr.length()) {
            out[i] = arr.optJSONObject(i)?.optDouble("cpu", 0.0)?.toFloat() ?: 0f
        }
        out
    } catch (e: Exception) {
        FloatArray(0)
    }

    private fun row(parent: LinearLayout, title: String, value: String) {
        val v = layoutInflater.inflate(R.layout.item_row, parent, false)
        v.findViewById<TextView>(R.id.rowTitle).text = title
        v.findViewById<TextView>(R.id.rowValue).text = value
        parent.addView(v)
    }

    private fun snack(text: String) {
        if (_binding == null) return
        Snackbar.make(binding.root, text, Snackbar.LENGTH_LONG).show()
    }
}
