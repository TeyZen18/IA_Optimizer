package io.teyzen.iaoptimizer.ui

import android.content.Intent
import android.os.Bundle
import android.text.Editable
import android.text.TextWatcher
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import androidx.fragment.app.Fragment
import androidx.recyclerview.widget.LinearLayoutManager
import io.teyzen.iaoptimizer.R
import io.teyzen.iaoptimizer.core.AppInfo
import io.teyzen.iaoptimizer.core.AppRepository
import io.teyzen.iaoptimizer.databinding.FragmentAppsBinding

/** Lista completa de apps (PackageManager) con busqueda y filtros. */
class AppsFragment : Fragment(), io.teyzen.iaoptimizer.VisibleTab {

    private var _binding: FragmentAppsBinding? = null
    private val binding get() = _binding!!
    private val adapter = AppAdapter { openDetail(it) }
    private var filter = AppRepository.Filter.ALL
    private var query = ""

    override fun onCreateView(
        inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?
    ): View {
        _binding = FragmentAppsBinding.inflate(inflater, container, false)
        return binding.root
    }

    override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
        binding.recycler.layoutManager = LinearLayoutManager(requireContext())
        binding.recycler.adapter = adapter

        binding.filters.setOnCheckedChangeListener { _, checkedId ->
            filter = when (checkedId) {
                R.id.chip_user -> AppRepository.Filter.USER
                R.id.chip_system -> AppRepository.Filter.SYSTEM
                R.id.chip_games -> AppRepository.Filter.GAMES
                else -> AppRepository.Filter.ALL
            }
            reload()
        }
        binding.search.addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) { }
            override fun onTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) {
                query = s?.toString() ?: ""
                reload()
            }
            override fun afterTextChanged(s: Editable?) { }
        })
    }

    override fun onResume() {
        super.onResume()
        reload()
    }

    override fun onTabVisible() {
        reload()
    }

    private var reloadToken = 0

    private fun reload() {
        if (_binding == null) return
        val ctx = context ?: return
        val token = ++reloadToken
        Thread {
            val all = AppRepository.list(ctx, filter, force = true)
            val q = query.trim().lowercase()
            val list = if (q.isEmpty()) all else all.filter {
                it.label.lowercase().contains(q) || it.packageName.lowercase().contains(q)
            }
            view?.post {
                if (_binding == null || token != reloadToken) return@post
                adapter.submit(list)
                binding.empty.visibility = if (list.isEmpty()) View.VISIBLE else View.GONE
            }
            // Precalienta iconos para que el scroll no se trabe.
            Thread {
                for (app in list) {
                    AppRepository.icon(ctx, app.packageName)
                }
            }.apply { isDaemon = true }.start()
        }.apply { isDaemon = true }.start()
    }

    private fun openDetail(app: AppInfo) {
        val intent = Intent(requireContext(), AppDetailActivity::class.java)
        intent.putExtra(AppDetailActivity.EXTRA_PACKAGE, app.packageName)
        intent.putExtra(AppDetailActivity.EXTRA_LABEL, app.label)
        startActivity(intent)
    }

    override fun onDestroyView() {
        super.onDestroyView()
        _binding = null
    }
}
