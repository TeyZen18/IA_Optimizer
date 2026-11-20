package io.teyzen.iaoptimizer.ui

import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.View
import android.view.ViewGroup
import android.widget.GridLayout
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import io.teyzen.iaoptimizer.R
import io.teyzen.iaoptimizer.core.RustBridge
import io.teyzen.iaoptimizer.databinding.ActivityCommandCenterBinding
import org.json.JSONObject
import java.util.Locale
import kotlin.concurrent.thread

/**
 * Comandos en Línea: panel profesional del motor en ejecución.
 *
 * Todo en vivo (sondeo cada 2 s): vitales, satisfacción con tendencia,
 * predicción térmica predicho-vs-real, convergencia del error, matriz de
 * conocimiento por clúster, puerta de exploración, plan de control,
 * auditoría de bucles y propiedad de los límites. Los gráficos se animan
 * con cada muestra nueva; nada es estático ni decorativo.
 */
class CommandCenterActivity : AppCompatActivity() {

    private lateinit var b: ActivityCommandCenterBinding
    private val handler = Handler(Looper.getMainLooper())
    private val satHistory = ArrayDeque<Float>()
    private var lastPoll = 0L
    private var polls = 0

    // Tarjetas de vitales y auditoría: se crean una vez y se actualizan en el
    // sitio (sin reconstruir, que provocaría saltos de scroll).
    private val vitalViews = mutableMapOf<String, Pair<TextView, ProgressBar>>()
    private val auditViews = mutableMapOf<String, TextView>()

    private val ticker = object : Runnable {
        override fun run() {
            thread {
                val cc = RustBridge.commandCenterJson()
                val agent = RustBridge.agentJson()
                runOnUiThread { paint(cc, agent) }
            }
            handler.postDelayed(this, 2000)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        b = ActivityCommandCenterBinding.inflate(layoutInflater)
        setContentView(b.root)
        b.ccBack.setOnClickListener { finish() }
        buildVitals()
        buildAudit()
        pulse()
    }

    override fun onResume() {
        super.onResume()
        handler.removeCallbacks(ticker)
        handler.post(ticker)
    }

    override fun onPause() {
        super.onPause()
        handler.removeCallbacks(ticker)
    }

    /** Latido del punto EN VIVO. */
    private fun pulse() {
        b.ccLiveDot.animate().alpha(0.3f).setDuration(650).withEndAction {
            if (!isFinishing) {
                b.ccLiveDot.animate().alpha(1f).setDuration(650).withEndAction { pulse() }.start()
            }
        }.start()
    }

    private fun dp(v: Int): Int = (v * resources.displayMetrics.density).toInt()

    private fun buildVitals() {
        listOf("cpu" to "CPU", "ram" to "RAM", "battery" to "Batería", "temp" to "Temperatura").forEach { (key, label) ->
            val card = LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                setBackgroundResource(R.drawable.bg_card)
                setPadding(dp(14), dp(12), dp(14), dp(12))
            }
            val l = TextView(this).apply {
                text = label
                setTextColor(getColor(R.color.text_dim))
                textSize = 11f
            }
            val v = TextView(this).apply {
                text = "—"
                setTextColor(getColor(R.color.text))
                textSize = 22f
                typeface = android.graphics.Typeface.MONOSPACE
            }
            val bar = ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal).apply {
                max = 100
                progress = 0
                progressDrawable = getDrawable(R.drawable.bar_metric)?.mutate()
                layoutParams = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(6)).apply {
                    topMargin = dp(6)
                }
            }
            card.addView(l)
            card.addView(v)
            card.addView(bar)
            val lp = GridLayout.LayoutParams().apply {
                width = 0
                height = ViewGroup.LayoutParams.WRAP_CONTENT
                columnSpec = GridLayout.spec(GridLayout.UNDEFINED, 1, 1f)
                setMargins(dp(4), dp(4), dp(4), dp(4))
            }
            b.ccVitals.addView(card, lp)
            vitalViews[key] = v to bar
        }
    }

    private fun buildAudit() {
        listOf(
            "mejorado" to "Mejorado", "regresado" to "Regresado",
            "anulaciones" to "Anulaciones", "sondas" to "Sondas"
        ).forEach { (key, label) ->
            val card = LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                setBackgroundResource(R.drawable.bg_card)
                setPadding(dp(14), dp(10), dp(14), dp(10))
            }
            val v = TextView(this).apply {
                text = "0"
                setTextColor(getColor(R.color.primary))
                textSize = 20f
                typeface = android.graphics.Typeface.MONOSPACE
            }
            val l = TextView(this).apply {
                text = label
                setTextColor(getColor(R.color.text_dim))
                textSize = 11f
            }
            card.addView(v)
            card.addView(l)
            val lp = GridLayout.LayoutParams().apply {
                width = 0
                height = ViewGroup.LayoutParams.WRAP_CONTENT
                columnSpec = GridLayout.spec(GridLayout.UNDEFINED, 1, 1f)
                setMargins(dp(4), dp(4), dp(4), dp(4))
            }
            b.ccAudit.addView(card, lp)
            auditViews[key] = v
        }
    }

    private fun paint(ccJson: String, agentJson: String) {
        val o = try { JSONObject(ccJson) } catch (e: Exception) {
            b.ccFooter.text = "Sin conexión con el daemon (¿módulo activo?)"
            return
        }
        val agent = try { JSONObject(agentJson) } catch (e: Exception) { JSONObject() }
        polls++
        lastPoll = System.currentTimeMillis()

        // Vitales (con barras reales, no decorativas).
        val v = o.optJSONObject("vitals") ?: JSONObject()
        val cpu = v.optDouble("cpu", 0.0)
        val ram = v.optDouble("ram", 0.0)
        val bat = v.optDouble("battery", 0.0)
        val temp = v.optDouble("temp_c", 0.0)
        setVital("cpu", String.format(Locale.US, "%.0f %%", cpu), cpu.toInt())
        setVital("ram", String.format(Locale.US, "%.0f %%", ram), ram.toInt())
        setVital("battery", String.format(Locale.US, "%.0f %%", bat), bat.toInt())
        // Temperatura: barra sobre 90 °C como referencia visual.
        setVital("temp", String.format(Locale.US, "%.1f °C", temp), (temp / 90 * 100).toInt())

        // Satisfacción + tendencia viva.
        val sat = o.optDouble("satisfaction", 0.0)
        val target = agent.optDouble("target", 0.75)
        satHistory.addLast(sat.toFloat())
        while (satHistory.size > 90) satHistory.removeFirst()
        b.ccSatValue.text = String.format(Locale.US, "%.3f", sat)
        b.ccSatTarget.text = String.format(Locale.US, "Objetivo %.2f · %s", target,
            agent.optJSONObject("app_context")?.optString("profile", "—") ?: "—")
        val inside = sat >= target
        b.ccSatBadge.text = if (inside) "Dentro del objetivo" else "Por debajo"
        b.ccSatBadge.setTextColor(getColor(if (inside) R.color.success else R.color.warning))
        b.ccSparkSat.setData(satHistory.toFloatArray())

        // Predicción térmica: real vs predicho (décimas de °C en la API).
        val pairs = o.optJSONArray("thermal_prediction")
        if (pairs != null && pairs.length() >= 2) {
            val real = FloatArray(pairs.length())
            val pred = FloatArray(pairs.length())
            for (i in 0 until pairs.length()) {
                val p = pairs.optJSONObject(i) ?: continue
                real[i] = (p.optInt("actual") / 10f)
                pred[i] = (p.optInt("predicted") / 10f)
            }
            b.ccSparkThermal.setDualData(real, pred, getColor(R.color.secondary))
        }
        val es = o.optJSONObject("error_summary") ?: JSONObject()
        b.ccPredVerdict.text = "Veredicto del motor: ${es.optString("verdict", "—")} · ${es.optInt("loops", 0)} bucles medidos"
        b.ccPredError.text = String.format(Locale.US, "error %.3f → %.3f",
            es.optDouble("first", 0.0), es.optDouble("last", 0.0))

        // Convergencia del error.
        val errs = o.optJSONArray("prediction_error")
        if (errs != null && errs.length() >= 2) {
            val ea = FloatArray(errs.length())
            for (i in 0 until errs.length()) ea[i] = errs.optDouble(i).toFloat()
            b.ccSparkError.setData(ea)
            b.ccSparkError.setColor(getColor(R.color.warning))
        }
        b.ccErrorText.text = if (es.optDouble("last", 1.0) < es.optDouble("first", 0.0)) {
            "El error baja: el modelo se está calibrando con este dispositivo."
        } else {
            "El error aún no cae: el motor sigue midiendo antes de confiar."
        }

        // Matriz de conocimiento por clúster.
        val k = o.optJSONObject("knowledge") ?: JSONObject()
        val cards = k.optJSONArray("cards")
        b.ccKnowledge.removeAllViews()
        if (cards != null) {
            for (i in 0 until cards.length()) {
                val c = cards.optJSONObject(i) ?: continue
                val row = layoutInflater.inflate(R.layout.item_row, b.ccKnowledge, false)
                val state = c.optString("state", "—")
                row.findViewById<TextView>(R.id.rowTitle).text =
                    "Clúster ${c.optString("policy").removePrefix("policy")} · ${c.optInt("samples")} muestras"
                row.findViewById<TextView>(R.id.rowValue).text =
                    "$state · incertidumbre ${String.format(Locale.US, "%.3f", c.optDouble("epistemic", 0.0))}"
                row.findViewById<TextView>(R.id.rowTitle).setTextColor(
                    getColor(if (state == "Util conocido") R.color.success else R.color.warning)
                )
                b.ccKnowledge.addView(row)
            }
        }

        // Exploración + plan.
        val gate = o.optJSONObject("exploration_gate") ?: JSONObject()
        b.ccGate.text = "${gate.optString("text", "—")} · sondas usadas: ${gate.optInt("probes_used", 0)}"
        val plan = o.optJSONObject("control_plan") ?: JSONObject()
        b.ccPlan.text = "Siguiente acción: ${plan.optString("next_action", "—")} · reversible: sí"
        val cov = plan.optDouble("coverage_pct", 0.0).toInt().coerceIn(0, 100)
        b.ccCoverageBar.progress = cov
        b.ccCoverageText.text = "Cobertura verificada: $cov %"

        // Auditoría.
        val audit = o.optJSONObject("loop_audit") ?: JSONObject()
        auditViews["mejorado"]?.text = audit.optInt("mejorado", 0).toString()
        auditViews["regresado"]?.text = audit.optInt("regresado", 0).toString()
        auditViews["anulaciones"]?.text = audit.optInt("anulaciones", 0).toString()
        auditViews["sondas"]?.text = audit.optInt("sondas", 0).toString()

        // Propiedad de los límites (min/max por clúster, en MHz).
        val own = o.optJSONArray("ownership")
        b.ccOwnership.removeAllViews()
        if (own != null) {
            for (i in 0 until own.length()) {
                val w = own.optJSONObject(i) ?: continue
                val node = w.optString("node")
                val cluster = node.substringAfterLast(":")
                val intent = w.optString("intent")
                val min = intent.substringBefore(":").toLongOrNull() ?: 0
                val max = intent.substringAfter(":").toLongOrNull() ?: 0
                val row = layoutInflater.inflate(R.layout.item_row, b.ccOwnership, false)
                row.findViewById<TextView>(R.id.rowTitle).text =
                    "Límites de CPU · clúster ${cluster.removePrefix("policy")}"
                row.findViewById<TextView>(R.id.rowValue).text =
                    "${min / 1000}–${max / 1000} MHz · IA Motors · ${w.optString("state", "—")}"
                b.ccOwnership.addView(row)
            }
        }

        val cycle = agent.optInt("cycle", 0)
        b.ccFooter.text = "Ciclo $cycle · en vivo · sondeo cada 2 s · muestra $polls"
    }

    private fun setVital(key: String, text: String, pct: Int) {
        val (tv, bar) = vitalViews[key] ?: return
        tv.text = text
        bar.progress = pct.coerceIn(0, 100)
    }
}
