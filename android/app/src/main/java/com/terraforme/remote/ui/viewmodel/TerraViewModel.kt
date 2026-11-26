package com.terraforme.remote.ui.viewmodel

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.terraforme.remote.data.AgentInfo
import com.terraforme.remote.data.ChatMessage
import com.terraforme.remote.data.DEFAULT_PORT
import com.terraforme.remote.data.HostStore
import com.terraforme.remote.data.Instance
import com.terraforme.remote.data.RunDetail
import com.terraforme.remote.data.RunItem
import com.terraforme.remote.data.Usage
import com.terraforme.remote.data.TerraApi
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

data class TerraUiState(
    val serverUrl: String = "",
    val isConnected: Boolean = false,
    val isLoading: Boolean = false,
    val errorMessage: String? = null,
    val instances: List<Instance> = emptyList(),
    val agents: List<AgentInfo> = emptyList(),
    val runs: List<RunItem> = emptyList(),
    val usage: Usage? = null,
    val selectedRunDetail: RunDetail? = null,
    val isLoadingDetail: Boolean = false,
    val isSending: Boolean = false,
    val sendResult: String? = null,
    val isDiscovering: Boolean = false,
    val openInstanceId: String? = null,
    val history: List<ChatMessage> = emptyList()
)

class TerraViewModel(app: Application) : AndroidViewModel(app) {

    private val api = TerraApi()

    // As máquinas que já conectaram, no SQLite: o app abre na primeira (as do
    // Tailscale antes) com a última lista de instâncias dela, e tenta as
    // outras antes de varrer a rede.
    private val store = HostStore(app)
    private val _uiState = MutableStateFlow(
        store.urls().firstOrNull()?.let { url ->
            TerraUiState(
                serverUrl = url,
                instances = store.instances(url)?.let { runCatching { api.parseStatus(it) }.getOrNull() }.orEmpty()
            )
        } ?: TerraUiState()
    )
    val uiState: StateFlow<TerraUiState> = _uiState.asStateFlow()

    private var pollJob: Job? = null
    private var historyJob: Job? = null

    init {
        startPolling()
        // Desconectado, procura o terra a cada 5s: o do PC pode abrir depois
        // do app ou mudar de IP.
        viewModelScope.launch {
            while (isActive) {
                if (!_uiState.value.isConnected) discover()
                delay(5_000)
            }
        }
    }

    /** Tenta as máquinas salvas (Tailscale primeiro) e, sem nenhuma, varre a rede. */
    fun discover() {
        if (_uiState.value.isDiscovering) return
        viewModelScope.launch {
            _uiState.value = _uiState.value.copy(isDiscovering = true)
            val found = store.urls().firstOrNull { api.checkHealth(it) } ?: api.discover()
            _uiState.value = _uiState.value.copy(isDiscovering = false)
            if (found != null) {
                updateServerUrl(found)
            } else {
                _uiState.value = _uiState.value.copy(errorMessage = "Nenhum terra encontrado na rede")
            }
        }
    }

    /** O toque em Desconectado: recarrega e, sem resposta, procura na rede. */
    fun reconnect() {
        refreshAll()
        discover()
    }

    /** Abre a instância `id` e acompanha o histórico do chat da aba `tab`. */
    fun openInstance(id: String?, tab: String? = null) {
        _uiState.value = _uiState.value.copy(openInstanceId = id, sendResult = null, history = emptyList())
        historyJob?.cancel()
        if (id == null) return
        val tabId = tab ?: _uiState.value.instances.find { it.id == id }?.status?.tabs?.firstOrNull()?.id ?: return
        historyJob = viewModelScope.launch {
            while (isActive) {
                runCatching { api.getHistory(_uiState.value.serverUrl, tabId) }
                    .onSuccess { _uiState.value = _uiState.value.copy(history = it) }
                delay(2000)
            }
        }
    }

    fun updateServerUrl(url: String) {
        val clean = url.trim().trimEnd('/')
        _uiState.value = _uiState.value.copy(serverUrl = clean, isConnected = false)
        refreshAll()
        // Achou pela rede local: salva também o Tailscale da máquina, que vale fora de casa.
        viewModelScope.launch {
            api.tailscaleIp(clean)?.let { withContext(Dispatchers.IO) { store.save("http://$it:$DEFAULT_PORT") } }
        }
    }

    fun startPolling() {
        pollJob?.cancel()
        pollJob = viewModelScope.launch {
            while (isActive) {
                pollStatus()
                delay(2000)
            }
        }
    }

    fun stopPolling() {
        pollJob?.cancel()
    }

    fun refreshAll() {
        viewModelScope.launch {
            _uiState.value = _uiState.value.copy(isLoading = true)
            pollStatus()
            fetchAgents()
            fetchRuns()
            _uiState.value = _uiState.value.copy(isLoading = false)
        }
    }

    private suspend fun pollStatus() {
        val url = _uiState.value.serverUrl
        try {
            val json = api.getStatusJson(url)
            val instances = api.parseStatus(json)
            val usage = runCatching { api.getUsage(url) }.getOrNull()
            withContext(Dispatchers.IO) { store.save(url, json) }
            _uiState.value = _uiState.value.copy(
                isConnected = true,
                instances = instances,
                usage = usage ?: _uiState.value.usage,
                errorMessage = null
            )
        } catch (e: Exception) {
            _uiState.value = _uiState.value.copy(
                isConnected = false,
                errorMessage = e.localizedMessage ?: "Não foi possível conectar ao servidor"
            )
        }
    }

    fun fetchAgents() {
        val url = _uiState.value.serverUrl
        viewModelScope.launch {
            try {
                val agents = api.getAgents(url)
                _uiState.value = _uiState.value.copy(agents = agents)
            } catch (_: Exception) {}
        }
    }

    fun fetchRuns() {
        val url = _uiState.value.serverUrl
        viewModelScope.launch {
            try {
                val runs = api.getRuns(url)
                _uiState.value = _uiState.value.copy(runs = runs)
            } catch (_: Exception) {}
        }
    }

    fun loadRunDetail(runId: String) {
        val url = _uiState.value.serverUrl
        viewModelScope.launch {
            _uiState.value = _uiState.value.copy(isLoadingDetail = true, selectedRunDetail = null)
            try {
                val detail = api.getRunDetail(url, runId)
                _uiState.value = _uiState.value.copy(selectedRunDetail = detail, isLoadingDetail = false)
            } catch (e: Exception) {
                _uiState.value = _uiState.value.copy(
                    isLoadingDetail = false,
                    errorMessage = "Erro ao carregar execução: ${e.localizedMessage}"
                )
            }
        }
    }

    fun clearRunDetail() {
        _uiState.value = _uiState.value.copy(selectedRunDetail = null)
    }

    fun sendInput(tab: String, text: String) {
        val url = _uiState.value.serverUrl
        viewModelScope.launch {
            _uiState.value = _uiState.value.copy(isSending = true, sendResult = null)
            val res = api.sendInput(url, tab, text)
            _uiState.value = _uiState.value.copy(
                isSending = false,
                sendResult = if (res.ok) "Enviado ao terra." else "Erro: ${res.error}"
            )
        }
    }

    fun clearSendResult() {
        _uiState.value = _uiState.value.copy(sendResult = null)
    }
}
