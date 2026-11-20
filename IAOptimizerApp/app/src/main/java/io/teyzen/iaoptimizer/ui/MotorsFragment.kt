package io.teyzen.iaoptimizer.ui

import android.os.Bundle
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import androidx.fragment.app.Fragment
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.RecyclerView
import com.google.android.material.button.MaterialButton
import com.google.android.material.chip.Chip
import com.google.android.material.chip.ChipGroup
import com.google.android.material.slider.Slider
import io.teyzen.iaoptimizer.R
import io.teyzen.iaoptimizer.core.AgentData
import io.teyzen.iaoptimizer.core.AgentParser
import io.teyzen.iaoptimizer.core.BypassData
import io.teyzen.iaoptimizer.core.BypassParser
import io.teyzen.iaoptimizer.core.Knob
import io.teyzen.iaoptimizer.core.Labels
import io.teyzen.iaoptimizer.core.RepairParser
import io.teyzen.iaoptimizer.core.RustBridge
import io.teyzen.iaoptimizer.core.ScanParser
import io.teyzen.iaoptimizer.databinding.FragmentMotorsBinding
import org.json.JSONObject
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlin.concurrent.thread

/**
 * IA Motors: resumen, tweaks (controles del kernel), diario, aprendizaje,
 * energia y sistema. Todo alimentado por la API local del daemon Rust.
 */
class MotorsFragment : Fragment(), io.teyzen.iaoptimizer.VisibleTab {

    private var _binding: FragmentMotorsBinding? = null
    private val binding get() = _binding!!
    private val bridge = io.teyzen.iaoptimizer.core.RustBridge
    private val handler = android.os.Handler(android.os.Looper.getMainLooper())
    private var refreshing = false
    internal var lastRenderer = ""
    internal var lastResolution = "default"

    /** Control agrupado con subpaneles dedicados (tarjetas por sección). */
    internal val control = ControlPanel(this)

    private var agent: AgentData? = null
    private var knobs: List<Knob> = emptyList()
    private var journalFilter = "cambios"
    private var filterKeys = listOf(
        "todos", "cambios", "sondas", "usuario", "eficaces", "fallidos", "auditoria"
    )
    private var search = ""
    private var currentProfile = "balanced"
    internal var aiActive = true
    // Flags anti-eco: el refresco periódico sincroniza los controles sin
    // disparar sus listeners (que reenviarían la orden anterior).
    private var syncingMaster = false
    private var syncingPrefs = false

    private val journalAdapter = JournalAdapter()

    override fun onCreateView(
        inflater: LayoutInflater,
        container: ViewGroup?,
        savedInstanceState: Bundle?
    ): View {
        _binding = FragmentMotorsBinding.inflate(inflater, container, false)
        return binding.root
    }

    override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
        setupTabs(
            listOf(
                "Descripción", "Diario", "Aprendizaje", "Controles", "Sistema"
            )
        )
        control.attach()

        binding.root.findViewById<RecyclerView>(R.id.diagList)?.apply {
            layoutManager = LinearLayoutManager(requireContext())
            adapter = journalAdapter
        }
        binding.root.findViewById<EditText>(R.id.diagSearch)?.apply {
            addTextChangedListener(object : android.text.TextWatcher {
                override fun afterTextChanged(s: android.text.Editable?) {
                    search = s?.toString() ?: ""
                    journalAdapter.submit(filtered())
                }
                override fun beforeTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) {}
                override fun onTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) {}
            })
        }
        val filters = binding.root.findViewById<ChipGroup>(R.id.diagFilters)
        // Categorias reales del diario del daemon. Antes los filtros buscaban
        // categorias que el daemon nunca escribe ("weight", "review", ...) y
        // por eso salian siempre vacios.
        filterKeys = listOf(
            "todos", "cambios", "sondas", "usuario", "eficaces", "fallidos", "auditoria"
        )
        listOf(
            "todos" to "Todos",
            "cambios" to "Cambios",
            "sondas" to "Sondas",
            "usuario" to "Anuladas",
            "eficaces" to "Eficaces",
            "fallidos" to "No sostenidos",
            "auditoria" to "Auditoría"
        ).forEach { (key, label) ->
            val c = Chip(requireContext()).apply {
                text = label
                isCheckable = true
                isChecked = key == "cambios"
                id = View.generateViewId()
            }
            filters?.addView(c)
        }
        journalFilter = "cambios"
        filters?.setOnCheckedChangeListener { group, checkedId ->
            val idx = (0 until group.childCount).firstOrNull { group.getChildAt(it).id == checkedId } ?: 0
            journalFilter = filterKeys[idx]
            journalAdapter.submit(filtered())
        }
        binding.root.findViewById<View>(R.id.btnApply)?.setOnClickListener {
            thread {
                val r = bridge.applyProfile(currentProfile)
                activity?.runOnUiThread {
                    if (_binding == null) return@runOnUiThread
                    Toast.makeText(requireContext(), r.ifEmpty { "sin conexión" }, Toast.LENGTH_SHORT).show()
                    refresh()
                }
            }
        }

        // Pantalla y Energía viven ahora en Control → Mostrar / Fuerza.
        // HUD y tema viven en Control → HUD de rendimiento / Esquema de colores.
        // Panel de juego in-game (Game Space avanzado): preferencia persistida.
        val panelSwitch = binding.root.findViewById<com.google.android.material.materialswitch.MaterialSwitch>(R.id.switch_game_panel)
        panelSwitch?.isChecked = io.teyzen.iaoptimizer.core.GameStore.panelEnabled(requireContext())
        panelSwitch?.setOnCheckedChangeListener { _, on ->
            if (on && !io.teyzen.iaoptimizer.hud.HudController.canOverlay(requireContext())) {
                io.teyzen.iaoptimizer.hud.HudController.requestOverlay(requireContext())
                panelSwitch.isChecked = false
                return@setOnCheckedChangeListener
            }
            io.teyzen.iaoptimizer.core.GameStore.setPanelEnabled(requireContext(), on)
            if (on) io.teyzen.iaoptimizer.game.GamePanelController.start(requireContext())
            else io.teyzen.iaoptimizer.game.GamePanelController.stop(requireContext())
        }
        // Encabezado [1]: retroceso a Inicio.
        binding.root.findViewById<View>(R.id.btn_motors_back)?.setOnClickListener {
            (activity as? io.teyzen.iaoptimizer.MainActivity)?.selectTab(R.id.nav_inicio)
        }

        // [2] Interruptor maestro de IA Motors (mismo efecto que el de Inicio).
        binding.root.findViewById<com.google.android.material.materialswitch.MaterialSwitch>(R.id.switch_master)
            ?.setOnCheckedChangeListener { _, on ->
                if (syncingMaster) return@setOnCheckedChangeListener
                thread {
                    bridge.setAi(on)
                    activity?.runOnUiThread {
                        if (_binding == null) return@runOnUiThread
                        aiActive = on
                        Toast.makeText(
                            requireContext(),
                            if (on) "IA Motors activada: modo autónomo" else "IA Motors apagada: control manual",
                            Toast.LENGTH_SHORT
                        ).show()
                        refresh()
                    }
                }
            }

        // [3] Árbol de capacidades.
        binding.root.findViewById<View>(R.id.card_capabilities)?.setOnClickListener { showCapabilities() }

        // [8] Objetivo de optimización.
        binding.root.findViewById<com.google.android.material.button.MaterialButtonToggleGroup>(R.id.goal_group)
            ?.addOnButtonCheckedListener { _, checkedId, isChecked ->
                if (syncingPrefs || !isChecked) return@addOnButtonCheckedListener
                val g = when (checkedId) {
                    R.id.btn_goal_actuacion -> "actuacion"
                    R.id.btn_goal_bateria -> "bateria"
                    else -> "equilibrado"
                }
                thread {
                    bridge.setPrefs(g, null)
                    activity?.runOnUiThread {
                        if (_binding == null) return@runOnUiThread
                        refresh()
                    }
                }
            }

        // [9] Comportamiento de intervención.
        binding.root.findViewById<com.google.android.material.button.MaterialButtonToggleGroup>(R.id.behavior_group)
            ?.addOnButtonCheckedListener { _, checkedId, isChecked ->
                if (syncingPrefs || !isChecked) return@addOnButtonCheckedListener
                val b = when (checkedId) {
                    R.id.btn_behavior_calma -> "calma"
                    R.id.btn_behavior_decisivo -> "decisivo"
                    else -> "equilibrado"
                }
                thread {
                    bridge.setPrefs(null, b)
                    activity?.runOnUiThread {
                        if (_binding == null) return@runOnUiThread
                        refresh()
                    }
                }
            }

        // [12] Comandos en Línea: pantalla profesional con gráficos en vivo.
        binding.root.findViewById<View>(R.id.card_command)?.setOnClickListener {
            startActivity(android.content.Intent(requireContext(), CommandCenterActivity::class.java))
        }

        // Pintado INMEDIATO desde la caché: la pantalla nunca abre en blanco
        // ni espera al daemon; el refresh() de abajo actualiza en segundo plano.
        val cache = io.teyzen.iaoptimizer.core.StateCache
        val cachedAgent = AgentParser.parse(cache.agent())
        if (cachedAgent != null) {
            agent = cachedAgent
            try {
                val st = JSONObject(cache.state())
                currentProfile = st.optString("profile", "balanced")
                aiActive = st.optBoolean("ai", true)
            } catch (e: Exception) { }
            paintResumen(cachedAgent, cache.warnings(), cache.thermal(), cache.history())
            paintAprendizaje(cachedAgent)
            journalAdapter.submit(filtered())
        }
        refresh()
    }

    override fun onTabVisible() {
        updateOverlayState()
        control.resume()
        refresh()
        handler.removeCallbacks(liteRefresh)
        handler.postDelayed(liteRefresh, 5000)
    }

    override fun onTabHidden() {
        handler.removeCallbacks(liteRefresh)
        control.pause()
    }

    /** Refresco ligero periódico: solo el Resumen (gráfico en vivo). */
    private val liteRefresh = object : Runnable {
        override fun run() {
            if (_binding == null) return
            thread {
                val a = AgentParser.parse(bridge.agentJson())
                val history = bridge.historyJson()
                val thermal = bridge.thermalJson()
                val warns = bridge.warningsJson()
                activity?.runOnUiThread {
                    if (_binding == null) return@runOnUiThread
                    agent = a
                    paintResumen(a, warns, thermal, history)
                    journalAdapter.submit(filtered())
                    control.refreshOpen()
                }
            }
            handler.postDelayed(this, 5000)
        }
    }

    private fun updateOverlayState() {
        // El estado del permiso de superposición vive ahora en
        // Control → HUD de rendimiento (una sola fuente por dato).
    }

    private fun filtered(): List<io.teyzen.iaoptimizer.core.JournalEntry> {
        val a = agent ?: return emptyList()
        val list = a.journal.filter { e ->
            val okSearch = search.isBlank() || e.reason.contains(search, true) ||
                e.path.contains(search, true) || e.kind.contains(search, true)
            val okFilter = when (journalFilter) {
                "todos" -> true
                // Cambios reales: lo que el motor o el usuario decidieron.
                "cambios" -> e.kind != "ciclo" && e.kind != "sonda"
                "sondas" -> e.kind == "sonda"
                "usuario" -> e.kind == "usuario"
                "eficaces" -> e.ok
                "fallidos" -> !e.ok
                "auditoria" -> e.kind == "ciclo"
                else -> true
            }
            okSearch && okFilter
        }
        return list.take(200)
    }

    fun refresh() {
        if (refreshing) return
        refreshing = true
        thread {
            try {
                // El estado se lee en este hilo de trabajo: antes se hacía en
                // el de UI y NetworkOnMainThreadException dejaba el perfil
                // siempre en "balanced".
                val stateRaw = bridge.stateJson()
                val a = AgentParser.parse(bridge.agentJson())
                val history = bridge.historyJson()
                val thermal = bridge.thermalJson()
                val warns = bridge.warningsJson()
                activity?.runOnUiThread {
                    if (_binding == null) return@runOnUiThread
                    agent = a
                    currentProfile = try {
                        JSONObject(stateRaw).optString("profile", "balanced")
                    } catch (e: Exception) {
                        "balanced"
                    }
                    aiActive = try {
                        JSONObject(stateRaw).optBoolean("ai", true)
                    } catch (e: Exception) {
                        true
                    }
                    binding.root.findViewById<View>(R.id.btnApply)?.isEnabled = !aiActive
                    paintResumen(a, warns, thermal, history)
                    paintAprendizaje(a)
                    journalAdapter.submit(filtered())
                }
                // Cada sección se pinta según llegan sus datos: la primera
                // pantalla no espera a las nueve consultas al daemon.
                val k = ScanParser.parse(bridge.scanJson())
                activity?.runOnUiThread {
                    if (_binding == null) return@runOnUiThread
                    knobs = k
                    control.setKnobs(k)
                }
                val dev = bridge.deviceJson()
                val rep = RepairParser.parse(bridge.repairJson())
                val audit = bridge.auditJson()
                activity?.runOnUiThread {
                    if (_binding == null) return@runOnUiThread
                    paintSistema(dev, rep, audit)
                    control.refreshOpen()
                }
            } finally {
                refreshing = false
            }
        }
    }

    private fun paintResumen(a: AgentData?, warnsJson: String, thermalJson: String, historyJson: String) {
        paintChart(historyJson)
        paintObjectives(a)
        paintMaster()
        paintMeasured(a)
        paintLive(a)
        paintHeat(a)
        paintPrefs(a)
        paintThermalSafety(a, thermalJson)
        paintEvidence(a)
        val box = binding.root.findViewById<LinearLayout>(R.id.resumenBox) ?: return
        box.removeAllViews()
        if (a == null) {
            row(box, "IA Motors", "Sin datos (¿daemon activo?)")
            return
        }
        // Avisos del kernel y errores sin resolver: el resto de lecturas vive
        // en sus tarjetas propias (una sola fuente por dato).
        paintAvisos(box, warnsJson)
        if (a.unresolved.isNotEmpty()) {
            header(box, "Errores sin resolver (${a.unresolved.size})")
            a.unresolved.take(6).forEach {
                row(box, it.kind, Labels.choice("", it.path, it.path.substringAfterLast('/')) + " · " + it.reason)
            }
        }
    }

    /** [2] Tarjeta maestra: estado del interruptor y modo de operación. */
    private fun paintMaster() {
        val root = binding.root
        val sw = root.findViewById<com.google.android.material.materialswitch.MaterialSwitch>(R.id.switch_master)
        if (sw != null) {
            syncingMaster = true
            sw.isChecked = aiActive
            syncingMaster = false
        }
        root.findViewById<TextView>(R.id.master_subtitle)?.text =
            if (aiActive) "Modo autónomo · perfil ${profileLabel(currentProfile)} · ella decide y verifica"
            else "Apagada: control manual del sistema"
    }

    /** [5] Satisfacción objetiva: valor grande, objetivo, planificador y edad. */
    private fun paintMeasured(a: AgentData?) {
        val root = binding.root
        val value = root.findViewById<TextView>(R.id.sat_value) ?: return
        if (a == null) {
            value.text = "—"
            root.findViewById<TextView>(R.id.sat_badge)?.text = "Sin datos"
            root.findViewById<TextView>(R.id.sat_target)?.text = "Objetivo — · planificador —"
            root.findViewById<TextView>(R.id.sat_updated)?.text = "Sin lecturas todavía"
            return
        }
        value.text = String.format(Locale.US, "%.3f", a.satisfaction)
        val inside = a.satisfaction >= a.target
        val badge = root.findViewById<TextView>(R.id.sat_badge)
        badge?.text = if (inside) "Dentro del objetivo" else "Por debajo"
        badge?.setTextColor(resources.getColor(if (inside) R.color.success else R.color.warning, null))
        root.findViewById<TextView>(R.id.sat_target)?.text = String.format(
            Locale.US, "Objetivo %.2f · planificador: %s", a.target, a.phase
        )
        val age = io.teyzen.iaoptimizer.core.StateCache.ageS("/api/agent")
        root.findViewById<TextView>(R.id.sat_updated)?.text =
            "Ciclo ${a.cycle} · actualizado hace ${if (age > 3600) "—" else "$age s"}"
    }

    /** [6] Lecturas en vivo con icono y contexto, divisores suaves. */
    private fun paintLive(a: AgentData?) {
        val box = binding.root.findViewById<LinearLayout>(R.id.live_box) ?: return
        box.removeAllViews()
        if (a == null) {
            liveRow(box, R.drawable.ic_cpu, "CPU cargar", "—", null)
            return
        }
        val tempMax = maxOf(a.socMax, a.batteryC)
        liveRow(box, R.drawable.ic_cpu, "CPU cargar", String.format(Locale.US, "%.0f %%", a.actuacion), null)
        liveRow(box, R.drawable.ic_thermo, "Temperatura", String.format(Locale.US, "%.1f °C", tempMax),
            "chip ${String.format(Locale.US, "%.1f", a.socMax)} °C · batería ${String.format(Locale.US, "%.1f", a.batteryC)} °C")
        liveRow(box, R.drawable.ic_battery, "Batería", "${a.batteryPct} %", if (a.charging) "Cargando" else "En uso")
        liveRow(box, R.drawable.ic_ram, "Memoria en uso", String.format(Locale.US, "%.1f %%", a.ramPct), null)
        val ctx = if (a.app == "-" || a.app.isEmpty()) "Ninguna app en primer plano" else a.app
        liveRow(box, R.drawable.ic_phone, "Contexto", ctx,
            "pantalla ${a.screen} · perfil ${profileLabel(a.profile)} · preajuste IA Motors")
    }

    private fun liveRow(parent: LinearLayout, iconRes: Int, title: String, value: String, sub: String?) {
        val density = resources.displayMetrics.density
        if (parent.childCount > 0) {
            val div = View(requireContext()).apply {
                setBackgroundColor(resources.getColor(R.color.outline, null))
                layoutParams = LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 1)
            }
            parent.addView(div)
        }
        val row = LinearLayout(requireContext()).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = android.view.Gravity.CENTER_VERTICAL
            setPadding(0, (10 * density).toInt(), 0, (10 * density).toInt())
        }
        val iv = android.widget.ImageView(requireContext()).apply {
            setImageResource(iconRes)
            setColorFilter(resources.getColor(R.color.primary, null))
            layoutParams = LinearLayout.LayoutParams((24 * density).toInt(), (24 * density).toInt())
        }
        row.addView(iv)
        val col = LinearLayout(requireContext()).apply {
            orientation = LinearLayout.VERTICAL
            layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f).apply {
                marginStart = (12 * density).toInt()
            }
        }
        val t = TextView(requireContext()).apply {
            text = title
            setTextColor(resources.getColor(R.color.text_dim, null))
            textSize = 11f
        }
        val v = TextView(requireContext()).apply {
            text = value
            setTextColor(resources.getColor(R.color.text, null))
            textSize = 14f
            typeface = android.graphics.Typeface.DEFAULT_BOLD
        }
        col.addView(t)
        col.addView(v)
        if (sub != null) {
            col.addView(TextView(requireContext()).apply {
                text = sub
                setTextColor(resources.getColor(R.color.text_dim, null))
                textSize = 11f
            })
        }
        row.addView(col)
        parent.addView(row)
    }

    /** [7] Presupuesto de calor: solo lo medido de verdad en este móvil. */
    private fun paintHeat(a: AgentData?) {
        val hint = binding.root.findViewById<TextView>(R.id.heat_hint) ?: return
        binding.root.findViewById<TextView>(R.id.heat_title)?.text = "Medido en este dispositivo"
        hint.text = when {
            a == null -> "Sin datos del motor."
            a.heatMeasured -> "Última sesión intensa: ${fmtDur(a.heatLastS)} hasta la primera caída de seguridad · mejor ${fmtDur(a.heatBestS)} · mediana ${fmtDur(a.heatMedianS)} · ${a.heatSessions} sesiones observadas"
            a.heatInSession -> "Sesión intensa en curso: midiendo el tiempo hasta la primera caída de seguridad…"
            else -> "Aún no se ha medido ninguna sesión pesada. Se medirá solo durante juego o carga sostenida."
        }
    }

    private fun fmtDur(s: Long): String =
        if (s >= 60) "${s / 60} min ${s % 60} s" else "$s s"

    /** [8][9] Selectores de objetivo y comportamiento (sincronizados sin eco). */
    private fun paintPrefs(a: AgentData?) {
        val root = binding.root
        val goal = a?.goal ?: "equilibrado"
        val behavior = a?.behavior ?: "equilibrado"
        syncingPrefs = true
        root.findViewById<com.google.android.material.button.MaterialButtonToggleGroup>(R.id.goal_group)?.check(
            when (goal) {
                "actuacion" -> R.id.btn_goal_actuacion
                "bateria" -> R.id.btn_goal_bateria
                else -> R.id.btn_goal_equilibrado
            }
        )
        root.findViewById<com.google.android.material.button.MaterialButtonToggleGroup>(R.id.behavior_group)?.check(
            when (behavior) {
                "calma" -> R.id.btn_behavior_calma
                "decisivo" -> R.id.btn_behavior_decisivo
                else -> R.id.btn_behavior_equilibrado
            }
        )
        syncingPrefs = false
        root.findViewById<TextView>(R.id.goal_desc)?.text =
            "Optimizando para: ${goalLabel(goal)} · modo activo: ${profileLabel(a?.profile ?: currentProfile)}. Tu preferencia decide los pesos."
        root.findViewById<TextView>(R.id.preset_subtitle)?.text =
            "Perfil activo: ${profileLabel(a?.profile ?: currentProfile)}"
    }

    private fun goalLabel(g: String): String = when (g) {
        "actuacion" -> "Actuación"
        "bateria" -> "Batería"
        else -> "Equilibrado"
    }

    private fun profileLabel(p: String): String = when (p) {
        "eco" -> "Eco"
        "performance" -> "Rendimiento"
        "balanced" -> "Balanceado"
        else -> p
    }

    /** [11] Seguridad térmica: gobernador + aplicación efectiva del tope. */
    private fun paintThermalSafety(a: AgentData?, thermalJson: String) {
        val root = binding.root
        val govHint = root.findViewById<TextView>(R.id.thermal_gov_hint) ?: return
        val badge = root.findViewById<TextView>(R.id.thermal_gov_badge)
        val capHint = root.findViewById<TextView>(R.id.thermal_cap_hint)
        if (a == null) {
            govHint.text = "Sin lecturas todavía"
            capHint?.text = "Sin lecturas todavía"
            return
        }
        govHint.text = String.format(
            Locale.US, "%s · %d intervenciones medidas · SoC %.1f °C · batería %.1f °C",
            a.governor, a.interventions, a.socMax, a.batteryC
        )
        val hot = a.socMax > 65 || a.batteryC > 42
        badge?.text = if (hot) "Limitando" else "Normal"
        badge?.setTextColor(resources.getColor(if (hot) R.color.warning else R.color.success, null))
        val intensity = try {
            JSONObject(thermalJson).optDouble("intensity", 0.0)
        } catch (e: Exception) { 0.0 }
        val cap = try {
            JSONObject(io.teyzen.iaoptimizer.core.StateCache.state()).optDouble("freq_cap", 1.0)
        } catch (e: Exception) { 1.0 }
        capHint?.text = buildString {
            if (intensity > 0.05) {
                append(String.format(Locale.US, "Tope aplicado al %.0f %% para no pasar del objetivo térmico.", intensity * 100))
            } else {
                append("Sin limitación activa: el tope se aplicará solo si hace falta.")
            }
            if (cap < 0.999) append(String.format(Locale.US, " Techo del usuario: %.0f %%.", cap * 100))
        }
    }

    /** [12] Última evidencia medida del diario. */
    private fun paintEvidence(a: AgentData?) {
        val root = binding.root
        val title = root.findViewById<TextView>(R.id.evidence_title) ?: return
        val e = a?.journal?.firstOrNull()
        if (e == null) {
            title.text = "Sin evidencia todavía"
            root.findViewById<TextView>(R.id.evidence_time)?.text = "—"
            root.findViewById<TextView>(R.id.evidence_kind)?.text = "—"
            root.findViewById<TextView>(R.id.evidence_body)?.text =
                "El motor registrará aquí su última decisión medida con antes y después."
            return
        }
        title.text = e.reason.ifEmpty { "Ciclo del motor" }
        root.findViewById<TextView>(R.id.evidence_time)?.text =
            SimpleDateFormat("HH:mm:ss", Locale.getDefault()).format(Date(e.t * 1000))
        root.findViewById<TextView>(R.id.evidence_kind)?.text =
            "${e.kind} · ${if (e.ok) "verificado" else "no sostenido"}"
        root.findViewById<TextView>(R.id.evidence_body)?.text = buildString {
            if (e.value.isNotEmpty() && e.value != "-") {
                append(Labels.choice("", e.path, e.value))
            }
            if (e.path.isNotEmpty() && e.path != "-") {
                if (isNotEmpty()) append(" · ")
                append(Labels.choice("", e.path, e.path.substringAfterLast('/')))
            }
            if (isNotEmpty()) append("\n")
            append(String.format(Locale.US, "Efecto medido: temperatura %+.2f °C · carga %+.1f %%", e.tempD, e.cpuD * 100))
            if (e.outcome.isNotEmpty() && !e.outcome.contains("temp")) append(" · ").append(e.outcome)
        }
    }

    /** [3] Árbol de capacidades: qué detecta, qué planea y qué gobierna. */
    private fun showCapabilities() {
        val a = agent
        val sb = StringBuilder()
        sb.append("Detectado en este dispositivo:\n")
        if (a != null && a.features.isNotEmpty()) {
            a.features.forEach { sb.append("  • ").append(it).append('\n') }
        } else sb.append("  • sin datos del daemon\n")
        sb.append("\nPlan del ciclo actual:\n")
        if (a != null && a.plan.isNotEmpty()) {
            a.plan.forEach { sb.append("  • ").append(it).append('\n') }
        } else sb.append("  • en espera\n")
        sb.append("\nQué gobierna:\n")
        sb.append("  • CPU: gobernador y frecuencias por clúster\n")
        sb.append("  • Almacenamiento: planificador de I/O\n")
        sb.append("  • Memoria: ZRAM\n")
        sb.append("  • Térmica: limitador con histéresis\n")
        sb.append("  • Energía: carga directa (solo si la activas)\n")
        sb.append("  • Pantalla: resolución y refresco\n\n")
        sb.append("Cada cambio se verifica con relectura y queda registrado en el Diario.")
        androidx.appcompat.app.AlertDialog.Builder(requireContext())
            .setTitle("¿Qué hace IA Motors?")
            .setMessage(sb.toString())
            .setPositiveButton("Cerrar", null)
            .show()
    }

    /** Gráfico del historial en vivo (carga de CPU ciclo a ciclo). */
    private fun paintChart(json: String) {
        val chart = binding.root.findViewById<io.teyzen.iaoptimizer.ui.SparklineView>(R.id.chart_history)
        val caption = binding.root.findViewById<TextView>(R.id.chart_caption)
        if (chart == null || caption == null) return
        try {
            val arr = org.json.JSONArray(json)
            if (arr.length() < 2) {
                chart.setData(FloatArray(0))
                caption.text = "—"
                return
            }
            val cpu = FloatArray(arr.length())
            var tmax = 0f
            for (i in 0 until arr.length()) {
                val o = arr.optJSONObject(i) ?: continue
                cpu[i] = (o.optDouble("cpu", 0.0) * 100).toFloat()
                tmax = maxOf(tmax, o.optDouble("temp", 0.0).toFloat())
            }
            chart.setData(cpu)
            var suma = 0.0
            var pico = 0.0
            for (v in cpu) {
                suma += v
                pico = maxOf(pico, v.toDouble())
            }
            caption.text = String.format(
                Locale.US,
                "carga media %.0f %% · pico %.0f %% · pico térmico %.1f °C",
                suma / cpu.size, pico, tmax
            )
        } catch (e: Exception) {
            chart.setData(FloatArray(0))
            caption.text = "—"
        }
    }

    /** Puntaje de satisfacción y barras de progreso de los objetivos. */
    private fun paintObjectives(a: AgentData?) {
        val box = binding.root.findViewById<LinearLayout>(R.id.objectives_box) ?: return
        box.removeAllViews()
        if (a == null) {
            row(box, "Objetivos", "sin datos")
            return
        }
        val score = layoutInflater.inflate(R.layout.item_row, box, false)
        score.findViewById<TextView>(R.id.rowTitle).text = "Satisfacción"
        score.findViewById<TextView>(R.id.rowValue).text = String.format(
            Locale.US, "%.3f / Objetivo %.3f", a.satisfaction, a.target
        )
        box.addView(score)
        // Los objetivos ya vienen 0..100 del daemon: no se multiplican otra vez.
        metric(box, "Actuación", a.actuacion.toInt().coerceIn(0, 100), R.color.primary)
        metric(box, "Batería", a.bateria.toInt().coerceIn(0, 100), R.color.secondary)
        metric(box, "Espacio térmico", a.espacio.toInt().coerceIn(0, 100), R.color.warning)
    }

    private fun metric(parent: LinearLayout, label: String, pct: Int, colorRes: Int) {
        val v = layoutInflater.inflate(R.layout.item_metric, parent, false)
        v.findViewById<TextView>(R.id.metricLabel).text = label
        v.findViewById<TextView>(R.id.metricValue).text = String.format(Locale.US, "%d %%", pct)
        val bar = v.findViewById<android.widget.ProgressBar>(R.id.metricBar)
        bar.progress = pct
        // El tinte va SOLO a la capa de progreso: si se tiñe el layer-list
        // entero, el fondo toma el mismo color y la barra parece siempre llena.
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

    /** Formatea un valor ya escalado con su unidad. */
    private fun fmtValue(scaled: Double, unitLabel: String): String = when {
        unitLabel == "MHz" -> "${scaled.toLong()} MHz"
        unitLabel == "ns" && scaled >= 1_000_000 ->
            String.format(Locale.US, "%.2f ms", scaled / 1_000_000.0)
        unitLabel == "ns" && scaled >= 1000 ->
            String.format(Locale.US, "%.2f µs", scaled / 1000.0)
        unitLabel == "ns" -> "${scaled.toLong()} ns"
        unitLabel == "%" -> "${scaled.toLong()} %"
        unitLabel.isBlank() -> scaled.toLong().toString()
        else -> String.format(Locale.US, "%.2f %s", scaled, unitLabel)
    }

    /** Escala legible para frecuencias: siempre MHz reales del hardware.
     *  Los nodos de CPU (cpufreq) vienen en kHz y los de GPU/devfreq en Hz;
     *  ambos se muestran en MHz. El divisor se decide por dominio, no por
     *  magnitud (2000000 kHz son 2000 MHz, no 2 MHz). */
    private fun scaleOf(k: Knob, info: Labels.Info): Pair<Long, String> {
        if (info.unit != "MHz" && info.unit != "GHz") return 1L to (info.unit ?: "")
        return if (k.domain == "cpu_freq") 1_000L to "MHz" else 1_000_000L to "MHz"
    }

    private fun applyKnob(path: String, value: String, onApplied: ((String) -> Unit)? = null) {
        if (aiActive) {
            Toast.makeText(requireContext(), "IA Motors está activa: apágala en Inicio para control manual", Toast.LENGTH_SHORT).show()
            return
        }
        thread {
            val r = bridge.setKnob(path, value)
            activity?.runOnUiThread {
                if (_binding == null) return@runOnUiThread
                val note = try {
                    JSONObject(r).optString("note", "")
                } catch (e: Exception) {
                    ""
                }
                Toast.makeText(
                    requireContext(),
                    if (r.isEmpty()) "sin conexión" else note.ifEmpty { "aplicado" },
                    Toast.LENGTH_SHORT
                ).show()
                if (r.contains("\"ok\":true")) {
                    val after = try {
                        JSONObject(r).optString("after", "")
                    } catch (e: Exception) {
                        ""
                    }
                    if (after.isNotEmpty()) onApplied?.invoke(after)
                }
            }
        }
    }

    internal fun knobRow(k: Knob): View {
        val info = Labels.knob(k.domain, k.path)
        val v = layoutInflater.inflate(R.layout.item_knob, binding.root as ViewGroup?, false)
        v.findViewById<TextView>(R.id.knobTitle).text = info.title
        v.findViewById<TextView>(R.id.knobHint).text = info.hint
        val value = v.findViewById<TextView>(R.id.knobValue)
        val slider = v.findViewById<Slider>(R.id.knobSlider)
        val choices = v.findViewById<com.google.android.material.button.MaterialButtonToggleGroup>(R.id.knobChoices)

        val (factor, unitLabel) = scaleOf(k, info)
        val range = if (k.min != null && k.max != null && k.max > k.min) k.min!!..k.max!! else null

        when {
            k.writable && range != null && info.numeric
                && (range.last - range.first) / factor > 0 -> {
                slider.visibility = View.VISIBLE
                slider.valueFrom = (range.first / factor).toFloat()
                slider.valueTo = (range.last / factor).toFloat()
                // Material Slider EXIGE que stepSize divida exactamente al
                // rango (valueTo-valueFrom): si no, crashea al dibujar.
                // En MHz se busca un paso bonito que divida el rango real del
                // cluster; en el resto, un paso entero cercano a rango/20.
                val spanInt = (range.last - range.first) / factor
                val step: Float = if (unitLabel == "MHz") {
                    listOf(10L, 5L, 2L, 1L).firstOrNull { spanInt % it == 0L }?.toFloat() ?: 1f
                } else {
                    var n = 20
                    while (n > 1 && spanInt % n != 0L) n--
                    (spanInt / n).toFloat().coerceAtLeast(1f)
                }
                slider.stepSize = step
                val cur = k.current.toLongOrNull() ?: range.first
                val raw = cur.toFloat() / factor
                // Alinear al paso SIN salirse del rango.
                val nSteps = Math.floor(((slider.valueTo - slider.valueFrom) / step).toDouble()).toLong()
                val n = Math.round(((raw - slider.valueFrom) / step).toDouble())
                    .coerceIn(0, nSteps)
                slider.value = slider.valueFrom + n * step
                value.text = fmtValue(slider.value.toDouble(), unitLabel)
                slider.addOnChangeListener { _, nv, fromUser ->
                    if (fromUser) value.text = fmtValue(nv.toDouble(), unitLabel)
                }
                slider.addOnSliderTouchListener(object : Slider.OnSliderTouchListener {
                    override fun onStartTrackingTouch(s: Slider) { control.interacting = true }
                    override fun onStopTrackingTouch(s: Slider) {
                        control.interacting = false
                        applyKnob(k.path, (s.value * factor).toLong().toString()) { after ->
                            val nv = after.toLongOrNull()
                            if (nv != null) value.text = fmtValue(nv.toDouble() / factor, unitLabel)
                        }
                    }
                })
            }
            k.writable && k.allowed.size > 1 -> {
                choices.visibility = View.VISIBLE
                value.text = displayValue(k, info, factor, unitLabel)
                k.allowed.take(6).forEach { opt ->
                    val b = MaterialButton(requireContext()).apply {
                        text = Labels.choice(k.domain, k.path, opt)
                        isAllCaps = false
                        textSize = 12f
                        minHeight = 0
                        insetTop = 0
                        insetBottom = 0
                        backgroundTintList = android.content.res.ColorStateList
                            .valueOf(resources.getColor(R.color.surface_variant, null))
                        setTextColor(resources.getColor(R.color.text, null))
                        layoutParams = LinearLayout.LayoutParams(
                            0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f
                        )
                        setOnClickListener { applyKnob(k.path, opt) }
                    }
                    choices.addView(b)
                }
                (0 until choices.childCount).forEach { i ->
                    val b = choices.getChildAt(i) as? MaterialButton ?: return@forEach
                    if (b.text.toString() == k.current) b.isChecked = true
                }
            }
            else -> {
                value.text = displayValue(k, info, factor, unitLabel)
            }
        }
        // Con IA Motors activa el control es de solo lectura: la IA decide.
        slider.isEnabled = !aiActive
        (0 until choices.childCount).forEach { i ->
            choices.getChildAt(i).isEnabled = !aiActive
        }
        return v
    }

    /** Valor en lenguaje claro: unidades legibles y sin volcados tecnicos. */
    private fun displayValue(k: Knob, info: Labels.Info, factor: Long, unitLabel: String): String {
        val cur = k.current.trim()
        if (cur.isEmpty() || cur == "-") return "—"
        val num = cur.toLongOrNull()
        if (num != null && info.numeric) return fmtValue(num.toDouble() / factor, unitLabel)
        val clean = cur.replace('\n', ' ').replace(Regex("\\s+"), " ").trim()
        if (clean.contains('=') || clean.contains(';')) {
            return if (clean.contains("-1")) "Sin límites activos" else "Con límites configurados"
        }
        Labels.choice(k.domain, k.path, clean).let { if (it != clean) return it }
        return if (clean.length > 48) clean.take(45) + "…" else clean
    }

    private fun paintAprendizaje(a: AgentData?) {
        val box = binding.root.findViewById<LinearLayout>(R.id.aprendBox) ?: return
        box.removeAllViews()
        if (a == null) {
            row(box, "Aprendizaje", "Sin datos")
            return
        }
        header(box, "Muestra")
        row(box, "Ventana", String.format(Locale.US, "%.1f h", a.windowHours))
        row(box, "Decisiones verificadas", a.verified.toString())
        row(box, "Decisiones medidas", a.measured.toString())
        row(box, "Anulaciones del usuario", a.overrides.toString())
        row(box, "Decisiones en muestra", a.sample.toString())
        val honest = a.sample > 0
        row(
            box,
            "Honestidad del aprendizaje",
            if (honest) {
                val pct = (a.honesty * 100).toInt()
                val verif = a.verified.coerceAtLeast(0)
                val tot = a.verified + a.failed
                "muestras verificadas $verif/$tot · tasa verificada $pct % · ${fmtWindow(a.windowHours)}"
            } else {
                "sin muestra: no hay decisiones verificadas todavía en esta ventana"
            }
        )
        val hint = TextView(requireContext()).apply {
            text = if (honest) {
                val dif = a.windowHours - 2f
                if (dif < 0) {
                    "Verificado con ${fmtWindow(a.windowHours)}; faltan ${fmtWindow(-dif)} para la ventana mínima de 2 h."
                } else {
                    "Verificado con ${fmtWindow(a.windowHours)}; no se apoya en la duración declarada."
                }
            } else {
                "Se muestra el dato solo cuando hay decisiones verificadas en la ventana."
            }
            setTextColor(resources.getColor(R.color.text_dim, null))
            textSize = 12f
            setPadding(14, 4, 14, 10)
        }
        box.addView(hint)

        if (a.weights.isNotEmpty()) {
            header(box, "Pesos térmicos (peso °C por muestra verificada)")
            a.weights.take(12).forEach {
                row(
                    box,
                    "${it.mode} · ${it.kind}",
                    String.format(Locale.US, "%+.2f °C  (%d muestras)", it.tempDelta, it.samples)
                )
            }
        }
        if (a.perApp.isNotEmpty()) {
            header(box, "Por app")
            a.perApp.take(10).forEach { row(box, it.app, "${it.verified} verificadas") }
        }
    }

    private fun fmtWindow(h: Float): String {
        return if (h >= 1f) String.format(Locale.US, "%.1f h", h)
        else String.format(Locale.US, "%d min", (h * 60).toInt().coerceAtLeast(1))
    }

    private fun setupTabs(names: List<String>) {
        val tabs = binding.mtTabs
        tabs.removeAllTabs()
        names.forEach { tabs.addTab(tabs.newTab().setText(it)) }
        tabs.addOnTabSelectedListener(object : com.google.android.material.tabs.TabLayout.OnTabSelectedListener {
            override fun onTabSelected(tab: com.google.android.material.tabs.TabLayout.Tab) {
                binding.panes.displayedChild = tab.position
                binding.motorsBadge.text = tab.text ?: "—"
            }
            override fun onTabUnselected(tab: com.google.android.material.tabs.TabLayout.Tab) {}
            override fun onTabReselected(tab: com.google.android.material.tabs.TabLayout.Tab) {}
        })
        tabs.getTabAt(0)?.select()
        // El badge refleja la primera solapa aunque TabLayout no dispare el
        // listener (la primera ya viene seleccionada por defecto).
        binding.motorsBadge.text = names.firstOrNull() ?: "—"
    }

    /**
     * Control térmico: la temperatura de control (batería + zona del chip), el
     * objetivo elegido y la intensidad de limitación. En lenguaje claro.
     */
    private fun paintTermico(box: LinearLayout, json: String) {
        try {
            val o = org.json.JSONObject(json)
            header(box, "Control térmico")
            row(box, "Temperatura de control", String.format(Locale.US, "%.1f °C", o.optDouble("control_c", 0.0)))
            row(box, "Batería / chip", String.format(Locale.US, "%.1f / %.1f °C",
                o.optDouble("battery_c", 0.0), o.optDouble("soc_c", 0.0)))
            row(box, "Objetivo", String.format(Locale.US, "%.1f °C", o.optDouble("target_c", 0.0)))
            val intensidad = o.optDouble("intensity", 0.0)
            row(box, "Limitación ahora", String.format(Locale.US, "%.0f %%", intensidad * 100))
            row(box, "Actuador", Labels.choice("", "", o.optString("actuator", "-")))
            row(box, "Intervenciones", o.optInt("interventions", 0).toString())
            if (intensidad < 0.05) {
                row(box, "Estado", "Sin limitación: va dentro del objetivo.")
            } else {
                row(box, "Estado", "Limitando para no pasar del objetivo.")
            }
        } catch (e: Exception) {
            header(box, "Control térmico")
            row(box, "Estado", "sin datos")
        }
    }

    /**
     * Panel de advertencias/excepciones del kernel. Cada aviso dice qué nodo no
     * se pudo usar y por qué, en lenguaje claro: el kernel restringe el
     * parámetro, no es un fallo de la app.
     */
    private fun paintAvisos(box: LinearLayout, warnsJson: String) {
        val warns = try {
            org.json.JSONObject(warnsJson).optJSONArray("warnings")
        } catch (e: Exception) {
            null
        }
        if (warns == null || warns.length() == 0) {
            header(box, "Avisos del kernel")
            row(box, "Estado", "Sin avisos: todos los controles son accesibles")
            return
        }
        header(box, "Avisos del kernel (${warns.length()})")
        for (i in 0 until warns.length()) {
            val w = warns.optJSONObject(i) ?: continue
            val card = layoutInflater.inflate(R.layout.item_warning, box, false)
            val path = w.optString("path")
            val leaf = path.substringAfterLast('/')
            // Dos clusters distintos pueden tener el mismo nombre de nodo; el
            // cluster los diferencia en el titulo.
            val cluster = Regex("policy\\d+").find(path)?.value
                ?.replace("policy", "clúster ") ?: ""
            val nombre = Labels.choice("", path, leaf) + if (cluster.isNotEmpty()) " ($cluster)" else ""
            card.findViewById<TextView>(R.id.warnTitle).text = "No se pudo usar $nombre"
            card.findViewById<TextView>(R.id.warnHint).text =
                warnText(w.optString("kind"), w.optString("detail"))
            box.addView(card)
        }
    }

    /** Explicacion amigable: el kernel restringe el nodo, no es un fallo de la app. */
    private fun warnText(kind: String, detail: String): String {
        val base = when (kind) {
            "no escribible" ->
                "El kernel de tu dispositivo no permite modificar este parámetro."
            "sin efecto" ->
                "El cambio no se sostuvo: el kernel lo rechazó y se restauró el valor anterior."
            else -> "El kernel no expone este parámetro en tu hardware."
        }
        return "$base La app funciona con normalidad; esta opción no está disponible en tu dispositivo. ($detail)"
    }

    private fun paintSistema(dev: String, rep: io.teyzen.iaoptimizer.core.RepairData?, auditJson: String) {
        val box = binding.root.findViewById<LinearLayout>(R.id.sistemaBox) ?: return
        box.removeAllViews()
        header(box, "Dispositivo")
        try {
            val o = org.json.JSONObject(dev)
            listOf(
                "kernel_version" to "Kernel",
                "device" to "Dispositivo",
                "soc" to "Chipset",
                "android_version" to "Android"
            ).forEach { (key, label) ->
                if (!o.isNull(key)) row(box, label, o.optString(key))
            }
            val feat = o.optJSONArray("features")
            if (feat != null && feat.length() > 0) {
                row(box, "Compatibilidades", (0 until feat.length()).joinToString(", ") { feat.optString(it) })
            }
        } catch (e: Exception) {
            row(box, "Dispositivo", dev.ifEmpty { "sin datos" })
        }
        header(box, "Diagnóstico")
        if (rep == null || rep.findings.isEmpty()) {
            row(box, "Diagnóstico", "Sin hallazgos")
        } else {
            rep.findings.forEach {
                val color = resources.getColor(if (it.ok) R.color.success else R.color.warning, null)
                val v = layoutInflater.inflate(R.layout.item_row, box, false)
                v.findViewById<TextView>(R.id.rowTitle).text = it.name
                v.findViewById<TextView>(R.id.rowTitle).setTextColor(color)
                v.findViewById<TextView>(R.id.rowValue).text =
                    buildString {
                        append(if (it.ok) "correcto · " else "revisar · ")
                        append(it.detail)
                        it.fix?.let { f -> append(" · acción: ").append(f) }
                    }
                box.addView(v)
            }
        }
        if (rep != null && rep.blacklist.isNotEmpty()) {
            header(box, "Excluidos por el usuario")
            rep.blacklist.forEach { row(box, "excluido", it) }
        }
        // Autovigilancia (antes en la solapa Avisos, ahora junto al sistema).
        try {
            val o = JSONObject(auditJson)
            header(box, "Autovigilancia")
            row(box, "Ciclo de auditoría", o.optInt("cycle", 0).toString())
            row(box, "Controles escribibles", o.optInt("writable", 0).toString())
            row(box, "Controles de solo lectura", o.optInt("read_only", 0).toString())
            val healed = o.optJSONArray("healed")
            if (healed != null && healed.length() > 0) {
                header(box, "Correcciones automáticas")
                for (i in 0 until healed.length()) row(box, "Corregido", healed.optString(i))
            }
            val failures = o.optJSONArray("failures")
            if (failures != null && failures.length() > 0) {
                header(box, "Nodos caídos")
                for (i in 0 until failures.length()) row(box, "Caído", failures.optString(i))
            }
            val errors = o.optJSONArray("errors")
            if (errors != null && errors.length() > 0) {
                header(box, "Errores sin resolver")
                for (i in 0 until errors.length()) {
                    val e = errors.optJSONObject(i) ?: continue
                    row(box, "Error", Labels.choice("", "", e.optString("kind")) + " · " + e.optString("path").substringAfterLast('/'))
                }
            }
            val bl = o.optJSONArray("blacklist")
            if (bl != null && bl.length() > 0) {
                header(box, "Rutas aisladas")
                for (i in 0 until bl.length()) row(box, "Aislada", bl.optString(i).substringAfterLast('/'))
            }
        } catch (e: Exception) {
            header(box, "Autovigilancia")
            row(box, "Estado", "sin datos")
        }
        val heal = com.google.android.material.button.MaterialButton(
            requireContext(), null,
            com.google.android.material.R.attr.materialButtonOutlinedStyle
        ).apply {
            text = "Reparar ahora"
            isAllCaps = false
            setOnClickListener {
                thread {
                    bridge.runAudit()
                    activity?.runOnUiThread {
                        if (_binding == null) return@runOnUiThread
                        Toast.makeText(requireContext(), "Auditoría ejecutada", Toast.LENGTH_SHORT).show()
                        refresh()
                    }
                }
            }
        }
        box.addView(heal)
    }

    /** Pantalla: refresco, renderizador y resolución (Control → Mostrar). */
    internal fun paintPantalla(json: String) {
        val d = io.teyzen.iaoptimizer.core.DisplayParser.parse(json) ?: return
        val root = binding.root
        root.findViewById<TextView>(R.id.displayRefresh)?.text =
            String.format(Locale.US, "%.1f Hz", d.refreshHz)

        // Frecuencia de actualización: elegible incluso con IA Motors activa.
        val refresh = root.findViewById<com.google.android.material.textfield.MaterialAutoCompleteTextView>(R.id.refreshValue)
        val hzOptions = listOf(0 to "Automática", 60 to "60 Hz", 90 to "90 Hz", 120 to "120 Hz")
        refresh?.setSimpleItems(hzOptions.map { it.second }.toTypedArray())
        refresh?.setText("Automática", false)
        refresh?.setOnItemClickListener { _, _, pos, _ ->
            if (pos in hzOptions.indices) {
                val v = hzOptions[pos].first
                thread {
                    val r = bridge.setRefresh(v)
                    activity?.runOnUiThread {
                        if (_binding == null) return@runOnUiThread
                        val note = try { JSONObject(r).optString("detail", "") } catch (e: Exception) { "" }
                        Toast.makeText(requireContext(), note.ifEmpty { "sin conexión" }, Toast.LENGTH_SHORT).show()
                    }
                }
            }
        }

        val renderer = root.findViewById<com.google.android.material.textfield.MaterialAutoCompleteTextView>(R.id.rendererValue)
        val renderers = d.rendererOptions.filter { it.isNotBlank() }.ifEmpty { listOf(d.renderer) }
        renderer?.setSimpleItems(renderers.toTypedArray())
        renderer?.setText(d.renderer, false)
        lastRenderer = d.renderer
        renderer?.setOnItemClickListener { _, _, pos, _ ->
            if (pos in renderers.indices) {
                lastRenderer = renderers[pos]
                thread { bridge.setRenderer(lastRenderer) }
            }
        }

        val resolution = root.findViewById<com.google.android.material.textfield.MaterialAutoCompleteTextView>(R.id.resolutionValue)
        val options = listOf(
            "default" to "Original",
            "720x1600" to "720 × 1600",
            "1080x2400" to "1080 × 2400"
        )
        resolution?.setSimpleItems(options.map { it.second }.toTypedArray())
        resolution?.setText(d.overrideSize.ifEmpty { "Original" }, false)
        lastResolution = if (d.overrideSize.isEmpty()) "default" else d.overrideSize
        resolution?.setOnItemClickListener { _, _, pos, _ ->
            if (pos in options.indices) {
                lastResolution = options[pos].first
                thread { bridge.setResolution(lastResolution) }
            }
        }
    }

    private fun bullet(parent: LinearLayout, text: String) {
        val tv = TextView(requireContext()).apply {
            this.text = "•  $text"
            setTextColor(resources.getColor(R.color.text, null))
            textSize = 14f
            setPadding(
                resources.getDimensionPixelSize(R.dimen.space_lg),
                resources.getDimensionPixelSize(R.dimen.space_xs),
                resources.getDimensionPixelSize(R.dimen.space_lg),
                resources.getDimensionPixelSize(R.dimen.space_xs)
            )
        }
        parent.addView(tv)
    }

    private fun header(parent: LinearLayout, text: String) {
        val tv = TextView(requireContext()).apply {
            this.text = text
            setTextColor(resources.getColor(R.color.primary, null))
            textSize = 12f
            letterSpacing = 0.08f
            setPadding(14, 16, 14, 6)
        }
        parent.addView(tv)
    }

    private fun row(parent: LinearLayout, title: String, value: String) {
        val v = layoutInflater.inflate(R.layout.item_row, parent, false)
        v.findViewById<TextView>(R.id.rowTitle).text = title
        v.findViewById<TextView>(R.id.rowValue).text = value
        parent.addView(v)
    }

    override fun onDestroyView() {
        super.onDestroyView()
        _binding = null
    }

    private inner class JournalAdapter : RecyclerView.Adapter<JournalAdapter.VH>() {
        private val items = mutableListOf<io.teyzen.iaoptimizer.core.JournalEntry>()
        private val fmt = SimpleDateFormat("dd/MM HH:mm:ss", Locale.getDefault())

        fun submit(list: List<io.teyzen.iaoptimizer.core.JournalEntry>) {
            items.clear()
            items.addAll(list)
            notifyDataSetChanged()
        }

        override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): VH {
            val v = layoutInflater.inflate(R.layout.item_journal, parent, false)
            return VH(v)
        }

        override fun getItemCount() = items.size

        override fun onBindViewHolder(holder: VH, position: Int) {
            val e = items[position]
            val title = buildString {
                append(fmt.format(Date(e.t * 1000)))
                append("  ·  ").append(e.kind)
                if (e.profile.isNotBlank()) append("  ·  ").append(e.profile)
            }
            val body = buildString {
                append(e.reason)
                if (e.value.isNotBlank()) append(" · ").append(e.value)
                if (e.path.isNotBlank()) append(" · ").append(e.path.substringAfterLast('/'))
                if (e.outcome.isNotBlank()) {
                    if (!e.outcome.contains("temp")) append(" · ").append(e.outcome)
                }
            }
            holder.itemView.findViewById<TextView>(R.id.jTitle).text = title
            holder.itemView.findViewById<TextView>(R.id.jBody).text = body
        }

        inner class VH(v: View) : RecyclerView.ViewHolder(v)
    }
}
