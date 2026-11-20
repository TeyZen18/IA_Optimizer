package io.teyzen.iaoptimizer.ui

import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import androidx.recyclerview.widget.RecyclerView
import io.teyzen.iaoptimizer.R
import io.teyzen.iaoptimizer.core.AppInfo
import io.teyzen.iaoptimizer.databinding.ItemAppBinding

/** Adapter de apps con icono real cacheado. */
class AppAdapter(
    private val onClick: (AppInfo) -> Unit
) : RecyclerView.Adapter<AppAdapter.VH>() {

    private val items = mutableListOf<AppInfo>()

    fun submit(list: List<AppInfo>) {
        items.clear()
        items.addAll(list)
        notifyDataSetChanged()
    }

    override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): VH {
        val binding = ItemAppBinding.inflate(
            LayoutInflater.from(parent.context), parent, false
        )
        return VH(binding)
    }

    override fun getItemCount(): Int = items.size

    override fun onBindViewHolder(holder: VH, position: Int) {
        val app = items[position]
        holder.binding.name.text = app.label
        holder.binding.pkg.text = app.packageName +
            (if (app.sizeMb > 0) " · ${app.sizeMb} MB" else "") +
            (if (app.isSystem) " · sistema" else "")
        val icon = AppIconLoader.load(holder.binding.root.context, app.packageName)
        if (icon != null) holder.binding.icon.setImageDrawable(icon)
        holder.binding.badgeGame.visibility = if (app.isGame) View.VISIBLE else View.GONE
        holder.binding.root.setOnClickListener { onClick(app) }
    }

    class VH(val binding: ItemAppBinding) : RecyclerView.ViewHolder(binding.root)
}
