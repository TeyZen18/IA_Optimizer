package io.teyzen.iaoptimizer.ui

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.Path
import android.graphics.Shader
import android.util.AttributeSet
import android.view.View

/**
 * Gráfico de línea ligero para el historial en vivo.
 *
 * Se dibuja con Canvas (sin dependencias) porque el historial cabe en memoria
 * y cambia cada pocos segundos: no hace falta una librería de gráficos.
 * Acepta valores 0..1 ya normalizados; el llamador decide qué series pinta.
 */
class SparklineView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyle: Int = 0
) : View(context, attrs, defStyle) {

    private val line = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE
        strokeWidth = 4f
        strokeCap = Paint.Cap.ROUND
        strokeJoin = Paint.Join.ROUND
    }
    private val fill = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.FILL }
    private val grid = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE
        strokeWidth = 1f
        color = Color.parseColor("#14FFFFFF")
    }

    private var values = FloatArray(0)
    private var lineColor = Color.parseColor("#34D399")
    // Segunda serie opcional (p. ej. predicho vs real): solo línea, sin relleno.
    private var values2: FloatArray? = null
    private var color2 = Color.parseColor("#22D3EE")

    fun setColor(color: Int) {
        lineColor = color
        invalidate()
    }

    /** `data` en orden cronológico; se normaliza solo al dibujar. */
    fun setData(data: FloatArray) {
        values = data
        values2 = null
        invalidate()
    }

    /** Dos series comparadas con la misma escala (orden cronológico). */
    fun setDualData(a: FloatArray, b: FloatArray, secondColor: Int = color2) {
        values = a
        values2 = b
        color2 = secondColor
        invalidate()
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val w = width.toFloat()
        val h = height.toFloat()
        if (w <= 0f || h <= 0f) return

        // Rejilla de referencia: tres líneas horizontales muy tenues.
        for (i in 1..3) {
            val y = h * i / 4f
            canvas.drawLine(0f, y, w, y, grid)
        }

        if (values.size < 2) {
            val p = Paint(Paint.ANTI_ALIAS_FLAG).apply {
                color = lineColor
                textSize = h * 0.32f
                textAlign = Paint.Align.CENTER
            }
            canvas.drawText("—", w / 2f, h * 0.62f, p)
            return
        }

        var min = Float.MAX_VALUE
        var max = -Float.MAX_VALUE
        for (v in values) {
            if (v < min) min = v
            if (v > max) max = v
        }
        values2?.let { s2 ->
            for (v in s2) {
                if (v < min) min = v
                if (v > max) max = v
            }
        }
        val span = (max - min).takeIf { it > 1e-3f } ?: 1f
        // Margen interno para que la línea no corte el borde.
        val padV = h * 0.12f

        val stepX = w / (values.size - 1)
        val path = Path()
        val pts = ArrayList<Float>(values.size * 2)
        for (i in values.indices) {
            val x = i * stepX
            val y = h - padV - ((values[i] - min) / span) * (h - 2 * padV)
            pts.add(x)
            pts.add(y)
            if (i == 0) path.moveTo(x, y) else path.lineTo(x, y)
        }

        // Relleno con degradado bajo la línea.
        val fillPath = Path(path)
        fillPath.lineTo(w, h)
        fillPath.lineTo(0f, h)
        fillPath.close()
        fill.shader = LinearGradient(
            0f, 0f, 0f, h,
            (lineColor and 0x00FFFFFF) or 0x55000000,
            (lineColor and 0x00FFFFFF) or 0x00000000,
            Shader.TileMode.CLAMP
        )
        canvas.drawPath(fillPath, fill)

        line.color = lineColor
        canvas.drawPath(path, line)

        // Segunda serie: línea fina comparativa (predicho vs real, etc.).
        values2?.let { s2 ->
            if (s2.size >= 2) {
                val stepX2 = w / (s2.size - 1)
                val p2 = Path()
                var lx = 0f; var ly = 0f
                for (i in s2.indices) {
                    val x = i * stepX2
                    val y = h - padV - ((s2[i] - min) / span) * (h - 2 * padV)
                    if (i == 0) p2.moveTo(x, y) else p2.lineTo(x, y)
                    lx = x; ly = y
                }
                val paint2 = Paint(line).apply {
                    color = color2
                    strokeWidth = 3f
                    pathEffect = android.graphics.DashPathEffect(floatArrayOf(10f, 6f), 0f)
                }
                canvas.drawPath(p2, paint2)
                paint2.pathEffect = null
                canvas.drawCircle(lx, ly, 4f, paint2)
            }
        }

        // Punto final: deja claro que es "ahora".
        val lastX = pts[pts.size - 2]
        val lastY = pts[pts.size - 1]
        canvas.drawCircle(lastX, lastY, line.strokeWidth * 0.9f, line)
    }
}
