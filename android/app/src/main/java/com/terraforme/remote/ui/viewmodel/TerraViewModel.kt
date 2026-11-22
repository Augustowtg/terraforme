package com.terraforme.remote.ui.viewmodel

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.terraforme.remote.data.AgentInfo
import com.terraforme.remote.data.Instance
import com.terraforme.remote.data.RunDetail
import com.terraforme.remote.data.RunItem
import com.terraforme.remote.data.Usage
import com.terraforme.remote.data.TerraApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

data class TerraUiState(
    val serverUrl: String = "http://localhost:8765",
    val isConnected: Boolean = false,
    val isLoading: Boolean = false,
    val errorMessage: String? = null,
    val instances: List<Instance> = emptyList(),
    val agents: List<AgentInfo> = emptyList(),
    val runs: List<RunItem> = emptyList(),
    val usage: Usage? = null,
    val selectedRunDetail: RunDetail? = null,
    val isLoadingDetail: Boolean = false,
    val lastUpdated: String? = null,
    val isSending: Boolean = false,
    val sendResult: String? = null,
    val isDiscovering: Boolean = false,
    val openInstanceId: String? = null
)

class TerraViewModel : ViewModel() {

    private val api = TerraApi()
    private val _uiState = MutableStateFlow(TerraUiState())
    val uiState: StateFlow<TerraUiState> = _uiState.asStateFlow()

    private var pollJob: Job? = null

    init {
        startPolling()
        // Sem servidor no endereço de agora, procura o terra na rede.
        viewModelScope.launch {
            if (!api.checkHealth(_uiState.value.serverUrl)) discover()
        }
    }

    fun discover() {
        if (_uiState.value.isDiscovering) return
        viewModelScope.launch {
            _uiState.value = _uiState.value.copy(isDiscovering = true)
            val found = api.discover()
            _uiState.value = _uiState.value.copy(isDiscovering = false)
            if (found != null) {
                updateServerUrl(found)
            } else {
                _uiState.value = _uiState.value.copy(errorMessage = "Nenhum terra encontrado na rede")
            }
        }
    }

    fun openInstance(id: String?) {
        _uiState.value = _uiState.value.copy(openInstanceId = id, sendResult = null)
    }

    fun updateServerUrl(url: String) {
        val clean = url.trim().trimEnd('/')
        _uiState.value = _uiState.value.copy(serverUrl = clean, isConnected = false)
        refreshAll()
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
            val instances = api.getStatus(url)
            val usage = runCatching { api.getUsage(url) }.getOrNull()
            val time = SimpleDateFormat("HH:mm:ss", Locale.getDefault()).format(Date())
            _uiState.value = _uiState.value.copy(
                isConnected = true,
                instances = instances,
                usage = usage ?: _uiState.value.usage,
                errorMessage = null,
                lastUpdated = time
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
