package com.terraforme.remote.data

data class Instance(
    val id: String,
    val pid: Long,
    val app: String,
    val cwd: String,
    val workspace: String,
    val started: Long,
    val health: String,
    val status: Status
)

data class Status(
    val state: String, // "running" or "idle"
    val task: String?,
    val chats: Int,
    val agents: Int,
    val tabs: List<Tab>
) {
    val isRunning: Boolean get() = state.equals("running", ignoreCase = true)
}

data class Tab(
    val id: String,
    val title: String,
    val name: String?,
    val kind: String,
    val task: String?,
    val run: String?
)

data class AgentInfo(
    val name: String,
    val cwd: String,
    val model: String?,
    val maxTurns: Int?,
    val dailyBudget: Int?,
    val tools: List<String>,
    val description: String?
)

data class RunItem(
    val id: String,
    val agent: String,
    val state: String, // "running", "done", "failed", "queued"
    val summary: String?,
    val error: String?,
    val started: Long?,
    val finished: Long?,
    val provider: String?,
    val model: String?,
    val usage: RunUsage?
) {
    val isRunning: Boolean get() = state == "running"
    val isDone: Boolean get() = state == "done"
    val isFailed: Boolean get() = state == "failed"
}

data class RunUsage(
    val input: Long,
    val output: Long,
    val cacheRead: Long,
    val costUsd: Double,
    val turns: Long
)

data class RunDetail(
    val run: RunItem,
    val stderr: String,
    val output: List<String>
)

data class WorkspaceItem(
    val name: String,
    val path: String
)

data class SendMessageResult(
    val ok: Boolean,
    val id: String?,
    val error: String?
)

data class AgentUsage(
    val agent: String,
    val runs: Int,
    val tokens: Long,
    val costUsd: Double,
    val budget: Int?,
    val alert: String? // "warning" | "blocked"
)

data class Usage(
    val agents: List<AgentUsage>,
    val totalRuns: Int,
    val totalTokens: Long,
    val totalCostUsd: Double,
    val running: Int
)
