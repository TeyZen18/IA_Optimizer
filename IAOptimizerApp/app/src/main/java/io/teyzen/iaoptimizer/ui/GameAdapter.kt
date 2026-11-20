package io.teyzen.iaoptimizer.ui

import android.view.LayoutInflater
import android.view.ViewGroup
import androidx.recyclerview.widget.RecyclerView
import io.teyzen.iaoptimizer.core.AppInfo
import io.teyzen.iaoptimizer.core.GameStore
import io.teyzen.iaoptimizer.databinding.ItemGameBinding

/** Tarjeta interactiva de juego: perfil propio, HUD y inicio rapido. */
class GameAdapter(
    private val onLaunch: (AppInfo) -> Unit,
    private val onProfile: (AppInfo, String) -> Unit,
    private val onHud: (AppInfo, Boolean) -> Unit
) : RecyclerView.Adapter<GameAdapter.VH>() {

    private val items = mutableListOf<AppInfo>()
    private val profiles = listOf(
        "auto" to "Automático",
        "eco" to "Eco",
        "balanced" to "Balanceado",
        "performance" to "Rendimiento"
    )

    fun submit(list: List<AppInfo>) {
        items.clear()
        items.addAll(list)
        notifyDataSetChanged()
    }

    override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): VH {
        val b = ItemGameBinding.inflate(LayoutInflater.from(parent.context), parent, false)
        return VH(b)
    }

    override fun getItemCount() = items.size

    override fun onBindViewHolder(holder: VH, position: Int) {
        val app = items[position]
        val ctx = holder.b.root.context
        holder.b.name.text = app.label
        holder.b.packageName.text = app.packageName
        AppIconLoader.load(ctx, app.packageName)?.let { holder.b.icon.setImageDrawable(it) }

        val current = GameStore.profile(ctx, app.packageName)
        holder.b.profileValue.setSimpleItems(profiles.map { it.second }.toTypedArray())
        holder.b.profileValue.setText(profiles.firstOrNull { it.first == current }?.second ?: "Automático", false)
        holder.b.profileValue.setOnItemClickListener { _, _, pos, _ ->
            if (pos in profiles.indices) {
                GameStore.setProfile(ctx, app.packageName, profiles[pos].first)
                onProfile(app, profiles[pos].first)
            }
        }

        holder.b.hudSwitch.setOnCheckedChangeListener(null)
        holder.b.hudSwitch.isChecked = GameStore.setting(ctx, app.packageName, "hud") == "1"
        holder.b.hudSwitch.setOnCheckedChangeListener { _, on ->
            GameStore.setting(ctx, app.packageName, "hud", if (on) "1" else "0")
            onHud(app, on)
        }

        holder.b.btnLaunch.setOnClickListener { onLaunch(app) }
    }

    class VH(val b: ItemGameBinding) : RecyclerView.ViewHolder(b.root)
}
