package com.terraforme.remote.data

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.BufferedReader
import java.io.InputStreamReader
import java.io.OutputStreamWriter
import java.net.HttpURLConnection
import java.net.Inet4Address
import java.net.NetworkInterface
import java.net.URL

const val DEFAULT_PORT = 8765

class TerraApi {

    suspend fun checkHealth(baseUrl: String): Boolean = withContext(Dispatchers.IO) {
        try {
            val url = URL("${cleanUrl(baseUrl)}/api/health")
            val conn = (url.openConnection() as HttpURLConnection).apply {
                connectTimeout = 3000
                readTimeout = 3000
                requestMethod = "GET"
            }
            conn.responseCode in 200..299
        } catch (e: Exception) {
            false
        }
    }

    suspend fun getStatus(baseUrl: String): List<Instance> = withContext(Dispatchers.IO) {
        val jsonStr = httpGet("${cleanUrl(baseUrl)}/api/status")
        val json = JSONObject(jsonStr)
        val instancesArr = json.optJSONArray("instances") ?: JSONArray()
        val list = mutableListOf<Instance>()

        for (i in 0 until instancesArr.length()) {
            val obj = instancesArr.getJSONObject(i)
            val statusObj = obj.optJSONObject("status") ?: JSONObject()
            val tabsArr = statusObj.optJSONArray("tabs") ?: JSONArray()
            val tabs = mutableListOf<Tab>()

            for (j in 0 until tabsArr.length()) {
                val tabObj = tabsArr.getJSONObject(j)
                tabs.add(
                    Tab(
                        id = tabObj.optString("id", ""),
                        title = tabObj.optString("title", "Aba"),
                        name = if (tabObj.has("name") && !tabObj.isNull("name")) tabObj.getString("name") else null,
                        kind = tabObj.optString("kind", "manual"),
                        task = if (tabObj.has("task") && !tabObj.isNull("task")) tabObj.getString("task") else null,
                        run = if (tabObj.has("run") && !tabObj.isNull("run")) tabObj.getString("run") else null
                    )
                )
            }

            val status = Status(
                state = statusObj.optString("state", "idle"),
                task = if (statusObj.has("task") && !statusObj.isNull("task")) statusObj.getString("task") else null,
                chats = statusObj.optInt("chats", 1),
                agents = statusObj.optInt("agents", 0),
                tabs = tabs
            )

            list.add(
                Instance(
                    id = obj.optString("id", ""),
                    pid = obj.optLong("pid", 0),
                    app = obj.optString("app", ""),
                    cwd = obj.optString("cwd", ""),
                    workspace = obj.optString("workspace", "workspace"),
                    started = obj.optLong("started", 0),
                    health = obj.optString("health", "alive"),
                    status = status
                )
            )
        }
        list
    }

    suspend fun getAgents(baseUrl: String): List<AgentInfo> = withContext(Dispatchers.IO) {
        val jsonStr = httpGet("${cleanUrl(baseUrl)}/api/agents")
        val json = JSONObject(jsonStr)
        val arr = json.optJSONArray("agents") ?: JSONArray()
        val list = mutableListOf<AgentInfo>()

        for (i in 0 until arr.length()) {
            val obj = arr.getJSONObject(i)
            val toolsArr = obj.optJSONArray("tools") ?: JSONArray()
            val tools = mutableListOf<String>()
            for (j in 0 until toolsArr.length()) {
                tools.add(toolsArr.getString(j))
            }

            list.add(
                AgentInfo(
                    name = obj.optString("name", ""),
                    cwd = obj.optString("cwd", ""),
                    model = if (obj.has("model") && !obj.isNull("model")) obj.getString("model") else null,
                    maxTurns = if (obj.has("max_turns") && !obj.isNull("max_turns")) obj.getInt("max_turns") else null,
                    dailyBudget = if (obj.has("daily_budget") && !obj.isNull("daily_budget")) obj.getInt("daily_budget") else null,
                    tools = tools,
                    description = if (obj.has("description") && !obj.isNull("description")) obj.getString("description") else null
                )
            )
        }
        list
    }

    suspend fun getUsage(baseUrl: String): Usage = withContext(Dispatchers.IO) {
        val json = JSONObject(httpGet("${cleanUrl(baseUrl)}/api/usage"))
        val arr = json.optJSONArray("agents") ?: JSONArray()
        val agents = (0 until arr.length()).map { i ->
            val o = arr.getJSONObject(i)
            AgentUsage(
                agent = o.optString("agent"),
                runs = o.optInt("runs"),
                tokens = o.optLong("tokens"),
                costUsd = o.optDouble("cost_usd", 0.0),
                budget = if (o.isNull("budget")) null else o.optInt("budget"),
                alert = if (o.isNull("alert")) null else o.optString("alert")
            )
        }
        Usage(
            agents = agents,
            totalRuns = json.optInt("total_runs"),
            totalTokens = json.optLong("total_tokens"),
            totalCostUsd = json.optDouble("total_cost_usd", 0.0),
            running = json.optInt("running")
        )
    }

    suspend fun getRuns(baseUrl: String): List<RunItem> = withContext(Dispatchers.IO) {
        val jsonStr = httpGet("${cleanUrl(baseUrl)}/api/runs")
        val json = JSONObject(jsonStr)
        val arr = json.optJSONArray("runs") ?: JSONArray()
        val list = mutableListOf<RunItem>()

        for (i in 0 until arr.length()) {
            val obj = arr.getJSONObject(i)
            val usageObj = obj.optJSONObject("usage")
            val usage = usageObj?.let {
                RunUsage(
                    input = it.optLong("input", 0),
                    output = it.optLong("output", 0),
                    cacheRead = it.optLong("cache_read", 0),
                    costUsd = it.optDouble("cost_usd", 0.0),
                    turns = it.optLong("turns", 0)
                )
            }

            list.add(
                RunItem(
                    id = obj.optString("id", ""),
                    agent = obj.optString("agent", "agente"),
                    state = obj.optString("state", "queued"),
                    summary = if (obj.has("summary") && !obj.isNull("summary")) obj.getString("summary") else null,
                    error = if (obj.has("error") && !obj.isNull("error")) obj.getString("error") else null,
                    started = if (obj.has("started") && !obj.isNull("started")) obj.getLong("started") else null,
                    finished = if (obj.has("finished") && !obj.isNull("finished")) obj.getLong("finished") else null,
                    provider = if (obj.has("provider") && !obj.isNull("provider")) obj.getString("provider") else null,
                    model = if (obj.has("model") && !obj.isNull("model")) obj.getString("model") else null,
                    usage = usage
                )
            )
        }
        list
    }

    suspend fun getRunDetail(baseUrl: String, runId: String): RunDetail = withContext(Dispatchers.IO) {
        val jsonStr = httpGet("${cleanUrl(baseUrl)}/api/runs/$runId")
        val json = JSONObject(jsonStr)
        val obj = json.getJSONObject("run")
        val stderr = json.optString("stderr", "")

        val outputArr = json.optJSONArray("output") ?: JSONArray()
        val outputLines = mutableListOf<String>()
        for (i in 0 until outputArr.length()) {
            val item = outputArr.get(i)
            outputLines.add(item.toString())
        }

        val usageObj = obj.optJSONObject("usage")
        val usage = usageObj?.let {
            RunUsage(
                input = it.optLong("input", 0),
                output = it.optLong("output", 0),
                cacheRead = it.optLong("cache_read", 0),
                costUsd = it.optDouble("cost_usd", 0.0),
                turns = it.optLong("turns", 0)
            )
        }

        val runItem = RunItem(
            id = obj.optString("id", runId),
            agent = obj.optString("agent", "agente"),
            state = obj.optString("state", "queued"),
            summary = if (obj.has("summary") && !obj.isNull("summary")) obj.getString("summary") else null,
            error = if (obj.has("error") && !obj.isNull("error")) obj.getString("error") else null,
            started = if (obj.has("started") && !obj.isNull("started")) obj.getLong("started") else null,
            finished = if (obj.has("finished") && !obj.isNull("finished")) obj.getLong("finished") else null,
            provider = if (obj.has("provider") && !obj.isNull("provider")) obj.getString("provider") else null,
            model = if (obj.has("model") && !obj.isNull("model")) obj.getString("model") else null,
            usage = usage
        )

        RunDetail(run = runItem, stderr = stderr, output = outputLines)
    }

    /** Digita `text` na aba `tab` do terra, como no teclado do PC. */
    suspend fun sendInput(baseUrl: String, tab: String, text: String): SendMessageResult = withContext(Dispatchers.IO) {
        try {
            val body = JSONObject().apply {
                put("tab", tab)
                put("text", text)
            }
            val json = JSONObject(httpPost("${cleanUrl(baseUrl)}/api/input", body.toString()))
            if (json.optBoolean("ok", false)) {
                SendMessageResult(ok = true, id = null, error = null)
            } else {
                SendMessageResult(ok = false, id = null, error = json.optString("error", "Erro ao digitar na aba"))
            }
        } catch (e: Exception) {
            SendMessageResult(ok = false, id = null, error = e.localizedMessage ?: "Erro de conexão")
        }
    }

    /**
     * Procura o terra na rede: o localhost (USB com adb reverse) e cada IP da
     * sub-rede /24 do celular, na porta padrão. Devolve a URL do primeiro que
     * responde como terraforme.
     */
    // shortcut: varre só /24; redes maiores pedem o IP à mão nas configurações.
    suspend fun discover(port: Int = DEFAULT_PORT): String? = coroutineScope {
        val prefixes = NetworkInterface.getNetworkInterfaces().toList()
            .filter { it.isUp && !it.isLoopback }
            .flatMap { it.inetAddresses.toList() }
            .filterIsInstance<Inet4Address>()
            .filter { it.isSiteLocalAddress }
            .mapNotNull { it.hostAddress?.substringBeforeLast('.') }
            .distinct()
        val candidates = listOf("http://localhost:$port") +
            prefixes.flatMap { p -> (1..254).map { "http://$p.$it:$port" } }
        candidates
            .map { url -> async(Dispatchers.IO) { url.takeIf { isTerra(it) } } }
            .awaitAll()
            .firstOrNull { it != null }
    }

    private fun isTerra(baseUrl: String): Boolean = try {
        val conn = (URL("$baseUrl/api/health").openConnection() as HttpURLConnection).apply {
            connectTimeout = 400
            readTimeout = 800
        }
        conn.responseCode == 200 && conn.inputStream.bufferedReader().use { it.readText() }.contains("\"terraforme\"")
    } catch (e: Exception) {
        false
    }

    private fun httpGet(urlStr: String): String {
        val url = URL(urlStr)
        val conn = (url.openConnection() as HttpURLConnection).apply {
            connectTimeout = 4000
            readTimeout = 6000
            requestMethod = "GET"
            setRequestProperty("Accept", "application/json")
        }
        val code = conn.responseCode
        val stream = if (code in 200..299) conn.inputStream else conn.errorStream
        val reader = BufferedReader(InputStreamReader(stream ?: conn.inputStream))
        return reader.use { it.readText() }
    }

    private fun httpPost(urlStr: String, jsonBody: String): String {
        val url = URL(urlStr)
        val conn = (url.openConnection() as HttpURLConnection).apply {
            connectTimeout = 5000
            readTimeout = 10000
            requestMethod = "POST"
            doOutput = true
            setRequestProperty("Content-Type", "application/json")
            setRequestProperty("Accept", "application/json")
        }
        OutputStreamWriter(conn.outputStream).use { it.write(jsonBody) }
        val code = conn.responseCode
        val stream = if (code in 200..299) conn.inputStream else conn.errorStream
        val reader = BufferedReader(InputStreamReader(stream ?: conn.inputStream))
        return reader.use { it.readText() }
    }

    private fun cleanUrl(url: String): String {
        return url.trim().trimEnd('/')
    }
}
